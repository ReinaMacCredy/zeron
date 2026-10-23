//! The pull request board: grouping, row status text, search, sort, the
//! personal views, and the copy for a board that could not load.
use chrono::{DateTime, Utc};

use crate::capability_errors;
use crate::change_request_assessment::{AttentionReason, Blocker, Facts};
use crate::{ChangeRequestListItem, ChangeRequestMergeability, ChangeRequestReviewDecision};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Attention,
    Review,
    Approved,
    Drafts,
}

impl Group {
    pub const ALL: [Self; 4] = [Self::Attention, Self::Review, Self::Approved, Self::Drafts];

    pub fn label(self) -> &'static str {
        match self {
            Self::Attention => "Needs attention",
            Self::Review => "Awaiting review",
            Self::Approved => "Approved",
            Self::Drafts => "Drafts",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Attention => "attention",
            Self::Review => "review",
            Self::Approved => "approved",
            Self::Drafts => "drafts",
        }
    }
}

pub fn group(item: &ChangeRequestListItem) -> Group {
    let assessment = Facts::from_item(item).assess();
    if item.is_draft {
        Group::Drafts
    } else if !assessment.blockers.is_empty() || !assessment.attention.is_empty() {
        Group::Attention
    } else if item.review_decision == ChangeRequestReviewDecision::Approved {
        Group::Approved
    } else {
        Group::Review
    }
}

