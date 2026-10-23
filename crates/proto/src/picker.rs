use std::collections::BTreeSet;

use crate::{AgentProfile, AgentProfileDraft, ChatConfig, profile_line};

pub const PROFILES: &str = "Profiles";
pub const NO_PROFILE: &str = "No profile";
pub const NO_PROFILE_NOTE: &str = "Keeps the model, drops instructions";
pub const EDITED: &str = "Edited";
pub const REVERT: &str = "Revert";
pub const SAVE: &str = "Save";
pub const RETRY: &str = "Retry";

#[derive(Clone, Debug, PartialEq)]
pub enum ProfileDrift {
    Harness,
    Model,
    Effort,
    Mode,
    Options(Vec<String>),
    Tools,
    Instructions,
}

impl AgentProfile {
    pub fn apply_to(&self, config: &mut ChatConfig) {
        if config.harness != self.harness {
            config.harness = self.harness;
            config.model = None;
            config.model_options.clear();
        }
        if let Some(model) = &self.model {
            config.model = Some(model.clone());
        }
        if let Some(level) = self.reasoning {
            config.reasoning = Some(level);
        }
        if let Some(mode) = self.permission_mode {
            config.permission_mode = mode;
        }
        if !self.model_options.is_empty() {
            config.model_options = self.model_options.clone();
        }
        config.instructions = self.instructions.clone();
        config.disabled_tools = self.disabled_tools.clone();
        config.profile_id = Some(self.id.clone());
    }

    pub fn draft(&self) -> AgentProfileDraft {
        AgentProfileDraft {
            name: self.name.clone(),
            harness: self.harness,
            model: self.model.clone(),
            reasoning: self.reasoning,
            permission_mode: self.permission_mode,
            model_options: self.model_options.clone(),
            notes: self.notes.clone(),
            instructions: self.instructions.clone(),
            disabled_tools: self.disabled_tools.clone(),
        }
    }

    pub fn line(&self, model_label: Option<&str>) -> String {
        profile_line(
            self.harness.display_name(),
            model_label.or(self.model.as_deref()),
            self.permission_mode,
            self.reasoning,
            self.disabled_tools.len(),
        )
    }

    pub fn drift(&self, current: &ChatConfig) -> Vec<ProfileDrift> {
        if current.harness != self.harness {
            return vec![ProfileDrift::Harness];
        }
        let mut drift = Vec::new();
        if self
            .model
            .as_ref()
            .is_some_and(|model| current.model.as_ref() != Some(model))
        {
            drift.push(ProfileDrift::Model);
        }
        if self
            .reasoning
            .is_some_and(|level| current.reasoning != Some(level))
        {
            drift.push(ProfileDrift::Effort);
        }
        if self
            .permission_mode
            .is_some_and(|mode| current.permission_mode != mode)
        {
            drift.push(ProfileDrift::Mode);
        }
        let options: Vec<String> = self
            .model_options
            .iter()
            .filter(|(key, value)| current.model_options.get(*key) != Some(value))
            .map(|(key, _)| key.clone())
            .collect();
        if !options.is_empty() {
            drift.push(ProfileDrift::Options(options));
        }
        if self.disabled_tools.iter().collect::<BTreeSet<_>>()
            != current.disabled_tools.iter().collect::<BTreeSet<_>>()
        {
            drift.push(ProfileDrift::Tools);
        }
        if self.instructions != current.instructions {
            drift.push(ProfileDrift::Instructions);
        }
        drift
    }

    pub fn absorb(&self, current: &ChatConfig) -> AgentProfileDraft {
        let mut draft = self.draft();
        let harness_moved = current.harness != self.harness;
        draft.harness = current.harness;
        if self.model.is_some() || harness_moved {
            draft.model = current.model.clone();
        }
        if self.reasoning.is_some() {
            draft.reasoning = current.reasoning;
        }
        if self.permission_mode.is_some() {
            draft.permission_mode = Some(current.permission_mode);
        }
        draft.model_options = if harness_moved {
            Default::default()
        } else {
            self.model_options
                .keys()
                .filter_map(|key| {
                    current
                        .model_options
                        .get(key)
                        .map(|value| (key.clone(), value.clone()))
                })
                .collect()
        };
        draft.disabled_tools = current.disabled_tools.clone();
        draft
    }

