//! Agent profiles: saved agents, models, modes, efforts, options,
//! instructions and tools off, kept by the engine (`ListAgentProfiles` and
//! its writes, crates/engine/src/agent_profiles.rs) and applied to a chat,
//! compared with it and saved from it by the shipping app's own rules
//! (crates/proto/src/picker.rs).
//!
//! `profiles.list` is the list as scripts read it: `{ state, error?,
//! profiles }`, `state` "idle" until asked for, then "loading", "ready" or
//! "error". `profiles.load` asks for it again; `profiles.save`,
//! `profiles.absorb`, `profiles.remove` and `profiles.reorder` write it and
//! settle with nothing (or reject with the engine's words);
//! `profiles.system_prompt` and `profiles.set_system_prompt` read and write
//! the host's own prompt; `profiles.tools` lists what a profile can turn
//! off. `profiles.drift` (sync) compares a profile with a new chat's picks;
//! a chat's comparison rides its session (`profile`, src/chat.rs).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use serde_json::{Value as Json, json};
use zeron_proto::picker::{ProfileDrift, savable};
use zeron_proto::tools::{HarnessToolInventory, ToolSettingsState, listed_tool_groups, unlisted_rules, zeron_rule};
use zeron_proto::{AccessMode, AgentProfile, AgentProfileDraft, AgentProfilesSnapshot, ChatConfig, HarnessId, ReasoningLevel};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Shared, Value};

use crate::auth::text;
use crate::engine;

#[derive(Clone, Copy, Default, PartialEq)]
enum State {
    #[default]
    Idle,
    Loading,
    Ready,
    Error,
}

#[derive(Default)]
struct Profiles {
    list: RefCell<Vec<AgentProfile>>,
    state: Cell<State>,
    error: RefCell<Option<String>>,
    shared: RefCell<Vec<Shared>>,
    seeded: Cell<bool>,
}

thread_local! {
    static PROFILES: Rc<Profiles> = Rc::new(Profiles::default());
}

/// The profile of that id, as last listed.
pub(crate) fn find(id: &str) -> Option<AgentProfile> {
    PROFILES.with(|p| p.list.borrow().iter().find(|profile| profile.id == id).cloned())
}

pub fn install(_cx: &mut WindowContext, host: &ScriptHost) {
    let profiles = PROFILES.with(Rc::clone);
    profiles.shared.borrow_mut().push(host.shared("profiles.list", profiles.value()));
    let p = profiles.clone();
    host.action("profiles.load", move |_| {
        let p = p.clone();
        engine::get().spawn(async move { p.load().await });
    });
    let p = profiles.clone();
    host.async_action("profiles.save", move |args| {
        let p = p.clone();
        async move {
            let id = args.field("id").map(text).filter(|s| !s.is_empty());
            let draft = draft_of(args.field("draft").unwrap_or(&Value::Nil))?;
            p.write("UpsertAgentProfile", json!({ "profileId": id, "profile": draft })).await
        }
    });
    let p = profiles.clone();
    host.async_action("profiles.absorb", move |args| {
        let p = p.clone();
        async move {
            let id = args.field("id").map(text).unwrap_or_default();
            let profile = find(&id).ok_or("Agent profile not found")?;
            let config = current_config(args.field("current").unwrap_or(&Value::Nil)).ok_or("failed")?;
            let draft = profile.absorb(&config);
            p.write("UpsertAgentProfile", json!({ "profileId": id, "profile": draft })).await
        }
    });
    let p = profiles.clone();
    host.async_action("profiles.remove", move |args| {
        let p = p.clone();
        async move {
            let id = args.field("id").map(text).unwrap_or_default();
            p.write("DeleteAgentProfile", json!({ "profileId": id })).await
        }
    });
    let p = profiles.clone();
    host.async_action("profiles.reorder", move |args| {
        let p = p.clone();
        async move {
            let ids: Vec<String> = args.field("ids").map(Value::list).unwrap_or_default().iter().map(text).collect();
            p.write("ReorderAgentProfiles", json!({ "profileIds": ids })).await
        }
    });
    host.async_action("profiles.system_prompt", |_| async move {
        let answer = engine::get().call("GetSystemPrompt", json!({})).await?;
        Ok(Value::from(answer["prompt"].as_str().unwrap_or_default()))
    });
    host.async_action("profiles.set_system_prompt", |args| async move {
        let prompt = args.field("prompt").map(text).unwrap_or_default();
        let answer = engine::get().call("SetSystemPrompt", json!({ "prompt": prompt.trim() })).await?;
        Ok(Value::from(answer["prompt"].as_str().unwrap_or_default()))
    });
    host.async_action("profiles.tools", |args| async move {
        let harness = args.field("harness").map(text).unwrap_or_default();
        let off: Vec<String> = args.field("tools_off").map(Value::list).unwrap_or_default().iter().map(text).collect();
        let ceiling = engine::get()
            .call("GetToolSettings", json!({}))
            .await
            .and_then(|v| serde_json::from_value::<ToolSettingsState>(v).map_err(|e| e.to_string()));
        let claude = if harness == "claude-code" {
            Some(
                engine::get()
                    .call("ListHarnessTools", json!({ "harness": "claude-code" }))
                    .await
                    .and_then(|v| serde_json::from_value::<HarnessToolInventory>(v).map_err(|e| e.to_string())),
            )
        } else {
            None
        };
        Ok(tools_value(ceiling, claude, &off))
    });
    host.function("profiles.drift", |args| {
        let id = args.field("profile").map(text).unwrap_or_default();
        match (find(&id), current_config(&args)) {
            (Some(profile), Some(config)) => applied_value(&profile, &config),
            _ => Value::Nil,
        }
    });
    host.function("profiles.current", |args| current_config(&args).map_or(Value::Nil, |c| draft_value(&current_draft(&c))));
}