/// Indices into `items` for each non-empty group, in board order.
pub fn grouped(items: &[ChangeRequestListItem]) -> Vec<(Group, Vec<usize>)> {
    let groups: Vec<_> = items.iter().map(group).collect();
    Group::ALL
        .into_iter()
        .filter_map(|group| {
            let members: Vec<_> = (0..items.len())
                .filter(|index| groups[*index] == group)
                .collect();
            (!members.is_empty()).then_some((group, members))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Muted,
    Danger,
    Warning,
    Success,
}

/// The pull request glyph's color.
pub fn glyph_tone(item: &ChangeRequestListItem) -> Tone {
    if item.is_draft {
        Tone::Muted
    } else if item.mergeability == ChangeRequestMergeability::Conflicting {
        Tone::Danger
    } else if item.review_decision == ChangeRequestReviewDecision::ChangesRequested {
        Tone::Warning
    } else {
        Tone::Success
    }
}

/// The color of a row's status line, under its title.
pub fn status_tone(item: &ChangeRequestListItem) -> Tone {
    if item.mergeability == ChangeRequestMergeability::Conflicting {
        Tone::Danger
    } else if item.review_decision == ChangeRequestReviewDecision::ChangesRequested {
        Tone::Warning
    } else {
        Tone::Muted
    }
}

/// The status line under a row's title. Failing CI is left to the checks
/// column, so it is not said twice.
pub fn row_status(item: &ChangeRequestListItem) -> String {
    let assessment = Facts::from_item(item).assess();
    let mut labels: Vec<_> = assessment
        .blockers
        .iter()
        .filter(|blocker| **blocker != Blocker::FailingCi)
        .map(|blocker| blocker.label())
        .collect();
    if item.is_draft {
        labels.insert(0, "Draft");
    }
    if item.review_decision == ChangeRequestReviewDecision::Approved {
        labels.push("Approved");
    }
    labels.extend(
        assessment
            .attention
            .iter()
            .filter(|reason| **reason != AttentionReason::FailingCi)
            .map(|reason| reason.label()),
    );
    labels.join(" · ")
}

/// Everything a row says about its state, for search and accessibility.
pub fn status_description(item: &ChangeRequestListItem) -> String {
    let assessment = Facts::from_item(item).assess();
    let mut labels = vec![if item.is_draft { "Draft" } else { "Open" }];
    labels.extend(assessment.blockers.iter().map(|blocker| blocker.label()));
    if item.review_decision == ChangeRequestReviewDecision::Approved {
        labels.push("Approved");
    }
    labels.extend(
        assessment
            .attention
            .iter()
            .filter(|reason| **reason != AttentionReason::FailingCi)
            .map(|reason| reason.label()),
    );
    labels.join(" · ")
}

pub fn matches_query(item: &ChangeRequestListItem, query: &str) -> bool {
    let text = format!(
        "{} {} #{} {} {}",
        item.title,
        item.repository,
        item.number,
        status_description(item),
        item.author.login
    )
    .to_lowercase();
    query
        .split_whitespace()
        .all(|word| text.contains(&word.to_lowercase()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Changes,
    Opened,
    Updated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub field: SortField,
    pub direction: SortDirection,
}

impl Sort {
    pub const DEFAULT: Self = Self {
        field: SortField::Updated,
        direction: SortDirection::Descending,
    };
}

pub fn sort(items: &mut [ChangeRequestListItem], sort: Sort) {
    items.sort_by(|left, right| {
        let primary = match sort.field {
            SortField::Changes => left
                .additions
                .saturating_add(left.deletions)
                .cmp(&right.additions.saturating_add(right.deletions)),
            SortField::Opened => left.created_at.cmp(&right.created_at),
            SortField::Updated => left.updated_at.cmp(&right.updated_at),
        };
        let primary = match sort.direction {
            SortDirection::Ascending => primary,
            SortDirection::Descending => primary.reverse(),
        };
        primary
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.repository.cmp(&right.repository))
            .then_with(|| left.number.cmp(&right.number))
    });
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PersonalView {
    #[default]
    Everything,
    NeedsYou,
    Starred,
}

impl PersonalView {
    pub fn shows(self, item: &ChangeRequestListItem, stars: &[String]) -> bool {
        match self {
            Self::Everything => true,
            Self::NeedsYou => !Facts::from_item(item).assess().attention.is_empty(),
            Self::Starred => stars.contains(&star_key(&item.url)),
        }
    }

    pub fn empty_copy(self) -> &'static str {
        match self {
            Self::Everything => "No matching pull requests",
            Self::NeedsYou => "Nothing needs your attention",
            Self::Starred => "No starred pull requests",
        }
    }
}

/// Stable across devices and remote spellings; stars are local preferences.
pub fn star_key(url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        return url.trim_end_matches('/').to_ascii_lowercase();
    };
    parsed.set_fragment(None);
    parsed.set_query(None);
    let path = parsed.path().trim_end_matches('/').to_owned();
    parsed.set_path(&path);
    parsed.to_string().to_ascii_lowercase()
}

pub fn valid_repository_filter(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageError {
    CliUnavailable,
    Authentication,
    RateLimited,
    Network,
    RemoteOffline(String),
    UpdateRequired(String),
}

impl PageError {
    pub fn from_capability(code: &str) -> Self {
        match code {
            capability_errors::PULL_REQUESTS_CLI_UNAVAILABLE => Self::CliUnavailable,
            capability_errors::PULL_REQUESTS_AUTHENTICATION => Self::Authentication,
            capability_errors::PULL_REQUESTS_RATE_LIMITED => Self::RateLimited,
            _ => Self::Network,
        }
    }

    /// Title and next step, in words.
    pub fn copy(&self) -> (String, String) {
        match self {
            Self::CliUnavailable => (
                "GitHub CLI isn’t available on this device".into(),
                "Install gh and sign in to view your pull requests.".into(),
            ),
            Self::Authentication => (
                "Sign in to GitHub on this device".into(),
                "Run gh auth login, then refresh this page.".into(),
            ),
            Self::RateLimited => (
                "GitHub’s rate limit was reached".into(),
                "Requests are paused for 15 minutes. Your loaded results stay available.".into(),
            ),
            Self::Network => (
                "Couldn’t load pull requests".into(),
                "Check the repository name and your connection, then try again.".into(),
            ),
            Self::RemoteOffline(name) => (
                format!("{name} is offline"),
                "Reconnect the device and try again.".into(),
            ),
            Self::UpdateRequired(name) => (
                format!("Update Zeron on {name}"),
                "This device doesn’t support pull requests yet.".into(),
            ),
        }
    }
}

pub fn relative_time(timestamp: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = now.signed_duration_since(timestamp).num_seconds().max(0);
    let (amount, unit) = if seconds < 60 {
        return "just now".into();
    } else if seconds < 3_600 {
        (seconds / 60, "minute")
    } else if seconds < 86_400 {
        (seconds / 3_600, "hour")
    } else if seconds < 604_800 {
        (seconds / 86_400, "day")
    } else if seconds < 2_592_000 {
        (seconds / 604_800, "week")
    } else if seconds < 31_536_000 {
        (seconds / 2_592_000, "month")
    } else {
        (seconds / 31_536_000, "year")
    };
    format!("{amount} {unit}{} ago", if amount == 1 { "" } else { "s" })
}

pub fn compact_relative_time(timestamp: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = now.signed_duration_since(timestamp).num_seconds().max(0);
    if seconds < 60 {
        "now".into()
    } else if seconds < 3_600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3_600)
    } else if seconds < 604_800 {
        format!("{}d ago", seconds / 86_400)
    } else if seconds < 2_592_000 {
        format!("{}w ago", seconds / 604_800)
    } else {
        format!("{}mo ago", seconds / 2_592_000)
    }
}

pub fn format_compact_count(value: u64) -> String {
    if value < 1_000 {
        value.to_string()
    } else if value < 1_000_000 {
        let tenths = value / 100;
        if tenths % 10 == 0 {
            format!("{}k", value / 1_000)
        } else {
            format!("{}.{:01}k", value / 1_000, tenths % 10)
        }
    } else {
        let tenths = value / 100_000;
        if tenths % 10 == 0 {
            format!("{}m", value / 1_000_000)
        } else {
            format!("{}.{:01}m", value / 1_000_000, tenths % 10)
        }
    }
}
