//! First-run setup in three steps: a welcome that offers sync, the agents on
//! this device, and "You're all set" with the way to a first project.
//!
//! Shown in place of the Ready shell until it's finished
//! (`onboardingCompleted` in `ui-settings.json`); only a fresh install starts
//! it. Each step is a full-window [`render_page`]: the Settings column, type
//! and blocks straight on the window's surface. The shell owns what the steps
//! can't do from here — the sign-in flow, agent updates, Docs and the
//! add-project dialog — and hears about them through [`OnboardingEvent`]. Its
//! sign-in steps render through the same page, inside the welcome step while
//! setup runs; the signed-in edge moves setup on to the agents step.

use gpui::{
    AnyElement, Context, Entity, EventEmitter, IntoElement, SharedString, Subscription, Window,
    div, prelude::*, px,
};

use zeron_engine::registry::HarnessDescriptor;
use zeron_proto::HarnessId;

use crate::icons::{self, icon};
use crate::motion;
use crate::popover::{self, Loadable};
use crate::settings::harnesses::HarnessesPage;
use crate::settings::widgets;
use crate::state::{AppState, EngineHandle};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Agents,
    Done,
}

impl Step {
    const ALL: [Step; 3] = [Step::Welcome, Step::Agents, Step::Done];

    /// `ZERON_ONBOARDING` values: a step name, or anything else to start at
    /// the beginning.
    pub fn parse(value: &str) -> Step {
        match value {
            "agents" | "providers" => Step::Agents,
            "done" => Step::Done,
            _ => Step::Welcome,
        }
    }

    fn next(self) -> Step {
        match self {
            Step::Welcome => Step::Agents,
            Step::Agents | Step::Done => Step::Done,
        }
    }

    fn previous(self) -> Step {
        match self {
            Step::Welcome | Step::Agents => Step::Welcome,
            Step::Done => Step::Agents,
        }
    }

    fn number(self) -> usize {
        Step::ALL.iter().position(|step| *step == self).unwrap_or(0) + 1
    }

    fn label(self) -> &'static str {
        match self {
            Step::Welcome => "Welcome",
            Step::Agents => "Your agents",
            Step::Done => "Done",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Step::Welcome => "welcome",
            Step::Agents => "agents",
            Step::Done => "done",
        }
    }
}

/// A page's content measure: narrower than Settings' column, since one
/// question sits in it, with the same air either side.
const COLUMN_WIDTH: f32 = 560.0;
const COLUMN_PAD_X: f32 = 40.0;
/// The steps' shared frame, tall enough for the agents list.
const FRAME_MIN_HEIGHT: f32 = 420.0;
const FRAME_MAX_HEIGHT: f32 = 640.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingEvent {
    SignIn,
    OpenDocs,
    Finish { add_project: bool },
}

/// What the agents step can say about this device's agents. Pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentsSummary {
    Looking,
    Unavailable(String),
    NoneInstalled,
    Installed(Vec<String>),
}

pub fn agents_summary(harnesses: &Loadable<Vec<HarnessDescriptor>>) -> AgentsSummary {
    match harnesses {
        Loadable::Idle | Loadable::Loading => AgentsSummary::Looking,
        Loadable::Error(error) => AgentsSummary::Unavailable(error.clone()),
        Loadable::Ready(list) => {
            let names: Vec<String> = crate::pickers::visible_harnesses(list)
                .into_iter()
                .filter(|descriptor| descriptor.installed && descriptor.id != HarnessId::Mock)
                .map(|descriptor| descriptor.name)
                .collect();
            if names.is_empty() {
                AgentsSummary::NoneInstalled
            } else {
                AgentsSummary::Installed(names)
            }
        }
    }
}

/// "Claude Code", "Claude Code and Codex", "Claude Code, Codex and Cursor".
pub fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A list short enough for one line: past three, the first two and a count.
pub fn short_names(names: &[String]) -> String {
    match names {
        [first, second, rest @ ..] if rest.len() > 1 => {
            format!("{first}, {second} and {} more", rest.len())
        }
        _ => join_names(names),
    }
}

fn this_device() -> &'static str {
    if cfg!(target_os = "macos") {
        "this Mac"
    } else {
        "this computer"
    }
}

