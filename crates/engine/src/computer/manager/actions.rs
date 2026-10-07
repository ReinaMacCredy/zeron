use serde_json::Map;
use serde_json::Value;
use zeron_proto::computer::{
    COMPUTER_PROVISION_SUMMARY_MAX_LENGTH, COMPUTER_TEXT_MAX_LENGTH,
    ComputerAccessibilityTreeResult, ComputerActionResult, ComputerAvailability,
    ComputerCursorPosition, ComputerDeliveryMode, ComputerGetScreenSizeResult,
    ComputerInputModifier, ComputerLaunchAppResult, ComputerLaunchWindowReason,
    ComputerLaunchWindowStatus, ComputerListAppsResult, ComputerListWindowsResult,
    ComputerProvisionResult, ComputerRect, ComputerScreenshot, ComputerState, ComputerStatusResult,
    ComputerTarget, ComputerVerifyStateResult, ComputerZoomResult, CuaComputerTask,
};

use super::targeting::PreparedTarget;
use super::{
    COMPUTER_PASTE_RESTORE_MS, ComputerClickButton, ComputerClickGesture, ComputerManager,
    ComputerNotedResult, ComputerPasteResult, ComputerStateRequest, agent_thread_id,
    click_gesture_unsupported_error, clipboard_not_restorable_error, clipboard_unsupported_error,
    error_message, menu_app_not_found_error, rects_overlap, triple_click_unsupported_error,
    window_not_found_error,
};
use crate::computer::backend::{
    ComputerAgentDialect, ComputerBackendActionResult, ComputerBackendError, ComputerBackendMethod,
    ComputerBrowserCall, ComputerBrowserCallResult, ComputerCaptureRequest, ComputerError,
    ComputerLaunchOptions, ComputerMenuBackendTarget, ComputerMenuTarget, ComputerStateOptions,
    ComputerTextSelection, clamp_computer_message,
};
use crate::computer::call_context::{mark_computer_call, timed_computer_leg};
use crate::computer::denylist::ComputerDenylistError;
use crate::computer::desktop_operation_queue::{
    OperationSignal, assert_desktop_operation_active, desktop_operation_signal,
    with_desktop_delivery_mode,
};
use crate::computer::element_identity::ObservedComputerTarget;
use crate::computer::ui_tree_targeting::{ComputerTargetError, ComputerTargetErrorCode};
use crate::computer::ui_tree_text::describe_computer_ui_tree;
use crate::computer::utf8_truncation::{clamp_text_to_length, utf16_len};
use crate::computer::visible_use::ComputerForegroundAuthorization;
use crate::computer::wait_for_window::{InputReadyCheck, wait_for_window};

