use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Settings,
    Global,
    SettingsThenGlobal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Enum(&'static [&'static str]),
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Zeron,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Model,
    Effort,
    Permission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Plain,
    SwitchModels,
    RemoteControl,
}

#[derive(Clone, Copy, Debug)]
pub struct Setting {
    pub id: &'static str,
    pub label: &'static str,
    pub cli: Option<&'static str>,
    pub store: Store,
    pub path: &'static str,
    pub default: &'static str,
    pub kind: Kind,
    pub scope: Scope,
    pub chip: Option<Chip>,
    pub codec: Codec,
}

const BOOL: Kind = Kind::Bool;

const fn row(
    id: &'static str,
    label: &'static str,
    cli: bool,
    store: Store,
    path: &'static str,
    default: &'static str,
    kind: Kind,
    scope: Scope,
) -> Setting {
    Setting {
        id,
        label,
        cli: if cli { Some(id) } else { None },
        store,
        path,
        default,
        kind,
        scope,
        chip: None,
        codec: Codec::Plain,
    }
}

const fn chip(id: &'static str, label: &'static str, chip: Chip) -> Setting {
    Setting {
        id,
        label,
        cli: None,
        store: Store::Settings,
        path: "",
        default: "",
        kind: Kind::Text,
        scope: Scope::Zeron,
        chip: Some(chip),
        codec: Codec::Plain,
    }
}

const fn coded(mut setting: Setting, codec: Codec) -> Setting {
    setting.codec = codec;
    setting
}

use Scope::{Terminal as T, Zeron as Z};
use Store::{Global as G, Settings as S, SettingsThenGlobal as SG};

pub const SWITCH_AUTOMATICALLY: &str = "Switch automatically";
pub const ASK_EACH_TIME: &str = "Ask each time";

