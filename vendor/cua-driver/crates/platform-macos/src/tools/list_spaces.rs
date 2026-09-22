use async_trait::async_trait;
use cua_driver_core::{
    protocol::ToolResult,
    tool::{Tool, ToolDef},
};
use serde_json::{json, Value};

pub struct ListSpacesTool;

#[async_trait]
impl Tool for ListSpacesTool {
    fn def(&self) -> &ToolDef {
        static DEF: std::sync::OnceLock<ToolDef> = std::sync::OnceLock::new();
        DEF.get_or_init(|| ToolDef {
            name: "list_spaces".into(),
            description: "Read the managed Spaces for each macOS display, including Spaces without windows. Stable UUID, session numeric ID, display and current status are observations only. This does not create, move, switch or own a Space. Incomplete or changing identity is reported as unknown.".into(),
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            read_only: true, destructive: false, idempotent: true, open_world: false,
        })
    }

    async fn invoke(&self, _args: Value) -> ToolResult {
        match crate::spaces::managed_spaces() {
            Ok(inventory) => ToolResult::text(format!(
                "Observed {} managed Space(s).",
                inventory.spaces.len()
            ))
            .with_structured(json!(inventory)),
            Err(message) => ToolResult::error(message).with_structured(json!({
                "status":"refused", "code":"computer_spaces_unavailable", "message":message
            })),
        }
    }
}
