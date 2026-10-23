use zeron_proto::tools::{
    HarnessToolInventory, TOOL_GROUPS, ToolGroup, ToolSettingsState, rules_on, set_rules,
    set_zeron_group, tool_group, unlisted_rules, zeron_rule, zeron_unlocked,
};
use zeron_proto::{AgentProfile, HarnessId};
use zeron_rpc::methods;

use super::{Page, Row, RowKind, Section};
use crate::client::Client;
use crate::error::{ClientError, Result};

async fn ceiling(client: &Client, device_id: &str) -> std::result::Result<Vec<String>, String> {
    client
        .host_call::<ToolSettingsState>(
            device_id,
            methods::GET_TOOL_SETTINGS,
            serde_json::json!({}),
        )
        .await
        .map(|state| state.disabled)
        .map_err(|e| e.to_string())
}

async fn inventory(
    client: &Client,
    device_id: &str,
    harness: HarnessId,
) -> Option<std::result::Result<HarnessToolInventory, String>> {
    (harness == HarnessId::ClaudeCode).then_some(())?;
    Some(
        client
            .host_call(
                device_id,
                methods::LIST_HARNESS_TOOLS,
                serde_json::json!({ "harness": HarnessId::ClaudeCode }),
            )
            .await
            .map_err(|e| e.to_string()),
    )
}

