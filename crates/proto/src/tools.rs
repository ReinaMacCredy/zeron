use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::computer::ZERON_COMPUTER_TOOL_NAMES;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolGroup {
    Threads,
    Workspace,
    Delegation,
    Pages,
    Devices,
    Computer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolInfo {
    pub name: &'static str,
    pub summary: &'static str,
    pub needs: Option<&'static str>,
}

#[derive(Debug)]
pub struct ToolGroupInfo {
    pub group: ToolGroup,
    pub title: &'static str,
    pub summary: &'static str,
    tools: &'static [ToolInfo],
}

const fn tool(name: &'static str, summary: &'static str) -> ToolInfo {
    ToolInfo {
        name,
        summary,
        needs: None,
    }
}

const fn needs(name: &'static str, summary: &'static str, needs: &'static str) -> ToolInfo {
    ToolInfo {
        name,
        summary,
        needs: Some(needs),
    }
}

pub const TOOL_GROUPS: &[ToolGroupInfo] = &[
    ToolGroupInfo {
        group: ToolGroup::Threads,
        title: "Threads",
        summary: "Read, write and steer the chats in the sidebar.",
        tools: &[
            tool(
                "list_threads",
                "List the chats in the sidebar, every project included, with their live status.",
            ),
            tool("read_thread", "Read the tail of a thread's transcript."),
            tool(
                "create_thread",
                "Open a new peer thread and give it a first message.",
            ),
            tool(
                "send_message_to_thread",
                "Send a message to another thread on this device.",
            ),
            tool(
                "wait_threads",
                "Wait until listed threads stop working or an inbox message arrives.",
            ),
            tool("set_thread_title", "Rename a thread on this device."),
            tool(
                "set_thread_archived",
                "Archive or unarchive one or several threads.",
            ),
            tool(
                "interrupt_thread",
                "Stop another thread's running turn, as its stop button does.",
            ),
            tool(
                "respond_to_input",
                "Answer the question or permission request another thread is blocked on.",
            ),
        ],
    },
    ToolGroupInfo {
        group: ToolGroup::Workspace,
        title: "Questions, terminal and actions",
        summary: "Reach the person watching, the terminal panel and the project Actions menu.",
        tools: &[
            tool(
                "ask_user",
                "Ask the person watching this chat one to four questions and wait for answers.",
            ),
            tool(
                "terminal",
                "Use this chat's terminals in the terminal panel, live.",
            ),
            tool(
                "project_actions",
                "Manage the saved commands in the project's Actions menu.",
            ),
        ],
    },
    ToolGroupInfo {
        group: ToolGroup::Delegation,
        title: "Delegation",
        summary: "Run child agents on any installed harness and model.",
        tools: &[
            tool("list_projects", "List the projects on this device."),
            tool(
                "list_harnesses",
                "List the agent harnesses on this device and whether each is available.",
            ),
            tool(
                "list_models",
                "List the models a harness offers, with reasoning levels.",
            ),
            tool(
                "orchestrator_capabilities",
                "Harnesses and current models from the same catalog as the composer.",
            ),
            tool(
                "delegate_task",
                "Delegate one task to a Zeron-owned child agent of this chat.",
            ),
            needs(
                "task_status",
                "Read a delegated task created by this chat.",
                "delegate_task",
            ),
            needs(
                "task_cancel",
                "Request interruption of an active delegated task.",
                "delegate_task",
            ),
        ],
    },
    ToolGroupInfo {
        group: ToolGroup::Pages,
        title: "HTML pages",
        summary: "Show live HTML pages (charts, tables, mockups) inline in the thread.",
        tools: &[
            needs(
                "html_preview",
                "Screenshot a page in Zeron's web engine to check it before showing it.",
                "html_render",
            ),
            tool(
                "html_render",
                "Show a finished HTML page inline in this thread, above the reply.",
            ),
        ],
    },
    ToolGroupInfo {
        group: ToolGroup::Devices,
        title: "Device hub",
        summary: "iOS Simulators and Android Emulators in the Device panel.",
        tools: &[
            tool(
                "device_list",
                "List simulators and emulators and which are already open.",
            ),
            needs(
                "device_open",
                "Boot a device and show it in the user's Device panel.",
                "device_list",
            ),
            needs(
                "device_screenshot",
                "Capture the screen of an open device as a PNG.",
                "device_list",
            ),
            needs(
                "device_close",
                "Remove a device from this thread's Device panel.",
                "device_list",
            ),
        ],
    },
    ToolGroupInfo {
        group: ToolGroup::Computer,
        title: "Computer",
        summary: "Desktop and browser control on this Mac.",
        tools: &[],
    },
];

impl ToolGroupInfo {
    pub fn tools(&self) -> Vec<ToolInfo> {
        match self.group {
            ToolGroup::Computer => ZERON_COMPUTER_TOOL_NAMES
                .iter()
                .map(|name| tool(name, ""))
                .collect(),
            _ => self.tools.to_vec(),
        }
    }
}

pub fn tool_group(group: ToolGroup) -> &'static ToolGroupInfo {
    TOOL_GROUPS
        .iter()
        .find(|info| info.group == group)
        .expect("every ToolGroup has an entry")
}

