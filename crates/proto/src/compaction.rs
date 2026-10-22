use chrono::{DateTime, Utc};

use crate::HarnessId;

// Claude's prompt cache lives at most an hour; the thresholds match Claude's own resume dialog.
const RESUME_COMPACTION_TOKENS: u64 = 100_000;
const RESUME_COMPACTION_IDLE_MINUTES: i64 = 70;

pub const RESUME_COMPACTION_TITLE: &str = "Resume with less context";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResumeCompaction {
    pub tokens: u64,
    pub idle_minutes: i64,
}

impl ResumeCompaction {
    pub fn idle_label(self) -> String {
        let minutes = self.idle_minutes;
        if minutes >= 24 * 60 {
            format!("{}d", minutes / (24 * 60))
        } else if minutes >= 60 && minutes % 60 != 0 {
            format!("{}h {}m", minutes / 60, minutes % 60)
        } else if minutes >= 60 {
            format!("{}h", minutes / 60)
        } else {
            format!("{minutes}m")
        }
    }

    pub fn body(self) -> String {
        format!(
            "{} tokens from {} ago \u{b7} cache expired",
            crate::model_options::compact_tokens(self.tokens),
            self.idle_label()
        )
    }
}

pub fn resume_compaction_due(
    harness: HarnessId,
    tokens: Option<u64>,
    last_message_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<ResumeCompaction> {
    if harness != HarnessId::ClaudeCode {
        return None;
    }
    let tokens = tokens.filter(|tokens| *tokens >= RESUME_COMPACTION_TOKENS)?;
    let idle_minutes = (now - last_message_at?).num_minutes();
    (idle_minutes >= RESUME_COMPACTION_IDLE_MINUTES).then_some(ResumeCompaction {
        tokens,
        idle_minutes,
    })
}

/// What a `CompactChat` reply tells the person: nothing once compacted (the
/// boundary entry arrives through the doc), else why not.
pub fn compact_outcome_notice(reply: &serde_json::Value) -> Option<String> {
    (reply.get("compacted") != Some(&serde_json::Value::Bool(true))).then(|| {
        reply
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Nothing to compact")
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_compaction_is_due_only_for_a_large_idle_claude_chat() {
        let now = Utc::now();
        let ago = |minutes| Some(now - chrono::Duration::minutes(minutes));
        let due = |harness, tokens, at| resume_compaction_due(harness, tokens, at, now);
        assert_eq!(
            due(HarnessId::ClaudeCode, Some(184_312), ago(145)),
            Some(ResumeCompaction {
                tokens: 184_312,
                idle_minutes: 145
            })
        );
        assert!(due(HarnessId::ClaudeCode, Some(100_000), ago(70)).is_some());
        assert!(due(HarnessId::ClaudeCode, Some(99_999), ago(145)).is_none());
        assert!(due(HarnessId::ClaudeCode, Some(184_312), ago(69)).is_none());
        assert!(due(HarnessId::ClaudeCode, None, ago(145)).is_none());
        assert!(due(HarnessId::ClaudeCode, Some(184_312), None).is_none());
        assert!(due(HarnessId::Codex, Some(184_312), ago(145)).is_none());
    }

    #[test]
    fn resume_compaction_idle_label_reads_in_hours_and_days() {
        let label = |idle_minutes| {
            ResumeCompaction {
                tokens: 0,
                idle_minutes,
            }
            .idle_label()
        };
        assert_eq!(label(45), "45m");
        assert_eq!(label(120), "2h");
        assert_eq!(label(145), "2h 25m");
        assert_eq!(label(3 * 24 * 60 + 5), "3d");
        let offer = ResumeCompaction {
            tokens: 184_312,
            idle_minutes: 145,
        };
        assert_eq!(
            offer.body(),
            "184K tokens from 2h 25m ago \u{b7} cache expired"
        );
    }

    #[test]
    fn a_compact_reply_speaks_only_when_nothing_was_compacted() {
        assert_eq!(
            compact_outcome_notice(&serde_json::json!({"compacted": true})),
            None
        );
        assert_eq!(
            compact_outcome_notice(&serde_json::json!({"compacted": false})).unwrap(),
            "Nothing to compact"
        );
        assert_eq!(
            compact_outcome_notice(
                &serde_json::json!({"compacted": false, "message": "Chat is busy"})
            )
            .unwrap(),
            "Chat is busy"
        );
    }
}
