use super::*;
use chrono::DateTime;

const LABEL_CHARS: usize = 120;
const PUBLISH_EVERY_MS: i64 = 10_000;

pub type ThreadActivities = HashMap<String, ThreadActivity>;

/// What a working turn last did that a person could see; reasoning is not that.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ThreadActivity {
    #[serde(rename = "activity", skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(rename = "activityAt", with = "chrono::serde::ts_seconds")]
    pub at: DateTime<Utc>,
    #[serde(
        rename = "quietSince",
        skip_serializing_if = "Option::is_none",
        with = "chrono::serde::ts_seconds_option"
    )]
    pub quiet_since: Option<DateTime<Utc>>,
    #[serde(skip)]
    by_tool: bool,
    #[serde(skip)]
    published_at: DateTime<Utc>,
}

fn first_text_line(parts: &[MessagePart]) -> Option<String> {
    parts.iter().rev().find_map(|part| match part {
        MessagePart::Text { text, .. } => text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| line.chars().take(LABEL_CHARS).collect()),
        _ => None,
    })
}

fn tool_label(call: &ToolCall) -> String {
    let (label, detail) = zeron_proto::view::tool_chip_content(call);
    let line = if detail.is_empty() {
        label.to_string()
    } else {
        format!("{label} {detail}")
    };
    line.chars().take(LABEL_CHARS).collect()
}

fn begin(all: &mut ThreadActivities, chat_id: &str, now: DateTime<Utc>) {
    all.insert(
        chat_id.to_string(),
        ThreadActivity {
            label: None,
            at: now,
            quiet_since: None,
            by_tool: false,
            published_at: now,
        },
    );
}

/// True when watchers should hear of it: a new label, a cleared mark, or a stale time.
fn note(
    all: &mut ThreadActivities,
    chat_id: &str,
    event: &AgentEvent,
    folded: &[MessagePart],
    now: DateTime<Utc>,
) -> bool {
    let tool = match event {
        AgentEvent::ToolCall { call, .. } => Some(call),
        AgentEvent::ToolResult { .. } => None,
        AgentEvent::TextDelta { text } if !text.is_empty() => None,
        _ => return false,
    };
    let Some(entry) = all.get_mut(chat_id) else {
        return false;
    };
    entry.at = now;
    let mut changed = entry.quiet_since.take().is_some();
    let label = match tool {
        Some(call) => Some(tool_label(call)),
        None if matches!(event, AgentEvent::TextDelta { .. }) && !entry.by_tool => {
            first_text_line(folded)
        }
        None => None,
    };
    if let Some(label) = label.filter(|label| entry.label.as_ref() != Some(label)) {
        entry.label = Some(label);
        changed = true;
    }
    entry.by_tool |= tool.is_some();
    if changed || (now - entry.published_at).num_milliseconds() >= PUBLISH_EVERY_MS {
        entry.published_at = now;
        return true;
    }
    false
}

fn mark(all: &mut ThreadActivities, chat_id: &str, since: DateTime<Utc>) -> bool {
    match all.get_mut(chat_id) {
        Some(entry) if entry.at == since && entry.quiet_since.is_none() => {
            entry.quiet_since = Some(since);
            true
        }
        _ => false,
    }
}

impl Inner {
    pub(super) fn begin_activity(&self, chat_id: &str) {
        self.thread_activity
            .send_modify(|all| begin(all, chat_id, Utc::now()));
    }

    pub(super) fn end_activity(&self, chat_id: &str) {
        self.thread_activity
            .send_if_modified(|all| all.remove(chat_id).is_some());
    }

    pub(super) fn note_activity(&self, chat_id: &str, event: &AgentEvent, folded: &[MessagePart]) {
        if matches!(
            event,
            AgentEvent::ToolCall { .. }
                | AgentEvent::ToolResult { .. }
                | AgentEvent::TextDelta { .. }
        ) {
            self.thread_activity
                .send_if_modified(|all| note(all, chat_id, event, folded, Utc::now()));
        }
    }
}

impl SessionsEngine {
    pub fn thread_activity(&self, chat_id: &str) -> Option<ThreadActivity> {
        self.inner.thread_activity.borrow().get(chat_id).cloned()
    }

    pub fn watch_thread_activity(&self) -> watch::Receiver<ThreadActivities> {
        self.inner.thread_activity.subscribe()
    }

    /// False when the spell that began at `since` is already marked or over.
    pub fn mark_quiet(&self, chat_id: &str, since: DateTime<Utc>) -> bool {
        self.inner
            .thread_activity
            .send_if_modified(|all| mark(all, chat_id, since))
    }

    pub fn open_tools(&self, chat_id: &str) -> usize {
        lock(&self.inner.runs).get(chat_id).map_or(0, |run| {
            run.open_tools.load(std::sync::atomic::Ordering::Acquire)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(text: &str) -> MessagePart {
        MessagePart::Text {
            id: "t".into(),
            text: text.into(),
        }
    }

    #[test]
    fn activity_names_the_newest_call_or_else_the_first_line_of_text_and_clears_a_quiet_mark() {
        let at = |s: i64| DateTime::from_timestamp(1_800_000_000 + s, 0).unwrap();
        let mut all = ThreadActivities::new();
        let delta = |text: &str| AgentEvent::TextDelta { text: text.into() };
        assert!(!note(&mut all, "c", &delta("x"), &[], at(0)));
        assert!(all.is_empty(), "no working turn, nothing to note");
        begin(&mut all, "c", at(0));
        let reasoning = AgentEvent::ReasoningDelta {
            text: "thinking".into(),
        };
        assert!(!note(&mut all, "c", &reasoning, &[], at(1)));
        assert_eq!(all["c"].at, at(0), "reasoning is no activity");
        assert!(note(
            &mut all,
            "c",
            &delta("Plan"),
            &[text("\n  Plan: write index.html\nthen test")],
            at(2)
        ));
        assert_eq!(all["c"].label.as_deref(), Some("Plan: write index.html"));
        assert!(
            !note(
                &mut all,
                "c",
                &delta(" more"),
                &[text("Plan: write index.html")],
                at(3)
            ),
            "a moment later the same line goes unannounced"
        );
        assert_eq!(all["c"].at, at(3), "but the time still moves");
        assert!(mark(&mut all, "c", at(3)));
        assert!(!mark(&mut all, "c", at(3)), "one mark per quiet spell");
        assert!(
            !mark(&mut all, "c", at(2)),
            "a spell activity already ended"
        );
        let call = AgentEvent::ToolCall {
            id: "t1".into(),
            call: ToolCall::Exec {
                command: "ls -la".into(),
            },
        };
        assert!(note(&mut all, "c", &call, &[], at(400)));
        assert_eq!(all["c"].quiet_since, None, "activity clears the mark");
        assert_eq!(all["c"].label.as_deref(), Some("Run ls -la"));
        note(&mut all, "c", &delta("ok"), &[text("Looks fine")], at(401));
        assert_eq!(
            all["c"].label.as_deref(),
            Some("Run ls -la"),
            "once the turn made a call, its newest call names it"
        );
        assert_eq!(
            serde_json::to_value(&all["c"]).unwrap(),
            serde_json::json!({ "activity": "Run ls -la", "activityAt": 1_800_000_401 })
        );
    }
}