pub struct Onboarding {
    state: Entity<AppState>,
    step: Step,
    /// Sign in was pressed: the signed-in edge moves on to the agents step.
    awaiting_sign_in: bool,
    /// What the agents page last found, mirrored for the summary and the
    /// way on.
    harnesses: Loadable<Vec<HarnessDescriptor>>,
    /// Settings → Providers' rows for this device — toggles, installs, and
    /// each agent's settings and sign-in under its caret.
    agents_page: Option<Entity<HarnessesPage>>,
    /// The engine the agents page talks to; the sync switch replaces it.
    agents_engine: Option<EngineHandle>,
    _observe_state: Subscription,
    _observe_agents: Option<Subscription>,
}

impl EventEmitter<OnboardingEvent> for Onboarding {}

impl Onboarding {
    pub fn new(state: Entity<AppState>, step: Step, cx: &mut Context<Self>) -> Self {
        let observe_state = cx.observe(&state, |this, state, cx| {
            if this.awaiting_sign_in && state.read(cx).signed_in() {
                this.awaiting_sign_in = false;
                this.go(Step::Agents, cx);
            }
            // The engine arrived, or the sync switch replaced it: the agents
            // page has to ask the new one.
            let engine_changed = match (state.read(cx).engine(), &this.agents_engine) {
                (Some(engine), Some(seen)) => !engine.same_connection(seen),
                (Some(_), None) => true,
                (None, _) => false,
            };
            if engine_changed {
                this.load(cx);
            }
            cx.notify();
        });
        let mut onboarding = Self {
            state,
            step: Step::Welcome,
            awaiting_sign_in: false,
            harnesses: Loadable::Idle,
            agents_page: None,
            agents_engine: None,
            _observe_state: observe_state,
            _observe_agents: None,
        };
        onboarding.go(step, cx);
        onboarding
    }

    pub fn step(&self) -> Step {
        self.step
    }

    fn go(&mut self, step: Step, cx: &mut Context<Self>) {
        self.step = step;
        // Look for agents from the start, so the list is there on arrival.
        if !matches!(self.harnesses, Loadable::Ready(_) | Loadable::Loading) {
            self.load(cx);
        }
        cx.notify();
    }

    fn finish(&mut self, add_project: bool, cx: &mut Context<Self>) {
        cx.emit(OnboardingEvent::Finish { add_project });
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        self.awaiting_sign_in = true;
        cx.emit(OnboardingEvent::SignIn);
    }

    /// This device's agents: the Providers page, built against the current
    /// engine, or asked again. A painted list stays while a rescan runs.
    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if let Some(page) = &self.agents_page
            && self
                .agents_engine
                .as_ref()
                .is_some_and(|seen| seen.same_connection(&engine))
        {
            page.update(cx, |page, cx| page.reload(cx));
            return;
        }
        let state = self.state.clone();
        let page = cx.new(|cx| HarnessesPage::new_embedded(state, cx));
        self._observe_agents = Some(cx.observe(&page, |this, page, cx| {
            this.harnesses = page.read(cx).harnesses().clone();
            cx.notify();
        }));
        self.harnesses = page.read(cx).harnesses().clone();
        self.agents_page = Some(page);
        self.agents_engine = Some(engine);
        cx.notify();
    }
}

// ---- the page ----

/// One full-window page of setup. The sign-in flow fills one too, so it reads
/// as part of setup wherever it started.
pub(crate) struct PageParts {
    /// Keys the body's entrance: a new key replays it.
    pub key: SharedString,
    /// The step progress, on the numbered steps.
    pub header: Option<AnyElement>,
    pub body: AnyElement,
    pub footer: Option<AnyElement>,
    /// One frame shared by the steps (header on top, footer at the bottom)
    /// so Back and Continue never move; otherwise the column fits its
    /// content.
    pub fixed: bool,
}

/// A sign-in step the shell owns, shown in the welcome step's place while it
/// runs.
pub(crate) struct SignInSlot {
    pub key: SharedString,
    pub body: AnyElement,
    pub footer: AnyElement,
}

