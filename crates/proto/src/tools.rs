use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::computer::ZERON_COMPUTER_TOOL_NAMES;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolGroup {
    Threads,
    Workspace,
    UiDriver,
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
            tool(
                "watch_threads",
                "Be woken whenever listed threads block on a question or a tool approval.",
            ),
            tool("set_thread_title", "Rename a thread on this device."),
            tool(
                "set_thread_archived",
                "Archive or unarchive one or several threads.",
            ),
            tool(
                "set_thread_settled",
                "Settle or unsettle threads in the sidebar's Inbox.",
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
        title: "Questions, terminal, actions and tasks",
        summary: "Reach the person watching, the terminal panel, the project Actions menu, the Tasks list and Zeron's Docs.",
        tools: &[
            tool(
                "ask_user",
                "Ask the person watching this chat one to four questions and wait for answers.",
            ),
            tool(
                "request_secret",
                "Ask the person for a secret in a private card; the agent gets a private file, never the value.",
            ),
            tool(
                "terminal",
                "Use this chat's terminals in the terminal panel, live.",
            ),
            tool(
                "project_actions",
                "Manage the saved commands in the project's Actions menu.",
            ),
            tool(
                "tasks",
                "Write, update and assign tasks on the person's Tasks list, and plan an orchestrated task's steps.",
            ),
            tool(
                "context_settings",
                "Read this chat's context settings and set its auto-compact window.",
            ),
            tool(
                "compact",
                "Compact this chat's own context once its turn ends, then optionally continue.",
            ),
            tool(
                "docs",
                "Read Zeron's own Docs and release notes to answer how Zeron works.",
            ),
        ],
    },
    ToolGroupInfo {
        group: ToolGroup::UiDriver,
        title: "UI driver",
        summary: "Read a running Zeron app's UI and drive a Zeron Dev build.",
        tools: &[
            tool("ui_driver", "Inspect a running Zeron app's UI."),
            needs(
                "ui_driver_drive",
                "Click, type and change a Zeron Dev build's UI, never the app serving the chat.",
                "ui_driver",
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
                "orchestrator_capabilities",
                "Harnesses, their models and availability, from the same catalog as the composer.",
            ),
            tool(
                "delegate_task",
                "Delegate one task to a Zeron-owned child agent of this chat.",
            ),
            tool(
                "review_changes",
                "Have a fresh reviewer agent review this chat's changes, as the Review button does.",
            ),
            tool(
                "resolve_findings",
                "Report what this chat did about each finding of a review.",
            ),
            tool(
                "agent_profiles",
                "Create, change or remove the agent profiles in the model picker when asked.",
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
        title: "Pages and visuals",
        summary: "Show live HTML pages and native visuals (charts, tables, mockups) inline in the thread.",
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
            tool(
                "show_visual",
                "Show a native interactive visual (chart, table, 3D view) inline in this thread.",
            ),
            tool(
                "show_image",
                "Show an image file (screenshot, photo) inline in this thread.",
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

    pub fn for_chat(&self, chat_disabled: &[String]) -> ToolSettings {
        let mut settings = self.clone();
        for name in chat_disabled
            .iter()
            .filter_map(|name| name.strip_prefix(ZERON_TOOL_PREFIX))
        {
            for label in action_labels(name).unwrap_or(&[]).iter().chain([&name]) {
                settings.set_tool(label, false);
            }
        }
        settings
    }

    pub fn chat_tool_off(&self, chat_disabled: &[String], tool: &str) -> bool {
        self.for_chat(chat_disabled).disabled(&[]).contains(tool)
    }
}

pub const ZERON_TOOL_PREFIX: &str = "mcp__zeron__";

/// Tools that fold several former tools into actions keep a switch per action, under the former names.
pub const ACTION_TOOLS: [(&str, &[&str]); 2] = [
    (
        "device",
        &[
            "device_list",
            "device_open",
            "device_screenshot",
            "device_close",
        ],
    ),
    (
        "set_thread",
        &[
            "set_thread_title",
            "set_thread_archived",
            "set_thread_settled",
        ],
    ),
];

pub fn action_labels(tool: &str) -> Option<&'static [&'static str]> {
    ACTION_TOOLS
        .iter()
        .find(|(name, _)| *name == tool)
        .map(|(_, labels)| *labels)
}

pub fn folding_tool(label: &str) -> Option<&'static str> {
    ACTION_TOOLS
        .iter()
        .find(|(_, labels)| labels.contains(&label))
        .map(|(name, _)| *name)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReceivedCount {
    pub tools: usize,
    pub live_tools: usize,
    pub actions: usize,
    pub live_actions: usize,
}

impl ReceivedCount {
    pub fn all_live(&self) -> bool {
        self.live_tools == self.tools && self.live_actions == self.actions
    }
}

impl ToolGroupInfo {
    pub fn received(&self, disabled: &BTreeSet<&str>) -> ReceivedCount {
        let tools = match self.group {
            // The engine advertises the single JS `computer` tool on macOS, the full catalog elsewhere.
            ToolGroup::Computer if cfg!(target_os = "macos") => vec![tool("computer", "")],
            _ => self.tools(),
        };
        let mut count = ReceivedCount::default();
        let mut folds: Vec<(&str, bool)> = Vec::new();
        for tool in tools {
            let live = !disabled.contains(tool.name);
            match folding_tool(tool.name) {
                Some(name) => {
                    count.actions += 1;
                    count.live_actions += usize::from(live);
                    match folds.iter_mut().find(|(folded, _)| *folded == name) {
                        Some((_, any)) => *any |= live,
                        None => folds.push((name, live)),
                    }
                }
                None => {
                    count.tools += 1;
                    count.live_tools += usize::from(live);
                }
            }
        }
        count.tools += folds.len();
        count.live_tools += folds.iter().filter(|(_, live)| *live).count();
        count
    }
}

pub fn action_label(tool: &str, args: &serde_json::Value) -> Option<&'static str> {
    let wanted = match tool {
        "device" => format!("device_{}", args["action"].as_str()?),
        "set_thread" => format!(
            "set_thread_{}",
            ["title", "archived", "settled"]
                .into_iter()
                .find(|field| !args[*field].is_null())?
        ),
        _ => return None,
    };
    action_labels(tool)?
        .iter()
        .copied()
        .find(|label| *label == wanted)
}

pub fn mcp_server_rule(server: &str) -> String {
    let name: String = server
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("mcp__{name}")
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BuiltinToolGroup {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct McpServerEntry {
    pub name: String,
    pub rule: String,
    pub status: String,
    pub source: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HarnessToolInventory {
    pub builtin: Vec<BuiltinToolGroup>,
    pub mcp_servers: Vec<McpServerEntry>,
}

pub fn group_live(group: ToolGroup, disabled: &BTreeSet<&str>) -> bool {
    tool_group(group)
        .tools()
        .iter()
        .any(|tool| !disabled.contains(tool.name))
}

pub fn zeron_rule(tool: &str) -> String {
    format!("{ZERON_TOOL_PREFIX}{tool}")
}

pub fn set_rules(disabled: &mut Vec<String>, rules: &[String], on: bool) {
    disabled.retain(|rule| !rules.contains(rule));
    if !on {
        disabled.extend(rules.iter().cloned());
    }
}

pub fn rules_on(disabled: &[String], rules: &[String]) -> usize {
    rules.iter().filter(|rule| !disabled.contains(rule)).count()
}

pub fn zeron_unlocked(info: &ToolGroupInfo, ceiling: &[String]) -> Vec<String> {
    info.tools()
        .iter()
        .filter(|tool| !ceiling.iter().any(|off| off == tool.name))
        .map(|tool| zeron_rule(tool.name))
        .collect()
}

pub fn set_zeron_group(
    disabled: &mut Vec<String>,
    info: &ToolGroupInfo,
    ceiling: &[String],
    on: bool,
) {
    if on {
        let all: Vec<String> = info.tools().iter().map(|t| zeron_rule(t.name)).collect();
        set_rules(disabled, &all, true);
    } else {
        set_rules(disabled, &zeron_unlocked(info, ceiling), false);
    }
}

pub fn unlisted_rules(disabled: &[String], inventory: &HarnessToolInventory) -> Vec<String> {
    disabled
        .iter()
        .filter(|rule| match rule.strip_prefix(ZERON_TOOL_PREFIX) {
            Some(name) => {
                action_labels(name).is_none()
                    && !TOOL_GROUPS
                        .iter()
                        .any(|info| info.tools().iter().any(|tool| tool.name == name))
            }
            None => {
                !inventory.builtin.iter().any(|g| g.tools.contains(rule))
                    && !inventory.mcp_servers.iter().any(|s| &s.rule == *rule)
            }
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn a_zeron_group_switch_writes_only_unlocked_tools() {
        let pages = tool_group(ToolGroup::Pages);
        let ceiling = rules(&["html_preview"]);
        let mut disabled = rules(&["WebFetch"]);
        set_zeron_group(&mut disabled, pages, &ceiling, false);
        assert_eq!(
            disabled,
            rules(&[
                "WebFetch",
                "mcp__zeron__html_render",
                "mcp__zeron__show_visual",
                "mcp__zeron__show_image"
            ])
        );
        assert_eq!(rules_on(&disabled, &zeron_unlocked(pages, &ceiling)), 0);

        disabled.push("mcp__zeron__html_preview".into());
        set_zeron_group(&mut disabled, pages, &ceiling, true);
        assert_eq!(disabled, rules(&["WebFetch"]));

        set_rules(&mut disabled, &[zeron_rule("html_render")], false);
        set_rules(&mut disabled, &[zeron_rule("html_render")], false);
        assert_eq!(disabled, rules(&["WebFetch", "mcp__zeron__html_render"]));
    }

    #[test]
    fn built_in_groups_and_servers_write_their_rules() {
        let web = BuiltinToolGroup {
            id: "web".into(),
            title: "Web".into(),
            summary: String::new(),
            tools: rules(&["WebSearch", "WebFetch"]),
        };
        let xcode = McpServerEntry {
            name: "xcode".into(),
            rule: mcp_server_rule("xcode"),
            status: "connected".into(),
            source: "user".into(),
        };
        let mut disabled = Vec::new();
        set_rules(&mut disabled, &web.tools, false);
        set_rules(&mut disabled, std::slice::from_ref(&xcode.rule), false);
        assert_eq!(disabled, rules(&["WebSearch", "WebFetch", "mcp__xcode"]));
        assert_eq!(rules_on(&disabled, &web.tools), 0);

        disabled.push("mcp__gone".into());
        disabled.push("mcp__zeron__retired_tool".into());
        disabled.push("mcp__zeron__terminal".into());
        disabled.push("mcp__zeron__device".into());
        let inventory = HarnessToolInventory {
            builtin: vec![web.clone()],
            mcp_servers: vec![xcode],
        };
        assert_eq!(
            unlisted_rules(&disabled, &inventory),
            rules(&["mcp__gone", "mcp__zeron__retired_tool"])
        );

        set_rules(&mut disabled, &web.tools, true);
        assert_eq!(
            disabled,
            rules(&[
                "mcp__xcode",
                "mcp__gone",
                "mcp__zeron__retired_tool",
                "mcp__zeron__terminal",
                "mcp__zeron__device"
            ])
        );
    }

    #[test]
    fn everything_is_on_by_default() {
        assert!(ToolSettings::default().disabled(&[]).is_empty());
    }

    #[test]
    fn a_group_off_keeps_the_tool_choices() {
        let mut settings = ToolSettings::default();
        settings.set_tool("interrupt_thread", false);
        settings.set_group(ToolGroup::Threads, false);
        assert_eq!(settings.disabled(&[]).len(), 11);
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
    fn a_chat_turns_off_its_zeron_entries_only() {
        let chat = vec![
            "mcp__zeron__delegate_task".to_owned(),
            "WebFetch".to_owned(),
            "mcp__xcode".to_owned(),
        ];
        assert_eq!(
            ToolSettings::default().for_chat(&chat).disabled(&[]),
            BTreeSet::from(["delegate_task", "task_cancel", "task_status"])
        );
    }

    #[test]
    fn server_rules_match_the_cli_spelling() {
        assert_eq!(mcp_server_rule("codex-cu"), "mcp__codex-cu");
        assert_eq!(
            mcp_server_rule("claude.ai Claude Docs"),
            "mcp__claude_ai_Claude_Docs"
        );
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
    fn received_counts_fold_actions_into_their_tool() {
        let count = |group, disabled: &BTreeSet<&str>| tool_group(group).received(disabled);
        let none = BTreeSet::new();
        let threads = count(ToolGroup::Threads, &none);
        assert_eq!((threads.tools, threads.actions), (9, 3));
        let devices = count(ToolGroup::Devices, &none);
        assert_eq!((devices.tools, devices.actions), (1, 4));
        let computer = count(ToolGroup::Computer, &none).tools;
        if cfg!(target_os = "macos") {
            assert_eq!(computer, 1);
        } else {
            assert_eq!(computer, ZERON_COMPUTER_TOOL_NAMES.len());
        }

        let mut settings = ToolSettings::default();
        settings.set_tool("device_close", false);
        let devices = count(ToolGroup::Devices, &settings.disabled(&[]));
        assert_eq!((devices.live_tools, devices.live_actions), (1, 3));
        assert!(!devices.all_live());
        settings.set_tool("device_list", false);
        assert_eq!(
            count(ToolGroup::Devices, &settings.disabled(&[])).live_tools,
            0
        );
    }

    #[test]
    fn an_action_tool_answers_to_its_actions_switches() {
        let call = |tool: &str, args: serde_json::Value| action_label(tool, &args);
        assert_eq!(
            call("device", serde_json::json!({ "action": "close" })),
            Some("device_close")
        );
        assert_eq!(
            call("device", serde_json::json!({ "action": "boot" })),
            None
        );
        assert_eq!(
            call(
                "set_thread",
                serde_json::json!({ "threadIds": ["a"], "settled": true })
            ),
            Some("set_thread_settled")
        );
        assert_eq!(
            call("terminal", serde_json::json!({ "action": "list" })),
            None
        );
        for (_, labels) in ACTION_TOOLS {
            for label in labels {
                assert!(
                    TOOL_GROUPS
                        .iter()
                        .any(|group| group.tools().iter().any(|tool| tool.name == *label)),
                    "{label} keeps its Settings > Tools switch"
                );
            }
        }
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
