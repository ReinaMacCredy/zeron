
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

pub fn compact_count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.2}M", n as f64 / 1_000_000.0),
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
