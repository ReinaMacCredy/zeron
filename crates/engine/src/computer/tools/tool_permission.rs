use serde_json::Value;
use zeron_proto::PermissionMode;

pub use zeron_proto::computer::ZERON_COMPUTER_TOOL_NAMES;

const MCP_PREFIX: &str = "mcp__zeron__";
const TOOL_PREFIX: &str = "zeron_";

pub fn canonical_zeron_computer_tool_name(value: &str) -> Option<&'static str> {
    let normalized = value.trim().to_lowercase();
    let canonical = normalized
        .strip_prefix(MCP_PREFIX)
        .or_else(|| normalized.strip_prefix(TOOL_PREFIX))
        .unwrap_or(&normalized);
    ZERON_COMPUTER_TOOL_NAMES
        .iter()
        .find(|name| **name == canonical)
        .copied()
}

pub fn qualified_zeron_computer_tool_name(value: &str) -> Option<&'static str> {
    let normalized = value.trim().to_lowercase();
    if !normalized.starts_with(MCP_PREFIX) && !normalized.starts_with(TOOL_PREFIX) {
        return None;
    }
    canonical_zeron_computer_tool_name(&normalized)
}

pub fn is_zeron_computer_tool_family_name(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|name| canonical_zeron_computer_tool_name(name).is_some())
}

#[derive(Debug, Clone, Default)]
pub struct ProviderToolPermission<'a> {
    pub name: Option<&'a Value>,
    pub title: Option<&'a Value>,
    pub raw_input: Option<&'a Value>,
    pub metadata: Option<&'a Value>,
}

fn first_record_string<'a>(value: Option<&'a Value>, keys: &[&str]) -> Option<&'a str> {
    let record = value?.as_object()?;
    keys.iter()
        .find_map(|key| record.get(*key).and_then(Value::as_str))
}

const TOOL_NAME_KEYS: &[&str] = &["_toolName", "toolName", "tool_name"];

pub fn computer_tool_name_from_provider_permission(
    input: &ProviderToolPermission<'_>,
) -> Option<&'static str> {
    if let Some(name) = input.name.and_then(Value::as_str) {
        return qualified_zeron_computer_tool_name(name);
    }
    if let Some(name) = first_record_string(input.raw_input, TOOL_NAME_KEYS) {
        return qualified_zeron_computer_tool_name(name);
    }
    if let Some(name) = first_record_string(input.metadata, TOOL_NAME_KEYS) {
        return qualified_zeron_computer_tool_name(name);
    }
    input
        .title
        .and_then(Value::as_str)
        .and_then(qualified_zeron_computer_tool_name)
}

pub struct ProviderToolAllowInput<'a> {
    pub computer_control_enabled: bool,
    pub active_turn: bool,
    pub permission_mode: PermissionMode,
    pub permission: ProviderToolPermission<'a>,
}

