use std::future::Future;

use futures::future::BoxFuture;
use serde_json::json;
use zeron_proto::computer::{ComputerActionResult, ComputerEvent, ComputerWindow, CuaEffect};

use super::{
    ComputerManager, ComputerManagerEvent, ComputerNotedResult, ForegroundRestoreInfo,
    ForegroundRestoreStatus, activation_unsupported_error, agent_thread_id, json_string,
    window_not_found_error,
};
use crate::computer::backend::{
    ComputerAgentDialect, ComputerBackendActionResult, ComputerBackendError, ComputerBackendMethod,
    ComputerError, ComputerShieldTarget, CuaActionError, clamp_computer_message,
    computer_backend_action_result,
};
use crate::computer::call_context::{
    current_computer_call, mark_computer_call, timed_computer_leg,
};
use crate::computer::desktop_operation_queue::{
    assert_desktop_operation_active, without_desktop_cancellation,
};
use crate::computer::shield::masked_activation_opted_in;
use crate::computer::visible_use::{
    COMPUTER_FOREGROUND_NOT_REQUESTED_CODE, COMPUTER_FOREGROUND_USER_INTERACTION_CODE,
    COMPUTER_USER_INTERACTION_QUIET_MS, ComputerForegroundAuthorization,
};

fn frontmost_id(windows: &[ComputerWindow]) -> Option<String> {
    windows
        .iter()
        .find(|window| window.visible && !window.minimized)
        .map(|window| window.id.clone())
}

impl ComputerManager {
    pub(super) fn assert_foreground_allowed(
        &self,
        thread_id: Option<&str>,
        authorization: Option<ComputerForegroundAuthorization>,
    ) -> Result<(), ComputerError> {
        if agent_thread_id(thread_id).is_none() {
            return Ok(());
        }
        if !authorization.is_some_and(|authorization| authorization.user_requested_visible_use) {
            return Err(CuaActionError::new(
                "The user's task did not ask for this app or window to be shown. Stay in the background: keep observing and acting through background input, or ask the user to confirm they want to watch — their reply that asks to see the screen authorizes the raise.",
                CuaEffect::NotDispatched,
            )
            .code(COMPUTER_FOREGROUND_NOT_REQUESTED_CODE)
            .into());
        }
        let last_input = self.state().last_user_desktop_input_at;
        if let Some(last_input) = last_input
            && self.now_ms().saturating_sub(last_input) < COMPUTER_USER_INTERACTION_QUIET_MS
        {
            return Err(CuaActionError::new(
                "The user was interacting with the desktop moments ago; bringing a window forward now would take their focus. Wait for the desktop to be quiet, then retry if the task still needs foreground delivery.",
                CuaEffect::NotDispatched,
            )
            .code(COMPUTER_FOREGROUND_USER_INTERACTION_CODE)
            .into());
        }
        Ok(())
    }

    fn can_raise(&self) -> bool {
        self.backend.supports(ComputerBackendMethod::RaiseWindow)
            && self.backend.capabilities().raise
    }

    pub async fn activate_window(
        &self,
        thread_id: Option<&str>,
        window_id: &str,
        authorization: Option<ComputerForegroundAuthorization>,
    ) -> Result<ComputerActionResult, ComputerError> {
        let gate = move || self.assert_foreground_allowed(thread_id, authorization);
        self.with_desktop_control(
            thread_id,
            || async move {
                mark_computer_call("computer_activate_window");
                if !self.can_raise() {
                    return Err(activation_unsupported_error());
                }
                let target = self.resolve_window_target(thread_id, window_id).await?;
                if let Some(pid) = target.pid {
                    self.assert_space_app_mutation_allowed(thread_id, pid)
                        .await?;
                }
                timed_computer_leg("dispatch", async {
                    self.backend.raise_window(window_id).await?;
                    assert_desktop_operation_active()?;
                    self.backend.focus_window(window_id).await
                })
                .await?;
                Ok(self.action_result(
                    thread_id,
                    "computer_activate_window",
                    None,
                    None,
                    Some(window_id),
                ))
            },
            Some(&gate),
        )
        .await
    }

