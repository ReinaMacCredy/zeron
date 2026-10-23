use super::*;
use crate::settings::SINCE_INTRO_ID;
use crate::tasks::{SampleRow, StatusTone, StripTree, TaskCenter, TreeStatus, tree_glyph};
use std::time::Instant;
use zeron_proto::mission::{INTRO_DISMISS, INTRO_NEXT, INTRO_PAGES, INTRO_TRY};

const PAGE_IN: motion::MotionSpec = motion::MotionSpec::new(250, motion::EASE_OUT);
const PAGE_SHIFT: f32 = 8.0;
pub(super) const VISUAL_HEIGHT: f32 = 172.0;
const TREE_PAGE: usize = 2;
const TEXT_HEIGHT: f32 = 23.0 + 8.0 + 4.0 * 21.0;

pub(super) const TASKS_HOME: &str = "tasks-home";
pub(super) const MISSION_TABS: &str = "mission-tabs";
pub(super) const SIDEBAR_MISSION: &str = "sidebar-mission";
pub(super) const SUPERVISOR_CONVERSATION: &str = "supervisor-conversation";

struct Page {
    id: &'static str,
    title: &'static str,
    body: &'static str,
}

const MISSION_PAGES: [Page; 4] = [
    mission_page(0),
    mission_page(1),
    mission_page(2),
    mission_page(3),
];

const fn mission_page(ix: usize) -> Page {
    Page {
        id: "",
        title: INTRO_PAGES[ix].title,
        body: INTRO_PAGES[ix].body,
    }
}