impl Profiles {
    fn value(&self) -> Value {
        let state = match self.state.get() {
            State::Idle => "idle",
            State::Loading => "loading",
            State::Ready => "ready",
            State::Error => "error",
        };
        let list: Vec<Value> = self.list.borrow().iter().map(profile_value).collect();
        Value::record([
            ("state", Value::from(state)),
            ("error", self.error.borrow().as_deref().map_or(Value::Nil, Value::from)),
            ("profiles", Value::List(list.into())),
        ])
    }

    fn publish(&self) {
        let value = self.value();
        for shared in self.shared.borrow().iter() {
            shared.set(value.clone());
        }
        crate::chat::profiles_changed();
    }

    fn took(&self, snapshot: Json) -> Result<(), String> {
        let snapshot: AgentProfilesSnapshot = serde_json::from_value(snapshot).map_err(|e| e.to_string())?;
        *self.list.borrow_mut() = snapshot.profiles;
        *self.error.borrow_mut() = None;
        self.state.set(State::Ready);
        self.publish();
        Ok(())
    }

    async fn load(self: &Rc<Self>) {
        if self.state.get() == State::Loading {
            return;
        }
        let first = self.state.get() == State::Idle;
        self.state.set(State::Loading);
        if first {
            self.publish();
        }
        let seed = crate::chat::mock_seed();
        if let Some(error) = seed.as_ref().and_then(|s| s["profilesError"].as_str()) {
            *self.error.borrow_mut() = Some(error.to_owned());
            self.state.set(State::Error);
            self.publish();
            return;
        }
        let mut answer = engine::get().call("ListAgentProfiles", json!({})).await;
        if let (Ok(listed), Some(seed)) = (&answer, &seed)
            && !self.seeded.replace(true)
        {
            answer = seed_profiles(listed.clone(), seed).await;
        }
        let done = answer.and_then(|snapshot| self.took(snapshot));
        if let Err(error) = done {
            eprintln!("zeron-zgx: profiles: the list didn't come: {error}");
            *self.error.borrow_mut() = Some(error);
            self.state.set(State::Error);
            self.publish();
        }
    }

    async fn write(self: &Rc<Self>, method: &str, params: Json) -> Result<Value, String> {
        crate::plugins::preview::refuse("profiles.save")?;
        let snapshot = engine::get().call(method, params).await?;
        self.took(snapshot)?;
        Ok(Value::Nil)
    }
}

