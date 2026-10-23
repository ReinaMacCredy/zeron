
use crate::{ChildRoute, DelegationStatus};

pub const CHARS_PER_TOKEN: f64 = 4.0;

pub fn estimated_tokens(chars: usize) -> u64 {
    (chars as f64 / CHARS_PER_TOKEN).ceil() as u64
}

/// Compact elapsed formatting, using at most two units up to days.
pub fn format_elapsed(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86_400 {
        format!("{}h {}m", secs / 3_600, (secs % 3_600) / 60)
    } else {
        format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3_600)
    }
}

/// A group summary while its advisor call is still out: the shared summary
/// has no resolution bit, and this is the one chip that waits minutes.
pub fn advisor_out(summary: &str) -> String {
    summary.replace("onsulted advisor", "onsulting advisor")
}

pub fn compact_count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.2}M", n as f64 / 1_000_000.0),
    }
}

pub fn format_kb(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

pub fn token_count(n: u64) -> String {
    let trim = |s: String| {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    let n = n as f64;
    if n >= 1e10 {
        format!("{}B", trim(format!("{:.1}", n / 1e9)))
    } else if n >= 999_500_000.0 {
        format!("{}B", trim(format!("{:.2}", n / 1e9)))
    } else if n >= 1e7 {
        format!("{}M", trim(format!("{:.1}", n / 1e6)))
    } else if n >= 999_500.0 {
        format!("{}M", trim(format!("{:.2}", n / 1e6)))
    } else if n >= 1e5 {
        format!("{:.0}K", n / 1e3)
    } else if n >= 1e3 {
        format!("{}K", trim(format!("{:.1}", n / 1e3)))
    } else {
        format!("{n}")
    }
}

pub fn delegate_status_text(status: Option<DelegationStatus>) -> &'static str {
    use DelegationStatus::*;
    match status {
        Some(Running) => "Running",
        Some(Completed) => "Completed",
        Some(Failed) => "Failed",
        Some(Cancelled | Interrupted) => "Stopped",
        None => "Unavailable",
    }
}

pub fn delegate_state_text(
    status: Option<DelegationStatus>,
    elapsed_secs: Option<i64>,
    limited: bool,
) -> String {
    match (status == Some(DelegationStatus::Running), elapsed_secs) {
        _ if limited => "Hit usage limit".into(),
        (false, Some(secs)) => format!(
            "{} in {}",
            delegate_status_text(status),
            format_elapsed(secs)
        ),
        _ => delegate_status_text(status).into(),
    }
}

pub fn route_short_reason(route: &ChildRoute) -> String {
    let reason = route.reason.as_deref().unwrap_or("route unavailable");
    match reason.strip_suffix(" out of quota") {
        Some(harness) => format!("{harness} quota"),
        None => match reason.strip_suffix(" is not available") {
            Some(harness) => format!("{harness} unavailable"),
            None => reason.to_string(),
        },
    }
}

fn handback_status_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut quoted = false;
    text.lines().filter(move |line| match *line {
        "<agent-response>" => {
            quoted = true;
            false
        }
        "</agent-response>" => {
            quoted = false;
            false
        }
        _ => !quoted,
    })
}

pub fn wake_task_ids(text: &str) -> Option<Vec<String>> {
    let ids: Vec<String> = handback_status_lines(text)
        .filter_map(|line| {
            let (id, rest) = line.strip_prefix("Delegated task ")?.split_once(" (\"")?;
            [
                "completed.",
                "failed.",
                "was cancelled.",
                "was interrupted.",
                "ended.",
            ]
            .iter()
            .any(|outcome| rest.ends_with(&format!("\") {outcome}")))
            .then(|| id.to_string())
        })
        .collect();
    if !ids.is_empty() {
        return Some(ids);
    }
    // Saved transcripts keep wakes written before the handback carried the reply.
    if let Some(rest) = text.strip_prefix("Delegated task ") {
        let (id, _) = rest.split_once(" reached a terminal state.")?;
        return Some(vec![id.to_string()]);
    }
    let rest = text.strip_prefix("Delegated tasks ")?;
    let (ids, _) = rest.split_once(" reached terminal states.")?;
    Some(ids.split(", ").map(str::to_string).collect())
}

