use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use zeron_proto::routing::{Preview, RoutingSettings};
use zeron_proto::tools::{ToolGroup, ToolSettings};
use zeron_proto::{AgentAccountsSnapshot, AgentProfile, AgentProfilesSnapshot, LimitResumes};
use zeron_rpc::methods as m;

use super::fixtures::{MAC, MINI};
use crate::error::{ClientError, Result};

pub(super) struct HostSettings {
    accounts: HashMap<String, AgentAccountsSnapshot>,
    tools: ToolSettings,
    auto_resume: bool,
    routing: RoutingSettings,
    profiles: AgentProfilesSnapshot,
    system_prompt: String,
    pub(super) disabled_harnesses: Vec<String>,
    instances: Value,
    updates: Value,
    login: Option<(String, u32)>,
    review: zeron_proto::review::ReviewSettings,
}

impl HostSettings {
    pub(super) fn new() -> Self {
        let mut tools = ToolSettings::default();
        tools.set_tool("html_render", false);
        let routing = serde_json::from_value(json!({
            "roles": {
                "worker": { "kind": "target", "harness": "codex", "model": "gpt-5.5", "reasoning": "high" },
                "validator": { "kind": "sameAsParent" },
                "research": { "kind": "target", "harness": "claude-code", "model": "claude-sonnet-5-5" },
                "design": { "kind": "target", "harness": "grok" }
            },
            "fallback": [{ "harness": "claude-code" }]
        }))
        .expect("demo routing decodes");
        let mut profiles: AgentProfilesSnapshot = serde_json::from_value(json!({ "profiles": [
            { "id": "ui-work", "name": "UI work", "harness": "claude-code", "model": "claude-opus-5-5",
              "permissionMode": "auto", "reasoning": "high",
              "instructions": "Match the kit; prove UI with a screenshot.",
              "disabledTools": ["mcp__zeron__computer", "mcp__zeron__device_open", "WebSearch",
                                "mcp__figma", "mcp__retired_server"] },
            { "id": "reviewer", "name": "reviewer", "harness": "codex", "model": "gpt-5.5",
              "permissionMode": "ask", "reasoning": "high", "byAgent": true,
              "notes": "Review a finished diff before it lands." },
            { "id": "quick-fix", "name": "Quick fix", "harness": "grok", "model": "grok-4",
              "permissionMode": "full", "reasoning": "low" }
        ]}))
        .expect("demo profiles decode");
        profiles.profiles.splice(0..0, super::composer::profiles());
        let instances: Vec<Value> = super::composer::instances()
            .into_iter()
            .map(|instance| json!(instance))
            .chain([
                json!({ "id": "claude-work", "driver": "claude-code", "label": "Work",
                  "accent": "#3e9cf0",
                  "config": { "kind": "native", "homeDir": { "kind": "managed" } } }),
                json!({ "id": super::composer::ACP_INSTANCE, "driver": "acp", "label": "Gemini CLI",
                  "config": { "kind": "acp", "iconPath": super::composer::ACP_ICON,
                    "source": { "kind": "registry",
                    "agentId": "gemini", "version": "0.9.1", "distribution": "npx" } } }),
            ])
            .collect();
        Self {
            accounts: HashMap::new(),
            tools,
            auto_resume: true,
            routing,
            profiles,
            system_prompt: String::new(),
            disabled_harnesses: Vec::new(),
            instances: json!({ "instances": instances }),
            updates: json!([
                { "harness": "claude-code", "installedVersion": "2.1.3", "latestVersion": "2.1.4",
                  "phase": "available", "canApply": true },
                { "harness": "codex", "installedVersion": "0.51.0", "phase": "current" },
                { "harness": "grok", "installedVersion": "1.4.2", "phase": "current" },
                { "harness": "pi", "installedVersion": "0.9.0", "phase": "current" }
            ]),
            login: None,
            review: zeron_proto::review::ReviewSettings {
                instructions: "Flag anything that could drop a draft or a queued message.".into(),
                ..Default::default()
            },
        }
    }

    fn accounts(&mut self, device_id: &str) -> &mut AgentAccountsSnapshot {
        self.accounts
            .entry(device_id.to_owned())
            .or_insert_with(|| demo_accounts(device_id, Utc::now()))
    }

    fn add_account(&mut self, device_id: &str, harness: &str) {
        self.login = None;
        let accounts = self.accounts(device_id);
        let n = accounts.accounts.len();
        if let Ok(account) = serde_json::from_value(json!({
            "id": format!("added-{n}"), "harness": harness, "email": format!("reina+{n}@zeron.sh"),
            "planLabel": "Pro", "active": false, "switchable": true
        })) {
            accounts.accounts.push(account);
        }
    }

