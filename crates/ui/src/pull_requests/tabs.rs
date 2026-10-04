use gpui::{AnyElement, ClipboardItem, Context, SharedString, div, prelude::*, px};

use super::{PullRequestKey, PullRequestsPage};
use crate::popover::{self, Loadable};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CloseTabs {
    This,
    Others,
    ToTheRight,
    All,
}

const CLOSE_ROWS: [(CloseTabs, &str); 4] = [
    (CloseTabs::This, "Close"),
    (CloseTabs::Others, "Close others"),
    (CloseTabs::ToTheRight, "Close to the right"),
    (CloseTabs::All, "Close all"),
];

pub(super) fn tabs_closed_by(
    tabs: &[PullRequestKey],
    key: &PullRequestKey,
    which: CloseTabs,
) -> Vec<PullRequestKey> {
    let Some(at) = tabs.iter().position(|tab| tab == key) else {
        return Vec::new();
    };
    match which {
        CloseTabs::This => vec![key.clone()],
        CloseTabs::Others => tabs.iter().filter(|tab| *tab != key).cloned().collect(),
        CloseTabs::ToTheRight => tabs[at + 1..].to_vec(),
        CloseTabs::All => tabs.to_vec(),
    }
}

impl PullRequestsPage {
    pub(super) fn open_tab_menu(
        &mut self,
        key: PullRequestKey,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.tab_menu.open((key, position));
        cx.notify();
    }

    fn close_tab_menu(&mut self, cx: &mut Context<Self>) {
        if self.tab_menu.begin_close() {
            popover::reap_popup(cx, |page: &mut Self| &mut page.tab_menu);
            cx.notify();
        }
    }

    pub(super) fn close_tabs(
        &mut self,
        key: &PullRequestKey,
        which: CloseTabs,
        cx: &mut Context<Self>,
    ) {
        if which == CloseTabs::This {
            self.close_tab(key, cx);
            return;
        }
        let closing = tabs_closed_by(&self.tabs, key, which);
        self.tabs.retain(|tab| !closing.contains(tab));
        for tab in &closing {
            self.details.remove(tab);
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| closing.contains(active))
        {
            self.active = if self.tabs.contains(key) {
                Some(key.clone())
            } else {
                self.tabs.last().cloned()
            };
        }
        cx.notify();
    }

    pub fn close_active_tab(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(key) = self.active.clone() else {
            return false;
        };
        self.close_tab(&key, cx);
        true
    }

    pub fn copy_active_link(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(state) = self.active.as_ref().and_then(|key| self.details.get(key)) else {
            return false;
        };
        let url = match &state.detail {
            Loadable::Ready(detail) => detail.item.url.clone(),
            _ => state.seed().url.clone(),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
        self.toast_with("PR link copied", url, cx);
        true
    }

    pub(super) fn render_tab_menu(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (key, position) = self.tab_menu.get().cloned()?;
        let closing = self.tab_menu.closing_since();
        let theme = theme.for_popup();
        let at = self.tabs.iter().position(|tab| *tab == key);
        if at.is_none() && closing.is_none() {
            return None;
        }
        let count = self.tabs.len();
        let mut menu = popover::popover_card(&theme)
            .w(px(170.0))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|page, _, _, cx| page.close_tab_menu(cx)));
        for (which, label) in CLOSE_ROWS {
            let enabled = at.is_some_and(|at| match which {
                CloseTabs::Others => count > 1,
                CloseTabs::ToTheRight => at + 1 < count,
                CloseTabs::This | CloseTabs::All => true,
            });
            let target = key.clone();
            menu = menu.child(
                popover::menu_row(&theme, false, format!("pull-request-tab-menu-{label}"))
                    .id(SharedString::from(format!("pull-request-tab-menu-{label}")))
                    .debug_selector(move || format!("pull-request-tab-menu-{label}"))
                    .role(gpui::Role::MenuItem)
                    .aria_label(label)
                    .when(!enabled, |el| el.opacity(0.5).cursor_default())
                    .when(enabled, |el| {
                        el.on_click(cx.listener(move |page, _, _, cx| {
                            page.close_tab_menu(cx);
                            page.close_tabs(&target, which, cx);
                        }))
                    })
                    .child(div().child(label)),
            );
        }
        Some(popover::menu_at(
            "pull-request-tab-menu",
            position,
            menu.into_any_element(),
            closing,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(number: u64) -> zeron_proto::ChangeRequestListItem {
        serde_json::from_value(serde_json::json!({
            "provider": "github", "host": "github.com", "repository": "acme/zeron",
            "number": number, "title": format!("Change {number}"),
            "url": format!("https://github.com/acme/zeron/pull/{number}"),
            "headRef": "h", "baseRef": "main", "state": "open", "isDraft": false,
            "mergeability": "unknown", "additions": 0, "deletions": 0,
            "createdAt": "2026-10-01T00:00:00Z", "updatedAt": "2026-10-01T00:00:00Z",
            "viewerReviewRequested": false, "labels": []
        }))
        .unwrap()
    }

    #[gpui::test]
    fn right_click_close_others_keeps_only_the_clicked_tab(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        let state = cx.new(|_| crate::state::AppState::new());
        let (page, cx) = cx.add_window_view(|_, cx| PullRequestsPage::new(state, cx));
        page.update(cx, |page, cx| {
            for number in 1..=3 {
                page.open_pull_request(&item(number), cx);
            }
        });
        cx.run_until_parked();
        let tab = cx.debug_bounds("pull-request-tab-2").expect("tab #2 drawn");
        cx.simulate_event(gpui::MouseDownEvent {
            button: gpui::MouseButton::Right,
            position: tab.center(),
            modifiers: gpui::Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        cx.run_until_parked();
        let others = cx
            .debug_bounds("pull-request-tab-menu-Close others")
            .expect("menu open");
        cx.simulate_click(others.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        page.update(cx, |page, _| {
            let numbers: Vec<u64> = page.tabs.iter().map(|tab| tab.number).collect();
            assert_eq!(numbers, [2]);
            assert_eq!(page.active.as_ref().map(|key| key.number), Some(2));
        });
    }

    #[test]
    fn close_menu_rows_pick_the_tabs_t3code_closes() {
        let key = |number| PullRequestKey {
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            number,
            device: None,
        };
        let tabs = [key(1), key(2), key(3)];
        let numbers = |which| -> Vec<u64> {
            tabs_closed_by(&tabs, &key(2), which)
                .iter()
                .map(|tab| tab.number)
                .collect()
        };
        assert_eq!(numbers(CloseTabs::This), [2]);
        assert_eq!(numbers(CloseTabs::Others), [1, 3]);
        assert_eq!(numbers(CloseTabs::ToTheRight), [3]);
        assert_eq!(numbers(CloseTabs::All), [1, 2, 3]);
        assert!(tabs_closed_by(&tabs, &key(9), CloseTabs::All).is_empty());
    }
}
