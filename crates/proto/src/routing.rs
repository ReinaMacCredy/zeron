//! Settings > Routing: which harness and model each delegated role runs on,
//! and what happens when the routed harness is out of quota. Owned by the
//! engine; the desktop and the phone edit it over `Get/SetRoutingSettings`.
use std::collections::BTreeMap;

use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::{HarnessId, RouteTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Worker,
    Validator,
    Research,
    Design,
    General,
}

impl Role {
    pub const ALL: [Role; 5] = [
        Role::Worker,
        Role::Validator,
        Role::Research,
        Role::Design,
        Role::General,
    ];

    pub fn parse(name: &str) -> Option<Role> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "worker" | "implementation" => Role::Worker,
            "validator" | "review" | "test" => Role::Validator,
            "research" => Role::Research,
            "design" => Role::Design,
            "general" => Role::General,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::Worker => "worker",
            Role::Validator => "validator",
            Role::Research => "research",
            Role::Design => "design",
            Role::General => "general",
        }
    }

    pub fn names() -> String {
        Role::ALL.map(Role::name).join(", ")
    }

    pub fn summary(self) -> &'static str {
        match self {
            Role::Worker => "Write and change code, incl. tests",
            Role::Validator => "Review a diff or finished step, run tests and QA",
            Role::Research => "Read code, docs and the web, report back",
            Role::Design => "Shape an approach or a UI before code",
            Role::General => "Anything else",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoleRoute {
    #[default]
    Off,
    SameAsParent,
    Target(RouteTarget),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutOfQuota {
    #[default]
    Native,
    NextInList,
    Wait,
    Ask,
}

impl OutOfQuota {
    pub const ALL: [OutOfQuota; 4] = [
        OutOfQuota::Native,
        OutOfQuota::NextInList,
        OutOfQuota::Wait,
        OutOfQuota::Ask,
    ];

    pub fn label(self) -> &'static str {
        match self {
            OutOfQuota::Native => "Native sub-agent",
            OutOfQuota::NextInList => "Next in fallback list",
            OutOfQuota::Wait => "Wait for reset",
            OutOfQuota::Ask => "Ask me",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            OutOfQuota::Native => {
                "Run the work as the chat's own sub-agent, on the chat's model. If native \
                 sub-agents are Off, open a Zeron child on the chat's model instead. Mid-run: \
                 switch the child to the chat's model."
            }
            OutOfQuota::NextInList => "Try the next harness below that still has quota.",
            OutOfQuota::Wait => "Start or resume when the window resets, like auto-resume today.",
            OutOfQuota::Ask => "Stop and ask in the chat, with the options above.",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MidRun {
    Ask,
    After10s,
    #[default]
    After30s,
}

impl MidRun {
    pub const ALL: [(MidRun, &'static str); 3] = [
        (MidRun::Ask, "Ask"),
        (MidRun::After10s, "10 s"),
        (MidRun::After30s, "30 s"),
    ];

    pub fn delay_ms(self) -> Option<i64> {
        match self {
            MidRun::Ask => None,
            MidRun::After10s => Some(10_000),
            MidRun::After30s => Some(30_000),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativePolicy {
    #[default]
    SameModelOnly,
    Always,
    Off,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RoutingSettings {
    pub roles: BTreeMap<Role, RoleRoute>,
    pub out_of_quota: OutOfQuota,
    pub fallback: Vec<RouteTarget>,
    pub mid_run: MidRun,
    pub native: NativePolicy,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub model_notes: BTreeMap<String, String>,
}

pub const DEFAULT_MODEL_NOTES: [(&str, &str); 8] = [
    (
        "claude-haiku-5-5",
        "Cheapest. Search, logs, summaries, classification.",
    ),
    (
        "claude-sonnet-5-5",
        "Low cost. Well-scoped coding from a spec, bug fixes, repeated review and investigation. Start at medium.",
    ),
    (
        "claude-opus-5-5",
        "Mid cost. Everyday engineering, multi-part features, UI work, computer use and screenshots. Start at medium.",
    ),
    (
        "claude-fable-5-2",
        "High cost. Major refactors, architecture, leading many agents, deep research. Start at medium.",
    ),
    (
        "gpt-6-luna",
        "Cheapest. Narrow, high-volume work with a checkable result: search, logs, summaries.",
    ),
    (
        "gpt-6.1-sol",
        "Low cost. Complex coding and long agent runs; the default GPT coding model. Start at medium.",
    ),
    (
        "gpt-6.1-terra",
        "Low cost. Code review, screenshots and visual checks, precise computer use, writing. Start at medium.",
    ),
    (
        "gpt-6-astra",
        "Highest cost. Only when Sol at high is not enough.",
    ),
];

pub fn default_model_note(model: &str) -> Option<&'static str> {
    DEFAULT_MODEL_NOTES
        .iter()
        .find(|(id, _)| *id == model)
        .map(|(_, note)| *note)
}

impl RoutingSettings {
    pub fn model_note<'a>(&'a self, model: &str) -> Option<&'a str> {
        self.model_notes
            .get(model)
            .map(String::as_str)
            .or_else(|| default_model_note(model))
            .filter(|note| !note.trim().is_empty())
    }

    pub fn route(&self, role: Role) -> &RoleRoute {
        static OFF: RoleRoute = RoleRoute::Off;
        self.roles.get(&role).unwrap_or(&OFF)
    }

    pub fn targets(&self) -> impl Iterator<Item = &RouteTarget> {
        self.roles
            .values()
            .filter_map(|route| match route {
                RoleRoute::Target(target) => Some(target),
                _ => None,
            })
            .chain(&self.fallback)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QuotaState {
    pub known: bool,
    pub used_fraction: Option<f64>,
    pub resets_at: Option<i64>,
    pub exhausted: bool,
}

/// `PreviewRoutes` reply rows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Fallback {
    pub target: Option<RouteTarget>,
    pub native: bool,
    pub reason: String,
    pub resets_at: Option<i64>,
    pub multiplier: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Row {
    pub role: String,
    pub source: String,
    pub target: Option<RouteTarget>,
    pub multiplier: Option<f64>,
    pub quota: QuotaState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Fallback>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preview {
    pub base: Option<RouteTarget>,
    pub rows: Vec<Row>,
    pub guidance: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub quotas: BTreeMap<String, QuotaState>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub model_notes: BTreeMap<String, String>,
}

impl Row {
    pub fn fell_back(&self) -> bool {
        self.source == "fell_back" || self.fallback.is_some()
    }

    /// "Falling back: ..." under a role whose route is out of quota.
    pub fn fallback_line(&self) -> Option<String> {
        if !self.fell_back() {
            return None;
        }
        Some(match &self.fallback {
            Some(fallback) => {
                let line = match (&fallback.target, fallback.native) {
                    (Some(target), true) => format!(
                        "Falling back: {} native sub-agent",
                        short_name(target.harness)
                    ),
                    (Some(target), false) => format!("Falling back: {}", target_label(target)),
                    (None, _) => format!("Out of quota: {}", fallback.reason),
                };
                match until(fallback.resets_at) {
                    Some(until) => format!("{line} {until}"),
                    None => line,
                }
            }
            None => {
                let to = self
                    .target
                    .as_ref()
                    .map_or_else(|| "the chat's model".to_string(), target_label);
                match &self.reason {
                    Some(reason) => format!("Falling back: {to} ({reason})"),
                    None => format!("Falling back: {to}"),
                }
            }
        })
    }
}

pub fn short_name(harness: HarnessId) -> &'static str {
    let name = harness.display_name();
    name.split(' ').next().unwrap_or(name)
}

pub fn target_label(target: &RouteTarget) -> String {
    match &target.model {
        Some(model) => format!("{} {model}", target.harness.display_name()),
        None => target.harness.display_name().to_string(),
    }
}

pub fn multiplier_text(multiplier: f64) -> String {
    let text = format!("{multiplier:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text}x")
}

fn until(resets_at: Option<i64>) -> Option<String> {
    let at = Utc.timestamp_millis_opt(resets_at?).single()?;
    Some(format!(
        "until {}",
        at.with_timezone(&chrono::Local).format("%-I:%M %p")
    ))
}
