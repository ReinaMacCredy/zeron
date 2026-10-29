//! Stall detectors of watched trees (general-mc U17-U20), on thresholds shortened for the whole
//! test process, so these tests live apart from thread_tools' wake-count assertions.

use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use zeron_doc::{CommandBasedOn, SessionCommandEntry, SessionCommandPayload, SessionCommandStatus};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::supervisor::{SupervisorEventKind, SupervisorEventSummary, SupervisorSnapshot};
use zeron_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SteeringMode, ToolCall,
};

const STALL_MS: u64 = 1_500;

fn init_env() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: set before any engine in this process reads it; every test shares the values.
        unsafe {
            std::env::set_var("ZERON_SUPERVISOR_STALL_MS", STALL_MS.to_string());
            std::env::set_var("ZERON_SUPERVISOR_FOLD_MS", "200");
        }
    });
}

/// Fails on "please fail", asks on "please ask", writes the named file on "please write <path>", and on "please fan out"
/// ends its turn with a background sub-agent still running until the run is cancelled. Only a
/// steerable run parks after its turn, so fanning out runs on the steerable `Pi` id.
struct StallHarness {
    prompts: Arc<Mutex<Vec<String>>>,
    id: HarnessId,
}

#[async_trait]
impl Harness for StallHarness {
    fn id(&self) -> HarnessId {
        self.id
    }
    fn display_name(&self) -> &str {
        "Stall"
    }
    fn supports_steering(&self) -> bool {
        self.id != HarnessId::Mock
    }
    fn steering_mode(&self) -> SteeringMode {
        match self.supports_steering() {
            true => SteeringMode::StepBoundary,
            false => SteeringMode::TurnBoundary,
        }
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.prompts.lock().unwrap().push(request.prompt.clone());
        let prompt = request.prompt.clone();
        let RunControls {
            request_input,
            interrupt,
            turn,
            ..
        } = controls;
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<AgentEvent, HarnessError>>(8);
        tokio::spawn(async move {
            let _ = tx
                .send(Ok(AgentEvent::SessionStarted {
                    harness: HarnessId::Mock,
                    model: "stall-1".into(),
                    tools: vec![],
                    cwd: request.cwd.clone(),
                    session_id: "hs-stall".into(),
                    assistant_message_id: "a-stall".into(),
                }))
                .await;
            if prompt.starts_with("please fail") {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let _ = tx
                    .send(Ok(AgentEvent::Done {
                        status: DoneStatus::Errored,
                        result: None,
                        error: Some("boom".into()),
                        session_id: Some("hs-stall".into()),
                    }))
                    .await;
                return;
            }
            let mut text = "noted".to_string();
            if prompt.starts_with("please ask") {
                let answers = (request_input)(vec![zeron_proto::UserInputQuestion {
                    id: "q-ship".into(),
                    header: "Agent question".into(),
                    question: "Ship it?".into(),
                    options: vec!["Yes".into(), "No".into()],
                    multi_select: false,
                    kind: None,
                    prefill: None,
                    multiline: false,
                    approval: false,
                }])
                .await
                .unwrap_or_default();
                text = answers
                    .first()
                    .and_then(|a| a.labels.first().cloned())
                    .unwrap_or_else(|| "no answer".into());
            }
            if let Some(path) = prompt.strip_prefix("please write ") {
                std::fs::write(path.trim(), "left behind\n").unwrap();
                for event in [
                    AgentEvent::ToolCall {
                        id: "toolu_write".into(),
                        call: ToolCall::WriteFile {
                            path: path.trim().into(),
                            content: None,
                        },
                    },
                    AgentEvent::ToolResult {
                        id: "toolu_write".into(),
                        is_error: false,
                        output: None,
                        diff: None,
                    },
                ] {
                    let _ = tx.send(Ok(event)).await;
                }
            }
            let fans_out = prompt.starts_with("please fan out");
            if fans_out {
                for event in [
                    AgentEvent::ToolCall {
                        id: "toolu_scout".into(),
                        call: ToolCall::Unknown {
                            name: "Agent: scout".into(),
                            input: Some(
                                serde_json::json!({ "description": "scout", "prompt": "look" }),
                            ),
                        },
                    },
                    AgentEvent::ToolResult {
                        id: "toolu_scout".into(),
                        is_error: false,
                        output: None,
                        diff: None,
                    },
                ] {
                    let _ = tx.send(Ok(event)).await;
                }
                turn.set_background(1);
            }
            let _ = tx.send(Ok(AgentEvent::TextDelta { text })).await;
            let _ = tx
                .send(Ok(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: Some("hs-stall".into()),
                }))
                .await;
            if fans_out {
                interrupt.cancelled().await;
            }
        });
        Ok(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        })
        .boxed())
    }
}