/// Computer and Devices keep their own switches (`computerControlEnabled`,
/// Device hub agent access) and reach `disabled` through `also_off`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ToolSettings {
    pub disabled_tools: Vec<String>,
    pub disabled_groups: Vec<ToolGroup>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ToolSettingsState {
    pub settings: ToolSettings,
    pub disabled: Vec<String>,
}

impl ToolSettings {
    pub fn tool_on(&self, name: &str) -> bool {
        !self.disabled_tools.iter().any(|tool| tool == name)
    }

    pub fn group_on(&self, group: ToolGroup) -> bool {
        !self.disabled_groups.contains(&group)
    }

    pub fn set_tool(&mut self, name: &str, on: bool) {
        self.disabled_tools.retain(|tool| tool != name);
        if !on {
            self.disabled_tools.push(name.to_owned());
        }
    }

    pub fn set_group(&mut self, group: ToolGroup, on: bool) {
        self.disabled_groups.retain(|g| *g != group);
        if !on {
            self.disabled_groups.push(group);
        }
    }

    pub fn disabled(&self, also_off: &[ToolGroup]) -> BTreeSet<&'static str> {
        let mut off = BTreeSet::new();
        let mut dependents = Vec::new();
        for info in TOOL_GROUPS {
            let group_off = !self.group_on(info.group) || also_off.contains(&info.group);
            for tool in info.tools() {
                if group_off || !self.tool_on(tool.name) {
                    off.insert(tool.name);
                } else if let Some(need) = tool.needs {
                    dependents.push((tool.name, need));
                }
            }
        }
        for (name, need) in dependents {
            if off.contains(need) {
                off.insert(name);
            }
        }
        off
    }

    pub fn state(&self, also_off: &[ToolGroup]) -> ToolSettingsState {
        ToolSettingsState {
            settings: self.clone(),
            disabled: self
                .disabled(also_off)
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }
}

pub fn group_live(group: ToolGroup, disabled: &BTreeSet<&str>) -> bool {
    tool_group(group)
        .tools()
        .iter()
        .any(|tool| !disabled.contains(tool.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_is_on_by_default() {
        assert!(ToolSettings::default().disabled(&[]).is_empty());
    }

    #[test]
    fn a_group_off_keeps_the_tool_choices() {
        let mut settings = ToolSettings::default();
        settings.set_tool("interrupt_thread", false);
        settings.set_group(ToolGroup::Threads, false);
        assert_eq!(settings.disabled(&[]).len(), 9);
        settings.set_group(ToolGroup::Threads, true);
        assert_eq!(settings.disabled(&[]), BTreeSet::from(["interrupt_thread"]));
    }

    #[test]
    fn a_dependent_tool_follows_its_need() {
        let mut settings = ToolSettings::default();
        settings.set_tool("delegate_task", false);
        assert_eq!(
            settings.disabled(&[]),
            BTreeSet::from(["delegate_task", "task_cancel", "task_status"])
        );
        assert!(group_live(ToolGroup::Delegation, &settings.disabled(&[])));
    }

    #[test]
    fn also_off_covers_groups_switched_elsewhere() {
        let disabled = ToolSettings::default().disabled(&[ToolGroup::Devices, ToolGroup::Computer]);
        assert!(!group_live(ToolGroup::Devices, &disabled));
        assert!(!group_live(ToolGroup::Computer, &disabled));
        assert!(group_live(ToolGroup::Threads, &disabled));
        assert_eq!(disabled.len(), 4 + ZERON_COMPUTER_TOOL_NAMES.len());
    }

    #[test]
    fn unknown_names_are_ignored() {
        let mut settings = ToolSettings::default();
        settings.set_tool("retired_tool", false);
        assert!(settings.disabled(&[]).is_empty());
    }

    #[test]
    fn wire_shape() {
        let mut settings = ToolSettings::default();
        settings.set_tool("terminal", false);
        settings.set_group(ToolGroup::Delegation, false);
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            serde_json::json!({ "disabledTools": ["terminal"], "disabledGroups": ["delegation"] })
        );
        assert_eq!(
            serde_json::from_value::<ToolSettings>(serde_json::json!({})).unwrap(),
            ToolSettings::default()
        );
    }
}
