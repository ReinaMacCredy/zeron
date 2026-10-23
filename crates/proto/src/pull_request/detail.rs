//! Pull request detail: status chips, the CI summary, and failures in words.
use crate::ChangeRequestDetail;
use crate::capability_errors as codes;
use crate::change_request_assessment::{CiState, Facts};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusTone {
    Neutral,
    Positive,
    Warning,
    Negative,
    Merged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusGlyph {
    PullRequest,
    CloseCircle,
    Document,
    Check,
    DangerTriangle,
    ClockCircle,
}

/// Label, glyph and tone for a GitHub state, review decision or check status.
pub fn status_style(raw: &str) -> (String, StatusGlyph, StatusTone) {
    match raw.to_ascii_uppercase().replace(' ', "_").as_str() {
        "OPEN" => (
            "Open".into(),
            StatusGlyph::PullRequest,
            StatusTone::Positive,
        ),
        "MERGED" => (
            "Merged".into(),
            StatusGlyph::PullRequest,
            StatusTone::Merged,
        ),
        "CLOSED" => (
            "Closed".into(),
            StatusGlyph::CloseCircle,
            StatusTone::Negative,
        ),
        "DRAFT" => ("Draft".into(), StatusGlyph::Document, StatusTone::Neutral),
        "APPROVED" => ("Approved".into(), StatusGlyph::Check, StatusTone::Positive),
        "SUCCESS" => ("Passed".into(), StatusGlyph::Check, StatusTone::Positive),
        "FAILURE" | "ERROR" | "TIMED_OUT" | "ACTION_REQUIRED" => (
            humanize(raw),
            StatusGlyph::CloseCircle,
            StatusTone::Negative,
        ),
        "CHANGES_REQUESTED" => (
            "Changes requested".into(),
            StatusGlyph::DangerTriangle,
            StatusTone::Warning,
        ),
        "REVIEW_REQUIRED" => (
            "Awaiting review".into(),
            StatusGlyph::ClockCircle,
            StatusTone::Warning,
        ),
        "IN_PROGRESS" | "PENDING" | "QUEUED" | "WAITING" => {
            (humanize(raw), StatusGlyph::ClockCircle, StatusTone::Warning)
        }
        _ => (humanize(raw), StatusGlyph::Document, StatusTone::Neutral),
    }
}

pub fn humanize(raw: &str) -> String {
    let text = raw.replace('_', " ").to_lowercase();
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_else(|| "Not reported".into())
}

/// The raw status the CI chip shows; [`status_style`] names it.
pub fn ci_status(detail: &ChangeRequestDetail) -> &'static str {
    match Facts::from_detail(detail).assess().ci.state {
        CiState::Unknown => "CI unavailable",
        CiState::NoChecks => "No checks reported",
        CiState::Failed => "FAILURE",
        CiState::Pending => "PENDING",
        CiState::Passed => "SUCCESS",
        CiState::Skipped => "SKIPPED",
    }
}

/// What the checks card says when it has no rows.
pub fn checks_empty_copy(detail: &ChangeRequestDetail) -> &'static str {
    match Facts::from_detail(detail).assess().ci.state {
        CiState::Unknown => "CI metadata is unavailable. Refresh to try again.",
        CiState::NoChecks => "No checks reported.",
        _ => "Check details are unavailable. Open this pull request on GitHub.",
    }
}

pub const UPDATE_REQUIRED_REASON: &str = "Update Zeron on the selected device.";

/// What went wrong and what to do, for a capability error code. Codes and
/// transport details never reach the reader.
pub fn failure_reason(code: &str) -> &'static str {
    match code {
        codes::PULL_REQUESTS_CLI_UNAVAILABLE => {
            "Install GitHub CLI (gh) on this device and sign in."
        }
        codes::PULL_REQUESTS_AUTHENTICATION => "Run gh auth login on this device, then refresh.",
        codes::PULL_REQUESTS_RATE_LIMITED => {
            "GitHub’s rate limit was reached. Try again in 15 minutes."
        }
        codes::PULL_REQUESTS_TIMEOUT => "GitHub didn’t respond in time. Try again.",
        codes::PULL_REQUESTS_DECODE => {
            "GitHub’s response was too large or unreadable. Open it on GitHub instead."
        }
        _ => "Check the link and your connection, then try again.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ci_summary_handles_mixed_and_unreported_checks() {
        let check = |conclusion: &str, state: &str, status: &str| crate::ChangeRequestCheck {
            conclusion: conclusion.into(),
            state: state.into(),
            status: status.into(),
            ..Default::default()
        };
        let ci_status = |checks: &[crate::ChangeRequestCheck]| {
            super::ci_status(&ChangeRequestDetail {
                ci: crate::change_request_assessment::ChangeRequestCi::from_checks(checks),
                status_check_rollup: checks.to_vec(),
                ..Default::default()
            })
        };
        assert_eq!(
            super::ci_status(&ChangeRequestDetail::default()),
            "CI unavailable"
        );
        assert_eq!(ci_status(&[]), "No checks reported");
        assert_eq!(
            ci_status(&[
                check("SUCCESS", "", "COMPLETED"),
                check("SKIPPED", "", "COMPLETED")
            ]),
            "SUCCESS"
        );
        assert_eq!(
            ci_status(&[check("", "SUCCESS", ""), check("", "", "IN_PROGRESS")]),
            "PENDING"
        );
        assert_eq!(
            ci_status(&[check("FAILURE", "", "COMPLETED"), check("", "", "QUEUED")]),
            "FAILURE"
        );
        assert_eq!(ci_status(&[check("CANCELLED", "", "COMPLETED")]), "FAILURE");
        assert_eq!(ci_status(&[check("SKIPPED", "", "COMPLETED")]), "SKIPPED");
        assert_eq!(ci_status(&[check("", "", "")]), "PENDING");
    }

    #[test]
    fn status_cues_distinguish_passed_pending_failed_and_skipped() {
        assert_eq!(status_style("SUCCESS").2, StatusTone::Positive);
        assert_eq!(status_style("IN_PROGRESS").2, StatusTone::Warning);
        assert_eq!(status_style("FAILURE").2, StatusTone::Negative);
        assert_eq!(status_style("SKIPPED").2, StatusTone::Neutral);
        assert_eq!(status_style("CANCELLED").2, StatusTone::Neutral);
        assert_eq!(status_style("MERGED").2, StatusTone::Merged);
        assert_eq!(status_style("CHANGES_REQUESTED").0, "Changes requested");
        assert_eq!(status_style("REVIEW_REQUIRED").0, "Awaiting review");
    }
}
