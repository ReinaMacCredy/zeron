//! Pull request writes: merge methods in words, the merge confirmation, and
//! host refusals made readable.
use crate::ChangeRequestMergeMethod;

pub const MERGE_METHODS: [(ChangeRequestMergeMethod, &str, &str, &str); 3] = [
    (
        ChangeRequestMergeMethod::Merge,
        "Merge",
        "merge",
        "Merge commit",
    ),
    (
        ChangeRequestMergeMethod::Squash,
        "Squash and merge",
        "squash",
        "Squash",
    ),
    (
        ChangeRequestMergeMethod::Rebase,
        "Rebase and merge",
        "rebase",
        "Rebase",
    ),
];

pub const MERGE_HINT: &str = "The host refused the merge. Check that you have write access, that the checks it requires have passed, and that the branch is not conflicting.";
pub const REFUSED: &str = "The host refused it.";

pub fn method_entry(
    method: ChangeRequestMergeMethod,
) -> (&'static str, &'static str, &'static str) {
    MERGE_METHODS
        .iter()
        .find(|(known, ..)| *known == method)
        .map(|(_, label, word, choice)| (*label, *word, *choice))
        .unwrap_or(("Merge", "merge", "Merge commit"))
}

pub fn pick_method(
    current: Option<ChangeRequestMergeMethod>,
    last: Option<ChangeRequestMergeMethod>,
    allowed: &[ChangeRequestMergeMethod],
) -> ChangeRequestMergeMethod {
    current
        .filter(|method| allowed.contains(method))
        .or(last.filter(|method| allowed.contains(method)))
        .or(allowed.first().copied())
        .unwrap_or(ChangeRequestMergeMethod::Merge)
}

pub fn merge_confirm(
    method: ChangeRequestMergeMethod,
    number: u64,
    base: &str,
) -> (String, String, &'static str) {
    let (label, word, _) = method_entry(method);
    let into = if base.is_empty() {
        String::new()
    } else {
        format!(" into {base}")
    };
    (
        "Merge pull request?".into(),
        format!("This merges #{number}{into} using {word}."),
        label,
    )
}

pub fn readable(message: &str, hint: &str) -> String {
    let trimmed = message.trim();
    let generic = trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("command failed")
        || trimmed.to_ascii_lowercase().contains("exited with code")
        || trimmed.eq_ignore_ascii_case("unknown error");
    if generic {
        hint.to_owned()
    } else if trimmed.chars().count() > 320 {
        format!("{}…", trimmed.chars().take(319).collect::<String>())
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_method_prefers_the_pick_then_the_last_then_the_first_allowed() {
        use ChangeRequestMergeMethod::*;
        assert_eq!(
            pick_method(Some(Rebase), Some(Squash), &[Merge, Rebase]),
            Rebase
        );
        assert_eq!(
            pick_method(Some(Squash), Some(Rebase), &[Merge, Rebase]),
            Rebase
        );
        assert_eq!(pick_method(None, Some(Squash), &[Merge, Rebase]), Merge);
        assert_eq!(pick_method(None, None, &[]), Merge);
    }

    #[test]
    fn merge_confirmation_and_failures_read_in_words() {
        assert_eq!(
            merge_confirm(ChangeRequestMergeMethod::Squash, 7, "main"),
            (
                "Merge pull request?".to_owned(),
                "This merges #7 into main using squash.".to_owned(),
                "Squash and merge"
            )
        );
        assert_eq!(readable("  ", "hint"), "hint");
        assert_eq!(readable("command failed", "hint"), "hint");
        assert_eq!(
            readable("Base branch was modified", "hint"),
            "Base branch was modified"
        );
        assert_eq!(readable(&"x".repeat(400), "hint").chars().count(), 320);
    }
}
