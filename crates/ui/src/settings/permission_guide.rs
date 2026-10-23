//! The one macOS permission guide, shared by Settings › Appshots and Settings › Computer.
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{AnyElement, Context, SharedString, Window, div, prelude::*, px};

use crate::appshots::{AppshotCapabilities, CapabilityState};
use crate::motion;
use crate::settings::widgets;
use crate::theme::Theme;

/// The open/close motion of an expanded permission guide (Synara's
/// `DISCLOSURE_TRANSITION_MS` grid collapse: 220ms ease-out, height + fade,
/// content drifting 4px).
const GUIDE_MOTION: motion::MotionSpec = motion::MotionSpec::new(220, motion::EASE_OUT);
/// Height used for the very first reveal frame, before the guide's content has
/// been measured once.
const GUIDE_FALLBACK_HEIGHT: f32 = 240.0;
/// macOS fires no event when a TCC grant changes; an open guide re-reads the
/// preflight state on this cadence until the grant shows up.
const GUIDE_POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPane {
    InputMonitoring,
    ScreenRecording,
    Accessibility,
}

impl PermissionPane {
    pub fn label(self) -> &'static str {
        match self {
            Self::InputMonitoring => "Input Monitoring",
            Self::ScreenRecording => "Screen Recording",
            Self::Accessibility => "Accessibility",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::InputMonitoring => "input-monitoring",
            Self::ScreenRecording => "screen-recording",
            Self::Accessibility => "accessibility",
        }
    }

    pub fn state(self, capabilities: AppshotCapabilities) -> CapabilityState {
        match self {
            Self::InputMonitoring => capabilities.input_monitoring,
            Self::ScreenRecording => capabilities.window_capture,
            Self::Accessibility => capabilities.application_text,
        }
    }

    pub fn settings_url(self) -> Option<&'static str> {
        match self {
            Self::InputMonitoring => crate::appshots::input_monitoring_settings_url(),
            Self::ScreenRecording => crate::appshots::capture_settings_url(),
            Self::Accessibility => crate::appshots::semantic_settings_url(),
        }
    }

    pub fn request(self) {
        match self {
            Self::InputMonitoring => crate::appshots::request_input_monitoring_access(),
            Self::ScreenRecording => crate::appshots::request_capture_access(),
            Self::Accessibility => crate::appshots::request_semantic_access(),
        }
    }

    pub fn from_permission(permission: zeron_proto::computer::ComputerPermission) -> Self {
        use zeron_proto::computer::ComputerPermission;
        match permission {
            ComputerPermission::Accessibility => Self::Accessibility,
            ComputerPermission::ScreenRecording => Self::ScreenRecording,
            ComputerPermission::InputMonitoring => Self::InputMonitoring,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::InputMonitoring => 0,
            Self::ScreenRecording => 1,
            Self::Accessibility => 2,
        }
    }
}

#[derive(Default)]
pub struct PermissionGuide {
    open: Option<PermissionPane>,
    /// (pane, tween start, opening). While closing, `open` is already `None`
    /// and the pane here is what still renders.
    tween: Option<(PermissionPane, Instant, bool)>,
    measured_height: Rc<Cell<f32>>,
    poll: Option<gpui::Task<()>>,
    prompted: [bool; 3],
    #[cfg(test)]
    pub rendered: Vec<PermissionPane>,
}

impl PermissionGuide {
    pub fn open_pane(&self) -> Option<PermissionPane> {
        self.open
    }

    pub fn prompted(&self, pane: PermissionPane) -> bool {
        self.prompted[pane.index()]
    }

    pub fn set_prompted(&mut self, pane: PermissionPane) {
        self.prompted[pane.index()] = true;
    }

    pub fn request_once(&mut self, pane: PermissionPane) {
        if !self.prompted(pane) {
            self.set_prompted(pane);
            pane.request();
        }
    }

    pub fn open<V: 'static>(&mut self, pane: PermissionPane, cx: &mut Context<V>) {
        self.open = Some(pane);
        self.tween = Some((pane, motion::now(), true));
        self.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(GUIDE_POLL_INTERVAL).await;
                let alive = this.update(cx, |_, cx| {
                    crate::appshots::refresh_registration();
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }

    pub fn close<V: 'static>(&mut self, cx: &mut Context<V>) {
        if let Some(pane) = self.open.take() {
            self.tween = Some((pane, motion::now(), false));
        }
        self.poll = None;
        cx.notify();
    }