/// A mock Dev's profiles (the seed's `profiles`, each saved once, by an
/// agent where it says `byAgent`), for the picker's and Settings' proofs.
async fn seed_profiles(listed: Json, seed: &Json) -> Result<Json, String> {
    let mut last = listed.clone();
    let have: Vec<String> = listed["profiles"].as_array().into_iter().flatten().filter_map(|p| p["name"].as_str().map(Into::into)).collect();
    for profile in seed["profiles"].as_array().into_iter().flatten() {
        let name = profile["name"].as_str().unwrap_or_default();
        if have.iter().any(|n| n == name) {
            continue;
        }
        let by_agent = profile["byAgent"].as_bool() == Some(true);
        last = engine::get().call("UpsertAgentProfile", json!({ "profile": profile, "byAgent": by_agent })).await?;
    }
    Ok(last)
}

fn access_id(mode: AccessMode) -> &'static str {
    match mode {
        AccessMode::Supervised => "supervised",
        AccessMode::AutoAcceptEdits => "edits",
        AccessMode::FullAccess => "full",
    }
}

fn access_of(id: &str) -> Option<AccessMode> {
    match id {
        "supervised" => Some(AccessMode::Supervised),
        "edits" => Some(AccessMode::AutoAcceptEdits),
        "full" => Some(AccessMode::FullAccess),
        _ => None,
    }
}

fn level_id(level: ReasoningLevel) -> String {
    serde_json::to_value(level).ok().and_then(|v| v.as_str().map(Into::into)).unwrap_or_default()
}

fn level_of(id: &str) -> Option<ReasoningLevel> {
    serde_json::from_value(json!(id)).ok()
}

fn harness_of(id: &str) -> Option<HarnessId> {
    serde_json::from_value(json!(id)).ok()
}

/// An agent's name, as its catalog lists it.
fn agent_label(harness: HarnessId) -> String {
    crate::chat::agent_name(harness.as_str()).unwrap_or_else(|| harness.display_name().to_owned())
}

/// A model's name, as its agent's catalog lists it (its id, not listed).
fn model_label(harness: HarnessId, model: &str) -> String {
    crate::chat::catalog_model(harness.as_str(), model)
        .and_then(|m| m["label"].as_str().map(Into::into))
        .unwrap_or_else(|| model.to_owned())
}

/// A brand's mark (the shipping app's `harness_brand_icon`), and an ACP
/// registry agent's own icon.
fn mark_fields(harness: HarnessId) -> [(&'static str, Value); 2] {
    let id = harness.as_str();
    let mark = crate::agents::glyph(id).unwrap_or_else(|| "pi".into());
    [("mark", Value::from(mark.as_str())), ("icon", crate::chat::agent_icon(id).map_or(Value::Nil, |i| Value::from(i.as_str())))]
}

fn options_record(options: &serde_json::Map<String, Json>) -> Value {
    Value::record(options.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.as_str(), Value::from(s)))).collect::<Vec<_>>())
}

fn strings(list: &[String]) -> Value {
    Value::List(list.iter().map(|s| Value::from(s.as_str())).collect::<Vec<_>>().into())
}

fn profile_value(p: &AgentProfile) -> Value {
    let mut fields = vec![
        ("id", Value::from(p.id.as_str())),
        ("name", Value::from(p.name.as_str())),
        ("harness", Value::from(p.harness.as_str())),
        ("harness_label", Value::from(agent_label(p.harness).as_str())),
        ("options", options_record(&p.model_options)),
        ("notes", Value::from(p.notes.as_str())),
        ("instructions", Value::from(p.instructions.as_str())),
        ("tools_off", strings(&p.disabled_tools)),
        ("by_agent", Value::Bool(p.by_agent)),
    ];
    fields.extend(mark_fields(p.harness));
    if let Some(model) = &p.model {
        fields.push(("model", Value::from(model.as_str())));
        fields.push(("model_label", Value::from(model_label(p.harness, model).as_str())));
    }
    if let Some(level) = p.reasoning {
        fields.push(("effort", Value::from(level_id(level).as_str())));
        fields.push(("effort_label", Value::from(crate::chat::effort_word(&level_id(level)).as_str())));
    }
    if let Some(mode) = p.access {
        fields.push(("access", Value::from(access_id(mode))));
    }
    Value::record(fields)
}

