//! Model catalog + effort mapping for Claude Code, ported from zeron's
//! `packages/harness/src/claude.ts` (which itself mirrors Claude Code's own
//! picker via t3code's catalog).
//!
//! Runtime initialize discovery appends concrete model ids to this curated list.
//! Curated rows retain their labels, effort ladders, and option sets because the
//! CLI can under-report supported modes. The shared initialize probe also supplies
//! slash commands and is cached by credential and binary context.

pub use zeron_proto::model_catalog::claude::static_models;
pub(crate) use zeron_proto::model_catalog::claude::{FULL_LADDER, context_window};
use zeron_proto::{Model, ReasoningLevel};

/// The ultrathink directive rides every user message as a prompt prefix — that
/// is how the mode actually works in Claude Code (a prompt convention, not an
/// effort flag). Applied to the initial prompt AND every steer.
pub(crate) const ULTRATHINK_PREFIX: &str = "Ultrathink:\n";

pub(crate) fn apply_ultrathink(reasoning: Option<ReasoningLevel>, text: &str) -> String {
    if reasoning == Some(ReasoningLevel::Ultrathink)
        && zeron_proto::invocation::leading_command(text).is_none()
    {
        format!("{ULTRATHINK_PREFIX}{text}")
    } else {
        text.to_owned()
    }
}

fn contains_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

/// Models whose CLI accepts `xhigh` natively; elsewhere it clamps to `max`
/// (mirroring Claude Code's own normalization). Substring port of claude.ts's
/// `/fable-5|opus-4-[7-9]|opus-[5-9]|sonnet-[5-9]/`.
pub(crate) fn supports_xhigh(model: &str) -> bool {
    contains_any(
        model,
        &[
            "fable-5", "opus-4-7", "opus-4-8", "opus-4-9", "opus-5", "opus-6", "opus-7", "opus-8",
            "opus-9", "sonnet-5", "sonnet-6", "sonnet-7", "sonnet-8", "sonnet-9",
        ],
    )
}

/// Map the unified level to the `--effort` flag value the CLI accepts for this
/// model. The special modes don't translate directly: `ultrathink` is a prompt
/// prefix (no flag), `ultracode` runs as `xhigh` plus the ultracode setting,
/// and `ultra` is a Codex-only tier (Claude tops out at `max`).
pub(crate) fn to_effort(
    reasoning: Option<ReasoningLevel>,
    model: Option<&str>,
) -> Option<&'static str> {
    let base = match reasoning? {
        ReasoningLevel::Ultrathink => return None,
        ReasoningLevel::Minimal | ReasoningLevel::Low => "low",
        ReasoningLevel::Medium => "medium",
        ReasoningLevel::High => "high",
        ReasoningLevel::XHigh | ReasoningLevel::Ultracode => "xhigh",
        ReasoningLevel::Max | ReasoningLevel::Ultra => "max",
    };
    if base == "xhigh" && !model.is_some_and(supports_xhigh) {
        return Some("max");
    }
    Some(base)
}

/// The built-ins a chat may run without, grouped as a person thinks of
/// them; names from the CLI's `init` frame (2.1.293). Bash, Read, Edit,
/// Write and ToolSearch stay on: a coding chat without them cannot work.
pub(crate) fn builtin_tool_groups() -> Vec<zeron_proto::tools::BuiltinToolGroup> {
    [
        (
            "web",
            "Web",
            "Search the web and fetch pages.",
            &["WebSearch", "WebFetch"][..],
        ),
        (
            "agents",
            "Sub-agents",
            "Spawn and message sub-agents.",
            &["Task", "SendMessage", "ListAgents"],
        ),
        (
            "tasks",
            "Task list",
            "Track a todo list.",
            &["TaskCreate", "TaskGet", "TaskList", "TaskUpdate"],
        ),
        (
            "schedule",
            "Scheduling",
            "Cron jobs, wakeups and remote triggers.",
            &[
                "CronCreate",
                "CronDelete",
                "CronList",
                "ScheduleWakeup",
                "RemoteTrigger",
            ],
        ),
        (
            "workflows",
            "Workflows",
            "Multi-agent workflow scripts.",
            &["Workflow"],
        ),
        (
            "worktrees",
            "Worktrees",
            "Enter and leave git worktrees.",
            &["EnterWorktree", "ExitWorktree"],
        ),
        ("skills", "Skills", "Load skills on demand.", &["Skill"]),
        (
            "lsp",
            "Language server",
            "Code intelligence queries.",
            &["LSP"],
        ),
        (
            "notebooks",
            "Notebooks",
            "Edit Jupyter notebooks.",
            &["NotebookEdit"],
        ),
        (
            "notifications",
            "Push notifications",
            "Notify your devices.",
            &["PushNotification"],
        ),
    ]
    .into_iter()
    .map(
        |(id, title, summary, tools)| zeron_proto::tools::BuiltinToolGroup {
            id: id.into(),
            title: title.into(),
            summary: summary.into(),
            tools: tools.iter().map(|t| (*t).into()).collect(),
        },
    )
    .collect()
}

