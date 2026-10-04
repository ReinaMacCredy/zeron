use std::time::{Duration, Instant};

use gpui::{
    AnyElement, ClipboardItem, Context, Entity, MouseButton, Pixels, Point, ScrollHandle,
    SharedString, Task, Window, div, point, prelude::*, px,
};
use zeron_markdown::parser::BlockTree;
use zeron_proto::{
    ChangeRequestCheck, ChangeRequestCheckStatus, ChangeRequestChecksState, ChangeRequestDetail,
    ChangeRequestFailure, ChangeRequestListItem, ChangeRequestState, ChangeRequestVerdict,
    GetChangeRequestRequest,
};
use zeron_rpc::methods;

use super::actions::ActionsState;
use super::activity::ActivityState;
use super::code::{CodeEvent, PullRequestCode};
use super::{
    PullRequestKey, PullRequestsPage, Tones, avatar, diff_stat, failure_message, label_color,
    relative_time,
};
use crate::icons::{self, icon};
use crate::popover::Loadable;
use crate::theme::Theme;

pub(super) const PANEL_MIN_WIDTH: f32 = 360.0;
const FOLD_HEIGHT: f32 = 104.0;
const COPIED_FOR: Duration = Duration::from_millis(1600);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Description,
    Checks,
    Comments,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DetailTab {
    Summary,
    Timeline,
    Code,
}

pub(super) struct ChecksPopover {
    key: PullRequestKey,
    position: Point<Pixels>,
    from_header: bool,
}

pub(crate) fn checks_verdict_label(state: ChangeRequestChecksState) -> &'static str {
    match state {
        ChangeRequestChecksState::Passing => "All checks have passed",
        ChangeRequestChecksState::Failing => "Some checks were not successful",
        ChangeRequestChecksState::Pending => "Some checks haven't completed yet",
    }
}

pub(super) struct DetailState {
    seed: ChangeRequestListItem,
    pub(super) detail: Loadable<ChangeRequestDetail>,
    body: Option<BlockTree>,
    scroll: ScrollHandle,
    condensed: bool,
    closed_sections: Vec<Section>,
    copied: Option<(&'static str, Instant)>,
    tab: DetailTab,
    code: Option<Entity<PullRequestCode>>,
    code_sub: Option<gpui::Subscription>,
    pub(super) activity: ActivityState,
    pub(super) actions: ActionsState,
    _task: Option<Task<()>>,
}

impl DetailState {
    pub(super) fn seed(&self) -> &ChangeRequestListItem {
        &self.seed
    }

    pub(super) fn seed_title(&self) -> &str {
        &self.seed.title
    }