/// A draft as a script holds one (`profile_value`'s fields, `model` a bare
/// id), as the engine takes it.
fn draft_of(v: &Value) -> Result<AgentProfileDraft, String> {
    let s = |key: &str| v.field(key).map(text).filter(|t| !t.is_empty());
    let harness = s("harness").and_then(|h| harness_of(&h)).ok_or("Pick an agent for the profile.")?;
    let mut options = serde_json::Map::new();
    if let Some(Value::Map(map)) = v.field("options") {
        for (k, choice) in map.iter() {
            let choice = text(choice);
            if !choice.is_empty() {
                options.insert(k.to_string(), json!(choice));
            }
        }
    }
    Ok(AgentProfileDraft {
        name: s("name").unwrap_or_default(),
        harness,
        model: s("model"),
        reasoning: s("effort").and_then(|l| level_of(&l)),
        access: s("access").and_then(|a| access_of(&a)),
        model_options: options,
        notes: s("notes").unwrap_or_default(),
        instructions: s("instructions").unwrap_or_default(),
        disabled_tools: v.field("tools_off").map(Value::list).unwrap_or_default().iter().map(text).filter(|t| !t.is_empty()).collect(),
    })
}

fn draft_value(d: &AgentProfileDraft) -> Value {
    let mut fields = vec![
        ("name", Value::from(d.name.as_str())),
        ("harness", Value::from(d.harness.as_str())),
        ("options", options_record(&d.model_options)),
        ("notes", Value::from(d.notes.as_str())),
        ("instructions", Value::from(d.instructions.as_str())),
        ("tools_off", strings(&d.disabled_tools)),
    ];
    if let Some(model) = &d.model {
        fields.push(("model", Value::from(model.as_str())));
    }
    if let Some(level) = d.reasoning {
        fields.push(("effort", Value::from(level_id(level).as_str())));
    }
    if let Some(mode) = d.access {
        fields.push(("access", Value::from(access_id(mode))));
    }
    Value::record(fields)
}

/// A chat's config as the engine keeps it, read as the shipping app's.
pub(crate) fn config_of(config: &Json) -> Option<ChatConfig> {
    let mut c = config.clone();
    let fields = c.as_object_mut()?;
    fields.entry("sandbox").or_insert(json!("workspace-write"));
    for key in ["model", "reasoning"] {
        fields.entry(key).or_insert(Json::Null);
    }
    serde_json::from_value(c).ok()
}

/// What a script compares or saves: a chat's own config (`chat`), or a new
/// chat's picks (src/chat.rs `draft_config`: `model`, `effort`, `access`,
/// `options`, `fast`) with the profile it took (`profile`: its
/// instructions and tools, which a new chat has no picker of its own for).
fn current_config(v: &Value) -> Option<ChatConfig> {
    if let Some(chat) = v.field("chat").map(text).filter(|t| !t.is_empty()) {
        return config_of(&crate::chat::chat_config(&chat)?);
    }
    let mut config = config_of(&crate::chat::draft_config(v))?;
    if let Some(profile) = v.field("profile").map(text).and_then(|id| find(&id)) {
        config.instructions = profile.instructions;
        config.disabled_tools = profile.disabled_tools;
        config.profile_id = Some(profile.id);
    }
    Some(config)
}

/// gpui's "Save current as profile": the chat's agent, model, effort, mode,
/// options, instructions and tools.
fn current_draft(c: &ChatConfig) -> AgentProfileDraft {
    AgentProfileDraft {
        name: String::new(),
        harness: c.harness,
        model: c.model.clone(),
        reasoning: c.reasoning,
        access: Some(c.access),
        model_options: c.model_options.clone(),
        notes: String::new(),
        instructions: c.instructions.clone(),
        disabled_tools: c.disabled_tools.clone(),
    }
}

