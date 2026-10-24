use serde_json::{Map, Value, json};
use zeron_proto::claude_config::{
    self as cc, ClaudeConfigSnapshot, Kind, LocalCommandOutput, Sources,
};

const SETTINGS_FILE: &str = "/Users/demo/.claude/settings.json";

const USAGE: &str = "You are currently using your subscription to power your Claude Code usage

Current session: 5% used · resets Oct 25 at 2:50am (Asia/Saigon)
Current week (all models): 81% used · resets Oct 29 at 5am (Asia/Saigon)
Current week (Fable): 2% used · resets Oct 29 at 5am (Asia/Saigon)

What's contributing to your limits usage?
Approximate · sessions on this machine only · shares overlap

Last 24h · 2895 requests · 147 sessions
  85%  Give \"general-purpose\" subagents a cheaper model
  77%  Stop sessions you no longer need · some ran 8 hours or more
  77%  Start fresh when you switch tasks · run /clear
  24%  Run fewer sessions at once · up to 7 at once
  17%  See what fills your context · run /context
  Top subagents: general-purpose 49%
  Top MCP servers: zeron 3%

Last 7d · 67142 requests · 1484 sessions
  90%  Give \"general-purpose\" subagents a cheaper model
  80%  Stop sessions you no longer need · some ran 8 hours or more
  51%  Run fewer sessions at once · up to 27 at once
  Top subagents: general-purpose 57%, maestro-delegate 7%
  Top MCP servers: zeron 1%";

const CONTEXT: &str = "## Context Usage

**Model:** claude-haiku-5-5
**Tokens:** 50k / 1m (5%)

### Estimated usage by category

| Category | Tokens | Percentage |
|----------|--------|------------|
| System prompt | 3.3k | 0.3% |
| System tools | 971 | 0.1% |
| MCP tools | 599 | 0.1% |
| MCP tools (deferred) | 33.9k | 3.4% |
| Custom agents | 1.8k | 0.2% |
| Memory files | 32.7k | 3.3% |
| Skills | 9.7k | 1.0% |
| Messages | 10 | 0.0% |
| Free space | 917k | 91.7% |
| Autocompact buffer | 33k | 3.3% |

### MCP Tools

| Tool | Server | Tokens |
|------|--------|--------|
| mcp__semble__search | semble | 519 |
| mcp__zeron__docs | zeron | 1.1k |";

const MCP: &str = "7 MCP server(s): 4 connected, 3 not connected, 0 disabled. Use `/mcp` in the terminal for details.";

fn command_reply(command: &str) -> Option<&'static str> {
    Some(match command {
        "/context" => CONTEXT,
        "/usage" | "/cost" => USAGE,
        "/mcp" => MCP,
        _ => return None,
    })
}

fn init() -> Value {
    json!({
        "type": "system",
        "subtype": "init",
        "claude_code_version": "2.1.310",
        "cwd": "/Users/demo/Code/zeron",
        "output_style": "default",
        "mcp_servers": [
            { "name": "zeron", "status": "connected" },
            { "name": "semble", "status": "connected" },
            { "name": "codex-cu", "status": "connected" },
            { "name": "Claude Docs", "status": "connected" },
            { "name": "mcp-agent-mail", "status": "failed" },
            { "name": "paper", "status": "failed" },
            { "name": "xcode", "status": "failed" }
        ],
        "memory_paths": {
            "user": "/Users/demo/.claude/CLAUDE.md",
            "project": "/Users/demo/Code/zeron/CLAUDE.md"
        },
        "plugins": [ { "name": "codex" }, { "name": "pstack" } ]
    })
}

pub(super) fn default_settings() -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("theme".into(), "dark".into());
    map.insert("autoCompactEnabled".into(), true.into());
    map.insert("switchModelsOnFlag".into(), false.into());
    map
}

pub(super) fn snapshot(settings: &Map<String, Value>) -> ClaudeConfigSnapshot {
    let sources = Sources {
        user: settings.clone(),
        ..Default::default()
    };
    ClaudeConfigSnapshot {
        values: sources.values(),
        files: vec![SETTINGS_FILE.into()],
    }
}

fn set_path(map: &mut Map<String, Value>, path: &str, value: Value) {
    let mut parts = path.split('.').peekable();
    let mut map = map;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            map.insert(part.into(), value);
            return;
        }
        let next = map
            .entry(part.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !next.is_object() {
            *next = Value::Object(Map::new());
        }
        map = next.as_object_mut().unwrap();
    }
}

pub(super) fn run(settings: &mut Map<String, Value>, command: &str) -> LocalCommandOutput {
    let text = match command.strip_prefix("/config ") {
        Some(line) => {
            let (key, value) = line.split_once('=').unwrap_or((line, ""));
            match cc::SETTINGS.iter().find(|s| s.cli == Some(key)) {
                Some(_) if key == "agentPushNotifEnabled" => {
                    format!("{key} isn't a /config setting.")
                }
                None => format!("{key} isn't a /config setting."),
                Some(setting) => {
                    let raw = match (setting.kind, setting.codec, value) {
                        (_, cc::Codec::SwitchModels, v) => {
                            Value::Bool(v == cc::SWITCH_AUTOMATICALLY)
                        }
                        (Kind::Bool, _, v) => Value::Bool(v == "true"),
                        (_, _, v) => Value::String(v.into()),
                    };
                    set_path(settings, setting.path, raw);
                    format!("Set {key} to {value}")
                }
            }
        }
        None => command_reply(command).unwrap_or_default().to_string(),
    };
    LocalCommandOutput {
        init: Some(init()),
        text,
    }
}