    pub(super) fn handles(method: &str) -> bool {
        matches!(
            method,
            m::LIST_AGENT_ACCOUNTS
                | m::ACTIVATE_AGENT_ACCOUNT
                | m::FORGET_AGENT_ACCOUNT
                | m::GET_TOOL_SETTINGS
                | m::SET_TOOL_SETTINGS
                | m::WATCH_LIMIT_RESUMES
                | m::SET_AUTO_RESUME_LIMITS
                | m::GET_ROUTING_SETTINGS
                | m::SET_ROUTING_SETTINGS
                | m::PREVIEW_ROUTES
                | m::LIST_AGENT_PROFILES
                | m::UPSERT_AGENT_PROFILE
                | m::DELETE_AGENT_PROFILE
                | m::REORDER_AGENT_PROFILES
                | m::LIST_HARNESS_TOOLS
                | m::GET_SYSTEM_PROMPT
                | m::SET_SYSTEM_PROMPT
                | m::WATCH_HARNESS_UPDATES
                | m::CHECK_HARNESS_UPDATES
                | m::APPLY_HARNESS_UPDATE
                | m::SET_HARNESS_ENABLED
                | m::LIST_PROVIDER_INSTANCES
                | m::DELETE_PROVIDER_INSTANCE
                | m::UPSERT_PROVIDER_INSTANCE
                | m::SEARCH_ACP_REGISTRY
                | m::PREPARE_ACP_AGENT
                | m::LIST_ACP_AUTH_METHODS
                | m::GET_USAGE_SUMMARY
                | m::START_AGENT_LOGIN
                | m::POLL_AGENT_LOGIN
                | m::COMPLETE_AGENT_LOGIN
                | m::CANCEL_AGENT_LOGIN
                | m::GET_REVIEW_SETTINGS
                | m::SET_REVIEW_SETTINGS
        )
    }

