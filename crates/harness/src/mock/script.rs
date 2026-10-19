use futures::StreamExt;
use futures::stream::BoxStream;

use zeron_proto::{AgentEvent, DoneStatus, RunRequest, UserInputQuestion};

use super::{mock_chunk_chars, mock_delay, rechunk};
use crate::{HarnessError, RunControls};

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(untagged)]
enum Step {
    Sleep {
        sleep: u64,
    },
    Ask {
        ask: Vec<UserInputQuestion>,
    },
    EchoAnswers {
        #[serde(rename = "echoAnswers")]
        echo_answers: String,
    },
    AwaitInterrupt {
        #[serde(rename = "awaitInterrupt")]
        await_interrupt: bool,
    },
    Event(AgentEvent),
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct Turn {
    #[serde(default, rename = "match")]
    pattern: Option<String>,
    steps: Vec<Step>,
}

static MOCK_SCRIPT_TURNS: std::sync::Mutex<std::collections::BTreeMap<String, usize>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

fn done(status: DoneStatus) -> AgentEvent {
    AgentEvent::Done {
        status,
        result: None,
        error: None,
        session_id: None,
    }
}

fn steered() -> AgentEvent {
    AgentEvent::Steered {
        assistant_message_id: None,
        next_assistant_message_id: None,
    }
}

pub(super) fn confirm_steers(
    steering: &mut tokio::sync::mpsc::Receiver<crate::SteerMessage>,
    tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
) -> bool {
    while steering.try_recv().is_ok() {
        if tx.send(steered()).is_err() {
            return false;
        }
    }
    true
}

pub(super) fn run(
    path: &str,
    request: &RunRequest,
    controls: RunControls,
) -> BoxStream<'static, Result<AgentEvent, HarnessError>> {
    let picked = std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|content| parse_script(path, &content))
        .and_then(|turns| {
            let key = format!(
                "{path}\n{}",
                chat_id(request.mcp.as_ref()).unwrap_or_default()
            );
            let mut index = MOCK_SCRIPT_TURNS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            pick_turn(&turns, &request.prompt, index.entry(key).or_default())
                .map(turn_steps)
                .ok_or_else(|| "no turn without `match` and none matched the prompt".to_string())
        });
    match picked {
        Ok(steps) => play_turn(steps, controls),
        Err(error) => futures::stream::iter([
            Ok(AgentEvent::Error {
                message: format!("ZERON_MOCK_SCRIPT {path}: {error}"),
            }),
            Ok(done(DoneStatus::Errored)),
        ])
        .boxed(),
    }
}

fn parse_script(path: &str, content: &str) -> Result<Vec<Turn>, String> {
    if path.ends_with(".jsonl") {
        let (turns, skipped) = journal_turns(content);
        if skipped > 0 {
            tracing::warn!(path, skipped, "mock journal: skipped unparsable lines");
        }
        if turns.is_empty() {
            return Err("journal has no replayable events".into());
        }
        return Ok(turns);
    }
    let value: serde_json::Value = serde_json::from_str(content).map_err(|e| e.to_string())?;
    let is_turns = value
        .as_array()
        .and_then(|items| items.first())
        .is_some_and(|first| first.get("steps").is_some());
    if is_turns {
        let turns: Vec<Turn> = serde_json::from_str(content).map_err(|e| e.to_string())?;
        if turns.is_empty() {
            return Err("script has no turns".into());
        }
        return Ok(turns);
    }
    let steps: Vec<Step> = serde_json::from_str(content).map_err(|e| e.to_string())?;
    if steps.is_empty() {
        return Err("script has no steps".into());
    }
    Ok(vec![Turn {
        pattern: None,
        steps,
    }])
}