pub fn should_allow_zeron_computer_provider_tool(input: &ProviderToolAllowInput<'_>) -> bool {
    input.computer_control_enabled
        && input.active_turn
        && input.permission_mode == PermissionMode::Ask
        && computer_tool_name_from_provider_permission(&input.permission).is_some()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn named(name: &Value) -> ProviderToolPermission<'_> {
        ProviderToolPermission {
            name: Some(name),
            ..ProviderToolPermission::default()
        }
    }

    #[test]
    fn recognizes_the_exact_owned_tools() {
        for (provider, canonical) in [
            ("computer_click", "computer_click"),
            ("zeron_computer_type_text", "computer_type_text"),
            ("zeron_computer_select_text", "computer_select_text"),
            (
                "mcp__zeron__computer_read_clipboard",
                "computer_read_clipboard",
            ),
            ("mcp__zeron__computer_inspect", "computer_inspect"),
        ] {
            assert_eq!(
                canonical_zeron_computer_tool_name(provider),
                Some(canonical),
                "{provider}"
            );
        }
    }

    #[test]
    fn does_not_trust_another_or_unknown_tool() {
        for provider in [
            "computer_future_tool",
            "mcp__other__computer_click",
            "other_computer_click",
            "mcp__zeron__zeron_send_message",
        ] {
            assert_eq!(
                canonical_zeron_computer_tool_name(provider),
                None,
                "{provider}"
            );
        }
    }

    #[test]
    fn requires_a_provider_namespace_when_server_provenance_was_not_proved_separately() {
        assert_eq!(qualified_zeron_computer_tool_name("computer_click"), None);
        assert_eq!(
            qualified_zeron_computer_tool_name("mcp__zeron__computer_click"),
            Some("computer_click")
        );
        assert_eq!(
            qualified_zeron_computer_tool_name("zeron_computer_click"),
            Some("computer_click")
        );
    }

    #[test]
    fn reads_only_explicit_provider_tool_name_fields() {
        let raw = json!({ "_toolName": "mcp__zeron__computer_scroll" });
        assert_eq!(
            computer_tool_name_from_provider_permission(&ProviderToolPermission {
                raw_input: Some(&raw),
                ..ProviderToolPermission::default()
            }),
            Some("computer_scroll")
        );
        let metadata = json!({ "toolName": "zeron_computer_get_state" });
        assert_eq!(
            computer_tool_name_from_provider_permission(&ProviderToolPermission {
                metadata: Some(&metadata),
                ..ProviderToolPermission::default()
            }),
            Some("computer_get_state")
        );
        let prose = json!({ "description": "run computer_click" });
        assert_eq!(
            computer_tool_name_from_provider_permission(&ProviderToolPermission {
                metadata: Some(&prose),
                ..ProviderToolPermission::default()
            }),
            None
        );
    }

    #[test]
    fn does_not_let_lower_priority_fields_override_an_authoritative_wrong_namespace() {
        let name = json!("mcp__other__computer_click");
        let raw = json!({ "_toolName": "mcp__zeron__computer_click" });
        assert_eq!(
            computer_tool_name_from_provider_permission(&ProviderToolPermission {
                name: Some(&name),
                raw_input: Some(&raw),
                ..ProviderToolPermission::default()
            }),
            None
        );
        let raw = json!({ "_toolName": "mcp__other__computer_click" });
        let metadata = json!({ "toolName": "mcp__zeron__computer_click" });
        let title = json!("mcp__zeron__computer_click");
        assert_eq!(
            computer_tool_name_from_provider_permission(&ProviderToolPermission {
                raw_input: Some(&raw),
                metadata: Some(&metadata),
                title: Some(&title),
                ..ProviderToolPermission::default()
            }),
            None
        );
    }

    #[test]
    fn does_not_infer_provider_provenance_from_a_bare_title_or_metadata_name() {
        let title = json!("computer_click");
        assert_eq!(
            computer_tool_name_from_provider_permission(&ProviderToolPermission {
                title: Some(&title),
                ..ProviderToolPermission::default()
            }),
            None
        );
        let metadata = json!({ "toolName": "computer_click" });
        assert_eq!(
            computer_tool_name_from_provider_permission(&ProviderToolPermission {
                metadata: Some(&metadata),
                ..ProviderToolPermission::default()
            }),
            None
        );
    }

    #[test]
    fn never_authorizes_from_model_prose() {
        let prose = json!("Please approve computer_click");
        let metadata = json!({ "toolName": "Please approve computer_click" });
        assert_eq!(
            computer_tool_name_from_provider_permission(&ProviderToolPermission {
                title: Some(&prose),
                ..ProviderToolPermission::default()
            }),
            None
        );
        assert_eq!(
            computer_tool_name_from_provider_permission(&named(&prose)),
            None
        );
        assert_eq!(
            computer_tool_name_from_provider_permission(&ProviderToolPermission {
                metadata: Some(&metadata),
                ..ProviderToolPermission::default()
            }),
            None
        );
        assert!(!is_zeron_computer_tool_family_name(&prose));
        assert!(!should_allow_zeron_computer_provider_tool(
            &ProviderToolAllowInput {
                computer_control_enabled: true,
                active_turn: true,
                permission_mode: PermissionMode::Ask,
                permission: ProviderToolPermission {
                    title: Some(&prose),
                    ..ProviderToolPermission::default()
                },
            }
        ));
    }

    #[test]
    fn matches_the_computer_family_in_any_namespace_spelling_for_the_denial_hook() {
        for name in [
            "computer_click",
            "zeron_computer_get_state",
            "mcp__zeron__computer_screenshot",
            "  MCP__ZERON__COMPUTER_WAIT  ",
        ] {
            assert!(is_zeron_computer_tool_family_name(&json!(name)), "{name}");
        }
    }

    #[test]
    fn keeps_unknown_and_foreign_names_out_of_the_computer_family() {
        for value in [
            json!("computer_future_tool"),
            json!("mcp__other__computer_click"),
            json!("other_computer_click"),
            json!("zeron_frobnicate"),
            Value::Null,
            json!(42),
        ] {
            assert!(!is_zeron_computer_tool_family_name(&value), "{value}");
        }
    }

    #[test]
    fn requires_current_capability_active_turn_and_ask_mode() {
        let name = json!("mcp__zeron__computer_click");
        let allowed = |enabled: bool, active: bool, mode: PermissionMode| {
            should_allow_zeron_computer_provider_tool(&ProviderToolAllowInput {
                computer_control_enabled: enabled,
                active_turn: active,
                permission_mode: mode,
                permission: named(&name),
            })
        };
        assert!(allowed(true, true, PermissionMode::Ask));
        assert!(!allowed(false, true, PermissionMode::Ask));
        assert!(!allowed(true, false, PermissionMode::Ask));
        assert!(!allowed(true, true, PermissionMode::Auto));
        assert!(!allowed(true, true, PermissionMode::Full));
    }

    #[test]
    fn owns_every_served_name_exactly_once() {
        let mut names = ZERON_COMPUTER_TOOL_NAMES.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 43);
    }
}
