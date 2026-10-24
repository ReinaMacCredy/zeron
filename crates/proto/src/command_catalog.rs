use crate::app_commands::{BTW_COMMAND, COMPACT_COMMAND, COMPUTER_USE_COMMAND, SIDECHAT_COMMAND};
use crate::invocation::{
    Invocation, Skill, native_skill_identity, valid_invocation_name, valid_skill_command_name,
    valid_skill_path,
};
use crate::{HarnessId, SlashCommand, SlashCommandKind};

/// Zeron's `/compact` rides every harness's list; one that advertises its own
/// keeps it, so the popup never shows the name twice.
pub fn with_compact_entry(mut commands: Vec<SlashCommand>) -> Vec<SlashCommand> {
    if commands.iter().any(|c| c.name == COMPACT_COMMAND) {
        return commands;
    }
    commands.insert(
        0,
        SlashCommand {
            name: COMPACT_COMMAND.into(),
            description: "Free up context by summarizing the conversation".into(),
            input_hint: Some("focus".into()),
            kind: SlashCommandKind::BuiltIn,
        },
    );
    commands
}

/// Mirrors `Harness::computer_tools_reachable`: no Opencode, Cursor, Pi or Antigravity.
fn offers_computer_use(harness: HarnessId) -> bool {
    matches!(
        harness,
        HarnessId::ClaudeCode
            | HarnessId::Codex
            | HarnessId::Devin
            | HarnessId::Grok
            | HarnessId::Hermes
    )
}

/// Zeron owns `/computer-use` even where the harness advertises a native one.
pub fn with_computer_use_entry(
    harness: HarnessId,
    mut commands: Vec<SlashCommand>,
) -> Vec<SlashCommand> {
    if !offers_computer_use(harness) {
        return commands;
    }
    commands.retain(|c| c.name != COMPUTER_USE_COMMAND);
    commands.insert(
        0,
        SlashCommand {
            name: COMPUTER_USE_COMMAND.into(),
            description: "Use Zeron Computer for this request only".into(),
            input_hint: Some("request".into()),
            kind: SlashCommandKind::BuiltIn,
        },
    );
    commands
}

/// Same ride as `/compact`: zeron's `/btw` and `/sidechat` join every
/// harness's list unless the harness already advertises that name.
pub fn with_sidechat_entry(mut commands: Vec<SlashCommand>) -> Vec<SlashCommand> {
    for name in [SIDECHAT_COMMAND, BTW_COMMAND] {
        if commands.iter().any(|c| c.name == name) {
            continue;
        }
        commands.insert(
            0,
            SlashCommand {
                name: name.into(),
                description: "Ask about this chat on the side, without touching it".into(),
                input_hint: Some("prompt".into()),
                kind: SlashCommandKind::BuiltIn,
            },
        );
    }
    commands
}

// Skills complete under `$`, so `/` keeps only the agent's built-ins.
pub fn with_zeron_commands(
    harness: Option<HarnessId>,
    mut commands: Vec<SlashCommand>,
) -> Vec<SlashCommand> {
    commands.retain(|command| command.kind == SlashCommandKind::BuiltIn);
    let commands = match harness {
        Some(harness) => with_computer_use_entry(harness, commands),
        None => commands,
    };
    let commands = with_sidechat_entry(with_compact_entry(commands));
    if harness == Some(HarnessId::ClaudeCode) {
        with_terminal_commands(commands)
    } else {
        commands
    }
}

pub const TERMINAL_ONLY_DESCRIPTION: &str = "Runs only in Claude Code's terminal";

// Headless Claude Code leaves terminal-only commands out of its catalog, so typing one finds
// nothing; Zeron's own command keeps a shared name such as /resume.
pub fn with_terminal_commands(mut commands: Vec<SlashCommand>) -> Vec<SlashCommand> {
    let zeron = WorkspaceCommand::catalog();
    for name in crate::app_commands::TERMINAL_ONLY_COMMANDS {
        if !commands.iter().any(|c| c.name == *name) && !zeron.iter().any(|z| z.1 == *name) {
            commands.push(SlashCommand {
                name: (*name).into(),
                description: TERMINAL_ONLY_DESCRIPTION.into(),
                input_hint: None,
                kind: SlashCommandKind::BuiltIn,
            });
        }
    }
    commands
}