pub fn step_handback_threads(text: &str) -> Vec<String> {
    handback_status_lines(text)
        .filter(|line| line.starts_with("Step \""))
        .filter_map(|line| {
            let (_, thread) = line.rsplit_once(" on its thread ")?;
            thread
                .strip_suffix('.')
                .filter(|thread| !thread.contains(char::is_whitespace))
                .map(str::to_string)
        })
        .collect()
}

pub fn queue_from_label(
    chat_id: &str,
    from: Option<&str>,
    text: &str,
    title: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let from = from.filter(|from| *from != chat_id)?;
    let steps = step_handback_threads(text);
    let senders = if steps.len() > 1 {
        steps
    } else {
        vec![from.to_string()]
    };
    let titles: Vec<String> = senders
        .iter()
        .map(|id| {
            let title = title(id).unwrap_or_else(|| id.clone());
            title.split_whitespace().collect::<Vec<_>>().join(" ")
        })
        .collect();
    Some(format!("from {} · held for turn end", titles.join(", ")))
}

pub fn handback_quotes(text: &str) -> Option<String> {
    if wake_task_ids(text).is_none() && step_handback_threads(text).is_empty() {
        return None;
    }
    let mut quoted = false;
    let lines: Vec<String> = text
        .lines()
        .filter_map(|line| match line {
            "<agent-response>" => {
                quoted = true;
                None
            }
            "</agent-response>" => {
                quoted = false;
                None
            }
            "" if quoted => Some(">".to_string()),
            _ if quoted => Some(format!("> {line}")),
            _ => Some(line.to_string()),
        })
        .collect();
    Some(lines.join("\n"))
}

