pub use zeron_proto::review::*;

const MAX_PATCH_BYTES: usize = 120_000;

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

pub fn chosen_reviewer(
    choice: ReviewerChoice,
    chat: Option<zeron_proto::HarnessId>,
    options: &ReviewerOptions,
) -> Result<Reviewer, String> {
    use zeron_proto::HarnessId;
    // A mock chat's diff never goes to a real model it didn't name.
    let choice = match choice {
        ReviewerChoice::Auto if chat == Some(HarnessId::Mock) => ReviewerChoice::Harness {
            harness: HarnessId::Mock,
            model: None,
            reasoning: None,
        },
        choice => choice,
    };
    let reviewer = resolve_reviewer(&choice, chat, options);
    let unavailable = match &choice {
        ReviewerChoice::Auto => None,
        ReviewerChoice::Harness { harness, .. } => (reviewer.harness != Some(*harness)
            || reviewer.profile.is_some())
        .then(|| harness.display_name().to_string()),
        ReviewerChoice::Profile { id } => (reviewer.profile.as_ref().map(|p| &p.id) != Some(id))
            .then(|| {
                options
                    .profiles
                    .iter()
                    .find(|p| &p.id == id)
                    .map_or_else(|| "The chosen profile".to_string(), |p| p.name.clone())
            }),
    };
    match unavailable {
        Some(name) => Err(format!(
            "{name} is unavailable here, so nothing was reviewed. Choose another reviewer in Settings > Code review."
        )),
        None => Ok(reviewer),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reviewer_is_never_one_the_person_did_not_choose() {
        use zeron_proto::HarnessId;
        let offered = |ids: &[HarnessId]| ReviewerOptions {
            offered: ids.to_vec(),
            ..ReviewerOptions::default()
        };
        let real = offered(&[HarnessId::ClaudeCode, HarnessId::Codex]);
        let mock = ReviewerChoice::Harness {
            harness: HarnessId::Mock,
            model: None,
            reasoning: None,
        };
        assert!(chosen_reviewer(mock.clone(), Some(HarnessId::Mock), &real).is_err());
        assert!(chosen_reviewer(ReviewerChoice::Auto, Some(HarnessId::Mock), &real).is_err());
        let with_mock = offered(&[HarnessId::ClaudeCode, HarnessId::Mock]);
        let auto =
            chosen_reviewer(ReviewerChoice::Auto, Some(HarnessId::Mock), &with_mock).unwrap();
        assert_eq!(auto.harness, Some(HarnessId::Mock));
        assert_eq!(
            chosen_reviewer(mock, None, &with_mock).unwrap().harness,
            Some(HarnessId::Mock)
        );
        let codex = ReviewerChoice::Harness {
            harness: HarnessId::Codex,
            model: None,
            reasoning: None,
        };
        assert!(chosen_reviewer(codex, Some(HarnessId::ClaudeCode), &with_mock).is_err());
        let gone = ReviewerChoice::Profile {
            id: "deleted".into(),
        };
        assert!(chosen_reviewer(gone, Some(HarnessId::ClaudeCode), &real).is_err());
        let auto =
            chosen_reviewer(ReviewerChoice::Auto, Some(HarnessId::ClaudeCode), &real).unwrap();
        assert_eq!(auto.harness, Some(HarnessId::Codex));
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
}
