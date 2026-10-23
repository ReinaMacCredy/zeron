use zeron_proto::tools::{TOOL_GROUPS, ToolGroup, ToolSettings, ToolSettingsState};
use zeron_rpc::methods;

use super::{Page, Row, RowKind, Section};
use crate::client::Client;
use crate::error::{ClientError, Result};

pub(super) async fn state(client: &Client, device_id: &str) -> Result<ToolSettingsState> {
    client
        .host_call(
            device_id,
            methods::GET_TOOL_SETTINGS,
            serde_json::Value::Null,
        )
        .await
}

pub(super) async fn page(client: &Client, device_id: &str) -> Result<Page> {
    Ok(build(&state(client, device_id).await?))
}

pub(super) async fn act(client: &Client, device_id: &str, action: &str, on: bool) -> Result<()> {
    let mut settings = state(client, device_id).await?.settings;
    if !edit(&mut settings, action, on) {
        return Err(ClientError::InvalidArgument(action.to_owned()));
    }
    client
        .inner
        .host_rpc(
            device_id,
            methods::SET_TOOL_SETTINGS,
            serde_json::to_value(&settings).unwrap_or_default(),
        )
        .await?;
    Ok(())
}

fn own_switch(group: ToolGroup) -> Option<&'static str> {
    match group {
        ToolGroup::Computer => Some("Same switch as Settings > Computer on the Mac"),
        ToolGroup::Devices => {
            Some("Same switch as agent access in Settings > Device hub on the Mac")
        }
        _ => None,
    }
}

fn group_key(group: ToolGroup) -> String {
    serde_json::to_value(group)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

pub(super) fn counts(state: &ToolSettingsState) -> (usize, usize) {
    let total: usize = TOOL_GROUPS.iter().map(|info| info.tools().len()).sum();
    (total.saturating_sub(state.disabled.len()), total)
}

fn edit(settings: &mut ToolSettings, action: &str, on: bool) -> bool {
    let switchable = || {
        TOOL_GROUPS
            .iter()
            .filter(|info| own_switch(info.group).is_none())
    };
    match action {
        "all-on" | "all-off" => {
            let on = action == "all-on";
            for info in switchable() {
                if on {
                    for tool in info.tools() {
                        settings.set_tool(tool.name, true);
                    }
                }
                settings.set_group(info.group, on);
            }
        }
        _ => {
            if let Some(key) = action.strip_prefix("group:") {
                let Some(info) = switchable().find(|info| group_key(info.group) == key) else {
                    return false;
                };
                settings.set_group(info.group, on);
            } else if let Some(name) = action.strip_prefix("tool:") {
                settings.set_tool(name, on);
            } else {
                return false;
            }
        }
    }
    true
}

pub(super) fn build(state: &ToolSettingsState) -> Page {
    let (live, total) = counts(state);
    let summary = if live == total {
        format!(
            "Agents on this Mac can call all {total} Zeron tools across {} groups.",
            TOOL_GROUPS.len()
        )
    } else if live > 0 {
        "A call to a tool you turn off is refused right away.".to_owned()
    } else {
        "Agents see no Zeron MCP tools. Turn a group on and calls to it work again.".to_owned()
    };
    let mut sections = vec![Section {
        title: None,
        footer: Some("Applies to every chat on this Mac.".into()),
        rows: vec![
            Row::new(
                "summary",
                format!("{live} of {total} tools on"),
                RowKind::Info,
            )
            .subtitle(summary),
            Row::new(
                "all-on",
                "Turn all on",
                RowKind::Action { destructive: false },
            ),
            Row::new(
                "all-off",
                "Turn all off",
                RowKind::Action { destructive: false },
            ),
        ],
    }];
    for info in TOOL_GROUPS {
        let tools = info.tools();
        let live = tools
            .iter()
            .filter(|tool| !state.disabled.iter().any(|d| d == tool.name))
            .count();
        let count = if live == tools.len() || live == 0 {
            format!("{} tools", tools.len())
        } else {
            format!("{live} of {} tools on", tools.len())
        };
        let group_on = state.settings.group_on(info.group) && live > 0;
        let mut rows = vec![match own_switch(info.group) {
            Some(note) => Row::new(
                format!("group:{}", group_key(info.group)),
                info.title,
                RowKind::Info,
            )
            .value(Some(if live == 0 { "Off" } else { "On" }.to_owned()))
            .subtitle(Some(format!("{count} · {note}"))),
            None => Row::new(
                format!("group:{}", group_key(info.group)),
                info.title,
                RowKind::Toggle { on: group_on },
            )
            .subtitle(Some(format!("{count} · {}", info.summary))),
        }];
        if group_on && own_switch(info.group).is_none() {
            rows.extend(tools.iter().map(|tool| {
                let waits_on = tool
                    .needs
                    .filter(|need| state.disabled.iter().any(|d| d == need));
                let subtitle = match waits_on {
                    Some(need) => Some(format!("Off while {need} is off")),
                    None => (!tool.summary.is_empty()).then(|| tool.summary.to_owned()),
                };
                Row::new(
                    format!("tool:{}", tool.name),
                    tool.name,
                    RowKind::Toggle {
                        on: state.settings.tool_on(tool.name),
                    },
                )
                .subtitle(subtitle)
            }));
        }
        sections.push(Section {
            title: Some(info.title.to_owned()),
            footer: None,
            rows,
        });
    }
    Page {
        title: "Tools".into(),
        sections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_a_group_off_keeps_its_own_tool_choices() {
        let mut settings = ToolSettings::default();
        assert!(edit(&mut settings, "tool:delegate_task", false));
        assert!(edit(&mut settings, "group:delegation", false));
        assert!(!settings.group_on(ToolGroup::Delegation));
        assert!(!settings.tool_on("delegate_task"));
        assert!(edit(&mut settings, "all-on", true));
        assert!(settings.group_on(ToolGroup::Delegation));
        assert!(settings.tool_on("delegate_task"));
        assert!(!edit(&mut settings, "group:computer", true));
    }

    #[test]
    fn tools_page_counts_what_the_engine_reports_off() {
        let state = ToolSettings::default().state(&[ToolGroup::Computer]);
        let page = build(&state);
        let (live, total) = counts(&state);
        assert!(live < total);
        assert_eq!(
            page.sections[0].rows[0].title,
            format!("{live} of {total} tools on")
        );
        let computer = page
            .sections
            .iter()
            .find(|s| s.title.as_deref() == Some("Computer"))
            .unwrap();
        assert_eq!(computer.rows.len(), 1);
        assert_eq!(computer.rows[0].kind, RowKind::Info);
    }
}
