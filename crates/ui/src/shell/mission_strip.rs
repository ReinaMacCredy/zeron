use super::*;
use crate::tasks::Selection;

const STRIP_HEIGHT: f32 = 30.0;
pub(super) const STRIP_GAP: f32 = 6.0;
const STRIP_RADIUS: f32 = 10.0;
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
        let reveal = self.strip_reveal(chat_id.clone(), mission.is_some());
        let tree = mission
            .as_ref()
            .filter(|mission| mission.role == crate::tasks::MissionRole::Lead)
            .map(|_| self.mission_tree_for(&chat_id, cx));
        if tree.is_none() {
            let lead = mission.as_ref().and_then(|mission| {
                let state = self.state.read(cx);
                let task = state.tasks.iter().find(|task| task.id == mission.task_id)?;
                task.chat_id.clone()
            });
            let same_mission = self
                .mission_tree
                .as_ref()
                .is_some_and(|(tree, _)| Some(tree.read(cx).root_id()) == lead.as_deref());
            if !same_mission {
                self.mission_tree = None;
            }
        }
        let mission = mission?;
        let theme = Theme::of(cx).for_popup();
        let (_, _, complete) = chat_status_tone(zeron_proto::ChatIndicator::Completed, &theme);
        let zeron_proto::PlanProgress { done, total } = mission.progress;
        let percent = (done * 100).checked_div(total).unwrap_or(0).min(100) as u8;
        let task_id = mission.task_id.clone();
        let selection = mission.role.selection();
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
                .aria_label(format!("Open Mission Control: {}", mission.title))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(2.0))
                .whitespace_nowrap()
                .text_color(theme.text)
                .cursor_pointer()
                .child("Mission Control")
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
                None => format!("Open Mission Control: {}", mission.title),
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
            .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
                match shell.mission_tree.as_ref().map(|(tree, _)| tree.clone()) {
                    Some(tree) => shell.toggle_mission_tree(tree, cx),
                    None => shell.open_mission(task_id.clone(), selection.clone(), cx),
                }
            }));
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

    fn mission_tree_for(
        &mut self,
        lead_chat_id: &str,
        cx: &mut Context<Self>,
    ) -> Entity<crate::tasks::StripTree> {
        if let Some((tree, _)) = &self.mission_tree
            && tree.read(cx).root_id() == lead_chat_id
        {
            return tree.clone();
        }
        let open = !self.closed_mission_trees.contains(lead_chat_id);
        let state = self.state.clone();
        let root = lead_chat_id.to_string();
        let tree = cx.new(|cx| {
            crate::tasks::StripTree::new(state, root, true, open, crate::tasks::TreeHost::Strip, cx)
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
        self.mission_tree = Some((tree.clone(), events));
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

    fn toggle_mission_tree(
        &mut self,
        tree: Entity<crate::tasks::StripTree>,
        cx: &mut Context<Self>,
    ) {
        tree.update(cx, |tree, cx| {
            let open = !tree.is_open();
            tree.set_open(open, cx);
            if open {
                self.closed_mission_trees.remove(tree.root_id());
            } else {
                self.closed_mission_trees.insert(tree.root_id().to_string());
            }
        });
        cx.notify();
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
