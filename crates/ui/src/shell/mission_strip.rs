use super::*;
use crate::tasks::Selection;

const STRIP_HEIGHT: f32 = 30.0;
pub(super) const STRIP_GAP: f32 = 6.0;
pub(super) const STRIP_BAND: f32 = STRIP_GAP + STRIP_HEIGHT;
const STRIP_REVEAL: motion::MotionSpec = motion::MotionSpec::new(300, motion::EASE);

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
        let now = self.strip_now();
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

    fn strip_now(&self) -> std::time::Instant {
        self.render_time.unwrap_or_else(|| self.motion_time())
    }
}