    pub(super) fn call(&mut self, device_id: &str, method: &str, params: &Value) -> Result<Value> {
        match method {
            m::LIST_AGENT_ACCOUNTS => to_json(self.accounts(device_id)),
            m::ACTIVATE_AGENT_ACCOUNT | m::FORGET_AGENT_ACCOUNT => {
                let harness = params["harness"].clone();
                let id = params["accountId"].as_str().unwrap_or_default();
                let same_harness = |a: &zeron_proto::AgentAccount| {
                    to_json(&a.harness).ok() == Some(harness.clone())
                };
                let accounts = self.accounts(device_id);
                if method == m::FORGET_AGENT_ACCOUNT {
                    accounts
                        .accounts
                        .retain(|a| !(same_harness(a) && a.id == id));
                } else {
                    for account in accounts.accounts.iter_mut() {
                        if same_harness(account) {
                            account.active = account.id == id;
                        }
                    }
                }
                to_json(accounts)
            }
            m::GET_TOOL_SETTINGS => to_json(&self.tools.state(&[ToolGroup::Computer])),
            m::SET_TOOL_SETTINGS => {
                self.tools = serde_json::from_value(params.clone())
                    .map_err(|e| ClientError::InvalidArgument(e.to_string()))?;
                to_json(&self.tools.state(&[ToolGroup::Computer]))
            }
            m::GET_ROUTING_SETTINGS => to_json(&self.routing),
            m::GET_REVIEW_SETTINGS => to_json(&self.review),
            m::SET_REVIEW_SETTINGS => {
                self.review = serde_json::from_value(params.clone())
                    .map_err(|e| ClientError::InvalidArgument(e.to_string()))?;
                to_json(&self.review)
            }
            m::SET_ROUTING_SETTINGS => {
                self.routing = serde_json::from_value(params.clone())
                    .map_err(|e| ClientError::InvalidArgument(e.to_string()))?;
                to_json(&self.routing)
            }
            m::PREVIEW_ROUTES => to_json(&self.preview()),
            m::LIST_AGENT_PROFILES => to_json(&self.profiles),
            m::UPSERT_AGENT_PROFILE => {
                let draft: zeron_proto::AgentProfileDraft =
                    serde_json::from_value(params["profile"].clone())
                        .map_err(|e| ClientError::InvalidArgument(e.to_string()))?;
                if draft.name.trim().is_empty() {
                    return Err(ClientError::HostError("Profile name is required".into()));
                }
                let id = params["profileId"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| draft.name.to_lowercase().replace(' ', "-"));
                let profile = AgentProfile {
                    id: id.clone(),
                    name: draft.name.trim().to_owned(),
                    harness: draft.harness,
                    model: draft.model,
                    reasoning: draft.reasoning,
                    permission_mode: draft.permission_mode,
                    model_options: draft.model_options,
                    notes: draft.notes,
                    instructions: draft.instructions,
                    disabled_tools: draft.disabled_tools,
                    by_agent: false,
                };
                match self.profiles.profiles.iter_mut().find(|p| p.id == id) {
                    Some(slot) => *slot = profile,
                    None => self.profiles.profiles.push(profile),
                }
                to_json(&self.profiles)
            }
            m::DELETE_AGENT_PROFILE => {
                let id = params["profileId"].as_str().unwrap_or_default();
                self.profiles.profiles.retain(|p| p.id != id);
                to_json(&self.profiles)
            }
            m::REORDER_AGENT_PROFILES => {
                let order: Vec<&str> = params["profileIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                self.profiles.profiles.sort_by_key(|p| {
                    order
                        .iter()
                        .position(|id| *id == p.id)
                        .unwrap_or(usize::MAX)
                });
                to_json(&self.profiles)
            }
            m::LIST_HARNESS_TOOLS => Ok(json!({
                "builtin": [
                    { "id": "shell", "title": "Shell", "summary": "Run commands and background shells.",
                      "tools": ["Bash", "BashOutput", "KillShell"] },
                    { "id": "web", "title": "Web", "summary": "Fetch pages and search the web.",
                      "tools": ["WebFetch", "WebSearch"] },
                    { "id": "agents", "title": "Subagents", "summary": "Spawn and message subagents.",
                      "tools": ["Agent", "SendMessage"] },
                    { "id": "skills", "title": "Skills", "summary": "Load skills on demand.",
                      "tools": ["Skill"] }
                ],
                "mcpServers": [
                    { "name": "linear", "rule": "mcp__linear", "status": "connected", "source": "user" },
                    { "name": "figma", "rule": "mcp__figma", "status": "needs auth", "source": "project" }
                ]
            })),
            m::WATCH_HARNESS_UPDATES | m::CHECK_HARNESS_UPDATES => Ok(self.updates.clone()),
            m::APPLY_HARNESS_UPDATE => {
                let harness = params["harness"].clone();
                for status in self.updates.as_array_mut().into_iter().flatten() {
                    if status["harness"] == harness {
                        status["installedVersion"] = status["latestVersion"].clone();
                        status["phase"] = json!("updated");
                    }
                }
                Ok(self.updates.clone())
            }
            m::SET_HARNESS_ENABLED => {
                let harness = params["harness"].as_str().unwrap_or_default().to_owned();
                self.disabled_harnesses.retain(|h| *h != harness);
                if params["enabled"] == json!(false) {
                    self.disabled_harnesses.push(harness);
                }
                Ok(json!({}))
            }
            m::START_AGENT_LOGIN => {
                let harness = params["harness"].as_str().unwrap_or_default().to_owned();
                let start = match harness.as_str() {
                    "claude-code" => json!({ "loginId": "demo-login", "mode": "paste-code",
                        "url": "https://claude.ai/oauth/authorize?code=true&client_id=demo" }),
                    "codex" => json!({ "loginId": "demo-login", "mode": "browser",
                        "url": "http://localhost:1455/auth/start", "callbackPort": 1455 }),
                    _ => json!({ "loginId": "demo-login", "mode": "browser",
                        "url": "https://example.com/device" }),
                };
                self.login = Some((harness, 0));
                Ok(start)
            }
            m::POLL_AGENT_LOGIN => {
                let Some((harness, polls)) = self.login.as_mut() else {
                    return Ok(json!({ "status": "error", "message": "No sign-in is running." }));
                };
                *polls += 1;
                if *polls > 2 && harness != "claude-code" {
                    let harness = harness.clone();
                    self.add_account(device_id, &harness);
                    return Ok(json!({ "status": "done" }));
                }
                Ok(match harness.as_str() {
                    "claude-code" | "codex" => json!({ "status": "pending" }),
                    _ => {
                        json!({ "status": "pending", "message": "Enter code WXYZ-4821 on the sign-in page." })
                    }
                })
            }
            m::COMPLETE_AGENT_LOGIN => {
                if let Some((harness, _)) = self.login.clone() {
                    self.add_account(device_id, &harness);
                }
                to_json(self.accounts(device_id))
            }
            m::CANCEL_AGENT_LOGIN => {
                self.login = None;
                Ok(json!({ "ok": true }))
            }
            m::GET_USAGE_SUMMARY
                if params["days"].as_u64()
                    == Some(zeron_proto::claude_panel::STATS_DAYS.into()) =>
            {
                Ok(super::claude::stats_summary())
            }
            m::GET_USAGE_SUMMARY => {
                let scale = params["days"].as_f64().unwrap_or(30.0) / 30.0;
                let bucket = |harness: &str, model: &str, cost: f64, tokens: f64| {
                    json!({
                        "period": 0, "harness": harness, "model": model,
                        "tokens": {
                            "uncachedInput": (tokens * 0.16 * scale) as u64,
                            "cachedInput": (tokens * 0.8 * scale) as u64,
                            "cacheWrite": 0,
                            "output": (tokens * 0.04 * scale) as u64
                        },
                        "costUsd": cost * scale, "cacheSavingsUsd": cost * 1.33 * scale,
                        "records": 100, "unpricedRecords": 0
                    })
                };
                Ok(json!({
                    "hourly": false, "periods": [Utc::now().timestamp_millis()],
                    "untilMs": Utc::now().timestamp_millis(), "pricesLoaded": true,
                    "buckets": [
                        bucket("claude-code", "claude-opus-5-5", 105.80, 41.2e6),
                        bucket("codex", "gpt-5.5", 71.10, 33.0e6),
                        bucket("grok", "grok-4", 5.50, 2.4e6)
                    ],
                    "sessions": [
                        { "harness": "claude-code", "sessions": (120.0 * scale) as u64 },
                        { "harness": "codex", "sessions": (80.0 * scale) as u64 },
                        { "harness": "grok", "sessions": (14.0 * scale) as u64 }
                    ]
                }))
            }
            m::LIST_PROVIDER_INSTANCES => Ok(self.instances.clone()),
            m::DELETE_PROVIDER_INSTANCE => {
                let id = params["instanceId"].clone();
                if let Some(list) = self.instances["instances"].as_array_mut() {
                    list.retain(|i| i["id"] != id);
                }
                Ok(self.instances.clone())
            }
            m::UPSERT_PROVIDER_INSTANCE => {
                let instance = params["instance"].clone();
                if instance["label"]
                    .as_str()
                    .is_none_or(|l| l.trim().is_empty())
                {
                    return Err(ClientError::HostError(
                        "Provider instance label is required".into(),
                    ));
                }
                if let Some(list) = self.instances["instances"].as_array_mut() {
                    match list.iter_mut().find(|i| i["id"] == instance["id"]) {
                        Some(slot) => *slot = instance,
                        None => list.push(instance),
                    }
                }
                Ok(self.instances.clone())
            }
            m::SEARCH_ACP_REGISTRY => {
                let query = params["query"].as_str().unwrap_or_default().to_lowercase();
                let agents = registry()
                    .into_iter()
                    .filter(|a| {
                        query.is_empty()
                            || a["id"].as_str().unwrap_or_default().contains(&query)
                            || a["name"]
                                .as_str()
                                .unwrap_or_default()
                                .to_lowercase()
                                .contains(&query)
                    })
                    .collect::<Vec<_>>();
                Ok(json!(agents))
            }
            m::PREPARE_ACP_AGENT => {
                let id = params["agentId"].as_str().unwrap_or_default();
                let agent = registry()
                    .into_iter()
                    .find(|a| a["id"] == id)
                    .ok_or_else(|| ClientError::HostError(format!("No ACP agent '{id}'")))?;
                if agent["unverified"] == json!(true) && params["allowUnverified"] != json!(true) {
                    return Err(ClientError::HostError(
                        "No checksum for this download".into(),
                    ));
                }
                Ok(json!({
                    "agentId": agent["id"], "version": agent["version"],
                    "distribution": agent["distributions"][0], "displayName": agent["name"]
                }))
            }
            m::LIST_ACP_AUTH_METHODS => Ok(json!([
                { "id": "oauth-personal", "name": "Log in with Google", "type": "agent",
                  "description": "Opens Google sign-in in the browser." },
                { "id": "gemini-api-key", "name": "Use a Gemini API key", "type": "env_var",
                  "description": "Set GEMINI_API_KEY in this instance's environment." }
            ])),
            m::GET_SYSTEM_PROMPT => Ok(json!({ "prompt": self.system_prompt })),
            m::SET_SYSTEM_PROMPT => {
                self.system_prompt = params["prompt"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                Ok(json!({ "prompt": self.system_prompt }))
            }
            m::SET_AUTO_RESUME_LIMITS => {
                self.auto_resume = params["enabled"].as_bool().unwrap_or(self.auto_resume);
                self.limits()
            }
            _ => self.limits(),
        }
    }

