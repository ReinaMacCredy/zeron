//! Agent CLI updates: the per-device update watch, Update all and its
//! dialog.

use super::*;
use zeron_proto::{HarnessId, HarnessUpdatePhase as Phase, HarnessUpdateStatus};

pub(super) fn versionless_notification_key(device: &str, harness: HarnessId) -> String {
    format!("{device}:{harness:?}:versionless")
}

pub(super) fn notification_key(device: &str, status: &HarnessUpdateStatus) -> Option<String> {
    if status.phase != Phase::Available {
        return None;
    }
    Some(match status.latest_version.as_deref() {
        Some(version) => format!("{device}:{:?}:{version}", status.harness),
        None => versionless_notification_key(device, status.harness),
    })
}

#[derive(Default)]
pub(super) struct DeviceUpdates {
    online: bool,
    pub(super) connected: bool,
    pub(super) statuses: Vec<HarnessUpdateStatus>,
    watch: Option<Task<()>>,
    limit_watch: Option<Task<()>>,
}

// Reconcile presence without throwing away a disconnected device's last known
// updates. Removed/unsupported devices leave the inventory entirely.
fn reconcile_devices(
    devices: &mut std::collections::BTreeMap<String, DeviceUpdates>,
    desired: &std::collections::BTreeMap<String, bool>,
) -> Vec<String> {
    devices.retain(|id, _| desired.contains_key(id));
    let mut start = Vec::new();
    for (id, online) in desired {
        let updates = devices.entry(id.clone()).or_default();
        updates.online = *online;
        if !online {
            updates.watch = None;
            updates.limit_watch = None;
            updates.connected = false;
        } else if updates.watch.is_none() {
            start.push(id.clone());
        }
    }
    start
}

pub(super) fn action(status: &HarnessUpdateStatus) -> Option<(&'static str, Option<&'static str>)> {
    match status.phase {
        Phase::Available if status.can_apply => {
            Some(("Update", Some(methods::APPLY_HARNESS_UPDATE)))
        }
        Phase::Available => Some(("View steps", None)),
        Phase::WaitingForIdle | Phase::Preparing | Phase::Downloading => {
            Some(("Cancel", Some(methods::CANCEL_HARNESS_UPDATE)))
        }
        Phase::Failed if status.can_apply && status.install_failed() => {
            Some(("Retry update", Some(methods::APPLY_HARNESS_UPDATE)))
        }
        Phase::Failed => Some(("Check again", Some(methods::CHECK_HARNESS_UPDATES))),
        _ => None,
    }
}

pub(super) fn detail(status: &HarnessUpdateStatus) -> String {
    match status.phase {
        Phase::Available => match (&status.installed_version, &status.latest_version) {
            (Some(installed), Some(latest)) => format!("{installed} → {latest}"),
            (_, Some(latest)) => format!("Version {latest} available"),
            _ => "Update available".into(),
        },
        Phase::WaitingForIdle => "Waiting for the current run".into(),
        Phase::Preparing => "Preparing update…".into(),
        Phase::Downloading => status
            .progress
            .as_ref()
            .and_then(|p| p.message.clone())
            .unwrap_or_else(|| "Downloading…".into()),
        Phase::Installing => "Installing…".into(),
        Phase::Verifying => "Verifying installation…".into(),
        Phase::Updated => status
            .installed_version
            .as_ref()
            .map(|v| format!("Updated to {v}"))
            .unwrap_or_else(|| "Updated".into()),
        Phase::Failed => status
            .error
            .as_ref()
            .map(|e| e.message.clone())
            .unwrap_or_else(|| {
                if status.install_failed() {
                    "Update failed".into()
                } else {
                    "Couldn’t check for updates".into()
                }
            }),
        _ => "Checking for updates…".into(),
    }
}

