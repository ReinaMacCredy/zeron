use chrono::{DateTime, Utc};
use zeron_doc::SessionMessageEntry;
use zeron_doc::tree::TreeSource;
use zeron_proto::{Chat, ChatIndicator, HarnessId, Task, TaskStep};

use super::{Brand, Rollup};
use crate::state::AppState;

pub(crate) use zeron_doc::tree::NodeKind;

pub(crate) type TreeNode = zeron_doc::tree::TreeNode<Brand>;

pub(crate) struct Source<'a> {
    pub state: &'a AppState,
    pub now: DateTime<Utc>,
}

impl TreeSource for Source<'_> {
    type Brand = Brand;

    fn chats(&self) -> &[Chat] {
        &self.state.chats
    }

    fn tasks(&self) -> &[Task] {
        &self.state.tasks
    }

    fn steps(&self) -> &[TaskStep] {
        &self.state.task_steps
    }

    fn now(&self) -> DateTime<Utc> {
        self.now
    }

    fn indicator(&self, chat: &Chat) -> ChatIndicator {
        self.state.display_status_for(chat, self.now)
    }

    fn running_subagents(&self, chat_id: &str) -> usize {
        self.state.running_subagents_for(chat_id, self.now) as usize
    }

    fn started_at(&self, chat_id: &str) -> Option<DateTime<Utc>> {
        self.state
            .session_for(chat_id)
            .and_then(|session| session.started_at)
    }

    fn hidden(&self, chat_id: &str) -> bool {
        self.state.is_hidden_chat(chat_id)
    }

    fn brand(&self, harness: Option<HarnessId>, model: Option<&str>) -> Brand {
        crate::pickers::agent_brand_icon(harness, model)
    }

    fn rollup(&self, task: &Task) -> Rollup {
        super::rollup(self.state, task, self.now)
    }

    fn transcript(&self, chat_id: &str) -> Box<dyn Iterator<Item = &SessionMessageEntry> + '_> {
        Box::new(if self.state.selected_chat.as_deref() == Some(chat_id) {
            self.state.transcript.iter()
        } else {
            self.state.sub_transcript(chat_id).iter()
        })
    }
}

pub(crate) fn tree_rows(
    state: &AppState,
    root_chat_id: &str,
    native: bool,
    now: DateTime<Utc>,
) -> Vec<TreeNode> {
    zeron_doc::tree::tree_rows(&Source { state, now }, root_chat_id, native)
}