    pub fn toggle<V: 'static>(&mut self, pane: PermissionPane, cx: &mut Context<V>) {
        if self.open == Some(pane) {
            self.close(cx);
        } else {
            self.open(pane, cx);
        }
    }

    pub fn close_if_granted<V: 'static>(
        &mut self,
        capabilities: AppshotCapabilities,
        cx: &mut Context<V>,
    ) -> Option<PermissionPane> {
        let pane = self
            .open
            .filter(|pane| pane.state(capabilities).is_ready())?;
        self.close(cx);
        Some(pane)
    }

    fn showing(&self, pane: PermissionPane) -> bool {
        self.open == Some(pane)
            || matches!(self.tween, Some((tween_pane, _, _)) if tween_pane == pane)
    }
}

pub trait PermissionGuideHost: Sized + 'static {
    fn permission_guide(&mut self) -> &mut PermissionGuide;

    fn guide_opening(&mut self) {}
}

pub fn permission_badge(theme: &Theme, state: CapabilityState, prompted: bool) -> gpui::Div {
    let (dot, label) = match state {
        CapabilityState::Ready | CapabilityState::UserSelection => (theme.success, "Granted"),
        CapabilityState::PermissionRequired if prompted => (theme.danger, "Denied"),
        CapabilityState::PermissionRequired => (theme.border, "Not requested yet"),
        CapabilityState::SetupRequired => (theme.border, "Set up"),
        CapabilityState::Checking => (theme.border, "Checking"),
        CapabilityState::Unavailable => (theme.border, "Unavailable"),
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text_muted)
        .child(div().size(px(6.0)).rounded_full().bg(dot))
        .child(SharedString::from(label))
}

/// Synara's `Button size="xs" variant="outline"`: 24px tall, bordered, quiet
/// hover wash. The caller adds id, label child and click.
pub fn outline_button(theme: &Theme) -> gpui::Div {
    div()
        .flex_none()
        .h(px(24.0))
        .px(px(7.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(4.0))
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text)
        .cursor_pointer()
        .hover(|s| s.bg(crate::theme::ink(0.05)))
}

