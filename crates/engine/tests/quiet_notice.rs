//! A delegated child that works without a tool call or a word for the quiet
//! window wakes its owner once per quiet spell. Its own binary: the window is
//! an environment knob, and a short one would reach every other test's child.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::{
    AgentEvent, Compaction, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SteeringMode,
};

/// The first run streams what the test feeds it; every later run (the
/// owner's) answers at once.
struct FedHarness {
    feed: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<AgentEvent>>>,
}

fn done() -> AgentEvent {
    AgentEvent::Done {
        status: DoneStatus::Completed,
        result: None,
        error: None,
        session_id: Some("hs-fed".into()),
    }
}

#[async_trait]
impl Harness for FedHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Fed"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    fn compaction(&self) -> Compaction {
        Compaction::SlashText("/compact".into())
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        _request: RunRequest,
        _controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        let started = AgentEvent::SessionStarted {
            harness: HarnessId::Mock,
            model: "fed-1".into(),
            tools: vec![],
            cwd: "/tmp".into(),
            session_id: "hs-fed".into(),
            assistant_message_id: "a-fed".into(),
        };
        let fed = self.feed.lock().unwrap().take();
        let head = futures::stream::iter([Ok(started)]);
        Ok(match fed {
            Some(rx) => head
                .chain(futures::stream::unfold(rx, |mut rx| async move {
                    rx.recv().await.map(|event| (Ok(event), rx))
                }))
                .boxed(),
            None => head
                .chain(futures::stream::iter([
                    Ok(AgentEvent::TextDelta {
                        text: "noted".into(),
                    }),
                    Ok(done()),
                ]))
                .boxed(),
        })
    }
}

async fn wait_for<F: FnMut() -> bool>(mut predicate: F, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn notices(core: &EngineCore, child: &str) -> Vec<String> {
    core.doc_host
        .open("lead")
        .expect("open lead")
        .doc()
        .read_entries()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry.from_chat_id.as_deref() == Some(child))
        .map(|entry| {
            entry
                .parts
                .iter()
                .filter_map(|part| match part {
                    zeron_doc::MessagePart::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>()
        })
        .filter(|text| text.contains("has been quiet for"))
        .collect()
}

#[tokio::test]
async fn a_quiet_working_child_wakes_its_owner_once_per_quiet_spell_and_activity_clears_it() {
    // SAFETY: this binary's only test reads it.
    unsafe { std::env::set_var("ZERON_QUIET_NOTICE_MS", "400") };
    let (feed, rx) = tokio::sync::mpsc::unbounded_channel();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(FedHarness {
        feed: Mutex::new(Some(rx)),
    }));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    core.workspace
        .create_space("space-q", "dev-a", "/tmp", None, false)
        .expect("space");
    core.workspace
        .create_chat("lead", Some("space-q"), None, None, None)
        .expect("lead chat");
    let params: zeron_engine::doc_host::DelegateTaskParams =
        serde_json::from_value(serde_json::json!({ "ownerChatId": "lead", "task": "think hard" }))
            .unwrap();
    let child = core
        .doc_host
        .delegate_task(params, Default::default())
        .await
        .expect("delegate")
        .task_id;
    feed.send(AgentEvent::TextDelta {
        text: "Looking.".into(),
    })
    .unwrap();
    for _ in 0..6 {
        feed.send(AgentEvent::ReasoningDelta { text: "hmm".into() })
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    wait_for(
        || notices(&core, &child).len() == 1,
        "the first quiet notice",
    )
    .await;
    assert!(
        notices(&core, &child)[0].starts_with(&format!(
            "Delegated task {child} (\"think hard\") has been quiet for 0 minutes: no tool call and no text, only reasoning. It is still running."
        )),
        "{:?}",
        notices(&core, &child)
    );
    assert!(
        zeron_proto::transcript::wake_task_ids(&notices(&core, &child)[0]).is_none(),
        "a quiet notice is no finished turn"
    );
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(notices(&core, &child).len(), 1, "once per quiet spell");
    let marked = core
        .sessions
        .thread_activity(&child)
        .expect("a working turn");
    assert!(marked.quiet_since.is_some());

    feed.send(AgentEvent::TextDelta {
        text: " Writing the file now.".into(),
    })
    .unwrap();
    wait_for(
        || {
            core.sessions
                .thread_activity(&child)
                .is_some_and(|activity| activity.quiet_since.is_none())
        },
        "activity to clear the mark",
    )
    .await;
    wait_for(
        || notices(&core, &child).len() == 2,
        "a later quiet spell's notice",
    )
    .await;
    assert!(
        core.doc_host
            .task_status("lead", &child, false)
            .await
            .unwrap()
            .status
            == "running",
        "a notice stops nothing"
    );
    feed.send(done()).unwrap();
    wait_for(
        || core.sessions.thread_activity(&child).is_none(),
        "the turn's end to drop its activity",
    )
    .await;
    core.shutdown().await;
}