    pub(super) fn new(seed: ChangeRequestListItem, cx: &mut Context<PullRequestsPage>) -> Self {
        Self {
            activity: ActivityState::new(PullRequestKey::of(&seed), cx),
            actions: ActionsState::default(),
            seed,
            detail: Loadable::Idle,
            body: None,
            scroll: ScrollHandle::new(),
            condensed: false,
            closed_sections: Vec::new(),
            copied: None,
            tab: DetailTab::Summary,
            code: None,
            code_sub: None,
            _task: None,
        }
    }
}

pub(crate) fn is_workflow_approval(check: &ChangeRequestCheck) -> bool {
    check.status == ChangeRequestCheckStatus::ActionRequired
        && check.url.as_deref().is_some_and(|url| {
            url.split_once("/actions/runs/").is_some_and(|(_, rest)| {
                let digits = rest.chars().take_while(char::is_ascii_digit).count();
                digits > 0 && matches!(rest[digits..].chars().next(), None | Some('/'))
            })
        })
}

pub(crate) fn strip_html(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut fence: Option<String> = None;
    let mut in_comment = false;
    for line in markdown.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(marker) = &fence {
            if trimmed.starts_with(marker.as_str()) {
                fence = None;
            }
            out.push_str(line);
            continue;
        }
        if !in_comment && (trimmed.starts_with("```") || trimmed.starts_with("~~~")) {
            fence = Some(trimmed[..3].to_string());
            out.push_str(line);
            continue;
        }
        let mut rest = line;
        let mut kept = String::new();
        while !rest.is_empty() {
            if in_comment {
                match rest.find("-->") {
                    Some(end) => {
                        rest = &rest[end + 3..];
                        in_comment = false;
                    }
                    None => rest = "",
                }
                continue;
            }
            let Some(start) = rest.find(['<', '`']) else {
                kept.push_str(rest);
                break;
            };
            kept.push_str(&rest[..start]);
            rest = &rest[start..];
            if rest.starts_with('`') {
                let ticks = rest.len() - rest.trim_start_matches('`').len();
                let marker = &rest[..ticks];
                match rest[ticks..].find(marker) {
                    Some(end) => {
                        kept.push_str(&rest[..ticks + end + ticks]);
                        rest = &rest[ticks + end + ticks..];
                    }
                    None => {
                        kept.push_str(rest);
                        rest = "";
                    }
                }
            } else if rest.starts_with("<!--") {
                in_comment = true;
                rest = &rest[4..];
            } else if let Some(end) = rest.find('>').filter(|_| {
                rest[1..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '/')
            }) {
                rest = &rest[end + 1..];
            } else {
                kept.push('<');
                rest = &rest[1..];
            }
        }
        if kept.trim().is_empty() && !line.trim().is_empty() {
            continue;
        }
        out.push_str(&kept);
    }
    out
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

pub(crate) fn summarize_checks(checks: &[ChangeRequestCheck]) -> String {
    use ChangeRequestCheckStatus as S;
    let total = checks.len();
    if total == 0 {
        return "No checks reported".into();
    }
    let count = |status: &[S]| {
        checks
            .iter()
            .filter(|check| status.contains(&check.status))
            .count()
    };
    let approvals = checks
        .iter()
        .filter(|check| is_workflow_approval(check))
        .count();
    let other_action = count(&[S::ActionRequired]) - approvals;
    let failed = count(&[S::Failure, S::Cancelled]);
    let pending = count(&[S::Pending]);
    let passed = count(&[S::Success]);
    if failed > 0 {
        format!("{failed} of {total} failing")
    } else if approvals > 0 && other_action > 0 {
        format!(
            "{} and {} awaiting action",
            plural(approvals, "workflow", "workflows"),
            plural(other_action, "check", "checks")
        )
    } else if approvals > 0 {
        format!(
            "{} awaiting approval",
            plural(approvals, "workflow", "workflows")
        )
    } else if other_action > 0 {
        format!(
            "{} awaiting action",
            plural(other_action, "check", "checks")
        )
    } else if pending > 0 {
        format!("{pending} of {total} running")
    } else if passed == total {
        "All checks passed".into()
    } else {
        format!("{passed} of {total} passing")
    }
}

fn check_presentation(
    status: ChangeRequestCheckStatus,
    tones: Tones,
    theme: &Theme,
) -> (&'static str, gpui::Hsla, &'static str) {
    use ChangeRequestCheckStatus as S;
    match status {
        S::Pending => (icons::LUCIDE_REFRESH_CW, tones.pending, "Running"),
        S::ActionRequired => (icons::LUCIDE_CIRCLE_DOT, tones.pending, "Awaiting action"),
        S::Success => (icons::LUCIDE_CIRCLE_CHECK, tones.open, "Passed"),
        S::Failure => (icons::LUCIDE_CIRCLE_X, tones.conflict, "Failed"),
        S::Cancelled => (icons::LUCIDE_CIRCLE_X, tones.conflict, "Cancelled"),
        S::Skipped => (
            icons::LUCIDE_CIRCLE_DASHED,
            theme.text_muted.opacity(0.7),
            "Skipped",
        ),
        S::Neutral => (
            icons::LUCIDE_CIRCLE_DASHED,
            theme.text_muted.opacity(0.7),
            "Neutral",
        ),
    }
}

pub(super) fn verdict_word(state: &str) -> String {
    let words = state.replace(['_', '-'], " ").to_lowercase();
    let mut chars = words.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

impl PullRequestsPage {
    pub(super) fn open_pull_request(
        &mut self,
        item: &ChangeRequestListItem,
        cx: &mut Context<Self>,
    ) {
        let key = PullRequestKey::of(item);
        if !self.tabs.contains(&key) {
            self.tabs.push(key.clone());
        }
        if !self.details.contains_key(&key) {
            self.details
                .insert(key.clone(), DetailState::new(item.clone(), cx));
            self.load_detail(&key, cx);
        }
        self.ensure_activity(&key, cx);
        if matches!(
            self.details
                .get(&key)
                .map(|state| &state.actions.permissions),
            Some(Loadable::Idle)
        ) {
            self.load_permissions(&key, cx);
        }
        self.active = Some(key);
        cx.notify();
    }

    pub(super) fn toggle_checks_popover(
        &mut self,
        item: &ChangeRequestListItem,
        position: Point<Pixels>,
        from_header: bool,
        cx: &mut Context<Self>,
    ) {
        let key = PullRequestKey::of(item);
        if self
            .checks_popover
            .as_ref()
            .is_some_and(|open| open.key == key)
        {
            self.checks_popover = None;
            cx.notify();
            return;
        }
        if !self.details.contains_key(&key) {
            self.details
                .insert(key.clone(), DetailState::new(item.clone(), cx));
            self.load_detail(&key, cx);
        }
        self.checks_popover = Some(ChecksPopover {
            key,
            position,
            from_header,
        });
        cx.notify();
    }

    pub fn dismiss_overlay(&mut self, cx: &mut Context<Self>) -> bool {
        if self.menu.is_open() {
            self.close_menu(cx);
            return true;
        }
        if self.checks_popover.take().is_some() {
            cx.notify();
            return true;
        }
        if self.checkout_menu.take().is_some() {
            cx.notify();
            return true;
        }
        if self.dismiss_action_overlay(cx) {
            return true;
        }
        if self.dismiss_reaction_pickers(cx) {
            return true;
        }
        if let Some(code) = self
            .active
            .as_ref()
            .and_then(|key| self.details.get(key))
            .and_then(|state| state.code.clone())
        {
            return code.update(cx, |code, cx| code.dismiss_overlay(cx));
        }
        false
    }

    pub(super) fn open_commit(&mut self, key: &PullRequestKey, oid: &str, cx: &mut Context<Self>) {
        self.show_tab(key, DetailTab::Code, Some(oid), cx);
    }

    fn show_tab(
        &mut self,
        key: &PullRequestKey,
        tab: DetailTab,
        commit: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if tab == DetailTab::Timeline {
            self.ensure_activity(key, cx);
        }
        let page_state = self.state.clone();
        let Some(state) = self.details.get_mut(key) else {
            return;
        };
        state.tab = tab;
        if tab != DetailTab::Code
            && let Some(code) = state.code.clone()
        {
            code.update(cx, |code, cx| code.dismiss_overlay(cx));
        }
        if tab == DetailTab::Code {
            let created = state.code.is_none();
            let code = state
                .code
                .get_or_insert_with(|| {
                    cx.new(|cx| PullRequestCode::new(page_state, key.clone(), cx))
                })
                .clone();
            if created {
                let event_key = key.clone();
                state.code_sub = Some(cx.subscribe(&code, move |page, _, event, cx| match event {
                    CodeEvent::Toast(title, Some(description)) => {
                        page.toast_with(title.clone(), description.clone(), cx)
                    }
                    CodeEvent::Toast(title, None) => page.toast(title.clone(), cx),
                    CodeEvent::Refresh => {
                        page.load_activity(&event_key, cx);
                        page.load_detail(&event_key, cx);
                    }
                }));
            }
            let commits = match &state.detail {
                Loadable::Ready(detail) => detail.commits.clone(),
                _ => Vec::new(),
            };
            code.update(cx, |code, cx| {
                code.set_commits(&commits, cx);
                match commit {
                    Some(oid) => code.show_commit(oid, cx),
                    None => code.ensure_loaded(cx),
                }
            });
            self.sync_code_review(key, cx);
        }
        cx.notify();
    }

    /// Pushes the threads and what the reader may do with them into the Code tab.
    pub(super) fn sync_code_review(&mut self, key: &PullRequestKey, cx: &mut Context<Self>) {
        let Some(state) = self.details.get(key) else {
            return;
        };
        let Some(code) = state.code.clone() else {
            return;
        };
        let threads = state
            .activity
            .activity
            .ready()
            .map(|activity| super::review::threads_of(&activity.comments))
            .unwrap_or_default();
        let permissions = state.actions.permissions.ready();
        let context = super::review::ReviewContext {
            threads,
            can_comment: permissions.is_some(),
            can_resolve: permissions.is_some_and(|p| p.can_write || p.did_author),
            verdicts: match permissions {
                None => Vec::new(),
                Some(p) if p.did_author => vec![zeron_proto::ChangeRequestReviewEvent::Comment],
                Some(_) => vec![
                    zeron_proto::ChangeRequestReviewEvent::Comment,
                    zeron_proto::ChangeRequestReviewEvent::Approve,
                    zeron_proto::ChangeRequestReviewEvent::RequestChanges,
                ],
            },
        };
        code.update(cx, |code, cx| code.set_review(context, cx));
    }

    pub(super) fn render_checks_popover(
        &self,
        theme: &Theme,
        tones: Tones,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let open = self.checks_popover.as_ref()?;
        let state = self.details.get(&open.key)?;
        let muted = theme.text_muted;
        let item = match &state.detail {
            Loadable::Ready(detail) => &detail.item,
            _ => &state.seed,
        };
        let mut card = crate::popover::popover_card(theme)
            .bg(theme.surface_dialog)
            .id("pull-request-checks-popover")
            .occlude()
            .w(px(320.0))
            .p(px(12.0))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|page, _, _, cx| {
                page.checks_popover = None;
                cx.notify();
            }));
        if let Some(verdict) = item.checks_state {
            card = card.child(
                div()
                    .mb(px(8.0))
                    .text_size(px(14.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(checks_verdict_label(verdict)),
            );
        }
        let body: AnyElement = match &state.detail {
            Loadable::Ready(detail) => {
                if open.from_header {
                    card = card.child(
                        div()
                            .mb(px(8.0))
                            .text_size(px(12.0))
                            .text_color(muted)
                            .child(summarize_checks(&detail.checks)),
                    );
                }
                if detail.checks.is_empty() {
                    div()
                        .text_size(px(12.0))
                        .text_color(muted)
                        .child("No checks reported")
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .children(detail.checks.iter().enumerate().map(|(index, check)| {
                            let (glyph, tone, label) =
                                check_presentation(check.status, tones, theme);
                            let url = check.url.clone();
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .text_size(px(12.0))
                                .child(icon(glyph).size(px(14.0)).text_color(tone).flex_none())
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .text_color(theme.text)
                                        .child(check.name.clone()),
                                )
                                .child(div().text_color(muted).child(label))
                                .children(url.map(|url| {
                                    div()
                                        .id(("pull-request-check-details", index))
                                        .text_color(theme.accent)
                                        .cursor_pointer()
                                        .hover(|style| style.underline())
                                        .on_click(move |_, _, cx| cx.open_url(&url))
                                        .child("Details")
                                }))
                        }))
                        .into_any_element()
                }
            }
            Loadable::Error(error) => div()
                .text_size(px(12.0))
                .text_color(muted)
                .child(error.clone())
                .into_any_element(),
            Loadable::Idle | Loadable::Loading => div()
                .text_size(px(12.0))
                .text_color(muted)
                .child("Loading checks…")
                .into_any_element(),
        };
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position(open.position + point(px(-8.0), px(12.0)))
                    .snap_to_window_with_margin(px(8.0))
                    .child(card.child(body)),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    pub(super) fn close_tab(&mut self, key: &PullRequestKey, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab == key) else {
            return;
        };
        self.tabs.remove(index);
        self.details.remove(key);
        if self.active.as_ref() == Some(key) {
            self.active = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .cloned();
        }
        cx.notify();
    }

    pub(super) fn load_detail(&mut self, key: &PullRequestKey, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(state) = self.details.get_mut(key) else {
            return;
        };
        state.detail = Loadable::Loading;
        let params = serde_json::to_value(GetChangeRequestRequest {
            host: key.host.clone(),
            repository: key.repository.clone(),
            number: key.number,
        })
        .unwrap_or(serde_json::Value::Null);
        let key = key.clone();
        state._task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::GET_CHANGE_REQUEST, params)
                .await;
            this.update(cx, |page, cx| {
                let Some(state) = page.details.get_mut(&key) else {
                    return;
                };
                state.detail = match result {
                    Ok(value) => match serde_json::from_value::<ChangeRequestDetail>(value) {
                        Ok(detail) => {
                            if let Some(code) = state.code.clone() {
                                let commits = detail.commits.clone();
                                code.update(cx, |code, cx| code.set_commits(&commits, cx));
                            }
                            let body = strip_html(&detail.body);
                            state.body =
                                Some(crate::markdown::parse_full(if body.trim().is_empty() {
                                    "_No description provided._"
                                } else {
                                    &body
                                }));
                            Loadable::Ready(detail)
                        }
                        Err(error) => Loadable::Error(error.to_string()),
                    },
                    Err(error) => {
                        let message = error.to_string();
                        Loadable::Error(
                            serde_json::from_str::<ChangeRequestFailure>(&message)
                                .map(|failure| failure_message(failure).to_string())
                                .unwrap_or(message),
                        )
                    }
                };
                cx.notify();
            })
            .ok();
        }));
    }

    fn mark_copied(&mut self, key: &PullRequestKey, what: &'static str, cx: &mut Context<Self>) {
        if let Some(state) = self.details.get_mut(key) {
            state.copied = Some((what, Instant::now()));
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn panel_width(&self, viewport: f32, cx: &gpui::App) -> f32 {
        let stored = self
            .panel_width
            .or(crate::settings::current(cx).pull_requests_panel_width)
            .unwrap_or((viewport / 2.0).floor());
        clamp_panel_width(stored, viewport)
    }

    pub(super) fn render_panel(
        &mut self,
        theme: &Theme,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tones = Tones::of(theme);
        let tabstrip = self.render_tabstrip(theme, tones, cx);
        let body = match self.active.clone() {
            Some(key) => self.render_detail(&key, theme, tones, window, cx),
            None => div().into_any_element(),
        };
        div()
            .w(px(width))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .child(tabstrip)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    fn render_tabstrip(&self, theme: &Theme, tones: Tones, cx: &mut Context<Self>) -> AnyElement {
        let mut strip = div()
            .h(px(Theme::TITLEBAR_HEIGHT))
            .pt(px(Theme::TITLEBAR_TOP_PAD))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .pl(px(8.0))
            .overflow_hidden();
        for key in &self.tabs {
            let Some(state) = self.details.get(key) else {
                continue;
            };
            let item = match &state.detail {
                Loadable::Ready(detail) => &detail.item,
                _ => &state.seed,
            };
            let (glyph, tone) = tab_glyph(item, tones);
            let active = self.active.as_ref() == Some(key);
            let hovered = self.hovered_tab.as_ref() == Some(key);
            let id = format!(
                "pull-request-tab-{}-{}-{}",
                key.host, key.repository, key.number
            );
            let open_key = key.clone();
            let close_key = key.clone();
            let middle_key = key.clone();
            let hover_key = key.clone();
            strip = strip.child(
                div()
                    .id(SharedString::from(id))
                    .role(gpui::Role::Tab)
                    .aria_label(SharedString::from(format!("#{}", key.number)))
                    .h(px(24.0))
                    .max_w(px(144.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .pl(px(6.0))
                    .pr(px(8.0))
                    .rounded(px(8.0))
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .when(active, |el| el.bg(theme.ink(0.055)).text_color(theme.text))
                    .when(!active, |el| {
                        el.text_color(theme.text_muted)
                            .hover(|style| style.bg(theme.ink(0.035)).text_color(theme.text))
                    })
                    .on_hover(cx.listener(move |page, hovered: &bool, _, cx| {
                        let next = hovered.then(|| hover_key.clone());
                        if page.hovered_tab != next {
                            page.hovered_tab = next;
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |page, _, _, cx| page.close_tab(&middle_key, cx)),
                    )
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.active = Some(open_key.clone());
                        cx.notify();
                    }))
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-close", key.number)))
                            .role(gpui::Role::Button)
                            .aria_label(SharedString::from(format!("Close #{}", key.number)))
                            .size(px(16.0))
                            .flex_none()
                            .rounded(px(4.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|style| style.bg(theme.ink(0.08)))
                            .on_click(cx.listener(move |page, _, _, cx| {
                                cx.stop_propagation();
                                page.close_tab(&close_key, cx);
                            }))
                            .child(if hovered {
                                icon(icons::LUCIDE_X).size(px(12.0)).text_color(theme.text)
                            } else {
                                icon(glyph).size(px(12.0)).text_color(tone)
                            }),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(format!("#{}", key.number)),
                    ),
            );
        }
        strip.into_any_element()
    }

    fn render_detail(
        &mut self,
        key: &PullRequestKey,
        theme: &Theme,
        tones: Tones,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.details.get_mut(key) else {
            return div().into_any_element();
        };
        let offset = state.scroll.offset();
        if !state.condensed && -f32::from(offset.y) > FOLD_HEIGHT + 32.0 {
            state.condensed = true;
            state
                .scroll
                .set_offset(point(offset.x, offset.y + px(FOLD_HEIGHT)));
        } else if state.condensed && -f32::from(offset.y) < 4.0 {
            state.condensed = false;
        }
        if let Some((_, at)) = state.copied
            && at.elapsed() >= COPIED_FOR
        {
            state.copied = None;
        }
        let state = &self.details[key];
        let detail = match &state.detail {
            Loadable::Ready(detail) => Some(detail),
            _ => None,
        };
        let item = detail.map(|detail| &detail.item).unwrap_or(&state.seed);
        let header = self.render_header(key, item, detail, theme, tones, cx);
        if let (DetailTab::Code, Some(code)) = (state.tab, state.code.clone()) {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(header)
                .child(div().flex_1().min_h_0().child(code))
                .into_any_element();
        }
        let body: AnyElement = match &state.detail {
            Loadable::Ready(detail) if state.tab == DetailTab::Timeline => {
                self.render_timeline(key, detail, theme, tones, window, cx)
            }
            Loadable::Ready(detail) => self.render_summary(key, detail, theme, tones, window, cx),
            Loadable::Error(error) => self.render_detail_error(key, item, error, theme, cx),
            Loadable::Idle | Loadable::Loading => self.render_summary_ghost(theme),
        };
        let scroll = state.scroll.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id(SharedString::from(format!(
                        "pull-request-body-{}-{}-{}",
                        key.host, key.repository, key.number
                    )))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                    .child(body),
            )
            .into_any_element()
    }

    fn render_header(
        &self,
        key: &PullRequestKey,
        item: &ChangeRequestListItem,
        detail: Option<&ChangeRequestDetail>,
        theme: &Theme,
        tones: Tones,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.details[key];
        let condensed = state.condensed;
        let copied = state.copied.map(|(what, _)| what);
        let (_, tone) = tab_glyph(item, tones);
        let muted = theme.text_muted;
        let pr_url = item.url.clone();
        let repo_url = format!("https://{}/{}", item.host, item.repository);
        let number_link = |id: &'static str| {
            let url = pr_url.clone();
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(2.0))
                .flex_none()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(tone)
                .cursor_pointer()
                .hover(|style| style.underline())
                .on_click(move |_, _, cx| cx.open_url(&url))
                .child(format!("#{}", item.number))
        };
        let row_a_left: AnyElement = if condensed {
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .min_w_0()
                .text_size(px(12.0))
                .child(number_link("pull-request-number-condensed"))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(item.title.clone()),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .min_w_0()
                .text_size(px(12.0))
                .text_color(muted)
                .child(
                    div()
                        .id("pull-request-repository")
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(|style| style.underline().text_color(theme.text))
                        .on_click(move |_, _, cx| cx.open_url(&repo_url))
                        .child(item.repository.clone()),
                )
                .child(
                    number_link("pull-request-number").child(
                        icon(icons::LUCIDE_EXTERNAL_LINK)
                            .size(px(10.0))
                            .text_color(tone),
                    ),
                )
                .into_any_element()
        };
        let cluster = self.render_action_cluster(key, item, detail, theme, tones, cx);
        let row_a = div()
            .h(px(28.0))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .px(px(16.0))
            .child(div().flex_1().min_w_0().child(row_a_left))
            .child(cluster);

        let files = detail.map(|detail| detail.changed_files);
        let title = (!condensed).then(|| self.render_title(key, item, theme, cx));
        let mut warning = self.render_out_of_date(key, item, theme, cx);
        let mut branch = |small: bool| {
            let head_key = key.clone();
            let head = item.head_ref.clone();
            let head_copied = copied == Some("head");
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .min_w_0()
                .font_family(theme.font_mono.clone())
                .text_size(px(if small { 11.0 } else { 12.0 }))
                .text_color(muted.opacity(if small { 0.65 } else { 0.7 }))
                .child(
                    div()
                        .max_w(gpui::relative(0.4))
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(item.base_ref.clone()),
                )
                .children(warning.take())
                .child(
                    icon(icons::LUCIDE_ARROW_LEFT)
                        .size(px(if small { 12.0 } else { 14.0 }))
                        .text_color(muted.opacity(0.6))
                        .flex_none(),
                )
                .child(
                    div()
                        .id(if small {
                            "pull-request-head-condensed"
                        } else {
                            "pull-request-head"
                        })
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .rounded(px(4.0))
                        .px(px(4.0))
                        .py(px(2.0))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.ink(0.045)).text_color(theme.text))
                        .on_click(cx.listener(move |page, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(head.clone()));
                            page.mark_copied(&head_key, "head", cx);
                        }))
                        .child(if head_copied {
                            "Copied".to_string()
                        } else {
                            item.head_ref.clone()
                        }),
                )
        };
        let file_stat = |small: bool| {
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .flex_none()
                .text_size(px(if small { 11.0 } else { 12.0 }))
                .text_color(muted)
                .children(files.map(|files| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(
                            icon(icons::LUCIDE_FILE_DIFF)
                                .size(px(if small { 12.0 } else { 14.0 }))
                                .text_color(muted),
                        )
                        .child(if small {
                            files.to_string()
                        } else if files == 1 {
                            "1 file".to_string()
                        } else {
                            format!("{files} files")
                        })
                }))
                .when(item.additions + item.deletions > 0, |el| {
                    el.child(div().font_family(theme.font_mono.clone()).child(diff_stat(
                        item.additions,
                        item.deletions,
                        tones,
                        theme,
                    )))
                })
        };
        let now = chrono::Utc::now();
        let mut header = div()
            .flex_none()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme.hairline(0.045))
            .child(row_a);
        if condensed {
            header = header.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(16.0))
                    .pt(px(4.0))
                    .pb(px(8.0))
                    .text_size(px(12.0))
                    .text_color(muted)
                    .children(
                        item.author
                            .as_ref()
                            .map(|author| avatar(&author.login, theme)),
                    )
                    .child(relative_time(now, item.updated_at))
                    .child(div().w(px(1.0)).h(px(12.0)).bg(theme.hairline(0.08)))
                    .child(div().flex_1().min_w_0().child(branch(true)))
                    .child(file_stat(true)),
            );
        } else {
            let checkout = format!("gh pr checkout {}", item.number);
            let checkout_key = key.clone();
            let checkout_copied = copied == Some("checkout");
            header = header.child(
                div()
                    .px(px(16.0))
                    .pb(px(16.0))
                    .mt(px(4.0))
                    .flex()
                    .flex_col()
                    .children(title)
                    .child(
                        div()
                            .mt(px(8.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(px(12.0))
                            .text_color(muted)
                            .children(item.author.as_ref().map(|author| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(avatar(&author.login, theme))
                                    .child(author.login.clone())
                            }))
                            .child(div().text_color(muted.opacity(0.5)).child("·"))
                            .child(format!("updated {}", relative_time(now, item.updated_at)))
                            .child(div().flex_1())
                            .child(
                                div()
                                    .id("pull-request-checkout-command")
                                    .font_family(theme.font_mono.clone())
                                    .rounded(px(4.0))
                                    .px(px(4.0))
                                    .py(px(2.0))
                                    .cursor_pointer()
                                    .hover(|style| {
                                        style.bg(theme.ink(0.045)).text_color(theme.text)
                                    })
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            checkout.clone(),
                                        ));
                                        page.mark_copied(&checkout_key, "checkout", cx);
                                    }))
                                    .child(if checkout_copied {
                                        "Copied".to_string()
                                    } else {
                                        format!("gh pr checkout {}", item.number)
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(16.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(div().flex_1().min_w_0().child(branch(false)))
                            .child(file_stat(false)),
                    ),
            );
        }
        let checks_summary = detail.map(|detail| {
            let (glyph, tone) = match detail.item.checks_state {
                Some(state) => tones.checks(state),
                None => (icons::LUCIDE_CIRCLE_DOT, muted),
            };
            let popover_item = detail.item.clone();
            div()
                .id("pull-request-header-checks")
                .role(gpui::Role::Button)
                .aria_label("Checks")
                .cursor_pointer()
                .on_click(cx.listener(move |page, event: &gpui::ClickEvent, _, cx| {
                    page.toggle_checks_popover(&popover_item, event.position(), true, cx);
                }))
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_size(px(12.0))
                .text_color(muted)
                .child(icon(glyph).size(px(14.0)).text_color(tone))
                .child(summarize_checks(&detail.checks))
        });
        header
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .border_t_1()
                    .border_color(theme.hairline(0.045))
                    .px(px(16.0))
                    .py(px(8.0))
                    .child(self.render_tabs(key, state.tab, theme, cx))
                    .child(div().flex_1())
                    .map(|el| {
                        if state.tab == DetailTab::Timeline {
                            el.child(self.render_timeline_controls(key, theme, tones, cx))
                        } else {
                            match detail.and_then(|detail| {
                                self.render_approve_workflows(key, detail, theme, cx)
                            }) {
                                Some(approve) => el.child(approve),
                                None => el.children(checks_summary),
                            }
                        }
                    }),
            )
            .into_any_element()
    }

    fn render_summary(
        &self,
        key: &PullRequestKey,
        detail: &ChangeRequestDetail,
        theme: &Theme,
        tones: Tones,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.details[key];
        let muted = theme.text_muted;
        let meta_row = |glyph: &'static str, label: &'static str, value: AnyElement| {
            div()
                .min_h(px(32.0))
                .py(px(6.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(px(12.0))
                .child(
                    div()
                        .w(px(96.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_color(muted)
                        .child(icon(glyph).size(px(14.0)).text_color(muted))
                        .child(label),
                )
                .child(div().flex_1().min_w_0().text_color(theme.text).child(value))
        };
        let none = || div().text_color(muted).child("None").into_any_element();
        let reviewers: AnyElement = if detail.reviewers.is_empty() {
            none()
        } else {
            div()
                .flex()
                .items_center()
                .children(
                    detail
                        .reviewers
                        .iter()
                        .enumerate()
                        .map(|(index, reviewer)| {
                            let ring = match reviewer.verdict {
                                Some(ChangeRequestVerdict::Approved) => tones.open,
                                Some(ChangeRequestVerdict::ChangesRequested) => tones.conflict,
                                None => theme.bg,
                            };
                            let ring = if reviewer.stale {
                                ring.opacity(0.35)
                            } else {
                                ring
                            };
                            div()
                                .when(index > 0, |el| el.ml(px(-4.0)))
                                .rounded_full()
                                .border_2()
                                .border_color(ring)
                                .child(avatar(&reviewer.login, theme))
                        }),
                )
                .into_any_element()
        };
        let labels: AnyElement = if detail.item.labels.is_empty() {
            none()
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap(px(4.0))
                .children(detail.item.labels.iter().map(|label| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .max_w(px(192.0))
                        .rounded_full()
                        .border_1()
                        .border_color(theme.hairline(0.08))
                        .bg(theme.ink(0.025))
                        .pl(px(6.0))
                        .pr(px(8.0))
                        .py(px(2.0))
                        .child(
                            div()
                                .size(px(8.0))
                                .flex_none()
                                .rounded_full()
                                .bg(label_color(label.color.as_deref(), theme)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(label.name.clone()),
                        )
                }))
                .into_any_element()
        };
        let comment_label = self.comments_meta(key);
        let meta = div()
            .px(px(16.0))
            .py(px(12.0))
            .child(meta_row(
                icons::LUCIDE_USERS,
                "Reviewers",
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(reviewers)
                    .child(self.render_picker(key, false, theme, cx))
                    .into_any_element(),
            ))
            .child(meta_row(
                icons::LUCIDE_TAG,
                "Labels",
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(labels)
                    .child(self.render_picker(key, true, theme, cx))
                    .into_any_element(),
            ))
            .child(meta_row(
                icons::LUCIDE_MESSAGE_SQUARE,
                "Comments",
                div().child(comment_label).into_any_element(),
            ));

        let mut options = crate::markdown::render::RenderOptions::settled(SharedString::from(
            format!("pull-request-body-{}", key.number),
        ));
        options.link = None;
        let editor = self.description_editor(key, theme, cx);
        let pencil = self.description_pencil(key, &detail.body, theme, cx);
        let description: AnyElement = match (&state.body, editor) {
            (_, Some(editor)) => editor,
            (Some(tree), None) => div()
                .child(crate::markdown::render::render_tree(
                    tree,
                    &options,
                    theme,
                    window,
                    &|_| None,
                ))
                .children(
                    self.description_reactions(key, theme, cx)
                        .map(|bar| div().mt(px(8.0)).child(bar)),
                )
                .into_any_element(),
            (None, None) => div().into_any_element(),
        };
        let checks: AnyElement = if detail.checks.is_empty() {
            div()
                .text_size(px(12.0))
                .text_color(muted)
                .child("No checks reported.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .children(detail.checks.iter().enumerate().map(|(index, check)| {
                    let (glyph, tone, label) = check_presentation(check.status, tones, theme);
                    let url = check.url.clone();
                    div()
                        .id(("pull-request-check", index))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(8.0))
                        .text_size(px(12.0))
                        .when(url.is_some(), |el| {
                            el.cursor_pointer()
                                .hover(|style| style.bg(theme.ink(0.035)))
                        })
                        .on_click(move |_, _, cx| {
                            if let Some(url) = &url {
                                cx.open_url(url);
                            }
                        })
                        .child(icon(glyph).size(px(14.0)).text_color(tone).flex_none())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_color(theme.text)
                                .child(check.name.clone()),
                        )
                        .child(div().text_color(muted).child(label))
                }))
                .into_any_element()
        };
        let comments = self.render_comments(key, detail, theme, tones, window, cx);
        let comments_order = self.render_comments_order(key, theme, cx);
        div()
            .flex()
            .flex_col()
            .pb(px(32.0))
            .child(meta)
            .child(self.render_section(
                key,
                Section::Description,
                "Description",
                None,
                pencil,
                description,
                theme,
                cx,
            ))
            .child(self.render_section(
                key,
                Section::Checks,
                "Checks",
                Some(detail.checks.len() as u64),
                None,
                checks,
                theme,
                cx,
            ))
            .child(self.render_section(
                key,
                Section::Comments,
                "Comments",
                self.comment_count(key),
                comments_order,
                comments,
                theme,
                cx,
            ))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_section(
        &self,
        key: &PullRequestKey,
        section: Section,
        title: &'static str,
        count: Option<u64>,
        extra: Option<AnyElement>,
        body: AnyElement,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = !self.details[key].closed_sections.contains(&section);
        let toggle_key = key.clone();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(SharedString::from(format!("pull-request-section-{title}")))
                    .role(gpui::Role::Button)
                    .aria_label(title)
                    .border_t_1()
                    .border_color(theme.hairline(0.045))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(16.0))
                    .py(px(12.0))
                    .text_size(px(14.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .cursor_pointer()
                    .on_click(cx.listener(move |page, _, _, cx| {
                        if let Some(state) = page.details.get_mut(&toggle_key) {
                            if let Some(index) = state
                                .closed_sections
                                .iter()
                                .position(|closed| *closed == section)
                            {
                                state.closed_sections.remove(index);
                            } else {
                                state.closed_sections.push(section);
                            }
                            cx.notify();
                        }
                    }))
                    .child(title)
                    .child(
                        icon(icons::LUCIDE_CHEVRON_RIGHT)
                            .size(px(14.0))
                            .text_color(theme.text_muted)
                            .when(open, |svg| {
                                svg.with_transformation(gpui::Transformation::rotate(
                                    gpui::percentage(0.25),
                                ))
                            }),
                    )
                    .children(count.map(|count| {
                        div()
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_color(theme.text_muted)
                            .child(count.to_string())
                    }))
                    .when(open, |el| {
                        el.children(extra.map(|extra| div().ml_auto().child(extra)))
                    }),
            )
            .when(open, |el| {
                el.child(div().px(px(16.0)).pb(px(16.0)).child(body))
            })
            .into_any_element()
    }

    fn render_summary_ghost(&self, theme: &Theme) -> AnyElement {
        let bar = theme.text_muted.opacity(0.15);
        div()
            .px(px(16.0))
            .py(px(12.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .children([0.5, 0.35, 0.4].map(|width| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().w(px(96.0)).h(px(12.0)).rounded(px(4.0)).bg(bar))
                    .child(
                        div()
                            .h(px(12.0))
                            .w(gpui::relative(width))
                            .rounded(px(4.0))
                            .bg(bar),
                    )
            }))
            .child(
                div()
                    .mt(px(12.0))
                    .h(px(12.0))
                    .w(gpui::relative(0.8))
                    .rounded(px(4.0))
                    .bg(bar),
            )
            .child(
                div()
                    .h(px(12.0))
                    .w(gpui::relative(0.65))
                    .rounded(px(4.0))
                    .bg(bar),
            )
            .child(
                div()
                    .h(px(12.0))
                    .w(gpui::relative(0.7))
                    .rounded(px(4.0))
                    .bg(bar),
            )
            .into_any_element()
    }

    fn render_detail_error(
        &self,
        key: &PullRequestKey,
        item: &ChangeRequestListItem,
        error: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let retry_key = key.clone();
        let url = item.url.clone();
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.0))
            .px(px(16.0))
            .py(px(48.0))
            .child(
                div()
                    .text_size(px(16.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child("Could not load this pull request"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme.text_muted)
                    .text_center()
                    .child(error.to_string()),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        super::state_button(
                            "pull-request-detail-retry",
                            icons::LUCIDE_REFRESH_CW,
                            "Retry",
                            theme,
                        )
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.load_detail(&retry_key, cx);
                            cx.notify();
                        })),
                    )
                    .child(
                        super::state_button(
                            "pull-request-detail-open",
                            icons::LUCIDE_EXTERNAL_LINK,
                            "Open on GitHub",
                            theme,
                        )
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                    ),
            )
            .into_any_element()
    }
}