fn journal_turns(content: &str) -> (Vec<Turn>, usize) {
    #[derive(serde::Deserialize)]
    struct Line {
        event: AgentEvent,
    }
    let (mut turns, mut steps, mut skipped) = (Vec::new(), Vec::new(), 0);
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(Line { event }) = serde_json::from_str(line) else {
            skipped += 1;
            continue;
        };
        match event {
            AgentEvent::SessionStarted { .. }
            | AgentEvent::Anchor { .. }
            | AgentEvent::BranchPending { .. }
            | AgentEvent::SessionTombstoned {}
            | AgentEvent::InputResolved { .. } => {}
            AgentEvent::InputRequested { questions, .. } => {
                steps.push(Step::Ask { ask: questions })
            }
            // A recorded session id would become the mock chat's resume id.
            AgentEvent::Done {
                status,
                result,
                error,
                ..
            } => {
                steps.push(Step::Event(AgentEvent::Done {
                    status,
                    result,
                    error,
                    session_id: None,
                }));
                turns.push(Turn {
                    pattern: None,
                    steps: std::mem::take(&mut steps),
                });
            }
            event => steps.push(Step::Event(event)),
        }
    }
    if !steps.is_empty() {
        turns.push(Turn {
            pattern: None,
            steps,
        });
    }
    (turns, skipped)
}

fn chat_id(mcp: Option<&zeron_proto::McpServer>) -> Option<&str> {
    mcp?.args
        .windows(2)
        .find(|pair| pair[0] == "--chat")
        .map(|pair| pair[1].as_str())
}

fn pick_turn<'a>(turns: &'a [Turn], prompt: &str, next: &mut usize) -> Option<&'a Turn> {
    if let Some(turn) = turns
        .iter()
        .find(|turn| turn.pattern.as_deref().is_some_and(|p| prompt.contains(p)))
    {
        return Some(turn);
    }
    let ordered: Vec<&Turn> = turns.iter().filter(|turn| turn.pattern.is_none()).collect();
    let turn = ordered.get(*next).or(ordered.last()).copied()?;
    *next = (*next + 1).min(ordered.len());
    Some(turn)
}

fn turn_steps(turn: &Turn) -> Vec<Step> {
    let mut steps = turn.steps.clone();
    let ends = steps.iter().any(|step| {
        matches!(
            step,
            Step::Event(AgentEvent::Done { .. })
                | Step::AwaitInterrupt {
                    await_interrupt: true
                }
        )
    });
    if !ends {
        steps.push(Step::Event(done(DoneStatus::Completed)));
    }
    steps
}

