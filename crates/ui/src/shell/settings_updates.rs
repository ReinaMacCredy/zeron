use super::*;
use crate::app_update::{AppUpdate, Flow};
use crate::settings::widgets;
use zeron_proto::{HarnessUpdatePhase as Phase, HarnessUpdateStatus};

enum MacAction {
    None(&'static str),
    Check(&'static str),
    Download,
    Retry,
    RetryCheck,
    Restart,
    OpenDownloads,
}

impl Shell {
    pub(super) fn settings_header_action(
        &mut self,
        section: SettingsSection,
        theme: &Theme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        match section {
            SettingsSection::Updates => Some(self.render_update_everything_button(theme, cx)),
            SettingsSection::Playbooks => Some(self.render_new_playbook_button(theme, cx)),
            SettingsSection::Harnesses => self
                .harnesses_page
                .clone()
                .map(|page| page.update(cx, |page, cx| page.render_header_actions(theme, cx))),
            SettingsSection::AgentProfiles => self
                .agent_profiles_page
                .clone()
                .map(|page| page.update(cx, |page, cx| page.render_header_action(theme, cx))),
            SettingsSection::Shortcuts | SettingsSection::Appshots => self
                .shortcuts_page
                .clone()
                .and_then(|page| page.update(cx, |page, cx| page.render_header_action(theme, cx))),
            _ => None,
        }
    }

    fn render_update_everything_button(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let ready = self.update_all_plan(true, cx).items() > 0;
        widgets::action_button(theme, widgets::ActionTone::Solid)
            .id("update-everything")
            .debug_selector(|| "update-everything".into())
            .role(gpui::Role::Button)
            .aria_label("Update everything")
            .aria_disabled(!ready)
            .flex_none()
            .when(!ready, |el| el.opacity(0.5))
            .when(ready, |el| {
                el.tab_index(0)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.open_update_all(true, cx)))
            })
            .child(
                icon(icons::RESTART)
                    .size(px(14.0))
                    .text_color(theme.on_solid),
            )
            .child("Update everything")
            .into_any_element()
    }

