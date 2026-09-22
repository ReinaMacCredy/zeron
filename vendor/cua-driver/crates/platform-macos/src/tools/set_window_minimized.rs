//! Minimize or unminimize one exact window without activating it.
//!
//! The window-level half of hidden workspaces: a minimized window stays on
//! its Space, keeps a coherent frame and AX surface, and remains fully
//! addressable for background semantic reads and writes (get_window_state,
//! set_value, element clicks, semantic type_text). Unminimizing restores it
//! in place without raising the owning app frontmost.

use async_trait::async_trait;
use core_foundation::base::{CFRelease, CFTypeRef};
use cua_driver_core::{
    action_record::{
        effect_from_value_readback, ActionEvidence, ActionExecutionRecord, ActionTransport,
        ActualDelivery, EvidenceKind, RequestedDelivery,
    },
    protocol::ToolResult,
    tool::{Tool, ToolDef},
};
use serde_json::Value;

pub struct SetWindowMinimizedTool;

static DEF: std::sync::OnceLock<ToolDef> = std::sync::OnceLock::new();

fn def() -> &'static ToolDef {
    DEF.get_or_init(|| ToolDef {
        name: "set_window_minimized".into(),
        description: "Minimize or unminimize one exact window without activating it or \
             switching Spaces. A minimized window keeps its Space membership, AX surface \
             and element addresses, so background semantic actions (set_value, element \
             clicks, semantic type_text, get_window_state) keep working on it — the \
             hidden-workspace lifecycle at window granularity."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["pid", "window_id", "minimized"],
            "properties": {
                "session": { "type": "string", "description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session." },
                "pid": { "type": "integer" },
                "window_id": { "type": "integer" },
                "minimized": {
                    "type": "boolean",
                    "description": "true minimizes the window into the Dock; false restores it."
                }
            },
            "additionalProperties": false
        }),
        read_only: false,
        destructive: false,
        idempotent: true,
        open_world: true,
    })
}

#[derive(Debug)]
struct MinimizeOutcome {
    requested: bool,
    observed: Option<bool>,
}

fn mutate_and_verify(pid: i32, window_id: u32, minimized: bool) -> Result<MinimizeOutcome, String> {
    use crate::{
        ax::bindings::{
            ax_get_window_id, copy_ax_windows, copy_bool_attr, is_attribute_settable,
            kAXErrorSuccess, set_bool_attr, AXUIElementCreateApplication,
            AXUIElementSetMessagingTimeout,
        },
        windows::WindowOwner,
    };

    match crate::windows::resolve_window_owner(pid, window_id) {
        WindowOwner::SamePid => {}
        WindowOwner::ForeignPid {
            owner_pid,
            owner_app_name,
        } => {
            return Err(format!(
                "window_id {window_id} belongs to pid {owner_pid} ({owner_app_name}), not pid {pid}"
            ));
        }
        WindowOwner::Unknown => {
            return Err(format!(
                "window_id {window_id} is closed, stale, or unknown to WindowServer"
            ));
        }
    }

    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return Err(format!(
                "could not create an accessibility element for pid {pid}"
            ));
        }
        AXUIElementSetMessagingTimeout(app, 2.0);
        let windows = copy_ax_windows(app);
        let target = windows
            .iter()
            .copied()
            .find(|window| ax_get_window_id(*window) == Some(window_id));

        let result = if let Some(target) = target {
            AXUIElementSetMessagingTimeout(target, 2.0);
            if !is_attribute_settable(target, "AXMinimized") {
                Err(format!(
                    "window_id {window_id} does not expose a settable AXMinimized"
                ))
            } else {
                let error = set_bool_attr(target, "AXMinimized", minimized);
                if error != kAXErrorSuccess {
                    Err(format!(
                        "AXMinimized write was rejected with AXError {error}"
                    ))
                } else {
                    let mut observed = None;
                    for _ in 0..30 {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        if let Some(state) = copy_bool_attr(target, "AXMinimized") {
                            observed = Some(state);
                            if state == minimized {
                                break;
                            }
                        }
                    }
                    Ok(MinimizeOutcome {
                        requested: minimized,
                        observed,
                    })
                }
            }
        } else {
            Err(format!(
                "window_id {window_id} belongs to pid {pid} in WindowServer but has no matching AXWindow"
            ))
        };

        for window in windows {
            CFRelease(window as CFTypeRef);
        }
        CFRelease(app as CFTypeRef);
        result
    }
}

fn action_record(outcome: &MinimizeOutcome) -> ActionExecutionRecord {
    let confirmed = outcome.observed == Some(outcome.requested);
    let effect = effect_from_value_readback(confirmed, confirmed, outcome.observed.is_some());
    let mut builder = ActionExecutionRecord::builder(
        effect,
        ActionTransport::MacosAxValue,
        RequestedDelivery::NotApplicable,
    )
    .actual_delivery(ActualDelivery::NotApplicable)
    .detail(format!(
        "requested_minimized={} observed={:?}",
        outcome.requested, outcome.observed,
    ));
    if outcome.observed.is_some() {
        builder = builder.evidence(ActionEvidence {
            kind: EvidenceKind::ValueReadback,
            detail: if confirmed {
                "AXMinimized readback matched the request".into()
            } else {
                "AXMinimized readback did not match the request".into()
            },
        });
    }
    builder
        .build()
        .expect("set_window_minimized record is valid")
}

#[async_trait]
impl Tool for SetWindowMinimizedTool {
    fn def(&self) -> &ToolDef {
        def()
    }

    async fn invoke(&self, args: Value) -> ToolResult {
        use cua_driver_core::tool_args::ArgsExt;
        let pid = match args.require_i32("pid") {
            Ok(v) => v,
            Err(e) => return e,
        };
        let window_id = match args
            .get("window_id")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0)
        {
            Some(v) => v,
            None => {
                return ToolResult::error(
                    "set_window_minimized: window_id must be a positive integer",
                )
            }
        };
        let minimized = match args.get("minimized").and_then(Value::as_bool) {
            Some(v) => v,
            None => {
                return ToolResult::error(
                    "set_window_minimized: minimized must be a boolean (true minimizes, false restores)",
                )
            }
        };
        let outcome =
            match tokio::task::spawn_blocking(move || mutate_and_verify(pid, window_id, minimized))
                .await
            {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => {
                    return ToolResult::error(format!("set_window_minimized: {error}"))
                }
                Err(error) => {
                    return ToolResult::error(format!(
                        "set_window_minimized: blocking task failed: {error}"
                    ))
                }
            };
        let confirmed = outcome.observed == Some(outcome.requested);
        ToolResult::text(if confirmed {
            if outcome.requested {
                "Minimized the window; it remains addressable for background semantic actions."
            } else {
                "Restored the window without activating it."
            }
        } else {
            "The minimize mutation was attempted, but the resulting state could not be confirmed."
        })
        .with_action_record(action_record(&outcome))
    }
}
