use std::collections::HashSet;

use super::*;

const PENDING_CHILD: Duration = Duration::from_secs(10);

pub(super) struct PendingChild {
    parent: String,
    target: PendingTarget,
    until: std::time::Instant,
}

enum PendingTarget {
    Agent(String),
    Chat(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildKind {
    Agent,
    Fork,
    SideChat,
}

impl ChildKind {
    /// A child with only a creator is a thread an agent started.
    pub(crate) fn of(chat: &zeron_proto::Chat) -> Self {
        if chat.sidechat_source_chat_id.is_some() {
            Self::SideChat
        } else if chat.forked_from_chat_id.is_some() {
            Self::Fork
        } else {
            Self::Agent
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            Self::Agent => icons::BOT,
            Self::Fork => icons::FORK,
            Self::SideChat => icons::CHAT_ROUND_LINE,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::Fork => "Fork",
            Self::SideChat => "Side chat",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChildRef {
    Subagent {
        chat_id: String,
        doc_id: String,
        title: String,
        frozen: bool,
    },
    Chat(String),
}

impl ChildRef {
    pub(crate) fn of_summary(agent: &transcript::SubagentSummary) -> Self {
        if agent.delegated {
            return Self::Chat(agent.doc_id.clone());
        }
        Self::Subagent {
            chat_id: agent.chat_id.clone(),
            doc_id: agent.doc_id.clone(),
            title: agent.title.to_string(),
            frozen: agent.status != Some(zeron_doc::SubagentStatus::Running),
        }
    }
}

impl Shell {
    pub(crate) fn open_child(&mut self, child: ChildRef, cx: &mut Context<Self>) {
        match child {
            ChildRef::Subagent {
                chat_id,
                doc_id,
                title,
                frozen,
            } => self.add_subagent_surface(chat_id, doc_id, title, frozen, cx),
            ChildRef::Chat(chat_id) => {
                let Some(chat) = self
                    .state
                    .read(cx)
                    .chats
                    .iter()
                    .find(|chat| chat.id == chat_id)
                    .cloned()
                else {
                    if !self.active_chat.is_empty() {
                        self.pending_child = Some(PendingChild {
                            parent: self.active_chat.clone(),
                            target: PendingTarget::Chat(chat_id),
                            until: std::time::Instant::now() + PENDING_CHILD,
                        });
                    }
                    return;
                };
                if chat.sidechat_source_chat_id.is_some() {
                    self.open_sidechat_in_right_pane(&chat, None, SidechatStaged::default(), cx);
                } else {
                    let key = self.panel_key(cx);
                    self.open_side_chat(chat, key, cx);
                }
            }
        }
    }

    pub(super) fn child_kind(&self, surface: RightSurface, cx: &App) -> Option<ChildKind> {
        match surface {
            RightSurface::Subagent(_) => Some(ChildKind::Agent),
            RightSurface::Sidechat(_) => Some(ChildKind::SideChat),
            RightSurface::SideChat(id) => self.side_chats.get(&id).map(|tab| {
                if tab.fork.is_some() {
                    return ChildKind::Fork;
                }
                tab.state
                    .read(cx)
                    .selected_chat_row()
                    .map_or(ChildKind::SideChat, ChildKind::of)
            }),
            _ => None,
        }
    }

    pub(super) fn surface_child(&self, surface: RightSurface, cx: &App) -> Option<String> {
        match surface {
            RightSurface::Subagent(id) => self.subagent_tabs.get(&id).map(|tab| tab.doc_id.clone()),
            RightSurface::Sidechat(id) => {
                self.sidechat_tabs.get(&id).map(|tab| tab.chat_id.clone())
            }
            RightSurface::SideChat(id) => self
                .side_chats
                .get(&id)
                .and_then(|tab| tab.state.read(cx).selected_chat.clone()),
            _ => None,
        }
    }

    pub(super) fn open_children(&self, cx: &App) -> HashSet<String> {
        if !self.right_pane_open(cx) {
            return HashSet::new();
        }
        self.right_tabs
            .get(&self.panel_key(cx))
            .into_iter()
            .flatten()
            .filter_map(|surface| self.surface_child(*surface, cx))
            .collect()
    }

    pub(super) fn sync_open_children(&mut self, cx: &mut Context<Self>) {
        let open = self.open_children(cx);
        self.transcript
            .update(cx, |transcript, cx| transcript.set_open_children(open, cx));
    }

    /// Tray rows carry only the label; the engine matches background agents to spawns the same way.
    pub(super) fn open_agent_of(&mut self, chat_id: String, label: String, cx: &mut Context<Self>) {
        self.await_child(chat_id, PendingTarget::Agent(label), cx);
    }

    pub(crate) fn open_thread(&mut self, chat_id: String, cx: &mut Context<Self>) {
        match self.state.read(cx).sidecar_host(&chat_id, Utc::now()) {
            Some(parent) => self.await_child(parent, PendingTarget::Chat(chat_id), cx),
            None => self.open_chat(chat_id, cx),
        }
    }

    fn await_child(&mut self, parent: String, target: PendingTarget, cx: &mut Context<Self>) {
        if !matches!(self.route, Route::Chat) || self.active_chat != parent {
            self.open_chat(parent.clone(), cx);
        }
        self.pending_child = Some(PendingChild {
            parent,
            target,
            until: std::time::Instant::now() + PENDING_CHILD,
        });
        self.resolve_pending_child(cx);
    }

    pub(super) fn resolve_pending_child(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_child.as_ref() else {
            return;
        };
        if std::time::Instant::now() > pending.until
            || self.state.read(cx).selected_chat.as_deref() != Some(pending.parent.as_str())
        {
            self.pending_child = None;
            return;
        }
        if self.active_chat != pending.parent {
            return;
        }
        let child = match &pending.target {
            PendingTarget::Agent(label) => {
                let label = transcript::single_line(label);
                self.transcript
                    .read(cx)
                    .subagent_summaries(cx)
                    .into_iter()
                    .find(|agent| transcript::single_line(&agent.title) == label)
                    .map(|agent| ChildRef::of_summary(&agent))
            }
            PendingTarget::Chat(id) => self
                .state
                .read(cx)
                .chats
                .iter()
                .any(|chat| &chat.id == id)
                .then(|| ChildRef::Chat(id.clone())),
        };
        if let Some(child) = child {
            self.pending_child = None;
            self.open_child(child, cx);
        }
    }
}