    fn preview(&self) -> Preview {
        let rows = self
            .routing
            .roles
            .keys()
            .map(|role| {
                let used = match role.name() {
                    "worker" => Some(0.18),
                    "research" => Some(0.62),
                    _ => None,
                };
                if role.name() == "design" {
                    return json!({
                        "role": "design", "source": "fell_back",
                        "quota": { "known": true, "usedFraction": 1.0, "exhausted": true },
                        "fallback": { "target": { "harness": "claude-code" }, "native": true, "reason": "out of quota" }
                    });
                }
                json!({
                    "role": role.name(), "source": "routed",
                    "quota": { "known": used.is_some(), "usedFraction": used }
                })
            })
            .collect::<Vec<_>>();
        serde_json::from_value(json!({ "rows": rows })).unwrap_or_default()
    }

    fn limits(&self) -> Result<Value> {
        to_json(&LimitResumes {
            auto_resume: self.auto_resume,
            chats: Vec::new(),
        })
    }
}

fn registry() -> Vec<Value> {
    vec![
        json!({ "id": "gemini", "name": "Gemini CLI", "version": "0.9.1",
                "description": "Google's coding agent", "distributions": ["npx"], "installed": true }),
        json!({ "id": "goose", "name": "Goose", "version": "1.8.0",
                "description": "Block's open agent", "distributions": ["binary"], "unverified": true }),
        json!({ "id": "qwen-code", "name": "Qwen Code", "version": "0.2.0",
                "description": "Qwen's terminal agent", "distributions": ["npx"] }),
        json!({ "id": "fast-agent", "name": "fast-agent", "version": "0.3.4",
                "description": "MCP-native agent framework", "distributions": ["uvx"] }),
    ]
}

