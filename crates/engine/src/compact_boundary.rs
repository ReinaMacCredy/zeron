//! Prompt and input shape are benched in .maestro/topic/compact-nudge-bench (dataset/tune: 0 of 96 holdout follow-up runs NEW); change them only with a rerun.

use zeron_doc::{
    ClassifierRecord, ClassifierVerdict, MessagePart, MessageRole, SessionMessageEntry,
};
use zeron_proto::HarnessId;

use crate::predictions::{entry_text, is_person_prompt, tail};
use crate::registry::{HarnessRegistry, TitleSettings};
use crate::titles::{OneShot, OneShotTarget, mid_tier_model, one_shot_target, run_one_shot};

const INSTRUCTIONS: &str = "You decide whether a chat should be compacted before its next message is answered. Compacting replaces the conversation so far with a short summary of its conclusions; the specific details (numbers, steps, file contents, exact wording) are lost.

Messages are often informal Vietnamese with typos.

Answer NEW only when the new message starts a different task or topic whose work will not need those details, only the conclusions.
Answer CONTINUE when it is a follow-up, a clarification, a correction, a question about something just said, or the next step of the same task.
These are CONTINUE even when they open with \"ok\", \"giờ\", \"now\" or \"next\": approving or choosing what the agent just proposed (\"ok làm đi\", \"oke đó làm đi\", \"yes 1\", \"làm luôn\"); asking to test, run, open, merge or ship the work just done; reporting a result, a problem or a doubt about it; asking how to do the next step of the plan being discussed; asking to act on what the agent just found or explained.

A new task usually comes right after a finished one: the agent's last reply reports the work done (merged, committed, tests green, cleaned up, or a complete answer) and the user then asks for a different deliverable instead of reacting to that reply. That is NEW even inside the same project or app, for example another feature next to the one just built, or a different question in the same domain, as long as the new work can start from a summary of what exists. It stays CONTINUE when the request builds on specifics from the recent turns (a command, a number, a finding, an option or next step the agent offered).

When unsure, answer CONTINUE: a missed compaction is retried later, a wrong one loses details for good.

Reply with exactly one word: NEW or CONTINUE.";

const TURNS: usize = 4;
const CLIP_CHARS: usize = 800;

fn clip(text: &str) -> String {
    if text.chars().count() <= CLIP_CHARS {
        text.to_string()
    } else {
        format!("...{}", tail(text, CLIP_CHARS))
    }
}

fn last_text(entry: &SessionMessageEntry) -> Option<&str> {
    entry.parts.iter().rev().find_map(|part| match part {
        MessagePart::Text { text, .. } if !text.trim().is_empty() => Some(text.trim()),
        _ => None,
    })
}

pub(crate) fn classifier_input(earlier: &[SessionMessageEntry], message: &str) -> String {
    let mut turns: Vec<(String, Option<&str>)> = Vec::new();
    for entry in earlier {
        if is_person_prompt(entry) {
            let text = entry_text(entry);
            if !text.is_empty() {
                turns.push((text, None));
            }
        } else if entry.role == MessageRole::Assistant
            && let Some(turn) = turns.last_mut()
            && let Some(text) = last_text(entry)
        {
            turn.1 = Some(text);
        }
    }
    let mut lines = vec!["Recent conversation:".to_string()];
    for (user, agent) in &turns[turns.len().saturating_sub(TURNS)..] {
        lines.push(format!("user: {}", clip(user)));
        if let Some(agent) = agent {
            lines.push(format!("agent: {}", clip(agent)));
        }
    }
    lines.extend([
        String::new(),
        "New message:".into(),
        format!("user: {}", message.trim()),
    ]);
    lines.join("\n")
}

fn verdict(raw: &str) -> String {
    raw.trim().to_ascii_uppercase()
}

pub(crate) async fn classifier_target(
    registry: &HarnessRegistry,
    settings: TitleSettings,
    session: HarnessId,
) -> Option<OneShotTarget> {
    one_shot_target(
        registry,
        settings.harness,
        settings.model,
        session,
        mid_tier_model,
    )
    .await
}

/// Any failure reads as a follow-up: a missed compaction is retried later.
pub(crate) async fn classify(target: OneShotTarget, input: String) -> ClassifierRecord {
    let started = std::time::Instant::now();
    let mut record = ClassifierRecord {
        harness: Some(target.harness_id),
        model: target.model.clone(),
        input: input.clone(),
        ..Default::default()
    };
    match run_one_shot(
        target.harness.as_ref(),
        target.lease,
        OneShot {
            harness: target.harness_id,
            model: target.model,
            prompt: input,
            instructions: INSTRUCTIONS,
            attempt_timeout: std::time::Duration::from_secs(20),
            retry_delays_ms: &[],
        },
        verdict,
    )
    .await
    {
        Ok(reply) => {
            record.verdict = if reply.text.starts_with("NEW") {
                ClassifierVerdict::New
            } else {
                ClassifierVerdict::Continue
            };
            record.reply = reply.text;
            record.model = record.model.take().or(reply.model);
            record.input_tokens = reply.usage.map(|(input, _)| input);
            record.output_tokens = reply.usage.map(|(_, output)| output);
        }
        Err(err) => {
            tracing::debug!(error = %err, "boundary classifier skipped");
            record.reply = err.to_string();
        }
    }
    record.duration_ms = started.elapsed().as_millis() as u64;
    record
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(role: MessageRole, text: &str) -> SessionMessageEntry {
        SessionMessageEntry {
            id: text.into(),
            role,
            parts: vec![MessagePart::Text {
                id: format!("{text}:text"),
                text: text.into(),
            }],
            created_at: 0,
            device_id: "d".into(),
            status: None,
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        }
    }

    #[test]
    fn input_keeps_the_last_turns_with_the_agents_last_words() {
        let mut entries: Vec<_> = (0..5)
            .map(|n| entry(MessageRole::User, &format!("ask {n}")))
            .collect();
        let mut thread = entry(MessageRole::User, "from another thread");
        thread.from_chat_id = Some("other".into());
        entries.push(thread);
        entries.push(entry(MessageRole::Assistant, "first part"));
        entries.push(entry(MessageRole::Assistant, &"x".repeat(900)));
        let input = classifier_input(&entries, " next thing ");
        let expected = format!(
            "Recent conversation:\nuser: ask 1\nuser: ask 2\nuser: ask 3\nuser: ask 4\nagent: ...{}\n\nNew message:\nuser: next thing",
            "x".repeat(800)
        );
        assert_eq!(input, expected);
    }
}