    pub(super) fn render_updates_page(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).for_settings_surface();
        let this_mac = self.render_this_mac(&theme, cx);
        let agents = self.render_agent_clis(&theme, cx);
        let devices = self.render_update_devices(&theme, cx);
        let column = widgets::settings_page()
            .child(widgets::section(&theme, "This Mac", this_mac).mt_0())
            .child(agents)
            .child(widgets::section(&theme, "Devices", devices));
        div()
            .id("updates-page-host")
            .relative()
            .size_full()
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("updates-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.updates_scroll.scroll)
                        .child(column),
                )
                .fade_overflow_y(&self.updates_scroll.scroll),
            )
            .into_any_element()
    }

    fn render_this_mac(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let current = zeron_update::current_version();
        let card = widgets::section_card(theme).mt_0();
        let Some(update) = AppUpdate::global(cx) else {
            return card.child(
                widgets::card_row(theme, true)
                    .child(version_title(theme, current))
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .child("Updates are off in this build."),
                    ),
            );
        };
        let (description, tone, action, glyph) = {
            let update = update.read(cx);
            mac_state(update)
        };
        let description_color = match tone {
            Tone::Muted => theme.text_muted,
            Tone::Danger => theme.danger_muted,
        };
        let glyph = glyph.map(|phase| {
            let fill = if phase == update_glyph::GlyphPhase::Failed {
                theme.danger
            } else {
                theme.accent
            };
            div()
                .size(px(app_rail::BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .child(update_glyph::update_glyph(phase, fill, theme.on_accent))
        });
        let button = self.mac_button(action, theme, cx);
        let version_row = widgets::card_row(theme, true)
            .id("updates-version")
            .debug_selector(|| "updates-version".into())
            .child(
                div()
                    .flex_1()
                    .min_w(px(200.0))
                    .child(version_title(theme, current))
                    .child(
                        div()
                            .mt(px(Theme::TEXT_STACK_GAP))
                            .text_size(crate::typography::ui_rems(widgets::ROW_DESCRIPTION_SIZE))
                            .line_height(crate::typography::ui_rems(16.0))
                            .text_color(description_color)
                            .child(description),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .children(glyph)
                    .children(button),
            );
        let (automatic, forced) = {
            let update = update.read(cx);
            (
                update.automatic(),
                zeron_update::desktop_auto_update_override().is_some(),
            )
        };
        let auto_row = widgets::card_row(theme, false)
            .child(
                div()
                    .flex_1()
                    .min_w(px(200.0))
                    .child(widgets::row_title(theme, "Automatic updates"))
                    .child(widgets::meta_line(
                        theme,
                        vec![
                            div()
                                .child(if forced {
                                    "Set by ZERON_AUTO_UPDATE for this launch."
                                } else {
                                    "Download in the background and install when Zeron quits. Off, the download and the install each wait for a click."
                                })
                                .into_any_element(),
                        ],
                    )),
            )
            .child(
                widgets::toggle_switch(theme, automatic, "updates-automatic")
                    .id("updates-automatic")
                    .debug_selector(|| "updates-automatic".into())
                    .role(gpui::Role::Switch)
                    .aria_label("Automatic updates")
                    .aria_toggled(if automatic {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .when(forced, |el| el.opacity(0.5))
                    .when(!forced, |el| {
                        el.tab_index(0)
                            .cursor_pointer()
                            .focus_visible(|s| s.border_2().border_color(theme.accent))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                crate::settings::update(SavePolicy::Immediate, cx, |settings| {
                                    settings.auto_update = !automatic
                                });
                                if let Some(update) = AppUpdate::global(cx) {
                                    update.update(cx, |update, cx| {
                                        update.set_automatic(!automatic, cx)
                                    });
                                }
                            }))
                    }),
            );
        card.child(version_row).child(auto_row)
    }

    fn mac_button(
        &self,
        action: MacAction,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let button = |label: &'static str, tone: widgets::ActionTone| {
            widgets::text_action(theme, tone, label)
                .id("updates-mac-button")
                .debug_selector(|| "updates-mac-button".into())
                .role(gpui::Role::Button)
                .aria_label(label)
                .flex_none()
        };
        let on = |el: gpui::Stateful<gpui::Div>,
                  run: fn(&mut Shell, &mut Context<Shell>)|
         -> AnyElement {
            el.tab_index(0)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| run(this, cx)))
                .into_any_element()
        };
        Some(match action {
            MacAction::None(label) => button(label, widgets::ActionTone::Filled)
                .opacity(0.5)
                .aria_disabled(true)
                .into_any_element(),
            MacAction::Check(label) => on(button(label, widgets::ActionTone::Filled), |_, cx| {
                if let Some(update) = AppUpdate::global(cx) {
                    update.update(cx, |update, cx| update.check_in_place(cx));
                }
            }),
            MacAction::RetryCheck => on(button("Retry", widgets::ActionTone::Filled), |_, cx| {
                if let Some(update) = AppUpdate::global(cx) {
                    update.update(cx, |update, cx| update.check_in_place(cx));
                }
            }),
            MacAction::Download => on(button("Download", widgets::ActionTone::Filled), |_, cx| {
                if let Some(update) = AppUpdate::global(cx) {
                    update.update(cx, |update, cx| update.start_download(cx));
                }
            }),
            MacAction::Retry => on(button("Retry", widgets::ActionTone::Filled), |_, cx| {
                if let Some(update) = AppUpdate::global(cx) {
                    update.update(cx, |update, cx| update.start_download(cx));
                }
            }),
            MacAction::Restart => on(
                button("Restart to update", widgets::ActionTone::Solid)
                    .bg(theme.accent)
                    .text_color(theme.on_accent),
                |this, cx| {
                    let staged = AppUpdate::global(cx).and_then(|update| update.read(cx).staged());
                    if let Some(staged) = staged {
                        this.apply_staged_update(staged, cx);
                    }
                },
            ),
            MacAction::OpenDownloads => on(
                button("Open download page", widgets::ActionTone::Filled),
                |_, cx| cx.open_url(zeron_update::LATEST_RELEASE_PAGE),
            ),
        })
    }

    fn render_agent_clis(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let local = self.state.read(cx).local_device_id.clone();
        let statuses: Vec<HarnessUpdateStatus> = local
            .as_ref()
            .and_then(|id| self.harness_update_devices.get(id))
            .filter(|device| device.connected)
            .map(|device| {
                device
                    .statuses
                    .iter()
                    .filter(|status| status.installed_version.is_some())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let pending: Vec<zeron_proto::HarnessId> = statuses
            .iter()
            .filter(|status| status.phase == Phase::Available && status.can_apply)
            .map(|status| status.harness)
            .collect();
        let update_all = widgets::text_action(theme, widgets::ActionTone::Filled, "Update all")
            .id("updates-agents-update-all")
            .debug_selector(|| "updates-agents-update-all".into())
            .role(gpui::Role::Button)
            .aria_disabled(pending.is_empty())
            .when(pending.is_empty(), |el| el.opacity(0.5))
            .when(!pending.is_empty(), |el| {
                let device = local.clone();
                el.tab_index(0)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(device) = device.clone() {
                            this.update_local_agents(device, pending.clone(), cx);
                        }
                    }))
            });
        let mut card = widgets::section_card(theme).mt_0();
        if statuses.is_empty() {
            card = card.child(
                widgets::card_row(theme, true).child(
                    div()
                        .text_size(crate::typography::ui_rems(widgets::ROW_DESCRIPTION_SIZE))
                        .text_color(theme.text_muted)
                        .child("No agent CLI updates to show from this host yet."),
                ),
            );
        }
        for (ix, status) in statuses.into_iter().enumerate() {
            let row = self.render_agent_row(
                ix == 0,
                local.clone().unwrap_or_default(),
                status,
                theme,
                cx,
            );
            card = card.child(row);
        }
        div()
            .mt(px(32.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(widgets::section_title(theme, "Agent CLIs").child(update_all))
            .child(card)
    }

    fn render_agent_row(
        &mut self,
        first: bool,
        device: String,
        status: HarnessUpdateStatus,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let harness = status.harness;
        let (mark, tint) = crate::pickers::harness_brand_icon(harness);
        let (line, color) = agent_line(&status, theme);
        let selected = crate::settings::harnesses::UPDATE_POLICIES
            .iter()
            .position(|(policy, ..)| *policy == status.policy)
            .unwrap_or(0);
        let closed = widgets::SelectState::default();
        let policy_device = device.clone();
        let policy = widgets::select(
            format!("updates-policy-{harness:?}"),
            "Update policy",
            theme,
            move |shell: &mut Shell| shell.updates_policy_selects.entry(harness).or_default(),
        )
        .options(
            crate::settings::harnesses::UPDATE_POLICIES
                .iter()
                .map(|(_, label, note)| widgets::SelectOption::new(*label).detail(*note)),
            selected,
        )
        .width(140.0)
        .on_select(move |shell, selected, _, cx| {
            if let Some((policy, ..)) = crate::settings::harnesses::UPDATE_POLICIES.get(selected) {
                shell.set_agent_update_policy(policy_device.clone(), harness, *policy, cx);
            }
        })
        .render(
            self.updates_policy_selects.get(&harness).unwrap_or(&closed),
            cx,
        );
        let action = harness_updates::action(&status);
        let label = action.map_or("Update", |(label, _)| label);
        let button = widgets::text_action(theme, widgets::ActionTone::Filled, label)
            .id(SharedString::from(format!("updates-agent-{harness:?}")))
            .role(gpui::Role::Button)
            .flex_none();
        let button =
            match action.and_then(|(_, method)| method) {
                Some(method) => button.tab_index(0).cursor_pointer().on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.run_harness_update_action(device.clone(), method, harness, cx)
                    },
                )),
                None if action.is_some() => {
                    button
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_settings(SettingsSection::Harnesses, cx)
                        }))
                }
                None => button.opacity(0.5).aria_disabled(true),
            };
        widgets::card_row(theme, first)
            .child(
                div()
                    .flex_none()
                    .w(px(24.0))
                    .h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        icon(mark)
                            .size(px(15.0))
                            .text_color(tint.unwrap_or(theme.text_muted)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .child(widgets::row_title(theme, harness.display_name()))
                    .child(
                        div()
                            .mt(px(Theme::TEXT_STACK_GAP))
                            .text_size(crate::typography::ui_rems(widgets::ROW_DESCRIPTION_SIZE))
                            .line_height(crate::typography::ui_rems(16.0))
                            .text_color(color)
                            .child(SharedString::from(line)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(policy)
                    .child(button),
            )
    }

    fn set_agent_update_policy(
        &mut self,
        device: String,
        harness: zeron_proto::HarnessId,
        policy: zeron_proto::HarnessUpdatePolicy,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = serde_json::json!({
            "harness": harness,
            "policy": policy,
            "targetDeviceId": device,
        });
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::SET_HARNESS_UPDATE_POLICY, params)
                .await;
            this.update(cx, |shell, cx| {
                if let Err(error) = result {
                    shell.show_notice(
                        crate::toast::ToastKind::Error,
                        format!("Update policy: {error}"),
                        cx,
                    );
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn update_local_agents(
        &mut self,
        device: String,
        harnesses: Vec<zeron_proto::HarnessId>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = serde_json::json!({
            "harnesses": harnesses,
            "continueThreads": true,
            "targetDeviceId": device,
        });
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::UPDATE_HARNESSES, params)
                .await;
            this.update(cx, |shell, cx| {
                if let Err(error) = result {
                    shell.show_notice(
                        crate::toast::ToastKind::Error,
                        format!("Agent updates: {error}"),
                        cx,
                    );
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_update_devices(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let now = Utc::now();
        let state = self.state.read(cx);
        let local = state.local_device_id.clone();
        let newest = AppUpdate::global(cx)
            .and_then(|update| update.read(cx).available().map(str::to_owned))
            .unwrap_or_else(|| zeron_update::current_version().to_owned());
        let mut devices: Vec<&zeron_proto::Device> = state.listed_devices().collect();
        devices.sort_by_key(|device| local.as_deref() != Some(device.id.as_str()));
        let mut card = widgets::section_card(theme).mt_0();
        if devices.is_empty() {
            card = card.child(
                widgets::card_row(theme, true).child(
                    div()
                        .text_size(crate::typography::ui_rems(widgets::ROW_DESCRIPTION_SIZE))
                        .text_color(theme.text_muted)
                        .child("No paired devices."),
                ),
            );
        }
        for (ix, device) in devices.into_iter().enumerate() {
            let is_local = local.as_deref() == Some(device.id.as_str());
            let version = device.version.as_deref().filter(|v| !v.is_empty());
            let mut meta =
                vec![crate::settings::devices::platform_label(&device.platform).to_owned()];
            if let Some(version) = version {
                meta.push(format!("v{version}"));
            }
            meta.push(if is_local {
                "This device".to_owned()
            } else if state.device_online(&device.id, now) {
                "Online".to_owned()
            } else {
                "Offline".to_owned()
            });
            let (status, color) = if is_local {
                ("Updated above", theme.text_muted)
            } else if version.is_some_and(|v| zeron_update::version_newer(&newest, v)) {
                ("Update on that device", theme.text_muted)
            } else if version.is_some() {
                ("Up to date", theme.success_muted)
            } else {
                ("Version unknown", theme.text_muted)
            };
            card = card.child(
                widgets::card_row(theme, ix == 0)
                    .child(widgets::row_tile(
                        theme,
                        spaces::device_glyph(&device.platform),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(160.0))
                            .child(widgets::row_title(theme, device.name.clone()))
                            .child(widgets::meta_line(
                                theme,
                                vec![
                                    div()
                                        .child(SharedString::from(meta.join(" · ")))
                                        .into_any_element(),
                                ],
                            )),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(color)
                            .child(status),
                    ),
            );
        }
        card
    }
}

#[derive(Clone, Copy)]
enum Tone {
    Muted,
    Danger,
}

fn version_title(theme: &Theme, version: &str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(widgets::row_title(theme, "Version"))
        .child(
            div()
                .px(px(5.0))
                .py(px(1.0))
                .rounded(px(4.0))
                .bg(theme.wash(0.07))
                .font_family(theme.font_mono.clone())
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(version.to_owned())),
        )
}

fn mac_state(
    update: &AppUpdate,
) -> (
    SharedString,
    Tone,
    MacAction,
    Option<update_glyph::GlyphPhase>,
) {
    let glyph = Some(update_glyph::GlyphPhase::of(update.flow()));
    if update.checking() {
        return (
            "Checking for updates…".into(),
            Tone::Muted,
            MacAction::None("Checking…"),
            None,
        );
    }
    match update.flow() {
        Flow::Downloading { version } => (
            format!("Downloading update {version}…").into(),
            Tone::Muted,
            MacAction::None("Downloading…"),
            glyph,
        ),
        Flow::Ready { version, .. } => (
            format!("Update {version} downloaded. Restart Zeron to install it.").into(),
            Tone::Muted,
            MacAction::Restart,
            glyph,
        ),
        Flow::Installed => (
            "Restarting…".into(),
            Tone::Muted,
            MacAction::None("Restarting…"),
            None,
        ),
        Flow::Failed { version, message } => (
            format!("Download failed for {version}: {message}").into(),
            Tone::Danger,
            MacAction::Retry,
            glyph,
        ),
        Flow::Idle => {
            if let Some(latest) = update.available() {
                if let Some(blocker) = update.blocker() {
                    return (
                        blocker.to_string().into(),
                        Tone::Muted,
                        MacAction::OpenDownloads,
                        None,
                    );
                }
                if update.self_updating() {
                    return (
                        format!("Update {latest} available.").into(),
                        Tone::Muted,
                        MacAction::Download,
                        glyph,
                    );
                }
                if matches!(update.install(), zeron_update::InstallKind::Managed { .. }) {
                    return (
                        format!("Update {latest} available. Run `zeron update` in a terminal to install it.")
                            .into(),
                        Tone::Muted,
                        MacAction::Check("Check for Updates"),
                        None,
                    );
                }
                return (
                    format!("Update {latest} available. This copy of Zeron can't update itself.")
                        .into(),
                    Tone::Muted,
                    MacAction::OpenDownloads,
                    None,
                );
            }
            match update.status() {
                Some(status) if status.error.is_some() => (
                    format!(
                        "Couldn't check for updates: {}",
                        status
                            .error
                            .as_deref()
                            .and_then(|error| error.rsplit(": ").next())
                            .unwrap_or_default()
                    )
                    .into(),
                    Tone::Danger,
                    MacAction::RetryCheck,
                    None,
                ),
                Some(status) if status.checked_at.is_some() => (
                    "Up to date.".into(),
                    Tone::Muted,
                    MacAction::Check("Check for Updates"),
                    None,
                ),
                _ => (
                    "Current version of the application.".into(),
                    Tone::Muted,
                    MacAction::Check("Check for Updates"),
                    None,
                ),
            }
        }
    }
}

fn agent_line(status: &HarnessUpdateStatus, theme: &Theme) -> (String, gpui::Hsla) {
    let installed = status.installed_version.clone().unwrap_or_default();
    let latest = status.latest_version.clone();
    let arrow = |tail: &str| match &latest {
        Some(latest) if installed.is_empty() => format!("{latest}{tail}"),
        Some(latest) => format!("{installed} → {latest}{tail}"),
        None => format!("{installed}{tail}"),
    };
    if status.policy == zeron_proto::HarnessUpdatePolicy::Off {
        return (format!("{installed} · not checked"), theme.text_muted);
    }
    match status.phase {
        Phase::Available => (arrow(" available"), theme.text_muted),
        Phase::WaitingForIdle => (arrow(" · waiting for idle"), theme.warning),
        Phase::Preparing | Phase::Downloading | Phase::Installing | Phase::Verifying => {
            (harness_updates::detail(status), theme.accent)
        }
        Phase::Updated => (harness_updates::detail(status), theme.success_muted),
        Phase::Failed => (harness_updates::detail(status), theme.danger_muted),
        Phase::ManualActionRequired => (
            match &status.manual_command {
                Some(command) => arrow(&format!(" · run `{command}`")),
                None => arrow(" · needs a manual update"),
            },
            theme.text_muted,
        ),
        Phase::Current => (format!("{installed} · up to date"), theme.text_muted),
        Phase::Dormant | Phase::Checking => (format!("{installed} · checking…"), theme.text_muted),
    }
}
