use serde_json::{Value, json};
use zeron_proto::computer::{ComputerConsentApp, CuaEffect, McpToolResult};

use super::args::{read_nested_screenshot_target, read_screenshot_target};
use super::results::mcp_json_error;
use super::tool_input::{ToolArgs, is_safe_integer, read_number_arg};
use super::{ComputerToolContext, ComputerTools};
use crate::computer::backend::{ComputerError, cua_effect_name};
use crate::computer::manager::{ComputerConsent, ComputerConsentTarget};

pub const COMPUTER_APP_CONSENT_DENIED_CODE: &str = "app_consent_denied";

const READ_ONLY_RUN_STEPS: &[&str] = &["wait", "get_state", "verify_state"];

pub(super) enum ConsentRefusal {
    Task,
    App {
        app: ComputerConsentApp,
        step: Option<usize>,
    },
}

pub fn app_consent_refusal_fields(app: &ComputerConsentApp) -> Value {
    let identity = match &app.bundle_id {
        Some(bundle_id) => format!("{} ({bundle_id})", app.name),
        None => format!("{} (unverified app)", app.name),
    };
    json!({
        "code": COMPUTER_APP_CONSENT_DENIED_CODE,
        "message": format!("Computer access to {identity} was denied or cancelled; no input was sent. Do not retry this app in this turn."),
        "effect": cua_effect_name(CuaEffect::NotDispatched),
        "retryAllowed": false,
        "app": {
            "name": app.name,
            "bundleId": app.bundle_id,
            "pid": app.pid,
            "verified": app.verified(),
        },
    })
}

pub(super) fn app_consent_refusal_result(
    app: &ComputerConsentApp,
    step: Option<usize>,
) -> McpToolResult {
    let mut body = json!({ "error": app_consent_refusal_fields(app) });
    if let Some(step) = step {
        body["step"] = json!(step);
    }
    mcp_json_error(&body)
}

fn text(args: &ToolArgs, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn positive_pid(args: &ToolArgs) -> Option<u32> {
    read_number_arg(args, "pid")
        .ok()
        .flatten()
        .filter(|pid| is_safe_integer(*pid) && *pid > 0.0 && *pid <= f64::from(u32::MAX))
        .map(|pid| pid as u32)
}

impl ComputerTools {
    fn target_window(&self, args: &ToolArgs, thread: &str) -> Option<String> {
        read_screenshot_target(args)
            .ok()
            .and_then(|raw| self.resolve_target(raw, thread).ok())
            .and_then(|resolved| resolved.target.window_id)
            .or_else(|| text(args, "window_id"))
            .or_else(|| text(args, "windowId"))
    }

    fn call_consent_targets(
        &self,
        name: &str,
        args: &ToolArgs,
        thread: &str,
    ) -> Vec<ComputerConsentTarget> {
        match name {
            "computer_launch_app" => text(args, "app")
                .map(ComputerConsentTarget::App)
                .into_iter()
                .collect(),
            "computer_read_clipboard" | "computer_write_clipboard" => Vec::new(),
            "computer_drag" => ["from", "to"]
                .into_iter()
                .filter_map(|end| {
                    let raw = read_nested_screenshot_target(args, end).ok()?;
                    self.resolve_target(raw, thread).ok()?.target.window_id
                })
                .map(ComputerConsentTarget::Window)
                .collect(),
            _ => {
                let mut targets = Vec::new();
                targets.extend(positive_pid(args).map(ComputerConsentTarget::Pid));
                targets.extend(text(args, "app").map(ComputerConsentTarget::App));
                targets.extend(
                    self.target_window(args, thread)
                        .map(ComputerConsentTarget::Window),
                );
                targets
            }
        }
    }

    pub(super) fn consent_targets(
        &self,
        name: &str,
        args: &ToolArgs,
        thread: &str,
    ) -> Vec<(Option<usize>, ComputerConsentTarget)> {
        let mut found: Vec<(Option<usize>, ComputerConsentTarget)> = Vec::new();
        let mut push = |step: Option<usize>, targets: Vec<ComputerConsentTarget>| {
            for target in targets {
                if !found.iter().any(|(_, seen)| *seen == target) {
                    found.push((step, target));
                }
            }
        };
        if name != "computer_run" {
            push(None, self.call_consent_targets(name, args, thread));
            return found;
        }
        let Some(Value::Array(steps)) = args.get("steps") else {
            return found;
        };
        for (index, step) in steps.iter().enumerate() {
            let Some(step) = step.as_object() else {
                continue;
            };
            let Some(kind) = step.get("type").and_then(Value::as_str) else {
                continue;
            };
            if READ_ONLY_RUN_STEPS.contains(&kind) {
                continue;
            }
            push(
                Some(index),
                self.call_consent_targets(&format!("computer_{kind}"), step, thread),
            );
        }
        found
    }

    pub(super) async fn authorize_consent_targets(
        &self,
        name: &str,
        approval_args: &ToolArgs,
        context: &ComputerToolContext,
        targets: Vec<(Option<usize>, ComputerConsentTarget)>,
    ) -> Result<Option<ConsentRefusal>, ComputerError> {
        let caller = self.caller(context);
        if targets.is_empty() {
            let consent = self
                .manager
                .authorize_consent(name, approval_args, &caller, context.signal.clone(), None)
                .await?;
            return Ok((consent != ComputerConsent::Granted).then_some(ConsentRefusal::Task));
        }
        for (step, target) in targets {
            match self
                .manager
                .authorize_consent(
                    name,
                    approval_args,
                    &caller,
                    context.signal.clone(),
                    Some(&target),
                )
                .await?
            {
                ComputerConsent::Granted => {}
                ComputerConsent::Denied(Some(app)) => {
                    return Ok(Some(ConsentRefusal::App { app, step }));
                }
                ComputerConsent::Denied(None) => return Ok(Some(ConsentRefusal::Task)),
            }
        }
        Ok(None)
    }
}
