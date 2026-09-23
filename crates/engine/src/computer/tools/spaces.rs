use async_trait::async_trait;
use futures::FutureExt;
use serde_json::{Map, Value, json};
use zeron_proto::computer::{
    ComputerSpaceErrorCode, ComputerSpaceReservation, McpToolDefinition, McpToolResult,
};

use super::desktop::json_of;
use super::results::{ToolFailure, ToolValue};
use super::tool_input::{ToolArgs, ToolInputError};
use super::{ComputerToolContext, ComputerToolEntry, ComputerToolFamily, ComputerTools};
use crate::computer::desktop_operation_queue::assert_desktop_operation_active;
use crate::computer::manager::ComputerStateRequest;
use crate::computer::model_desktop_observation::with_model_desktop_observation;
use crate::computer::space_broker::ComputerSpaceOwner;
use crate::computer::ui_tree_targeting::ComputerTargetError;

const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

fn space_id(value: Option<&Value>, required: bool) -> Result<Option<u64>, ToolInputError> {
    let Some(value) = value else {
        if !required {
            return Ok(None);
        }
        return Err(invalid_space_id());
    };
    match value.as_f64() {
        Some(id) if id.fract() == 0.0 && id > 0.0 && id <= MAX_SAFE_INTEGER => Ok(Some(id as u64)),
        _ => Err(invalid_space_id()),
    }
}

fn invalid_space_id() -> ToolInputError {
    ToolInputError::new(
        "\"space_id\" must be a positive safe integer from the current Space inventory.",
    )
}

fn public_reservation(value: Option<ComputerSpaceReservation>) -> Value {
    let Some(reservation) = value else {
        return Value::Null;
    };
    let mut selection: Map<String, Value> = match json_of(&reservation) {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    selection.remove("threadId");
    selection.remove("turnId");
    Value::Object(selection)
}

pub struct ComputerSpaceTools {
    entry: ComputerToolEntry,
}

impl Default for ComputerSpaceTools {
    fn default() -> Self {
        Self {
            entry: ComputerToolEntry {
                definition: McpToolDefinition {
                    name: "computer_spaces".into(),
                    description: "Observe managed macOS Spaces, including empty Spaces, or reserve an existing user-designated noncurrent Space for this task. Selection returns an exact window_id to drive in place; it never activates or moves anything. No native Space create/move/switch/follow is supported. Read computer_help topic spaces for designation and limits.".into(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "operation": {
                                "type": "string",
                                "description": "list (default), reserve, release, select, or peek. create, move, switch and follow return an explicit unsupported-operation refusal.",
                            },
                            "space_id": {
                                "type": "number",
                                "description": "Exact current native Space ID; required for reserve, optional filter for list/peek. Do not use Mission Control position numbers.",
                            },
                            "window_id": {
                                "type": "string",
                                "description": "Exact existing window in the reserved Space; required for select, optional with reserve or peek. Selection changes only the task's logical target.",
                            },
                        },
                        "additionalProperties": false,
                    }),
                    annotations: Some(json!({
                        "title": "Manage the task's desktop Space",
                        "readOnlyHint": false,
                        "destructiveHint": false,
                        "idempotentHint": true,
                        "openWorldHint": false,
                    })),
                },
                discovery_only: true,
            },
        }
    }
}

