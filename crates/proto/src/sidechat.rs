use chrono::{DateTime, Duration, Utc};

pub const SIDECHAT_LIFETIME_HOURS: i64 = 24;
pub const NEW_SIDECHAT: &str = "New sidechat";
pub const SIDECHAT_TAB_PREFIX: &str = "Sidechats (btw) \u{b7} ";

pub fn sidechat_tab_label(live: usize) -> String {
    match live {
        0 => "New sidechat (btw)".to_string(),
        count => format!("{SIDECHAT_TAB_PREFIX}{count}"),
    }
}

pub fn sidechat_context_line(source_title: &str) -> String {
    format!("Sidechat of {source_title} · source transcript copied as reference-only context")
}

/// The first non-blank line of the sidechat's opening prompt.
pub fn sidechat_tab_title(prompt: Option<&str>) -> String {
    prompt
        .and_then(|text| text.lines().map(str::trim).find(|line| !line.is_empty()))
        .unwrap_or("New btw")
        .to_string()
}

pub fn sidechat_time_left(created_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let left = created_at + Duration::hours(SIDECHAT_LIFETIME_HOURS) - now;
    let left = if left.num_hours() >= 1 {
        format!("{}h", left.num_hours())
    } else {
        format!("{}m", left.num_minutes().max(1))
    };
    format!("{left} left of {SIDECHAT_LIFETIME_HOURS} h")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tab_label_counts_live_sidechats() {
        assert_eq!(sidechat_tab_label(0), "New sidechat (btw)");
        assert_eq!(sidechat_tab_label(2), "Sidechats (btw) \u{b7} 2");
    }

    #[test]
    fn a_sidechat_is_titled_by_its_first_prompt_line() {
        assert_eq!(sidechat_tab_title(Some("\n  Why 2ⁿ?\nmore")), "Why 2ⁿ?");
        assert_eq!(sidechat_tab_title(Some("  ")), "New btw");
        assert_eq!(sidechat_tab_title(None), "New btw");
    }

    #[test]
    fn time_left_counts_down_from_creation() {
        let created = Utc::now();
        assert_eq!(
            sidechat_time_left(created, created + Duration::hours(3)),
            "21h left of 24 h"
        );
        assert_eq!(
            sidechat_time_left(created, created + Duration::minutes(23 * 60 + 50)),
            "10m left of 24 h"
        );
    }
}
