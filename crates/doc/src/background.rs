use zeron_proto::background::{AgentRun, RunState};

use crate::{MessagePart, MessageRole, SessionMessageEntry, SubagentStatus};

/// Spawn chips from the latest prompt onward, plus any chip still running
/// from an earlier turn. A chip without a status predates the field and has
/// no truthful state to report, so it is left out.
pub fn agent_runs<'a>(
    transcript: impl IntoIterator<Item = &'a SessionMessageEntry>,
    turn_start: usize,
) -> Vec<AgentRun> {
    transcript
        .into_iter()
        .enumerate()
        .flat_map(|(ix, entry)| {
            entry.parts.iter().filter_map(move |part| {
                let MessagePart::Tool {
                    id,
                    call,
                    subagent_ref: Some(doc_id),
                    subagent_status: Some(status),
                    ..
                } = part
                else {
                    return None;
                };
                if !call.is_subagent_spawn() {
                    return None;
                }
                let state = match status {
                    SubagentStatus::Running => RunState::Working,
                    SubagentStatus::Done => RunState::Completed,
                    SubagentStatus::Failed => RunState::Failed,
                    SubagentStatus::Stopped => RunState::Stopped,
                };
                (ix >= turn_start || state == RunState::Working).then(|| AgentRun {
                    part_id: id.clone(),
                    doc_id: doc_id.clone(),
                    title: call.subagent_title(),
                    model: call.subagent_model().map(str::to_string),
                    reasoning: call.subagent_reasoning(),
                    state,
                    started_ms: entry.created_at,
                    delegated: false,
                    harness: None,
                })
            })
        })
        .collect()
}

pub fn latest_prompt<'a, I>(transcript: I) -> Option<usize>
where
    I: IntoIterator<Item = &'a SessionMessageEntry>,
    I::IntoIter: DoubleEndedIterator + ExactSizeIterator,
{
    transcript
        .into_iter()
        .rposition(|entry| entry.role == MessageRole::User && entry.answers_request_id.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::ToolCall;

    fn entry(
        id: &str,
        role: MessageRole,
        created_at: i64,
        parts: Vec<MessagePart>,
    ) -> SessionMessageEntry {
        SessionMessageEntry {
            id: id.into(),
            role,
            parts,
            created_at,
            device_id: "dev".into(),
            status: None,
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        }
    }

    fn tool(id: &str, call: ToolCall, status: Option<SubagentStatus>) -> MessagePart {
        MessagePart::Tool {
            id: id.into(),
            call,
            is_error: false,
            resolved: true,
            output: None,
            diff: None,
            output_ref: None,
            output_bytes: None,
            diff_ref: None,
            diff_stats: None,
            subagent_ref: Some(format!("c1--sub--{id}")),
            subagent_status: status,
            subagent_tail: None,
            html_render: None,
            delegate_route: None,
        }
    }

    fn spawn(id: &str, name: &str, status: Option<SubagentStatus>) -> MessagePart {
        tool(
            id,
            ToolCall::Unknown {
                name: name.into(),
                input: Some(serde_json::json!({ "model": "opus" })),
            },
            status,
        )
    }

    #[test]
    fn spawn_chips_of_the_latest_turn_and_earlier_running_ones_become_runs() {
        let transcript = vec![
            entry("u1", MessageRole::User, 1, vec![]),
            entry(
                "a1",
                MessageRole::Assistant,
                2,
                vec![
                    spawn(
                        "old-done",
                        "Agent: old finished",
                        Some(SubagentStatus::Done),
                    ),
                    spawn(
                        "old-live",
                        "Agent: old still going",
                        Some(SubagentStatus::Running),
                    ),
                ],
            ),
            entry("u2", MessageRole::User, 3, vec![]),
            entry(
                "a2",
                MessageRole::Assistant,
                4,
                vec![
                    spawn(
                        "new-live",
                        "Agent: audit drag",
                        Some(SubagentStatus::Running),
                    ),
                    spawn(
                        "new-fail",
                        "Agent: rewrite fixture",
                        Some(SubagentStatus::Failed),
                    ),
                    spawn("legacy", "Agent: no status", None),
                    tool(
                        "stray",
                        ToolCall::Exec {
                            command: "ls".into(),
                        },
                        Some(SubagentStatus::Running),
                    ),
                ],
            ),
        ];
        let runs = agent_runs(&transcript, latest_prompt(&transcript).unwrap_or(0));
        let ids: Vec<&str> = runs.iter().map(|run| run.part_id.as_str()).collect();
        assert_eq!(ids, ["old-live", "new-live", "new-fail"]);
        assert_eq!(runs[0].state, RunState::Working);
        assert_eq!(runs[0].started_ms, 2);
        assert_eq!(runs[1].title, "audit drag");
        assert_eq!(runs[1].model.as_deref(), Some("opus"));
        assert_eq!(runs[1].doc_id, "c1--sub--new-live");
        assert_eq!(runs[2].state, RunState::Failed);
    }

    #[test]
    fn an_answer_entry_does_not_start_a_new_turn_window() {
        let mut answer = entry("u2", MessageRole::User, 3, vec![]);
        answer.answers_request_id = Some("req".into());
        let transcript = vec![
            entry("u1", MessageRole::User, 1, vec![]),
            entry(
                "a1",
                MessageRole::Assistant,
                2,
                vec![spawn("done", "Agent: finished", Some(SubagentStatus::Done))],
            ),
            answer,
        ];
        assert_eq!(latest_prompt(&transcript), Some(0));
        assert_eq!(agent_runs(&transcript, 0).len(), 1);
    }
}
