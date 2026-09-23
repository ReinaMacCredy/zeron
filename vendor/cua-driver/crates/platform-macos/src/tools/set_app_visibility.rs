//! Hide or unhide a running application without activating it.
//!
//! This is the workspace-lifecycle half of hidden workspaces: an app hidden
//! this way keeps its windows fully addressable for background semantic AX
//! reads and writes while rendering nothing on screen. Unhiding restores the
//! windows in place; neither direction raises the app frontmost.

use async_trait::async_trait;
use cua_driver_core::{
    action_record::{
        effect_from_value_readback, ActionEvidence, ActionExecutionRecord, ActionTransport,
        ActualDelivery, EvidenceKind, RequestedDelivery,
    },
    protocol::ToolResult,
    tool::{Tool, ToolDef},
};
use serde_json::Value;

pub struct SetAppVisibilityTool;

static DEF: std::sync::OnceLock<ToolDef> = std::sync::OnceLock::new();

fn def() -> &'static ToolDef {
    DEF.get_or_init(|| ToolDef {
        name: "set_app_visibility".into(),
        description: "Hide or unhide a running application without activating it. \
             A hidden app keeps its windows, stays on its Space, and remains fully \
             addressable for background accessibility reads and semantic writes \
             (set_value, element clicks, semantic type_text) — the hidden-workspace \
             lifecycle. Unhiding restores the windows where they were; neither \
             direction brings the app to the foreground."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["pid", "hidden"],
            "properties": {
                "session": { "type": "string", "description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session." },
                "pid": { "type": "integer" },
                "hidden": {
                    "type": "boolean",
                    "description": "true hides every window of the application (Cmd+H equivalent); false unhides."
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
struct VisibilityOutcome {
    requested_hidden: bool,
    observed: Option<bool>,
}

fn mutate_and_verify(pid: i32, hidden: bool) -> Result<VisibilityOutcome, String> {
    crate::apps::set_app_hidden(pid, hidden).map(|observed| VisibilityOutcome {
        requested_hidden: hidden,
        observed,
    })
}

fn action_record(outcome: &VisibilityOutcome) -> ActionExecutionRecord {
    let confirmed = outcome.observed == Some(outcome.requested_hidden);
    let changed = outcome.observed == Some(outcome.requested_hidden);
    let effect = effect_from_value_readback(confirmed, changed, outcome.observed.is_some());
    let mut builder = ActionExecutionRecord::builder(
        effect,
        ActionTransport::MacosAxValue,
        RequestedDelivery::NotApplicable,
    )
    .actual_delivery(ActualDelivery::NotApplicable)
    .detail(format!(
        "requested_hidden={} observed={:?}",
        outcome.requested_hidden, outcome.observed,
    ));
    if outcome.observed.is_some() {
        builder = builder.evidence(ActionEvidence {
            kind: EvidenceKind::ValueReadback,
            detail: if confirmed {
                "NSRunningApplication.isHidden matched the request".into()
            } else {
                "NSRunningApplication.isHidden did not match the request".into()
            },
        });
    }
    builder.build().expect("set_app_visibility record is valid")
}

#[async_trait]
impl Tool for SetAppVisibilityTool {
    fn def(&self) -> &ToolDef {
        def()
    }

    async fn invoke(&self, args: Value) -> ToolResult {
        use cua_driver_core::tool_args::ArgsExt;
        let pid = match args.require_i32("pid") {
            Ok(v) => v,
            Err(e) => return e,
        };
        if pid <= 0 {
            return ToolResult::error("set_app_visibility: pid must be a positive integer");
        }
        let hidden = match args.get("hidden").and_then(Value::as_bool) {
            Some(v) => v,
            None => {
                return ToolResult::error(
                    "set_app_visibility: hidden must be a boolean (true hides, false unhides)",
                )
            }
        };
        let outcome =
            match tokio::task::spawn_blocking(move || mutate_and_verify(pid, hidden)).await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => return ToolResult::error(format!("set_app_visibility: {error}")),
                Err(error) => {
                    return ToolResult::error(format!(
                        "set_app_visibility: blocking task failed: {error}"
                    ))
                }
            };
        let confirmed = outcome.observed == Some(outcome.requested_hidden);
        ToolResult::text(if confirmed {
            if outcome.requested_hidden {
                "Hid the application; its windows remain addressable for background semantic actions."
            } else {
                "Unhid the application."
            }
        } else {
            "The visibility mutation was attempted, but the resulting state could not be confirmed."
        })
        .with_action_record(action_record(&outcome))
    }
}