struct Rig {
    core: EngineCore,
    prompts: Arc<Mutex<Vec<String>>>,
    dir: tempfile::TempDir,
}

fn rig() -> Rig {
    init_env();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    for id in [HarnessId::Mock, HarnessId::Pi] {
        registry.register(Arc::new(StallHarness {
            prompts: prompts.clone(),
            id,
        }));
    }
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    core.workspace
        .create_space("space-s", "dev-a", "/tmp", None, false)
        .expect("space");
    core.workspace
        .create_chat("lead", Some("space-s"), None, None, None)
        .expect("lead chat");
    core.workspace.rename_chat("lead", "Shop backend").unwrap();
    Rig { core, prompts, dir }
}

fn child(rig: &Rig, id: &str, cwd: Option<&str>) {
    rig.core
        .workspace
        .create_chat_from(
            id,
            Some("space-s"),
            None,
            None,
            cwd.map(str::to_owned),
            Some("lead"),
        )
        .unwrap();
    rig.core
        .workspace
        .rename_chat(id, &format!("Work {id}"))
        .unwrap();
}

async fn call(rig: &Rig, method: &'static str, params: serde_json::Value) -> serde_json::Value {
    match zeron_rpc::RpcService::handle(&*rig.core.rpc_service(), method, params)
        .await
        .unwrap()
    {
        zeron_rpc::RpcReply::Value(value) => value,
        _ => serde_json::Value::Null,
    }
}

async fn watch_lead(rig: &Rig) -> String {
    let ensured = call(
        rig,
        zeron_rpc::methods::ENSURE_SUPERVISOR,
        serde_json::json!({ "config": { "harness": "mock", "sandbox": "workspace-write" } }),
    )
    .await;
    call(
        rig,
        zeron_rpc::methods::MUTATE,
        serde_json::json!({ "op": "setTreeWatched", "rootChatId": "lead", "watched": true }),
    )
    .await;
    snapshot_until(rig, |s| s.watched_roots == ["lead"]).await;
    ensured["chatId"].as_str().unwrap().to_string()
}

async fn snapshot_until(
    rig: &Rig,
    ready: impl Fn(&SupervisorSnapshot) -> bool,
) -> SupervisorSnapshot {
    let zeron_rpc::RpcReply::Stream(mut watch) = zeron_rpc::RpcService::handle(
        &*rig.core.rpc_service(),
        zeron_rpc::methods::WATCH_SUPERVISOR,
        serde_json::json!({}),
    )
    .await
    .unwrap() else {
        panic!("a snapshot stream");
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let item = tokio::time::timeout_at(deadline, watch.next())
            .await
            .expect("timed out waiting for the Supervisor snapshot")
            .unwrap();
        let snapshot = serde_json::from_value(item).unwrap();
        if ready(&snapshot) {
            return snapshot;
        }
    }
}

fn events(snapshot: &SupervisorSnapshot, kind: SupervisorEventKind) -> Vec<SupervisorEventSummary> {
    snapshot
        .wakes
        .iter()
        .flat_map(|wake| &wake.events)
        .filter(|event| event.kind == kind)
        .cloned()
        .collect()
}

/// Delivered and dropped alike: an event that reached the router at all.
fn offered(
    snapshot: &SupervisorSnapshot,
    kind: SupervisorEventKind,
) -> Vec<SupervisorEventSummary> {
    snapshot
        .wakes
        .iter()
        .flat_map(|wake| wake.events.iter().chain(&wake.dropped))
        .filter(|event| event.kind == kind)
        .cloned()
        .collect()
}