    pub async fn foreground_with_restore(
        &self,
        thread_id: Option<&str>,
        window_id: &str,
        input: Option<BoxFuture<'_, Result<(), ComputerError>>>,
        authorization: Option<ComputerForegroundAuthorization>,
    ) -> Result<ComputerNotedResult, ComputerError> {
        if let Some(call) = current_computer_call()
            && let Some(timing) = &call.timing
        {
            timing.count("foreground_excursion", 1);
        }
        let gate = move || self.assert_foreground_allowed(thread_id, authorization);
        self.with_desktop_control(
            thread_id,
            || async move {
                mark_computer_call("computer_activate_window");
                if !self.can_raise() {
                    return Err(activation_unsupported_error());
                }
                let windows = timed_computer_leg("resolve", self.read_windows()).await?;
                let target = windows
                    .iter()
                    .find(|candidate| candidate.id == window_id)
                    .cloned()
                    .ok_or_else(|| window_not_found_error(window_id))?;
                let previous = frontmost_id(&windows);
                self.assert_driven_app_allowed(target.app_name.as_deref().unwrap_or(window_id))?;
                self.assert_window_input_allowed_window(thread_id, &target)
                    .await?;
                if let Some(pid) = target.pid {
                    self.assert_space_app_mutation_allowed(thread_id, pid)
                        .await?;
                }
                let shield = self.engage_activation_shield(&target).await?;
                let outcome = async {
                    let (restore, note) = self.run_excursion(window_id, previous, input).await?;
                    let _ = self.read_windows().await;
                    let merged = computer_backend_action_result(
                        &self.computer_id,
                        "computer_activate_window",
                        ComputerBackendActionResult {
                            window_id: Some(window_id.to_owned()),
                            ..ComputerBackendActionResult::default()
                        },
                    );
                    self.emit_foreground_restore_action(
                        thread_id,
                        &merged,
                        restore,
                        note.as_deref(),
                        shield.is_some(),
                    );
                    Ok(ComputerNotedResult {
                        result: merged,
                        note,
                    })
                }
                .await;
                if let Some(shield) = &shield {
                    self.release_activation_shield(shield).await;
                }
                outcome
            },
            Some(&gate),
        )
        .await
    }

    async fn run_excursion(
        &self,
        window_id: &str,
        previous: Option<String>,
        input: Option<BoxFuture<'_, Result<(), ComputerError>>>,
    ) -> Result<(ForegroundRestoreInfo, Option<String>), ComputerError> {
        timed_computer_leg("dispatch", async {
            self.backend.raise_window(window_id).await?;
            assert_desktop_operation_active()?;
            self.backend.focus_window(window_id).await
        })
        .await?;
        if let Some(input) = input {
            let outcome = match assert_desktop_operation_active() {
                Ok(()) => input.await,
                Err(error) => Err(error.into()),
            };
            if let Err(error) = outcome {
                if let Some(previous) = previous.as_deref().filter(|id| *id != window_id) {
                    let _ = self.backend.raise_window(previous).await;
                    let _ = self.backend.focus_window(previous).await;
                }
                return Err(error);
            }
        }
        Ok(match previous {
            None => (
                ForegroundRestoreInfo {
                    restored_window_id: None,
                    restore_status: ForegroundRestoreStatus::FrontmostUnobservable,
                },
                Some(
                    "No frontmost window was observable before activation, so nothing was restored."
                        .to_owned(),
                ),
            ),
            Some(previous) if previous == window_id => (
                ForegroundRestoreInfo {
                    restored_window_id: None,
                    restore_status: ForegroundRestoreStatus::AlreadyFrontmost,
                },
                None,
            ),
            Some(previous) => {
                let restored = async {
                    assert_desktop_operation_active()?;
                    self.backend.raise_window(&previous).await?;
                    self.backend.focus_window(&previous).await
                }
                .await;
                match restored {
                    Ok(()) => (
                        ForegroundRestoreInfo {
                            restored_window_id: Some(previous),
                            restore_status: ForegroundRestoreStatus::Restored,
                        },
                        None,
                    ),
                    Err(_) => {
                        let note = format!(
                            "Activated window {} but could not restore the previously frontmost window {} to the foreground; the desktop was left with {} raised.",
                            json_string(window_id),
                            json_string(&previous),
                            json_string(window_id)
                        );
                        (
                            ForegroundRestoreInfo {
                                restored_window_id: Some(previous),
                                restore_status: ForegroundRestoreStatus::RestoreMissed,
                            },
                            Some(note),
                        )
                    }
                }
            }
        })
    }

