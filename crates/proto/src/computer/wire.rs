use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::driver::{CuaComputerTask, CuaReply};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerHostMethod {
    Call,
    Probe,
    Setup,
    Stop,
    EndTask,
    Shield,
    EndBrowserThread,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerDeliveryMode {
    Background,
    Foreground,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerHostCall {
    pub correlation_id: String,
    pub method: ComputerHostMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub args: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<CuaComputerTask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_mode: Option<ComputerDeliveryMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_observation: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchComputerHostRequestsParams {
    pub bundle_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerHostRequest {
    pub host_token: String,
    #[serde(flatten)]
    pub call: ComputerHostCall,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ComputerHostStreamItem {
    #[serde(rename_all = "camelCase")]
    Registered {
        host_token: String,
    },
    #[serde(rename_all = "camelCase")]
    Refused {
        retry_after_ms: u64,
    },
    Request(ComputerHostRequest),
    #[serde(rename_all = "camelCase")]
    Cancel {
        host_token: String,
        correlation_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerStillFrame {
    pub sequence: u64,
    pub timestamp_ms: f64,
    pub keyframe: bool,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RespondComputerHostRequestParams {
    pub host_token: String,
    pub correlation_id: String,
    pub result: CuaReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpRequestId {
    Number(i64),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerActionParams {
    pub chat_id: String,
    pub run_secret: String,
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    pub request_id: McpRequestId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelComputerActionParams {
    pub chat_id: String,
    pub run_secret: String,
    pub request_id: McpRequestId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetComputerCatalogParams {
    pub chat_id: String,
    pub run_secret: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpContent {
    Text {
        text: String,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_error: bool,
}

impl McpToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![McpContent::Text { text: text.into() }],
            ..Self::default()
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            is_error: true,
            ..Self::text(text)
        }
    }

    pub fn first_text(&self) -> Option<&str> {
        self.content.iter().find_map(|part| match part {
            McpContent::Text { text } => Some(text.as_str()),
            McpContent::Image { .. } => None,
        })
    }

    pub fn has_image(&self) -> bool {
        self.content
            .iter()
            .any(|part| matches!(part, McpContent::Image { .. }))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ComputerCatalog {
    pub tools: Vec<McpToolDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetComputerStillFrameParams {
    pub chat_id: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerPreviewSize {
    #[default]
    Compact,
    Large,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentCursorColorMode {
    #[default]
    Stock,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ComputerSettings {
    pub computer_control_enabled: bool,
    pub auto_open_computer_pane: bool,
    pub computer_preview_size: ComputerPreviewSize,
    pub agent_cursor_color_mode: AgentCursorColorMode,
    pub agent_cursor_fill_color: String,
    pub agent_cursor_rim_color: String,
}

impl Default for ComputerSettings {
    fn default() -> Self {
        Self {
            computer_control_enabled: false,
            auto_open_computer_pane: true,
            computer_preview_size: ComputerPreviewSize::Compact,
            agent_cursor_color_mode: AgentCursorColorMode::Stock,
            agent_cursor_fill_color: String::new(),
            agent_cursor_rim_color: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCursorColors {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rim: Option<String>,
}

pub fn normalize_cursor_hex_color(value: &str) -> String {
    let candidate = value.trim().to_ascii_lowercase();
    let valid = candidate.len() == 7
        && candidate.starts_with('#')
        && candidate[1..]
            .chars()
            .all(|c| matches!(c, '0'..='9' | 'a'..='f'));
    if valid { candidate } else { String::new() }
}

impl ComputerSettings {
    pub fn agent_cursor_colors(&self) -> Option<AgentCursorColors> {
        if self.agent_cursor_color_mode != AgentCursorColorMode::Custom {
            return None;
        }
        let channel =
            |value: &str| Some(normalize_cursor_hex_color(value)).filter(|c| !c.is_empty());
        let colors = AgentCursorColors {
            fill: channel(&self.agent_cursor_fill_color),
            rim: channel(&self.agent_cursor_rim_color),
        };
        (colors != AgentCursorColors::default()).then_some(colors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn host_request_flattens_the_call_beside_the_token() {
        let wire = json!({
            "hostToken": "tok",
            "correlationId": "c1",
            "method": "call",
            "name": "click",
            "args": { "pid": 7, "window_id": 9 },
            "task": { "threadId": "chat-1", "turnId": "turn-1" },
            "deliveryMode": "background",
            "modelObservation": false,
        });
        let request: ComputerHostRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(request.call.method, ComputerHostMethod::Call);
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        let stop =
            json!({ "hostToken": "tok", "correlationId": "c2", "method": "end_browser_thread" });
        let request: ComputerHostRequest = serde_json::from_value(stop.clone()).unwrap();
        assert_eq!(request.call.method, ComputerHostMethod::EndBrowserThread);
        assert_eq!(serde_json::to_value(&request).unwrap(), stop);
    }

    #[test]
    fn host_stream_items_are_tagged_by_kind() {
        let cases = [
            json!({ "kind": "registered", "hostToken": "tok" }),
            json!({ "kind": "refused", "retryAfterMs": 2000 }),
            json!({
                "kind": "request",
                "hostToken": "tok",
                "correlationId": "c1",
                "method": "call",
                "name": "click",
                "args": { "pid": 7 },
            }),
            json!({ "kind": "cancel", "hostToken": "tok", "correlationId": "c1" }),
        ];
        for wire in cases {
            let item: ComputerHostStreamItem = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(serde_json::to_value(&item).unwrap(), wire);
        }
        let request: ComputerHostStreamItem = serde_json::from_value(json!({
            "kind": "request",
            "hostToken": "tok",
            "correlationId": "c2",
            "method": "probe",
        }))
        .unwrap();
        let ComputerHostStreamItem::Request(request) = request else {
            panic!("expected a request item");
        };
        assert_eq!(request.host_token, "tok");
        assert_eq!(request.call.method, ComputerHostMethod::Probe);
    }

    #[test]
    fn host_reply_carries_a_full_cua_reply() {
        let wire = json!({
            "hostToken": "tok",
            "correlationId": "c1",
            "result": { "ok": false, "error": "host_lost", "effect": "not-dispatched" },
        });
        let reply: RespondComputerHostRequestParams = serde_json::from_value(wire.clone()).unwrap();
        assert!(!reply.result.ok);
        assert_eq!(serde_json::to_value(reply).unwrap(), wire);
    }

    #[test]
    fn mcp_request_ids_keep_their_json_rpc_type() {
        for id in [json!(7), json!("req-7")] {
            let params = json!({ "chatId": "c", "runSecret": "s", "requestId": id });
            let cancel: CancelComputerActionParams =
                serde_json::from_value(params.clone()).unwrap();
            assert_eq!(serde_json::to_value(cancel).unwrap(), params);
        }
        assert_ne!(McpRequestId::Number(7), McpRequestId::String("7".into()));
    }

    #[test]
    fn settings_default_like_synara_and_fill_missing_fields() {
        let settings: ComputerSettings =
            serde_json::from_value(json!({ "computerControlEnabled": true })).unwrap();
        assert_eq!(
            settings,
            ComputerSettings {
                computer_control_enabled: true,
                ..ComputerSettings::default()
            }
        );
        assert_eq!(
            serde_json::to_value(ComputerSettings::default()).unwrap(),
            json!({
                "computerControlEnabled": false,
                "autoOpenComputerPane": true,
                "computerPreviewSize": "compact",
                "agentCursorColorMode": "stock",
                "agentCursorFillColor": "",
                "agentCursorRimColor": "",
            })
        );
    }

    #[test]
    fn cursor_colors_resolve_only_in_custom_mode_with_valid_channels() {
        let custom = ComputerSettings {
            agent_cursor_color_mode: AgentCursorColorMode::Custom,
            agent_cursor_fill_color: " #A1B2C3 ".into(),
            agent_cursor_rim_color: "red".into(),
            ..ComputerSettings::default()
        };
        assert_eq!(
            custom.agent_cursor_colors(),
            Some(AgentCursorColors {
                fill: Some("#a1b2c3".into()),
                rim: None,
            })
        );
        let stock = ComputerSettings {
            agent_cursor_color_mode: AgentCursorColorMode::Stock,
            ..custom.clone()
        };
        assert_eq!(stock.agent_cursor_colors(), None);
        let unusable = ComputerSettings {
            agent_cursor_fill_color: "#12345".into(),
            ..custom
        };
        assert_eq!(unusable.agent_cursor_colors(), None);
    }

    #[test]
    fn tool_results_carry_mcp_content_blocks() {
        let result = McpToolResult {
            content: vec![
                McpContent::Text {
                    text: "{\"ok\":true}".into(),
                },
                McpContent::Image {
                    data: "iVBOR".into(),
                    mime_type: "image/png".into(),
                },
            ],
            structured_content: None,
            is_error: true,
        };
        let wire = json!({
            "content": [
                { "type": "text", "text": "{\"ok\":true}" },
                { "type": "image", "data": "iVBOR", "mimeType": "image/png" },
            ],
            "isError": true,
        });
        assert_eq!(serde_json::to_value(&result).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<McpToolResult>(wire).unwrap(),
            result
        );
        let plain = McpToolResult {
            structured_content: Some(json!({ "ok": true })),
            ..McpToolResult::text("{\"ok\":true}")
        };
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            json!({
                "content": [{ "type": "text", "text": "{\"ok\":true}" }],
                "structuredContent": { "ok": true },
            })
        );
        let definition = McpToolDefinition {
            name: "computer_click".into(),
            description: "Click".into(),
            input_schema: json!({ "type": "object" }),
            annotations: Some(json!({ "title": "Click" })),
        };
        assert_eq!(
            serde_json::to_value(ComputerCatalog {
                tools: vec![definition]
            })
            .unwrap(),
            json!({ "tools": [{
                "name": "computer_click",
                "description": "Click",
                "inputSchema": { "type": "object" },
                "annotations": { "title": "Click" },
            }] })
        );
    }
}