async fn spaces(
    tools: &ComputerTools,
    args: &ToolArgs,
    context: &ComputerToolContext,
) -> Result<ToolValue, ToolFailure> {
    if let Some(key) = args
        .keys()
        .find(|key| !matches!(key.as_str(), "operation" | "space_id" | "window_id"))
    {
        return Err(ToolInputError::new(format!("Unknown argument \"{key}\".")).into());
    }
    let operation = match args.get("operation") {
        None | Some(Value::Null) => "list",
        Some(Value::String(operation)) => operation.as_str(),
        Some(_) => return Err(ToolInputError::new("\"operation\" must be a string.").into()),
    };
    if matches!(operation, "create" | "move" | "switch" | "follow") {
        return Err(ComputerTargetError::space(
            ComputerSpaceErrorCode::ComputerSpaceOperationUnsupported,
            "This backend cannot create or switch managed Spaces, move windows between them, or follow a window by switching the user's desktop. Use an existing exact window in place. Nothing was moved or activated.",
        )
        .into());
    }
    if !matches!(
        operation,
        "list" | "reserve" | "release" | "select" | "peek"
    ) {
        return Err(ToolInputError::new("Unknown Space operation.").into());
    }
    let id = space_id(args.get("space_id"), operation == "reserve")?;
    let window_id = match args.get("window_id") {
        None => None,
        Some(Value::String(id)) if !id.trim().is_empty() && id.encode_utf16().count() <= 256 => {
            Some(id.as_str())
        }
        Some(_) => {
            return Err(
                ToolInputError::new("\"window_id\" must name an exact observed window.").into(),
            );
        }
    };
    if operation == "select" && window_id.is_none() {
        return Err(ToolInputError::new("\"select\" requires \"window_id\".").into());
    }
    let manager = tools.manager();
    let broker = manager.space_broker();
    let owner = ComputerSpaceOwner {
        thread_id: context.chat_id.clone(),
        turn_id: context.turn_id.clone(),
    };
    if operation == "release" {
        if id.is_some() || window_id.is_some() {
            return Err(ToolInputError::new(
                "Release acts only on this task's reservation and accepts no target.",
            )
            .into());
        }
        broker.release(&owner.thread_id, owner.turn_id.as_deref());
        return Ok(
            json!({ "operation": operation, "reservation": null, "changedDesktop": false }).into(),
        );
    }
    if let (Some(id), "reserve") = (id, operation) {
        let designation = context.seams.space_designation().await;
        tools.assert_turn_active(context)?;
        assert_desktop_operation_active()?;
        let recheck = async move {
            let fresh = context.seams.space_designation().await;
            tools.assert_turn_active(context).is_ok() && fresh.contains(&id)
        }
        .boxed();
        let reservation = broker
            .reserve(&owner, id, &designation, window_id, Some(recheck))
            .await?;
        return Ok(json!({
            "operation": operation,
            "reservation": public_reservation(Some(reservation)),
            "changedDesktop": false,
        })
        .into());
    }
    if let (Some(window_id), "select") = (window_id, operation) {
        if id.is_some() {
            return Err(
                ToolInputError::new("Select uses this task's reservation; omit space_id.").into(),
            );
        }
        let reservation = broker.select(&owner, window_id).await?;
        return Ok(json!({
            "operation": operation,
            "reservation": public_reservation(Some(reservation)),
            "changedDesktop": false,
        })
        .into());
    }
    let snapshot = broker.inspect(id).await?;
    if operation == "list" && window_id.is_some() {
        return Err(ToolInputError::new("Use peek to inspect one exact window.").into());
    }
    let target = match window_id {
        None => None,
        Some(window_id) => Some(
            snapshot
                .windows
                .iter()
                .find(|window| window.id == window_id)
                .ok_or_else(|| {
                    ComputerTargetError::space(
                        ComputerSpaceErrorCode::ComputerSpaceTargetOutsideReservation,
                        "That window is absent from this fresh Space inventory. List Spaces and windows again.",
                    )
                })?,
        ),
    };
    let mut payload = Map::new();
    payload.insert("operation".into(), json!(operation));
    payload.insert("inventory".into(), json_of(&snapshot.inventory));
    payload.insert("windows".into(), json_of(&snapshot.windows));
    payload.insert(
        "reservation".into(),
        public_reservation(broker.reservation_for(&owner)),
    );
    if let Some(target) = target {
        let state = with_model_desktop_observation(manager.get_state(ComputerStateRequest {
            window_id: Some(target.id.clone()),
            include_screenshot: Some(false),
            include_text: true,
            include_tree: None,
        }))
        .await?;
        payload.insert("state".into(), json_of(&state));
    }
    payload.insert("changedDesktop".into(), json!(false));
    Ok(Value::Object(payload).into())
}

#[async_trait]
impl ComputerToolFamily for ComputerSpaceTools {
    fn entries(&self) -> Vec<ComputerToolEntry> {
        vec![self.entry.clone()]
    }

    async fn call(
        &self,
        tools: &ComputerTools,
        name: &str,
        args: ToolArgs,
        context: &ComputerToolContext,
    ) -> McpToolResult {
        tools
            .handle(name, &args, context, || spaces(tools, &args, context))
            .await
    }
}