async fn wait_for<F>(mut predicate: F, what: &str)
where
    F: FnMut() -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn queue_run(rig: &Rig, chat_id: &str, prompt: &str, cwd: &str) {
    queue_run_on(rig, chat_id, prompt, cwd, None);
}

fn queue_run_on(rig: &Rig, chat_id: &str, prompt: &str, cwd: &str, harness: Option<HarnessId>) {
    let handle = rig.core.doc_host.open(chat_id).expect("open chat");
    handle
        .doc()
        .queue_command(&SessionCommandEntry {
            id: format!("cmd-{chat_id}-{}", rig.prompts.lock().unwrap().len()),
            payload: SessionCommandPayload::Run {
                request: RunRequest {
                    prompt: prompt.into(),
                    harness,
                    model: None,
                    reasoning: None,
                    model_options: Default::default(),
                    cwd: cwd.into(),
                    sandbox: SandboxLevel::WorkspaceWrite,
                    permission_mode: zeron_proto::PermissionMode::Full,
                    auto_approve: true,
                    access: zeron_proto::PermissionMode::Full.access(),
                    plan: false,
                    attachments: Vec::new(),
                    worktree: None,
                    branch: None,
                    resume: None,
                    forked_at: None,
                    computer_control: false,
                    mcp: None,
                    disabled_tools: Vec::new(),
                    env: Default::default(),
                },
                message_id: format!("msg-{chat_id}-{prompt}"),
            },
            issued_by: "viewer".into(),
            issued_at: chrono::Utc::now().timestamp_millis(),
            based_on: None::<CommandBasedOn>,
            expires_at: None,
            status: SessionCommandStatus::Pending,
            resolution: None,
            computer_generation: None,
        })
        .expect("queue command");
}

fn pending_request(rig: &Rig, chat_id: &str) -> Option<String> {
    let entries = rig
        .core
        .doc_host
        .open(chat_id)
        .ok()?
        .doc()
        .read_entries()
        .ok()?;
    entries.iter().rev().find_map(|entry| {
        entry.parts.iter().find_map(|part| match part {
            zeron_doc::MessagePart::Input {
                request_id,
                resolved: false,
                ..
            } => Some(request_id.clone()),
            _ => None,
        })
    })
}

fn idle(rig: &Rig, chat_id: &str) -> bool {
    rig.core
        .sessions
        .session_status(chat_id)
        .is_some_and(|s| s.status == zeron_proto::SessionStatus::Idle)
}

fn wakes_text(rig: &Rig) -> String {
    rig.prompts
        .lock()
        .unwrap()
        .iter()
        .filter(|prompt| prompt.contains("Supervisor wake"))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn a_watched_question_wakes_once_after_the_threshold_and_never_once_answered() {
    let rig = rig();
    watch_lead(&rig).await;
    child(&rig, "child-a", None);
    child(&rig, "child-b", None);

    queue_run(&rig, "child-b", "please ask", "/tmp");
    wait_for(
        || pending_request(&rig, "child-b").is_some(),
        "child-b's question",
    )
    .await;
    let answered = pending_request(&rig, "child-b").unwrap();
    call(
        &rig,
        zeron_rpc::methods::QUEUE_COMMAND,
        serde_json::json!({
            "chatId": "child-b",
            "command": {
                "kind": "respondInput", "requestId": answered,
                "answers": [{ "questionId": "q-ship", "labels": ["Yes"] }],
            },
        }),
    )
    .await;
    wait_for(
        || pending_request(&rig, "child-b").is_none() && idle(&rig, "child-b"),
        "the person's answer",
    )
    .await;

    queue_run(&rig, "child-a", "please ask", "/tmp");
    wait_for(
        || pending_request(&rig, "child-a").is_some(),
        "child-a's question",
    )
    .await;
    let request = pending_request(&rig, "child-a").unwrap();
    let snapshot = snapshot_until(&rig, |s| {
        !events(s, SupervisorEventKind::QuestionWaiting).is_empty()
    })
    .await;
    let [waiting] = events(&snapshot, SupervisorEventKind::QuestionWaiting)
        .try_into()
        .unwrap();
    assert_eq!(waiting.key, format!("question-waiting:child-a:{request}"));
    assert_eq!(waiting.thread_id.as_deref(), Some("child-a"));
    assert_eq!(waiting.root_chat_id.as_deref(), Some("lead"));

    tokio::time::sleep(Duration::from_millis(3 * STALL_MS)).await;
    let snapshot = snapshot_until(&rig, |_| true).await;
    let waiting = offered(&snapshot, SupervisorEventKind::QuestionWaiting);
    assert_eq!(
        waiting.len(),
        1,
        "once per request, never for the answered one: {waiting:?}"
    );
    wait_for(|| wakes_text(&rig).contains("has waited"), "the wake text").await;
    let text = wakes_text(&rig);
    assert!(
        text.contains("Thread \"Work child-a\" has waited ") && text.contains(" for an answer"),
        "{text}"
    );
    rig.core.shutdown().await;
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-C"])
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repository on `main` with one commit, plus a worktree per branch name in `branches`.
fn repo(rig: &Rig, branches: &[&str]) -> std::path::PathBuf {
    let main = rig.dir.path().join("repo");
    std::fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    std::fs::write(main.join("README.md"), "repo\n").unwrap();
    std::fs::write(main.join("shared.rs"), "fn shared() {}\n").unwrap();
    git(&main, &["add", "-A"]);
    git(&main, &["commit", "-q", "-m", "init"]);
    for branch in branches {
        let tree = rig.dir.path().join(branch);
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                tree.to_str().unwrap(),
            ],
        );
    }
    main
}