    pub async fn with_foreground_restore<T, F, Fut>(
        &self,
        thread_id: Option<&str>,
        action: F,
        authorization: Option<ComputerForegroundAuthorization>,
    ) -> Result<T, ComputerError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, ComputerError>>,
    {
        if let Some(call) = current_computer_call()
            && let Some(timing) = &call.timing
        {
            timing.count("foreground_excursion", 1);
        }
        let gate = move || {
            self.assert_foreground_allowed(thread_id, authorization)?;
            self.space_broker
                .assert_foreground_allowed(agent_thread_id(thread_id))?;
            Ok(())
        };
        self.with_desktop_control(
            thread_id,
            || async move {
                let before = timed_computer_leg("resolve", self.read_windows()).await?;
                let previous = frontmost_id(&before);
                let outcome = action().await;
                if let Some(previous) = previous
                    && self.can_raise()
                {
                    let frontmost = timed_computer_leg("resolve", self.read_windows())
                        .await
                        .ok()
                        .map(|after| frontmost_id(&after));
                    match frontmost {
                        None => self.warn(
                            "[computer] foreground call left focus unverified",
                            json!({ "previousWindowId": previous }),
                        ),
                        Some(Some(frontmost)) if frontmost != previous => {
                            let restored = async {
                                assert_desktop_operation_active()?;
                                self.backend.raise_window(&previous).await?;
                                self.backend.focus_window(&previous).await
                            }
                            .await;
                            if let Err(error) = restored {
                                self.warn(
                                    "[computer] foreground call left focus unrestored",
                                    json!({
                                        "restoredWindowId": previous,
                                        "error": error.to_string(),
                                    }),
                                );
                            }
                        }
                        Some(_) => {}
                    }
                }
                outcome
            },
            Some(&gate),
        )
        .await
    }

    fn emit_foreground_restore_action(
        &self,
        thread_id: Option<&str>,
        result: &ComputerActionResult,
        restore: ForegroundRestoreInfo,
        note: Option<&str>,
        masked: bool,
    ) {
        let attributed = agent_thread_id(thread_id);
        if let Some(attributed) = attributed {
            self.surface_pane_for_agent(attributed);
        }
        self.emit(ComputerManagerEvent {
            event: ComputerEvent::Action {
                window_id: result.window_id.clone(),
                delivery: result.delivery.clone(),
                action: "computer_activate_window".into(),
                ok: true,
                message: note.map(|note| {
                    clamp_computer_message(note, "The foreground window could not be restored.")
                }),
                thread_id: attributed.map(str::to_owned),
            },
            restore: Some(restore),
            masked,
        });
    }

    async fn engage_activation_shield(
        &self,
        target: &ComputerWindow,
    ) -> Result<Option<String>, ComputerError> {
        if !self.env.masked_activation
            || self.agent_dialect() != ComputerAgentDialect::Macos
            || self.env.masked_apps.is_empty()
        {
            return Ok(None);
        }
        let owner = match target.pid {
            Some(pid) => self
                .running_apps_for_denylist()
                .await
                .into_iter()
                .find(|candidate| candidate.pid == pid),
            None => None,
        };
        if !masked_activation_opted_in(
            &self.env.masked_apps,
            owner.as_ref().and_then(|owner| owner.bundle_id.as_deref()),
        ) {
            return Ok(None);
        }
        let Some(frame) = target
            .bounds
            .filter(|_| self.backend.supports(ComputerBackendMethod::EngageShield))
        else {
            return Err(ComputerBackendError::new(
                "Masked activation is armed for this app but the activation shield is unavailable; the window was not raised.",
            )
            .into());
        };
        let title = target.title.trim();
        let name = if !title.is_empty() {
            title
        } else {
            target.app_name.as_deref().unwrap_or("this window")
        };
        let shield_id = format!("shield-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let engaged = timed_computer_leg(
            "shield",
            self.backend.engage_shield(&ComputerShieldTarget {
                shield_id: shield_id.clone(),
                window_id: target.id.clone(),
                frame,
                label: format!("Zeron is activating {name}"),
            }),
        )
        .await;
        match engaged {
            Ok(id) => Ok(Some(id)),
            Err(error) => {
                self.release_activation_shield(&shield_id).await;
                match error {
                    ComputerError::Backend(_) => Err(error),
                    other => Err(ComputerBackendError::new(format!(
                        "The activation shield could not be shown, so the window was not raised: {other}"
                    ))
                    .into()),
                }
            }
        }
    }

    async fn release_activation_shield(&self, shield_id: &str) {
        if !self.backend.supports(ComputerBackendMethod::ReleaseShield) {
            return;
        }
        if let Err(error) =
            without_desktop_cancellation(self.backend.release_shield(shield_id)).await
        {
            self.warn(
                "[computer] activation shield release failed",
                json!({ "shieldId": shield_id, "error": error.to_string() }),
            );
        }
    }
}