const SINCE_PAGES: [Page; 4] = [
    Page {
        id: TASKS_HOME,
        title: "Tasks is home for missions",
        body: "Mission Control moved into Tasks. Missions, agent trees and tasks sit in one list, and the Missions, Trees and Needs you chips narrow it.",
    },
    Page {
        id: MISSION_TABS,
        title: "A mission\u{2019}s tabs travel together",
        body: "Steps you open join the mission\u{2019}s group in the titlebar. Click its title to fold the group to the Lead; hover lists the rest.",
    },
    Page {
        id: SIDEBAR_MISSION,
        title: "Missions on the sidebar",
        body: "A mission is one row with a progress ring. Its live steps sit under it and the rest fold into N more steps.",
    },
    Page {
        id: SUPERVISOR_CONVERSATION,
        title: "Ask the Supervisor about any thread",
        body: "Mention a thread in the Supervisor\u{2019}s Conversation and it starts watching that tree. Watched trees show an eye.",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Deck {
    Mission,
    Since,
}

impl Deck {
    fn pages(self) -> &'static [Page] {
        match self {
            Self::Mission => &MISSION_PAGES,
            Self::Since => &SINCE_PAGES,
        }
    }

    fn cta(self) -> &'static str {
        match self {
            Self::Mission => INTRO_TRY,
            Self::Since => "Open Tasks",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Intro {
    Open {
        deck: Deck,
        pages: Vec<usize>,
        page: usize,
        forward: bool,
        since: Option<Instant>,
        shown: Instant,
        tree: Option<Entity<StripTree>>,
    },
    Closed,
}

impl Intro {
    pub(super) fn at_boot() -> Self {
        match std::env::var("ZERON_OPEN_DIALOG").as_deref() {
            Ok("mission-intro") => Self::open(Deck::Mission, (0..MISSION_PAGES.len()).collect()),
            Ok("new-since") => Self::open(Deck::Since, (0..SINCE_PAGES.len()).collect()),
            _ => Self::Closed,
        }
    }

    fn open(deck: Deck, pages: Vec<usize>) -> Self {
        Self::Open {
            deck,
            pages,
            page: 0,
            forward: true,
            since: None,
            shown: motion::now(),
            tree: None,
        }
    }

    fn page(&self) -> Option<(Deck, &'static Page, usize, usize)> {
        let Self::Open {
            deck, pages, page, ..
        } = self
        else {
            return None;
        };
        Some((*deck, &deck.pages()[pages[*page]], *page, pages.len()))
    }
}

pub(super) fn since_pages(seen: &[String]) -> Vec<usize> {
    if seen.iter().any(|id| id == SINCE_INTRO_ID) {
        return Vec::new();
    }
    (0..SINCE_PAGES.len())
        .filter(|&ix| !seen.iter().any(|id| id == SINCE_PAGES[ix].id))
        .collect()
}

fn mission_started(state: &AppState) -> bool {
    !zeron_proto::mission::intro_due(&state.tasks)
}

fn mission_tab_open(state: &AppState, open_tabs: Option<&[String]>) -> bool {
    let members = crate::tasks::tab_members(state);
    open_tabs.is_some_and(|tabs| tabs.iter().any(|id| members.contains_key(id)))
}

impl Shell {
    pub(super) fn resolve_intros(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if !state.tasks_synced {
            return;
        }
        let chats_synced = state.chats_synced;
        if !self.settings.mission_intro_acknowledged && mission_started(state) {
            self.settings.mission_intro_acknowledged = true;
            self.schedule_save(cx);
        }
        if !self.since_pending || !chats_synced {
            return;
        }
        self.since_pending = false;
        if mission_tab_open(self.state.read(cx), self.settings.open_tabs.as_deref()) {
            self.mark_onboarded(MISSION_TABS, cx);
        }
        let pages = since_pages(&self.settings.onboarding_seen);
        if pages.is_empty() {
            self.mark_onboarded(SINCE_INTRO_ID, cx);
        } else if self.intro == Intro::Closed {
            self.intro = Intro::open(Deck::Since, pages);
            self.appshot_welcome = false;
            cx.notify();
        }
    }

    pub(super) fn mark_onboarded(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.settings.onboarding_seen.iter().any(|seen| seen == id) {
            return;
        }
        self.settings.onboarding_seen.push(id.to_owned());
        self.schedule_save(cx);
    }

    pub(super) fn on_plan_first(&mut self, cx: &mut Context<Self>) {
        if self.settings.mission_intro_acknowledged || self.intro != Intro::Closed {
            return;
        }
        if mission_started(self.state.read(cx)) {
            self.settings.mission_intro_acknowledged = true;
            self.schedule_save(cx);
            return;
        }
        self.intro = Intro::open(Deck::Mission, (0..MISSION_PAGES.len()).collect());
        cx.notify();
    }

    pub(super) fn intro_visible(&self) -> bool {
        matches!(self.intro, Intro::Open { .. })
            && !self.appshot_welcome
            && self.whats_new_dialog.is_none()
            && !self.sync_flow.has_visible_overlay()
    }

    pub(super) fn reopen_intro(&mut self, deck: Deck, cx: &mut Context<Self>) {
        self.intro = Intro::open(deck, (0..deck.pages().len()).collect());
        cx.notify();
    }

    pub(super) fn close_intro(&mut self, cx: &mut Context<Self>) {
        let Intro::Open { deck, .. } = std::mem::replace(&mut self.intro, Intro::Closed) else {
            return;
        };
        match deck {
            Deck::Mission if !self.settings.mission_intro_acknowledged => {
                self.settings.mission_intro_acknowledged = true;
                self.schedule_save(cx);
            }
            Deck::Mission => {}
            Deck::Since => self.mark_onboarded(SINCE_INTRO_ID, cx),
        }
        cx.notify();
    }

    pub(super) fn page_intro(&mut self, delta: isize, cx: &mut Context<Self>) {
        let reduced = motion::reduced_motion(cx);
        let Intro::Open {
            pages,
            page,
            forward,
            since,
            shown,
            tree,
            ..
        } = &mut self.intro
        else {
            return;
        };
        let next = page.saturating_add_signed(delta).min(pages.len() - 1);
        if next == *page {
            return;
        }
        *forward = next > *page;
        *since = (!reduced).then(motion::now);
        *shown = motion::now();
        *page = next;
        *tree = None;
        cx.notify();
    }

    fn finish_intro(&mut self, deck: Deck, cx: &mut Context<Self>) {
        self.close_intro(cx);
        match deck {
            Deck::Mission => self.new_mission(cx),
            Deck::Since => self.open_tasks(cx),
        }
    }

    pub(super) fn new_mission(&mut self, cx: &mut Context<Self>) {
        self.open_new_session(None, cx);
        self.composer.update(cx, |composer, cx| {
            composer.mission_pending = true;
            composer.focus_pending = true;
            cx.notify();
        });
    }

    fn mission_intro_tree(&mut self, cx: &mut Context<Self>) -> Entity<StripTree> {
        if let Intro::Open {
            tree: Some(tree), ..
        } = &self.intro
        {
            return tree.clone();
        }
        let state = self.state.clone();
        let tree = cx.new(|cx| StripTree::sample(state, sample_rows(), cx));
        if let Intro::Open { tree: slot, .. } = &mut self.intro {
            *slot = Some(tree.clone());
        }
        tree
    }

    pub(super) fn render_intro(
        &mut self,
        viewport: gpui::Size<Pixels>,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((deck, content, page, count)) = self.intro.page() else {
            return div().into_any_element();
        };
        let Intro::Open {
            forward,
            since,
            shown,
            ..
        } = self.intro
        else {
            return div().into_any_element();
        };
        let visual = match (deck, content.id) {
            (Deck::Mission, _) if page == TREE_PAGE => div()
                .size_full()
                .pt(px(8.0))
                .child(self.mission_intro_tree(cx))
                .into_any_element(),
            (Deck::Mission, _) => mission_visual(page, theme),
            (Deck::Since, id) => {
                let clock = loop_clock(shown, cx);
                if clock.is_some() {
                    window.request_animation_frame();
                }
                since_visual(id, theme, clock)
            }
        };
        let progress = since.map_or(1.0, |since| {
            let total = PAGE_IN.total().as_secs_f32() * motion::speed_scale();
            let raw = motion::since(since).as_secs_f32() / total.max(f32::EPSILON);
            if raw < 1.0 {
                window.request_animation_frame();
            }
            PAGE_IN.progress(raw)
        });
        let shift = (1.0 - progress) * PAGE_SHIFT * if forward { 1.0 } else { -1.0 };
        let last = page + 1 == count;
        let body = div()
            .relative()
            .left(px(shift))
            .opacity(progress)
            .child(
                div()
                    .mb(px(20.0))
                    .h(px(VISUAL_HEIGHT))
                    .overflow_hidden()
                    .rounded(px(14.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(crate::theme::ink(0.04))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(crate::typography::ui_rems(12.5))
                    .text_color(theme.text_muted)
                    .child(visual),
            )
            .child(
                div()
                    .min_h(px(TEXT_HEIGHT))
                    .child(popover::welcome_title(theme, content.title))
                    .child(popover::welcome_body(theme, content.body)),
            );
        let dots = div()
            .id("intro-pages")
            .role(gpui::Role::Group)
            .aria_label(SharedString::from(format!("Page {} of {count}", page + 1)))
            .flex_1()
            .flex()
            .items_center()
            .gap(px(5.0))
            .when(count > 1, |el| {
                el.children((0..count).map(|dot| {
                    div().size(px(6.0)).rounded_full().bg(if dot == page {
                        theme.text
                    } else {
                        crate::theme::ink(0.18)
                    })
                }))
            });
        let next = if last {
            popover::welcome_action(theme, "intro-finish", deck.cta(), true)
                .on_click(cx.listener(move |this, _, _, cx| this.finish_intro(deck, cx)))
        } else {
            popover::welcome_action(theme, "intro-next", INTRO_NEXT, true)
                .on_click(cx.listener(|this, _, _, cx| this.page_intro(1, cx)))
        };
        let card = popover::welcome_sheet(theme, "intro-card", content.title)
            .child(body)
            .child(
                div()
                    .mt(px(14.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(dots)
                    .child(
                        popover::welcome_action(theme, "intro-dismiss", INTRO_DISMISS, false)
                            .on_click(cx.listener(|this, _, _, cx| this.close_intro(cx))),
                    )
                    .child(next),
            )
            .into_any_element();
        popover::modal_glass("intro-sheet", viewport, card, 20.0)
    }
}

fn intro_chip(theme: &Theme, label: &'static str, icon: Option<&'static str>) -> gpui::Div {
    div()
        .h(px(22.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .rounded_full()
        .border_1()
        .border_color(theme.accent.opacity(0.5))
        .bg(theme.accent.opacity(0.16))
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text)
        .when_some(icon, |el, path| {
            el.child(icons::icon(path).size(px(12.0)).text_color(theme.text))
        })
        .child(label)
}

fn brand(model: &str) -> Option<(&'static str, Option<gpui::Hsla>)> {
    Some(crate::pickers::agent_brand_icon(None, Some(model)))
}

fn sample_rows() -> Vec<SampleRow> {
    let row = |title, status, word, brand, secs| SampleRow {
        title,
        status,
        word,
        brand,
        secs,
    };
    vec![
        row(
            "Watching 1 step",
            TreeStatus::Running,
            "Working",
            brand("claude-opus"),
            Some(95),
        ),
        row(
            "Scaffold the app",
            TreeStatus::Done,
            "Done",
            brand("gpt-5"),
            None,
        ),
        row(
            "Product grid with filters",
            TreeStatus::Running,
            "Working",
            brand("claude-sonnet"),
            Some(42),
        ),
        row(
            "Cart with persistence",
            TreeStatus::Waiting,
            "Needs you",
            brand("gpt-5"),
            Some(18),
        ),
        row("Final check", TreeStatus::Queued, "Planned", None, None),
    ]
}

fn read_chip(theme: &Theme, model: &str, title: &'static str) -> gpui::Div {
    let (_, _, done) = chat_status_tone(zeron_proto::ChatIndicator::Completed, theme);
    div()
        .h(px(24.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .rounded_full()
        .bg(theme.wash(0.07))
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text)
        .children(brand(model).map(|(mark, tint)| {
            icons::icon(mark)
                .size(px(12.0))
                .text_color(tint.unwrap_or(theme.text_muted))
        }))
        .child(div().size(px(6.0)).rounded_full().bg(done))
        .child(title)
}

fn mini_solid(theme: &Theme, label: &'static str) -> gpui::Div {
    div()
        .flex_none()
        .h(px(22.0))
        .px(px(9.0))
        .flex()
        .items_center()
        .rounded_full()
        .bg(theme.solid)
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.on_solid)
        .child(label)
}

fn plan_card(theme: &Theme, number: usize, title: &'static str) -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .flex_col()
        .pt(px(9.0))
        .px(px(10.0))
        .pb(px(7.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface_card)
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(8.0))
                .child(TaskCenter::avatar(None, theme).mt(px(1.0)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .flex()
                                .gap(px(4.0))
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(theme.text_muted)
                                .child(
                                    div()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(theme.text)
                                        .child(format!("Step {number}")),
                                )
                                .child("\u{b7} Not started"),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(crate::typography::ui_rems(13.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(title),
                        ),
                )
                .child(TaskCenter::chip(TreeStatus::Queued, "Planned", true, theme)),
        )
        .child(
            div().mt(px(5.0)).ml(px(30.0)).flex().justify_end().child(
                div()
                    .h(px(20.0))
                    .pl(px(8.0))
                    .pr(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_muted)
                    .child("Spec")
                    .child(
                        icons::icon(icons::LUCIDE_CHEVRON_RIGHT)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    ),
            ),
        )
}

fn plan_visual(theme: &Theme) -> AnyElement {
    let waiting = TreeStatus::Waiting.tone(theme);
    let pill = div()
        .flex_none()
        .h(px(22.0))
        .pl(px(7.0))
        .pr(px(9.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded_full()
        .bg(waiting.opacity(0.15))
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(waiting)
        .child(tree_glyph(TreeStatus::Waiting, 11.0, waiting, true))
        .child("Needs you \u{b7} plan review");
    let track = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(div().flex_1().flex().gap(px(3.0)).children((0..3).map(|_| {
            div()
                .flex_1()
                .h(px(5.0))
                .rounded(px(2.5))
                .bg(theme.wash(0.10))
        })))
        .child(
            div()
                .flex()
                .gap(px(3.0))
                .text_size(crate::typography::ui_rems(11.0))
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child("0/3"),
                )
                .child("done"),
        );
    div()
        .size_full()
        .px(px(12.0))
        .pt(px(14.0))
        .flex()
        .flex_col()
        .gap(px(9.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(crate::typography::ui_rems(12.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child("Ship the shop"),
                )
                .child(pill)
                .child(mini_solid(theme, "Approve and run")),
        )
        .child(track)
        .child(plan_card(theme, 1, "Scaffold the app"))
        .child(plan_card(theme, 2, "Product grid with filters"))
        .into_any_element()
}

fn sheet_row(
    theme: &Theme,
    question: &'static str,
    options: [&'static str; 2],
    picked: Option<usize>,
) -> gpui::Div {
    let segments = div()
        .flex_none()
        .flex()
        .items_center()
        .h(px(22.0))
        .p(px(2.0))
        .rounded(px(7.0))
        .bg(theme.wash(0.06))
        .border_1()
        .border_color(theme.border)
        .children(options.into_iter().enumerate().map(|(ix, option)| {
            let chosen = picked == Some(ix);
            div()
                .h(px(16.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .rounded(px(5.0))
                .text_size(crate::typography::ui_rems(11.5))
                .when(chosen, |el| {
                    el.bg(theme.solid)
                        .text_color(theme.on_solid)
                        .font_weight(gpui::FontWeight::MEDIUM)
                })
                .when(!chosen, |el| el.text_color(theme.text))
                .child(option)
        }));
    div()
        .h(px(34.0))
        .pl(px(10.0))
        .pr(px(8.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .border_t_1()
        .border_color(crate::settings::widgets::row_divider(theme))
        .child(div().size(px(6.0)).rounded_full().bg(theme.warning))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(crate::typography::ui_rems(12.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text)
                .child(question),
        )
        .child(segments)
}

fn supervisor_visual(theme: &Theme) -> AnyElement {
    let (_, _, watching) = chat_status_tone(zeron_proto::ChatIndicator::Completed, theme);
    let (_, _, working) = chat_status_tone(zeron_proto::ChatIndicator::Working, theme);
    let status = div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text_muted)
        .child(div().size(px(7.0)).rounded_full().bg(watching))
        .child(
            div()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(watching)
                .child("Watching"),
        )
        .child("1 mission \u{b7} 2 of 12 wakes this hour");
    let header = div()
        .h(px(30.0))
        .pl(px(10.0))
        .pr(px(10.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .text_size(crate::typography::ui_rems(11.5))
        .child(crate::loaders::upload_progress_arc(
            50,
            12.0,
            theme.text_faint.opacity(0.35),
            working,
        ))
        .child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child("Ship the shop"),
        )
        .child(div().text_color(theme.warning).child("2 to answer"));
    let footer = div()
        .h(px(34.0))
        .pl(px(10.0))
        .pr(px(6.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .border_t_1()
        .border_color(crate::settings::widgets::row_divider(theme))
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text_faint)
        .child(div().flex_1().child("1 of 2 picked"))
        .child(
            div()
                .h(px(22.0))
                .px(px(9.0))
                .flex()
                .items_center()
                .rounded(px(7.0))
                .bg(theme.solid)
                .text_color(theme.on_solid)
                .font_weight(gpui::FontWeight::MEDIUM)
                .child("Send 1 to the Lead"),
        );
    div()
        .size_full()
        .px(px(14.0))
        .pt(px(12.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(status)
        .child(
            div()
                .rounded(px(10.0))
                .border_1()
                .border_color(theme.border)
                .bg(crate::settings::widgets::block_fill(theme))
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(header)
                .child(sheet_row(
                    theme,
                    "Use a Stripe test account?",
                    ["Approve", "Use the mock"],
                    Some(0),
                ))
                .child(sheet_row(
                    theme,
                    "Ship on Friday?",
                    ["Yes", "Next week"],
                    None,
                ))
                .child(footer),
        )
        .into_any_element()
}

fn mission_visual(page: usize, theme: &Theme) -> AnyElement {
    match page {
        0 => div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(intro_chip(
                        theme,
                        "Mission Control",
                        Some(icons::MISSION_CONTROL),
                    ))
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(10.5))
                            .text_color(theme.text_faint)
                            .child("\u{21e7}Tab"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().mr(px(4.0)).child("Reads"))
                    .child(read_chip(theme, "claude-opus", "Checkout notes"))
                    .child(read_chip(theme, "gpt-5", "Pricing research")),
            )
            .into_any_element(),
        1 => plan_visual(theme),
        _ => supervisor_visual(theme),
    }
}

fn since_row(
    theme: &Theme,
    lead: AnyElement,
    title: &'static str,
    meta: Option<&'static str>,
) -> gpui::Div {
    div()
        .h(px(28.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .rounded(px(8.0))
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text)
        .child(lead)
        .child(div().flex_1().min_w_0().truncate().child(title))
        .children(meta.map(|meta| {
            div()
                .flex_none()
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_faint)
                .child(meta)
        }))
}

fn ring(theme: &Theme, percent: u8) -> AnyElement {
    let (_, _, working) = chat_status_tone(zeron_proto::ChatIndicator::Working, theme);
    crate::loaders::upload_progress_arc(percent, 12.0, theme.text_faint.opacity(0.35), working)
}

fn running(theme: &Theme, reduced: bool) -> AnyElement {
    let (_, _, working) = chat_status_tone(zeron_proto::ChatIndicator::Working, theme);
    tree_glyph(TreeStatus::Running, 12.0, working, reduced)
}

fn ramp(t: f32, start: f32, ms: f32, curve: motion::CubicBezier) -> f32 {
    curve.eval(((t - start) * 1000.0 / ms).clamp(0.0, 1.0))
}

fn rise(el: gpui::Div, t: f32, start: f32) -> gpui::Div {
    let p = ramp(t, start, 220.0, motion::EASE_OUT);
    el.relative().top(px((1.0 - p) * 6.0)).opacity(p)
}

fn task_box(theme: &Theme) -> AnyElement {
    div()
        .flex_none()
        .size(px(12.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(theme.border_strong)
        .into_any_element()
}

fn filter_chip(theme: &Theme, label: &'static str, count: &'static str, on: bool) -> gpui::Div {
    div()
        .flex_none()
        .h(px(22.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(if on {
            theme.border_strong
        } else {
            theme.border
        })
        .when(on, |el| el.bg(theme.wash(0.06)))
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(if on { theme.text } else { theme.text_muted })
        .child(label)
        .child(div().text_color(theme.text_faint).child(count))
}

fn tasks_visual(theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    const CHIPS: [(&str, &str); 4] = [
        ("All", "3"),
        ("Missions", "1"),
        ("Trees", "1"),
        ("Needs you", "0"),
    ];
    const STARTS: [f32; 4] = [0.0, 2.0, 3.4, 4.8];
    let filter = STARTS.iter().rposition(|start| t >= *start).unwrap_or(0);
    let start = STARTS[filter];
    let rows: &[usize] = match filter {
        0 => &[0, 1, 2],
        1 => &[0],
        2 => &[1],
        _ => &[],
    };
    let row = |ix: usize| match ix {
        0 => since_row(theme, ring(theme, 80), "Ship the shop", Some("4/5")),
        1 => since_row(
            theme,
            running(theme, reduced),
            "Fix tab strip jitter",
            Some("tree \u{b7} 3"),
        ),
        _ => since_row(
            theme,
            task_box(theme),
            "Write the release notes",
            Some("task"),
        ),
    };
    div()
        .size_full()
        .px(px(14.0))
        .pt(px(14.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div().flex().gap(px(6.0)).mb(px(2.0)).children(
                CHIPS
                    .iter()
                    .enumerate()
                    .map(|(ix, (label, count))| filter_chip(theme, label, count, ix == filter)),
            ),
        )
        .children(
            rows.iter()
                .enumerate()
                .map(|(n, &ix)| rise(row(ix), t, start + n as f32 * 0.07)),
        )
        .when(rows.is_empty(), |el| {
            el.child(rise(
                div()
                    .h(px(28.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_faint)
                    .child("Nothing needs you"),
                t,
                start,
            ))
        })
        .into_any_element()
}

fn tabs_visual(theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    let tone = motion::mix(theme.accent, theme.text, 0.2);
    let fold =
        ramp(t, 1.6, 240.0, motion::EASE_OUT) * (1.0 - ramp(t, 4.0, 240.0, motion::EASE_OUT));
    let hover = ramp(t, 2.4, 150.0, motion::EASE) * (1.0 - ramp(t, 3.6, 120.0, motion::EASE));
    let pressed = (1.45..1.65).contains(&t) || (3.85..4.05).contains(&t);
    let tab = |label: &'static str, selected: bool, width: f32, open: f32| {
        div()
            .flex_none()
            .w(px(width * open))
            .ml(px(2.0 * open))
            .opacity(open)
            .when(open < 1.0, |el| el.overflow_hidden())
            .child(
                div()
                    .w(px(width))
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.0))
                    .text_size(px(12.0))
                    .whitespace_nowrap()
                    .text_color(if selected {
                        theme.text
                    } else {
                        theme.text_muted
                    })
                    .when(selected, |el| {
                        el.bg(crate::theme::glass_selected_bg())
                            .shadow(crate::theme::glass_selected_shadows())
                    })
                    .child(label),
            )
    };
    let title = div()
        .flex_none()
        .h(px(26.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded(px(8.0))
        .when(pressed, |el| el.bg(theme.wash(0.1)))
        .when(!pressed && hover > 0.0, |el| {
            el.bg(theme.wash(0.06 * hover))
        })
        .text_size(px(12.0))
        .text_color(tone)
        .child(
            icons::icon(icons::LUCIDE_FOLDER)
                .size(px(12.0))
                .text_color(tone),
        )
        .child("Ship the shop");
    let menu = div()
        .w(px(170.0))
        .p(px(4.0))
        .flex()
        .flex_col()
        .gap(px(1.0))
        .rounded(px(9.0))
        .border_1()
        .border_color(theme.border)
        .bg(crate::settings::widgets::block_fill(theme))
        .relative()
        .top(px((1.0 - hover) * -4.0))
        .opacity(hover)
        .children(
            ["Step 2", "Step 3"]
                .map(|label| since_row(theme, running(theme, reduced), label, None).h(px(24.0))),
        );
    let caption = if (2.3..3.8).contains(&t) {
        "Hover the title to list the rest"
    } else if (3.8..4.4).contains(&t) {
        "Click it again to unfold"
    } else {
        "Click the title to fold the group to the Lead"
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.0))
        .child(
            div().w(px(300.0)).flex().child(rise(
                div()
                    .flex()
                    .items_center()
                    .pr(px(3.0))
                    .rounded(px(9.0))
                    .border_1()
                    .border_color(tone.opacity(0.45))
                    .child(title)
                    .child(tab("Lead", true, 46.0, 1.0))
                    .child(tab("Step 2", false, 58.0, 1.0 - fold))
                    .child(tab("Step 3", false, 58.0, 1.0 - fold)),
                t,
                0.0,
            )),
        )
        .child(div().w(px(300.0)).h(px(60.0)).child(menu))
        .child(
            div()
                .text_size(crate::typography::ui_rems(11.5))
                .text_color(theme.text_faint)
                .child(caption),
        )
        .into_any_element()
}

fn sidebar_visual(theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    const DONE_AT: f32 = 2.2;
    const NEXT_AT: f32 = 2.9;
    let nested = |row: gpui::Div| row.ml(px(18.0));
    let finished = t >= DONE_AT;
    let next = t >= NEXT_AT;
    let leave = ramp(t, 2.8, 260.0, motion::EASE_OUT);
    let percent = (40.0 + 20.0 * ramp(t, DONE_AT, 500.0, motion::EASE_OUT)).round() as u8;
    let (_, _, ok) = chat_status_tone(zeron_proto::ChatIndicator::Completed, theme);
    let first = if finished {
        tree_glyph(TreeStatus::Done, 12.0, ok, true)
    } else {
        running(theme, reduced)
    };
    div()
        .w(px(250.0))
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(theme.border)
        .bg(crate::settings::widgets::block_fill(theme))
        .child(rise(
            since_row(
                theme,
                ring(theme, percent),
                "Ship the shop",
                Some(if finished { "3/5" } else { "2/5" }),
            )
            .bg(theme.wash(0.06)),
            t,
            0.0,
        ))
        .child(
            div()
                .h(px(28.0 * (1.0 - leave)))
                .overflow_hidden()
                .opacity(1.0 - leave)
                .child(rise(
                    nested(since_row(theme, first, "Product grid with filters", None)),
                    t,
                    0.07,
                )),
        )
        .child(rise(
            nested(since_row(
                theme,
                running(theme, reduced),
                "Cart with persistence",
                None,
            )),
            t,
            0.14,
        ))
        .when(next, |el| {
            el.child(rise(
                nested(since_row(
                    theme,
                    running(theme, reduced),
                    "Checkout flow",
                    None,
                )),
                t,
                NEXT_AT,
            ))
        })
        .child(rise(
            nested(
                div()
                    .h(px(24.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted.opacity(0.6))
                    .child(if next { "1 more step" } else { "2 more steps" }),
            ),
            t,
            0.21,
        ))
        .into_any_element()
}

fn supervisor_conversation_visual(theme: &Theme, t: f32) -> AnyElement {
    const PREFIX: &str = "Keep an eye on ";
    const MENTION: &str = "@Fix tab strip jitter";
    const WATCH_AT: f32 = 2.4;
    let tone = crate::supervisor::ok_tone(theme);
    let total = PREFIX.len() + MENTION.len();
    let typed = (((t - 0.3) / 0.045).max(0.0) as usize).min(total);
    let mention = &MENTION[..typed.saturating_sub(PREFIX.len())];
    let caret = t < WATCH_AT && (typed < total || (t * 2.0).fract() < 0.5);
    let bubble = div()
        .h(px(34.0))
        .max_w(px(300.0))
        .px(px(12.0))
        .rounded(px(14.0))
        .bg(theme.wash(0.08))
        .text_size(crate::typography::ui_rems(12.5))
        .text_color(theme.text)
        .flex()
        .items_center()
        .gap(px(4.0))
        .child(PREFIX[..typed.min(PREFIX.len())].trim_end())
        .when(!mention.is_empty(), |el| {
            el.child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.accent)
                    .child(mention),
            )
        })
        .when(caret, |el| {
            el.child(div().ml(px(-3.0)).w(px(1.5)).h(px(14.0)).bg(theme.text))
        });
    let pop = ramp(t, WATCH_AT, 220.0, motion::EASE_SNAP);
    let watched = div()
        .relative()
        .top(px((1.0 - pop) * 8.0))
        .opacity(ramp(t, WATCH_AT, 150.0, motion::EASE))
        .h(px(26.0))
        .pl(px(9.0))
        .pr(px(12.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded_full()
        .border_1()
        .border_color(tone.opacity(0.55))
        .bg(tone.opacity(0.08))
        .text_size(crate::typography::ui_rems(12.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(
            icons::icon(icons::LUCIDE_EYE)
                .size(px(13.0))
                .text_color(tone),
        )
        .child("Watched");
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(12.0))
        .child(bubble)
        .child(watched)
        .into_any_element()
}

pub(super) fn loop_clock(shown: Instant, cx: &App) -> Option<f32> {
    (!motion::reduced_motion(cx))
        .then(|| motion::since(shown).as_secs_f32() / motion::speed_scale().max(f32::EPSILON))
}

pub(super) fn is_since_visual(id: &str) -> bool {
    SINCE_PAGES.iter().any(|page| page.id == id)
}

pub(super) fn since_visual(id: &str, theme: &Theme, clock: Option<f32>) -> AnyElement {
    let (period, rest) = match id {
        TASKS_HOME => (6.6, 1.9),
        MISSION_TABS => (6.0, 1.5),
        SIDEBAR_MISSION => (6.4, 4.5),
        _ => (6.0, 4.5),
    };
    let reduced = clock.is_none();
    let t = clock.map_or(rest, |clock| clock % period);
    let visual = match id {
        TASKS_HOME => tasks_visual(theme, t, reduced),
        MISSION_TABS => tabs_visual(theme, t, reduced),
        SIDEBAR_MISSION => sidebar_visual(theme, t, reduced),
        _ => supervisor_conversation_visual(theme, t),
    };
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .opacity(1.0 - ramp(t, period - 0.35, 300.0, motion::EASE_IN))
        .child(visual)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{MISSION_TABS, SIDEBAR_MISSION, SUPERVISOR_CONVERSATION, TASKS_HOME, since_pages};
    use crate::settings::SINCE_INTRO_ID;

    fn seen(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn the_since_sheet_drops_used_pages_and_never_returns_once_closed() {
        assert_eq!(since_pages(&[]), vec![0, 1, 2, 3]);
        assert_eq!(since_pages(&seen(&[TASKS_HOME, MISSION_TABS])), vec![2, 3]);
        assert_eq!(
            since_pages(&seen(&[
                TASKS_HOME,
                MISSION_TABS,
                SIDEBAR_MISSION,
                SUPERVISOR_CONVERSATION
            ])),
            Vec::<usize>::new()
        );
        assert_eq!(since_pages(&seen(&[SINCE_INTRO_ID])), Vec::<usize>::new());
    }
}