/// How `current` stands against `profile`: what differs, as the drift line
/// words it, and whether Save can write it back (gpui's `applied_profile`:
/// an option at its default counts as picked, the picker leaving it out).
fn applied_value(profile: &AgentProfile, current: &ChatConfig) -> Value {
    let mut current = current.clone();
    if let Some(model) = &current.model
        && let Some(m) = crate::chat::catalog_model(current.harness.as_str(), model)
    {
        for o in m["options"].as_array().into_iter().flatten() {
            if let (Some(id), Some(default)) = (o["id"].as_str(), o["defaultChoice"].as_str()) {
                current.model_options.entry(id.to_owned()).or_insert(json!(default));
            }
        }
    }
    let drift = profile.drift(&current);
    let option_label = |key: &str| -> String {
        current
            .model
            .as_deref()
            .and_then(|m| crate::chat::catalog_model(current.harness.as_str(), m))
            .and_then(|m| m["options"].as_array()?.iter().find(|o| o["id"].as_str() == Some(key))?["label"].as_str().map(Into::into))
            .unwrap_or_else(|| key.to_owned())
    };
    let changes: Vec<Value> = drift
        .iter()
        .map(|change| match change {
            ProfileDrift::Harness => change_value("harness", agent_label(profile.harness), agent_label(current.harness)),
            ProfileDrift::Model => change_value(
                "model",
                profile.model.as_deref().map(|m| model_label(profile.harness, m)).unwrap_or_default(),
                current.model.as_deref().map(|m| model_label(current.harness, m)).unwrap_or_default(),
            ),
            ProfileDrift::Effort => change_value(
                "effort",
                profile.reasoning.map(|l| crate::chat::effort_word(&level_id(l))).unwrap_or_default(),
                current.reasoning.map(|l| crate::chat::effort_word(&level_id(l))).unwrap_or_default(),
            ),
            ProfileDrift::Mode => change_value("mode", profile.access.map(access_id).unwrap_or_default().to_owned(), access_id(current.access).to_owned()),
            ProfileDrift::Options(keys) => Value::record([
                ("kind", Value::from("options")),
                ("names", Value::List(keys.iter().map(|k| Value::from(option_label(k).as_str())).collect::<Vec<_>>().into())),
            ]),
            ProfileDrift::Tools => Value::record([("kind", Value::from("tools"))]),
            ProfileDrift::Instructions => Value::record([("kind", Value::from("instructions"))]),
        })
        .collect();
    let mut fields = vec![
        ("id", Value::from(profile.id.as_str())),
        ("name", Value::from(profile.name.as_str())),
        ("harness", Value::from(profile.harness.as_str())),
        ("edited", Value::Bool(!drift.is_empty())),
        ("savable", Value::Bool(savable(&drift))),
        ("drift", Value::List(changes.into())),
    ];
    fields.extend(mark_fields(profile.harness));
    Value::record(fields)
}

fn change_value(kind: &str, from: String, to: String) -> Value {
    Value::record([("kind", Value::from(kind)), ("from", Value::from(from.as_str())), ("to", Value::from(to.as_str()))])
}

/// A chat's `profile` field: its profile and how the chat stands against it
/// (nil: none, or one not listed yet).
pub(crate) fn applied(config: &Json) -> Value {
    let Some(id) = config["profileId"].as_str() else { return Value::Nil };
    match (find(id), config_of(config)) {
        (Some(profile), Some(current)) => applied_value(&profile, &current),
        _ => Value::Nil,
    }
}

/// A chat's config with `profile` applied (gpui's `apply_to`), or with none
/// (`detach_profile`: "No profile" keeps the model and drops the
/// instructions), keeping whatever else the config holds.
pub(crate) fn apply(config: &Json, profile: Option<&AgentProfile>) -> Option<Json> {
    let mut parsed = config_of(config)?;
    match profile {
        Some(p) => p.apply_to(&mut parsed),
        None => zeron_proto::picker::detach_profile(&mut parsed),
    }
    let written = serde_json::to_value(&parsed).ok()?;
    let mut out = if config.is_object() { config.clone() } else { json!({}) };
    let fields = out.as_object_mut()?;
    for key in ["model", "reasoning", "modelOptions", "instructions", "profileId", "disabledTools", "access"] {
        match written.get(key) {
            Some(v) => {
                fields.insert(key.to_owned(), v.clone());
            }
            None => {
                fields.remove(key);
            }
        }
    }
    fields.insert("harness".into(), written["harness"].clone());
    if config["harness"] != written["harness"] {
        fields.remove("instanceId");
    }
    Some(out)
}