pub(super) fn verdict_badge(
    glyph: &'static str,
    label: &'static str,
    tone: gpui::Hsla,
) -> AnyElement {
    div()
        .h(px(16.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .px(px(5.0))
        .rounded(px(4.0))
        .bg(tone.opacity(0.16))
        .text_size(px(10.0))
        .text_color(tone)
        .child(icon(glyph).size(px(12.0)).text_color(tone))
        .child(label)
        .into_any_element()
}

impl PullRequestsPage {
    fn render_tabs(
        &self,
        key: &PullRequestKey,
        current: DetailTab,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tabs = [
            ("Summary", Some(DetailTab::Summary)),
            ("Timeline", Some(DetailTab::Timeline)),
            ("Code", Some(DetailTab::Code)),
        ];
        div()
            .id("pull-request-detail-tabs")
            .role(gpui::Role::TabList)
            .flex()
            .gap(px(2.0))
            .rounded(px(10.0))
            .bg(theme.ink(0.036))
            .p(px(2.0))
            .children(tabs.map(|(label, tab)| {
                let active = tab == Some(current);
                let key = key.clone();
                div()
                    .id(SharedString::from(format!(
                        "pull-request-detail-tab-{label}"
                    )))
                    .role(gpui::Role::Tab)
                    .aria_label(label)
                    .h(px(24.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .rounded(px(8.0))
                    .text_size(px(12.0))
                    .when(active, |el| el.bg(theme.ink(0.065)).text_color(theme.text))
                    .when(!active && tab.is_some(), |el| {
                        el.text_color(theme.text_muted)
                            .cursor_pointer()
                            .hover(|style| style.text_color(theme.text))
                    })
                    .when(tab.is_none(), |el| {
                        el.text_color(theme.text_muted.opacity(0.5))
                    })
                    .when_some(tab.filter(|_| !active), |el, tab| {
                        el.on_click(
                            cx.listener(move |page, _, _, cx| page.show_tab(&key, tab, None, cx)),
                        )
                    })
                    .child(label)
            }))
            .into_any_element()
    }
}

fn tab_glyph(item: &ChangeRequestListItem, tones: Tones) -> (&'static str, gpui::Hsla) {
    match item.state {
        ChangeRequestState::Merged => (icons::LUCIDE_GIT_MERGE, tones.merged),
        ChangeRequestState::Closed => (icons::LUCIDE_GIT_PULL_REQUEST_CLOSED, tones.closed),
        ChangeRequestState::Open if item.is_draft => {
            (icons::LUCIDE_GIT_PULL_REQUEST_DRAFT, tones.draft)
        }
        ChangeRequestState::Open => (icons::LUCIDE_GIT_PULL_REQUEST, tones.open),
    }
}

pub(super) fn clamp_panel_width(width: f32, viewport: f32) -> f32 {
    let max = (0.7 * viewport).floor().min(viewport - PANEL_MIN_WIDTH);
    width.min(max).max(PANEL_MIN_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(status: ChangeRequestCheckStatus, url: Option<&str>) -> ChangeRequestCheck {
        ChangeRequestCheck {
            name: "check".into(),
            status,
            description: None,
            url: url.map(str::to_owned),
            workflow_name: None,
        }
    }

    #[test]
    fn checks_summary_uses_t3code_precedence_and_copy() {
        use ChangeRequestCheckStatus as S;
        assert_eq!(summarize_checks(&[]), "No checks reported");
        assert_eq!(
            summarize_checks(&[
                check(S::Success, None),
                check(S::Failure, None),
                check(S::Pending, None)
            ]),
            "1 of 3 failing"
        );
        let approval = "https://github.com/a/b/actions/runs/123";
        assert_eq!(
            summarize_checks(&[
                check(S::ActionRequired, Some(approval)),
                check(S::ActionRequired, Some("https://ci/x")),
                check(S::ActionRequired, Some("https://ci/y")),
            ]),
            "1 workflow and 2 checks awaiting action"
        );
        assert_eq!(
            summarize_checks(&[check(S::ActionRequired, Some(&format!("{approval}/job/9")))]),
            "1 workflow awaiting approval"
        );
        assert_eq!(
            summarize_checks(&[check(S::ActionRequired, Some("https://ci/x"))]),
            "1 check awaiting action"
        );
        assert_eq!(
            summarize_checks(&[check(S::Success, None), check(S::Pending, None)]),
            "1 of 2 running"
        );
        assert_eq!(
            summarize_checks(&[check(S::Success, None), check(S::Success, None)]),
            "All checks passed"
        );
        assert_eq!(
            summarize_checks(&[check(S::Success, None), check(S::Skipped, None)]),
            "1 of 2 passing"
        );
    }

    #[test]
    fn workflow_approval_needs_a_run_id_segment() {
        use ChangeRequestCheckStatus as S;
        assert!(is_workflow_approval(&check(
            S::ActionRequired,
            Some("https://h/o/r/actions/runs/42")
        )));
        assert!(!is_workflow_approval(&check(
            S::ActionRequired,
            Some("https://h/o/r/actions/runs/42x")
        )));
        assert!(!is_workflow_approval(&check(
            S::Success,
            Some("https://h/o/r/actions/runs/42")
        )));
    }

    #[test]
    fn html_comments_and_tags_drop_out_but_code_keeps_them() {
        let body = "Intro <b>bold</b> text.\n<!-- hidden\nstill hidden -->\n<picture><img src=\"x\"></picture>\nUse `<div>` here.\n```html\n<div>kept</div>\n```\na < b\n";
        assert_eq!(
            strip_html(body),
            "Intro bold text.\nUse `<div>` here.\n```html\n<div>kept</div>\n```\na < b\n"
        );
    }

    #[test]
    fn panel_width_clamps_to_t3code_bounds() {
        assert_eq!(clamp_panel_width(100.0, 1500.0), 360.0);
        assert_eq!(clamp_panel_width(5000.0, 1500.0), 1050.0);
        assert_eq!(clamp_panel_width(5000.0, 1000.0), 640.0);
        assert_eq!(clamp_panel_width(700.0, 1500.0), 700.0);
    }

    #[test]
    fn review_states_read_as_sentence_case() {
        assert_eq!(verdict_word("COMMENTED"), "Commented");
        assert_eq!(verdict_word("CHANGES_REQUESTED"), "Changes requested");
        assert_eq!(verdict_word("review-dismissed"), "Review dismissed");
    }
}