pub fn wake_summary(tasks: &[(&str, Option<DelegationStatus>)]) -> String {
    use DelegationStatus::*;
    let outcome = if tasks.iter().any(|(_, s)| *s == Some(Failed)) {
        "failed"
    } else if tasks
        .iter()
        .any(|(_, s)| matches!(s, Some(Cancelled | Interrupted)))
    {
        "stopped"
    } else {
        "finished"
    };
    match tasks {
        [(title, _)] => format!("Delegated task \u{201c}{title}\u{201d} {outcome}"),
        many => format!(
            "{} delegated tasks {outcome}: {}",
            many.len(),
            many.iter()
                .map(|(title, _)| *title)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Keeps the routing instructions in the stored prompt for agents, but shows
/// a concise attribution in the transcript.
pub fn agent_message_display(text: &str) -> String {
    let Some(rest) = text.strip_prefix("[Message from Zeron chat ") else {
        return text.to_owned();
    };
    let Some((header, body)) = rest.split_once("]\n\n") else {
        return text.to_owned();
    };
    let Some((label, id)) =
        header.rsplit_once(". Reply to it with the Zeron `send_message` tool, chat ")
    else {
        return text.to_owned();
    };
    let Some(id) = id.strip_suffix('.') else {
        return text.to_owned();
    };
    let suffix = format!(" ({id})");
    let name = label.strip_suffix(&suffix).unwrap_or(label);
    format!("Message from {name}\n\n{body}")
}

/// User and assistant turns inside a provider-switch packet, counted the way
/// `bounded_source_copy` wrote them: one `User: ` / `Assistant: ` line each.
pub fn handoff_message_count(packet: &str) -> usize {
    packet
        .lines()
        .filter(|line| line.starts_with("User: ") || line.starts_with("Assistant: "))
        .count()
}

pub fn handoff_size_label(chars: usize) -> String {
    format!("~{} tokens", token_count(estimated_tokens(chars)))
}

pub fn handoff_context_label(packet: &str, pointer: bool) -> String {
    if packet.is_empty() {
        "None".to_string()
    } else if pointer {
        "A link to the source thread".to_string()
    } else {
        let messages = match handoff_message_count(packet) {
            1 => "1 message".to_string(),
            n => format!("{n} messages"),
        };
        format!(
            "{messages} · {}",
            handoff_size_label(packet.chars().count())
        )
    }
}

pub fn handoff_footnote(pointer: bool) -> &'static str {
    if pointer {
        "Sent ahead of your next message so the new model reads the source thread itself."
    } else {
        "Sent ahead of your next message so the new model can continue this thread."
    }
}

pub struct BranchCopy {
    pub title: &'static str,
    pub context: String,
    pub option: &'static str,
    pub detail: &'static str,
}

pub fn branch_copy(rewind: bool, excerpt: &str) -> BranchCopy {
    let excerpt = if excerpt.is_empty() {
        String::new()
    } else {
        let mut text: String = excerpt.chars().take(72).collect();
        if excerpt.chars().count() > 72 {
            text.push('\u{2026}');
        }
        format!("\u{201C}{text}\u{201D}")
    };
    if rewind {
        BranchCopy {
            title: "Rewind to here",
            context: format!("Before {excerpt}"),
            option: "Restore conversation",
            detail: "Removes this prompt and everything after it and puts the prompt back in the composer. Files on disk stay as they are.",
        }
    } else {
        BranchCopy {
            title: "Fork chat from here",
            context: format!("After {excerpt}"),
            option: "Fork in this workspace",
            detail: "New chat with the conversation through this reply. This chat stays as is.",
        }
    }
}

pub struct LimitCardCopy {
    pub title: String,
    pub resets: String,
    pub switch: Option<(String, String)>,
    pub wait: (String, String),
}

pub fn limit_duration(ms: i64) -> String {
    let mins = (ms / 60_000).max(0);
    match (mins / 60, mins % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

pub fn limit_card_copy(
    name: &str,
    reset_clock: Option<&str>,
    resets_in_ms: Option<i64>,
    target: Option<&str>,
    auto: bool,
) -> LimitCardCopy {
    let resets = match (reset_clock, resets_in_ms) {
        (Some(at), Some(ms)) if ms > 0 => format!("resets {at}, in {}", limit_duration(ms)),
        (Some(at), _) => format!("resets {at}"),
        _ => "reset time unknown".to_owned(),
    };
    LimitCardCopy {
        title: format!("{name} hit its usage limit"),
        resets,
        switch: target.map(|to| {
            (
                format!("{to} (keeps transcript)"),
                format!(
                    "Same thread, provider switched. Back to {name} only if you switch it again."
                ),
            )
        }),
        wait: (
            reset_clock.map_or_else(
                || "Wait for reset".to_owned(),
                |at| format!("Wait for reset {at}"),
            ),
            if auto {
                format!("Auto-resumes on {name} at reset, as today.")
            } else {
                "Auto-resume is not armed: the thread stops here and its parent is told.".to_owned()
            },
        ),
    }
}

pub fn limit_pill_label(
    reset_clock: Option<&str>,
    upcoming: bool,
    auto: bool,
) -> (String, Option<&'static str>) {
    match (reset_clock, upcoming) {
        (Some(when), true) if auto => (format!("Usage limit · resumes {when}"), Some("Cancel")),
        (Some(when), true) => (
            format!("Usage limit · resets {when}"),
            Some("Resume at reset"),
        ),
        _ => ("Usage limit · reset time unknown".into(), None),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionWait {
    Queued,
    Classifying {
        harness: crate::HarnessId,
        model: Option<String>,
        agent_note: bool,
    },
    Compacting(Option<(u64, bool)>),
}

impl CompactionWait {
    pub fn words(&self) -> (String, Option<&'static str>) {
        match self {
            Self::Queued => (
                "Compaction queued · starts when the turn settles".to_string(),
                None,
            ),
            Self::Classifying { agent_note, .. } => (
                "Classifier".to_string(),
                Some(if *agent_note {
                    "· checking the agent's note"
                } else {
                    "· checking if this is a new task"
                }),
            ),
            Self::Compacting(Some((tokens, carry))) => (
                format!(
                    "Compacting {}{}",
                    compact_count(*tokens),
                    if *carry {
                        ", carrying the last turns over"
                    } else {
                        " before the new task"
                    }
                ),
                None,
            ),
            Self::Compacting(None) => ("Compacting context".to_string(), None),
        }
    }

    pub fn timed(&self) -> bool {
        !matches!(self, Self::Queued)
    }
}

#[cfg(test)]
mod limit_tests {
    use super::limit_duration;

    #[test]
    fn reset_durations_read_in_hours_and_minutes() {
        assert_eq!(limit_duration(72 * 60_000), "1h 12m");
        assert_eq!(limit_duration(120 * 60_000), "2h");
        assert_eq!(limit_duration(5 * 60_000 + 59_000), "5m");
        assert_eq!(limit_duration(-1), "0m");
    }
}
