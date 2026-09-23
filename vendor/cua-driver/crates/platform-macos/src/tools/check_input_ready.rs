use async_trait::async_trait;
use cua_driver_core::{
    protocol::ToolResult,
    tool::{Tool, ToolDef},
};
use serde_json::Value;

pub struct CheckInputReadyTool;
static DEF: std::sync::OnceLock<ToolDef> = std::sync::OnceLock::new();
#[async_trait]
impl Tool for CheckInputReadyTool {
    fn def(&self) -> &ToolDef {
        DEF.get_or_init(|| ToolDef {
            name: "check_input_ready".into(),
            description: "Read-only exact-window input readiness. Checks owner, active Space, visibility, modal-sheet focus and optional observed bounds. Never captures, activates, moves or sends input.".into(),
            input_schema: serde_json::json!({"type":"object","required":["pid","window_id"],"properties":{
                "pid":{"type":"integer","minimum":1},"window_id":{"type":"integer","minimum":1},
                "expected_window_bounds":{"type":"object","required":["x","y","width","height"],"properties":{
                    "x":{"type":"number"},"y":{"type":"number"},"width":{"type":"number","exclusiveMinimum":0},"height":{"type":"number","exclusiveMinimum":0}},"additionalProperties":false}},"additionalProperties":false}),
            read_only: true, destructive: false, idempotent: true, open_world: false,
        })
    }
    async fn invoke(&self, args: Value) -> ToolResult {
        match tokio::task::spawn_blocking(move || crate::input::target_guard::readiness(&args))
            .await
        {
            Ok(result) => result,
            Err(error) => {
                ToolResult::error(format!("Input readiness could not be checked: {error}"))
            }
        }
    }
}
