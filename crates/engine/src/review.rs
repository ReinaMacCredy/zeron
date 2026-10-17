use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zeron_proto::{AgentProfile, HarnessId, ReasoningLevel};

pub const REQUEST_PREFIX: &str = "zeron-review:";
const MAX_PATCH_BYTES: usize = 120_000;
const FENCE: &str = "```zeron-review";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewScope {
    Uncommitted,
    Branch,
    LatestTurn,
    Commit,
}

impl ReviewScope {
    pub const ALL: [ReviewScope; 3] = [Self::Branch, Self::Uncommitted, Self::LatestTurn];

    pub fn default_for(worktree: bool) -> Self {
        if worktree {
            Self::Branch
        } else {
            Self::Uncommitted
        }
    }

    pub fn mode(self) -> &'static str {
        match self {
            Self::Uncommitted => "workingTree",
            Self::Branch => "branch",
            Self::LatestTurn => "turn",
            Self::Commit => "commit",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Uncommitted => "uncommitted",
            Self::Branch => "branch",
            Self::LatestTurn => "turn",
            Self::Commit => "commit",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .chain([Self::Commit])
            .find(|scope| scope.key() == key)
    }

    pub fn label(self, base: Option<&str>) -> String {
        match self {
            Self::Uncommitted => "Uncommitted".into(),
            Self::Branch => format!("Branch vs {}", base.unwrap_or("base")),
            Self::LatestTurn => "Latest turn".into(),
            Self::Commit => "Commit".into(),
        }
    }
}

pub fn request_id(scope: ReviewScope, commit: Option<&str>, nonce: &str) -> String {
    match commit {
        Some(sha) => format!("{REQUEST_PREFIX}{}:{nonce}:{sha}", scope.key()),
        None => format!("{REQUEST_PREFIX}{}:{nonce}", scope.key()),
    }
}

pub fn is_review(delegation: &zeron_proto::Delegation) -> bool {
    delegation
        .client_request_id
        .as_deref()
        .is_some_and(|id| id.starts_with(REQUEST_PREFIX))
}

