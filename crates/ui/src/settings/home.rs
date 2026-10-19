use gpui::{Context, Window, div, prelude::*, px};

use super::permission_guide::{choice_button, row};
use super::widgets;
use crate::popover;
use crate::settings::{self, HomeWidgets};
use crate::theme::Theme;

pub(crate) type Field = fn(&mut HomeWidgets) -> &mut bool;

pub(crate) const WIDGETS: [(&str, &str, &str, Field); 8] = [
    (
        "home-toggle-working-now",
        "Working now",
        "Chats with a running turn, their elapsed time and current command.",
        |w| &mut w.working_now,
    ),
    (
        "home-toggle-missions",
        "Missions",
        "Running Mission Control tasks, their progress and what needs you.",
        |w| &mut w.missions,
    ),
    (
        "home-toggle-pull-requests",
        "Pull requests",
        "Pull requests waiting on your review and your own open ones.",
        |w| &mut w.pull_requests,
    ),
    (
        "home-toggle-git-activity",
        "Git activity",
        "Your commits over the last 14 days, across projects or one at a time.",
        |w| &mut w.git_activity,
    ),
    (
        "home-toggle-this-mac",
        "This Mac",
        "CPU, memory and disk use of this machine.",
        |w| &mut w.this_mac,
    ),
    (
        "home-toggle-running-services",
        "Running services",
        "Ports listening on this machine.",
        |w| &mut w.running_services,
    ),
    (
        "home-toggle-recent-areas",
        "Recent areas",
        "Projects and worktrees you worked in most recently.",
        |w| &mut w.recent_areas,
    ),
    (
        "home-toggle-ci-failures",
        "CI failures",
        "Failing checks on your open pull requests.",
        |w| &mut w.ci_failures,
    ),
];

#[derive(Default)]
pub struct HomeSettingsPage {
    scroll: widgets::PageScroll,
}

impl HomeSettingsPage {
    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }
}

impl popover::ScrollRailHost for HomeSettingsPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for HomeSettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let accent = theme.accent;
        let mut current = settings::home_widgets(cx);

        let widgets_card = widgets::section_card(&theme).mt(px(0.0)).children(
            WIDGETS
                .into_iter()
                .enumerate()
                .map(|(index, (id, title, description, field))| {
                    let on = *field(&mut current);
                    let toggle = widgets::toggle_switch(&theme, on, id)
                        .id(id)
                        .debug_selector(move || id.to_string())
                        .tab_index(0)
                        .role(gpui::Role::Switch)
                        .aria_label(title)
                        .aria_toggled(if on {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .focus_visible(move |style| style.border_2().border_color(accent))
                        .cursor_pointer()
                        .on_click(cx.listener(move |_, _, _, cx| {
                            settings::update_home_widgets(cx, |widgets| {
                                let value = field(widgets);
                                *value = !*value;
                            });
                            cx.notify();
                        }));
                    row(
                        &theme,
                        index == 0,
                        title,
                        description,
                        None,
                        None,
                        toggle.into_any_element(),
                        None,
                    )
                }),
        );

        let all_ports = current.services_all_ports;
        let mode = |id: &'static str, label: &'static str, value: bool| {
            choice_button(&theme, id, label, all_ports == value)
                .debug_selector(move || id.to_string())
                .on_click(cx.listener(move |_, _, _, cx| {
                    settings::update_home_widgets(cx, |widgets| widgets.services_all_ports = value);
                    cx.notify();
                }))
        };
        let services_card = widgets::section_card(&theme).mt(px(0.0)).child(row(
            &theme,
            true,
            "Show ports",
            "By Zeron lists servers running inside your Zeron projects and worktrees. All ports adds every other listening process on this Mac.",
            None,
            None,
            div()
                .flex()
                .gap(px(6.0))
                .child(mode("home-services-by-zeron", "By Zeron", false))
                .child(mode("home-services-all-ports", "All ports", true))
                .into_any_element(),
            None,
        ));

        let scrollbar = popover::rail(self, "home-settings-scrollbar", &theme, cx);
        div()
            .id("home-settings-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("home-settings-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(widgets::page_header(&theme, "Home", None))
                                .child(widgets::section(&theme, "Widgets", widgets_card))
                                .child(widgets::section(&theme, "Running services", services_card)),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::UiSettings;

    #[gpui::test]
    fn toggles_and_port_mode_persist(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::settings::init(UiSettings::default(), dir.path(), cx);
        });
        let (_page, cx) = cx.add_window_view(|_, _| HomeSettingsPage::default());
        cx.update(|window, cx| window.draw(cx).clear());
        let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            let bounds = cx.debug_bounds(selector).expect(selector);
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear());
        };

        click(cx, "home-toggle-git-activity");
        click(cx, "home-toggle-ci-failures");
        click(cx, "home-services-all-ports");
        cx.update(|_, cx| crate::settings::flush(cx));
        let saved = UiSettings::load(dir.path()).home_widgets;
        assert!(!saved.git_activity && !saved.ci_failures);
        assert!(saved.working_now && saved.services_all_ports);

        click(cx, "home-toggle-git-activity");
        click(cx, "home-services-by-zeron");
        cx.update(|_, cx| crate::settings::flush(cx));
        let saved = UiSettings::load(dir.path()).home_widgets;
        assert!(saved.git_activity && !saved.services_all_ports);
    }
}