fn wire(group: ToolGroup) -> String {
    serde_json::to_value(group)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn group_of(field: &str) -> Option<ToolGroup> {
    let group = field.strip_prefix("tools:")?;
    serde_json::from_value(serde_json::Value::String(group.to_owned())).ok()
}

fn toggle(id: String, title: impl Into<String>, on: bool, subtitle: &str) -> Row {
    Row::new(id, title, RowKind::Toggle { on })
        .subtitle((!subtitle.is_empty()).then(|| subtitle.to_owned()))
}

pub(super) async fn page(
    client: &Client,
    device_id: &str,
    profile: &AgentProfile,
    field: &str,
) -> Result<Page> {
    let ceiling = ceiling(client, device_id).await;
    if let Some(group) = group_of(field) {
        return Ok(group_page(profile, group, &ceiling.unwrap_or_default()));
    }
    let inventory = inventory(client, device_id, profile.harness).await;
    Ok(tools_page(profile, ceiling, inventory))
}

fn tools_page(
    profile: &AgentProfile,
    ceiling: std::result::Result<Vec<String>, String>,
    inventory: Option<std::result::Result<HarnessToolInventory, String>>,
) -> Page {
    let disabled = &profile.disabled_tools;
    let (ceiling, footer) = match ceiling {
        Ok(ceiling) => (ceiling, None),
        Err(error) => (
            Vec::new(),
            Some(format!("Settings > Tools unavailable: {error}")),
        ),
    };
    let zeron = TOOL_GROUPS
        .iter()
        .map(|info| {
            let unlocked = zeron_unlocked(info, &ceiling);
            let live = rules_on(disabled, &unlocked);
            let value = if unlocked.is_empty() {
                "off in Settings".to_owned()
            } else if live == 0 {
                "Off".to_owned()
            } else if live < unlocked.len() {
                format!("{live} of {}", unlocked.len())
            } else {
                "On".to_owned()
            };
            Row::new(
                format!("group:{}", wire(info.group)),
                info.title,
                RowKind::Link {
                    page: format!("profile:{}:tools:{}", profile.id, wire(info.group)),
                },
            )
            .value(Some(value))
        })
        .collect();
    let mut sections = vec![Section {
        title: Some("Zeron tools".into()),
        footer,
        rows: zeron,
    }];
    match &inventory {
        Some(Ok(inventory)) => {
            sections.push(Section {
                title: Some("Claude built-in".into()),
                footer: None,
                rows: inventory
                    .builtin
                    .iter()
                    .map(|group| {
                        toggle(
                            format!("builtin:{}", group.id),
                            &group.title,
                            rules_on(disabled, &group.tools) == group.tools.len(),
                            &group.summary,
                        )
                    })
                    .collect(),
            });
            sections.push(Section {
                title: Some("MCP servers".into()),
                footer: inventory
                    .mcp_servers
                    .is_empty()
                    .then(|| "No MCP servers in your Claude config.".into()),
                rows: inventory
                    .mcp_servers
                    .iter()
                    .map(|server| {
                        toggle(
                            format!("server:{}", server.rule),
                            &server.name,
                            !disabled.contains(&server.rule),
                            &server.status,
                        )
                    })
                    .collect(),
            });
        }
        Some(Err(error)) => sections.push(Section {
            title: Some("Claude built-in".into()),
            footer: Some(format!("Could not list Claude's tools: {error}")),
            rows: Vec::new(),
        }),
        None => {}
    }
    let known = match inventory {
        Some(Ok(inventory)) => inventory,
        _ => HarnessToolInventory::default(),
    };
    let unlisted: Vec<Row> = unlisted_rules(disabled, &known)
        .into_iter()
        .map(|rule| {
            toggle(
                format!("unlisted:{rule}"),
                rule,
                false,
                "not found · turn on to remove",
            )
        })
        .collect();
    if !unlisted.is_empty() {
        sections.push(Section {
            title: Some("Not found".into()),
            footer: None,
            rows: unlisted,
        });
    }
    Page {
        title: "Tools".into(),
        sections,
    }
}

fn group_page(profile: &AgentProfile, group: ToolGroup, ceiling: &[String]) -> Page {
    let info = tool_group(group);
    let disabled = &profile.disabled_tools;
    let unlocked = zeron_unlocked(info, ceiling);
    let all = if unlocked.is_empty() {
        Row::new("group", format!("All {}", info.title), RowKind::Info)
            .value(Some("off in Settings".into()))
    } else {
        toggle(
            "group".into(),
            format!("All {}", info.title),
            rules_on(disabled, &unlocked) > 0,
            "",
        )
    };
    let tools = info
        .tools()
        .into_iter()
        .map(|tool| {
            if ceiling.iter().any(|off| off == tool.name) {
                Row::new(format!("tool:{}", tool.name), tool.name, RowKind::Info)
                    .value(Some("off in Settings".into()))
            } else {
                toggle(
                    format!("tool:{}", tool.name),
                    tool.name,
                    !disabled.contains(&zeron_rule(tool.name)),
                    tool.summary,
                )
            }
        })
        .collect();
    Page {
        title: info.title.into(),
        sections: vec![
            Section {
                title: None,
                footer: Some(info.summary.into()),
                rows: vec![all],
            },
            Section {
                title: None,
                footer: None,
                rows: tools,
            },
        ],
    }
}

pub(super) async fn act(
    client: &Client,
    device_id: &str,
    disabled: &mut Vec<String>,
    field: &str,
    action: &str,
    on: bool,
) -> Result<()> {
    if action == "group" {
        let group = group_of(field).ok_or_else(|| ClientError::InvalidArgument(field.into()))?;
        let ceiling = ceiling(client, device_id).await.unwrap_or_default();
        set_zeron_group(disabled, tool_group(group), &ceiling, on);
    } else if let Some(name) = action.strip_prefix("tool:") {
        set_rules(disabled, &[zeron_rule(name)], on);
    } else if let Some(rule) = action.strip_prefix("server:") {
        set_rules(disabled, &[rule.to_owned()], on);
    } else if let Some(rule) = action.strip_prefix("unlisted:") {
        set_rules(disabled, &[rule.to_owned()], true);
    } else if let Some(id) = action.strip_prefix("builtin:") {
        let Some(Ok(inventory)) = inventory(client, device_id, HarnessId::ClaudeCode).await else {
            return Err(ClientError::NotFound(id.to_owned()));
        };
        let group = inventory
            .builtin
            .iter()
            .find(|g| g.id == id)
            .ok_or_else(|| ClientError::NotFound(id.to_owned()))?;
        set_rules(disabled, &group.tools, on);
    } else {
        return Err(ClientError::InvalidArgument(action.to_owned()));
    }
    Ok(())
}