fn play_turn(
    steps: Vec<Step>,
    controls: RunControls,
) -> BoxStream<'static, Result<AgentEvent, HarnessError>> {
    let delay = mock_delay();
    let chunk_chars = mock_chunk_chars();
    // A closed mailbox makes the engine replace the turn, which answers an open `ask` empty.
    let RunControls {
        request_input,
        interrupt,
        execution_lease,
        mut steering,
        ..
    } = controls;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    tokio::spawn(async move {
        let _lease = execution_lease;
        let mut labels: Vec<String> = Vec::new();
        for step in steps {
            if !confirm_steers(&mut steering, &tx) {
                return;
            }
            let events = match step {
                Step::Sleep { sleep } => {
                    tokio::time::sleep(std::time::Duration::from_millis(sleep)).await;
                    continue;
                }
                Step::Ask { ask } => {
                    labels = request_input(ask)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .flat_map(|answer| answer.labels)
                        .collect();
                    continue;
                }
                Step::AwaitInterrupt {
                    await_interrupt: false,
                } => continue,
                Step::AwaitInterrupt { .. } => loop {
                    tokio::select! {
                        _ = interrupt.cancelled() => {
                            let _ = tx.send(done(DoneStatus::Interrupted));
                            return;
                        }
                        Some(_) = steering.recv() => {
                            if tx.send(steered()).is_err() {
                                return;
                            }
                        }
                        _ = tx.closed() => return,
                    }
                },
                Step::EchoAnswers { echo_answers } => vec![AgentEvent::TextDelta {
                    text: format!("{echo_answers}{}", labels.join(", ")),
                }],
                Step::Event(event) => match chunk_chars {
                    Some(n) => rechunk(event, n),
                    None => vec![event],
                },
            };
            for event in events {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                if tx.send(event).is_err() {
                    return;
                }
            }
        }
    });
    futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|event| (Ok(event), rx))
    })
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(text: &str) -> Step {
        Step::Event(AgentEvent::TextDelta { text: text.into() })
    }

    fn turn(pattern: Option<&str>, label: &str) -> Turn {
        Turn {
            pattern: pattern.map(Into::into),
            steps: vec![text(label)],
        }
    }

    fn label(turn: Option<&Turn>) -> &str {
        match turn.map(|turn| &turn.steps[0]) {
            Some(Step::Event(AgentEvent::TextDelta { text })) => text,
            _ => "",
        }
    }

    #[test]
    fn a_step_list_is_one_turn_of_every_step_kind() {
        let script = r#"[
            {"sleep": 5},
            {"ask": [{"id": "q", "header": "H", "question": "Pick", "options": ["A", "B"]}]},
            {"echoAnswers": "Picked: "},
            {"type": "toolCall", "id": "t1", "call": {"kind": "exec", "command": "ls"}},
            {"awaitInterrupt": true}
        ]"#;
        let turns = parse_script("s.json", script).unwrap();
        assert_eq!(turns.len(), 1);
        let steps = &turns[0].steps;
        assert!(matches!(steps[0], Step::Sleep { sleep: 5 }));
        assert!(matches!(&steps[1], Step::Ask { ask } if ask[0].options == ["A", "B"]));
        assert!(
            matches!(&steps[2], Step::EchoAnswers { echo_answers } if echo_answers == "Picked: ")
        );
        assert!(matches!(&steps[3], Step::Event(AgentEvent::ToolCall { id, .. }) if id == "t1"));
        assert!(matches!(
            steps[4],
            Step::AwaitInterrupt {
                await_interrupt: true
            }
        ));
    }

    #[test]
    fn a_turn_list_keeps_its_match_and_a_bad_file_names_the_error() {
        let turns = parse_script(
            "s.json",
            r#"[{"match": "alpha", "steps": [{"type": "textDelta", "text": "a"}]}, {"steps": []}]"#,
        )
        .unwrap();
        assert_eq!(turns[0].pattern.as_deref(), Some("alpha"));
        assert_eq!(turns[1].pattern, None);
        assert!(parse_script("s.json", "[{\"type\": \"nope\"}]").is_err());
        assert!(parse_script("s.json", "[]").is_err());
        assert!(parse_script("s.json", "not json").is_err());
    }

    #[test]
    fn the_committed_scenario_mock_scripts_parse() {
        let script = include_str!("../../../../scripts/scenarios/mock-question.mock.json");
        let turns = parse_script("mock-question.mock.json", script).unwrap();
        assert!(turns.iter().all(|turn| !turn.steps.is_empty()));
        let journal = include_str!("../../../../scripts/scenarios/journal-replay.jsonl");
        let (turns, skipped) = journal_turns(journal);
        assert_eq!(skipped, 0);
        assert!(!turns.is_empty());
        assert!(parse_script("journal-replay.jsonl", journal).is_ok());
    }

    #[test]
    fn a_match_wins_without_advancing_the_ordered_index_which_wraps_to_the_last_turn() {
        let turns = [
            turn(None, "first"),
            turn(Some("alpha"), "matched"),
            turn(None, "second"),
        ];
        let (mut chat_a, mut chat_b) = (0, 0);
        assert_eq!(label(pick_turn(&turns, "go", &mut chat_a)), "first");
        assert_eq!(
            label(pick_turn(&turns, "do alpha now", &mut chat_a)),
            "matched"
        );
        assert_eq!(label(pick_turn(&turns, "go", &mut chat_b)), "first");
        assert_eq!(label(pick_turn(&turns, "go", &mut chat_a)), "second");
        assert_eq!(label(pick_turn(&turns, "go", &mut chat_a)), "second");
        assert_eq!(label(pick_turn(&turns, "go", &mut chat_b)), "second");
        assert!(pick_turn(&[turn(Some("x"), "only")], "y", &mut 0).is_none());
    }

    #[test]
    fn the_chat_id_comes_from_the_mcp_chat_arg() {
        let server = zeron_proto::McpServer {
            name: "zeron".into(),
            command: "zeron".into(),
            args: vec!["mcp".into(), "--chat".into(), "c1".into(), "--port".into()],
            env: Default::default(),
        };
        assert_eq!(chat_id(Some(&server)), Some("c1"));
        assert_eq!(chat_id(None), None);
    }

    #[test]
    fn a_journal_splits_at_untagged_done_and_drops_engine_records() {
        let journal = [
            r#"{"seq":1,"event":{"type":"sessionStarted","harness":"claude-code","model":"m","cwd":"/","sessionId":"s","assistantMessageId":"a"}}"#,
            r#"{"seq":2,"event":{"type":"anchor","role":"user","id":"u1"}}"#,
            r#"{"seq":3,"event":{"type":"textDelta","text":"hi"}}"#,
            r#"{"seq":4,"event":{"type":"inputRequested","requestId":"r","questions":[{"id":"q","header":"H","question":"Q","options":["A"]}]}}"#,
            r#"{"seq":5,"event":{"type":"inputResolved","requestId":"r"}}"#,
            r#"{"seq":6,"event":{"type":"subagent","parentToolUseId":"t","event":{"type":"done","status":"completed","result":null,"error":null,"sessionId":null}}}"#,
            r#"{"seq":7,"event":{"type":"done","status":"completed","result":"ok","error":null,"sessionId":"real-session"}}"#,
            "{\"seq\":8,\"event\":{\"type\":\"torn",
            r#"{"seq":9,"event":{"type":"branchPending","keep":null,"drop":null}}"#,
            r#"{"seq":10,"event":{"type":"sessionTombstoned"}}"#,
            r#"{"event":{"type":"toolCall","id":"t2","call":{"kind":"readFile","path":"a"}}}"#,
        ]
        .join("\n");
        let (turns, skipped) = journal_turns(&journal);
        assert_eq!(skipped, 1);
        assert_eq!(turns.len(), 2);
        let first = &turns[0].steps;
        assert_eq!(first.len(), 4);
        assert_eq!(first[0], text("hi"));
        assert!(matches!(&first[1], Step::Ask { ask } if ask[0].id == "q"));
        assert!(matches!(
            &first[2],
            Step::Event(AgentEvent::Subagent { .. })
        ));
        assert_eq!(
            first[3],
            Step::Event(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: Some("ok".into()),
                error: None,
                session_id: None,
            })
        );
        assert_eq!(turns[1].steps.len(), 1);
        let second = turn_steps(&turns[1]);
        assert_eq!(
            second.last(),
            Some(&Step::Event(done(DoneStatus::Completed)))
        );
        assert_eq!(turn_steps(&turns[0]), turns[0].steps);
    }

    #[test]
    fn an_await_interrupt_turn_gets_no_appended_done() {
        let waiting = Turn {
            pattern: None,
            steps: vec![
                text("working"),
                Step::AwaitInterrupt {
                    await_interrupt: true,
                },
            ],
        };
        assert_eq!(turn_steps(&waiting), waiting.steps);
    }

    fn steer(prompt: &str) -> crate::SteerMessage {
        crate::SteerMessage {
            prompt: prompt.into(),
            message_id: None,
            attachments: Vec::new(),
            config: None,
        }
    }

    fn controls_with(
        steering: tokio::sync::mpsc::Receiver<crate::SteerMessage>,
        answer: tokio::sync::oneshot::Receiver<Vec<zeron_proto::UserInputAnswer>>,
        asked: std::sync::Arc<tokio::sync::Notify>,
        interrupt: tokio_util::sync::CancellationToken,
    ) -> RunControls {
        let answer = std::sync::Mutex::new(Some(answer));
        RunControls {
            realtime: None,
            execution_lease: None,
            request_input: Box::new(move |_| {
                asked.notify_one();
                answer.lock().unwrap().take().unwrap()
            }),
            steering,
            interrupt,
            tool_server: None,
            stop_tasks: None,
            turn: Default::default(),
        }
    }

    #[tokio::test]
    async fn a_steer_sent_while_a_question_waits_lands_after_the_answer() {
        let (steer_tx, steering) = tokio::sync::mpsc::channel(4);
        let (answer_tx, answer) = tokio::sync::oneshot::channel();
        let turn = parse_script(
            "s.json",
            r#"[{"ask": [{"id": "q", "header": "Scope", "question": "Q", "options": ["Yes", "No"]}]},
                {"echoAnswers": "Scope: "}]"#,
        )
        .unwrap();
        let asked = std::sync::Arc::new(tokio::sync::Notify::new());
        let mut events = play_turn(
            turn_steps(&turn[0]),
            controls_with(steering, answer, asked.clone(), Default::default()),
        );
        asked.notified().await;
        steer_tx
            .send(steer("wake"))
            .await
            .expect("the mailbox stays open while the question waits");
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), events.next())
                .await
                .is_err(),
            "the steer does not end the wait"
        );
        answer_tx
            .send(vec![zeron_proto::UserInputAnswer {
                question_id: "q".into(),
                labels: vec!["Yes".into()],
            }])
            .unwrap();
        let rest: Vec<AgentEvent> = events.map(Result::unwrap).collect().await;
        assert!(
            matches!(
                &rest[..],
                [
                    AgentEvent::Steered { .. },
                    AgentEvent::TextDelta { text },
                    AgentEvent::Done { status: DoneStatus::Completed, .. },
                ] if text == "Scope: Yes"
            ),
            "{rest:?}"
        );
    }

    #[tokio::test]
    async fn a_held_turn_takes_a_steer_and_keeps_holding_until_stopped() {
        let (steer_tx, steering) = tokio::sync::mpsc::channel(4);
        let interrupt = tokio_util::sync::CancellationToken::new();
        let turn = parse_script("s.json", r#"[{"awaitInterrupt": true}]"#).unwrap();
        let mut events = play_turn(
            turn_steps(&turn[0]),
            controls_with(
                steering,
                tokio::sync::oneshot::channel().1,
                Default::default(),
                interrupt.clone(),
            ),
        );
        steer_tx.send(steer("wake")).await.expect("open mailbox");
        assert!(matches!(
            events.next().await,
            Some(Ok(AgentEvent::Steered { .. }))
        ));
        interrupt.cancel();
        assert_eq!(
            events.next().await.map(Result::unwrap),
            Some(done(DoneStatus::Interrupted))
        );
    }

    #[tokio::test]
    async fn a_turn_asks_then_echoes_the_picked_labels_and_ends_done() {
        let (_steer_tx, steering) = tokio::sync::mpsc::channel(1);
        let controls = RunControls {
            realtime: None,
            execution_lease: None,
            request_input: Box::new(|questions: Vec<UserInputQuestion>| {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = tx.send(vec![zeron_proto::UserInputAnswer {
                    question_id: questions[0].id.clone(),
                    labels: vec!["A".into(), "B".into()],
                }]);
                rx
            }),
            steering,
            interrupt: tokio_util::sync::CancellationToken::new(),
            tool_server: None,
            stop_tasks: None,
            turn: Default::default(),
        };
        let turn = parse_script(
            "s.json",
            r#"[{"ask": [{"id": "q", "header": "H", "question": "Q", "options": ["A", "B"]}]},
                {"echoAnswers": "Picked: "}]"#,
        )
        .unwrap();
        let events: Vec<AgentEvent> = play_turn(turn_steps(&turn[0]), controls)
            .map(Result::unwrap)
            .collect()
            .await;
        assert_eq!(
            events,
            [
                AgentEvent::TextDelta {
                    text: "Picked: A, B".into()
                },
                done(DoneStatus::Completed),
            ]
        );
    }
}