// Ids, labels, storage and defaults read from the Claude Code 2.1.310 binary's /config panel.
pub const SETTINGS: &[Setting] = &[
    row(
        "autoCompact",
        "Auto-compact",
        true,
        SG,
        "autoCompactEnabled",
        "true",
        BOOL,
        Z,
    ),
    row(
        "idleCompaction",
        "Idle compaction",
        false,
        S,
        "idleCompaction",
        "true",
        BOOL,
        Z,
    ),
    row(
        "autoContinueAtUsageLimit",
        "Continue automatically at usage limit",
        false,
        S,
        "autoContinueAtUsageLimit",
        "true",
        BOOL,
        Z,
    ),
    coded(
        row(
            "switchModelsOnFlag",
            "Switch models when a message is flagged",
            true,
            S,
            "switchModelsOnFlag",
            SWITCH_AUTOMATICALLY,
            Kind::Enum(&[SWITCH_AUTOMATICALLY, ASK_EACH_TIME]),
            Z,
        ),
        Codec::SwitchModels,
    ),
    row(
        "tips",
        "Show tips",
        true,
        S,
        "spinnerTipsEnabled",
        "true",
        BOOL,
        T,
    ),
    row(
        "feedbackDrafts",
        "Claude-drafted feedback",
        false,
        S,
        "feedbackDrafts",
        "notify",
        Kind::Enum(&["notify", "quiet", "off"]),
        T,
    ),
    row(
        "reduceMotion",
        "Reduce motion",
        true,
        S,
        "prefersReducedMotion",
        "false",
        BOOL,
        T,
    ),
    row(
        "thinking",
        "Thinking mode",
        true,
        S,
        "alwaysThinkingEnabled",
        "true",
        BOOL,
        Z,
    ),
    row("fast", "Fast mode", false, S, "fastMode", "false", BOOL, Z),
    row(
        "promptSuggestionEnabled",
        "Prompt suggestions",
        true,
        S,
        "promptSuggestionEnabled",
        "true",
        BOOL,
        T,
    ),
    row(
        "recap",
        "Session recap",
        true,
        S,
        "awaySummaryEnabled",
        "true",
        BOOL,
        T,
    ),
    row(
        "checkpoints",
        "Rewind code (checkpoints)",
        true,
        SG,
        "fileCheckpointingEnabled",
        "true",
        BOOL,
        Z,
    ),
    row(
        "workflows",
        "Dynamic workflows",
        true,
        S,
        "enableWorkflows",
        "true",
        BOOL,
        Z,
    ),
    row(
        "workflowKeywordTriggerEnabled",
        "Ultracode keyword trigger",
        true,
        S,
        "workflowKeywordTriggerEnabled",
        "true",
        BOOL,
        Z,
    ),
    row(
        "workflowSizeGuideline",
        "Dynamic workflow size",
        true,
        SG,
        "workflowSizeGuideline",
        "medium",
        Kind::Enum(&["unrestricted", "small", "medium", "large"]),
        Z,
    ),
    row(
        "artifacts",
        "Artifacts",
        false,
        S,
        "enableArtifact",
        "true",
        BOOL,
        Z,
    ),
    row(
        "verbose",
        "Verbose output",
        true,
        SG,
        "verbose",
        "false",
        BOOL,
        T,
    ),
    row(
        "progressBar",
        "Terminal progress bar",
        true,
        SG,
        "terminalProgressBarEnabled",
        "true",
        BOOL,
        T,
    ),
    row(
        "showStatusInTerminalTab",
        "Show status in terminal tab",
        false,
        G,
        "showStatusInTerminalTab",
        "false",
        BOOL,
        T,
    ),
    row(
        "turnDuration",
        "Show turn duration",
        true,
        SG,
        "showTurnDuration",
        "true",
        BOOL,
        T,
    ),
    row(
        "precomputeCompactionEnabled",
        "Precompute compaction",
        false,
        S,
        "precomputeCompactionEnabled",
        "true",
        BOOL,
        Z,
    ),
    row(
        "timestamps",
        "Show message timestamps",
        false,
        SG,
        "showMessageTimestamps",
        "false",
        BOOL,
        T,
    ),
    row(
        "timeFormat",
        "Time format",
        true,
        S,
        "timeFormat",
        "auto",
        Kind::Enum(&["auto", "12-hour", "24-hour", "24-hour-utc"]),
        T,
    ),
    chip("permissionMode", "Permission mode", Chip::Permission),
    row(
        "worktreeBaseRef",
        "Worktree base ref",
        true,
        S,
        "worktree.baseRef",
        "fresh",
        Kind::Enum(&["fresh", "head"]),
        Z,
    ),
    row(
        "useAutoModeDuringPlan",
        "Use auto mode during plan",
        true,
        S,
        "useAutoModeDuringPlan",
        "true",
        BOOL,
        Z,
    ),
    row(
        "gitignore",
        "Respect .gitignore in file picker",
        true,
        G,
        "respectGitignore",
        "true",
        BOOL,
        T,
    ),
    row(
        "copyFullResponse",
        "Skip the /copy picker",
        true,
        G,
        "copyFullResponse",
        "false",
        BOOL,
        T,
    ),
    row(
        "copyOnSelect",
        "Copy on select",
        true,
        G,
        "copyOnSelect",
        "true",
        BOOL,
        T,
    ),
    row(
        "autoScroll",
        "Auto-scroll",
        true,
        SG,
        "autoScrollEnabled",
        "true",
        BOOL,
        T,
    ),
    row(
        "defaultToAgentsView",
        "Open agents view by default",
        true,
        G,
        "defaultToAgentsView",
        "false",
        BOOL,
        T,
    ),
    row(
        "leftArrowOpensAgents",
        "← opens agents",
        true,
        G,
        "leftArrowOpensAgents",
        "true",
        BOOL,
        T,
    ),
    row(
        "theme",
        "Theme",
        true,
        SG,
        "theme",
        "dark",
        Kind::Enum(&[
            "auto",
            "dark",
            "light",
            "light-daltonized",
            "dark-daltonized",
            "light-ansi",
            "dark-ansi",
        ]),
        T,
    ),
    row(
        "outputStyle",
        "Output style",
        true,
        S,
        "outputStyle",
        "default",
        Kind::Enum(&["default", "Proactive", "Concise", "Explanatory", "Learning"]),
        Z,
    ),
    row(
        "defaultView",
        "Default view",
        false,
        S,
        "defaultView",
        "default",
        Kind::Enum(&["transcript", "chat", "default"]),
        T,
    ),
    row(
        "language",
        "Language",
        true,
        S,
        "language",
        "",
        Kind::Text,
        Z,
    ),
    chip("model", "Model", Chip::Model),
    chip("effort", "Effort", Chip::Effort),
    row(
        "teammateMode",
        "Teammate mode",
        true,
        SG,
        "teammateMode",
        "auto",
        Kind::Enum(&["auto", "tmux", "iterm2", "in-process"]),
        Z,
    ),
    row(
        "editor",
        "Editor mode",
        true,
        SG,
        "editorMode",
        "normal",
        Kind::Enum(&["normal", "vim"]),
        T,
    ),
    row(
        "notifChannel",
        "Notifications",
        true,
        SG,
        "preferredNotifChannel",
        "auto",
        Kind::Enum(&[
            "auto",
            "iterm2",
            "terminal_bell",
            "iterm2_with_bell",
            "kitty",
            "ghostty",
            "notifications_disabled",
        ]),
        T,
    ),
    row(
        "inputNeededNotifEnabled",
        "Push when actions required",
        true,
        SG,
        "inputNeededNotifEnabled",
        "false",
        BOOL,
        T,
    ),
    row(
        "agentPushNotifEnabled",
        "Push when Claude decides",
        true,
        SG,
        "agentPushNotifEnabled",
        "false",
        BOOL,
        T,
    ),
    row(
        "autoConnectIde",
        "Auto-connect to IDE (external terminal)",
        true,
        G,
        "autoConnectIde",
        "false",
        BOOL,
        T,
    ),
    row(
        "autoInstallIdeExtension",
        "Auto-install IDE extension",
        false,
        G,
        "autoInstallIdeExtension",
        "true",
        BOOL,
        T,
    ),
    row(
        "externalEditorContext",
        "Show last response in external editor",
        true,
        G,
        "externalEditorContext",
        "false",
        BOOL,
        T,
    ),
    row(
        "diffTool",
        "Diff tool",
        false,
        G,
        "diffTool",
        "auto",
        Kind::Enum(&["terminal", "auto"]),
        T,
    ),
    coded(
        row(
            "remoteControl",
            "Enable Remote Control for all sessions",
            true,
            G,
            "remoteControlAtStartup",
            "default",
            Kind::Enum(&["true", "false", "default"]),
            T,
        ),
        Codec::RemoteControl,
    ),
    row(
        "prStatus",
        "Show PR status footer",
        true,
        G,
        "prStatusFooterEnabled",
        "true",
        BOOL,
        T,
    ),
];