    pub fn drift_line(
        &self,
        drift: &[ProfileDrift],
        current: &ChatConfig,
        model_label: impl Fn(&str) -> String,
        option_label: impl Fn(&str) -> String,
    ) -> String {
        drift
            .iter()
            .map(|change| match change {
                ProfileDrift::Harness => format!(
                    "{} → {}",
                    self.harness.display_name(),
                    current.harness.display_name()
                ),
                ProfileDrift::Model => format!(
                    "{} → {}",
                    self.model.as_deref().map(&model_label).unwrap_or_default(),
                    current
                        .model
                        .as_deref()
                        .map(&model_label)
                        .unwrap_or_default()
                ),
                ProfileDrift::Effort => format!(
                    "effort {} → {}",
                    self.reasoning.map(|l| l.label()).unwrap_or_default(),
                    current.reasoning.map(|l| l.label()).unwrap_or_default()
                ),
                ProfileDrift::Mode => format!(
                    "{} → {}",
                    self.permission_mode
                        .map(|m| m.short_label())
                        .unwrap_or_default(),
                    current.permission_mode.short_label()
                ),
                ProfileDrift::Options(keys) => {
                    let names = keys
                        .iter()
                        .map(|key| option_label(key))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{names} changed")
                }
                ProfileDrift::Tools => "tools changed".to_string(),
                ProfileDrift::Instructions => "instructions changed".to_string(),
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

pub fn detach_profile(config: &mut ChatConfig) {
    config.profile_id = None;
    config.instructions.clear();
}

pub fn savable(drift: &[ProfileDrift]) -> bool {
    drift
        .iter()
        .any(|change| *change != ProfileDrift::Instructions)
}

pub fn catalog_error_text(message: &str) -> &str {
    message
        .strip_prefix("model discovery ")
        .and_then(|rest| rest.split_once(": "))
        .filter(|(code, _)| {
            matches!(
                *code,
                "timeout" | "failed" | "missing_executable" | "auth_required"
            )
        })
        .map_or(message, |(_, text)| text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HarnessId, PermissionMode, ReasoningLevel, SandboxLevel};

    fn profile(name: &str) -> AgentProfile {
        AgentProfile {
            id: name.to_lowercase(),
            name: name.into(),
            harness: HarnessId::Codex,
            model: None,
            reasoning: None,
            permission_mode: None,
            model_options: Default::default(),
            notes: String::new(),
            instructions: String::new(),
            by_agent: false,
            disabled_tools: Vec::new(),
        }
    }

    fn config(harness: HarnessId) -> ChatConfig {
        ChatConfig {
            harness,
            model: None,
            reasoning: None,
            model_options: Default::default(),
            sandbox: SandboxLevel::WorkspaceWrite,
            permission_mode: PermissionMode::Full,
            instructions: String::new(),
            profile_id: None,
            instance_id: None,
            disabled_tools: Vec::new(),
        }
    }

    fn tools(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn options(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn a_pick_only_overrides_what_the_profile_sets() {
        let mut config = ChatConfig {
            model: Some("opus".into()),
            reasoning: Some(ReasoningLevel::High),
            model_options: options(serde_json::json!({ "context": "1m" })),
            ..config(HarnessId::ClaudeCode)
        };
        let mut same = profile("Same");
        same.harness = HarnessId::ClaudeCode;
        same.permission_mode = Some(PermissionMode::Ask);
        same.apply_to(&mut config);
        assert_eq!(config.model.as_deref(), Some("opus"));
        assert_eq!(config.reasoning, Some(ReasoningLevel::High));
        assert_eq!(config.permission_mode, PermissionMode::Ask);
        assert_eq!(config.model_options["context"], "1m");

        let mut codex = profile("Codex");
        codex.model = Some("gpt-5.6".into());
        codex.apply_to(&mut config);
        assert_eq!(config.harness, HarnessId::Codex);
        assert_eq!(config.model.as_deref(), Some("gpt-5.6"));
        assert!(config.model_options.is_empty());
        assert_eq!(config.reasoning, Some(ReasoningLevel::High));
        assert_eq!(config.profile_id.as_deref(), Some("codex"));

        config.instructions = "Be terse".into();
        detach_profile(&mut config);
        assert_eq!(
            (config.profile_id, config.instructions),
            (None, String::new())
        );
    }

    #[test]
    fn a_pick_replaces_the_tool_set_even_when_empty() {
        let mut config = ChatConfig {
            disabled_tools: tools(&["WebFetch"]),
            ..config(HarnessId::ClaudeCode)
        };
        let mut lean = profile("Lean");
        lean.harness = HarnessId::ClaudeCode;
        lean.disabled_tools = tools(&["mcp__xcode", "mcp__zeron__delegate_task"]);
        lean.apply_to(&mut config);
        assert_eq!(
            config.disabled_tools,
            tools(&["mcp__xcode", "mcp__zeron__delegate_task"])
        );
        let mut full = profile("Full");
        full.harness = HarnessId::ClaudeCode;
        full.apply_to(&mut config);
        assert!(config.disabled_tools.is_empty());
        lean.harness = HarnessId::Codex;
        lean.apply_to(&mut config);
        assert_eq!(config.disabled_tools, lean.disabled_tools);
    }

    #[test]
    fn drift_compares_only_what_the_profile_sets() {
        let mut planner = profile("Planner");
        planner.harness = HarnessId::ClaudeCode;
        planner.reasoning = Some(ReasoningLevel::High);
        planner.model_options = options(serde_json::json!({ "context": "1m" }));
        let mut current = ChatConfig {
            model: Some("opus".into()),
            reasoning: Some(ReasoningLevel::High),
            model_options: options(serde_json::json!({ "context": "1m", "fast": "on" })),
            profile_id: Some("planner".into()),
            ..config(HarnessId::ClaudeCode)
        };
        assert!(planner.drift(&current).is_empty());

        current.reasoning = Some(ReasoningLevel::Max);
        current.model_options.remove("context");
        let drift = planner.drift(&current);
        assert_eq!(
            drift,
            [
                ProfileDrift::Effort,
                ProfileDrift::Options(vec!["context".into()])
            ]
        );
        assert!(savable(&drift));
        assert_eq!(
            planner.drift_line(&drift, &current, str::to_owned, |_| "Context".into()),
            "effort High → Max · Context changed"
        );
        let saved = planner.absorb(&current);
        assert_eq!(saved.reasoning, Some(ReasoningLevel::Max));
        assert!(saved.model.is_none());
        assert!(saved.permission_mode.is_none());
        assert!(saved.model_options.is_empty());

        current.harness = HarnessId::Codex;
        assert_eq!(planner.drift(&current), [ProfileDrift::Harness]);
        assert!(!savable(&[ProfileDrift::Instructions]));
    }

    #[test]
    fn drift_compares_tool_sets_and_save_absorbs_them() {
        let mut lean = profile("Lean");
        lean.harness = HarnessId::ClaudeCode;
        lean.disabled_tools = tools(&["WebFetch", "mcp__xcode"]);
        let mut current = ChatConfig {
            profile_id: Some("lean".into()),
            disabled_tools: tools(&["mcp__xcode", "WebFetch"]),
            ..config(HarnessId::ClaudeCode)
        };
        assert!(lean.drift(&current).is_empty());
        current.disabled_tools.push("NotebookEdit".into());
        assert_eq!(lean.drift(&current), [ProfileDrift::Tools]);
        assert_eq!(lean.absorb(&current).disabled_tools, current.disabled_tools);
        current.disabled_tools.clear();
        assert_eq!(lean.drift(&current), [ProfileDrift::Tools]);
        lean.disabled_tools.clear();
        assert!(lean.drift(&current).is_empty());
    }

    #[test]
    fn a_failed_catalog_shows_only_the_reason() {
        assert_eq!(
            catalog_error_text("model discovery timeout: timed out after 20s"),
            "timed out after 20s"
        );
        assert_eq!(
            catalog_error_text("model discovery weird: x"),
            "model discovery weird: x"
        );
    }
}
