//! Provider switch mid-chat: `setChatConfig` with another harness tombstones
//! the chat's session, the new harness's first turn carries the conversation
//! as text, later turns resume its own session, and the old harness's
//! checkpoints can no longer be branched at.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use zeron_doc::{
    MessagePart, MessageRole, MessageStatus, SessionCommandPayload, SessionMessageEntry,
};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::{
    AgentEvent, AnchorRole, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SessionBranch, SteeringMode,
};

const CHAT: &str = "chat-1";
const FORK: &str = "chat-fork";

#[derive(Debug, Clone, PartialEq)]
struct Seen {
    prompt: String,
    resume: Option<String>,
    branch: Option<SessionBranch>,
}

/// One registered harness: records every run it is asked for, mints a
/// session named after itself, and emits anchors like a real driver.
struct TestHarness {
    id: HarnessId,
    name: &'static str,
    runs: Arc<Mutex<Vec<Seen>>>,
    turns: AtomicUsize,
}

impl TestHarness {
    fn new(id: HarnessId, name: &'static str) -> Self {
        Self {
            id,
            name,
            runs: Arc::new(Mutex::new(Vec::new())),
            turns: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Harness for TestHarness {
    fn id(&self) -> HarnessId {
        self.id
    }
    fn display_name(&self) -> &str {
        self.name
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[ReasoningLevel::Medium]
    }
    fn branching(&self) -> bool {
        true
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        _controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.runs.lock().unwrap().push(Seen {
            prompt: request.prompt.clone(),
            resume: request.resume.clone(),
            branch: request.branch.clone(),
        });
        let n = self.turns.fetch_add(1, Ordering::SeqCst) + 1;
        let session_id = match (&request.resume, &request.branch) {
            (Some(_), Some(_)) => format!("{}-forked", self.name),
            (Some(resume), None) => resume.clone(),
            (None, _) => format!("{}-sess", self.name),
        };
        let events = vec![
            AgentEvent::SessionStarted {
                harness: self.id,
                model: "m".into(),
                tools: vec![],
                cwd: request.cwd.clone(),
                session_id: session_id.clone(),
                assistant_message_id: format!("{}-a-{n}", self.name),
            },
            AgentEvent::Anchor {
                role: AnchorRole::User,
                id: format!("{}-user-{n}", self.name),
                entry_id: None,
            },
            AgentEvent::TextDelta {
                text: format!("{} answer {n}", self.name),
            },
            AgentEvent::Anchor {
                role: AnchorRole::Assistant,
                id: format!("{}-reply-{n}", self.name),
                entry_id: None,
            },
            AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: Some(session_id),
            },
        ];
        Ok(futures::stream::iter(events.into_iter().map(Ok)).boxed())
    }
}

async fn wait_for<F: FnMut() -> bool>(mut predicate: F, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

fn entries(core: &EngineCore, chat_id: &str) -> Vec<SessionMessageEntry> {
    core.doc_host
        .open(chat_id)
        .ok()
        .and_then(|h| h.doc().read_entries().ok())
        .unwrap_or_default()
}

fn entry_text(entry: &SessionMessageEntry) -> String {
    entry
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn run_payload(prompt: &str, message_id: &str, cwd: &str) -> SessionCommandPayload {
    SessionCommandPayload::Run {
        request: RunRequest {
            prompt: prompt.into(),
            harness: None,
            model: None,
            reasoning: None,
            model_options: Default::default(),
            cwd: cwd.into(),
            sandbox: SandboxLevel::WorkspaceWrite,
            permission_mode: zeron_proto::PermissionMode::Full,
            auto_approve: true,
            attachments: Vec::new(),
            resume: None,
            worktree: None,
            branch: None,
            forked_at: None,
            computer_control: false,
            mcp: None,
        },
        message_id: message_id.into(),
    }
}

struct Rig {
    core: EngineCore,
    client: zeron_rpc::RpcClient,
    old: Arc<TestHarness>,
    new: Arc<TestHarness>,
    cwd: String,
}

impl Rig {
    async fn turn(&self, chat_id: &str, prompt: &str, message_id: &str) {
        let before = entries(&self.core, chat_id).len();
        self.core
            .doc_host
            .queue_command(chat_id, run_payload(prompt, message_id, &self.cwd))
            .expect("queue run");
        wait_for(
            || entries(&self.core, chat_id).len() >= before + 2,
            "turn entries",
        )
        .await;
        wait_for(
            || {
                entries(&self.core, chat_id)
                    .last()
                    .is_some_and(|e| e.status == Some(MessageStatus::Complete))
                    && !self.core.sessions.turn_in_flight(chat_id)
            },
            "turn settles",
        )
        .await;
        wait_for(
            || {
                self.session(chat_id)
                    .is_some_and(|session| !session.is_empty())
            },
            "session row",
        )
        .await;
    }

    fn session(&self, chat_id: &str) -> Option<String> {
        self.core
            .workspace
            .chat(chat_id)
            .expect("read row")
            .expect("row exists")
            .harness_session_id
    }

    /// The UI's provider switch: `setChatConfig` naming the other harness,
    /// with the transcript note it writes alongside.
    async fn switch_to(&self, harness: &str, note: bool) {
        let mut params = serde_json::json!({
            "op": "setChatConfig",
            "chatId": CHAT,
            "config": {
                "harness": harness,
                "model": null,
                "reasoning": null,
                "modelOptions": {},
                "sandbox": "workspace-write",
            },
        });
        if note {
            params["note"] = serde_json::json!({
                "text": "Provider changed from Old to New.",
                "hint": "The conversation so far is handed to New as text.",
            });
        }
        self.client
            .call(zeron_rpc::methods::MUTATE, params)
            .await
            .expect("setChatConfig");
    }

    fn reply_id(&self, chat_id: &str, ix: usize) -> String {
        let entries = entries(&self.core, chat_id);
        assert_eq!(entries[ix].role, MessageRole::Assistant);
        entries[ix].id.clone()
    }

    fn prompt_id(&self, chat_id: &str, ix: usize) -> String {
        let entries = entries(&self.core, chat_id);
        assert_eq!(entries[ix].role, MessageRole::User);
        entries[ix].id.clone()
    }
}

async fn rig(dir: &std::path::Path) -> Rig {
    let old = Arc::new(TestHarness::new(HarnessId::Mock, "Old"));
    let new = Arc::new(TestHarness::new(HarnessId::Pi, "New"));
    let registry = HarnessRegistry::new();
    registry.register(old.clone());
    registry.register(new.clone());
    let core = EngineCore::assemble(&dir.join("data"), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    let client = zeron_rpc::memory_client(core.rpc_service());
    client
        .call(
            zeron_rpc::methods::MUTATE,
            serde_json::json!({ "op": "createChat", "chatId": CHAT, "deviceId": core.device_id }),
        )
        .await
        .expect("createChat");
    core.workspace
        .rename_chat(CHAT, "Auth work")
        .expect("rename chat");
    Rig {
        core,
        client,
        old,
        new,
        cwd: dir.to_string_lossy().into_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_new_harness_starts_fresh_with_the_conversation_as_text_then_resumes_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.turn(CHAT, "first", "m1").await;
    rig.turn(CHAT, "second", "m2").await;
    assert_eq!(rig.session(CHAT).as_deref(), Some("Old-sess"));

    rig.switch_to("pi", true).await;
    assert_eq!(
        rig.session(CHAT).as_deref(),
        Some(""),
        "the old session is tombstoned, never handed to the new harness"
    );
    let divider = entries(&rig.core, CHAT).into_iter().last().unwrap();
    assert_eq!(divider.role, MessageRole::System);
    assert_eq!(
        entry_text(&divider),
        "Provider changed from Old to New.The conversation so far is handed to New as text."
    );
    // The packet is on the divider from the moment of the switch.
    let packet = match &divider.parts[2] {
        MessagePart::Handoff {
            harness: HarnessId::Pi,
            text,
            ..
        } => text.clone(),
        other => panic!("{other:?}"),
    };
    assert!(packet.ends_with("Assistant: Old answer 2\n</previous_conversation>"));

    rig.turn(CHAT, "third", "m3").await;
    let seen = rig.new.runs.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].resume, None);
    assert_eq!(seen[0].branch, None);
    assert_eq!(
        seen[0].prompt,
        "<provider_switch>\nThis chat was moved to you from another agent. Its conversation so far is below as plain text; that agent's tool results and file state were not carried over. Continue from where it left off.\n</provider_switch>\n\n<previous_conversation>\nUser: first\n\nAssistant: Old answer 1\n\nUser: second\n\nAssistant: Old answer 2\n</previous_conversation>\n\n<latest_user_message>\nthird\n</latest_user_message>"
    );
    assert_eq!(
        rig.old.runs.lock().unwrap().len(),
        2,
        "the old harness is not run again"
    );
    // The first turn sends the recorded packet verbatim; the doc keeps the
    // typed text and does not record a second copy on the user entry.
    assert!(seen[0].prompt.starts_with(packet.as_str()));
    let doc = entries(&rig.core, CHAT);
    let prompt = &doc[doc.len() - 2];
    assert_eq!(entry_text(prompt), "third");
    assert_eq!(prompt.parts.len(), 1);
    assert_eq!(rig.session(CHAT).as_deref(), Some("New-sess"));

    rig.turn(CHAT, "fourth", "m4").await;
    let seen = rig.new.runs.lock().unwrap().clone();
    assert_eq!(seen[1].prompt, "fourth");
    assert_eq!(seen[1].resume.as_deref(), Some("New-sess"));
    // Switching back hands the conversation over again, and the recorded
    // packet is not conversation: it never nests inside the next one.
    rig.switch_to("mock", true).await;
    rig.turn(CHAT, "fifth", "m5").await;
    let seen = rig.old.runs.lock().unwrap().clone();
    let last = &seen.last().unwrap().prompt;
    assert_eq!(last.matches("<provider_switch>").count(), 1, "{last}");
    assert!(
        last.contains("Assistant: New answer 1\n\nUser: fourth\n\nAssistant: New answer 2\n</previous_conversation>"),
        "{last}"
    );
}

async fn switch(rig: &Rig, harness: &str, from: &str, to: &str) {
    rig.client
        .call(
        zeron_rpc::methods::MUTATE,
        serde_json::json!({
            "op": "setChatConfig",
            "chatId": CHAT,
            "config": {
                "harness": harness, "model": null, "reasoning": null,
                "modelOptions": {}, "sandbox": "workspace-write",
            },
            "note": {
                "text": format!("Provider changed from {from} to {to}."),
                "hint": format!("{from} session closed. Handed to {to}; {from}'s files were not carried."),
            },
        }),
        )
        .await
        .expect("switch");
}

fn previous_harnesses(rig: &Rig) -> Vec<HarnessId> {
    rig.core
        .workspace
        .chat(CHAT)
        .unwrap()
        .unwrap()
        .previous_harnesses
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_switch_before_any_message_replaces_the_first_divider() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.turn(CHAT, "first", "m1").await;
    switch(&rig, "pi", "Old", "New").await;
    switch(&rig, "codex", "New", "Codex").await;
    let doc = entries(&rig.core, CHAT);
    // user, reply, one divider
    assert_eq!(doc.len(), 3);
    let divider = doc.last().unwrap();
    assert_eq!(divider.role, MessageRole::System);
    assert_eq!(
        entry_text(divider),
        "Provider changed from Old to Codex.Old session closed. Handed to Codex; Old's files were not carried."
    );
    assert!(matches!(
        &divider.parts[2],
        MessagePart::Handoff {
            harness: HarnessId::Codex,
            ..
        }
    ));
    // The harness in between never ran, so it leaves no mark.
    assert_eq!(previous_harnesses(&rig), vec![HarnessId::Mock]);
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_back_before_any_message_leaves_no_divider() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.turn(CHAT, "first", "m1").await;
    switch(&rig, "pi", "Old", "New").await;
    switch(&rig, "mock", "New", "Old").await;
    let doc = entries(&rig.core, CHAT);
    assert_eq!(doc.len(), 2);
    assert_eq!(previous_harnesses(&rig), vec![HarnessId::Mock]);

    // The session was still closed: the next turn hands over once, on the
    // user entry, to the harness the chat never visibly left.
    rig.turn(CHAT, "second", "m2").await;
    assert!(rig.new.runs.lock().unwrap().is_empty());
    let seen = rig.old.runs.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].resume, None);
    assert_eq!(seen[1].prompt.matches("<provider_switch>").count(), 1);
    let doc = entries(&rig.core, CHAT);
    assert!(doc[2].parts.iter().any(|part| matches!(
        part,
        MessagePart::Handoff {
            harness: HarnessId::Mock,
            ..
        }
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_switch_on_an_empty_chat_hands_nothing_over() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.switch_to("pi", false).await;
    rig.turn(CHAT, "first", "m1").await;
    let seen = rig.new.runs.lock().unwrap().clone();
    assert_eq!(seen[0].prompt, "first");
    assert_eq!(seen[0].resume, None);
    assert!(rig.old.runs.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn old_checkpoints_cannot_be_branched_after_the_switch() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.turn(CHAT, "first", "m1").await;
    rig.switch_to("pi", true).await;
    rig.turn(CHAT, "second", "m2").await;
    rig.turn(CHAT, "third", "m3").await;
    let doc = entries(&rig.core, CHAT);
    // user, reply, divider, user, reply, user, reply
    assert_eq!(doc.len(), 7);

    let err = rig
        .core
        .sessions
        .fork_chat(CHAT, &rig.reply_id(CHAT, 1), FORK)
        .expect_err("the old harness's reply is not a fork point");
    assert!(err.to_string().contains("previous provider"), "{err}");
    assert!(rig.core.workspace.chat(FORK).unwrap().is_none());

    // A reply the new harness produced branches as usual.
    rig.core
        .sessions
        .fork_chat(CHAT, &rig.reply_id(CHAT, 4), FORK)
        .expect("fork after the switch");
    rig.turn(FORK, "on the fork", "m4").await;
    let seen = rig.new.runs.lock().unwrap().clone();
    assert_eq!(
        seen.last().unwrap().branch,
        Some(SessionBranch {
            keep: Some("New-reply-1".into()),
            drop: Some("New-user-2".into()),
        })
    );

    // Rewinding to before the first prompt after the switch lands on the old
    // harness's reply, which cannot be branched: the session is tombstoned
    // again and the next run gets the remaining conversation as text.
    rig.core
        .sessions
        .rewind_chat(CHAT, &rig.prompt_id(CHAT, 3))
        .await
        .expect("rewind across the switch");
    assert_eq!(rig.session(CHAT).as_deref(), Some(""));
    rig.turn(CHAT, "second, again", "m5").await;
    let seen = rig.new.runs.lock().unwrap().clone();
    let last = seen.last().unwrap();
    assert_eq!(last.resume, None);
    assert!(
        last.prompt.contains("<previous_conversation>\nUser: first\n\nAssistant: Old answer 1\n</previous_conversation>"),
        "{}",
        last.prompt
    );
}