pub fn setting(id: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.id == id)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sources {
    #[serde(default)]
    pub user: serde_json::Map<String, Value>,
    #[serde(default)]
    pub project: serde_json::Map<String, Value>,
    #[serde(default)]
    pub local: serde_json::Map<String, Value>,
    #[serde(default)]
    pub global: serde_json::Map<String, Value>,
}

fn top_key(path: &str) -> &str {
    path.split('.').next().unwrap_or(path)
}

// Only keys the table names leave the host: these files also hold env vars and tokens.
pub fn keep_known_keys(map: &mut serde_json::Map<String, Value>) {
    map.retain(|key, _| {
        key == "disableWorkflows"
            || SETTINGS
                .iter()
                .any(|s| s.chip.is_none() && top_key(s.path) == key)
    });
}

fn lookup<'a>(map: &'a serde_json::Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut parts = path.split('.');
    let mut value = map.get(parts.next()?)?;
    for part in parts {
        value = value.get(part)?;
    }
    (!value.is_null()).then_some(value)
}

impl Sources {
    fn settings(&self, path: &str) -> Option<&Value> {
        [&self.local, &self.project, &self.user]
            .into_iter()
            .find_map(|map| lookup(map, path))
    }

    fn raw(&self, setting: &Setting) -> Option<&Value> {
        match setting.store {
            Store::Settings => self.settings(setting.path),
            Store::Global => lookup(&self.global, setting.path),
            Store::SettingsThenGlobal => self
                .settings(setting.path)
                .or_else(|| lookup(&self.global, setting.path)),
        }
    }

