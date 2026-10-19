use super::*;
use crate::tasks::{Selection, ZoomFrame};

const STRIP_HEIGHT: f32 = 30.0;
pub(super) const STRIP_GAP: f32 = 6.0;
pub(super) const STRIP_BAND: f32 = STRIP_GAP + STRIP_HEIGHT;
const STRIP_REVEAL: motion::MotionSpec = motion::MotionSpec::new(300, motion::EASE);

#[derive(Debug, Clone, Copy)]
pub(super) struct TaskZoom {
    entering: bool,
    reduced: bool,
    started: Option<std::time::Instant>,
}

impl TaskZoom {
    pub(super) fn new(entering: bool, reduced: bool) -> Self {
        Self {
            entering,
            reduced,
            started: None,
        }
    }
}

impl Shell {
    pub(super) fn render_mission_strip(
        &mut self,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Option<(AnyElement, f32)> {
        let (chat_id, mission) = {
            let state = self.state.read(cx);
            let chat_id = state.selected_chat.clone()?;
            let mission = crate::tasks::mission_thread(state, &chat_id, chrono::Utc::now());
            (chat_id, mission)
        };
        let reveal = self.strip_reveal(chat_id, mission.is_some());
        let mission = mission?;
        let theme = Theme::of(cx).clone();
        let (_, _, complete) = chat_status_tone(zeron_proto::ChatIndicator::Completed, &theme);
        let zeron_proto::PlanProgress { done, total } = mission.progress;
        let percent = (done * 100).checked_div(total).unwrap_or(0).min(100) as u8;
        let task_id = mission.task_id.clone();
        let selection = mission.role.selection();
        let strip = div()
            .id("mission-strip")
            .debug_selector(|| "mission-strip".into())
            .role(gpui::Role::Button)
            .aria_label(format!("Open Mission Control: {}", mission.title))
            .h(px(STRIP_HEIGHT))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(10.0))
            .bg(theme.wash(0.04))
            .border_1()
            .border_color(theme.border)
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text_muted)
            .cursor_pointer()
            .hover(|s| s.bg(theme.wash(0.08)))
            .child(div().flex_none().child(crate::loaders::upload_progress_arc(
                percent,
                13.0,
                theme.text_faint.opacity(0.6),
                complete,
            )))
            .child(
                div()
                    .min_w_0()
                    .max_w(gpui::relative(0.45))
                    .truncate()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(SharedString::from(mission.title)),
            )
            .child(
                div()
                    .debug_selector(|| "mission-strip-role".into())
                    .flex_none()
                    .whitespace_nowrap()
                    .child(mission.role.label()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(30.0))
                    .h(px(3.0))
                    .rounded(px(2.0))
                    .bg(theme.wash(0.08))
                    .overflow_hidden()
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(percent as f32 / 100.0))
                            .bg(complete),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .whitespace_nowrap()
                    .text_color(theme.text)
                    .child("Mission Control")
                    .child(
                        icon(icons::LUCIDE_CHEVRON_RIGHT)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    ),
            )
            .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
                shell.open_mission(task_id.clone(), selection.clone(), cx)
            }));
        Some((
            div()
                .w(px(width))
                .h(px(STRIP_HEIGHT * reveal))
                .px(px(Theme::SPACE_LG))
                .overflow_hidden()
                .opacity(reveal)
                .child(strip)
                .into_any_element(),
            reveal,
        ))
    }

    fn strip_reveal(&mut self, chat_id: String, shown: bool) -> f32 {
        let now = self.zoom_now();
        let target = if shown { 1.0 } else { 0.0 };
        let (seen, tween) = &mut self.task_slide.strip;
        if seen.as_deref() != Some(chat_id.as_str()) {
            *seen = Some(chat_id);
            *tween = motion::Tween::at(target, now);
        } else if !shown {
            *tween = motion::Tween::at(0.0, now);
        } else if tween.target() != target {
            tween.retarget(target, STRIP_REVEAL, self.reduced_motion, now);
        }
        if !tween.settled_at(now) {
            self.motion_active.set(true);
        }
        tween.value_at(now)
    }

    fn open_mission(&mut self, task_id: String, selection: Selection, cx: &mut Context<Self>) {
        let center = self.task_center_for(task_id.clone(), cx);
        center.update(cx, |center, cx| center.select(selection, cx));
        self.task_slide.armed = self.route == Route::Chat;
        self.open_task_center(task_id, cx);
    }

    pub(super) fn center_back_chat(&self, cx: &App) -> Option<String> {
        let state = self.state.read(cx);
        let id = match (self.route, self.nav.previous()) {
            (Route::Chat, _) => state.selected_chat.as_ref()?,
            (_, Some(NavEntry::Chat(id))) => id,
            _ => return None,
        };
        let chat = state.chats.iter().find(|chat| &chat.id == id)?;
        Some(transcript::single_line(&crate::home::chat_title(chat)))
    }

    pub(super) fn center_back(&mut self, cx: &mut Context<Self>) {
        if self.center_back_chat(cx).is_some() {
            self.navigate_back(cx);
        } else {
            self.back_to_tasks(cx);
        }
    }

    fn zoom_lands_on_mission(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        state
            .selected_chat
            .as_deref()
            .and_then(|chat| crate::tasks::mission_thread(state, chat, chrono::Utc::now()))
            .is_some_and(|mission| {
                Some(mission.task_id.as_str()) == self.task_center_task.as_deref()
            })
    }

    fn zoom_now(&self) -> std::time::Instant {
        self.render_time.unwrap_or_else(|| self.motion_time())
    }

    fn zoom_elapsed(&self, started: std::time::Instant) -> f32 {
        self.zoom_now()
            .saturating_duration_since(started)
            .as_secs_f32()
            * 1000.0
            / motion::speed_scale()
    }

    pub(super) fn task_zoom_fade(&mut self) -> f32 {
        let Some(zoom) = self.task_slide.zoom.filter(|zoom| zoom.reduced) else {
            return 1.0;
        };
        if (self.route == Route::TaskCenter) != zoom.entering {
            self.task_slide.zoom = None;
            return 1.0;
        }
        let started = zoom.started.unwrap_or_else(|| self.zoom_now());
        self.task_slide.zoom = Some(TaskZoom {
            started: Some(started),
            ..zoom
        });
        let total = task_actions::CENTER_FADE.total().as_millis() as f32;
        let raw = self.zoom_elapsed(started) / total;
        if raw >= 1.0 {
            self.task_slide.zoom = None;
            return 1.0;
        }
        self.motion_active.set(true);
        task_actions::CENTER_FADE.progress(raw)
    }

    fn task_zoom_frame(&mut self, cx: &App) -> Option<ZoomFrame> {
        let zoom = self.task_slide.zoom.filter(|zoom| !zoom.reduced)?;
        if (self.route == Route::TaskCenter) != zoom.entering {
            self.task_slide.zoom = None;
            return None;
        }
        let center = self.task_center.as_ref().map(|(center, _)| center.clone());
        let started = match zoom.started {
            Some(started) => Some(started),
            None if !zoom.entering && !self.zoom_lands_on_mission(cx) => {
                self.task_slide.zoom = None;
                return None;
            }
            None if center.is_some_and(|center| center.read(cx).zoom_ready()) => {
                Some(self.zoom_now())
            }
            None => None,
        };
        self.task_slide.zoom = Some(TaskZoom { started, ..zoom });
        let frame = ZoomFrame {
            entering: zoom.entering,
            elapsed: started.map_or(0.0, |started| self.zoom_elapsed(started)),
            backdrop: self
                .settings
                .inset_shell
                .then(|| Theme::of(cx).inset_tones().body),
        };
        if frame.settled() {
            self.task_slide.zoom = None;
            return None;
        }
        self.motion_active.set(true);
        Some(frame)
    }

    pub(super) fn render_task_zoom(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let zoom = self.task_slide.zoom?;
        if zoom.reduced {
            if self.route != Route::TaskCenter {
                return None;
            }
            let alpha = self.task_zoom_fade();
            let page = self.render_task_center_page(window, cx);
            return Some(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .opacity(alpha)
                    .child(page)
                    .into_any_element(),
            );
        }
        let frame = self.task_zoom_frame(cx);
        let center = self.task_center.as_ref().map(|(center, _)| center.clone());
        if let Some(center) = &center {
            center.update(cx, |center, _| center.set_zoom(frame));
        }
        if frame.is_none() && self.route != Route::TaskCenter {
            return None;
        }
        Some(self.render_task_center_page(window, cx))
    }

    pub(super) fn render_task_zoom_title_bar(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let zoom = self.task_slide.zoom.filter(|zoom| !zoom.reduced)?;
        if (self.route == Route::TaskCenter) != zoom.entering {
            return None;
        }
        let alpha = ZoomFrame {
            entering: zoom.entering,
            elapsed: zoom
                .started
                .map_or(0.0, |started| self.zoom_elapsed(started)),
            backdrop: None,
        }
        .titlebar();
        let bar = self.render_task_center_title_bar(cx);
        Some(div().opacity(alpha).child(bar).into_any_element())
    }
}