impl SignInSlot {
    /// The slot on its own, outside setup: the same page without progress.
    pub fn into_page(self) -> PageParts {
        PageParts {
            key: self.key,
            header: None,
            body: self.body,
            footer: Some(self.footer),
            fixed: false,
        }
    }
}

/// The page: a centered column under the titlebar strip, on the window's own
/// surface the way Settings sits on it — no card. The header and footer keep
/// their own entrance keys, so moving between steps fades only the body
/// while the step pills and the buttons hold still.
pub(crate) fn render_page(parts: PageParts, window: &Window) -> AnyElement {
    let fixed = parts.fixed;
    let height = (f32::from(window.viewport_size().height) - Theme::TITLEBAR_HEIGHT - 64.0)
        .clamp(FRAME_MIN_HEIGHT, FRAME_MAX_HEIGHT);
    let column = div()
        .relative()
        .w_full()
        .max_w(px(COLUMN_WIDTH + 2.0 * COLUMN_PAD_X))
        .px(px(COLUMN_PAD_X))
        .when(fixed, |column| column.h(px(height)))
        .flex()
        .flex_col()
        .children(parts.header.map(|header| {
            motion::fade_in(
                "onboarding-header",
                div().flex_none().h(px(20.0)).mb(px(28.0)).child(header),
            )
        }))
        .child(
            div()
                .when(fixed, |el| el.flex_1().min_h_0())
                .flex()
                .flex_col()
                .child(motion::fade_in(
                    parts.key,
                    div()
                        .w_full()
                        .when(fixed, |el| el.h_full())
                        .flex()
                        .flex_col()
                        .child(parts.body),
                )),
        )
        .children(parts.footer.map(|footer| {
            motion::fade_in(
                "onboarding-footer",
                div().flex_none().pt(px(24.0)).child(footer),
            )
        }));
    div()
        .absolute()
        .inset_0()
        .child(
            div()
                .absolute()
                .inset_0()
                .pt(px(Theme::TITLEBAR_HEIGHT))
                .pb(px(Theme::TITLEBAR_HEIGHT / 2.0))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .child(column),
        )
        .into_any_element()
}

// ---- pieces ----

/// A page's headline and the line under it: the Settings page header.
pub(crate) fn heading(theme: &Theme, title: &str, subtitle: impl Into<SharedString>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .child(widgets::page_header(theme, title, None))
        .child(widgets::page_subtitle(theme, subtitle))
}

/// One block holding a single quiet line: where sync stands, what the
/// sign-in is waiting on.
pub(crate) fn status_block(
    theme: &Theme,
    leading: AnyElement,
    text: impl Into<SharedString>,
    emphasized: bool,
) -> gpui::Div {
    widgets::section_card(theme).mt(px(28.0)).child(
        widgets::card_row(theme, true)
            .min_h(px(44.0))
            .py(px(10.0))
            .flex_nowrap()
            .gap(px(10.0))
            .child(div().flex_none().flex().items_center().child(leading))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(crate::typography::ui_rems(12.5))
                    .line_height(px(17.0))
                    .text_color(if emphasized {
                        theme.text
                    } else {
                        theme.text_muted
                    })
                    .child(text.into()),
            ),
    )
}

