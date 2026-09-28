use std::future::Future;
use std::sync::atomic::Ordering;

use serde_json::{Map, Value, json};
use zeron_proto::computer::{
    ComputerAuditEffect, ComputerAvailability, ComputerDeliveryMode, CuaComputerTask,
    McpToolResult, parse_cua_action_diagnostics,
};

use super::consent::{
    COMPUTER_APP_CONSENT_DENIED_CODE, COMPUTER_APP_WINDOW_REQUIRED_CODE, ConsentRefusal,
    app_consent_refusal_result, app_window_required_result,
};
use super::progress_guard::{ComputerProgressAction, ComputerProgressOutcome, ComputerProgressScope};
use super::results::{
    AuditOutcome, ToolFailure, ToolValue, computer_audit_error_outcome,
    computer_audit_result_window_id, computer_audit_success_effect, computer_audit_target,
    failure_result, mcp_json, mcp_json_error, result_availability, with_app_on_result,
    with_disclosure_on_result, with_guidance_on_result, with_setup_note, with_setup_note_on_result,
};
use super::tool_input::{ToolArgs, ToolInputError};
use super::{
    COMPUTER_TOOL_REFRESH_GUIDANCE, ComputerToolContext, ComputerTools,
    DISCLOSED_FIRST_MUTATIONS_MAX, computer_tool_requires_approval, is_mutating_tool_call,
};
use crate::computer::audit_log::{
    ComputerAuditEntry, computer_audit_gateway_request_id, summarize_computer_audit_args,
};
use crate::computer::backend::ComputerError;
use crate::computer::cursor_activity::cursor_tool_activity;
use crate::computer::desktop_operation_queue::{
    DesktopOperationError, OperationSignal, with_desktop_delivery_mode,
    with_desktop_operation_signal,
};
use crate::computer::manager::{COMPUTER_CONTROL_ON_DISCLOSURE, ComputerCaller};
use crate::computer::model_desktop_observation::with_model_desktop_observation;
use crate::computer::setup_signal::{ComputerSetupInput, ComputerSetupSignal, computer_setup_signal};
use crate::computer::task_context::with_computer_task;
use crate::computer::turn_timing::TurnCallKind;

type Handled = Result<ToolValue, ToolFailure>;

fn split(result: Handled) -> Result<Handled, ComputerError> {
    match result {
        Err(ToolFailure::Computer(error)) => Err(error),
        other => Ok(other),
    }
}

fn join(result: Result<Handled, ComputerError>) -> Handled {
    result.unwrap_or_else(|error| Err(ToolFailure::Computer(error)))
}

fn stable_json(value: &Value) -> String {
    match value {
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(stable_json).collect::<Vec<_>>().join(",")
        ),
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!("{}:{}", Value::String(key.clone()), stable_json(&map[key])))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        other => other.to_string(),
    }
}

pub(super) fn repeated_action_key(tool_name: &str, args: &ToolArgs) -> String {
    let rest: Map<String, Value> = args
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                "screenshot_id" | "include_screenshot" | "screenshotId" | "includeScreenshot"
            )
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    format!("{tool_name}{}", stable_json(&Value::Object(rest)))
}

fn delivery_mode(args: &ToolArgs) -> Option<&str> {
    args.get("delivery_mode").and_then(Value::as_str)
}

fn observes_desktop(name: &str) -> bool {
    matches!(
        name,
        "computer_get_state" | "computer_screenshot" | "computer_wait" | "computer_run"
    )
}

fn times_turn(name: &str, mutating: bool) -> bool {
    name != "computer_run"
        && (mutating
            || matches!(
                name,
                "computer_get_state"
                    | "computer_screenshot"
                    | "computer_get_accessibility_tree"
                    | "computer_wait"
            ))
}

impl ComputerTools {
    pub(super) fn caller(&self, context: &ComputerToolContext) -> ComputerCaller {
        ComputerCaller {
            chat_id: context.chat_id.clone(),
            turn_id: context.turn_id.clone(),
        }
    }

    pub(super) fn task(&self, context: &ComputerToolContext) -> CuaComputerTask {
        CuaComputerTask {
            thread_id: context.chat_id.clone(),
            turn_id: context.turn_id.clone(),
            label: context.chat_label.clone(),
        }
    }