fn path(dir: &std::path::Path) -> &str {
    dir.to_str().unwrap()
}

#[tokio::test]
async fn a_watched_turn_that_leaves_files_or_an_unmerged_branch_wakes_once_per_head() {
    let rig = rig();
    watch_lead(&rig).await;
    let main = repo(&rig, &["clean", "feat"]);
    let (clean, feat) = (rig.dir.path().join("clean"), rig.dir.path().join("feat"));
    std::fs::write(feat.join("feature.rs"), "fn feature() {}\n").unwrap();
    git(&feat, &["add", "-A"]);
    git(&feat, &["commit", "-q", "-m", "feature"]);
    child(&rig, "child-dirty", Some(path(&main)));
    child(&rig, "child-clean", Some(path(&clean)));
    child(&rig, "child-feat", Some(path(&feat)));

    let notes = main.join("notes.txt");
    queue_run(
        &rig,
        "child-dirty",
        &format!("please write {}", path(&notes)),
        path(&main),
    );
    queue_run(&rig, "child-clean", "just work", path(&clean));
    queue_run(&rig, "child-feat", "just work", path(&feat));
    let snapshot = snapshot_until(&rig, |s| {
        !events(s, SupervisorEventKind::Dirty).is_empty()
            && !events(s, SupervisorEventKind::Unmerged).is_empty()
    })
    .await;
    let [dirty] = events(&snapshot, SupervisorEventKind::Dirty)
        .try_into()
        .unwrap();
    assert_eq!(dirty.thread_id.as_deref(), Some("child-dirty"));
    assert_eq!(dirty.root_chat_id.as_deref(), Some("lead"));
    assert_eq!(
        dirty.label,
        "Thread \"Work child-dirty\" ended its turn with 1 uncommitted file in its checkout"
    );
    let [unmerged] = events(&snapshot, SupervisorEventKind::Unmerged)
        .try_into()
        .unwrap();
    assert_eq!(unmerged.thread_id.as_deref(), Some("child-feat"));
    assert_eq!(
        unmerged.label,
        "Thread \"Work child-feat\" left branch feat unmerged into main"
    );

    queue_run(&rig, "child-dirty", "just work", path(&main));
    wait_for(
        || {
            rig.prompts
                .lock()
                .unwrap()
                .iter()
                .filter(|p| *p == "just work")
                .count()
                == 3
        },
        "the second turn",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(3 * STALL_MS)).await;
    let snapshot = snapshot_until(&rig, |_| true).await;
    for kind in [SupervisorEventKind::Dirty, SupervisorEventKind::Unmerged] {
        assert_eq!(
            offered(&snapshot, kind).len(),
            1,
            "{kind:?} once per head sha"
        );
    }
    let clean_events: Vec<_> = snapshot
        .wakes
        .iter()
        .flat_map(|wake| wake.events.iter().chain(&wake.dropped))
        .filter(|event| event.thread_id.as_deref() == Some("child-clean"))
        .collect();
    assert!(
        clean_events.is_empty(),
        "a clean merged checkout: {clean_events:?}"
    );
    assert!(
        wakes_text(&rig).contains("with 1 uncommitted file in its checkout"),
        "{}",
        wakes_text(&rig)
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_limited_chat_idle_past_its_reset_and_a_lost_handback_wake_once_but_a_busy_one_never() {
    let rig = rig();
    watch_lead(&rig).await;
    child(&rig, "child-limited", None);
    child(&rig, "child-busy", None);

    queue_run_on(
        &rig,
        "child-busy",
        "please fan out",
        "/tmp",
        Some(HarnessId::Pi),
    );
    wait_for(
        || idle(&rig, "child-busy") && rig.core.sessions.holds_background_work("child-busy"),
        "a turn that ended with its sub-agent still running",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let now = chrono::Utc::now().timestamp_millis();
    for chat in ["child-limited", "child-busy"] {
        rig.core
            .resume_schedule
            .note_limit(chat, Some(now - 200), None, now);
    }

    let view = rig
        .core
        .doc_host
        .delegate_task(
            serde_json::from_value(
                serde_json::json!({ "ownerChatId": "lead", "task": "count the tests" }),
            )
            .unwrap(),
            Default::default(),
        )
        .await
        .expect("delegate");
    let delegated = view.task_id.clone();
    let delivered = || {
        rig.core
            .workspace
            .chat(&delegated)
            .unwrap()
            .and_then(|chat| chat.delegation)
            .is_some_and(|d| d.delivery == Some(zeron_proto::DelegationDelivery::Delivered))
    };
    wait_for(
        || delivered() && idle(&rig, "lead"),
        "the owner's handback turn",
    )
    .await;
    rig.core
        .workspace
        .update_chat_delegation(&delegated, |d| {
            d.delivery = Some(zeron_proto::DelegationDelivery::Claimed);
            true
        })
        .unwrap();

    let snapshot = snapshot_until(&rig, |s| {
        !events(s, SupervisorEventKind::LimitIdle).is_empty()
            && !events(s, SupervisorEventKind::LostWake).is_empty()
    })
    .await;
    let [limit] = events(&snapshot, SupervisorEventKind::LimitIdle)
        .try_into()
        .unwrap();
    assert_eq!(limit.thread_id.as_deref(), Some("child-limited"));
    assert_eq!(limit.root_chat_id.as_deref(), Some("lead"));
    let [lost] = events(&snapshot, SupervisorEventKind::LostWake)
        .try_into()
        .unwrap();
    assert_eq!(lost.thread_id.as_deref(), Some(delegated.as_str()));
    assert_eq!(lost.root_chat_id.as_deref(), Some("lead"));

    tokio::time::sleep(Duration::from_millis(3 * STALL_MS)).await;
    assert!(
        rig.core.sessions.holds_background_work("child-busy"),
        "the sub-agent still runs"
    );
    let snapshot = snapshot_until(&rig, |_| true).await;
    let limits = offered(&snapshot, SupervisorEventKind::LimitIdle);
    assert_eq!(
        limits.len(),
        1,
        "once, and never for the chat whose sub-agent runs: {limits:?}"
    );
    assert_eq!(offered(&snapshot, SupervisorEventKind::LostWake).len(), 1);
    let text = wakes_text(&rig);
    assert!(
        text.contains("Thread \"Work child-limited\" is still idle")
            && text.contains("after its usage limit reset"),
        "{text}"
    );
    assert!(
        text.contains("and its owner never got the result"),
        "{text}"
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_worktree_branch_overlapping_the_main_checkouts_dirty_files_names_the_file_and_its_writer()
 {
    let rig = rig();
    watch_lead(&rig).await;
    let main = repo(&rig, &["feat"]);
    let feat = rig.dir.path().join("feat");
    std::fs::write(feat.join("shared.rs"), "fn shared() { feat() }\n").unwrap();
    std::fs::write(feat.join("feature.rs"), "fn feature() {}\n").unwrap();
    git(&feat, &["add", "-A"]);
    git(&feat, &["commit", "-q", "-m", "feature"]);
    rig.core
        .workspace
        .create_chat(
            "peer",
            Some("space-s"),
            None,
            None,
            Some(path(&main).into()),
        )
        .unwrap();
    rig.core
        .workspace
        .rename_chat("peer", "Peer edits")
        .unwrap();
    child(&rig, "child-feat", Some(path(&feat)));

    let shared = main.join("shared.rs");
    queue_run(
        &rig,
        "peer",
        &format!("please write {}", path(&shared)),
        path(&main),
    );
    wait_for(
        || idle(&rig, "peer") && std::fs::read_to_string(&shared).unwrap() == "left behind\n",
        "the peer's uncommitted edit",
    )
    .await;
    queue_run(&rig, "child-feat", "just work", path(&feat));
    let snapshot = snapshot_until(&rig, |s| {
        !events(s, SupervisorEventKind::MergeBlocked).is_empty()
    })
    .await;
    let [blocked] = events(&snapshot, SupervisorEventKind::MergeBlocked)
        .try_into()
        .unwrap();
    assert_eq!(blocked.thread_id.as_deref(), Some("child-feat"));
    assert_eq!(blocked.root_chat_id.as_deref(), Some("lead"));
    assert_eq!(
        blocked.label,
        "Thread \"Work child-feat\" cannot merge feat into main: the main checkout holds uncommitted changes to 1 file it also changed"
    );
    wait_for(
        || wakes_text(&rig).contains("cannot merge feat"),
        "the wake text",
    )
    .await;
    let text = wakes_text(&rig);
    assert!(text.contains(": shared.rs\n"), "{text}");
    assert!(
        text.contains("Thread \"Peer edits\" (thread peer) changed them last in that checkout."),
        "{text}"
    );
    assert!(!text.contains("feature.rs"), "only the overlap: {text}");

    tokio::time::sleep(Duration::from_millis(3 * STALL_MS)).await;
    let snapshot = snapshot_until(&rig, |_| true).await;
    assert_eq!(
        offered(&snapshot, SupervisorEventKind::MergeBlocked).len(),
        1
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_watched_thread_wakes_each_time_it_fails_and_each_delegated_task_wakes_when_it_finishes()
{
    let rig = rig();
    watch_lead(&rig).await;
    child(&rig, "child-fail", None);
    for round in 1..=2 {
        queue_run(&rig, "child-fail", &format!("please fail {round}"), "/tmp");
        snapshot_until(&rig, |s| {
            offered(s, SupervisorEventKind::NodeFailed).len() == round
        })
        .await;
    }
    let failed = offered(
        &snapshot_until(&rig, |_| true).await,
        SupervisorEventKind::NodeFailed,
    );
    assert!(
        failed.iter().all(|event| {
            event.thread_id.as_deref() == Some("child-fail")
                && event.root_chat_id.as_deref() == Some("lead")
                && event.label == "Thread \"Work child-fail\" failed"
        }),
        "{failed:?}"
    );

    let mut delegated = Vec::new();
    for (round, task) in [(1, "count the tests"), (2, "count the docs")] {
        let view = rig
            .core
            .doc_host
            .delegate_task(
                serde_json::from_value(serde_json::json!({ "ownerChatId": "lead", "task": task }))
                    .unwrap(),
                Default::default(),
            )
            .await
            .expect("delegate");
        delegated.push(view.task_id);
        snapshot_until(&rig, |s| {
            offered(s, SupervisorEventKind::NodeFinished).len() == round
        })
        .await;
    }
    let finished = offered(
        &snapshot_until(&rig, |_| true).await,
        SupervisorEventKind::NodeFinished,
    );
    let mut threads: Vec<String> = finished
        .iter()
        .filter_map(|event| event.thread_id.clone())
        .collect();
    threads.sort();
    delegated.sort();
    assert_eq!(threads, delegated, "{finished:?}");
    assert!(
        finished
            .iter()
            .all(|event| event.label.ends_with("completed")
                && event.root_chat_id.as_deref() == Some("lead")),
        "{finished:?}"
    );
    rig.core.shutdown().await;
}