/// The curated model list, mirroring claude.ts's `claudeEffortsFor` /
/// `claudeOptionsFor` ladders: full ladder (through ultracode/ultrathink) on
/// Fable 5, `max`-topped ladders on Opus/Sonnet/Haiku 5.5, no efforts but a
/// thinking toggle on Haiku 4.5; context-window select on the long-context families and
/// fast mode on Opus 4.5+.
///
/// `pub`: besides the discovery-side enrichment here, the UI's display-side
/// normalization borrows these labels so alias rows served by older engines
/// still read with their version numbers ("Opus 5.5", not "Opus").
pub(crate) fn configured_models(config_dir: &std::path::Path) -> Vec<Model> {
    models_with_settings(&config_dir.join("settings.json"))
}

fn models_with_settings(path: &std::path::Path) -> Vec<Model> {
    let mut models = static_models();
    let settings = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .unwrap_or_default();
    let ids = std::iter::once(settings.get("model")).chain(
        [
            "ANTHROPIC_MODEL",
            "ANTHROPIC_SMALL_FAST_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        ]
        .into_iter()
        .map(|key| settings.get("env").and_then(|env| env.get(key))),
    );
    for id in ids
        .flatten()
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        if !models.iter().any(|m| m.id == id) {
            models.push(Model {
                id: id.into(),
                label: id.into(),
                description: None,
                reasoning_levels: FULL_LADDER.to_vec(),
                options: vec![],
            });
        }
    }
    models
}

#[cfg(test)]
#[test]
fn settings_models_keep_manifest_and_full_ladder() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(
        &path,
        serde_json::json!({"model":"gateway/model", "env": {
            "ANTHROPIC_MODEL":"gateway/model", "ANTHROPIC_SMALL_FAST_MODEL":"fast",
            "ANTHROPIC_DEFAULT_OPUS_MODEL":"opus", "ANTHROPIC_DEFAULT_SONNET_MODEL":"sonnet",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL":"haiku"
        }})
        .to_string(),
    )
    .unwrap();
    let models = models_with_settings(&path);
    assert_eq!(&models[..static_models().len()], static_models());
    assert_eq!(models.len(), static_models().len() + 5);
    for model in &models[static_models().len()..] {
        assert_eq!(model.label, model.id);
        assert_eq!(model.reasoning_levels, FULL_LADDER);
    }
    std::fs::write(&path, "invalid").unwrap();
    assert_eq!(models_with_settings(&path), static_models());
}

/// Overlay only new concrete model IDs; curated metadata remains authoritative.
pub(super) fn with_discovered_models(
    mut models: Vec<Model>,
    response: &serde_json::Value,
) -> Result<Vec<Model>, crate::HarnessError> {
    let entries = response
        .get("response")
        .and_then(|v| v.get("models"))
        .and_then(serde_json::Value::as_array)
        .filter(|entries| !entries.is_empty())
        .ok_or_else(|| {
            crate::HarnessError::Protocol("Claude returned an empty model catalog".into())
        })?;
    let mut default = None;
    let mut valid = false;
    for entry in entries {
        let text = |key: &str| {
            entry
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
        };
        let Some(id) = text("resolvedModel").or_else(|| text("value")) else {
            continue;
        };
        // Bare aliases cannot become persisted selections. Prefer resolvedModel.
        if matches!(
            id.strip_suffix("[1m]").unwrap_or(id),
            "default" | "opus" | "sonnet" | "haiku" | "fable"
        ) {
            continue;
        }
        valid = true;
        // The CLI lists some models by dated snapshot ("claude-haiku-4-5-20251001");
        // fold those into the curated undated row instead of duplicating it.
        let id = snapshot_base(id)
            .filter(|base| models.iter().any(|model| model.id == *base))
            .unwrap_or(id);
        if text("value") == Some("default") {
            default = Some(id.to_owned());
        }
        if models.iter().any(|model| model.id == id) {
            continue;
        }
        let mut ladder = Vec::new();
        for effort in entry
            .get("supportedEffortLevels")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
        {
            let level = match effort {
                "low" => ReasoningLevel::Low,
                "medium" => ReasoningLevel::Medium,
                "high" => ReasoningLevel::High,
                "xhigh" => ReasoningLevel::XHigh,
                "max" => ReasoningLevel::Max,
                _ => continue,
            };
            if !ladder.contains(&level) {
                ladder.push(level);
            }
        }
        if ladder.contains(&ReasoningLevel::XHigh) {
            ladder.extend([ReasoningLevel::Ultracode, ReasoningLevel::Ultrathink]);
        }
        models.push(Model {
            id: id.into(),
            label: text("displayName").unwrap_or(id).into(),
            description: text("description").map(str::to_owned),
            reasoning_levels: ladder,
            options: vec![],
        });
    }
    if !valid {
        return Err(crate::HarnessError::Protocol(
            "Claude returned an empty model catalog".into(),
        ));
    }
    if let Some(default) = default {
        // The picker folds long-context duplicates into their curated base.
        // Put that base immediately behind the concrete default so folding
        // keeps the CLI's default family first without changing curated rows.
        if let Some(base) = default.strip_suffix("[1m]")
            && let Some(index) = models.iter().position(|m| m.id == base)
        {
            let model = models.remove(index);
            models.insert(0, model);
        }
        if let Some(index) = models.iter().position(|m| m.id == default) {
            let model = models.remove(index);
            models.insert(0, model);
        }
    }
    Ok(models)
}