pub fn choice_chip(theme: &Theme, selected: bool) -> gpui::Div {
    div()
        .h(px(24.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(if selected {
            theme.text.opacity(0.24)
        } else {
            theme.border
        })
        .bg(if selected {
            crate::theme::ink(0.09)
        } else {
            gpui::transparent_black()
        })
        .flex()
        .items_center()
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(if selected {
            theme.text
        } else {
            theme.text_muted
        })
        .cursor_pointer()
}

pub fn choice_button(
    theme: &Theme,
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    selected: bool,
) -> gpui::Stateful<gpui::Div> {
    let label = label.into();
    let accent = theme.accent;
    choice_chip(theme, selected)
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .aria_toggled(if selected {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .tab_index(0)
        .focus_visible(move |style| style.border_2().border_color(accent))
        .child(label)
}

pub fn section_label(theme: &Theme, title: &'static str) -> gpui::Div {
    div()
        .mt(px(24.0))
        .px(px(8.0))
        .py(px(4.0))
        .text_size(crate::typography::ui_rems(13.0))
        .text_color(theme.text_muted.opacity(0.58))
        .child(SharedString::from(title))
}

/// Synara's `SettingsRow`: title (+ optional reset glyph) and description on
/// the left, an optional status line under them, the control on the right.
/// `children` render full-width under the row (the expanded guide).
#[allow(clippy::too_many_arguments)]
pub fn row(
    theme: &Theme,
    first: bool,
    title: impl Into<SharedString>,
    description: &str,
    status: Option<SharedString>,
    reset: Option<AnyElement>,
    control: AnyElement,
    children: Option<AnyElement>,
) -> gpui::Div {
    let heading = div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .min_h(px(20.0))
        .child(widgets::row_title(theme, title))
        .children(reset);
    let mut copy = div()
        .flex_1()
        .min_w(px(160.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(heading)
        .child(
            div()
                .text_size(crate::typography::ui_rems(12.0))
                .line_height(px(18.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(description.to_string())),
        );
    if let Some(status) = status {
        copy = copy.child(
            div()
                .pt(px(4.0))
                .text_size(crate::typography::ui_rems(11.5))
                .text_color(theme.text_muted)
                .child(status),
        );
    }
    let mut container = div()
        .when(!first, |el| el.border_t_1().border_color(theme.border))
        .flex()
        .flex_col();
    container = container.child(
        div()
            .px(px(20.0))
            .py(px(14.0))
            .hover(|s| s.bg(crate::theme::ink(0.015)))
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(14.0))
            .child(copy)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(control),
            ),
    );
    container.children(children)
}

pub fn permission_rows<V: PermissionGuideHost>(
    this: &mut V,
    slug: &'static str,
    panes: &[(PermissionPane, &'static str)],
    capabilities: AppshotCapabilities,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Vec<AnyElement> {
    let accent = theme.accent;
    panes
        .iter()
        .copied()
        .enumerate()
        .map(|(ix, (pane, description))| {
            let guide = this.permission_guide();
            #[cfg(test)]
            guide.rendered.push(pane);
            let state = pane.state(capabilities);
            let guide_open = guide.open == Some(pane);
            let showing = guide.showing(pane);
            let mut control = div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(permission_badge(theme, state, guide.prompted(pane)));
            if !state.is_ready() && pane.settings_url().is_some() {
                let label: &'static str = if guide_open { "Hide steps" } else { "Grant" };
                control = control.child(
                    outline_button(theme)
                        .id((SharedString::from(format!("{slug}-permission-grant")), ix))
                        .debug_selector(move || format!("{slug}-permission-grant-{}", pane.slug()))
                        .role(gpui::Role::Button)
                        .aria_label(format!("{label}: {}", pane.label()))
                        .tab_index(0)
                        .focus_visible(move |style| style.border_2().border_color(accent))
                        .on_click(cx.listener(move |this: &mut V, _, _, cx| {
                            if this.permission_guide().open != Some(pane) {
                                this.guide_opening();
                            }
                            let guide = this.permission_guide();
                            guide.set_prompted(pane);
                            // Grant asks macOS first, which registers this
                            // build in the pane's list, then deep-links the
                            // pane and expands the steps; Hide steps only
                            // collapses them.
                            if guide.open != Some(pane) {
                                pane.request();
                                if let Some(url) = pane.settings_url() {
                                    cx.open_url(url);
                                }
                            }
                            guide.toggle(pane, cx);
                        }))
                        .child(label),
                );
            }
            let expanded = showing.then(|| {
                render_guide(
                    this.permission_guide(),
                    slug,
                    pane,
                    state.is_ready(),
                    theme,
                    window,
                    cx,
                )
            });
            row(
                theme,
                ix == 0,
                pane.label(),
                description,
                None,
                None,
                control.into_any_element(),
                expanded,
            )
            .debug_selector(move || format!("{slug}-permission-row-{}", pane.slug()))
            .into_any_element()
        })
        .collect()
}

fn render_guide<V: 'static>(
    guide: &mut PermissionGuide,
    slug: &'static str,
    pane: PermissionPane,
    granted: bool,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<V>,
) -> AnyElement {
    let accent = theme.accent;
    let app = crate::appshots::app_display_name();
    let open_label: SharedString = format!("Open {} settings", pane.label()).into();
    let numbered = |ix: usize, body: AnyElement| {
        div()
            .flex()
            .items_start()
            .gap(px(10.0))
            .child(
                div()
                    .flex_none()
                    .mt(px(2.0))
                    .size(px(20.0))
                    .rounded_full()
                    .border_1()
                    .border_color(theme.border)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(crate::typography::ui_rems(11.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(SharedString::from((ix + 1).to_string())),
            )
            .child(
                div()
                    .min_h(px(24.0))
                    .flex_1()
                    .text_size(crate::typography::ui_rems(12.5))
                    .line_height(px(24.0))
                    .text_color(theme.text_muted)
                    .child(body),
            )
    };
    let step_text = |copy: String| {
        div()
            .line_height(px(20.0))
            .child(SharedString::from(copy))
            .into_any_element()
    };
    let mut footer = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .pt(px(12.0))
        .border_t_1()
        .border_color(theme.border);
    if granted {
        footer = footer.child(
            div()
                .text_size(crate::typography::ui_rems(12.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.success)
                .child("Permission granted."),
        );
    } else {
        footer = footer
            .child(crate::loaders::mini_mono_spinner(
                format!("{slug}-guide-spinner-{}", pane.label()),
                3.0,
                theme.text_muted,
                cx.entity_id(),
                cx,
            ))
            .child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child("Watching for the change — this page updates automatically."),
            );
    }
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.bg)
        .p(px(16.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(numbered(
                    0,
                    div()
                        .flex()
                        .child(
                    outline_button(theme)
                        .id((SharedString::from(format!("{slug}-guide-open-settings")), pane as usize))
                        .role(gpui::Role::Button)
                        .aria_label(open_label.clone())
                        .tab_index(0)
                        .focus_visible(move |style| style.border_2().border_color(accent))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            if let Some(url) = pane.settings_url() {
                                cx.open_url(url);
                            }
                        }))
                        .child(open_label),
                        )
                        .into_any_element(),
                ))
                .child(numbered(
                    1,
                    step_text(format!(
                        "If this copy of {app} is already listed, turn it on. Otherwise, use + to choose this installed copy, then turn it on."
                    )),
                ))
                .child(numbered(
                    2,
                    step_text(
                        "Complete any macOS authentication. If macOS asks you to quit and reopen, do so before checking again.".into(),
                    ),
                )),
        )
        .child(footer)
        .child(
            div()
                .text_size(crate::typography::ui_rems(12.0))
                .line_height(px(18.0))
                .text_color(theme.text_muted)
                .child(
                    "Still denied after an update or rebuild? Remove this app from the list and add this copy again. Complete any macOS authentication, and restart if macOS asks you to quit and reopen.",
                ),
        );
    if crate::appshots::can_restart_app() {
        let restart_label: SharedString = format!("Restart {app}").into();
        body = body.child(
            div().child(
                outline_button(theme)
                    .id((
                        SharedString::from(format!("{slug}-guide-restart")),
                        pane as usize,
                    ))
                    .role(gpui::Role::Button)
                    .aria_label(restart_label.clone())
                    .tab_index(0)
                    .focus_visible(move |style| style.border_2().border_color(accent))
                    .on_click(cx.listener(|_, _, _, cx| crate::appshots::restart_app(cx)))
                    .child(restart_label),
            ),
        );
    }

    // Grid-collapse reveal: the wrapper clips to a tweened height while the
    // content fades and drifts 4px, measured each frame so the target is
    // the guide's real height rather than a guess.
    let measured = guide.measured_height.clone();
    let target = match measured.get() {
        height if height > 0.0 => height,
        _ => GUIDE_FALLBACK_HEIGHT,
    };
    let mut progress = 1.0;
    if let Some((_, start, opening)) = guide.tween {
        let raw = (motion::since(start).as_secs_f32()
            / (GUIDE_MOTION.total().as_secs_f32() * motion::speed_scale()))
        .clamp(0.0, 1.0);
        if raw >= 1.0 || motion::reduced_motion(cx) {
            guide.tween = None;
            progress = if opening { 1.0 } else { 0.0 };
        } else {
            let eased = GUIDE_MOTION.progress(raw);
            progress = if opening { eased } else { 1.0 - eased };
            window.request_animation_frame();
        }
    }
    let content = div()
        .on_children_prepainted({
            let measured = measured.clone();
            move |bounds, _, _| {
                if let Some(first) = bounds.first() {
                    measured.set(f32::from(first.size.height));
                }
            }
        })
        .px(px(20.0))
        .pb(px(14.0))
        .pt(px(0.0))
        .opacity(progress)
        .top(px(-4.0 * (1.0 - progress)))
        .relative()
        .child(body);
    div()
        .w_full()
        .overflow_hidden()
        .h(px(target * progress))
        .child(content)
        .into_any_element()
}