#[derive(Clone, Copy)]
enum KeyboardInput<'a> {
    Key(&'a str),
    Chord(&'a [String]),
}

impl ComputerManager {
    pub async fn availability(&self) -> Result<ComputerAvailability, ComputerError> {
        self.engage_backend();
        self.backend.availability(false).await
    }

    pub async fn get_status(&self) -> ComputerStatusResult {
        let engaged = self.state().backend_engaged;
        let availability = if engaged {
            self.backend.availability(true).await
        } else {
            self.backend.probe_availability().await
        };
        let availability =
            availability.unwrap_or_else(|error| ComputerAvailability::BackendUnavailable {
                message: clamp_computer_message(
                    &error_message(&error),
                    "The computer backend failed.",
                ),
            });
        ComputerStatusResult {
            provisionable: Some(self.backend.supports(ComputerBackendMethod::Provision)),
            computer_id: self.computer_id.clone(),
            availability: self.corrected_availability(availability),
            health: self.state().backend_health.clone(),
            capabilities: self.backend.capabilities(),
            input_stopped: None,
        }
    }

    pub fn driver_native_revision(&self) -> Option<u64> {
        self.backend.driver_native_revision()
    }

    pub async fn provision(&self) -> Result<ComputerProvisionResult, ComputerError> {
        self.engage_backend();
        if !self.backend.supports(ComputerBackendMethod::Provision) {
            return Err(ComputerError::Other(
                "This desktop backend has nothing to install.".into(),
            ));
        }
        let summary = clamp_text_to_length(
            &self.backend.provision().await?,
            COMPUTER_PROVISION_SUMMARY_MAX_LENGTH,
        );
        Ok(ComputerProvisionResult {
            summary,
            status: self.get_status().await,
        })
    }

    pub async fn list_windows(&self) -> Result<ComputerListWindowsResult, ComputerError> {
        self.engage_backend();
        let (availability, windows) =
            futures::join!(self.backend.availability(false), self.read_windows());
        Ok(ComputerListWindowsResult {
            computer_id: self.computer_id.clone(),
            windows: windows?,
            availability: availability?,
        })
    }

    pub async fn get_state(
        &self,
        request: ComputerStateRequest,
    ) -> Result<ComputerState, ComputerError> {
        self.engage_backend();
        let include_tree = request.include_tree.unwrap_or(request.include_text);
        match request.window_id.as_deref() {
            Some(window_id) => self.assert_window_content_allowed(window_id).await?,
            None => {
                if request.include_screenshot == Some(true)
                    || ((request.include_tree == Some(true) || request.include_text)
                        && self.agent_dialect() != ComputerAgentDialect::Macos)
                {
                    self.assert_no_denied_visible_window().await?;
                }
            }
        }
        let (state, availability) = futures::join!(
            self.backend.get_state(ComputerStateOptions {
                include_screenshot: request.include_screenshot == Some(true),
                include_tree,
                window_id: request.window_id.clone(),
                reuse_recent_tree: false,
            }),
            self.backend.availability(false)
        );
        let mut state = state?;
        let availability = availability?;
        if let Some(window_id) = request.window_id.as_deref() {
            self.refresh_input_pause(window_id, &state.windows).await?;
        }
        let input_pause = {
            let manager = self.state();
            manager
                .lease
                .as_ref()
                .and_then(|lease| manager.thread(&lease.thread_id))
                .and_then(|thread| thread.input_pause.clone())
                .or_else(|| {
                    let window_id = request.window_id.as_deref()?;
                    manager.threads.iter().find_map(|(_, thread)| {
                        thread
                            .input_pause
                            .as_ref()
                            .filter(|pause| pause.window_id.as_deref() == Some(window_id))
                            .cloned()
                    })
                })
        };
        state.availability = Some(self.corrected_availability(availability));
        if input_pause.is_some() {
            state.input_pause = input_pause;
        }
        if request.include_text
            && let Some(root) = &state.root
        {
            state.text = Some(describe_computer_ui_tree(root));
        }
        Ok(state)
    }

    pub(super) async fn refresh_input_pause(
        &self,
        window_id: &str,
        windows: &[zeron_proto::computer::ComputerWindow],
    ) -> Result<(), ComputerError> {
        if !self
            .backend
            .supports(ComputerBackendMethod::CheckInputReady)
        {
            return Ok(());
        }
        let observing =
            crate::computer::task_context::current_computer_task().map(|task| task.thread_id);
        let observed = windows.iter().find(|window| window.id == window_id);
        let snapshots: Vec<(String, u64, zeron_proto::computer::ComputerInputPause, u64)> = {
            let state = self.state();
            state
                .threads
                .iter()
                .filter_map(|(thread_id, thread)| {
                    let pause = thread.input_pause.as_ref()?;
                    if observing
                        .as_deref()
                        .is_some_and(|observer| observer != thread_id)
                    {
                        return None;
                    }
                    let matches = pause.window_id.is_none()
                        || pause.window_id.as_deref() == Some(window_id)
                        || (pause.pid.is_some()
                            && observed.is_some_and(|window| {
                                window.pid == pause.pid
                                    && window.visible
                                    && !window.minimized
                                    && window.on_current_space != Some(false)
                            }))
                        || (pause.pid.is_none()
                            && !windows
                                .iter()
                                .any(|window| Some(&window.id) == pause.window_id.as_ref()));
                    matches.then(|| {
                        (
                            thread_id.clone(),
                            thread.record,
                            pause.clone(),
                            self.control_state.generation(thread_id),
                        )
                    })
                })
                .collect()
        };
        if snapshots.is_empty() {
            return Ok(());
        }
        if self.backend.check_input_ready(window_id).await.is_err() {
            return Ok(());
        }
        assert_desktop_operation_active()?;
        for (thread_id, record, pause, generation) in snapshots {
            let cleared = {
                let mut state = self.state();
                let current = state.thread(&thread_id).is_some_and(|thread| {
                    thread.record == record && thread.input_pause.as_ref() == Some(&pause)
                });
                if !current
                    || state.suspended_threads.contains(&thread_id)
                    || !self.control_state.allows(&thread_id, generation)
                {
                    false
                } else {
                    let thread = state.thread_mut(&thread_id).expect("checked above");
                    thread.input_pause = None;
                    thread.last_error = None;
                    true
                }
            };
            if cleared {
                self.publish_cached(&thread_id);
            }
        }
        Ok(())
    }

    pub async fn capture_screenshot(
        &self,
        request: ComputerCaptureRequest,
    ) -> Result<ComputerScreenshot, ComputerError> {
        self.engage_backend();
        match &request {
            ComputerCaptureRequest::Window { window_id, .. } => {
                self.assert_window_content_allowed(window_id).await?;
            }
            ComputerCaptureRequest::Region { region, .. } => {
                let denied =
                    self.denied_visible_windows()
                        .await?
                        .into_iter()
                        .find(|(window, _)| {
                            window
                                .bounds
                                .as_ref()
                                .is_some_and(|bounds| rects_overlap(bounds, region))
                        });
                if let Some((_, found)) = denied {
                    return Err(ComputerDenylistError::from(found).into());
                }
            }
        }
        self.backend.capture_screenshot(request).await
    }

    pub async fn get_screen_size(&self) -> Result<ComputerGetScreenSizeResult, ComputerError> {
        self.engage_backend();
        let (availability, screen_size) = futures::join!(
            self.backend.availability(false),
            self.backend.get_screen_size()
        );
        Ok(ComputerGetScreenSizeResult {
            computer_id: self.computer_id.clone(),
            screen_size: screen_size?,
            availability: availability?,
        })
    }

    pub async fn launch_app(
        &self,
        thread_id: Option<&str>,
        app: &str,
        args: &[String],
        wait_for_window_ms: u64,
        options: ComputerLaunchOptions,
    ) -> Result<ComputerLaunchAppResult, ComputerError> {
        self.with_background_app_control(thread_id, app, || async move {
            mark_computer_call("computer_launch_app");
            assert_desktop_operation_active()?;
            self.assert_driven_app_allowed(app)?;
            self.space_broker
                .assert_native_launch_allowed(agent_thread_id(thread_id))?;
            let result =
                timed_computer_leg("dispatch", self.backend.launch_app(app, args, options))
                    .await?;
            let owner = agent_thread_id(thread_id);
            if result.focus_changed_during_launch == Some(true)
                && let Some(owner) = owner
                && crate::computer::desktop_operation_queue::desktop_delivery_mode()
                    != ComputerDeliveryMode::Foreground
            {
                self.state().thread_runtime(owner).input_pause =
                    Some(zeron_proto::computer::ComputerInputPause {
                        window_id: result.window.as_ref().map(|window| window.id.clone()),
                        pid: result.pid,
                        message: "The app changed desktop focus while launching. The launch already happened; do not replay it. Observe the app's exact window before continuing background input.".into(),
                        user_takeover: false,
                    });
                self.publish_cached(owner);
            }
            if let Some(owner) = owner {
                self.record_acted_pid(owner, result.pid);
            }
            self.emit_action(thread_id, "computer_launch_app", None);
            if result.window.is_none()
                && result.window_status != Some(ComputerLaunchWindowStatus::NoUsableWindow)
                && wait_for_window_ms > 0
            {
                let backend = self.backend.clone();
                let check: Box<InputReadyCheck> = Box::new(move |window_id: String| {
                    let backend = backend.clone();
                    Box::pin(async move {
                        backend
                            .check_input_ready(&window_id)
                            .await
                            .map_err(|error| error.to_string())
                    })
                });
                let checks = self
                    .backend
                    .supports(ComputerBackendMethod::CheckInputReady)
                    .then_some(check);
                let signal = desktop_operation_signal();
                let readiness = wait_for_window(
                    || self.read_windows(),
                    app,
                    wait_for_window_ms,
                    signal.as_ref(),
                    result.pid,
                    checks.as_deref(),
                )
                .await;
                let readiness = match readiness {
                    Ok(readiness) => readiness,
                    Err(_) => {
                        assert_desktop_operation_active()?;
                        crate::computer::wait_for_window::LaunchWindowReadiness {
                            window: None,
                            window_status: ComputerLaunchWindowStatus::NoUsableWindow,
                            window_reason: Some(ComputerLaunchWindowReason::InputUnavailable),
                        }
                    }
                };
                return Ok(ComputerLaunchAppResult {
                    window: readiness.window,
                    window_status: Some(readiness.window_status),
                    window_reason: readiness.window_reason,
                    ..result
                });
            }
            let window_status = result.window_status.unwrap_or(if result.window.is_some() {
                ComputerLaunchWindowStatus::Ready
            } else {
                ComputerLaunchWindowStatus::NotChecked
            });
            Ok(ComputerLaunchAppResult {
                window_status: Some(window_status),
                ..result
            })
        })
        .await
    }

    pub async fn list_apps(&self) -> Result<ComputerListAppsResult, ComputerError> {
        self.engage_backend();
        if !self.backend.supports(ComputerBackendMethod::ListApps) {
            return Err(
                ComputerBackendError::new("This backend cannot enumerate applications.").into(),
            );
        }
        let (availability, apps) =
            futures::join!(self.backend.availability(false), self.backend.list_apps());
        Ok(ComputerListAppsResult {
            computer_id: self.computer_id.clone(),
            apps: apps?,
            availability: availability?,
        })
    }

    pub async fn set_window_frame(
        &self,
        thread_id: Option<&str>,
        window_id: &str,
        frame: ComputerRect,
    ) -> Result<ComputerActionResult, ComputerError> {
        self.with_background_process_control(thread_id, Some(window_id), || async move {
            if !self.backend.supports(ComputerBackendMethod::SetWindowFrame) {
                return Err(ComputerBackendError::new(
                    "This backend cannot move or resize windows.",
                )
                .into());
            }
            let target = self.resolve_window_target(thread_id, window_id).await?;
            if let Some(pid) = target.pid {
                self.assert_space_app_mutation_allowed(thread_id, pid)
                    .await?;
            }
            let result =
                timed_computer_leg("dispatch", self.backend.set_window_frame(window_id, frame))
                    .await?;
            Ok(self.action_result(
                thread_id,
                "computer_set_window_frame",
                None,
                Some(result),
                Some(window_id),
            ))
        })
        .await
    }

    pub async fn invoke_menu(
        &self,
        thread_id: Option<&str>,
        target: ComputerMenuTarget,
        path: &[String],
        authorization: Option<ComputerForegroundAuthorization>,
    ) -> Result<ComputerActionResult, ComputerError> {
        self.with_foreground_restore(
            thread_id,
            || {
                with_desktop_delivery_mode(ComputerDeliveryMode::Foreground, async {
                    if !self.backend.supports(ComputerBackendMethod::InvokeMenu) {
                        return Err(ComputerBackendError::new(
                            "This backend cannot invoke menu items.",
                        )
                        .into());
                    }
                    if let ComputerMenuTarget::Window(window_id) = &target {
                        let window = self.resolve_window_target(thread_id, window_id).await?;
                        if let Some(pid) = window.pid {
                            self.assert_space_app_mutation_allowed(thread_id, pid)
                                .await?;
                        }
                        let result = timed_computer_leg(
                            "dispatch",
                            self.backend.invoke_menu(
                                &ComputerMenuBackendTarget::Window {
                                    window_id: window_id.clone(),
                                },
                                path,
                            ),
                        )
                        .await?;
                        return Ok(self.action_result(
                            thread_id,
                            "computer_invoke_menu",
                            None,
                            Some(result),
                            Some(window_id),
                        ));
                    }
                    let (pid, name) =
                        timed_computer_leg("resolve", self.resolve_menu_app_target(&target))
                            .await?;
                    let consent_key = match &target {
                        ComputerMenuTarget::App(app) => app.clone(),
                        _ => name.clone().unwrap_or_else(|| format!("pid {pid}")),
                    };
                    self.assert_driven_app_allowed(&consent_key)?;
                    if agent_thread_id(thread_id).is_some()
                        && let Some(found) = self.denied_match_for_pid(pid, name.as_deref()).await
                    {
                        return Err(ComputerDenylistError::from(found).into());
                    }
                    self.assert_space_app_mutation_allowed(thread_id, pid)
                        .await?;
                    let result = timed_computer_leg(
                        "dispatch",
                        self.backend
                            .invoke_menu(&ComputerMenuBackendTarget::Pid { pid }, path),
                    )
                    .await?;
                    Ok(self.action_result(
                        thread_id,
                        "computer_invoke_menu",
                        None,
                        Some(result),
                        None,
                    ))
                })
            },
            authorization,
        )
        .await
    }

    async fn resolve_menu_app_target(
        &self,
        target: &ComputerMenuTarget,
    ) -> Result<(u32, Option<String>), ComputerError> {
        if !self.backend.supports(ComputerBackendMethod::ListApps) {
            return Err(ComputerBackendError::new(
                "This backend cannot enumerate applications, so a menu target has to name an exact window.",
            )
            .into());
        }
        match target {
            ComputerMenuTarget::Pid(pid) => {
                if *pid == 0 {
                    return Err(ComputerTargetError::new(
                        ComputerTargetErrorCode::Invalid,
                        format!("\"pid\" must be a positive integer; got {pid}."),
                        Vec::new(),
                    )
                    .into());
                }
                let owner = self
                    .backend
                    .list_apps()
                    .await?
                    .into_iter()
                    .find(|app| app.pid == *pid && app.running);
                Ok((*pid, owner.map(|owner| owner.name)))
            }
            ComputerMenuTarget::App(app) => {
                let spelling = app.trim();
                let lowered = spelling.to_lowercase();
                let owner = self
                    .backend
                    .list_apps()
                    .await?
                    .into_iter()
                    .find(|candidate| {
                        candidate.running
                            && (candidate.name.trim().to_lowercase() == lowered
                                || candidate
                                    .bundle_id
                                    .as_deref()
                                    .is_some_and(|bundle| bundle.to_lowercase() == lowered))
                    });
                match owner {
                    Some(owner) => Ok((owner.pid, Some(owner.name))),
                    None => Err(menu_app_not_found_error(spelling)),
                }
            }
            ComputerMenuTarget::Window(window_id) => Err(window_not_found_error(window_id)),
        }
    }

    pub async fn set_window_minimized(
        &self,
        thread_id: Option<&str>,
        window_id: &str,
        minimized: bool,
    ) -> Result<ComputerActionResult, ComputerError> {
        self.with_background_process_control(thread_id, Some(window_id), || async move {
            if !self
                .backend
                .supports(ComputerBackendMethod::SetWindowMinimized)
            {
                return Err(ComputerBackendError::new(
                    "This backend cannot minimize or restore windows.",
                )
                .into());
            }
            let target = self.resolve_window_target(thread_id, window_id).await?;
            if let Some(pid) = target.pid {
                self.assert_space_app_mutation_allowed(thread_id, pid)
                    .await?;
            }
            let result = timed_computer_leg(
                "dispatch",
                self.backend.set_window_minimized(window_id, minimized),
            )
            .await?;
            Ok(self.action_result(
                thread_id,
                "computer_set_window_minimized",
                None,
                Some(result),
                Some(window_id),
            ))
        })
        .await
    }

    pub async fn set_app_visibility(
        &self,
        thread_id: Option<&str>,
        pid: u32,
        hidden: bool,
    ) -> Result<ComputerNotedResult, ComputerError> {
        self.with_desktop_control(
            thread_id,
            || async move {
                if !self.backend.supports(ComputerBackendMethod::SetAppVisibility) {
                    return Err(ComputerBackendError::new(
                        "This backend cannot hide or unhide applications.",
                    )
                    .into());
                }
                let named = timed_computer_leg("resolve", async {
                    if !self.backend.supports(ComputerBackendMethod::ListApps) {
                        return None;
                    }
                    self.backend
                        .list_apps()
                        .await
                        .ok()?
                        .into_iter()
                        .find(|app| app.pid == pid && app.running)
                        .map(|app| app.name)
                })
                .await;
                self.assert_driven_app_allowed(
                    &named.clone().unwrap_or_else(|| format!("pid {pid}")),
                )?;
                if agent_thread_id(thread_id).is_some()
                    && let Some(found) = self.denied_match_for_pid(pid, named.as_deref()).await
                {
                    return Err(ComputerDenylistError::from(found).into());
                }
                self.assert_space_app_mutation_allowed(thread_id, pid)
                    .await?;
                let result = timed_computer_leg(
                    "dispatch",
                    self.backend.set_app_visibility(pid, hidden),
                )
                .await?;
                let base = self.action_result(
                    thread_id,
                    "computer_set_app_visibility",
                    None,
                    Some(result),
                    None,
                );
                let windowless = {
                    let state = self.state();
                    !hidden
                        && state.last_known_window_ids.is_some()
                        && !state
                            .last_known_windows
                            .values()
                            .any(|window| window.pid == Some(pid))
                };
                Ok(ComputerNotedResult {
                    result: base,
                    note: windowless.then(|| {
                        "This app has no window in the last desktop listing, so the unhide had nothing to show. Create a window with the app-level computer_invoke_menu (name the app or pid, e.g. [\"File\", \"New Window\"]), or bind the driver-owned headless browser with computer_browser_prepare and computer_browser_state.".to_owned()
                    }),
                })
            },
            None,
        )
        .await
    }

    pub async fn verify_state(
        &self,
        window_id: &str,
        expect: &[Value],
    ) -> Result<ComputerVerifyStateResult, ComputerError> {
        self.engage_backend();
        if !self.backend.supports(ComputerBackendMethod::VerifyState) {
            return Err(
                ComputerBackendError::new("This backend cannot verify window state.").into(),
            );
        }
        self.assert_scoped_window_readable(window_id).await?;
        self.backend.verify_state(window_id, expect).await
    }

    pub async fn zoom_window(
        &self,
        window_id: &str,
        region: ComputerRect,
    ) -> Result<ComputerZoomResult, ComputerError> {
        self.engage_backend();
        if !self.backend.supports(ComputerBackendMethod::ZoomWindow) {
            return Err(
                ComputerBackendError::new("This backend cannot capture zoomed regions.").into(),
            );
        }
        self.assert_scoped_window_readable(window_id).await?;
        self.backend.zoom_window(window_id, region).await
    }

    pub async fn kill_app(
        &self,
        thread_id: Option<&str>,
        window_id: &str,
    ) -> Result<ComputerActionResult, ComputerError> {
        self.with_background_process_control(thread_id, Some(window_id), || async move {
            if !self.backend.supports(ComputerBackendMethod::KillApp) {
                return Err(ComputerBackendError::new(
                    "This backend cannot terminate applications.",
                )
                .into());
            }
            let windows = timed_computer_leg("resolve", self.read_windows()).await?;
            let target = windows
                .into_iter()
                .find(|candidate| candidate.id == window_id)
                .filter(|window| window.pid.is_some())
                .ok_or_else(|| window_not_found_error(window_id))?;
            self.admit_window_target(thread_id, &target).await?;
            let pid = target.pid.expect("filtered above");
            self.assert_space_app_mutation_allowed(thread_id, pid)
                .await?;
            let result = timed_computer_leg("dispatch", self.backend.kill_app(pid)).await?;
            Ok(self.action_result(
                thread_id,
                "computer_kill_app",
                None,
                Some(result),
                Some(window_id),
            ))
        })
        .await
    }

    pub async fn get_accessibility_tree(
        &self,
        window_id: Option<&str>,
    ) -> Result<ComputerAccessibilityTreeResult, ComputerError> {
        self.engage_backend();
        if !self
            .backend
            .supports(ComputerBackendMethod::GetAccessibilityTree)
        {
            return Err(ComputerBackendError::new(
                "This backend cannot read the desktop inventory.",
            )
            .into());
        }
        if let Some(window_id) = window_id {
            self.assert_scoped_window_readable(window_id).await?;
        }
        let (availability, snapshot) = futures::join!(
            self.backend.availability(false),
            self.backend.get_accessibility_tree(window_id)
        );
        let snapshot = snapshot?;
        Ok(ComputerAccessibilityTreeResult {
            computer_id: self.computer_id.clone(),
            apps: snapshot.apps,
            windows: snapshot.windows,
            window_id: window_id.map(str::to_owned),
            truncated: snapshot.truncated,
            availability: availability?,
        })
    }

    pub async fn get_cursor_position(
        &self,
        window_id: Option<&str>,
    ) -> Result<ComputerCursorPosition, ComputerError> {
        self.engage_backend();
        if !self
            .backend
            .supports(ComputerBackendMethod::GetCursorPosition)
        {
            return Err(
                ComputerBackendError::new("This backend cannot read the cursor position.").into(),
            );
        }
        if let Some(window_id) = window_id {
            self.assert_scoped_window_readable(window_id).await?;
        }
        let (availability, point) = futures::join!(
            self.backend.availability(false),
            self.backend.get_cursor_position(window_id)
        );
        let point = point?;
        Ok(ComputerCursorPosition {
            computer_id: self.computer_id.clone(),
            x: point.x,
            y: point.y,
            captured_at: point.captured_at,
            availability: availability?,
            window_id: point.window_id,
            inside_window: point.inside_window,
        })
    }

    pub async fn click(
        &self,
        thread_id: Option<&str>,
        target: impl Into<ObservedComputerTarget>,
        modifiers: &[ComputerInputModifier],
        gesture: ComputerClickGesture,
    ) -> Result<ComputerActionResult, ComputerError> {
        let target = target.into();
        let window_id = target.target.window_id.clone();
        self.with_background_process_control(thread_id, window_id.as_deref(), || async move {
            mark_computer_call("computer_click");
            let button = gesture.button.unwrap_or_default();
            let count = gesture.count.unwrap_or(1);
            let method = match (button, count) {
                (ComputerClickButton::Left, 1) => "click",
                (ComputerClickButton::Left, 2) => "doubleClick",
                (ComputerClickButton::Left, 3) => {
                    if !self.backend.supports(ComputerBackendMethod::TripleClick) {
                        return Err(triple_click_unsupported_error());
                    }
                    "tripleClick"
                }
                (ComputerClickButton::Right, 1) => "rightClick",
                (button, count) => return Err(click_gesture_unsupported_error(button, count)),
            };
            let resolved =
                timed_computer_leg("resolve", self.resolve_point_target(&target, thread_id))
                    .await?;
            let prepared = PreparedTarget::from(&resolved);
            timed_computer_leg(
                "resolve",
                self.prepare_resolved_target(Some(&prepared), thread_id),
            )
            .await?;
            if let Some(semantic) = &resolved.semantic
                && button == ComputerClickButton::Left
                && count == 1
                && modifiers.is_empty()
                && self.backend.supports_action(semantic, "AXPress")
            {
                assert_desktop_operation_active()?;
                let native = if self.agent_dialect() == ComputerAgentDialect::Macos {
                    "AXPress"
                } else {
                    "press"
                };
                let result =
                    timed_computer_leg("dispatch", self.backend.perform_action(semantic, native))
                        .await?;
                return Ok(self.action_result(
                    thread_id,
                    "computer_click",
                    Some(resolved.point),
                    Some(result),
                    resolved.window_id.as_deref(),
                ));
            }
            self.assert_target_can_use_coordinates(&target)?;
            let point = resolved.point;
            let window = resolved.window_id.clone();
            let result = self
                .inject_scoped("computer_click", &prepared, || async {
                    match method {
                        "click" => {
                            self.backend
                                .click(point, window.as_deref(), modifiers)
                                .await
                        }
                        "doubleClick" => {
                            self.backend
                                .double_click(point, window.as_deref(), modifiers)
                                .await
                        }
                        "tripleClick" => {
                            self.backend
                                .triple_click(point, window.as_deref(), modifiers)
                                .await
                        }
                        _ => {
                            self.backend
                                .right_click(point, window.as_deref(), modifiers)
                                .await
                        }
                    }
                })
                .await?;
            Ok(self.action_result(
                thread_id,
                "computer_click",
                Some(resolved.point),
                Some(result),
                resolved.window_id.as_deref(),
            ))
        })
        .await
    }

    pub async fn double_click(
        &self,
        thread_id: Option<&str>,
        target: impl Into<ObservedComputerTarget>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerActionResult, ComputerError> {
        self.click(
            thread_id,
            target,
            modifiers,
            ComputerClickGesture {
                count: Some(2),
                button: None,
            },
        )
        .await
    }

    pub async fn triple_click(
        &self,
        thread_id: Option<&str>,
        target: impl Into<ObservedComputerTarget>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerActionResult, ComputerError> {
        self.click(
            thread_id,
            target,
            modifiers,
            ComputerClickGesture {
                count: Some(3),
                button: None,
            },
        )
        .await
    }

    pub async fn right_click(
        &self,
        thread_id: Option<&str>,
        target: impl Into<ObservedComputerTarget>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerActionResult, ComputerError> {
        self.click(
            thread_id,
            target,
            modifiers,
            ComputerClickGesture {
                count: None,
                button: Some(ComputerClickButton::Right),
            },
        )
        .await
    }

    pub async fn move_cursor(
        &self,
        thread_id: Option<&str>,
        target: impl Into<ObservedComputerTarget>,
    ) -> Result<ComputerActionResult, ComputerError> {
        let target = target.into();
        self.assert_target_can_use_coordinates(&target)?;
        let window_id = target.target.window_id.clone();
        self.with_background_process_control(thread_id, window_id.as_deref(), || async move {
            let resolved =
                timed_computer_leg("resolve", self.resolve_point_target(&target, thread_id))
                    .await?;
            let prepared = PreparedTarget::from(&resolved);
            timed_computer_leg("resolve", self.reveal_target(Some(&prepared))).await?;
            let result = self
                .inject_scoped("computer_move_cursor", &prepared, || {
                    self.backend
                        .move_cursor(resolved.point, resolved.window_id.as_deref())
                })
                .await?;
            Ok(self.action_result(
                thread_id,
                "computer_move_cursor",
                Some(resolved.point),
                Some(result),
                resolved.window_id.as_deref(),
            ))
        })
        .await
    }

    pub async fn drag(
        &self,
        thread_id: Option<&str>,
        from: impl Into<ObservedComputerTarget>,
        to: impl Into<ObservedComputerTarget>,
        duration_ms: Option<f64>,
    ) -> Result<ComputerActionResult, ComputerError> {
        let from = from.into();
        let to = to.into();
        self.assert_target_can_use_coordinates(&from)?;
        self.assert_target_can_use_coordinates(&to)?;
        let duration_ms = duration_ms.unwrap_or(250.0);
        self.with_desktop_control(
            thread_id,
            || async move {
                let (resolved_from, resolved_to) = timed_computer_leg("resolve", async {
                    let (from, to) = futures::join!(
                        self.resolve_point_target(&from, thread_id),
                        self.resolve_point_target(&to, thread_id)
                    );
                    Ok::<_, ComputerError>((from?, to?))
                })
                .await?;
                let grabbed = if resolved_from.window_id.is_some() {
                    &resolved_from
                } else {
                    &resolved_to
                };
                let prepared = PreparedTarget::from(grabbed);
                timed_computer_leg(
                    "resolve",
                    self.prepare_resolved_target(Some(&prepared), thread_id),
                )
                .await?;
                let result = self
                    .inject_scoped("computer_drag", &prepared, || {
                        self.backend.drag(
                            resolved_from.point,
                            resolved_to.point,
                            duration_ms,
                            resolved_from.window_id.as_deref(),
                        )
                    })
                    .await?;
                Ok(self.action_result(
                    thread_id,
                    "computer_drag",
                    Some(resolved_to.point),
                    Some(result),
                    resolved_to
                        .window_id
                        .as_deref()
                        .or(resolved_from.window_id.as_deref()),
                ))
            },
            None,
        )
        .await
    }

    pub async fn type_text(
        &self,
        thread_id: Option<&str>,
        text: &str,
        window_id: Option<&str>,
    ) -> Result<ComputerActionResult, ComputerError> {
        if self.supports_focus_neutral_semantic_text()
            && let Some(window_id) = window_id
        {
            let target = ComputerTarget {
                window_id: Some(window_id.to_owned()),
                ..ComputerTarget::default()
            };
            match self.type_text_at(thread_id, text, target).await {
                Ok(result) => return Ok(result),
                Err(ComputerError::Target(error)) if error.unresolved_text_control => {}
                Err(error) => return Err(error),
            }
            return self
                .with_background_process_control(thread_id, Some(window_id), || async move {
                    let result = self
                        .run_keyboard_dispatch(thread_id, Some(window_id), || {
                            self.backend.type_text(text, Some(window_id), None)
                        })
                        .await?;
                    Ok(self.action_result(
                        thread_id,
                        "computer_type_text",
                        None,
                        Some(result),
                        Some(window_id),
                    ))
                })
                .await;
        }
        self.with_desktop_control(
            thread_id,
            || async move {
                let result = self
                    .run_keyboard_dispatch(thread_id, window_id, || {
                        self.backend.type_text(text, window_id, None)
                    })
                    .await?;
                Ok(self.action_result(
                    thread_id,
                    "computer_type_text",
                    None,
                    Some(result),
                    window_id,
                ))
            },
            None,
        )
        .await
    }

    pub async fn type_text_at(
        &self,
        thread_id: Option<&str>,
        text: &str,
        target: impl Into<ObservedComputerTarget>,
    ) -> Result<ComputerActionResult, ComputerError> {
        let target = target.into();
        if !self.supports_focus_neutral_semantic_text() {
            return Err(ComputerBackendError::new(
                "This computer backend cannot guarantee focus-neutral semantic text input.",
            )
            .into());
        }
        let Some(window_id) = target.target.window_id.clone() else {
            return Err(ComputerBackendError::new(
                "Focus-neutral text input requires an exact target window.",
            )
            .into());
        };
        self.with_background_window_control(thread_id, &window_id, || async {
            let resolved =
                timed_computer_leg("resolve", self.resolve_semantic_target(&target, true)).await?;
            assert_desktop_operation_active()?;
            let result = timed_computer_leg(
                "dispatch",
                self.backend
                    .type_text(text, Some(&window_id), Some(&resolved)),
            )
            .await?;
            Ok(self.action_result(
                thread_id,
                "computer_type_text",
                Some(resolved.point),
                Some(result),
                Some(&window_id),
            ))
        })
        .await
    }

    pub async fn press_key(
        &self,
        thread_id: Option<&str>,
        key: &str,
        window_id: Option<&str>,
        target: Option<ObservedComputerTarget>,
    ) -> Result<ComputerActionResult, ComputerError> {
        self.keyboard_action(thread_id, window_id, target, KeyboardInput::Key(key))
            .await
    }

    pub async fn hotkey(
        &self,
        thread_id: Option<&str>,
        keys: &[String],
        window_id: Option<&str>,
        target: Option<ObservedComputerTarget>,
    ) -> Result<ComputerActionResult, ComputerError> {
        self.keyboard_action(thread_id, window_id, target, KeyboardInput::Chord(keys))
            .await
    }

    async fn keyboard_action(
        &self,
        thread_id: Option<&str>,
        window_id: Option<&str>,
        target: Option<ObservedComputerTarget>,
        input: KeyboardInput<'_>,
    ) -> Result<ComputerActionResult, ComputerError> {
        let exact = self.keyboard_target_window(window_id, target.as_ref())?;
        let exact_ref = exact.as_deref();
        self.with_background_process_control(thread_id, exact_ref, || async move {
            let resolved = match &target {
                Some(target) => {
                    let mut scoped = target.clone();
                    if let Some(window) = exact_ref {
                        scoped.target.window_id = Some(window.to_owned());
                    }
                    Some(self.resolve_semantic_target(&scoped, true).await?)
                }
                None => None,
            };
            let result = self
                .run_keyboard_dispatch(thread_id, exact_ref, || async {
                    match input {
                        KeyboardInput::Key(key) => {
                            self.backend
                                .press_key(key, exact_ref, resolved.as_ref())
                                .await
                        }
                        KeyboardInput::Chord(keys) => {
                            self.backend
                                .hotkey(keys, exact_ref, resolved.as_ref())
                                .await
                        }
                    }
                })
                .await?;
            Ok(self.action_result(
                thread_id,
                "computer_press_key",
                resolved.as_ref().map(|resolved| resolved.point),
                Some(result),
                exact_ref,
            ))
        })
        .await
    }

    pub async fn read_clipboard(
        &self,
        thread_id: Option<&str>,
    ) -> Result<ComputerActionResult, ComputerError> {
        self.with_desktop_control(
            thread_id,
            || async move {
                if !self.backend.supports(ComputerBackendMethod::ReadClipboard) {
                    return Err(clipboard_unsupported_error());
                }
                let value = timed_computer_leg("dispatch", self.backend.read_clipboard()).await?;
                let length = utf16_len(&value);
                if length > COMPUTER_TEXT_MAX_LENGTH {
                    return Err(ComputerBackendError::new(format!(
                        "The desktop clipboard holds {length} characters of text, more than the {COMPUTER_TEXT_MAX_LENGTH} this tool returns."
                    ))
                    .into());
                }
                Ok(self.action_result(
                    thread_id,
                    "computer_read_clipboard",
                    None,
                    Some(ComputerBackendActionResult {
                        value: Some(value),
                        ..ComputerBackendActionResult::default()
                    }),
                    None,
                ))
            },
            None,
        )
        .await
    }

    pub async fn write_clipboard(
        &self,
        thread_id: Option<&str>,
        text: &str,
    ) -> Result<ComputerActionResult, ComputerError> {
        self.with_desktop_control(
            thread_id,
            || async move {
                if !self.backend.supports(ComputerBackendMethod::WriteClipboard) {
                    return Err(clipboard_unsupported_error());
                }
                timed_computer_leg("dispatch", self.backend.write_clipboard(text)).await?;
                Ok(self.action_result(thread_id, "computer_write_clipboard", None, None, None))
            },
            None,
        )
        .await
    }

    pub async fn paste(
        &self,
        thread_id: Option<&str>,
        text: &str,
        window_id: Option<&str>,
    ) -> Result<ComputerPasteResult, ComputerError> {
        self.with_desktop_control(
            thread_id,
            || async move {
                if !self.backend.supports(ComputerBackendMethod::ReadClipboard)
                    || !self.backend.supports(ComputerBackendMethod::WriteClipboard)
                {
                    return Err(clipboard_unsupported_error());
                }
                let previous =
                    match timed_computer_leg("dispatch", self.backend.read_clipboard()).await {
                        Ok(text) => Some(text),
                        // Pasting would replace content that cannot be put back.
                        Err(error)
                            if error.code()
                                == Some(
                                    crate::computer::cua_backend::CLIPBOARD_NOT_RESTORABLE_CODE,
                                ) =>
                        {
                            return Err(clipboard_not_restorable_error());
                        }
                        Err(_) => None,
                    };
                timed_computer_leg("dispatch", self.backend.write_clipboard(text)).await?;
                let keys: Vec<String> = if self.agent_dialect() == ComputerAgentDialect::Macos {
                    vec!["meta".into(), "v".into()]
                } else {
                    vec!["ctrl".into(), "v".into()]
                };
                let result = self
                    .run_keyboard_dispatch(thread_id, window_id, || {
                        self.backend.hotkey(&keys, window_id, None)
                    })
                    .await;
                let mut restored = false;
                if let Some(previous) = previous {
                    timed_computer_leg(
                        "settle",
                        tokio::time::sleep(std::time::Duration::from_millis(
                            COMPUTER_PASTE_RESTORE_MS,
                        )),
                    )
                    .await;
                    restored = self.backend.write_clipboard(&previous).await.is_ok();
                }
                let result = result?;
                Ok(ComputerPasteResult {
                    result: self.action_result(
                        thread_id,
                        "computer_paste",
                        None,
                        Some(result),
                        window_id,
                    ),
                    clipboard_restored: restored,
                })
            },
            None,
        )
        .await
    }

    pub async fn set_value(
        &self,
        thread_id: Option<&str>,
        target: impl Into<ObservedComputerTarget>,
        value: &str,
    ) -> Result<ComputerActionResult, ComputerError> {
        let target = target.into();
        let window_id = target.target.window_id.clone();
        self.with_semantic_control(thread_id, window_id.as_deref(), || async {
            let resolved = self
                .prepare_semantic_dispatch(&target, thread_id, false)
                .await?;
            let result =
                timed_computer_leg("dispatch", self.backend.set_value(&resolved, value)).await?;
            Ok(self.action_result(
                thread_id,
                "computer_set_value",
                Some(resolved.point),
                Some(result),
                resolved.node.window_id.as_deref(),
            ))
        })
        .await
    }

    pub async fn perform_action(
        &self,
        thread_id: Option<&str>,
        target: impl Into<ObservedComputerTarget>,
        action: &str,
    ) -> Result<ComputerActionResult, ComputerError> {
        let target = target.into();
        let window_id = target.target.window_id.clone();
        self.with_background_process_control(thread_id, window_id.as_deref(), || async {
            let resolved = self
                .prepare_semantic_dispatch(&target, thread_id, false)
                .await?;
            let result =
                timed_computer_leg("dispatch", self.backend.perform_action(&resolved, action))
                    .await?;
            Ok(self.action_result(
                thread_id,
                "computer_perform_action",
                Some(resolved.point),
                Some(result),
                resolved.node.window_id.as_deref(),
            ))
        })
        .await
    }

    pub async fn select_text(
        &self,
        thread_id: Option<&str>,
        target: impl Into<ObservedComputerTarget>,
        selection: impl Into<ComputerTextSelection>,
    ) -> Result<ComputerActionResult, ComputerError> {
        let target = target.into();
        let selection = selection.into();
        let window_id = target.target.window_id.clone();
        self.with_semantic_control(thread_id, window_id.as_deref(), || async {
            let resolved = self
                .prepare_semantic_dispatch(&target, thread_id, true)
                .await?;
            let range = match &selection {
                ComputerTextSelection::Range(range) => *range,
                ComputerTextSelection::Match(matcher) => matcher
                    .range_in(resolved.node.value.as_deref().unwrap_or_default())
                    .map_err(|message| {
                        ComputerTargetError::new(
                            ComputerTargetErrorCode::NotFound,
                            message,
                            Vec::new(),
                        )
                    })?,
            };
            let result =
                timed_computer_leg("dispatch", self.backend.select_text(&resolved, range)).await?;
            Ok(self.action_result(
                thread_id,
                "computer_select_text",
                Some(resolved.point),
                Some(result),
                resolved.node.window_id.as_deref(),
            ))
        })
        .await
    }

    pub async fn browser_call<B, BFut>(
        &self,
        thread_id: &str,
        turn_id: Option<&str>,
        name: &str,
        args: Map<String, Value>,
        signal: Option<OperationSignal>,
        before_dispatch: Option<B>,
    ) -> Result<ComputerBrowserCallResult, ComputerError>
    where
        B: FnOnce() -> BFut,
        BFut: std::future::Future<Output = Result<(), ComputerError>>,
    {
        let Some(browser) = self.backend.browser() else {
            return Err(ComputerBackendError::new(
                "This computer backend does not provide browser automation.",
            )
            .into());
        };
        let key = format!("browser:{thread_id}");
        self.with_agent_activity(
            thread_id,
            || async move {
                let Some(operation) = desktop_operation_signal() else {
                    return Err(ComputerBackendError::new(
                        "Computer browser call ran outside an operation context.",
                    )
                    .into());
                };
                operation.check()?;
                let rechecked = before_dispatch.is_some();
                if let Some(before_dispatch) = before_dispatch {
                    before_dispatch().await?;
                }
                operation.check()?;
                let windowed =
                    name == "browser_prepare" && args.get("windowed") == Some(&Value::Bool(true));
                if windowed {
                    self.space_broker
                        .assert_foreground_allowed(Some(thread_id))?;
                }
                let call = ComputerBrowserCall {
                    name: name.to_owned(),
                    args,
                    task: CuaComputerTask {
                        thread_id: thread_id.to_owned(),
                        turn_id: turn_id.map(str::to_owned),
                        label: None,
                    },
                    mutation: name != "get_browser_state",
                    signal: Some(operation),
                };
                if rechecked && windowed {
                    with_desktop_delivery_mode(ComputerDeliveryMode::Foreground, browser.call(call))
                        .await
                } else {
                    browser.call(call).await
                }
            },
            signal,
            turn_id,
            Some(&key),
        )
        .await
    }
}