/// `claude-haiku-4-5-20251001` -> `claude-haiku-4-5`.
fn snapshot_base(id: &str) -> Option<&str> {
    let (base, date) = id.rsplit_once('-')?;
    (date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit())).then_some(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_window_tokens_covers_every_choice() {
        for choice in context_window().choices {
            assert!(
                zeron_proto::model_options::context_window_tokens(&choice.id).is_some(),
                "{}",
                choice.id
            );
        }
    }

    #[test]
    fn effort_maps_special_modes() {
        assert_eq!(to_effort(None, None), None);
        assert_eq!(to_effort(Some(ReasoningLevel::Ultrathink), None), None);
        assert_eq!(
            to_effort(Some(ReasoningLevel::Minimal), Some("claude-fable-5")),
            Some("low")
        );
        assert_eq!(
            to_effort(Some(ReasoningLevel::Ultra), Some("claude-fable-5")),
            Some("max")
        );
        // ultracode -> xhigh where supported…
        assert_eq!(
            to_effort(Some(ReasoningLevel::Ultracode), Some("claude-fable-5")),
            Some("xhigh")
        );
        // …and xhigh clamps to max elsewhere.
        assert_eq!(
            to_effort(Some(ReasoningLevel::XHigh), Some("claude-opus-4-5")),
            Some("max")
        );
        assert_eq!(to_effort(Some(ReasoningLevel::XHigh), None), Some("max"));
    }

    #[test]
    fn dated_snapshots_fold_into_curated_rows() {
        let response = serde_json::json!({"response": {"models": [
            {"value": "claude-haiku-4-5-20251001", "resolvedModel": "claude-haiku-4-5-20251001",
             "displayName": "Haiku 4.5", "description": "Fastest for quick answers"},
            {"value": "claude-haiku-9-9-20990101", "displayName": "Haiku 9.9"},
        ]}});
        let models = with_discovered_models(static_models(), &response).unwrap();
        assert_eq!(models.iter().filter(|m| m.label == "Haiku 4.5").count(), 1);
        // Snapshots with no curated base still surface as-is.
        assert!(models.iter().any(|m| m.id == "claude-haiku-9-9-20990101"));
    }

    #[test]
    fn xhigh_family_matching() {
        assert!(supports_xhigh("claude-fable-5"));
        assert!(supports_xhigh("claude-fable-5-1"));
        assert!(supports_xhigh("claude-opus-5"));
        assert!(supports_xhigh("claude-opus-5-5"));
        assert!(supports_xhigh("claude-opus-5-5[1m]"));
        assert!(supports_xhigh("claude-opus-4-7-20260101"));
        assert!(!supports_xhigh("claude-opus-4-5"));
        assert!(!supports_xhigh("claude-sonnet-4-5"));
    }

    #[test]
    fn ultrathink_preserves_leading_commands_and_arguments() {
        for command in ["/compact", "/review focus on tests"] {
            for prefix in ["", " ", "   ", "\n", "\r\n  "] {
                let text = format!("{prefix}{command}");
                assert_eq!(
                    apply_ultrathink(Some(ReasoningLevel::Ultrathink), &text),
                    text
                );
            }
        }
        for literal in [
            "    /compact",
            "\t/compact",
            "\n    /compact",
            "\u{a0}/compact",
        ] {
            assert_eq!(
                apply_ultrathink(Some(ReasoningLevel::Ultrathink), literal),
                format!("{ULTRATHINK_PREFIX}{literal}")
            );
        }
    }

    #[test]
    fn ultrathink_prefixes_prompt() {
        assert_eq!(
            apply_ultrathink(Some(ReasoningLevel::Ultrathink), "do it"),
            "Ultrathink:\ndo it"
        );
        assert_eq!(
            apply_ultrathink(Some(ReasoningLevel::Max), "do it"),
            "do it"
        );
    }
}
