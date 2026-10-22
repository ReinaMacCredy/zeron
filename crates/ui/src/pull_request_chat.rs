use super::*;

pub use zeron_proto::pull_request::chat::{ContextChip, PrChatContext};

pub(crate) async fn find_checkout(
    engine: crate::state::EngineHandle,
    spaces: Vec<zeron_proto::Space>,
    device: String,
    repository: String,
    executor: gpui::BackgroundExecutor,
) -> Result<Option<zeron_proto::Space>, String> {
    let lookup = handoff::matching_checkout(&engine, spaces, &device, &repository);
    let deadline = executor.timer(std::time::Duration::from_secs(15));
    futures::pin_mut!(lookup, deadline);
    match futures::future::select(lookup, deadline).await {
        futures::future::Either::Left((result, _)) => result,
        futures::future::Either::Right(_) => {
            Err("Checking projects timed out. Check the device connection and try again.".into())
        }
    }
}

pub(crate) struct ChatDock {
    pub sheet: Option<AnyElement>,
    pub bar: Option<AnyElement>,
    pub orb: Orb,
    pub focus: Option<gpui::FocusHandle>,
    pub idle: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Orb {
    pub working: bool,
    pub unread: usize,
    pub draft: bool,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = shell, no_json)]
pub struct OpenPrChat;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ChatRevision {
    url: String,
    fetched: Option<Instant>,
    comments: usize,
    diff: Option<usize>,
    settled: bool,
}

impl PullRequestDetailPage {
    pub(crate) fn chat_context(&self) -> Option<PrChatContext> {
        PrChatContext::new(
            &self.url,
            self.target.clone(),
            self.detail.clone(),
            self.diff.as_ref().map(|diff| diff.patch.as_str()),
        )
    }

    pub(crate) fn chat_revision(&self) -> ChatRevision {
        ChatRevision {
            url: self.url.clone(),
            fetched: self.fetched,
            comments: self
                .detail
                .as_ref()
                .map_or(0, |detail| detail.activity_comments().count()),
            diff: self.diff.as_ref().map(|diff| Arc::as_ptr(diff) as usize),
            settled: self.chat_settled(),
        }
    }

    pub(crate) fn chat_settled(&self) -> bool {
        self.detail.is_some() || self.error.is_some()
    }

    pub(crate) fn set_chat_dock(&mut self, dock: Option<ChatDock>) {
        self.chat_dock = dock;
    }

    pub(crate) fn chat_expanded(&self) -> bool {
        self.chat_expanded
    }

    pub(crate) fn set_chat_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        if self.chat_expanded != expanded {
            self.chat_expanded = expanded;
            cx.notify();
        }
    }

    pub(super) fn chat_docked(&self) -> bool {
        self.chat_dock
            .as_ref()
            .is_some_and(|dock| dock.bar.is_some())
    }

    pub(super) fn chat_lift(&self) -> f32 {
        if self.chat_docked() {
            self.chat_float.height() + 16.0
        } else {
            0.0
        }
    }

    pub(crate) fn parked_focus(&self, window: &Window) -> bool {
        self.content_focus.is_focused(window)
            || self.tree_focus.is_focused(window)
            || self.files_focus.is_focused(window)
            || self.tab_focus.iter().any(|focus| focus.is_focused(window))
    }

    pub(crate) fn reading_focus(&self, window: &Window) -> bool {
        self.content_focus.is_focused(window)
            || self.tab_focus.iter().any(|focus| focus.is_focused(window))
    }

    pub(crate) fn overlay_open(&self) -> bool {
        self.actions.overlay_open()
            || self.files_expanded
            || self.image_preview.is_some()
            || self.image_task.is_some()
            || self.image_failed.is_some()
    }

    pub(super) fn park_hidden_chat_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        let hidden = !self.chat_docked()
            && self
                .chat_dock
                .as_ref()
                .and_then(|dock| dock.focus.as_ref())
                .is_some_and(|focus| focus.contains_focused(window, cx));
        if hidden {
            window.focus(&self.content_focus, cx);
        }
    }

    pub(crate) fn chat_orb(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let orb = self
            .chat_dock
            .as_ref()
            .map(|dock| dock.orb)
            .unwrap_or_default();
        let label = match orb.unread {
            0 => "Ask AI about this pull request".to_owned(),
            1 => "Ask AI about this pull request, 1 new reply".to_owned(),
            n => format!("Ask AI about this pull request, {n} new replies"),
        };
        let page = cx.weak_entity();
        crate::dock_orb::DockOrb {
            id: "pr-chat-orb",
            glyph: crate::icons::SPARK,
            tint: crate::icons::claude_brand(),
            label: label.into(),
            tooltip: "Ask AI  /",
            marks: crate::dock_orb::Marks {
                working: orb.working,
                count: orb.unread,
                dot: orb.draft,
            },
            open: self.chat_expanded,
        }
        .render(
            theme,
            &self.chat_float,
            |window, cx| window.dispatch_action(Box::new(OpenPrChat), cx),
            move |_, cx| {
                page.update(cx, |page, cx| page.set_chat_expanded(false, cx))
                    .ok();
            },
            cx,
        )
    }

    pub(super) fn chat_dock(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dock = self.chat_dock.take()?;
        let bar = dock.bar?;
        let idle = dock.idle;
        Some(
            div()
                .id("pr-chat-dock")
                .debug_selector(|| "pr-chat-dock".into())
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(if self.tab == Tab::Activity {
                    8.0
                } else {
                    self.nav_clearance()
                }))
                .child(
                    widgets::page_column().pt_0().pb_0().child(
                        crate::dock_orb::float(
                            "pr-chat-rise",
                            &self.chat_float,
                            cx.entity_id(),
                            cx.listener(move |page, _, _, cx| {
                                if idle {
                                    page.set_chat_expanded(false, cx);
                                }
                            }),
                            bar,
                        )
                        .children(dock.sheet.map(|sheet| {
                            div()
                                .absolute()
                                .bottom_full()
                                .left_0()
                                .right_0()
                                .pb(px(8.0))
                                .child(sheet)
                        })),
                    ),
                )
                .into_any_element(),
        )
    }
}