    pub(super) fn assert_turn_active(
        &self,
        context: &ComputerToolContext,
    ) -> Result<(), ComputerError> {
        context.seams.assert_turn_active()?;
        if context.signal.is_cancelled() {
            return Err(DesktopOperationError::Aborted.into());
        }
        Ok(())
    }

    fn audit(
        &self,
        name: &str,
        args: &ToolArgs,
        context: &ComputerToolContext,
        result_window_id: Option<String>,
        outcome: AuditOutcome,
    ) {
        if !computer_tool_requires_approval(name) {
            return;
        }
        let entry = ComputerAuditEntry {
            thread_id: Some(context.chat_id.clone()),
            turn_id: context.turn_id.clone(),
            gateway_request_id: computer_audit_gateway_request_id(&context.request_id),
            target: computer_audit_target(args, result_window_id),
            args: Some(summarize_computer_audit_args(args)),
            code: outcome.code,
            diagnostics: outcome.diagnostics,
            layer: outcome.layer,
            ..ComputerAuditEntry::new(name, outcome.effect)
        };
        self.manager.record_computer_audit(&entry);
    }

    fn progress_action(
        &self,
        name: &str,
        args: &ToolArgs,
        context: &ComputerToolContext,
    ) -> Option<ComputerProgressAction> {
        let turn_id = context.turn_id.clone()?;
        if !is_mutating_tool_call(name) {
            return None;
        }
        let requested = args.get("window_id").or_else(|| args.get("windowId"));
        let ref_window = args
            .get("ref")
            .and_then(Value::as_u64)
            .and_then(|element_ref| {
                match self.state().elements.lookup_ref(&context.chat_id, element_ref) {
                    super::elements::RefLookup::Found(entry) => entry.window_id,
                    _ => None,
                }
            });
        let target_key = match requested.and_then(Value::as_str) {
            Some(window) => window.to_owned(),
            None => ref_window.unwrap_or_else(|| {
                args.get("app")
                    .and_then(Value::as_str)
                    .unwrap_or("selected-window")
                    .to_owned()
            }),
        };
        Some(ComputerProgressAction {
            scope: ComputerProgressScope {
                thread_id: context.chat_id.clone(),
                turn_id,
                session_key: Some(context.session_key.clone()),
            },
            target_key,
            action_key: repeated_action_key(name, args),
        })
    }

    async fn setup_card(
        &self,
        name: &str,
        context: &ComputerToolContext,
        signal: Option<&ComputerSetupSignal>,
    ) {
        if let Some(signal) = signal {
            context.seams.setup_required(name, signal).await;
        }
    }

    pub(super) async fn handle<F, Fut>(
        &self,
        name: &str,
        args: &ToolArgs,
        context: &ComputerToolContext,
        run: F,
    ) -> McpToolResult
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Handled>,
    {
        let guidance = self
            .state()
            .guidance_cadence
            .should_refresh(&context.chat_id)
            .then_some(COMPUTER_TOOL_REFRESH_GUIDANCE);
        let progress = self.progress_action(name, args, context);
        let result = match Box::pin(self.admitted(name, args, context, progress.as_ref(), run)).await {
            Ok(result) => result,
            Err(error) => self.failed(name, args, context, progress.as_ref(), error).await,
        };
        with_guidance_on_result(result, guidance)
    }