/// What a profile can turn off (gpui's Settings → Profiles → Tools): Zeron's
/// own groups (less what Settings → Tools turned off for every chat),
/// Claude Code's built-in tools and MCP servers, and the rules it holds that
/// none of those list.
fn tools_value(
    ceiling: Result<ToolSettingsState, String>,
    claude: Option<Result<HarnessToolInventory, String>>,
    off: &[String],
) -> Value {
    let locked: Vec<String> = ceiling.as_ref().map(|c| c.disabled.clone()).unwrap_or_default();
    let groups: Vec<Value> = listed_tool_groups()
        .map(|info| {
            let tools: Vec<Value> = info
                .tools()
                .iter()
                .map(|t| {
                    Value::record([
                        ("name", Value::from(t.name)),
                        ("rule", Value::from(zeron_rule(t.name).as_str())),
                        ("locked", Value::Bool(locked.iter().any(|l| l == t.name))),
                    ])
                })
                .collect();
            Value::record([
                ("id", Value::from(format!("{:?}", info.group).as_str())),
                ("title", Value::from(info.title)),
                ("tools", Value::List(tools.into())),
            ])
        })
        .collect();
    let mut fields = vec![("groups", Value::List(groups.into()))];
    if let Err(error) = &ceiling {
        fields.push(("ceiling_error", Value::from(error.as_str())));
    }
    let inventory = match &claude {
        Some(Ok(inv)) => Some(inv.clone()),
        Some(Err(_)) | None => Some(HarnessToolInventory::default()),
    };
    if let Some(claude) = claude {
        fields.push((
            "claude",
            match claude {
                Ok(inv) => Value::record([
                    ("state", Value::from("ready")),
                    (
                        "builtin",
                        Value::List(
                            inv.builtin
                                .iter()
                                .map(|g| Value::record([("id", Value::from(g.id.as_str())), ("title", Value::from(g.title.as_str())), ("summary", Value::from(g.summary.as_str())), ("rules", strings(&g.tools))]))
                                .collect::<Vec<_>>()
                                .into(),
                        ),
                    ),
                    (
                        "mcp",
                        Value::List(
                            inv.mcp_servers
                                .iter()
                                .map(|s| Value::record([("name", Value::from(s.name.as_str())), ("rule", Value::from(s.rule.as_str())), ("status", Value::from(s.status.as_str()))]))
                                .collect::<Vec<_>>()
                                .into(),
                        ),
                    ),
                ]),
                Err(error) => Value::record([("state", Value::from("error")), ("error", Value::from(error.as_str()))]),
            },
        ));
    }
    let unlisted = inventory.map(|inv| unlisted_rules(off, &inv)).unwrap_or_default();
    fields.push(("unlisted", strings(&unlisted)));
    Value::record(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, harness: HarnessId) -> AgentProfile {
        AgentProfile {
            id: id.into(),
            name: id.to_uppercase(),
            harness,
            model: None,
            reasoning: None,
            access: None,
            model_options: Default::default(),
            notes: String::new(),
            instructions: String::new(),
            disabled_tools: Vec::new(),
            by_agent: false,
        }
    }

    #[test]
    fn a_profile_lands_as_the_shipping_app_applies_it_and_keeps_the_rest_of_the_config() {
        let chat = json!({
            "harness": "claude-code", "model": "opus", "reasoning": "high",
            "modelOptions": { "fastMode": "on" }, "sandbox": "workspace-write",
            "instanceId": "work", "spaceId": "kept", "plan": true,
        });
        let mut codex = profile("codex", HarnessId::Codex);
        codex.reasoning = Some(ReasoningLevel::XHigh);
        codex.access = Some(AccessMode::Supervised);
        codex.instructions = "Be terse".into();
        codex.disabled_tools = vec!["WebFetch".into()];
        let moved = apply(&chat, Some(&codex)).unwrap();
        assert_eq!(moved["harness"], "codex");
        assert!(moved.get("model").is_none_or(Json::is_null), "another agent drops the model: {moved}");
        assert_eq!(moved["modelOptions"], json!({}));
        assert_eq!(moved["reasoning"], "xhigh");
        assert_eq!(moved["access"], "supervised");
        assert_eq!((moved["profileId"].as_str(), moved["instructions"].as_str()), (Some("codex"), Some("Be terse")));
        assert_eq!(moved["disabledTools"], json!(["WebFetch"]));
        assert!(moved.get("instanceId").is_none());
        assert_eq!((moved["spaceId"].as_str(), moved["plan"].as_bool()), (Some("kept"), Some(true)));

        let mut same = profile("same", HarnessId::ClaudeCode);
        same.model = Some("sonnet".into());
        let kept = apply(&chat, Some(&same)).unwrap();
        assert_eq!((kept["model"].as_str(), kept["reasoning"].as_str()), (Some("sonnet"), Some("high")));
        assert_eq!(kept["modelOptions"], json!({ "fastMode": "on" }));
        assert_eq!(kept["instanceId"], "work");
        assert!(kept.get("disabledTools").is_none(), "an empty tool set replaces the chat's: {kept}");

        let none = apply(&moved, None).unwrap();
        assert!(none.get("profileId").is_none() && none.get("instructions").is_none());
        assert_eq!((none["harness"].as_str(), none["reasoning"].as_str()), (Some("codex"), Some("xhigh")));
    }

    #[test]
    fn a_chat_is_compared_only_on_what_its_profile_sets() {
        let mut planner = profile("planner", HarnessId::ClaudeCode);
        planner.reasoning = Some(ReasoningLevel::High);
        planner.access = Some(AccessMode::AutoAcceptEdits);
        let chat = json!({ "harness": "claude-code", "model": "opus", "reasoning": "high", "access": "auto-accept-edits", "profileId": "planner" });
        let config = config_of(&chat).unwrap();
        let same = applied_value(&planner, &config);
        assert_eq!(same.field("edited").map(Value::truthy), Some(false));
        assert!(same.field("drift").map(Value::list).unwrap_or_default().is_empty());

        let drifted = config_of(&json!({ "harness": "claude-code", "model": "opus", "reasoning": "max", "access": "supervised", "disabledTools": ["x"] })).unwrap();
        let v = applied_value(&planner, &drifted);
        assert_eq!(v.field("edited").map(Value::truthy), Some(true));
        assert_eq!(v.field("savable").map(Value::truthy), Some(true));
        let kinds: Vec<String> = v.field("drift").map(Value::list).unwrap_or_default().iter().filter_map(|c| c.field("kind").map(text)).collect();
        assert_eq!(kinds, ["effort", "mode", "tools"]);
        let effort = &v.field("drift").map(Value::list).unwrap_or_default()[0];
        assert_eq!((effort.field("from").map(text), effort.field("to").map(text)), (Some("High".into()), Some("Max".into())));

        let other = config_of(&json!({ "harness": "codex", "model": "gpt-6", "reasoning": "high" })).unwrap();
        let kinds: Vec<String> = applied_value(&planner, &other).field("drift").map(Value::list).unwrap_or_default().iter().filter_map(|c| c.field("kind").map(text)).collect();
        assert_eq!(kinds, ["harness"]);

        planner.instructions = "Plan first".into();
        let v = applied_value(&planner, &config);
        assert_eq!(v.field("savable").map(Value::truthy), Some(false), "instructions alone show Revert, not Save");
    }

    #[test]
    fn a_draft_goes_to_the_engine_as_it_was_shown() {
        let mut p = profile("lean", HarnessId::Codex);
        p.model = Some("gpt-6".into());
        p.reasoning = Some(ReasoningLevel::XHigh);
        p.access = Some(AccessMode::FullAccess);
        p.model_options.insert("serviceTier".into(), json!("fast"));
        p.notes = "Quick fixes".into();
        p.disabled_tools = vec!["mcp__zeron__delegate_task".into()];
        let shown = profile_value(&p);
        assert_eq!(shown.field("access").map(text).as_deref(), Some("full"));
        assert_eq!(shown.field("effort").map(text).as_deref(), Some("xhigh"));
        let back = draft_of(&shown).unwrap();
        assert_eq!(back, p.draft());
        assert!(draft_of(&Value::record([("name", Value::from("x"))])).is_err());
    }
}