pub fn parse_request(request_id: &str) -> Option<(ReviewScope, Option<String>)> {
    let mut parts = request_id.strip_prefix(REQUEST_PREFIX)?.split(':');
    let scope = ReviewScope::from_key(parts.next()?)?;
    let commit = parts
        .nth(1)
        .filter(|sha| !sha.is_empty())
        .map(str::to_string);
    Some((scope, commit))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReviewerChoice {
    #[default]
    Auto,
    Harness {
        harness: HarnessId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<ReasoningLevel>,
    },
    Profile {
        id: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReviewSettings {
    pub reviewer: ReviewerChoice,
    pub instructions: String,
}

#[derive(Debug, Clone, Default)]
pub struct ReviewerOptions {
    pub offered: Vec<HarnessId>,
    pub profiles: Vec<AgentProfile>,
    pub levels: HashMap<HarnessId, Vec<ReasoningLevel>>,
}

impl ReviewerOptions {
    pub fn label(&self, choice: &ReviewerChoice) -> String {
        match choice {
            ReviewerChoice::Auto => "Auto".to_string(),
            ReviewerChoice::Harness { harness, model, .. } => match model {
                Some(model) => format!("{} · {model}", harness.display_name()),
                None => harness.display_name().to_string(),
            },
            ReviewerChoice::Profile { id } => self
                .profiles
                .iter()
                .find(|profile| &profile.id == id)
                .map_or_else(
                    || "Missing profile".to_string(),
                    |profile| profile.name.clone(),
                ),
        }
    }

    pub fn choices(&self) -> Vec<ReviewerChoice> {
        std::iter::once(ReviewerChoice::Auto)
            .chain(self.offered.iter().map(|harness| ReviewerChoice::Harness {
                harness: *harness,
                model: None,
                reasoning: None,
            }))
            .chain(self.profiles.iter().map(|profile| ReviewerChoice::Profile {
                id: profile.id.clone(),
            }))
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reviewer {
    pub harness: Option<HarnessId>,
    pub model: Option<String>,
    pub reasoning: Option<ReasoningLevel>,
    pub profile: Option<AgentProfile>,
    pub note: Option<String>,
}

impl Reviewer {
    pub fn name(&self) -> String {
        match (&self.profile, self.harness) {
            (Some(profile), _) => profile.name.clone(),
            (None, Some(harness)) => harness.display_name().to_string(),
            (None, None) => "the chat's agent".to_string(),
        }
    }
}

pub fn other_family(chat: Option<HarnessId>, offered: &[HarnessId]) -> Option<HarnessId> {
    let wanted: &[HarnessId] = match chat {
        Some(HarnessId::ClaudeCode) => &[HarnessId::Codex],
        Some(HarnessId::Codex) => &[HarnessId::ClaudeCode],
        _ => &[HarnessId::ClaudeCode, HarnessId::Codex],
    };
    wanted.iter().copied().find(|id| offered.contains(id))
}

pub fn resolve_reviewer(
    choice: &ReviewerChoice,
    chat: Option<HarnessId>,
    options: &ReviewerOptions,
) -> Reviewer {
    let usable = |profile: &AgentProfile| options.offered.contains(&profile.harness);
    let from_profile = |profile: &AgentProfile| Reviewer {
        harness: Some(profile.harness),
        model: profile.model.clone(),
        reasoning: profile.reasoning,
        profile: Some(profile.clone()),
        note: None,
    };
    let skipped = match choice {
        ReviewerChoice::Auto => None,
        ReviewerChoice::Harness {
            harness,
            model,
            reasoning,
        } if options.offered.contains(harness) => {
            return Reviewer {
                harness: Some(*harness),
                model: model.clone(),
                reasoning: *reasoning,
                ..Reviewer::default()
            };
        }
        ReviewerChoice::Harness { harness, .. } => {
            Some(format!("{} is unavailable", harness.display_name()))
        }
        ReviewerChoice::Profile { id } => match options.profiles.iter().find(|p| &p.id == id) {
            Some(profile) if usable(profile) => return from_profile(profile),
            Some(profile) => Some(format!("{} is unavailable", profile.name)),
            None => Some("The chosen profile was deleted".to_string()),
        },
    };
    let named = options
        .profiles
        .iter()
        .find(|profile| profile.name.trim().eq_ignore_ascii_case("reviewer") && usable(profile));
    let mut reviewer = match (named, other_family(chat, &options.offered)) {
        (Some(profile), _) => from_profile(profile),
        (None, Some(harness)) => Reviewer {
            harness: Some(harness),
            ..Reviewer::default()
        },
        (None, None) => {
            let own = Reviewer {
                harness: chat,
                ..Reviewer::default()
            };
            Reviewer {
                note: Some(format!(
                    "No other agent is available, so {} reviews its own work",
                    own.name()
                )),
                ..own
            }
        }
    };
    if let Some(skipped) = skipped {
        reviewer.note = Some(format!("{skipped}, so {} reviews instead", reviewer.name()));
    }
    reviewer
}

pub fn default_base_ref(branches: &[String], current: Option<&str>) -> Option<String> {
    let first = branches.first()?;
    if current != Some(first.as_str()) {
        return Some(first.clone());
    }
    for candidate in ["main", "master"] {
        if branches.iter().any(|b| b == candidate) {
            return Some(candidate.to_string());
        }
    }
    branches
        .iter()
        .find(|b| current != Some(b.as_str()))
        .or(Some(first))
        .cloned()
}

pub struct PromptInput<'a> {
    pub scope: ReviewScope,
    pub base: Option<&'a str>,
    pub commit: Option<&'a str>,
    pub patch: &'a str,
    pub truncated: bool,
    pub files: &'a [String],
    pub instructions: &'a str,
}

pub fn prompt(input: PromptInput) -> String {
    let scope = match (input.scope, input.commit) {
        (ReviewScope::Commit, Some(sha)) => format!("commit {sha}"),
        (scope, _) => scope.label(input.base),
    };
    let mut out = format!(
        "Review these code changes ({scope}). Report problems only: never edit, create, or delete files, and never commit. You may read files in this checkout for context.\n\n"
    );
    let fits = !input.truncated && input.patch.len() <= MAX_PATCH_BYTES;
    if fits {
        out.push_str("```diff\n");
        out.push_str(input.patch.trim_end());
        out.push_str("\n```\n\n");
    } else {
        let how = match input.scope {
            ReviewScope::Uncommitted => {
                "Read it with `git diff HEAD`, plus `git status --porcelain` for new files.".into()
            }
            ReviewScope::Branch => format!(
                "Read it with `git diff $(git merge-base {} HEAD)`, plus `git status --porcelain` for new files.",
                input.base.unwrap_or("main")
            ),
            ReviewScope::LatestTurn => "Read the changed files listed below.".into(),
            ReviewScope::Commit => format!(
                "Read it with `git show {}`.",
                input.commit.unwrap_or("HEAD")
            ),
        };
        out.push_str(&format!(
            "The diff is too large to include. {how}\nChanged files:\n"
        ));
        for file in input.files {
            out.push_str(&format!("- {file}\n"));
        }
        out.push('\n');
    }
    out.push_str(concat!(
        "Look for correctness bugs, regressions, security problems, and risky assumptions. ",
        "Skip style preferences unless they hide a bug.\n\n",
    ));
    let instructions = input.instructions.trim();
    if !instructions.is_empty() {
        out.push_str("Additional instructions from the user:\n");
        out.push_str(instructions);
        out.push_str("\n\n");
    }
    out.push_str(concat!(
        "End your reply with exactly one fenced code block whose info string is zeron-review, ",
        "holding a JSON array of findings (an empty array when you found nothing):\n\n",
        "```zeron-review\n",
        "[{\"path\": \"src/lib.rs\", \"side\": \"R\", \"line\": 42, \"text\": \"let run = snapshot.run.unwrap();\", ",
        "\"severity\": \"bug\", \"title\": \"One line summary\", \"body\": \"Why it is wrong and how to fix it.\"}]\n",
        "```\n\n",
        "path is the file path exactly as the diff shows it. side is \"R\" for an added or unchanged line ",
        "(line = its number in the changed file) and \"L\" for a removed line (line = its number in the ",
        "original file). text is that line's exact content without the +/- marker. ",
        "severity is \"bug\", \"risk\", or \"nit\"."
    ));
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Bug,
    Risk,
    Nit,
}

impl Severity {
    pub fn word(self) -> &'static str {
        match self {
            Self::Bug => "Bug",
            Self::Risk => "Risk",
            Self::Nit => "Nit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingSide {
    Old,
    New,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewFinding {
    pub id: String,
    pub path: String,
    pub side: FindingSide,
    pub line: u32,
    pub text: Option<String>,
    pub severity: Severity,
    pub title: String,
    pub body: String,
}

impl ReviewFinding {
    pub fn location(&self) -> String {
        format!("{}:{}", self.path, self.line)
    }
}

#[derive(Deserialize)]
struct RawFinding {
    path: String,
    #[serde(default)]
    side: Option<String>,
    line: serde_json::Value,
    #[serde(default)]
    text: Option<String>,
    severity: String,
    title: String,
    #[serde(default)]
    body: String,
}

fn finding_from(ix: usize, raw: RawFinding) -> Option<ReviewFinding> {
    let line = match &raw.line {
        serde_json::Value::Number(n) => u32::try_from(n.as_u64()?).ok()?,
        serde_json::Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    let severity = match raw.severity.trim().to_ascii_lowercase().as_str() {
        "bug" => Severity::Bug,
        "risk" => Severity::Risk,
        "nit" => Severity::Nit,
        _ => return None,
    };
    Some(ReviewFinding {
        id: format!("F{}", ix + 1),
        path: raw.path.trim().trim_start_matches("./").to_string(),
        side: match raw.side.as_deref().map(str::trim) {
            Some("L" | "l" | "old") => FindingSide::Old,
            _ => FindingSide::New,
        },
        line,
        text: raw.text.filter(|text| !text.trim().is_empty()),
        severity,
        title: raw.title,
        body: raw.body,
    })
}

pub fn parse_findings(result: &str) -> Option<Vec<ReviewFinding>> {
    let start = result.rfind(FENCE)? + FENCE.len();
    let rest = result[start..].split_once('\n')?.1;
    let end = if rest.starts_with("```") {
        0
    } else {
        rest.find("\n```")?
    };
    let items: Vec<serde_json::Value> = serde_json::from_str(rest[..end].trim()).ok()?;
    let total = items.len();
    let findings: Vec<ReviewFinding> = items
        .into_iter()
        .filter_map(|value| serde_json::from_value::<RawFinding>(value).ok())
        .enumerate()
        .filter_map(|(ix, raw)| finding_from(ix, raw))
        .collect();
    (total == 0 || !findings.is_empty()).then_some(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(block: &str) -> String {
        format!("I looked at the diff.\n\n```zeron-review\n{block}\n```\n")
    }

    #[test]
    fn parses_the_last_block_and_an_empty_array_means_clean() {
        let text = format!(
            "{}\nSecond pass:\n{}",
            reply("[{\"path\":\"a\",\"line\":1,\"severity\":\"nit\",\"title\":\"x\"}]"),
            reply("[]")
        );
        assert_eq!(parse_findings(&text), Some(Vec::new()));
        let found = parse_findings(&reply(
            "[{\"path\":\"./src/a.rs\",\"side\":\"L\",\"line\":3,\"severity\":\"bug\",\"title\":\"t\",\"body\":\"b\"}]",
        ))
        .unwrap()
        .remove(0);
        assert_eq!(found.path, "src/a.rs");
        assert_eq!(found.side, FindingSide::Old);
        assert_eq!(found.severity, Severity::Bug);
    }

    #[test]
    fn one_bad_item_does_not_sink_the_block() {
        let found = parse_findings(&reply(
            "[{\"path\":\"a\",\"line\":\"4\",\"severity\":\"Bug\",\"title\":\"x\",\"text\":\"/// ```rust\"},\
             {\"path\":\"b\",\"line\":2,\"severity\":\"high\",\"title\":\"y\"}]",
        ))
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].severity), (4, Severity::Bug));
        assert_eq!(
            parse_findings(&reply(
                "[{\"path\":\"b\",\"line\":2,\"severity\":\"high\",\"title\":\"y\"}]"
            )),
            None
        );
    }

    #[test]
    fn a_missing_or_garbled_block_is_unreadable_not_clean() {
        assert_eq!(parse_findings("No issues found."), None);
        assert_eq!(parse_findings(&reply("[{\"path\": ")), None);
        assert_eq!(parse_findings("```zeron-review\n[]"), None);
    }

    #[test]
    fn findings_are_numbered_from_one_in_reply_order() {
        let found = parse_findings(&reply(
            "[{\"path\":\"a\",\"line\":1,\"severity\":\"bug\",\"title\":\"first\"},\
             {\"path\":\"b\",\"line\":2,\"severity\":\"high\",\"title\":\"dropped\"},\
             {\"path\":\"c\",\"line\":3,\"severity\":\"nit\",\"title\":\"third\"}]",
        ))
        .unwrap();
        let ids: Vec<(&str, &str)> = found
            .iter()
            .map(|f| (f.id.as_str(), f.title.as_str()))
            .collect();
        assert_eq!(ids, [("F1", "first"), ("F3", "third")]);
    }

    fn profile(id: &str, name: &str, harness: HarnessId) -> AgentProfile {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "harness": harness,
        }))
        .unwrap()
    }

    #[test]
    fn the_reviewer_resolves_by_choice_then_falls_back_with_a_note() {
        let options = ReviewerOptions {
            offered: vec![HarnessId::ClaudeCode, HarnessId::Codex],
            profiles: vec![profile("p1", "Reviewer", HarnessId::Codex)],
            levels: HashMap::new(),
        };
        let picked = resolve_reviewer(
            &ReviewerChoice::Harness {
                harness: HarnessId::ClaudeCode,
                model: None,
                reasoning: Some(ReasoningLevel::High),
            },
            Some(HarnessId::ClaudeCode),
            &options,
        );
        assert_eq!(
            (picked.harness, picked.reasoning),
            (Some(HarnessId::ClaudeCode), Some(ReasoningLevel::High))
        );
        let auto = resolve_reviewer(&ReviewerChoice::Auto, Some(HarnessId::Codex), &options);
        assert_eq!(auto.profile.map(|p| p.id), Some("p1".to_string()));
        assert_eq!(auto.note, None);

        let no_profiles = ReviewerOptions {
            profiles: Vec::new(),
            ..options.clone()
        };
        let auto = resolve_reviewer(&ReviewerChoice::Auto, Some(HarnessId::Codex), &no_profiles);
        assert_eq!(auto.harness, Some(HarnessId::ClaudeCode));

        let alone = ReviewerOptions {
            offered: vec![HarnessId::Codex],
            profiles: Vec::new(),
            levels: HashMap::new(),
        };
        let own = resolve_reviewer(&ReviewerChoice::Auto, Some(HarnessId::Codex), &alone);
        assert_eq!(own.harness, Some(HarnessId::Codex));
        assert!(own.note.is_some());

        let gone = resolve_reviewer(
            &ReviewerChoice::Profile {
                id: "deleted".into(),
            },
            Some(HarnessId::ClaudeCode),
            &no_profiles,
        );
        assert_eq!(gone.harness, Some(HarnessId::Codex));
        assert!(gone.note.unwrap().contains("deleted"));

        let unavailable = resolve_reviewer(
            &ReviewerChoice::Harness {
                harness: HarnessId::Mock,
                model: None,
                reasoning: None,
            },
            Some(HarnessId::ClaudeCode),
            &no_profiles,
        );
        assert_eq!(unavailable.harness, Some(HarnessId::Codex));
        assert!(unavailable.note.is_some());
    }

    #[test]
    fn user_instructions_go_before_the_findings_format() {
        let files = vec!["a.rs".to_string()];
        let input = |instructions| PromptInput {
            scope: ReviewScope::Uncommitted,
            base: None,
            commit: None,
            patch: "diff",
            truncated: false,
            files: &files,
            instructions,
        };
        let with = prompt(input("Write findings in Vietnamese."));
        let at = with.find("Write findings in Vietnamese.").unwrap();
        assert!(at < with.find("End your reply").unwrap());
        assert!(!prompt(input("  ")).contains("Additional instructions"));
    }

    #[test]
    fn the_request_id_carries_the_scope_and_commit() {
        let id = request_id(ReviewScope::Branch, None, "abc");
        assert!(id.starts_with(REQUEST_PREFIX));
        assert_eq!(parse_request(&id), Some((ReviewScope::Branch, None)));
        let id = request_id(ReviewScope::Commit, Some("a1b2c3d"), "abc");
        assert_eq!(
            parse_request(&id),
            Some((ReviewScope::Commit, Some("a1b2c3d".to_string())))
        );
        assert_eq!(parse_request("other:branch:x"), None);
    }

    #[test]
    fn saved_review_settings_round_trip_and_default_to_auto() {
        let settings: ReviewSettings = serde_json::from_value(serde_json::json!({
            "reviewer": {"kind": "harness", "harness": "claude-code", "reasoning": "high"},
            "instructions": "Be terse."
        }))
        .unwrap();
        assert_eq!(settings.instructions, "Be terse.");
        let back: ReviewSettings =
            serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
        assert_eq!(back, settings);
        assert_eq!(
            serde_json::from_value::<ReviewSettings>(serde_json::json!({})).unwrap(),
            ReviewSettings::default()
        );
    }
}
