use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const COMPUTER_BROWSER_TOOL_NAMES: &[&str] = &[
    "computer_browser_state",
    "computer_browser_prepare",
    "computer_browser_navigate",
    "computer_browser_click",
    "computer_browser_type",
    "computer_browser_dialog",
    "computer_browser_upload",
    "computer_browser_download",
    "computer_browser_pointer",
    "computer_browser_press",
];

pub const COMPUTER_BROWSER_REFUSAL_CODES: &[&str] = &[
    "browser_route_unavailable",
    "browser_requires_setup",
    "browser_binding_ambiguous",
    "browser_binding_stale",
    "browser_wrong_target_refused",
    "browser_tab_required",
    "browser_tab_not_found",
    "browser_ref_stale",
    "browser_input_trust_unavailable",
    "browser_endpoint_owner_mismatch",
    "browser_consent_required",
    "browser_consent_revoked",
    "browser_reconnect_exhausted",
    "browser_input_incomplete",
    "browser_action_unavailable",
    "browser_origin_outside_scope",
];

pub const COMPUTER_BROWSER_DRIVER_NAMES: &[(&str, &str)] = &[
    ("computer_browser_state", "get_browser_state"),
    ("computer_browser_prepare", "browser_prepare"),
    ("computer_browser_navigate", "browser_navigate"),
    ("computer_browser_click", "browser_click"),
    ("computer_browser_type", "browser_type"),
    ("computer_browser_dialog", "browser_dialog"),
    ("computer_browser_upload", "browser_set_input_files"),
    ("computer_browser_download", "browser_download"),
    ("computer_browser_pointer", "browser_pointer"),
    ("computer_browser_press", "browser_type"),
];

pub fn computer_browser_driver_name(tool: &str) -> Option<&'static str> {
    COMPUTER_BROWSER_DRIVER_NAMES
        .iter()
        .find(|(name, _)| *name == tool)
        .map(|(_, driver)| *driver)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputerBrowserRefusal {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerBrowserRefusedStatus {
    Refused,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputerBrowserRefusedContent {
    pub status: ComputerBrowserRefusedStatus,
    pub refusal: ComputerBrowserRefusal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerBrowserTabInfo {
    pub tab_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerBrowserState {
    pub target_id: String,
    pub tabs: Vec<ComputerBrowserTabInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer::CUA_BROWSER_TOOLS;
    use serde_json::json;

    #[test]
    fn every_gateway_name_maps_to_exactly_one_admitted_driver_tool() {
        assert_eq!(
            COMPUTER_BROWSER_DRIVER_NAMES.len(),
            COMPUTER_BROWSER_TOOL_NAMES.len()
        );
        for tool in COMPUTER_BROWSER_TOOL_NAMES {
            let driver = computer_browser_driver_name(tool).unwrap();
            assert!(CUA_BROWSER_TOOLS.contains(&driver), "{tool} -> {driver}");
        }
    }

    #[test]
    fn refused_content_requires_the_refused_status() {
        let wire = json!({
            "status": "refused",
            "refusal": { "code": "browser_tab_required", "message": "Pick a tab." },
        });
        let content: ComputerBrowserRefusedContent = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(content).unwrap(), wire);
        let mut completed = wire;
        completed["status"] = json!("completed");
        assert!(serde_json::from_value::<ComputerBrowserRefusedContent>(completed).is_err());
    }

    #[test]
    fn browser_state_keeps_unknown_engine_fields_out_of_the_way() {
        let state: ComputerBrowserState = serde_json::from_value(json!({
            "target_id": "t1",
            "tabs": [{ "tab_id": "a", "url": "https://example.com" }],
            "engine": "cdp",
        }))
        .unwrap();
        assert_eq!(state.tabs[0].tab_id, "a");
    }
}