impl Shell {
    pub(super) fn refresh_harness_update_watch(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if !matches!(state.connection, ConnectionStatus::Ready) {
            self.harness_update_devices.clear();
            return;
        }
        let Some(engine) = state.engine().cloned() else {
            return;
        };
        // Include the connected engine before its registry row arrives. Selection
        // of a chat, project, or composer host must not affect this inventory.
        let devices: std::collections::BTreeMap<_, _> = state
            .devices
            .iter()
            .map(|device| device.id.clone())
            .chain(std::iter::once(engine.engine_info().device_id.clone()))
            .filter(|id| state.device_supports(id, zeron_proto::capabilities::HARNESS_UPDATES_V1))
            .map(|id| {
                let online = state.device_online(&id, Utc::now());
                (id, online)
            })
            .collect();
        let limit_devices: Vec<String> = devices
            .keys()
            .filter(|id| state.device_supports(id, zeron_proto::capabilities::LIMIT_RESUME_V1))
            .cloned()
            .collect();
        let app_state = self.state.clone();
        for device in reconcile_devices(&mut self.harness_update_devices, &devices) {
            let updates = self.harness_update_devices.get_mut(&device).unwrap();
            if limit_devices.contains(&device) {
                updates.limit_watch = Some(spawn_limit_watch(
                    engine.clone(),
                    app_state.clone(),
                    device.clone(),
                    cx,
                ));
            }
            let engine = engine.clone();
            updates.watch = Some(cx.spawn(async move |this, cx| {
                let mut retry = 1;
                loop {
                    let result = engine
                        .client()
                        .subscribe_checked(
                            methods::WATCH_HARNESS_UPDATES,
                            serde_json::json!({ "targetDeviceId": device }),
                        )
                        .await;
                    if let Ok(mut stream) = result {
                        while let Some(value) = stream.recv().await {
                            let Ok(rows) = serde_json::from_value(value) else {
                                break;
                            };
                            if this
                                .update(cx, |shell, cx| {
                                    let Some(updates) =
                                        shell.harness_update_devices.get_mut(&device)
                                    else {
                                        return;
                                    };
                                    updates.statuses = rows;
                                    updates.connected = true;
                                    cx.notify();
                                })
                                .is_err()
                            {
                                return;
                            }
                            retry = 1;
                        }
                    }
                    if this
                        .update(cx, |shell, cx| {
                            if let Some(updates) = shell.harness_update_devices.get_mut(&device) {
                                updates.connected = false;
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        return;
                    }
                    cx.background_executor()
                        .timer(Duration::from_secs(retry))
                        .await;
                    retry = (retry * 2).min(15);
                }
            }));
        }
    }

    pub(super) fn run_harness_update_action(
        &mut self,
        device: String,
        method: &'static str,
        harness: HarnessId,
        cx: &mut Context<Self>,
    ) {
        let state = self.state.read(cx);
        let Some(engine) = state.engine().cloned() else {
            return;
        };
        if !self
            .harness_update_devices
            .get(&device)
            .is_some_and(|updates| updates.online && updates.connected)
            || !state.device_online(&device, Utc::now())
            || !state.device_supports(&device, zeron_proto::capabilities::HARNESS_UPDATES_V1)
        {
            return;
        }
        let params = serde_json::json!({ "harness": harness, "targetDeviceId": device });
        // Each request owns its lifetime: acting on a second device must not
        // cancel the first device's RPC. Progress comes from independent watches.
        cx.spawn(async move |this, cx| {
            let result = engine.client().call(method, params).await;
            this.update(cx, |shell, cx| {
                if let Err(error) = result
                    && !matches!(&error, zeron_rpc::RpcError::Failed(message)
                        if method == methods::APPLY_HARNESS_UPDATE && message == "update cancelled")
                {
                    shell.show_notice(
                        crate::toast::ToastKind::Error,
                        format!("Agent update ({device}): {error}"),
                        cx,
                    );
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn status(phase: Phase) -> HarnessUpdateStatus {
        HarnessUpdateStatus {
            harness: HarnessId::Codex,
            installed_version: Some("1.0".into()),
            latest_version: Some("2.0".into()),
            channel: None,
            source: Default::default(),
            policy: Default::default(),
            phase,
            progress: None,
            checked_at: None,
            error: None,
            can_apply: true,
            manual_command: None,
            compatibility: None,
            held_back: None,
        }
    }

    fn device(phase: Phase) -> DeviceUpdates {
        DeviceUpdates {
            online: true,
            connected: true,
            statuses: vec![status(phase)],
            watch: None,
            limit_watch: None,
        }
    }

    #[test]
    fn update_actions_follow_the_phase() {
        assert_eq!(action(&status(Phase::Available)).unwrap().0, "Update");
        assert!(action(&status(Phase::Installing)).is_none());
        assert_eq!(action(&status(Phase::Downloading)).unwrap().0, "Cancel");
    }

    #[test]
    fn disconnect_retains_statuses_but_reconnect_waits_for_fresh_status() {
        let mut devices = BTreeMap::from([("laptop".into(), device(Phase::Available))]);
        assert!(
            reconcile_devices(&mut devices, &BTreeMap::from([("laptop".into(), false)])).is_empty()
        );
        assert!(!devices["laptop"].connected);
        assert_eq!(devices["laptop"].statuses[0].phase, Phase::Available);
        assert_eq!(
            reconcile_devices(&mut devices, &BTreeMap::from([("laptop".into(), true)])),
            ["laptop"]
        );
        assert!(!devices["laptop"].connected);
    }

    #[test]
    fn presence_changes_only_restart_the_affected_host() {
        let mut devices = BTreeMap::from([
            ("desktop".into(), device(Phase::Available)),
            ("laptop".into(), device(Phase::Available)),
        ]);
        for updates in devices.values_mut() {
            updates.watch = Some(Task::ready(()));
        }
        let mut desired = BTreeMap::from([("desktop".into(), true), ("laptop".into(), true)]);
        assert!(reconcile_devices(&mut devices, &desired).is_empty());
        desired.insert("laptop".into(), false);
        assert!(reconcile_devices(&mut devices, &desired).is_empty());
        assert!(devices["desktop"].watch.is_some());
        assert!(devices["desktop"].connected);
        assert!(devices["laptop"].watch.is_none());
        desired.insert("laptop".into(), true);
        assert_eq!(reconcile_devices(&mut devices, &desired), ["laptop"]);
    }

    #[test]
    fn inventory_removes_departed_hosts() {
        let mut devices = BTreeMap::from([
            ("removed".into(), device(Phase::Available)),
            ("current".into(), device(Phase::Current)),
        ]);
        let start = reconcile_devices(
            &mut devices,
            &BTreeMap::from([
                ("current".into(), true),
                ("new".into(), true),
                ("offline".into(), false),
            ]),
        );
        assert_eq!(start, ["current", "new"]);
        assert!(!devices.contains_key("removed"));
        assert!(devices.contains_key("offline"));
    }
}

fn spawn_limit_watch(
    engine: crate::state::EngineHandle,
    state: Entity<AppState>,
    device: String,
    cx: &mut Context<Shell>,
) -> Task<()> {
    cx.spawn(async move |_, cx| {
        let mut retry = 1;
        loop {
            let result = engine
                .client()
                .subscribe_checked(
                    methods::WATCH_LIMIT_RESUMES,
                    serde_json::json!({ "targetDeviceId": device }),
                )
                .await;
            if let Ok(mut stream) = result {
                while let Some(value) = stream.recv().await {
                    let Ok(resumes) = serde_json::from_value::<zeron_proto::LimitResumes>(value)
                    else {
                        break;
                    };
                    state.update(cx, |state, cx| {
                        if state.limit_resumes.get(&device) != Some(&resumes) {
                            state.limit_resumes.insert(device.clone(), resumes);
                            cx.notify();
                        }
                    });
                    retry = 1;
                }
            }
            cx.background_executor()
                .timer(Duration::from_secs(retry))
                .await;
            retry = (retry * 2).min(15);
        }
    })
}

pub(super) struct UpdateAllPlan {
    app: Option<String>,
    devices: Vec<PlanDevice>,
    skipped: Vec<(String, &'static str)>,
    running: usize,
}

struct PlanDevice {
    id: String,
    name: String,
    harnesses: Vec<HarnessId>,
    local: bool,
    running: usize,
}

pub(super) const THIS_MACHINE: &str = if cfg!(target_os = "macos") {
    "This Mac"
} else {
    "This device"
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UpdateAllDialog {
    pub(super) this_mac: bool,
    pub(super) continue_threads: bool,
    pub(super) skipped_open: bool,
}

impl UpdateAllPlan {
    pub(super) fn items(&self) -> usize {
        usize::from(self.app.is_some())
            + self
                .devices
                .iter()
                .map(|d| d.harnesses.len())
                .sum::<usize>()
    }

    pub(super) fn agent_updates(&self) -> usize {
        self.devices.iter().map(|d| d.harnesses.len()).sum()
    }

    fn machines(&self) -> usize {
        let local_listed = self.devices.iter().any(|device| device.local);
        self.devices.len() + usize::from(self.app.is_some() && !local_listed)
    }

    fn title(&self) -> String {
        let machines = self.machines();
        format!(
            "Update {machines} {}?",
            if machines == 1 { "machine" } else { "machines" }
        )
    }

    fn threads_warning(&self, continue_threads: bool) -> Option<String> {
        if self.running == 0 {
            return None;
        }
        let threads = if self.running == 1 {
            "thread"
        } else {
            "threads"
        };
        let after = if self.app.is_some() {
            "restart"
        } else {
            "update"
        };
        Some(format!(
            "{} running {threads} will be interrupted{}",
            self.running,
            if continue_threads {
                format!(" and can resume after the {after}.")
            } else {
                ".".to_owned()
            }
        ))
    }
}
fn settled(status: &HarnessUpdateStatus) -> bool {
    matches!(
        status.phase,
        Phase::Current | Phase::Updated | Phase::Failed | Phase::ManualActionRequired
    )
}

impl Shell {
    pub(super) fn update_all_plan(&self, this_mac: bool, cx: &App) -> UpdateAllPlan {
        let state = self.state.read(cx);
        let local = state.local_device_id.as_deref();
        let app = crate::app_update::AppUpdate::global(cx).and_then(|update| {
            let update = update.read(cx);
            match update.flow() {
                crate::app_update::Flow::Ready { version, .. } => Some(version.clone()),
                crate::app_update::Flow::Idle
                    if update.blocker().is_none() && update.install().supports_desktop_update() =>
                {
                    update.available().map(str::to_owned)
                }
                _ => None,
            }
        });
        let now = Utc::now();
        let working: Vec<&zeron_proto::Chat> = state
            .chats
            .iter()
            .filter(|chat| matches!(state.indicator_for(&chat.id, now), Indicator::Working))
            .collect();
        let mut skipped = Vec::new();
        let mut devices: Vec<PlanDevice> = Vec::new();
        for (id, device) in &self.harness_update_devices {
            let is_local = local == Some(id.as_str());
            if this_mac && !is_local {
                continue;
            }
            let harnesses: Vec<HarnessId> = device
                .statuses
                .iter()
                .filter(|status| status.phase == Phase::Available && status.can_apply)
                .map(|status| status.harness)
                .collect();
            if harnesses.is_empty() {
                continue;
            }
            if !(device.online && device.connected) {
                skipped.push((
                    state.device_name(id).unwrap_or(id).to_owned(),
                    "not connected",
                ));
                continue;
            }
            devices.push(PlanDevice {
                id: id.clone(),
                name: if is_local {
                    THIS_MACHINE.to_owned()
                } else {
                    state.device_name(id).unwrap_or(id).to_owned()
                },
                harnesses,
                local: is_local,
                running: 0,
            });
        }
        devices.sort_by_key(|device| !device.local);
        if this_mac {
            for device in state.listed_devices() {
                if local != Some(device.id.as_str()) {
                    skipped.push((device.name.clone(), "update on that device"));
                }
            }
            // A signed-out capture bundle has no fleet; the update preview stands in.
            if crate::app_update::AppUpdate::global(cx)
                .is_some_and(|update| update.read(cx).preview())
            {
                skipped.push(("Mac mini".to_owned(), "update on that device"));
                skipped.push(("Studio box".to_owned(), "update on that device"));
            }
        }
        for device in &mut devices {
            device.running = working
                .iter()
                .filter(|chat| {
                    (app.is_some() && device.local && local == Some(chat.device_id.as_str()))
                        || (chat.device_id == device.id
                            && chat
                                .config
                                .as_ref()
                                .is_some_and(|config| device.harnesses.contains(&config.harness)))
                })
                .count();
        }
        let app_running = if app.is_some() && !devices.iter().any(|device| device.local) {
            working
                .iter()
                .filter(|chat| local == Some(chat.device_id.as_str()))
                .count()
        } else {
            0
        };
        let running = app_running + devices.iter().map(|device| device.running).sum::<usize>();
        UpdateAllPlan {
            app,
            devices,
            skipped,
            running,
        }
    }

    pub(super) fn open_update_all(&mut self, this_mac: bool, cx: &mut Context<Self>) {
        self.update_all_dialog = Some(UpdateAllDialog {
            this_mac,
            skipped_open: false,
            continue_threads: true,
        });
        cx.notify();
    }

    fn start_update_all(&mut self, this_mac: bool, continue_threads: bool, cx: &mut Context<Self>) {
        self.update_all_dialog = None;
        let plan = self.update_all_plan(this_mac, cx);
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let restarts = plan.app.is_some();
        if restarts {
            let local_harnesses = plan
                .devices
                .iter()
                .find(|device| device.local)
                .map(|device| device.harnesses.clone())
                .unwrap_or_default();
            self.update_all_restart = Some((local_harnesses, false));
            if let Some(update) = crate::app_update::AppUpdate::global(cx)
                && matches!(update.read(cx).flow(), crate::app_update::Flow::Idle)
            {
                update.update(cx, |update, cx| update.start_download(cx));
            }
        }
        cx.spawn(async move |this, cx| {
            let client = engine.client();
            let mut failures = Vec::new();
            if restarts
                && let Err(error) = client
                    .call(
                        methods::PREPARE_RESTART,
                        serde_json::json!({ "continueThreads": continue_threads }),
                    )
                    .await
            {
                failures.push(error.to_string());
            }
            for device in plan.devices {
                let continues = continue_threads && !(restarts && device.local);
                let params = serde_json::json!({
                    "harnesses": device.harnesses,
                    "continueThreads": continues,
                    "targetDeviceId": device.id,
                });
                if let Err(error) = client.call(methods::UPDATE_HARNESSES, params).await {
                    failures.push(format!("{}: {error}", device.name));
                }
            }
            this.update(cx, |shell, cx| {
                if let Some(pending) = shell.update_all_restart.as_mut() {
                    pending.1 = true;
                }
                if !failures.is_empty() {
                    shell.show_notice(
                        crate::toast::ToastKind::Error,
                        format!("Could not update everything: {}", failures.join("; ")),
                        cx,
                    );
                }
                shell.advance_update_all(cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The app restarts last: once the local agents have settled and the
    /// download is staged.
    pub(super) fn advance_update_all(&mut self, cx: &mut Context<Self>) {
        let Some((harnesses, requested)) = self.update_all_restart.as_ref() else {
            return;
        };
        if !requested {
            return;
        }
        let local = self.state.read(cx).local_device_id.clone();
        let agents_done = local
            .and_then(|id| self.harness_update_devices.get(&id))
            .is_none_or(|device| {
                harnesses.iter().all(|harness| {
                    device
                        .statuses
                        .iter()
                        .find(|status| status.harness == *harness)
                        .is_none_or(settled)
                })
            });
        let Some(update) = crate::app_update::AppUpdate::global(cx) else {
            self.update_all_restart = None;
            return;
        };
        match update.read(cx).flow().clone() {
            crate::app_update::Flow::Ready { staged, .. } if agents_done => {
                self.update_all_restart = None;
                self.apply_staged_update(staged, cx);
            }
            crate::app_update::Flow::Failed { message, .. } => {
                self.update_all_restart = None;
                self.show_notice(
                    crate::toast::ToastKind::Error,
                    format!("The app update failed: {message}"),
                    cx,
                );
            }
            _ => {}
        }
    }

    pub(super) fn render_update_all_dialog(
        &mut self,
        viewport: gpui::Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let dialog = self.update_all_dialog?;
        let plan = self.update_all_plan(dialog.this_mac, cx);
        if plan.items() == 0 {
            self.update_all_dialog = None;
            return None;
        }
        let theme = Theme::of(cx).for_popup();
        let muted = theme.text_muted;
        let reduced = motion::reduced_motion(cx);
        let mut description: Vec<AnyElement> = Vec::new();
        if let Some(version) = &plan.app {
            description.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child("Zeron")
                    .child(
                        div()
                            .font_family(theme.font_mono.clone())
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text)
                            .child(SharedString::from(version.clone())),
                    )
                    .into_any_element(),
            );
        }
        let agents = plan.agent_updates();
        if agents > 0 {
            if !description.is_empty() {
                description.push(div().child("·").into_any_element());
            }
            description.push(
                div()
                    .child(SharedString::from(format!(
                        "{agents} agent {}",
                        if agents == 1 { "update" } else { "updates" }
                    )))
                    .into_any_element(),
            );
        }
        let cell = |content: AnyElement, last: bool| {
            div()
                .py(px(7.0))
                .when(!last, |el| {
                    el.border_b_1().border_color(theme.border.opacity(0.65))
                })
                .child(content)
        };
        let mut rows: Vec<(
            String,
            Option<(&'static str, Option<&'static str>)>,
            String,
            usize,
        )> = Vec::new();
        if plan.app.is_some() && !plan.devices.iter().any(|device| device.local) {
            let others: usize = plan.devices.iter().map(|device| device.running).sum();
            rows.push((
                THIS_MACHINE.into(),
                Some(("This app", Some("restarts last"))),
                String::new(),
                plan.running - others,
            ));
        }
        for device in &plan.devices {
            let zeron = if device.local && plan.app.is_some() {
                Some(("This app", Some("restarts last")))
            } else {
                None
            };
            let agents: Vec<&str> = device.harnesses.iter().map(|h| h.display_name()).collect();
            rows.push((
                device.name.clone(),
                zeron,
                agents.join(", "),
                device.running,
            ));
        }
        let count = rows.len();
        let header = |label: &'static str| {
            div()
                .pb(px(6.0))
                .border_b_1()
                .border_color(theme.border)
                .text_size(crate::typography::ui_rems(11.5))
                .text_color(muted)
                .child(label)
        };
        let column = |cells: Vec<gpui::Div>, grow: bool| {
            div()
                .flex()
                .flex_col()
                .when(grow, |el| el.flex_1().min_w_0())
                .when(!grow, |el| el.flex_none())
                .children(cells)
        };
        let mut machine = vec![header("Machine")];
        let mut zeron = vec![header("Zeron")];
        let mut agents_col = vec![header("Agents")];
        let mut running = vec![header("")];
        for (ix, (name, app, agents, busy)) in rows.into_iter().enumerate() {
            let last = ix + 1 == count;
            let tall = app.and_then(|(_, sub)| sub).is_some();
            let line = |el: gpui::Div| if tall { el.min_h(px(36.0)) } else { el };
            machine.push(line(cell(
                div().child(SharedString::from(name)).into_any_element(),
                last,
            )));
            zeron.push(line(cell(
                match app {
                    Some((label, sub)) => div()
                        .child(label)
                        .children(sub.map(|sub| {
                            div()
                                .text_size(crate::typography::ui_rems(11.5))
                                .text_color(muted)
                                .child(sub)
                        }))
                        .into_any_element(),
                    None => div().text_color(muted).child("—").into_any_element(),
                },
                last,
            )));
            agents_col.push(line(cell(
                div()
                    .child(SharedString::from(if agents.is_empty() {
                        "—".to_owned()
                    } else {
                        agents
                    }))
                    .into_any_element(),
                last,
            )));
            running.push(line(cell(
                div()
                    .flex()
                    .justify_end()
                    .text_color(theme.warning)
                    .when(busy > 0, |el| {
                        el.child(SharedString::from(format!("{busy} running")))
                    })
                    .into_any_element(),
                last,
            )));
        }
        let table = div()
            .flex()
            .flex_row()
            .gap(px(16.0))
            .text_size(crate::typography::ui_rems(13.0))
            .child(column(machine, false))
            .child(column(zeron, false))
            .child(column(agents_col, true))
            .child(column(running, false));
        let skipped = (!plan.skipped.is_empty()).then(|| {
            let open = dialog.skipped_open;
            let t = motion::state_t("update-all-skipped", open, motion::COLLAPSE, reduced);
            let toggle = div()
                .id("update-all-skipped")
                .debug_selector(|| "update-all-skipped".into())
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(format!(
                    "{} skipped",
                    plan.skipped.len()
                )))
                .aria_expanded(open)
                .flex()
                .items_center()
                .gap(px(4.0))
                .py(px(2.0))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(muted)
                .cursor_pointer()
                .hover(|s| s.text_color(theme.text))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(dialog) = this.update_all_dialog.as_mut() {
                        dialog.skipped_open = !dialog.skipped_open;
                    }
                    cx.notify();
                }))
                .child(
                    icon(icons::ALT_ARROW_RIGHT)
                        .size(px(12.0))
                        .text_color(muted)
                        .with_transformation(gpui::Transformation::rotate(gpui::radians(
                            std::f32::consts::FRAC_PI_2 * t,
                        ))),
                )
                .child(SharedString::from(format!(
                    "{} skipped",
                    plan.skipped.len()
                )));
            let list_height = 19.0 * plan.skipped.len() as f32 + 6.0;
            div().flex().flex_col().child(toggle).child(
                div()
                    .h(px(list_height * t))
                    .overflow_hidden()
                    .opacity(t)
                    .child(
                        div()
                            .pt(px(6.0))
                            .pl(px(18.0))
                            .flex()
                            .flex_col()
                            .text_size(crate::typography::ui_rems(12.0))
                            .line_height(px(19.0))
                            .text_color(muted)
                            .children(plan.skipped.iter().map(|(name, reason)| {
                                div()
                                    .flex()
                                    .gap(px(4.0))
                                    .child(
                                        div()
                                            .text_color(theme.text)
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .child(SharedString::from(name.clone())),
                                    )
                                    .child(SharedString::from(format!("— {reason}")))
                            })),
                    ),
            )
        });
        let this_mac = dialog.this_mac;
        let continue_threads = dialog.continue_threads;
        let continue_box = (plan.running > 0).then(|| {
            div()
                .id("update-all-continue")
                .debug_selector(|| "update-all-continue".into())
                .mt(px(6.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .cursor_pointer()
                .role(gpui::Role::CheckBox)
                .aria_label("Continue threads after restarts")
                .aria_toggled(if continue_threads {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(dialog) = this.update_all_dialog.as_mut() {
                        dialog.continue_threads = !dialog.continue_threads;
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .size(px(14.0))
                        .flex_none()
                        .rounded(px(4.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(continue_threads, |el| el.bg(theme.accent))
                        .when(!continue_threads, |el| {
                            el.border_1().border_color(theme.border_strong)
                        })
                        .when(continue_threads, |el| {
                            el.child(icon(icons::CHECK).size(px(10.0)).text_color(theme.on_solid))
                        }),
                )
                .child("Continue threads after restarts")
        });
        let footer = div()
            .px(px(24.0))
            .pt(px(14.0))
            .pb(px(18.0))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .mr_auto()
                    .max_w(px(320.0))
                    .flex()
                    .flex_col()
                    .text_size(crate::typography::ui_rems(12.0))
                    .line_height(px(16.0))
                    .text_color(muted)
                    .children(
                        plan.threads_warning(continue_threads)
                            .map(SharedString::from),
                    )
                    .children(continue_box),
            )
            .child(
                popover::btn_ghost(&theme, "Cancel", "update-all-cancel")
                    .id("update-all-cancel")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.update_all_dialog = None;
                        cx.notify();
                    })),
            )
            .child(
                popover::btn_primary(&theme, "Update")
                    .id("update-all-confirm")
                    .debug_selector(|| "update-all-confirm".into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.start_update_all(this_mac, continue_threads, cx)
                    })),
            );
        let card = popover::dialog_card(&theme)
            .w(px(576.0))
            .max_w(viewport.width - px(48.0))
            .p_0()
            .id("update-all-dialog-card")
            .role(gpui::Role::Dialog)
            .aria_label(SharedString::from(plan.title()))
            .child(
                div()
                    .px(px(24.0))
                    .pt(px(20.0))
                    .pb(px(12.0))
                    .child(popover::dialog_title(&theme, &plan.title()))
                    .child(
                        div()
                            .mt(px(4.0))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .text_size(crate::typography::ui_rems(13.0))
                            .text_color(muted)
                            .children(description),
                    ),
            )
            .child(
                div()
                    .px(px(24.0))
                    .pb(px(16.0))
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(table)
                    .children(skipped),
            )
            .child(footer)
            .into_any_element();
        Some(popover::modal("update-all-dialog", viewport, card))
    }
}

pub(super) fn render_update_all_button(theme: &Theme, cx: &mut Context<Shell>) -> AnyElement {
    div()
        .id("harness-update-all")
        .debug_selector(|| "harness-update-all".into())
        .h(px(26.0))
        .px(px(9.0))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(crate::typography::ui_rems(11.5))
        .font_weight(gpui::FontWeight::MEDIUM)
        .bg(theme.text)
        .text_color(theme.on_solid)
        .role(gpui::Role::Button)
        .aria_label("Update all")
        .tab_index(0)
        .cursor_pointer()
        .hover(|el| el.opacity(0.85))
        .on_click(cx.listener(|this, _, _, cx| {
            cx.stop_propagation();
            this.open_update_all(false, cx);
        }))
        .child("Update all")
        .into_any_element()
}