/// The wide white button the welcome and the ending end on.
pub(crate) fn wide_button(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .w_full()
        .h(px(36.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(theme.text)
        .text_size(crate::typography::ui_rems(14.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.on_solid)
        .cursor_pointer()
        .hover(|s| s.opacity(0.9))
        .child(SharedString::from(label))
}

fn quiet_link(theme: &Theme, id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted.opacity(0.7))
        .cursor_pointer()
        .hover(|s| s.text_color(theme.text))
        .child(SharedString::from(label))
}

pub(crate) fn primary(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    popover::btn_primary(theme, label)
        .id(id)
        .px(px(14.0))
        .when(!enabled, |el| el.opacity(0.35).cursor_default())
}

pub(crate) fn ghost(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
) -> gpui::Stateful<gpui::Div> {
    popover::btn_ghost(theme, label, format!("{id}-fade")).id(id)
}

/// The way back (or out) on the left, the way forward on the right. The
/// leading button sits its label, not its hover pad, on the content edge.
pub(crate) fn footer_row(
    back: Option<gpui::Stateful<gpui::Div>>,
    forward: Vec<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(div().children(back.map(|back| back.ml(px(-12.0)))))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .children(forward),
        )
        .into_any_element()
}

/// The steps as pills, the current one outlined.
fn step_pills(theme: &Theme, current: Step) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .flex_row()
        .justify_center()
        .gap(px(8.0))
        .children(Step::ALL.map(|step| {
            let selected = step == current;
            div()
                .h(px(24.0))
                .px(px(12.0))
                .flex()
                .items_center()
                .rounded_full()
                .border_1()
                .border_color(if selected {
                    theme.text.opacity(0.5)
                } else {
                    theme.border
                })
                .text_size(crate::typography::ui_rems(12.0))
                .font_weight(if selected {
                    gpui::FontWeight::MEDIUM
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(if selected {
                    theme.text
                } else {
                    theme.text_muted
                })
                .gap(px(4.0))
                .when(step.number() < current.number(), |pill| {
                    pill.child("\u{2713}")
                })
                .child(SharedString::from(step.label()))
        }))
}

impl Onboarding {
    /// The current step as a page. `sign_in` stands in for the welcome
    /// step's body and footer while the shell's sign-in flow runs, under the
    /// same pills, so it reads as this step. `updates` is the shell's agent
    /// updates row, shown on the agents step.
    pub(crate) fn page_parts(
        &mut self,
        sign_in: Option<SignInSlot>,
        updates: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> PageParts {
        let theme = Theme::of(cx).for_settings_surface();
        if let Some(slot) = sign_in {
            return PageParts {
                key: slot.key,
                header: Some(step_pills(&theme, Step::Welcome).into_any_element()),
                body: slot.body,
                footer: Some(slot.footer),
                fixed: true,
            };
        }
        let step = self.step;
        let body = match step {
            Step::Welcome => self.render_welcome(&theme, cx),
            Step::Agents => self.render_providers(&theme, updates, cx),
            Step::Done => self.render_done(&theme, cx),
        };
        PageParts {
            key: SharedString::from(format!("onboarding-{}", step.key())),
            header: Some(step_pills(&theme, step).into_any_element()),
            body,
            footer: Some(self.render_footer(&theme, cx)),
            fixed: true,
        }
    }

    fn render_welcome(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let signed_in = state.signed_in().then(|| {
            state
                .auth_user()
                .map(|user| format!("Signed in as {}", user.email))
                .unwrap_or_else(|| "You're signed in".into())
        });
        let choice = match signed_in {
            Some(line) => div()
                .mt(px(28.0))
                .text_size(crate::typography::ui_rems(13.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(line))
                .into_any_element(),
            None => div()
                .mt(px(28.0))
                .flex()
                .flex_row()
                .gap(px(10.0))
                .child(
                    primary(theme, "onboarding-sign-in", "Sign in to sync", true)
                        .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
                )
                .child(
                    widgets::text_action(
                        theme,
                        widgets::ActionTone::Filled,
                        "Use without an account",
                    )
                    .id("onboarding-local")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.go(Step::Agents, cx))),
                )
                .into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .text_center()
            .child(
                icon(icons::ZERON_LOGO)
                    .w(px(36.6))
                    .h(px(42.0))
                    .text_color(theme.text),
            )
            .child(
                div()
                    .mt(px(28.0))
                    .text_size(crate::typography::ui_rems(24.0))
                    .line_height(px(30.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(SharedString::from("Welcome to Zeron")),
            )
            .child(
                div()
                    .mt(px(10.0))
                    .max_w(px(420.0))
                    .text_size(crate::typography::ui_rems(14.0))
                    .line_height(px(21.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!(
                        "Your agents in one window. Sign in to sync chats to your phone, or keep everything on {}.",
                        this_device()
                    ))),
            )
            .child(choice)
            .child(
                div()
                    .mt(px(20.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted.opacity(0.8))
                    .child(SharedString::from(match crate::appearance::mode(cx) {
                        crate::appearance::AppearanceMode::System => format!(
                            "Appearance follows your {}. Change it in Settings › Appearance.",
                            if cfg!(target_os = "macos") { "Mac" } else { "system" }
                        ),
                        mode => format!(
                            "Appearance is set to {}. Change it in Settings › Appearance.",
                            mode.label()
                        ),
                    })),
            )
            .into_any_element()
    }

    fn render_providers(
        &mut self,
        theme: &Theme,
        updates: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let signed_in = self.state.read(cx).signed_in();
        // Settings → Providers' own header controls: refresh, and once
        // you're signed in the device switcher, Cloud's logins among them.
        let toolbar = self
            .agents_page
            .clone()
            .map(|page| page.update(cx, |page, cx| page.render_toolbar(theme, signed_in, cx)));
        let place = self.agents_place(cx);
        let summary = agents_summary(&self.harnesses);
        let (title, detail): (&'static str, String) = match &summary {
            AgentsSummary::Looking => (
                "Your agents",
                format!("Looking for coding agents on {place}…"),
            ),
            AgentsSummary::Unavailable(_) => {
                ("Your agents", format!("Couldn't check {place} for agents."))
            }
            AgentsSummary::NoneInstalled => (
                "Install an agent",
                format!(
                    "Zeron runs the coding agents you already use, and none are on {place} yet. Install one to continue."
                ),
            ),
            AgentsSummary::Installed(names) => (
                "Your agents",
                format!(
                    "Found {} on {place}. Sign in to any you'd like to use.",
                    if names.len() > 3 {
                        format!("{} agents", names.len())
                    } else {
                        join_names(names)
                    },
                ),
            ),
        };
        // Settings → Providers' own rows: the switch, the caret and the
        // agent's settings and sign-in in its accordion.
        let list = self
            .agents_page
            .clone()
            .map(|page| div().mt(px(24.0)).flex_1().min_h_0().child(page));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .justify_between()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(heading(theme, title, detail)),
                    )
                    .children(toolbar),
            )
            .children(updates.map(|row| div().mt(px(20.0)).child(row)))
            .children(list)
            .into_any_element()
    }

    /// Where the agents page is looking, for its subtitle: this device, one
    /// of your others, or Cloud.
    fn agents_place(&self, cx: &Context<Self>) -> String {
        let target = self
            .agents_page
            .as_ref()
            .and_then(|page| page.read(cx).target_device().map(str::to_owned));
        match target {
            None => this_device().into(),
            Some(id) if id == zeron_proto::CLOUD_ACCOUNTS_DEVICE => "Cloud".into(),
            Some(id) => self
                .state
                .read(cx)
                .provider_devices()
                .find(|device| device.id == id)
                .map(|device| device.name.clone())
                .unwrap_or_else(|| "that device".into()),
        }
    }

    fn render_done(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let has_projects = !state.spaces.is_empty();
        let sync = if state.signed_in() {
            "Sync on"
        } else {
            "Sync off"
        };
        let agents = match agents_summary(&self.harnesses) {
            AgentsSummary::Installed(names) if names.len() == 1 => "1 agent ready".to_string(),
            AgentsSummary::Installed(names) => format!("{} agents ready", names.len()),
            AgentsSummary::NoneInstalled => "No agents yet".into(),
            AgentsSummary::Looking | AgentsSummary::Unavailable(_) => "Agents not checked".into(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px(48.0))
                    .rounded_full()
                    .bg(theme.accent_wash)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(icons::CHECK).size(px(22.0)).text_color(theme.accent)),
            )
            .child(
                div()
                    .mt(px(20.0))
                    .text_size(crate::typography::ui_rems(22.0))
                    .line_height(px(28.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(SharedString::from("You're all set")),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .text_size(crate::typography::ui_rems(13.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!("{sync} · {agents}"))),
            )
            .child(
                quiet_link(
                    theme,
                    "onboarding-docs",
                    "How Tasks and plans work, in Docs →",
                )
                .mt(px(14.0))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(OnboardingEvent::OpenDocs))),
            )
            .when(!has_projects, |el| {
                el.child(
                    div().mt(px(28.0)).w(px(320.0)).child(
                        wide_button(theme, "onboarding-add-project", "Add your first project")
                            .on_click(cx.listener(|this, _, _, cx| this.finish(true, cx))),
                    ),
                )
            })
            .into_any_element()
    }

    /// "Step N of 3" on the left; Back and the way forward on the right.
    fn render_footer(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let step = self.step;
        let back = (step != Step::Welcome).then(|| {
            ghost(theme, "onboarding-back", "Back")
                .on_click(cx.listener(|this, _, _, cx| this.go(this.step.previous(), cx)))
                .into_any_element()
        });
        let mut forward: Vec<AnyElement> = back.into_iter().collect();
        match step {
            Step::Agents => {
                let summary = agents_summary(&self.harnesses);
                let ready = matches!(summary, AgentsSummary::Installed(_));
                // Nothing to run yet: the way on is installing one, but the
                // rest of the app stays reachable.
                if matches!(
                    summary,
                    AgentsSummary::NoneInstalled | AgentsSummary::Unavailable(_)
                ) {
                    forward.push(
                        ghost(theme, "onboarding-skip-agents", "Skip for now")
                            .on_click(cx.listener(|this, _, _, cx| this.go(Step::Done, cx)))
                            .into_any_element(),
                    );
                }
                forward.push(
                    primary(theme, "onboarding-continue", "Continue", ready)
                        .when(ready, |el| {
                            el.on_click(cx.listener(|this, _, _, cx| this.go(Step::Done, cx)))
                        })
                        .into_any_element(),
                );
            }
            Step::Done => forward.push(
                primary(theme, "onboarding-finish", "Start", true)
                    .on_click(cx.listener(|this, _, _, cx| this.finish(false, cx)))
                    .into_any_element(),
            ),
            Step::Welcome => forward.push(
                primary(theme, "onboarding-continue", "Continue", true)
                    .on_click(cx.listener(|this, _, _, cx| this.go(this.step.next(), cx)))
                    .into_any_element(),
            ),
        }
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!(
                        "Step {} of {}",
                        step.number(),
                        Step::ALL.len()
                    ))),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .children(forward),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(id: HarnessId, name: &str, installed: bool) -> HarnessDescriptor {
        HarnessDescriptor {
            id,
            name: name.into(),
            supports_steering: false,
            steering_mode: zeron_proto::SteeringMode::TurnBoundary,
            reasoning_levels: Vec::new(),
            installed,
            can_install: true,
            enabled: None,
            access_modes: Vec::new(),
            supports_plan: false,
        }
    }

    #[test]
    fn steps_walk_forward_and_back() {
        use Step::*;
        let mut step = Welcome;
        let mut seen = vec![step];
        while step != Done {
            step = step.next();
            seen.push(step);
        }
        assert_eq!(seen, [Welcome, Agents, Done]);
        assert_eq!(
            seen.iter().map(|s| s.number()).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(Done.previous(), Agents);
        assert_eq!(Agents.previous(), Welcome);
        assert_eq!(Welcome.previous(), Welcome);
        assert_eq!(Step::parse("agents"), Agents);
        assert_eq!(Step::parse("providers"), Agents);
        assert_eq!(Step::parse("appearance"), Welcome);
    }

    #[test]
    fn the_agents_step_says_what_it_found() {
        assert_eq!(agents_summary(&Loadable::Loading), AgentsSummary::Looking);
        let none = Loadable::Ready(vec![
            descriptor(HarnessId::ClaudeCode, "Claude Code", false),
            descriptor(HarnessId::Codex, "Codex", false),
        ]);
        assert_eq!(agents_summary(&none), AgentsSummary::NoneInstalled);
        let some = Loadable::Ready(vec![
            descriptor(HarnessId::ClaudeCode, "Claude Code", true),
            descriptor(HarnessId::Codex, "Codex", false),
            descriptor(HarnessId::Cursor, "Cursor", true),
        ]);
        let AgentsSummary::Installed(names) = agents_summary(&some) else {
            panic!("expected installed agents");
        };
        assert_eq!(join_names(&names), "Claude Code and Cursor");
        let many: Vec<String> = ["A", "B", "C", "D", "E"].map(String::from).into();
        assert_eq!(short_names(&many), "A, B and 3 more");
        assert_eq!(short_names(&many[..3]), "A, B and C");
        assert_eq!(
            join_names(&["A".into(), "B".into(), "C".into()]),
            "A, B and C"
        );
    }
}
