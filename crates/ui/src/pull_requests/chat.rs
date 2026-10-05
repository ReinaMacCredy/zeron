use gpui::AnyElement;

use super::{PullRequestKey, PullRequestsPage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrChatScope {
    pub repository: Option<String>,
    pub pull_request: Option<PullRequestKey>,
}

impl PullRequestsPage {
    pub fn chat_scope(&self) -> PrChatScope {
        PrChatScope {
            repository: self.prefs.repository.clone(),
            pull_request: self.active.clone(),
        }
    }
}

pub(super) fn row_badge(_key: &PullRequestKey) -> Option<AnyElement> {
    None
}
