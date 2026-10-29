use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use zeron_doc::SessionMessageEntry;
use zeron_doc::background::{agent_runs, agent_tree, latest_prompt, rows_missing, walked_docs};
use zeron_proto::background::{AgentRun, delegate_runs, shell_tasks};
use zeron_proto::{BackgroundTask, HarnessId, Session};
use zeron_rpc::methods;

use super::{SessionCore, SessionHandle, lock};
use crate::error::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundRoster {
    pub agents: Vec<AgentRun>,
    pub shells: Vec<BackgroundTask>,
    /// The chat's own harness: names a run without a model, and only
    /// Claude Code stops its own subagents and shells.
    pub harness: Option<HarnessId>,
}

pub(super) struct AgentWalk {
    entries: Vec<SessionMessageEntry>,
    loaded: bool,
    watch: Option<CancellationToken>,
}

#[derive(serde::Deserialize)]
struct OutputFrame {
    #[serde(default)]
    lines: Vec<String>,
}

/// The host's own session row: background shells and the compact plan never
/// reach the synced registry (`background_tasks` is host-local).
pub(super) fn start(core: &Arc<SessionCore>) -> Option<CancellationToken> {
    let client = core.client().ok()?;
    if let Some(demo) = client.demo() {
        core.set_session(demo.background_session(&core.chat_id));
        return None;
    }
    Some(super::host_watch::spawn(
        core,
        methods::WATCH_SESSIONS,
        serde_json::json!({}),
        |core, item| {
            if let Ok(sessions) = serde_json::from_value::<Vec<Session>>(item) {
                core.set_session(sessions.into_iter().find(|s| s.chat_id == core.chat_id));
            }
        },
    ))
}

impl SessionCore {
    fn set_session(&self, session: Option<Session>) {
        if lock(&self.state).session != session {
            self.update_state(|st| st.session = session);
            self.schedule_refresh();
        }
    }

    fn sync_walks(self: &Arc<Self>, docs: Vec<String>, agents: &[AgentRun]) -> bool {
        let demo = self.client().ok().and_then(|client| client.demo().cloned());
        let mut st = lock(&self.state);
        st.walks.retain(|id, walk| {
            let keep = docs.contains(id);
            if !keep && let Some(watch) = &walk.watch {
                watch.cancel();
            }
            keep
        });
        let mut read = false;
        for id in docs {
            if st.walks.contains_key(&id) {
                continue;
            }
            read |= demo.is_some();
            let walk = match &demo {
                Some(demo) => AgentWalk {
                    entries: demo.subagent_transcript(&id),
                    loaded: true,
                    watch: None,
                },
                None => AgentWalk {
                    entries: Vec::new(),
                    loaded: false,
                    watch: Some(self.watch_walk(&id)),
                },
            };
            st.walks.insert(id, walk);
        }
        if st.walks.values().all(|walk| walk.loaded) {
            let counted =
                zeron_proto::view::running_subagents(st.session.as_ref(), chrono::Utc::now());
            let short = rows_missing(agents, counted);
            match st.widen {
                None => {
                    st.widen = Some(short);
                    if short {
                        drop(st);
                        self.schedule_refresh();
                    }
                }
                Some(true) if !short => st.widen = Some(false),
                _ => {}
            }
        }
        read
    }

    fn watch_walk(self: &Arc<Self>, doc_id: &str) -> CancellationToken {
        let id = doc_id.to_owned();
        super::host_watch::spawn(
            self,
            methods::WATCH_DOC_MESSAGES,
            serde_json::json!({ "chatId": doc_id }),
            move |core, item| {
                let Ok(update) = serde_json::from_value::<zeron_doc::TranscriptUpdate>(item) else {
                    return;
                };
                let changed = {
                    let mut st = lock(&core.state);
                    let Some(walk) = st.walks.get_mut(&id) else {
                        return;
                    };
                    let first = !std::mem::replace(&mut walk.loaded, true);
                    let before = agent_runs(&walk.entries, 0);
                    let _ = zeron_doc::apply_transcript_frame(&mut walk.entries, update.frame);
                    first || before != agent_runs(&walk.entries, 0)
                };
                if changed {
                    core.schedule_refresh();
                }
            },
        )
    }