    pub fn value(&self, setting: &Setting) -> String {
        let raw = self.raw(setting);
        if setting.id == "workflows"
            && self.settings("disableWorkflows") == Some(&Value::Bool(true))
        {
            return "false".into();
        }
        match (setting.codec, raw) {
            (Codec::SwitchModels, Some(Value::Bool(true))) => SWITCH_AUTOMATICALLY.into(),
            (Codec::SwitchModels, Some(Value::Bool(false))) => ASK_EACH_TIME.into(),
            (_, Some(Value::Bool(b))) => b.to_string(),
            (_, Some(Value::String(s))) => s.clone(),
            (_, Some(Value::Number(n))) => n.to_string(),
            _ => setting.default.into(),
        }
    }

    pub fn values(&self) -> BTreeMap<String, String> {
        SETTINGS
            .iter()
            .filter(|s| s.chip.is_none())
            .map(|s| (s.id.to_string(), self.value(s)))
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeConfigSnapshot {
    pub values: BTreeMap<String, String>,
    #[serde(default)]
    pub files: Vec<String>,
}

pub const COMMAND_NOTE_PREFIX: &str = "Claude Code /";
const COMMAND_TEXT_CAP: usize = 32 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandCard {
    pub command: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CommandCard {
    pub fn capped(mut self) -> Self {
        if self.text.len() > COMMAND_TEXT_CAP {
            let mut end = COMMAND_TEXT_CAP;
            while !self.text.is_char_boundary(end) {
                end -= 1;
            }
            self.text.truncate(end);
        }
        self
    }

    pub fn from_note(text: &str, hint: Option<&str>) -> Option<Self> {
        text.starts_with(COMMAND_NOTE_PREFIX)
            .then(|| serde_json::from_str(hint?).ok())
            .flatten()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalCommandOutput {
    #[serde(default)]
    pub init: Option<Value>,
    pub text: String,
}

// Claude Code takes the rest of the line as the value and keeps quotes, so none are added.
pub fn config_command(setting: &Setting, value: &str) -> Option<String> {
    let key = setting.cli?;
    if value.contains('"') || value.contains('\n') {
        return None;
    }
    Some(format!("/config {key}={value}"))
}

// Headless /config reports refusals with is_error false; only "Set … to …" means it saved.
pub fn config_saved(text: &str) -> bool {
    text.trim_start().starts_with("Set ")
}

pub fn config_key_unknown(text: &str) -> bool {
    text.contains("isn't a /config setting")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> serde_json::Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn ids_are_unique_and_every_writable_row_uses_a_headless_key() {
        let headless = [
            "agentPushNotifEnabled",
            "autoCompact",
            "autoConnectIde",
            "autoScroll",
            "checkpoints",
            "copyFullResponse",
            "copyOnSelect",
            "defaultToAgentsView",
            "editor",
            "externalEditorContext",
            "gitignore",
            "inputNeededNotifEnabled",
            "language",
            "leftArrowOpensAgents",
            "model",
            "notifChannel",
            "outputStyle",
            "permissionMode",
            "prStatus",
            "progressBar",
            "promptSuggestionEnabled",
            "recap",
            "reduceMotion",
            "remoteControl",
            "switchModelsOnFlag",
            "teammateMode",
            "theme",
            "thinking",
            "timeFormat",
            "tips",
            "turnDuration",
            "useAutoModeDuringPlan",
            "verbose",
            "workflowKeywordTriggerEnabled",
            "workflowSizeGuideline",
            "workflows",
            "worktreeBaseRef",
        ];
        let mut ids: Vec<_> = SETTINGS.iter().map(|s| s.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), SETTINGS.len());
        for s in SETTINGS {
            if let Some(key) = s.cli {
                assert!(
                    headless.contains(&key),
                    "{key} is not a headless /config key"
                );
            }
        }
        for read_only in [
            "idleCompaction",
            "autoContinueAtUsageLimit",
            "fast",
            "artifacts",
            "feedbackDrafts",
        ] {
            assert_eq!(setting(read_only).unwrap().cli, None, "{read_only}");
        }
        assert!(
            SETTINGS
                .iter()
                .filter(|s| s.chip.is_some())
                .all(|s| s.cli.is_none())
        );
    }

    #[test]
    fn values_follow_claude_codes_precedence_and_defaults() {
        let sources = Sources {
            user: map(json!({
                "theme": "light",
                "autoCompactEnabled": false,
                "worktree": {"baseRef": "head"},
                "outputStyle": "Concise",
                "switchModelsOnFlag": false,
            })),
            project: map(json!({"outputStyle": "Learning"})),
            local: map(json!({"outputStyle": "Explanatory", "spinnerTipsEnabled": false})),
            global: map(json!({
                "theme": "dark-ansi",
                "verbose": true,
                "copyOnSelect": false,
                "remoteControlAtStartup": true,
            })),
        };
        let values = sources.values();
        assert_eq!(values["theme"], "light");
        assert_eq!(values["verbose"], "true");
        assert_eq!(values["autoCompact"], "false");
        assert_eq!(values["worktreeBaseRef"], "head");
        assert_eq!(values["outputStyle"], "Explanatory");
        assert_eq!(values["tips"], "false");
        assert_eq!(values["copyOnSelect"], "false");
        assert_eq!(values["switchModelsOnFlag"], ASK_EACH_TIME);
        assert_eq!(values["remoteControl"], "true");
        assert_eq!(values["gitignore"], "true");
        assert_eq!(values["editor"], "normal");
        assert_eq!(values["language"], "");
        assert!(!values.contains_key("model"));
        let empty = Sources::default().values();
        assert_eq!(empty["switchModelsOnFlag"], SWITCH_AUTOMATICALLY);
        assert_eq!(empty["remoteControl"], "default");
        assert_eq!(empty["workflowSizeGuideline"], "medium");
    }

    #[test]
    fn disable_workflows_wins_and_unknown_keys_never_leave_the_host() {
        let sources = Sources {
            user: map(json!({"disableWorkflows": true, "enableWorkflows": true})),
            ..Default::default()
        };
        assert_eq!(sources.values()["workflows"], "false");
        let mut global = map(json!({
            "oauthAccount": {"email": "x"},
            "theme": "dark",
            "remoteControlAtStartup": false,
        }));
        keep_known_keys(&mut global);
        assert_eq!(
            global.keys().collect::<Vec<_>>(),
            ["remoteControlAtStartup", "theme"]
        );
    }

    #[test]
    fn command_cards_round_trip_through_a_note_and_stay_small() {
        let card = CommandCard {
            command: "/context".into(),
            text: "x".repeat(40 * 1024),
            ..Default::default()
        }
        .capped();
        assert_eq!(card.text.len(), 32 * 1024);
        let hint = serde_json::to_string(&card).unwrap();
        assert_eq!(
            CommandCard::from_note("Claude Code /context · 59.9k / 1M", Some(&hint)),
            Some(card)
        );
        assert_eq!(
            CommandCard::from_note("Context compacted", Some(&hint)),
            None
        );
        assert_eq!(
            CommandCard::from_note("Claude Code /usage", Some("{")),
            None
        );
    }

    #[test]
    fn config_lines_keep_spaces_unquoted_and_refuse_read_only_rows() {
        let switch = setting("switchModelsOnFlag").unwrap();
        assert_eq!(
            config_command(switch, ASK_EACH_TIME).as_deref(),
            Some("/config switchModelsOnFlag=Ask each time")
        );
        assert_eq!(
            config_command(setting("theme").unwrap(), "dark").as_deref(),
            Some("/config theme=dark")
        );
        assert_eq!(config_command(setting("fast").unwrap(), "true"), None);
        assert_eq!(config_command(setting("model").unwrap(), "opus"), None);
        assert_eq!(config_command(setting("language").unwrap(), "a\"b"), None);
        assert!(config_saved("Set Theme to dark"));
        assert!(!config_saved(
            "Verbose output takes true or false, not \"maybe\"."
        ));
        assert!(config_key_unknown(
            "remoteControl isn't a /config setting. Run /config to see what's available."
        ));
    }
}
