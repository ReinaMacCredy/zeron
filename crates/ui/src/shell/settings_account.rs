use super::*;
use crate::settings::widgets;

impl Shell {
    pub(super) fn render_account_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).for_settings_surface();
        let (name, identity, action) = {
            let state = self.state.read(cx);
            let (name, identity) =
                sidebar_account_identity(state.workspace_scope, self.sync_flow, state.auth_user());
            (
                name,
                identity,
                account_menu_action(state.workspace_scope, self.sync_flow),
            )
        };
        let initial: SharedString = name
            .trim()
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".into())
            .into();
        let avatar = div()
            .flex_none()
            .size(px(36.0))
            .rounded_full()
            .bg(theme.glass_hover().opacity(1.6))
            .flex()
            .items_center()
            .justify_center()
            .text_size(crate::typography::ui_rems(14.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.text)
            .child(initial);
        let identity_row = widgets::card_row(&theme, true)
            .id("account-identity")
            .debug_selector(|| "account-identity".into())
            .flex_nowrap()
            .gap(px(12.0))
            .child(avatar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(widgets::row_title(&theme, name))
                    .child(widgets::meta_line(
                        &theme,
                        vec![div().child(identity).into_any_element()],
                    )),
            );
        let action_row = action.map(|action| {
            let (title, detail, button): (&str, &str, AnyElement) = match action {
                AccountMenuAction::EnableSync => (
                    "Sync",
                    "Sign in to keep chats, projects and settings in step on your other devices.",
                    widgets::text_action(&theme, widgets::ActionTone::Solid, "Enable sync")
                        .id("account-enable-sync")
                        .debug_selector(|| "account-enable-sync".into())
                        .role(gpui::Role::Button)
                        .tab_index(0)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.start_sign_in(cx)))
                        .into_any_element(),
                ),
                AccountMenuAction::SyncInProgress => (
                    "Sync",
                    "Sync setup is in progress.",
                    widgets::text_action(&theme, widgets::ActionTone::Filled, "Setting up…")
                        .id("account-sync-progress")
                        .opacity(0.6)
                        .into_any_element(),
                ),
                AccountMenuAction::RestartPending => (
                    "Sync",
                    "Sync is ready to finish.",
                    widgets::text_action(&theme, widgets::ActionTone::Solid, "Finish sync setup")
                        .id("account-sync-restart")
                        .role(gpui::Role::Button)
                        .tab_index(0)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.reopen_sync_notice(cx)))
                        .into_any_element(),
                ),
                AccountMenuAction::SignOut => (
                    "Sign out",
                    "Removes your credentials, closes the synced workspace and continues on this device.",
                    widgets::text_action(&theme, widgets::ActionTone::Danger, "Sign out")
                        .id("account-sign-out")
                        .debug_selector(|| "account-sign-out".into())
                        .role(gpui::Role::Button)
                        .tab_index(0)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.request_sign_out(cx)))
                        .into_any_element(),
                ),
            };
            widgets::card_row(&theme, false)
                .flex_nowrap()
                .gap(px(12.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(widgets::row_title(&theme, title))
                        .child(widgets::meta_line(
                            &theme,
                            vec![div().child(detail).into_any_element()],
                        )),
                )
                .child(button)
        });
        let column = widgets::settings_page().child(
            widgets::section(
                &theme,
                "Account",
                widgets::section_card(&theme)
                    .mt_0()
                    .child(identity_row)
                    .children(action_row),
            )
            .mt_0(),
        );
        div()
            .id("account-page")
            .size_full()
            .overflow_y_scroll()
            .child(column)
            .into_any_element()
    }
}