    pub(crate) fn drop_background_task(&self, task_id: &str) {
        self.update_state(|st| {
            if let Some(session) = st.session.as_mut() {
                session.background_tasks.retain(|task| task.id != task_id);
            }
        });
    }
}

impl SessionHandle {
    pub fn background_roster(&self) -> BackgroundRoster {
        let snapshot = self.snapshot();
        let transcript = snapshot.transcript();
        let entries = transcript.iter().map(|entry| entry.message.as_ref());
        let turn_start = latest_prompt(entries.clone());
        let tree = || {
            let st = lock(&self.core.state);
            let agents = agent_tree(entries.clone(), turn_start.unwrap_or(0), &|id| {
                st.walks.get(id).map(|walk| walk.entries.as_slice())
            });
            let walk = walked_docs(entries.clone(), &agents, st.widen == Some(true));
            (agents, walk)
        };
        let (mut agents, walk) = tree();
        if self.core.sync_walks(walk, &agents) {
            agents = tree().0;
        }
        let pending_review = super::review::pending_task(&lock(&self.core.state));
        let chat = self.core.client().ok().and_then(|client| {
            let (state, _) = client.workspace.state();
            agents.extend(delegate_runs(
                &state.chats,
                &self.core.chat_id,
                turn_start.map_or(0, |ix| transcript[ix].message.created_at),
                pending_review.as_deref(),
            ));
            client.workspace.chat(&self.core.chat_id)
        });
        BackgroundRoster {
            agents,
            shells: shell_tasks(lock(&self.core.state).session.as_ref()),
            harness: chat
                .and_then(|chat| chat.config)
                .map(|config| config.harness),
        }
    }

    pub fn shell_output(&self, task_id: &str) -> Vec<String> {
        match &lock(&self.core.state).shell_output {
            Some((id, lines)) if id == task_id => lines.clone(),
            _ => Vec::new(),
        }
    }

    pub fn watch_shell_output(&self, task_id: Option<String>) {
        if let Some(watch) = lock(&self.core.output_watch).take() {
            watch.cancel();
        }
        let Some(task_id) = task_id else {
            self.core.update_state(|st| st.shell_output = None);
            return;
        };
        let Ok(client) = self.core.client() else {
            return;
        };
        if let Some(demo) = client.demo() {
            let lines = demo.shell_output(&task_id);
            self.core
                .update_state(|st| st.shell_output = Some((task_id, lines)));
            return;
        }
        let Some(device_id) = client
            .workspace
            .chat(&self.core.chat_id)
            .map(|c| c.device_id)
        else {
            return;
        };
        let params = serde_json::json!({
            "chatId": self.core.chat_id,
            "taskId": task_id,
            "targetDeviceId": device_id,
        });
        let watch = super::host_watch::spawn(
            &self.core,
            methods::WATCH_BACKGROUND_OUTPUT,
            params,
            move |core, item| {
                if let Ok(frame) = serde_json::from_value::<OutputFrame>(item) {
                    let id = task_id.clone();
                    core.update_state(|st| st.shell_output = Some((id, frame.lines)));
                }
            },
        );
        *lock(&self.core.output_watch) = Some(watch);
    }

    /// A delegated run is its own chat, cancelled as a task; a subagent or
    /// shell is stopped by its harness.
    pub async fn stop_background_run(&self, task_id: &str, delegated: bool) -> Result<()> {
        let client = self.core.client()?;
        let device_id = self.composer().host.device_id.clone();
        let (method, params) = if delegated {
            (
                methods::TASK_CANCEL,
                serde_json::json!({ "chatId": self.core.chat_id, "taskId": task_id }),
            )
        } else {
            (
                methods::STOP_BACKGROUND_TASK,
                serde_json::json!({
                    "chatId": self.core.chat_id,
                    "taskId": task_id,
                    "targetDeviceId": device_id,
                }),
            )
        };
        client.host_rpc(&device_id, method, params).await?;
        Ok(())
    }
}
