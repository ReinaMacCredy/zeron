use super::*;
use crate::tasks::{MissionIndex, MissionIndexEvent, StripTreeEvent};

impl Shell {
    pub(crate) fn open_mission_control(&mut self, cx: &mut Context<Self>) {
        if matches!(self.route, Route::MissionControl) {
            return;
        }
        self.enter_mission_control_route(cx);
        self.nav.push(NavEntry::MissionControl);
    }

    pub(super) fn enter_mission_control_route(&mut self, cx: &mut Context<Self>) {
        self.set_route(Route::MissionControl, cx);
        self.close_user_menu(cx);
        self.close_chat_menu(cx);
        cx.notify();
    }

    pub(super) fn render_mission_control_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let index = match &self.mission_index {
            Some((index, _)) => index.clone(),
            None => {
                let index = cx.new(|cx| MissionIndex::new(self.state.clone(), cx));
                let events = cx.subscribe(&index, |shell, _, event: &MissionIndexEvent, cx| {
                    shell.on_mission_index_event(event, cx)
                });
                self.mission_index = Some((index.clone(), events));
                index
            }
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(index)
            .into_any_element()
    }

    fn on_mission_index_event(&mut self, event: &MissionIndexEvent, cx: &mut Context<Self>) {
        match event {
            MissionIndexEvent::OpenTree { root, task_id } => match task_id {
                Some(task_id) => self.open_task_center(task_id.clone(), cx),
                None => self.open_chat(root.clone(), cx),
            },
            MissionIndexEvent::Tree(StripTreeEvent::Open {
                chat_id: Some(chat_id),
                ..
            }) => self.open_chat(chat_id.clone(), cx),
            MissionIndexEvent::Tree(StripTreeEvent::OpenNative {
                parent_chat_id,
                doc_id,
                title,
                frozen,
            }) => self.open_native(
                parent_chat_id.clone(),
                doc_id.clone(),
                title.clone(),
                *frozen,
                cx,
            ),
            MissionIndexEvent::Tree(_) => {}
        }
    }

    pub(super) fn render_mission_control_title_bar(
        &mut self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let plus_inset = TITLEBAR_ACTION_SLOT_WIDTH * self.titlebar_plus_alpha(cx);
        let crumb = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(crate::typography::ui_rems(12.5))
            .text_color(theme.text)
            .child(
                icon(icons::MISSION_CONTROL)
                    .size(px(13.0))
                    .text_color(theme.text_muted),
            )
            .child("Mission Control");
        let inner = div()
            .size_full()
            .flex()
            .items_center()
            .pl(px((self.sidebar_edge() + Theme::SPACE_LG)
                .max(self.title_bar_content_start() + plus_inset)))
            .pr(px(self.titlebar_right_pad(TITLEBAR_ACTION_EDGE_INSET)))
            .child(crumb);
        let bar = div()
            .relative()
            .h(px(Theme::TITLEBAR_HEIGHT))
            .flex_none()
            .children(self.titlebar_seam_divider(cx))
            .child(inner);
        self.titlebar_drag_region("mission-control-titlebar", bar, cx)
            .into_any_element()
    }
}
