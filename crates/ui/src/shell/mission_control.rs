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
            MissionIndexEvent::NewMission => self.new_mission(cx),
            MissionIndexEvent::OpenTree(root) => self.open_task_center(root.clone(), cx),
            MissionIndexEvent::OpenTreeAt(root, selection) => {
                let center = self.task_center_for(root.clone(), cx);
                center.update(cx, |center, cx| center.select(selection.clone(), cx));
                self.open_task_center(root.clone(), cx);
            }
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
}