    async fn admitted<F, Fut>(
        &self,
        name: &str,
        args: &ToolArgs,
        context: &ComputerToolContext,
        progress: Option<&ComputerProgressAction>,
        run: F,
    ) -> Result<McpToolResult, ToolFailure>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Handled>,
    {
        if name == "computer_invoke_menu" && delivery_mode(args) == Some("background") {
            return Err(ToolInputError::new(
                "computer_invoke_menu requires foreground delivery and explicit visible-use authorization; it cannot preserve background focus.",
            )
            .into());
        }
        if let Some(action) = progress {
            let blocked = self.state().progress_guard.check(action);
            if let Some(blocked) = blocked {
                self.audit(
                    name,
                    args,
                    context,
                    None,
                    AuditOutcome::new(ComputerAuditEffect::Refused, Some(blocked.code.as_str())),
                );
                return Ok(mcp_json_error(&json!({ "error": blocked })));
            }
        }
        let raises = matches!(name, "computer_activate_window" | "computer_invoke_menu");
        if computer_tool_requires_approval(name) {
            let mut approval_args = args.clone();
            if raises {
                approval_args.insert("delivery_mode".into(), json!("foreground"));
            }
            let targets = self.consent_targets(name, args, &context.chat_id).await;
            match self
                .authorize_consent_targets(name, &approval_args, context, targets)
                .await?
            {
                None => {}
                Some(ConsentRefusal::Task) => {
                    self.audit(
                        name,
                        args,
                        context,
                        None,
                        AuditOutcome::new(ComputerAuditEffect::Refused, Some("approval_denied")),
                    );
                    return Ok(McpToolResult::error(
                        "Computer action was denied or cancelled; no input was sent.",
                    ));
                }
                Some(ConsentRefusal::Unscoped { step }) => {
                    self.audit(
                        name,
                        args,
                        context,
                        None,
                        AuditOutcome::new(
                            ComputerAuditEffect::Refused,
                            Some(COMPUTER_APP_WINDOW_REQUIRED_CODE),
                        ),
                    );
                    return Ok(app_window_required_result(step));
                }
                Some(ConsentRefusal::App { app, step }) => {
                    self.audit(
                        name,
                        args,
                        context,
                        None,
                        AuditOutcome::new(
                            ComputerAuditEffect::Refused,
                            Some(COMPUTER_APP_CONSENT_DENIED_CODE),
                        ),
                    );
                    return Ok(app_consent_refusal_result(&app, step));
                }
            }
        }
        if name != "computer_scroll" {
            self.state()
                .unchanged_scrolls
                .retain(|(thread, _)| *thread != context.chat_id);
        }
        self.manager
            .set_thread_label(&context.chat_id, context.chat_label.as_deref());
        let mutating = is_mutating_tool_call(name);
        let finish_timing = match (&context.turn_id, times_turn(name, mutating)) {
            (Some(turn_id), true) => Some(self.turn_timings.begin(
                &context.chat_id,
                turn_id,
                if mutating {
                    TurnCallKind::Write
                } else {
                    TurnCallKind::Observation
                },
            )),
            _ => None,
        };
        let task = self.task(context);
        let invoke = move || async move {
            let run = Box::pin(async move { split(run().await) });
            if observes_desktop(name) {
                with_computer_task(task, with_model_desktop_observation(run)).await
            } else {
                with_computer_task(task, run).await
            }
        };
        let cursor = self.manager.cursor_activity().clone();
        let activity = cursor_tool_activity(name);
        let signal: OperationSignal = context.signal.clone().into();
        let value = if name == "computer_wait" {
            self.assert_turn_active(context)?;
            let value = join(
                with_desktop_operation_signal(
                    signal,
                    cursor.during(&context.chat_id, activity, invoke()),
                )
                .await,
            );
            self.assert_turn_active(context)?;
            value
        } else {
            let foreground = delivery_mode(args) == Some("foreground") || raises;
            let operation_key = (name == "computer_type_text"
                && delivery_mode(args) != Some("foreground")
                && self.manager.supports_focus_neutral_semantic_text())
            .then(|| {
                args.get("window_id")
                    .or_else(|| args.get("windowId"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
            })
            .flatten();
            let chat_id = context.chat_id.as_str();
            let manager = &self.manager;
            join(
                manager
                    .with_agent_activity(
                        chat_id,
                        || async move {
                            self.assert_turn_active(context)?;
                            if mutating {
                                context.mutation_dispatched.store(true, Ordering::SeqCst);
                            }
                            let authorization = if foreground && !raises {
                                Some(context.seams.foreground_authorization().await)
                            } else {
                                None
                            };
                            let mode = if foreground {
                                ComputerDeliveryMode::Foreground
                            } else {
                                ComputerDeliveryMode::Background
                            };
                            with_desktop_delivery_mode(mode, async {
                                if foreground && !raises {
                                    manager
                                        .with_foreground_restore(
                                            Some(chat_id),
                                            || cursor.during(chat_id, activity, invoke()),
                                            authorization,
                                        )
                                        .await
                                } else {
                                    cursor.during(chat_id, activity, invoke()).await
                                }
                            })
                            .await
                        },
                        Some(signal),
                        context.turn_id.as_deref(),
                        operation_key,
                    )
                    .await,
            )
        };
        let value = value?;
        let result_window_id = computer_audit_result_window_id(&value);
        let result_app = result_window_id
            .as_deref()
            .or_else(|| args.get("window_id").and_then(Value::as_str))
            .and_then(|window_id| self.manager.window_app_name(window_id));
        let success = computer_audit_success_effect(name, &value);
        let availability = result_availability(&value);
        if let Some(finish) = finish_timing {
            finish(
                !matches!(
                    success,
                    ComputerAuditEffect::NotDispatched
                        | ComputerAuditEffect::Refused
                        | ComputerAuditEffect::Error
                ) && !matches!(availability, Some(ComputerAvailability::PermissionRequired { .. })),
            );
        }
        let payload = value.payload();
        let measured = payload.as_ref();
        let diagnostics = parse_cua_action_diagnostics(&json!({
            "diagnostics": {
                "scroll_delta_y": measured
                    .and_then(|payload| payload.get("scroll"))
                    .and_then(|scroll| scroll.get("traveledY"))
                    .cloned(),
                "observation": measured.and_then(|payload| payload.get("observationEvidence")).cloned(),
            },
        }));
        self.audit(
            name,
            args,
            context,
            result_window_id,
            AuditOutcome {
                diagnostics,
                ..AuditOutcome::new(success, None)
            },
        );
        if let Some(action) = progress {
            let refusal_code = (name == "computer_run" && success == ComputerAuditEffect::NotDispatched)
                .then(|| {
                    measured
                        .and_then(|payload| payload.get("steps"))
                        .and_then(Value::as_array)
                        .and_then(|steps| {
                            steps.iter().find_map(|step| {
                                step.get("error")
                                    .and_then(|error| error.get("code"))
                                    .and_then(Value::as_str)
                            })
                        })
                })
                .flatten();
            self.state().progress_guard.record(
                action,
                &ComputerProgressOutcome::new(success, refusal_code),
            );
        }
        let missing = self.manager.missing_permissions().await;
        let signal = computer_setup_signal(ComputerSetupInput {
            availability: availability.as_ref(),
            missing: &missing,
            build_signature: self.manager.build_signature(),
            ..ComputerSetupInput::default()
        });
        let mut result = match value {
            ToolValue::Result(result) => with_setup_note_on_result(result, signal.as_ref()),
            ToolValue::Json(value) => mcp_json(&with_setup_note(value, signal.as_ref())),
        };
        if let Some(app) = result_app {
            result = with_app_on_result(result, &app);
        }
        if computer_tool_requires_approval(name) {
            let key = format!(
                "{}:{}",
                context.chat_id,
                context.turn_id.as_deref().unwrap_or("no-turn")
            );
            let first = {
                let mut state = self.state();
                let first = !state.disclosed_first_mutations.contains(&key);
                if first {
                    while state.disclosed_first_mutations.len() >= DISCLOSED_FIRST_MUTATIONS_MAX {
                        state.disclosed_first_mutations.remove(0);
                    }
                    state.disclosed_first_mutations.push(key);
                }
                first
            };
            if first {
                result = with_disclosure_on_result(result, COMPUTER_CONTROL_ON_DISCLOSURE);
            }
        }
        self.setup_card(name, context, signal.as_ref()).await;
        Ok(result)
    }

    async fn failed(
        &self,
        name: &str,
        args: &ToolArgs,
        context: &ComputerToolContext,
        progress: Option<&ComputerProgressAction>,
        error: ToolFailure,
    ) -> McpToolResult {
        let outcome = computer_audit_error_outcome(&error);
        let computer = error.computer();
        if !computer.is_some_and(ComputerError::control_revoked) {
            self.audit(name, args, context, None, outcome.clone());
        }
        if let Some(action) = progress
            && !computer.is_some_and(|error| error.setup_required() || error.control_revoked())
        {
            self.state().progress_guard.record(
                action,
                &ComputerProgressOutcome::new(outcome.effect, outcome.code.as_deref()),
            );
        }
        let failure = failure_result(&error);
        let missing = self.manager.missing_permissions().await;
        let signal = computer_setup_signal(ComputerSetupInput {
            failed_for_setup: computer.is_some_and(ComputerError::setup_required),
            missing: &missing,
            build_signature: self.manager.build_signature(),
            ..ComputerSetupInput::default()
        });
        self.setup_card(name, context, signal.as_ref()).await;
        with_setup_note_on_result(failure, signal.as_ref())
    }
}