// Send intercepts these from the typed text, so they insert as plain `/name`.
pub fn zeron_owned_command(name: &str) -> bool {
    [
        COMPACT_COMMAND,
        SIDECHAT_COMMAND,
        BTW_COMMAND,
        COMPUTER_USE_COMMAND,
    ]
    .contains(&name)
}

/// Human-readable presentation only; invocation names and paths stay canonical.
pub fn skill_display_name(name: &str) -> String {
    name.rsplit(':')
        .next()
        .unwrap_or(name)
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            let first = chars.next().unwrap();
            first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn compaction_unsupported(harness: HarnessId) -> String {
    format!("{} does not support compaction", harness.display_name())
}

pub fn invocation_detail(description: &str, input_hint: Option<&str>) -> String {
    match input_hint {
        Some(hint) if description.is_empty() => format!("<{hint}>"),
        Some(hint) => format!("{description} \u{b7} <{hint}>"),
        None => description.to_string(),
    }
}

pub fn invocation_insertion(invocation: &Invocation, supported: bool) -> String {
    if !supported && matches!(invocation, Invocation::Command { .. }) {
        invocation.prompt_text()
    } else {
        invocation.link()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationCandidate {
    pub name: String,
    pub description: String,
    pub input_hint: Option<String>,
    pub invocation: Invocation,
    pub workspace_command: Option<WorkspaceCommand>,
}

/// Commands implemented by Zeron, independently of the provider protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceCommand {
    Model,
    New,
    Resume,
    Settings,
    Diff,
    Files,
    Terminal,
    Rename,
    Stop,
}

impl WorkspaceCommand {
    fn catalog() -> &'static [(Self, &'static str, &'static str, bool)] {
        &[
            (
                Self::Model,
                "model",
                "Zeron: choose agent, model, and reasoning",
                false,
            ),
            (Self::New, "new", "Zeron: start a new conversation", false),
            (
                Self::Resume,
                "resume",
                "Zeron: search and open conversations",
                false,
            ),
            (Self::Settings, "settings", "Zeron: open settings", false),
            (Self::Diff, "diff", "Zeron: open changes", true),
            (Self::Files, "files", "Zeron: open project files", true),
            (Self::Terminal, "terminal", "Zeron: open a terminal", true),
            (
                Self::Rename,
                "rename",
                "Zeron: rename this conversation",
                true,
            ),
            (Self::Stop, "stop", "Zeron: stop the active run", true),
        ]
    }
}

pub fn with_workspace_commands(
    mut rows: Vec<InvocationCandidate>,
    in_chat: bool,
) -> Vec<InvocationCandidate> {
    rows.retain(|row| row.workspace_command.is_none());
    for &(command, name, description, needs_chat) in WorkspaceCommand::catalog() {
        if needs_chat && !in_chat {
            continue;
        }
        // Keep provider commands intact. Explicit Zeron names remain available
        // when a provider owns the unqualified name.
        let mut name = name.to_string();
        while rows.iter().any(|row| row.name == name) {
            name = format!("zeron:{name}");
        }
        rows.push(InvocationCandidate {
            invocation: Invocation::Command { name: name.clone() },
            name,
            description: description.into(),
            input_hint: None,
            workspace_command: Some(command),
        });
    }
    rows
}

pub fn workspace_command_for_text<'a, C>(
    text: &str,
    rows: impl IntoIterator<Item = (&'a str, Option<C>)>,
) -> Option<C> {
    let (name, argument) = crate::invocation::leading_command(text)?;
    if !argument.is_empty() {
        return None;
    }
    rows.into_iter().find(|(row, _)| *row == name)?.1
}

pub fn invocation_candidates(
    commands: Vec<SlashCommand>,
    skills: Vec<Skill>,
) -> Vec<InvocationCandidate> {
    // A remote engine may use an older catalog decoder. Every visible choice
    // must still survive the local canonical-reference decoder unchanged.
    let skills: Vec<_> = skills
        .into_iter()
        .filter(|skill| {
            valid_invocation_name(&skill.name)
                && valid_skill_path(&skill.path)
                && skill
                    .command
                    .as_ref()
                    .is_none_or(|command| valid_skill_command_name(&command.name))
        })
        .collect();
    let skill_commands: std::collections::HashSet<_> = skills
        .iter()
        .filter_map(|skill| skill.command.as_ref().map(|command| command.name.as_str()))
        .collect();
    let commands: Vec<_> = commands
        .into_iter()
        .filter(|command| {
            valid_invocation_name(&command.name) && !skill_commands.contains(command.name.as_str())
        })
        .collect();
    commands
        .into_iter()
        .map(|c| InvocationCandidate {
            input_hint: c.input_hint,
            name: c.name.clone(),
            description: c.description,
            invocation: Invocation::Command { name: c.name },
            workspace_command: None,
        })
        .chain(
            skills
                .into_iter()
                .filter(|s| s.enabled)
                .map(|s| InvocationCandidate {
                    input_hint: None,
                    name: s.name.clone(),
                    description: if native_skill_identity(&s.path) {
                        s.description.clone()
                    } else {
                        format!("{} — {}", s.description, s.path)
                    },
                    invocation: Invocation::Skill {
                        name: s.name,
                        path: s.path,
                        command: s.command,
                    },
                    workspace_command: None,
                }),
        )
        .collect()
}

/// The rows, whether the host's catalog supports this trigger, and the half
/// that failed when only one did (`true` for skills).
pub type InvocationCatalog<E> = (Vec<InvocationCandidate>, bool, Option<(E, bool)>);

pub fn merge_invocation_results<E>(
    commands: Result<Vec<SlashCommand>, E>,
    skills: Result<Option<Vec<Skill>>, E>,
    skill_only: bool,
) -> Result<InvocationCatalog<E>, E> {
    match (commands, skills) {
        (Ok(commands), Ok(skills)) => {
            let supported = !skill_only || skills.is_some();
            Ok((
                invocation_candidates(commands, skills.unwrap_or_default()),
                supported,
                None,
            ))
        }
        (Ok(commands), Err(error)) if !skill_only && !commands.is_empty() => Ok((
            invocation_candidates(commands, vec![]),
            true,
            Some((error, true)),
        )),
        (Err(error), Ok(Some(skills))) if skills.iter().any(|skill| skill.enabled) => Ok((
            invocation_candidates(vec![], skills),
            true,
            Some((error, false)),
        )),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogFailure {
    OutdatedEngine,
    Unreachable,
    Failed,
}

pub fn catalog_error_message(failure: CatalogFailure, skill: bool) -> &'static str {
    match (failure, skill) {
        (CatalogFailure::OutdatedEngine, true) => {
            "Skills require an updated engine on the selected device. Restart that device’s Zeron after updating."
        }
        (CatalogFailure::OutdatedEngine, false) => {
            "Commands require an updated engine on the selected device. Restart that device’s Zeron after updating."
        }
        (CatalogFailure::Unreachable, _) => "The session's device is unreachable",
        (CatalogFailure::Failed, true) => "Couldn't load this agent's skills",
        (CatalogFailure::Failed, false) => "Couldn't load this agent's commands",
    }
}

pub fn catalog_empty_message(
    skill: bool,
    supported: bool,
    catalog_empty: bool,
    separate_from_slash: bool,
) -> &'static str {
    match (catalog_empty, skill) {
        (true, true) if supported => "No skills available for this project",
        (true, true) => "This agent does not advertise skills",
        (true, false) if !separate_from_slash => "No commands or skills available",
        (true, false) => "No slash commands available in this integration",
        (false, true) => "No matching skills",
        (false, false) if !separate_from_slash => "No matching commands or skills",
        (false, false) => "No matching commands",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HARNESSES: [HarnessId; 9] = [
        HarnessId::Antigravity,
        HarnessId::ClaudeCode,
        HarnessId::Codex,
        HarnessId::Cursor,
        HarnessId::Devin,
        HarnessId::Grok,
        HarnessId::Hermes,
        HarnessId::Pi,
        HarnessId::Opencode,
    ];

    fn native(name: &str) -> SlashCommand {
        SlashCommand {
            name: name.into(),
            description: "native".into(),
            input_hint: None,
            kind: SlashCommandKind::BuiltIn,
        }
    }

    #[test]
    fn partial_discovery_keeps_commands_and_skills_independently() {
        let command = SlashCommand {
            name: "review".into(),
            description: String::new(),
            input_hint: None,
            kind: Default::default(),
        };
        let skill = crate::invocation::Skill {
            command: None,
            name: "review".into(),
            path: "/repo/SKILL.md".into(),
            description: String::new(),
            enabled: true,
        };
        let (rows, supported, warning) =
            merge_invocation_results(Ok(vec![command]), Err("ListSkills".to_string()), false)
                .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].invocation.prefix(), '/');
        assert!(supported && warning.is_some());
        assert_eq!(invocation_insertion(&rows[0].invocation, false), "/review");
        let (rows, _, warning) = merge_invocation_results(
            Err("commands unavailable".to_string()),
            Ok(Some(vec![skill])),
            false,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].invocation.prefix(), '$');
        assert!(warning.is_some());
        assert!(merge_invocation_results(Ok(vec![]), Err("closed".to_string()), true).is_err());
        let (rows, supported, warning) =
            merge_invocation_results::<String>(Ok(vec![]), Ok(None), true).unwrap();
        assert!(rows.is_empty() && !supported && warning.is_none());
    }

    #[test]
    fn every_harness_catalog_only_offers_round_trippable_references() {
        use crate::invocation::{Skill, SkillCommand, invocation_links};
        for harness in HARNESSES {
            let commands = ["review", "bad\ncommand", "two words"]
                .into_iter()
                .map(|name| SlashCommand {
                    name: name.into(),
                    description: String::new(),
                    input_hint: None,
                    kind: Default::default(),
                })
                .collect();
            let skills = [
                ("审查-é", "/repo/skill dir/SKILL.md", None),
                ("bad name", "/repo/SKILL.md", None),
                ("review", "/repo/bad\npath/SKILL.md", Some("review")),
                ("broken-native", "harness-skill:probe", Some("bad command")),
            ]
            .into_iter()
            .map(|(name, path, command)| Skill {
                name: name.into(),
                path: path.into(),
                description: String::new(),
                enabled: true,
                command: command.map(|name| SkillCommand {
                    name: name.into(),
                    harness,
                }),
            })
            .collect();
            let rows = invocation_candidates(commands, skills);
            assert_eq!(
                rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
                ["review", "审查-é"],
                "{harness:?}"
            );
            for row in rows {
                assert_eq!(
                    invocation_links(&row.invocation.link())[0].1,
                    row.invocation
                );
            }
        }
    }

    #[test]
    fn separated_native_skills_are_not_left_in_the_command_catalog() {
        use crate::invocation::{Skill, SkillCommand};
        for harness in HARNESSES {
            let commands = vec![
                SlashCommand {
                    name: "review".into(),
                    description: String::new(),
                    input_hint: None,
                    kind: Default::default(),
                },
                SlashCommand {
                    name: "compact".into(),
                    description: String::new(),
                    input_hint: None,
                    kind: Default::default(),
                },
            ];
            let skills = vec![Skill {
                name: "review".into(),
                path: "/repo/SKILL.md".into(),
                description: String::new(),
                enabled: true,
                command: Some(SkillCommand {
                    name: "review".into(),
                    harness,
                }),
            }];
            let rows = invocation_candidates(commands, skills);
            assert_eq!(rows.len(), 2);
            assert_eq!(
                rows.iter()
                    .filter(|row| row.invocation.prefix() == '/')
                    .map(|row| row.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["compact"]
            );
            assert_eq!(rows[1].invocation.prefix(), '$');
        }
    }

    #[test]
    fn combined_invocations_preserve_skill_identity_and_command_collisions() {
        use crate::invocation::{Invocation, Skill};
        let commands = vec![SlashCommand {
            name: "review".into(),
            description: "Command".into(),
            input_hint: None,
            kind: Default::default(),
        }];
        let skills = vec![
            Skill {
                command: None,
                name: "review".into(),
                path: "/a/SKILL.md".into(),
                description: "A".into(),
                enabled: true,
            },
            Skill {
                command: None,
                name: "review".into(),
                path: "/b/SKILL.md".into(),
                description: "B".into(),
                enabled: true,
            },
            Skill {
                command: None,
                name: "hidden".into(),
                path: "/c/SKILL.md".into(),
                description: String::new(),
                enabled: false,
            },
        ];
        assert_eq!(invocation_candidates(commands.clone(), vec![]).len(), 1);
        let dollar = invocation_candidates(vec![], skills.clone());
        assert_eq!(dollar.len(), 2);
        let slash = invocation_candidates(commands, skills);
        assert_eq!(slash.len(), 3);
        assert!(matches!(slash[0].invocation, Invocation::Command { .. }));
        for candidate in &slash[1..] {
            assert!(matches!(candidate.invocation, Invocation::Skill { .. }));
            assert!(candidate.invocation.prompt_text().contains("SKILL.md"));
        }
        assert_ne!(slash[1].invocation, slash[2].invocation);
    }

    #[test]
    fn slash_rows_keep_built_ins_and_zeron_commands() {
        let command = |name: &str, kind: SlashCommandKind| SlashCommand {
            name: name.into(),
            description: String::new(),
            input_hint: None,
            kind,
        };
        let names: Vec<String> = with_zeron_commands(
            Some(HarnessId::ClaudeCode),
            vec![
                command("maestro-work", SlashCommandKind::Skill),
                command("model", SlashCommandKind::BuiltIn),
            ],
        )
        .into_iter()
        .map(|command| command.name)
        .collect();
        assert_eq!(
            names[..5],
            [
                BTW_COMMAND,
                SIDECHAT_COMMAND,
                COMPACT_COMMAND,
                COMPUTER_USE_COMMAND,
                "model"
            ]
        );
        assert_eq!(
            names[5..],
            crate::app_commands::TERMINAL_ONLY_COMMANDS
                .iter()
                .filter(|name| **name != "resume")
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
            "Claude Code's terminal-only commands join at the end"
        );
        let advertised = with_zeron_commands(
            Some(HarnessId::ClaudeCode),
            vec![command("status", SlashCommandKind::BuiltIn)],
        );
        assert_eq!(advertised.iter().filter(|c| c.name == "status").count(), 1);
        assert!(
            with_zeron_commands(Some(HarnessId::Codex), vec![])
                .iter()
                .all(|c| c.description != TERMINAL_ONLY_DESCRIPTION)
        );
        assert!(zeron_owned_command(COMPACT_COMMAND));
        assert!(!zeron_owned_command("model"));
    }

    #[test]
    fn workspace_commands_preserve_native_commands_and_avoid_collisions() {
        let native = invocation_candidates(
            vec![
                SlashCommand {
                    name: "model".into(),
                    description: "Native model command".into(),
                    input_hint: Some("model id".into()),
                    kind: Default::default(),
                },
                SlashCommand {
                    name: "zeron:model".into(),
                    description: "Plugin command".into(),
                    input_hint: None,
                    kind: Default::default(),
                },
            ],
            vec![],
        );
        let typed = |text: &str, rows: &[InvocationCandidate]| {
            workspace_command_for_text(
                text,
                rows.iter().map(|r| (r.name.as_str(), r.workspace_command)),
            )
        };
        let rows = with_workspace_commands(native, true);
        assert_eq!(rows.len(), 11);
        assert!(rows[0].workspace_command.is_none());
        assert_eq!(rows[0].input_hint.as_deref(), Some("model id"));
        assert_eq!(typed("/model", &rows), None);
        assert_eq!(typed("/zeron:model", &rows), None);
        assert_eq!(
            typed("/zeron:zeron:model", &rows),
            Some(WorkspaceCommand::Model)
        );
        assert_eq!(with_workspace_commands(rows, true).len(), 11);
        let draft_rows = with_workspace_commands(vec![], false);
        assert_eq!(draft_rows.len(), 4);
        assert_eq!(typed("/diff", &draft_rows), None);
        assert_eq!(
            typed("/model  ", &draft_rows),
            Some(WorkspaceCommand::Model)
        );
        for literal in [
            "    /model",
            "`/model`",
            "```\n/model\n```",
            "please /model",
            "/model extra",
            "/model/path",
        ] {
            assert_eq!(typed(literal, &draft_rows), None, "{literal:?}");
        }
    }

    #[test]
    fn the_compact_entry_never_doubles_a_harness_that_advertises_it() {
        let advertised = |name: &str| SlashCommand {
            name: name.into(),
            description: "harness copy".into(),
            input_hint: None,
            kind: SlashCommandKind::BuiltIn,
        };
        let names = |commands: &[SlashCommand]| -> Vec<String> {
            commands.iter().map(|c| c.name.clone()).collect()
        };
        // Harness advertises it: exactly one row, the harness's own.
        let merged = with_compact_entry(vec![advertised("compact"), advertised("clear")]);
        assert_eq!(names(&merged), vec!["compact", "clear"]);
        assert_eq!(merged[0].description, "harness copy");
        // Harness does not (cursor advertises nothing at all): zeron's rides.
        let merged = with_compact_entry(Vec::new());
        assert_eq!(names(&merged), vec!["compact"]);
        assert_eq!(
            merged[0].description,
            "Free up context by summarizing the conversation"
        );
    }

    #[test]
    fn the_sidechat_entry_never_doubles_a_harness_that_advertises_it() {
        let advertised = |name: &str| SlashCommand {
            name: name.into(),
            description: "harness copy".into(),
            input_hint: None,
            kind: SlashCommandKind::BuiltIn,
        };
        let names = |commands: &[SlashCommand]| -> Vec<String> {
            commands.iter().map(|c| c.name.clone()).collect()
        };
        let merged = with_sidechat_entry(vec![advertised("sidechat"), advertised("clear")]);
        assert_eq!(names(&merged), vec!["btw", "sidechat", "clear"]);
        assert_eq!(merged[1].description, "harness copy");
        let merged = with_sidechat_entry(vec![advertised("btw")]);
        assert_eq!(names(&merged), vec!["sidechat", "btw"]);
        assert_eq!(merged[1].description, "harness copy");
        // The popup's real list: every zeron command rides, each exactly once.
        let merged = with_sidechat_entry(with_compact_entry(Vec::new()));
        assert_eq!(names(&merged), vec!["btw", "sidechat", "compact"]);
        for entry in &merged[..2] {
            assert_eq!(
                entry.description,
                "Ask about this chat on the side, without touching it"
            );
        }
    }

    #[test]
    fn computer_use_is_offered_where_the_catalog_exists_and_wins_a_native_collision() {
        let merged = with_computer_use_entry(
            HarnessId::Codex,
            vec![native("computer-use"), native("clear")],
        );
        let names: Vec<&str> = merged.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["computer-use", "clear"]);
        assert_eq!(
            merged[0].description,
            "Use Zeron Computer for this request only"
        );
        for harness in [
            HarnessId::Opencode,
            HarnessId::Cursor,
            HarnessId::Pi,
            HarnessId::Antigravity,
        ] {
            assert!(
                with_computer_use_entry(harness, vec![native("clear")])
                    .iter()
                    .all(|c| c.name != "computer-use")
            );
        }
        let all = with_sidechat_entry(with_compact_entry(with_computer_use_entry(
            HarnessId::ClaudeCode,
            Vec::new(),
        )));
        let names: Vec<&str> = all.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["btw", "sidechat", "compact", "computer-use"]);
    }

    #[test]
    fn the_empty_popup_says_whether_the_agent_or_the_query_came_up_empty() {
        assert_eq!(
            catalog_empty_message(true, false, true, true),
            "This agent does not advertise skills"
        );
        assert_eq!(
            catalog_empty_message(false, true, true, true),
            "No slash commands available in this integration"
        );
        assert_eq!(
            catalog_empty_message(false, true, false, false),
            "No matching commands or skills"
        );
        assert_eq!(
            invocation_detail("Free up context", Some("focus")),
            "Free up context \u{b7} <focus>"
        );
        assert_eq!(invocation_detail("", Some("focus")), "<focus>");
    }
}