fn to_json<T: serde::Serialize + ?Sized>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| ClientError::Internal(e.to_string()))
}

fn demo_accounts(device_id: &str, now: DateTime<Utc>) -> AgentAccountsSnapshot {
    let at = |minutes: i64| now + Duration::minutes(minutes);
    let mut accounts = match device_id {
        MAC => vec![
            json!({
                "id": "reina", "harness": "claude-code", "email": "reina@zeron.sh",
                "planLabel": "Max 20x", "active": true, "switchable": true,
                "usageWindows": [
                    { "label": "Session", "usedFraction": 0.38, "resetsAt": at(125) },
                    { "label": "Week", "usedFraction": 0.21, "resetsAt": at(3 * 24 * 60) }
                ],
                "extraUsage": { "enabled": true, "used": 12.4, "limit": 50.0, "currency": "USD" }
            }),
            json!({
                "id": "work", "harness": "claude-code", "email": "reina@acme.dev",
                "planLabel": "Team", "organization": "Acme", "active": false, "switchable": true
            }),
        ],
        MINI => vec![json!({
            "id": "codex", "harness": "codex", "email": "reina@zeron.sh",
            "planLabel": "Pro", "active": true, "switchable": true,
            "usageWindows": [
                { "label": "Session", "usedFraction": 0.12, "resetsAt": at(200) },
                { "label": "Week", "usedFraction": 0.30, "resetsAt": at(5 * 24 * 60) }
            ]
        })],
        _ => Vec::new(),
    };
    let primary: Vec<Value> = accounts.iter().map(|a| a["harness"].clone()).collect();
    let others = ["claude-code", "codex", "devin", "grok", "hermes"]
        .into_iter()
        .filter(|harness| !primary.contains(&json!(harness)));
    accounts.extend(others.enumerate().map(|(i, harness)| {
        json!({
            "id": format!("{device_id}-{harness}"), "harness": harness, "email": "dev@zeron.sh",
            "planLabel": "Max", "active": true,
            "usageWindows": [
                { "label": "Session", "usedFraction": 0.04 + 0.02 * i as f32, "resetsAt": null },
                { "label": "Week", "usedFraction": 0.08, "resetsAt": null }
            ]
        })
    }));
    serde_json::from_value(json!({ "accounts": accounts, "warnings": [] }))
        .expect("demo accounts decode")
}
