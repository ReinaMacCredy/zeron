use super::*;
use crate::tasks::Selection;
use zeron_proto::TaskStatus;

const STRIP_HEIGHT: f32 = 30.0;
pub(super) const STRIP_GAP: f32 = 6.0;
const STRIP_RADIUS: f32 = 10.0;
const STRIP_REVEAL: motion::MotionSpec = motion::MotionSpec::new(300, motion::EASE);
const CHECK_HEAD: f32 = 36.0;
const CHECK_ROW: f32 = 30.0;
const CHECK_ROWS_SHOWN: usize = 6;
const CHECK_FOLD: motion::MotionSpec = motion::MotionSpec::new(240, motion::EASE_OUT_QUINT);
const CHECK_TICK: motion::MotionSpec = motion::MotionSpec::new(300, motion::EASE_OUT_QUINT);
const CHECK_BAR: motion::MotionSpec = motion::MotionSpec::new(320, motion::EASE_OUT_QUINT);

impl Shell {
    pub(super) fn render_mission_strip(
        &mut self,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Option<(AnyElement, f32)> {
        let (chat_id, mission, gathered) = {
            let state = self.state.read(cx);
            let chat_id = state.selected_chat.clone()?;
            let now = chrono::Utc::now();
            let mission = crate::tasks::mission_thread(state, &chat_id, now);
            let gathered = match mission {
                Some(_) => Vec::new(),
                None => crate::tasks::gathered_tasks(state, &chat_id, now),
            };
            (chat_id, mission, gathered)
        };
        let reveal = self.strip_reveal(chat_id.clone(), mission.is_some() || gathered.len() > 1);
        if mission.is_none() && gathered.len() > 1 {
            return Some(self.render_tasks_strip(width, reveal, chat_id, gathered, cx));
        }
        let tree = mission
            .as_ref()
            .filter(|mission| mission.role == crate::tasks::MissionRole::Lead)
            .map(|_| self.mission_tree_for(&chat_id, cx));
        let mission = mission?;
        let theme = Theme::of(cx).for_popup();
        let (_, _, complete) = chat_status_tone(zeron_proto::ChatIndicator::Completed, &theme);
        let zeron_proto::PlanProgress { done, total } = mission.progress;
        let percent = (done * 100).checked_div(total).unwrap_or(0).min(100) as u8;
        let task_id = mission.task_id.clone();
        let selection = crate::tasks::mission_selection(&mission.role);
        let tree_height = match &tree {
            Some(tree) => {
                let (height, moving) = tree.update(cx, |tree, cx| tree.band(cx));
                if moving {
                    self.motion_active.set(true);
                }
                height
            }
            None => 0.0,
        };
        let open_mission = {
            let task_id = task_id.clone();
            let selection = selection.clone();
            div()
                .id("mission-strip-open")
                .debug_selector(|| "mission-strip-open".into())
                .role(gpui::Role::Button)
                .aria_label(format!(
                    "{}: {}",
                    zeron_proto::mission::OPEN_MISSION_PAGE,
                    mission.title
                ))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(2.0))
                .whitespace_nowrap()
                .text_color(theme.text)
                .cursor_pointer()
                .child(zeron_proto::mission::MISSION_PAGE)
                .child(
                    icon(icons::LUCIDE_CHEVRON_RIGHT)
                        .size(px(12.0))
                        .text_color(theme.text_muted),
                )
                .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
                    cx.stop_propagation();
                    shell.open_mission(task_id.clone(), selection.clone(), cx)
                }))
        };
        let open = tree.as_ref().map(|tree| tree.read(cx).is_open());
        let head = div()
            .id("mission-strip")
            .debug_selector(|| "mission-strip".into())
            .role(gpui::Role::Button)
            .aria_label(match open {
                Some(true) => format!("Fold the steps of {}", mission.title),
                Some(false) => format!("Show the steps of {}", mission.title),
                None => format!(
                    "{}: {}",
                    zeron_proto::mission::OPEN_MISSION_PAGE,
                    mission.title
                ),
            })
            .when_some(open, |el, open| el.aria_expanded(open))
            .flex_none()
            .h(px(STRIP_HEIGHT))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .cursor_pointer()
            .hover(|s| s.bg(theme.wash(0.04)))
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
                    .child(SharedString::from(mission.title.clone())),
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
            .child(open_mission)
            .on_click({
                let tree = tree.clone();
                cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| match &tree {
                    Some(tree) => tree.update(cx, |tree, cx| tree.set_open(!tree.is_open(), cx)),
                    None => shell.open_mission(task_id.clone(), selection.clone(), cx),
                })
            });
        let card = div()
            .flex()
            .flex_col()
            .rounded(px(STRIP_RADIUS))
            .border_1()
            .border_color(theme.border)
            .bg(crate::popover::surface_bg(&theme))
            .overflow_hidden()
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text_muted)
            .child(head)
            .children(tree.map(|tree| {
                div()
                    .flex_none()
                    .h(px(tree_height))
                    .overflow_hidden()
                    .child(tree)
            }));
        let height = STRIP_HEIGHT + tree_height;
        Some((
            div()
                .w(px(width))
                .h(px(height * reveal))
                .px(px(Theme::SPACE_LG))
                .overflow_hidden()
                .opacity(reveal)
                .child(crate::frost::frosted(
                    STRIP_RADIUS,
                    crate::frost::MENU_BLUR * reveal,
                    card,
                ))
                .into_any_element(),
            (STRIP_GAP + height) * reveal,
        ))
    }

    fn render_tasks_strip(
        &mut self,
        width: f32,
        reveal: f32,
        chat_id: String,
        tasks: Vec<crate::tasks::Gathered>,
        cx: &mut Context<Self>,
    ) -> (AnyElement, f32) {
        let theme = Theme::of(cx).for_popup();
        let now = self.strip_now();
        let reduced = self.reduced_motion;
        let open = {
            let fold = self
                .task_slide
                .folded
                .entry(chat_id.clone())
                .or_insert_with(|| motion::Tween::at(1.0, now));
            if !fold.settled_at(now) {
                self.motion_active.set(true);
            }
            fold.value_at(now)
        };
        let unfolded = self
            .task_slide
            .folded
            .get(&chat_id)
            .is_none_or(|fold| fold.target() > 0.5);
        let total = tasks.len();
        let done = tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Done)
            .count();
        let (_, _, complete) = chat_status_tone(zeron_proto::ChatIndicator::Completed, &theme);
        let fraction = done as f32 / total as f32;
        let before = done.saturating_sub(1) as f32 / total as f32;
        let bar = div()
            .mx(px(12.0))
            .h(px(3.0))
            .rounded(px(2.0))
            .bg(theme.wash(0.08))
            .overflow_hidden()
            .child({
                let fill = div().h_full().bg(complete);
                if reduced || done == 0 {
                    fill.w(gpui::relative(fraction)).into_any_element()
                } else {
                    fill.with_animation(
                        SharedString::from(format!("tasks-strip-bar-{chat_id}-{done}")),
                        CHECK_BAR.animation(),
                        move |el, t| el.w(gpui::relative(motion::lerp(before, fraction, t))),
                    )
                    .into_any_element()
                }
            });
        let fold_chat = chat_id.clone();
        let head = div()
            .id("tasks-strip-head")
            .debug_selector(|| "tasks-strip".into())
            .role(gpui::Role::Button)
            .aria_expanded(unfolded)
            .aria_label(format!(
                "{total} tasks in this thread, {done} of {total} done"
            ))
            .h(px(CHECK_HEAD))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .cursor_pointer()
            .text_size(crate::typography::ui_rems(13.0))
            .child(
                icon(icons::CHECKLIST)
                    .size(px(15.0))
                    .text_color(theme.text_muted),
            )
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(format!("{total} tasks in this thread")),
            )
            .child(
                div()
                    .debug_selector(|| "tasks-strip-done".into())
                    .text_color(theme.text_muted)
                    .child(format!("{done} of {total} done")),
            )
            .child(div().flex_1())
            .child(
                div()
                    .size(px(24.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        icon(if unfolded {
                            icons::LUCIDE_CHEVRON_DOWN
                        } else {
                            icons::LUCIDE_CHEVRON_RIGHT
                        })
                        .size(px(14.0))
                        .text_color(theme.text_muted),
                    ),
            )
            .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
                let now = shell.strip_now();
                let reduced = shell.reduced_motion;
                if let Some(fold) = shell.task_slide.folded.get_mut(&fold_chat) {
                    let to = if fold.target() > 0.5 { 0.0 } else { 1.0 };
                    fold.retarget(to, CHECK_FOLD, reduced, now);
                }
                cx.notify();
            }));
        let rows: Vec<AnyElement> = tasks
            .iter()
            .enumerate()
            .map(|(ix, task)| self.render_strip_task(ix, task, &chat_id, reduced, &theme, cx))
            .collect();
        let rows_h = 6.0 + CHECK_ROW * total.min(CHECK_ROWS_SHOWN) as f32 + 8.0;
        let list = div().h(px(rows_h * open)).overflow_hidden().child(
            div()
                .id("tasks-strip-rows")
                .h(px(rows_h))
                .pt(px(6.0))
                .pb(px(8.0))
                .px(px(6.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .children(rows),
        );
        let height = CHECK_HEAD + 3.0 + 9.0 * (1.0 - open) + rows_h * open + 2.0;
        let card = div()
            .flex()
            .flex_col()
            .rounded(px(STRIP_RADIUS))
            .border_1()
            .border_color(theme.border_strong)
            .bg(crate::popover::surface_bg(&theme))
            .overflow_hidden()
            .child(head)
            .child(bar)
            .child(div().h(px(9.0 * (1.0 - open))))
            .child(list);
        (
            div()
                .w(px(width))
                .h(px(height * reveal))
                .px(px(Theme::SPACE_LG))
                .overflow_hidden()
                .opacity(reveal)
                .child(crate::frost::frosted(
                    STRIP_RADIUS,
                    crate::frost::MENU_BLUR * reveal,
                    card,
                ))
                .into_any_element(),
            (STRIP_GAP + height) * reveal,
        )
    }

    fn render_strip_task(
        &self,
        ix: usize,
        task: &crate::tasks::Gathered,
        chat_id: &str,
        reduced: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let done = task.status == TaskStatus::Done;
        let (_, glyph, tone) = crate::tasks::column_tone(task.status, theme);
        let status = zeron_proto::mission::column_title(task.status);
        let hover_key = SharedString::from(format!("tasks-strip-row-{}", task.id));
        let hovered = !done && motion::hover_t(&hover_key) > 0.5;
        let mark = icon(glyph).size(px(14.0)).text_color(tone);
        let mark = if done && !reduced {
            div()
                .size(px(14.0))
                .flex()
                .items_center()
                .justify_center()
                .child(mark)
                .with_animation(
                    SharedString::from(format!("tasks-strip-tick-{chat_id}-{}", task.id)),
                    CHECK_TICK.animation(),
                    |el, t| {
                        let scale = if t < 0.6 {
                            motion::lerp(0.6, 1.15, t / 0.6)
                        } else {
                            motion::lerp(1.15, 1.0, (t - 0.6) / 0.4)
                        };
                        el.size(px(14.0 * scale))
                    },
                )
                .into_any_element()
        } else {
            mark.into_any_element()
        };
        let open_id = task.id.clone();
        let done_id = task.id.clone();
        let title_id = task.id.clone();
        div()
            .id(hover_key.clone())
            .debug_selector(move || format!("tasks-strip-row-{title_id}"))
            .h(px(CHECK_ROW))
            .flex_none()
            .px(px(6.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(7.0))
            .text_size(crate::typography::ui_rems(13.0))
            .hover(|s| s.bg(theme.glass_hover()))
            .on_hover(motion::hover_listener(hover_key))
            .child(div().w(px(14.0)).flex().justify_center().child(mark))
            .child(
                div()
                    .w(px(14.0))
                    .text_right()
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_faint)
                    .child((ix + 1).to_string()),
            )
            .child(
                div()
                    .id(SharedString::from(format!("tasks-strip-open-{}", task.id)))
                    .min_w_0()
                    .max_w(px(320.0))
                    .truncate()
                    .cursor_pointer()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(if done { theme.text_muted } else { theme.accent })
                    .when(done, |el| el.line_through())
                    .child(task.title.clone())
                    .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, window, cx| {
                        shell.open_task(open_id.clone(), window, cx)
                    })),
            )
            .child(div().flex_1())
            .child(if hovered {
                div()
                    .id(SharedString::from(format!("tasks-strip-done-{}", task.id)))
                    .debug_selector(|| "tasks-strip-mark-done".into())
                    .role(gpui::Role::Button)
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_muted)
                    .hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
                    .child("Mark done")
                    .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
                        shell.update_task(
                            &done_id,
                            zeron_proto::TaskPatch {
                                completed: Some(true),
                                ..Default::default()
                            },
                            cx,
                        )
                    }))
                    .into_any_element()
            } else {
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(tone)
                    .child(status)
                    .into_any_element()
            })
            .into_any_element()
    }

    fn mission_tree_for(
        &mut self,
        lead_chat_id: &str,
        cx: &mut Context<Self>,
    ) -> Entity<crate::tasks::StripTree> {
        if let Some((tree, _)) = self.mission_trees.get(lead_chat_id) {
            return tree.clone();
        }
        let state = self.state.clone();
        let root = lead_chat_id.to_string();
        let tree = cx.new(|cx| {
            crate::tasks::StripTree::new(state, root, true, true, crate::tasks::TreeHost::Strip, cx)
        });
        let lead = lead_chat_id.to_string();
        let events = cx.subscribe(
            &tree,
            move |shell, _, event: &crate::tasks::StripTreeEvent, cx| match event {
                crate::tasks::StripTreeEvent::Open { selection, chat_id } => match chat_id {
                    Some(chat_id) => shell.open_chat(chat_id.clone(), cx),
                    None => shell.open_mission(lead.clone(), selection.clone(), cx),
                },
                crate::tasks::StripTreeEvent::OpenNative {
                    parent_chat_id,
                    doc_id,
                    title,
                    frozen,
                } => shell.open_native(
                    parent_chat_id.clone(),
                    doc_id.clone(),
                    title.clone(),
                    *frozen,
                    cx,
                ),
                crate::tasks::StripTreeEvent::Brief(_) => {}
            },
        );
        self.mission_trees
            .insert(lead_chat_id.to_string(), (tree.clone(), events));
        tree
    }

    pub(super) fn open_native(
        &mut self,
        parent_chat_id: String,
        doc_id: String,
        title: String,
        frozen: bool,
        cx: &mut Context<Self>,
    ) {
        self.open_chat(parent_chat_id.clone(), cx);
        self.add_subagent_surface(parent_chat_id, doc_id, title, frozen, cx);
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

    pub(super) fn open_mission(
        &mut self,
        task_id: String,
        selection: Selection,
        cx: &mut Context<Self>,
    ) {
        let key = super::task_actions::tree_key(self.state.read(cx), &task_id);
        let center = self.task_center_for(key.clone(), cx);
        center.update(cx, |center, cx| center.select(selection, cx));
        self.open_task_center(key, cx);
    }

    fn strip_now(&self) -> std::time::Instant {
        self.render_time.unwrap_or_else(|| self.motion_time())
    }
}
