//! Mock harness for engine/UI tests: replays a scripted event sequence.

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use zeron_proto::{
    AccessMode, AgentEvent, Compaction, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest,
    SteeringMode, UserInputQuestion,
};

use crate::{Harness, HarnessError, RunControls};

mod script;

pub struct MockHarness {
    pub script: Vec<AgentEvent>,
}

/// Dev knobs: `ZERON_MOCK_COMPACT=claude|codex`, `ZERON_MOCK_CONTEXT` tokens, `ZERON_MOCK_SHELL=1`, `ZERON_MOCK_COMPACT_MS`, `ZERON_MOCK_SUGGESTION=<text>` after Done (`ZERON_MOCK_SUGGESTION_MS` later: 0 is Claude's shape, more is Codex's).
#[derive(Clone, Copy, PartialEq, Eq)]
enum CompactKnob {
    Claude,
    Codex,
}

fn compact_knob() -> Option<CompactKnob> {
    match std::env::var("ZERON_MOCK_COMPACT").ok().as_deref() {
        Some("claude") => Some(CompactKnob::Claude),
        Some("codex") => Some(CompactKnob::Codex),
        _ => None,
    }
}

fn knob_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

const KNOB_POST_TOKENS: u64 = 69_000;

fn compacted_events(session_id: String) -> Vec<AgentEvent> {
    vec![
        AgentEvent::Compacted {
            pre_tokens: Some(knob_number("ZERON_MOCK_CONTEXT", 212_000)),
            post_tokens: Some(KNOB_POST_TOKENS),
        },
        AgentEvent::Done {
            status: DoneStatus::Completed,
            result: Some(String::new()),
            error: None,
            session_id: Some(session_id),
        },
    ]
}

fn compact_knob_run(
    knob: CompactKnob,
    request: RunRequest,
) -> BoxStream<'static, Result<AgentEvent, HarnessError>> {
    static TURNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let session_id = request
        .resume
        .clone()
        .unwrap_or_else(|| format!("mock-{}", uuid_like()));
    let compacting = request.prompt.starts_with("/compact");
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    tokio::spawn(async move {
        let _ = tx.send(AgentEvent::SessionStarted {
            harness: HarnessId::Mock,
            model: "mock-1".into(),
            tools: vec![],
            cwd: request.cwd.clone(),
            session_id: session_id.clone(),
            assistant_message_id: uuid_like(),
        });
        if compacting {
            let _ = tx.send(AgentEvent::Compacting { active: true });
            tokio::time::sleep(std::time::Duration::from_millis(knob_number(
                "ZERON_MOCK_COMPACT_MS",
                30_000,
            )))
            .await;
            for event in compacted_events(session_id) {
                let _ = tx.send(event);
            }
            return;
        }
        let shells = std::env::var("ZERON_MOCK_SHELL").is_ok_and(|v| v == "1");
        let _ = tx.send(AgentEvent::BackgroundTasks {
            tasks: shells
                .then(|| zeron_proto::BackgroundTask {
                    id: "mock-shell".into(),
                    command: "npm run dev".into(),
                    started_at: chrono::Utc::now(),
                    ..Default::default()
                })
                .into_iter()
                .collect(),
        });
        tokio::time::sleep(mock_delay()).await;
        for tokens in [18_000, knob_number("ZERON_MOCK_CONTEXT", 212_000)] {
            let _ = tx.send(AgentEvent::ContextUsage {
                tokens: Some(tokens),
                window: Some(1_000_000),
            });
        }
        let _ = tx.send(AgentEvent::TextDelta {
            text: format!(
                "Done: {}",
                request.prompt.lines().last().unwrap_or_default()
            ),
        });
        if knob == CompactKnob::Codex {
            let turn = TURNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let _ = tx.send(AgentEvent::Anchor {
                role: zeron_proto::AnchorRole::Assistant,
                id: format!("mock-turn-{turn}"),
                entry_id: None,
            });
        }
        let _ = tx.send(AgentEvent::Done {
            status: DoneStatus::Completed,
            result: None,
            error: None,
            session_id: Some(session_id),
        });
    });
    futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|event| (Ok(event), rx))
    })
    .boxed()
}

fn uuid_like() -> String {
    format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

fn mock_usage() -> bool {
    std::env::var("ZERON_MOCK_USAGE")
        .ok()
        .is_some_and(|v| !v.is_empty() && v != "0")
}

static MOCK_COST_RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The message a `/error <message>` prompt fails the run with.
fn scripted_error(prompt: &str) -> Option<&str> {
    let message = prompt.trim().strip_prefix("/error ")?.trim();
    (!message.is_empty()).then_some(message)
}

/// The scripted question set for the `ZERON_MOCK_QUESTION` variant (exercises
/// the QuestionPanel end-to-end: single-select page, multi-select page).
fn question_script() -> Vec<UserInputQuestion> {
    vec![
        UserInputQuestion {
            approval: false,
            id: "q-sync".into(),
            header: "Question".into(),
            question: "Which sync strategy should the rewrite use?".into(),
            options: vec![
                "Poll the doc host every 120ms".into(),
                "Event-driven fold with coalesced commits".into(),
                "Hybrid: event-driven with a polling fallback".into(),
            ],
            prefill: None,
            multiline: false,
            multi_select: false,
            kind: None,
        },
        UserInputQuestion {
            approval: false,
            id: "q-gates".into(),
            header: "Question".into(),
            question: "Which suites should gate the merge?".into(),
            options: vec![
                "Unit tests".into(),
                "End-to-end (two-device)".into(),
                "Golden screenshots".into(),
            ],
            prefill: None,
            multiline: false,
            multi_select: true,
            kind: None,
        },
    ]
}

const MOCK_PLAN: &str = "Here's the plan. Nothing has changed yet.\n\n\
1. Add `AccessMode` to the run request and the chat's config.\n\
2. Map each mode onto the agent's approval policy and sandbox.\n\
3. Ask the person for anything the mode does not cover.\n\n\
Turn plan mode off to start.";

#[async_trait]
impl Harness for MockHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Mock"
    }
    fn supports_steering(&self) -> bool {
        true
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::StepBoundary
    }
    /// Only the long-tool runtime (`ZERON_MOCK_LONG_TOOL`) stops a turn in
    /// place, as Claude's and Codex's do.
    fn stops_turn_in_place(&self) -> bool {
        long_tool_mode()
    }
    /// (`ZERON_MOCK_STEER_WAITS=1`: a steer waits for the command instead,
    /// as every steer did before, for timing one against the other.)
    fn steer_stops_tool(&self) -> bool {
        long_tool_mode()
            && !std::env::var("ZERON_MOCK_STEER_WAITS")
                .ok()
                .is_some_and(|v| !v.is_empty() && v != "0")
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[ReasoningLevel::Medium]
    }
    fn access_modes(&self) -> &[AccessMode] {
        &AccessMode::ALL
    }
    fn supports_plan(&self) -> bool {
        true
    }
    fn compaction(&self) -> Compaction {
        match compact_knob() {
            Some(CompactKnob::Claude) => Compaction::SlashText("/compact".into()),
            Some(CompactKnob::Codex) => Compaction::Native,
            None => Compaction::None,
        }
    }
    fn compacts_in_place(&self) -> bool {
        compact_knob() == Some(CompactKnob::Claude)
    }
    fn compacts_a_copy(&self) -> bool {
        compact_knob() == Some(CompactKnob::Codex)
    }
    async fn compact(
        &self,
        request: RunRequest,
        _controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        if compact_knob().is_none() {
            return Err(HarnessError::Protocol(
                "compaction is not supported by this harness".into(),
            ));
        }
        let base = request.resume.clone().unwrap_or_default();
        let session_id = if request.branch.is_some() {
            format!("{base}-copy-{}", uuid_like())
        } else {
            base
        };
        let wait = std::time::Duration::from_millis(knob_number("ZERON_MOCK_COMPACT_MS", 30_000));
        Ok(futures::stream::once(async move {
            tokio::time::sleep(wait).await;
            futures::stream::iter(compacted_events(session_id).into_iter().map(Ok))
        })
        .flatten()
        .boxed())
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![
            Model {
                id: "mock-1".into(),
                label: "Mock 1".into(),
                description: None,
                reasoning_levels: vec![ReasoningLevel::Medium],
                options: vec![],
            },
            // Claude-mirroring demo model: lets scripted runs carry the same
            // chip labels ("Fable 5 · High") as a real Claude session.
            Model {
                id: "mock-fable-5".into(),
                label: "Fable 5".into(),
                description: None,
                reasoning_levels: vec![
                    ReasoningLevel::Low,
                    ReasoningLevel::Medium,
                    ReasoningLevel::High,
                    ReasoningLevel::XHigh,
                ],
                options: vec![],
            },
        ])
    }
    async fn run_one_shot(
        &self,
        request: RunRequest,
        controls: RunControls,
        instructions: &'static str,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        // The demo run calls tools, which a commit message's one-shot refuses.
        if instructions == crate::COMMIT_MESSAGE_INSTRUCTIONS {
            return Ok(futures::stream::iter([
                Ok(AgentEvent::TextDelta {
                    text: "Mock commit message\n\nWritten by the mock harness for the staged change."
                        .into(),
                }),
                Ok(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: None,
                }),
            ])
            .boxed());
        }
        // Titles, commit messages and compaction must not consume script turns or hang on awaitInterrupt.
        let request = RunRequest {
            access: zeron_proto::AccessMode::FullAccess,
            plan: false,
            ..request
        };
        self.knob_run(request, controls).await
    }

    async fn run(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        if let Some(knob) = compact_knob() {
            return Ok(compact_knob_run(knob, request));
        }
        match std::env::var("ZERON_MOCK_SCRIPT") {
            Ok(path) if !path.is_empty() => Ok(script::run(&path, &request, controls)),
            _ => self.knob_run(request, controls).await,
        }
    }
}

impl MockHarness {
    async fn knob_run(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        // Dev/testing knob: `ZERON_MOCK_TOOL_SERVER_FILE=<path>` writes the
        // run's `zeron mcp` server (command, args, run-secret env) as JSON, so
        // a proof can call the thread tools exactly as a live agent would.
        if let (Ok(path), Some(server)) = (
            std::env::var("ZERON_MOCK_TOOL_SERVER_FILE"),
            request.mcp.as_ref(),
        ) && let Ok(json) = serde_json::to_vec(server)
        {
            let _ = std::fs::write(path, json);
        }

        // Optional pacing knob for demos/manual testing: `ZERON_MOCK_DELAY_MS`
        // spaces the scripted events out so live-run UI states (working
        // indicator, streaming fade, trailing tool-group auto-open) are
        // observable. Unset (the default, and in tests) streams instantly.
        let delay = mock_delay();

        // Dev/testing knob: `ZERON_MOCK_QUESTION=1` swaps in a run that asks
        // the user questions mid-stream via `controls.request_input` (the
        // engine mints the request id, emits `InputRequested`, and resolves it
        // from the `RespondInput` doc command) — the only data-side way to put
        // the QuestionPanel on screen.
        let question_mode = std::env::var("ZERON_MOCK_QUESTION")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0");
        if question_mode {
            let request_input = controls.request_input;
            let mut steering = controls.steering;
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
            tokio::spawn(async move {
                let pause = if delay.is_zero() {
                    std::time::Duration::from_millis(50)
                } else {
                    delay
                };
                tokio::time::sleep(pause).await;
                let _ = tx.send(AgentEvent::TextDelta {
                    text:
                        "Before I wire the reconciliation path I need two decisions from you.\n\n"
                            .into(),
                });
                tokio::time::sleep(pause).await;
                let answers = request_input(question_script()).await.unwrap_or_default();
                if !script::confirm_steers(&mut steering, &tx) {
                    return;
                }
                let picked: Vec<String> = answers
                    .iter()
                    .flat_map(|a| a.labels.iter().cloned())
                    .collect();
                tokio::time::sleep(pause).await;
                let _ = tx.send(AgentEvent::TextDelta {
                    text: format!(
                        "Locked in: **{}**. Proceeding with the plan.",
                        if picked.is_empty() {
                            "your defaults".to_string()
                        } else {
                            picked.join("**, **")
                        }
                    ),
                });
                let _ = tx.send(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: None,
                });
            });
            let stream = futures::stream::unfold(rx, |mut rx| async move {
                rx.recv().await.map(|event| (Ok(event), rx))
            });
            return Ok(stream.boxed());
        }

        // Plan mode proposes and changes nothing; a mode short of full access
        // asks before the script's command, the way a real agent's approval
        // reaches the person.
        if request.plan || !request.access.is_full_access() {
            let request_input = controls.request_input;
            let plan = request.plan;
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
            tokio::spawn(async move {
                let pause = delay.max(std::time::Duration::from_millis(50));
                let send = |text: &str| {
                    let _ = tx.send(AgentEvent::TextDelta { text: text.into() });
                };
                if plan {
                    tokio::time::sleep(pause).await;
                    send(MOCK_PLAN);
                } else {
                    send("I'll run the test suite first.\n\n");
                    tokio::time::sleep(pause).await;
                    let question = crate::approval::command("Mock", "cargo test --workspace");
                    let answers = request_input(vec![question.clone()])
                        .await
                        .unwrap_or_default();
                    if crate::approval::approved(&question, &answers) {
                        let _ = tx.send(AgentEvent::ToolCall {
                            id: "mock-approved-tool".into(),
                            call: zeron_proto::ToolCall::Exec {
                                command: "cargo test --workspace".into(),
                            },
                        });
                        tokio::time::sleep(pause).await;
                        let _ = tx.send(AgentEvent::ToolResult {
                            id: "mock-approved-tool".into(),
                            is_error: false,
                            output: Some("test result: ok. 128 passed; 0 failed".into()),
                            diff: None,
                            exit_code: None,
                        });
                        send("All 128 tests pass.");
                    } else {
                        send("Understood, I won't run it.");
                    }
                }
                let _ = tx.send(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: None,
                });
            });
            let stream = futures::stream::unfold(rx, |mut rx| async move {
                rx.recv().await.map(|event| (Ok(event), rx))
            });
            return Ok(stream.boxed());
        }

        // Dev/testing knob: `ZERON_MOCK_BACKGROUND=1` starts three background
        // processes that outlive the turn — the only data-side way to fill the
        // background tray. The runtime parks like a real agent's: the
        // processes print output, a host stop ends one, later messages are
        // answered in place, and a teardown takes them all.
        if std::env::var("ZERON_MOCK_BACKGROUND")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0")
        {
            return Ok(background_run(controls, delay));
        }

        // Dev/testing knob: `ZERON_MOCK_LONG_TOOL=1` is a runtime with real
        // processes, for timing steers against a blocking command.
        if long_tool_mode() {
            return Ok(long_tool_run(
                request.prompt.clone(),
                request.attachments.clone(),
                controls,
            ));
        }

        // `/error <message>`: the run fails with that message (error cards
        // for any provider failure, without a real provider).
        if let Some(message) = scripted_error(&request.prompt) {
            let events = vec![
                Ok(AgentEvent::Error {
                    message: message.to_string(),
                }),
                Ok(AgentEvent::Done {
                    status: DoneStatus::Errored,
                    result: None,
                    error: None,
                    session_id: None,
                }),
            ];
            return Ok(futures::stream::iter(events).boxed());
        }
        // Dev/testing knob: `ZERON_MOCK_REPEAT=N` loops the script body N times
        // before the final Done — long single-reply streams for frame-cost /
        // smoothness measurement (the terminal `Done` is emitted exactly once,
        // at the very end).
        let repeat = std::env::var("ZERON_MOCK_REPEAT")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1)
            .max(1);
        // Dev/testing knob: `ZERON_MOCK_ERROR=1` appends a scripted error
        // before the terminal Done — the only data-side way to put the
        // transcript ErrorChip on screen with the mock harness.
        let mock_error = std::env::var("ZERON_MOCK_ERROR")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0");
        // Dev/testing knob: `ZERON_MOCK_TABLE=1` appends scripted GFM tables
        // before the terminal Done — a plain 3-column grid plus a wide/uneven
        // one (long prose cell beside short cells, mixed alignment) for
        // table-styling checks against the reference app.
        let mock_table = std::env::var("ZERON_MOCK_TABLE")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0");
        let done_ix = self
            .script
            .iter()
            .position(|e| matches!(e, AgentEvent::Done { .. }))
            .unwrap_or(self.script.len());
        let (body, tail) = self.script.split_at(done_ix);
        let usage_events = mock_usage()
            .then(|| {
                let runs = MOCK_COST_RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                [
                    AgentEvent::PromptCache {
                        read_tokens: 49_648,
                        write_tokens: 3_719,
                        ttl_secs: Some(3600),
                    },
                    AgentEvent::Cost {
                        session_id: "mock".into(),
                        total_usd: 0.0145358 * runs as f64,
                    },
                    AgentEvent::Usage {
                        input_tokens: 18,
                        output_tokens: 288,
                    },
                ]
            })
            .into_iter()
            .flatten();
        let error_event = mock_error.then(|| AgentEvent::Error {
            message: "Claude usage limit reached — try again after the limit resets.".into(),
        });
        // Dev/testing knob: `ZERON_MOCK_LIMIT_SECS=N` stops each turn on a
        // usage limit that resets N seconds out; the resume turn runs clean.
        let limit_events = std::env::var("ZERON_MOCK_LIMIT_SECS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|_| request.prompt.trim() != zeron_proto::RESUME_PROMPT)
            .filter(|_| {
                limit_matches(
                    &request.prompt,
                    std::env::var("ZERON_MOCK_LIMIT_MATCH").ok().as_deref(),
                )
            })
            .map(|secs| {
                [
                    AgentEvent::Error {
                        message: "Claude 5-hour limit reached — the turn was blocked. Try again after it resets."
                            .into(),
                    },
                    AgentEvent::UsageLimit {
                        resets_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .ok()
                            .and_then(|now| i64::try_from(now.as_millis()).ok())
                            .map(|now| now + secs * 1000),
                    },
                ]
            })
            .into_iter()
            .flatten();
        // Dev/testing knob: `ZERON_MOCK_CODE=1` appends rust + ts code blocks
        // (keywords, strings, numbers, comments) plus inline code — for
        // syntax-palette and inline-code styling checks against the reference.
        let reply_event = std::env::var("ZERON_MOCK_REPLY")
            .ok()
            .filter(|text| !text.is_empty())
            .map(|text| AgentEvent::TextDelta {
                text: format!("\n\n{text}"),
            });
        let mock_code = std::env::var("ZERON_MOCK_CODE")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0");
        let code_event = mock_code.then(|| AgentEvent::TextDelta {
            text: concat!(
                "\n### Code check\n\n",
                "The `fold_event_into_parts` helper feeds `writer.sync` on a `120ms` cadence:\n\n",
                "```rust\n",
                "// Fold one event into the accumulated parts.\n",
                "pub fn fold(mut acc: Vec<Part>, event: &AgentEvent) -> Vec<Part> {\n",
                "    let label = \"delta\";\n",
                "    if acc.len() > 128 {\n",
                "        acc.truncate(64); // keep the tail hot\n",
                "    }\n",
                "    acc\n",
                "}\n",
                "```\n\n",
                "```ts\n",
                "// Subscribe and fold on the client.\n",
                "const room = await connect(\"wss://mesh.local\", { retries: 3 });\n",
                "export function fold(parts: Part[], event: AgentEvent): Part[] {\n",
                "    return event.kind === \"delta\" ? [...parts, event] : parts;\n",
                "}\n",
                "```\n\n",
            )
            .into(),
        });
        let table_event = mock_table.then(|| AgentEvent::TextDelta {
            text: "\n### Table check\n\n\
                | Column A | Column B | Column C |\n\
                |---|---|---|\n\
                | a1 | b1 | c1 |\n\
                | a2 | b2 | c2 |\n\n\
                And a wide, uneven one:\n\n\
                | Stage | What happens | p95 |\n\
                |:--|:--|--:|\n\
                | Fold | Events fold into parts and diff into the Loro doc on a 120ms coalesced commit cadence, keeping the oplog RLE-merged across devices | 4.2ms |\n\
                | Sync | Session-room fan-out | 18ms |\n\n"
                .into(),
        });
        // Dev/testing knob: `ZERON_MOCK_MEND=1` appends a link/list-heavy
        // passage — bold-led list items, inline links, emphasis, strikethrough
        // — the shapes whose half-streamed markers the display mend
        // (crates/ui markdown/mend.rs) must hold steady while streaming.
        let mock_mend = std::env::var("ZERON_MOCK_MEND")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0");
        let mend_event = mock_mend.then(|| AgentEvent::TextDelta {
            text: concat!(
                "\n### Streaming mend check\n\n",
                "Inline styles hold while text arrives: **bold stays bold**, ",
                "*italic stays italic*, `code stays code`, and ~~this stays struck~~.\n\n",
                "- **Fold** — parts diff into the [Loro doc](https://loro.dev) on a 120ms cadence\n",
                "- **Relay** — commits fan out through the [session room](https://developers.cloudflare.com/durable-objects/) to every device\n",
                "- **Paint** — the [display tree](https://github.com/pulldown-cmark/pulldown-cmark) mends hanging markers in the last block only\n\n",
                "Links above never flash their URLs, and closing markers never reflow the paragraph.\n",
            )
            .into(),
        });
        // Dev/testing knob: `ZERON_MOCK_THINKING=1` appends a markdown-heavy
        // reasoning stream plus a command — the thought-process chip inside a
        // tool-group accordion, for checking that thinking renders styled
        // (bold/lists/inline code) instead of literal `**` markers.
        let mock_thinking = std::env::var("ZERON_MOCK_THINKING")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0");
        let thinking_events = mock_thinking
            .then(|| {
                [
                    AgentEvent::ReasoningDelta {
                        text: concat!(
                            "**Planning message rollback helper extraction**\n\n",
                            "The `discard_message` helper needs to roll back optimistic sends on error. ",
                            "Three constraints stand out:\n\n",
                            "1. **Rollback** — remove the user message only when the server never persisted it\n",
                            "2. **Credit state** — clear the exhaustion flag per workspace on auth changes\n",
                            "3. **Paywall copy** — resolve icon conflicts, *then* update the tests\n\n",
                            "Checking call sites before patching:\n\n",
                            "```rust\n",
                            "let rolled_back = ledger.discard(message_id)?;\n",
                            "```\n\n",
                            "The [ledger docs](https://example.com) say discard is idempotent, so ",
                            "re-running the rollback on a retry is safe.",
                        )
                        .into(),
                    },
                    AgentEvent::ToolCall {
                        id: "mock-think-tool".into(),
                        call: zeron_proto::ToolCall::Exec {
                            command: "rg -n walletInsufficient apps/word/src | wc -l".into(),
                        },
                    },
                    AgentEvent::ToolResult {
                        id: "mock-think-tool".into(),
                        is_error: false,
                        output: None,
                        diff: None,
                        exit_code: None,
                    },
                ]
            })
            .into_iter()
            .flatten();
        // Dev/testing knob: `ZERON_MOCK_HTML_RENDER=<page.html>` publishes
        // that page through the run's real `zeron mcp` server (engine store,
        // tool-part reference) and appends the matching tool call; `pick`, a
        // page of options to pick (`data-zeron-pick`); `path:<file>`, a file
        // in the chat's folder shown by its `path`.
        let html_render_events = match (
            std::env::var("ZERON_MOCK_HTML_RENDER").ok(),
            request.mcp.as_ref(),
        ) {
            (Some(page), Some(server)) if !page.is_empty() => {
                let height: u32 = std::env::var("ZERON_MOCK_HTML_HEIGHT")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(600);
                let (input, preview_input) = mock_page_input(&page, height);
                let preview = mock_mcp_call(server, "html_preview", preview_input.clone()).await;
                if let Ok(path) = std::env::var("ZERON_MOCK_HTML_PREVIEW_PNG") {
                    let _ = std::fs::write(format!("{path}.txt"), mock_mcp_text(&preview));
                    if let Some(png) = preview
                        .as_ref()
                        .ok()
                        .and_then(|result| result["content"][1]["data"].as_str())
                    {
                        use base64::Engine as _;
                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(png) {
                            let _ = std::fs::write(path, bytes);
                        }
                    }
                }
                let result = mock_mcp_call(server, "html_render", input.clone()).await;
                vec![
                    AgentEvent::ToolCall {
                        id: "mock-html-preview".into(),
                        call: zeron_proto::ToolCall::Mcp {
                            server: "zeron".into(),
                            tool: "html_preview".into(),
                            input: Some(preview_input),
                        },
                    },
                    AgentEvent::ToolResult {
                        id: "mock-html-preview".into(),
                        is_error: mock_mcp_failed(&preview),
                        output: Some(mock_mcp_text(&preview)),
                        diff: None,
                        exit_code: None,
                    },
                    AgentEvent::ToolCall {
                        id: "mock-html-render".into(),
                        call: zeron_proto::ToolCall::Mcp {
                            server: "zeron".into(),
                            tool: "html_render".into(),
                            input: Some(input),
                        },
                    },
                    AgentEvent::ToolResult {
                        id: "mock-html-render".into(),
                        is_error: mock_mcp_failed(&result),
                        output: Some(mock_mcp_text(&result)),
                        diff: None,
                        exit_code: None,
                    },
                ]
            }
            _ => Vec::new(),
        };
        let mut delegate_events = Vec::new();
        if let (Some(json), Some(server)) = (
            std::env::var("ZERON_MOCK_DELEGATE").ok(),
            request.mcp.as_ref(),
        ) {
            let calls = delegate_calls(&json, &request.prompt, script::chat_id(Some(server)));
            for (ix, input) in calls.into_iter().enumerate() {
                let id = format!("mock-delegate-{ix}");
                let result = mock_mcp_call(server, "delegate_task", input.clone()).await;
                if let Ok(view) = serde_json::from_str::<serde_json::Value>(&mock_mcp_text(&result))
                    && let Some(child) = view["childThreadId"].as_str()
                {
                    mark_delegated(child);
                }
                delegate_events.extend([
                    AgentEvent::ToolCall {
                        id: id.clone(),
                        call: zeron_proto::ToolCall::Mcp {
                            server: "zeron".into(),
                            tool: "delegate_task".into(),
                            input: Some(input),
                        },
                    },
                    AgentEvent::ToolResult {
                        id,
                        is_error: mock_mcp_failed(&result),
                        output: Some(mock_mcp_text(&result)),
                        diff: None,
                        exit_code: None,
                    },
                ]);
            }
        }
        // Dev/testing knob: `ZERON_MOCK_SUBAGENT=1` appends two spawn chips
        // whose nested traffic arrives as tagged `AgentEvent::Subagent`
        // events — the only data-side way to put spawn chips (running → done)
        // AND their openable subagent docs on screen with the mock harness.
        // The second subagent finishes after a beat of nested activity, so a
        // paced run (`ZERON_MOCK_DELAY_MS`) holds a Running chip long enough
        // to observe.
        let mock_subagent = std::env::var("ZERON_MOCK_SUBAGENT")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0");
        let subagent_events = mock_subagent
            .then(|| {
                let tag = |parent: &str, event: AgentEvent| AgentEvent::Subagent {
                    parent_tool_use_id: parent.into(),
                    event: Box::new(event),
                };
                let spawn = |id: &str, description: &str, prompt: &str| AgentEvent::ToolCall {
                    id: id.into(),
                    // The claude-driver spawn shape: `Agent: {description}`
                    // with the task in the input (names the chip AND the tab).
                    call: zeron_proto::ToolCall::Unknown {
                        name: format!("Agent: {description}"),
                        input: Some(serde_json::json!({
                            "description": description,
                            "prompt": prompt,
                        })),
                    },
                };
                let resolve = |id: &str| AgentEvent::ToolResult {
                    id: id.into(),
                    is_error: false,
                    output: None,
                    diff: None,
                    exit_code: None,
                };
                let done = AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: None,
                };
                vec![
                    AgentEvent::TextDelta {
                        text: "\n### Subagent check\n\nFanning out two scouts before the fold rewrite.\n\n".into(),
                    },
                    spawn(
                        "mock-sub-1",
                        "Audit the fold path",
                        "Read crates/doc and list every call site of fold_event_into_parts, checking each holds the byte cap.",
                    ),
                    spawn(
                        "mock-sub-2",
                        "Verify the commit cadence",
                        "Measure the 120ms coalesced commit cadence under a scripted delta burst.",
                    ),
                    // The spawn prompts seed each subagent's opening user
                    // entry (like the claude driver's Task-prompt seeding).
                    tag(
                        "mock-sub-1",
                        AgentEvent::UserMessage {
                            text: "Read crates/doc and list every call site of fold_event_into_parts, checking each holds the byte cap.".into(),
                        },
                    ),
                    tag(
                        "mock-sub-2",
                        AgentEvent::UserMessage {
                            text: "Measure the 120ms coalesced commit cadence under a scripted delta burst.".into(),
                        },
                    ),
                    tag(
                        "mock-sub-1",
                        AgentEvent::ContextUsage {
                            tokens: Some(135_500),
                            window: Some(200_000),
                        },
                    ),
                    tag(
                        "mock-sub-2",
                        AgentEvent::ContextUsage {
                            tokens: Some(88_200),
                            window: Some(200_000),
                        },
                    ),
                    tag(
                        "mock-sub-1",
                        AgentEvent::TextDelta {
                            text: "Scanning `crates/doc` for fold call sites.\n\n".into(),
                        },
                    ),
                    tag(
                        "mock-sub-1",
                        AgentEvent::ToolCall {
                            id: "sub1-grep".into(),
                            call: zeron_proto::ToolCall::Exec {
                                command: "grep -rn fold_event_into_parts crates".into(),
                            },
                        },
                    ),
                    tag("mock-sub-1", resolve("sub1-grep")),
                    tag(
                        "mock-sub-1",
                        AgentEvent::TextDelta {
                            text: "Three call sites: the live fold, the rebuild, and the subagent sink — every one applies the byte cap before persisting.".into(),
                        },
                    ),
                    resolve("mock-sub-1"),
                    tag("mock-sub-1", done.clone()),
                    // A steer AFTER the subagent settled (claude's queued
                    // SendMessage shape): resurrects the chip and reopens
                    // the frozen transcript for the resumed segment.
                    tag(
                        "mock-sub-1",
                        AgentEvent::UserMessage {
                            text: "One more: confirm the rebuild path holds the byte cap too."
                                .into(),
                        },
                    ),
                    tag(
                        "mock-sub-1",
                        AgentEvent::TextDelta {
                            text: "Rebuild path checked — same cap, applied before persisting."
                                .into(),
                        },
                    ),
                    tag("mock-sub-1", done.clone()),
                    tag(
                        "mock-sub-2",
                        AgentEvent::TextDelta {
                            text: "Driving a 2k-delta burst through the writer.\n\n".into(),
                        },
                    ),
                    tag(
                        "mock-sub-2",
                        AgentEvent::ToolCall {
                            id: "sub2-burst".into(),
                            call: zeron_proto::ToolCall::Exec {
                                command: "cargo test -p zeron-doc cadence_burst -- --nocapture".into(),
                            },
                        },
                    ),
                    resolve("mock-sub-2"),
                    tag("mock-sub-2", resolve("sub2-burst")),
                    tag(
                        "mock-sub-2",
                        AgentEvent::TextDelta {
                            text: "Commits land on the 120ms cadence; no commit carried more than one burst.".into(),
                        },
                    ),
                    // A parent→subagent steer: splits the transcript into a
                    // user entry + fresh assistant segment (like the claude
                    // driver's tagged user text blocks).
                    tag(
                        "mock-sub-2",
                        AgentEvent::UserMessage {
                            text: "Also verify the cadence holds while a steer lands mid-burst.".into(),
                        },
                    ),
                    tag(
                        "mock-sub-2",
                        AgentEvent::TextDelta {
                            text: "Re-running with a mid-burst steer injected.\n\n".into(),
                        },
                    ),
                    tag(
                        "mock-sub-2",
                        AgentEvent::ToolCall {
                            id: "sub2-steer-burst".into(),
                            call: zeron_proto::ToolCall::Exec {
                                command: "cargo test -p zeron-doc cadence_steer -- --nocapture"
                                    .into(),
                            },
                        },
                    ),
                    tag("mock-sub-2", resolve("sub2-steer-burst")),
                    tag(
                        "mock-sub-2",
                        AgentEvent::TextDelta {
                            text: "Watching the commit log while the burst drains: ".into(),
                        },
                    ),
                    tag("mock-sub-2", AgentEvent::TextDelta { text: "batch 1 clean, ".into() }),
                    tag("mock-sub-2", AgentEvent::TextDelta { text: "batch 2 clean, ".into() }),
                    tag("mock-sub-2", AgentEvent::TextDelta { text: "batch 3 clean, ".into() }),
                    tag("mock-sub-2", AgentEvent::TextDelta { text: "batch 4 clean, ".into() }),
                    tag("mock-sub-2", AgentEvent::TextDelta { text: "batch 5 clean — ".into() }),
                    tag(
                        "mock-sub-2",
                        AgentEvent::TextDelta {
                            text: "every window under 120ms.\n\nSteer landed between commits; the cadence held.".into(),
                        },
                    ),
                    tag("mock-sub-2", done),
                ]
            })
            .into_iter()
            .flatten();
        // With the code knob, also exercise a MULTILINE Exec command — the
        // round-9 chip breaker shape ("set -e\nfixture_in_original=0"): the
        // Run chip must stay one 30px line.
        let code_tool_events = mock_code
            .then(|| {
                [
                    AgentEvent::ToolCall {
                        id: "mock-code-tool".into(),
                        call: zeron_proto::ToolCall::Exec {
                            command: "set -e\nfixture_in_original=0\ngrep -rn \"veil\" crates/ui/src | wc -l".into(),
                        },
                    },
                    AgentEvent::ToolResult {
                        id: "mock-code-tool".into(),
                        is_error: false,
                        output: None,
                        diff: None,
                        exit_code: None,
                    },
                ]
            })
            .into_iter()
            .flatten();
        // Dev/testing knob: `ZERON_MOCK_READ_FILE=<path>` appends one Read
        // chip on that path (absolute or checkout-relative), the data-side way
        // to click a file badge into the Files pane without a real harness.
        let read_file_events = std::env::var("ZERON_MOCK_READ_FILE")
            .ok()
            .filter(|path| !path.is_empty())
            .map(|path| {
                [
                    AgentEvent::ToolCall {
                        id: "mock-read-file".into(),
                        call: zeron_proto::ToolCall::ReadFile { path },
                    },
                    AgentEvent::ToolResult {
                        id: "mock-read-file".into(),
                        is_error: false,
                        output: None,
                        diff: None,
                        exit_code: None,
                    },
                ]
            })
            .into_iter()
            .flatten();
        // Dev/testing knob: `ZERON_MOCK_WRITE=1` appends a Write (create) and
        // an Edit whose results carry a diff — the `+N −M` chip header and
        // the stats body a Claude Write/Edit folds to.
        let mock_write = std::env::var("ZERON_MOCK_WRITE")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0");
        let write_events = mock_write
            .then(|| {
                [
                    AgentEvent::ToolCall {
                        id: "mock-write".into(),
                        call: zeron_proto::ToolCall::WriteFile {
                            path: "packages/schema/test/verdict.test.ts".into(),
                            content: None,
                        },
                    },
                    AgentEvent::ToolResult {
                        id: "mock-write".into(),
                        is_error: false,
                        output: None,
                        diff: Some(zeron_proto::ToolDiff {
                            path: "packages/schema/test/verdict.test.ts".into(),
                            old_text: None,
                            new_text: "import { verdict } from \"../src/verdict\";\n\ntest(\"a verdict names its finding\", () => {\n  expect(verdict(\"f1\").finding).toBe(\"f1\");\n});\n".into(),
                            line_offset: None,
                        }),
                        exit_code: None,
                    },
                    AgentEvent::ToolCall {
                        id: "mock-edit".into(),
                        call: zeron_proto::ToolCall::EditFile {
                            path: "packages/schema/src/verdict.ts".into(),
                            old_string: None,
                            new_string: None,
                        },
                    },
                    AgentEvent::ToolResult {
                        id: "mock-edit".into(),
                        is_error: false,
                        output: None,
                        diff: Some(zeron_proto::ToolDiff {
                            path: "packages/schema/src/verdict.ts".into(),
                            old_text: Some("export function verdict(id: string) {\n  return { id };\n}\n".into()),
                            new_text: "export function verdict(finding: string) {\n  return { finding, severity: \"info\" };\n}\n".into(),
                            line_offset: None,
                        }),
                        exit_code: None,
                    },
                ]
            })
            .into_iter()
            .flatten();
        let compact_events = std::env::var("ZERON_MOCK_COMPACT")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0")
            .then(|| {
                [
                    AgentEvent::Compacting { active: true },
                    AgentEvent::Compacting { active: false },
                    AgentEvent::Compacted {
                        pre_tokens: Some(226_874),
                        post_tokens: Some(12_451),
                    },
                ]
            })
            .into_iter()
            .flatten();
        // Dev/testing knob: `ZERON_MOCK_TODO=1` appends a checklist the agent
        // works through — one `Todo` call rewritten in place per step, the way
        // TodoWrite does — so the todo panel's open / fold / finished states
        // can be watched live (pace it with `ZERON_MOCK_DELAY_MS`).
        let mock_todo = std::env::var("ZERON_MOCK_TODO")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0");
        let todo_events = mock_todo
            .then(|| {
                const STEPS: [&str; 8] = [
                    "Read the fold path in crates/doc",
                    "Add the status field to TodoItem",
                    "Map in-progress in every normalizer",
                    "Write the fold-window view model",
                    "Render the todo tray above the composer",
                    "Persist open state per chat",
                    "Run the unit tests",
                    "Write the design note",
                ];
                let list = |active: usize| zeron_proto::ToolCall::Todo {
                    items: STEPS
                        .iter()
                        .enumerate()
                        .map(|(ix, text)| {
                            zeron_proto::TodoItem::new(
                                *text,
                                match ix.cmp(&active) {
                                    std::cmp::Ordering::Less => zeron_proto::TodoStatus::Completed,
                                    std::cmp::Ordering::Equal => {
                                        zeron_proto::TodoStatus::InProgress
                                    }
                                    std::cmp::Ordering::Greater => zeron_proto::TodoStatus::Pending,
                                },
                            )
                        })
                        .collect(),
                };
                let mut events = vec![AgentEvent::TextDelta {
                    text: "\n### Todo check\n\nWorking through the checklist.\n\n".into(),
                }];
                // Steps 0..=8: the last write has every item completed.
                for active in 0..=STEPS.len() {
                    events.push(AgentEvent::ToolCall {
                        id: "mock-todo".into(),
                        call: list(active),
                    });
                    events.push(AgentEvent::ToolResult {
                        id: "mock-todo".into(),
                        is_error: false,
                        output: None,
                        diff: None,
                        exit_code: None,
                    });
                    events.push(AgentEvent::TextDelta {
                        text: format!("Update {} written.\n\n", active + 1),
                    });
                }
                events
            })
            .into_iter()
            .flatten();
        let events: Vec<Result<AgentEvent, HarnessError>> = body
            .iter()
            .cycle()
            .take(body.len() * repeat)
            .cloned()
            .chain(thinking_events)
            .chain(code_tool_events)
            .chain(write_events)
            .chain(subagent_events)
            .chain(read_file_events)
            .chain(html_render_events)
            .chain(delegate_events)
            .chain(reply_event)
            .chain(todo_events)
            .chain(code_event)
            .chain(table_event)
            .chain(mend_event)
            .chain(error_event)
            .chain(limit_events)
            .chain(compact_events)
            .chain(usage_events)
            .chain(tail.iter().cloned())
            .map(Ok)
            .collect();
        // Dev/testing knob: `ZERON_MOCK_CHARS=N` re-chunks every TextDelta
        // into N-char deltas, so `ZERON_MOCK_DELAY_MS` paces *characters*
        // instead of whole scripted blocks — delta boundaries then land inside
        // inline markers and links, which is the streaming shape real
        // harnesses produce and the display mend exists for.
        let events: Vec<Result<AgentEvent, HarnessError>> = match mock_chunk_chars() {
            None => events,
            Some(n) => events
                .into_iter()
                .flat_map(|event| match event {
                    Ok(event) => rechunk(event, n).into_iter().map(Ok).collect(),
                    other => vec![other],
                })
                .collect(),
        };
        // Dev/testing knob: `ZERON_MOCK_SUBAGENT_DELAY_MS` paces TAGGED
        // (subagent) events on their own clock — the parent turn settles at
        // `ZERON_MOCK_DELAY_MS` speed while the background subagents stream
        // on slowly, which is exactly the eager-done shape live tabs are
        // observed under (and the only way a rig click can reliably land
        // inside a subagent's streaming window).
        let sub_delay_ms = std::env::var("ZERON_MOCK_SUBAGENT_DELAY_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok());
        let suggestion_ms = knob_number("ZERON_MOCK_SUGGESTION_MS", 0);
        let suggestion = std::env::var("ZERON_MOCK_SUGGESTION")
            .ok()
            .filter(|text| !text.is_empty() && controls.turn.prompt_suggestions())
            .map(|text| AgentEvent::PromptSuggestion { text });
        let suggestion = futures::stream::iter(suggestion).then(move |event| async move {
            tokio::time::sleep(std::time::Duration::from_millis(suggestion_ms)).await;
            Ok(event)
        });
        if delay.is_zero() && sub_delay_ms.is_none() {
            return Ok(futures::stream::iter(events).chain(suggestion).boxed());
        }
        Ok(futures::stream::iter(events)
            .then(move |event| async move {
                let pause = match (&event, sub_delay_ms) {
                    (Ok(AgentEvent::Subagent { .. }), Some(ms)) => {
                        std::time::Duration::from_millis(ms)
                    }
                    _ => delay,
                };
                tokio::time::sleep(pause).await;
                event
            })
            .chain(suggestion)
            .boxed())
    }
}

fn limit_matches(prompt: &str, needle: Option<&str>) -> bool {
    needle
        .filter(|needle| !needle.is_empty())
        .is_none_or(|needle| prompt.contains(needle))
}

static MOCK_DELEGATED: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

// Once per chat and never in its own children, or wake turns would delegate forever.
fn delegate_calls(json: &str, prompt: &str, chat: Option<&str>) -> Vec<serde_json::Value> {
    let calls = match serde_json::from_str(json) {
        Ok(serde_json::Value::Array(calls)) => calls,
        Ok(call @ serde_json::Value::Object(_)) => vec![call],
        _ => return Vec::new(),
    };
    let own_child = calls.iter().any(|call| {
        call["task"]
            .as_str()
            .map(str::trim)
            .is_some_and(|task| !task.is_empty() && prompt.contains(task))
    });
    if calls.is_empty() || own_child || !chat.is_some_and(mark_delegated) {
        return Vec::new();
    }
    calls
}

fn mark_delegated(chat: &str) -> bool {
    MOCK_DELEGATED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(chat.to_owned())
}

fn mock_delay() -> std::time::Duration {
    std::time::Duration::from_millis(
        std::env::var("ZERON_MOCK_DELAY_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0),
    )
}

fn mock_chunk_chars() -> Option<usize> {
    std::env::var("ZERON_MOCK_CHARS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
}

fn rechunk(event: AgentEvent, n: usize) -> Vec<AgentEvent> {
    let chunks = |text: &str| -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        chars.chunks(n).map(|c| c.iter().collect()).collect()
    };
    match event {
        AgentEvent::TextDelta { text } => chunks(&text)
            .into_iter()
            .map(|text| AgentEvent::TextDelta { text })
            .collect(),
        // Reasoning streams the same char-paced way — thought
        // chips have their own streaming display (mend + live
        // tail), which whole-block deltas would never exercise.
        AgentEvent::ReasoningDelta { text } => chunks(&text)
            .into_iter()
            .map(|text| AgentEvent::ReasoningDelta { text })
            .collect(),
        other => vec![other],
    }
}

/// A page of three groups of two options each, as the U23 mockup has them.
pub const MOCK_PICK_PAGE: &str = r#"<!doctype html><html><head><title>Khung app</title><style>.g{display:grid;grid-template-columns:72px 1fr 1fr;gap:6px;align-items:center;margin:6px 0}button{height:34px;border:0;border-radius:8px;background:var(--card);color:var(--foreground);font:inherit;text-align:left;padding:0 8px;box-shadow:inset 0 0 0 .5px var(--border)}button[data-zeron-chosen]{background:var(--accent-surface)}.h{display:flex;justify-content:space-between;font-weight:600}.c{color:var(--muted-foreground);font-weight:400}</style></head><body><div class="h">Khung app <span class="c" data-zeron-count></span></div><div class="g" data-zeron-group-label="Góc trang"><span>Góc trang</span><button data-zeron-pick="A1" data-zeron-group="A" data-zeron-desc="Góc bo 12 như thẻ" data-zeron-label="Bo góc thường">A1 Bo góc thường</button><button data-zeron-pick="A2" data-zeron-group="A" data-zeron-desc="Tab dính sát mép trên cửa sổ" data-zeron-label="Nổi sát đỉnh">A2 Nổi sát đỉnh</button></div><div class="g" data-zeron-group-label="Thanh tab"><span>Thanh tab</span><button data-zeron-pick="B1" data-zeron-group="B" data-zeron-desc="Viền mảnh quanh từng tab" data-zeron-label="Có viền">B1 Có viền</button><button data-zeron-pick="B2" data-zeron-group="B" data-zeron-desc="Chỉ đổi nền khi chọn" data-zeron-label="Không viền">B2 Không viền</button></div><div class="g" data-zeron-group-label="Rail"><span>Rail</span><button data-zeron-pick="C1" data-zeron-group="C" data-zeron-desc="Ảnh tràn tới mép rail" data-zeron-label="Ảnh sát mép">C1 Ảnh sát mép</button><button data-zeron-pick="C2" data-zeron-group="C" data-zeron-desc="Rail cùng màu nền app" data-zeron-label="Cùng tông">C2 Cùng tông</button></div></body></html>"#;

/// The `html_render` and `html_preview` inputs `ZERON_MOCK_HTML_RENDER` asks for.
fn mock_page_input(page: &str, height: u32) -> (serde_json::Value, serde_json::Value) {
    if let Some(path) = page.strip_prefix("path:") {
        return (
            serde_json::json!({ "path": path, "title": "Mock page", "height": height }),
            serde_json::json!({ "path": path }),
        );
    }
    let (html, title) = if page == "pick" {
        (MOCK_PICK_PAGE.to_owned(), "Khung app")
    } else {
        (
            std::fs::read_to_string(page).unwrap_or_else(|_| "<p>mock page</p>".into()),
            "Mock page",
        )
    };
    (
        serde_json::json!({ "html": html, "title": title, "height": height }),
        serde_json::json!({ "html": html }),
    )
}

/// One `tools/call` over a fresh `zeron mcp` stdio server (dev knobs only).
async fn mock_mcp_call(
    server: &zeron_proto::McpServer,
    tool: &str,
    arguments: serde_json::Value,
) -> Result<serde_json::Value, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let mut child = tokio::process::Command::new(&server.command)
        .args(&server.args)
        .envs(&server.env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let mut lines = BufReader::new(child.stdout.take().ok_or("no stdout")?).lines();
    for message in [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"mock","version":"0"}}}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":tool,"arguments":arguments}}),
    ] {
        stdin
            .write_all(format!("{message}\n").as_bytes())
            .await
            .map_err(|e| e.to_string())?;
    }
    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let reply: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
        if reply["id"] == 2 {
            return Ok(reply["result"].clone());
        }
    }
    Err("zeron mcp exited without answering".into())
}

fn mock_mcp_failed(result: &Result<serde_json::Value, String>) -> bool {
    result
        .as_ref()
        .map_or(true, |result| result["isError"] == true)
}

fn mock_mcp_text(result: &Result<serde_json::Value, String>) -> String {
    match result {
        Ok(result) => result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        Err(error) => error.clone(),
    }
}

/// The background mode's processes: id, command, description, how long ago
/// it started, and the lines it prints in turn.
const BACKGROUND_PROCESSES: [(&str, &str, &str, i64, &[&str]); 3] = [
    (
        "mock-bg-dev",
        "pnpm --filter web dev --port 5173",
        "Start the web dev server",
        754,
        &[
            "  VITE v6.2.1  ready in 412 ms",
            "  \u{279c}  Local:   http://localhost:5173/",
            "  \u{279c}  Network: use --host to expose",
            "12:41:07 [vite] hmr update /src/routes/settings.tsx",
            "12:41:19 [vite] hmr update /src/components/Composer.tsx, /src/styles/composer.css",
            "12:41:32 [vite] page reload src/main.tsx",
        ],
    ),
    (
        "mock-bg-watch",
        "cargo watch -x 'test -p zeron-doc'",
        "Re-run the doc tests on change",
        187,
        &[
            "[Running 'cargo test -p zeron-doc']",
            "   Compiling zeron-doc v0.2.129 (/work/crates/doc)",
            "    Finished `test` profile [unoptimized + debuginfo] target(s) in 6.81s",
            "     Running unittests src/lib.rs (target/debug/deps/zeron_doc-3f1c)",
            "test result: ok. 412 passed; 0 failed; 0 ignored; finished in 1.92s",
            "[Finished running. Exit status: 0]",
        ],
    ),
    (
        "mock-bg-logs",
        "tail -f logs/api.log | grep --line-buffered -E 'WARN|ERROR'",
        "Watch the API log for errors",
        41,
        &[
            "WARN  sync::relay  reconnecting to device room (attempt 2)",
            "WARN  sync::relay  device room rejoined after 1.4s",
            "ERROR api::uploads  chunk 7 of 12 timed out; retrying",
        ],
    ),
];

/// `ZERON_MOCK_BACKGROUND`: one turn that starts [`BACKGROUND_PROCESSES`],
/// then a parked runtime that keeps them running (each prints its script
/// once, a line per tick).
fn background_run(
    controls: RunControls,
    delay: std::time::Duration,
) -> BoxStream<'static, Result<AgentEvent, HarnessError>> {
    let RunControls {
        mut steering,
        interrupt,
        turn,
        ..
    } = controls;
    let pause = delay.max(std::time::Duration::from_millis(120));
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    tokio::spawn(async move {
        let done = || AgentEvent::Done {
            status: DoneStatus::Completed,
            result: None,
            error: None,
            session_id: None,
        };
        let _ = tx.send(AgentEvent::TextDelta {
            text:
                "I'll keep the dev server, the doc tests and the API log running while we work.\n\n"
                    .into(),
        });
        let now = chrono::Utc::now();
        let mut listed = Vec::new();
        for (id, command, description, ago, lines) in BACKGROUND_PROCESSES {
            tokio::time::sleep(pause).await;
            let _ = tx.send(AgentEvent::ToolCall {
                id: format!("{id}-call"),
                call: zeron_proto::ToolCall::Exec {
                    command: command.into(),
                },
            });
            let _ = tx.send(AgentEvent::ToolResult {
                id: format!("{id}-call"),
                is_error: false,
                output: Some(format!("Command running in background with ID: {id}")),
                diff: None,
                exit_code: None,
            });
            listed.push(zeron_proto::BackgroundTask {
                id: id.into(),
                command: command.into(),
                description: Some(description.into()),
                started_at: now - chrono::Duration::seconds(ago),
                ..Default::default()
            });
            for line in &lines[..lines.len() / 2] {
                turn.append_background_output(id, &format!("{line}\n"));
            }
            turn.set_background_processes(listed.clone());
            turn.set_background(listed.len());
        }
        tokio::time::sleep(pause).await;
        let _ = tx.send(AgentEvent::TextDelta {
            text: "All three are up. Ask me to stop any of them, or stop them from the tray."
                .into(),
        });
        let _ = tx.send(done());

        // Parked: the processes print, stop when asked, and the runtime
        // answers whatever comes next.
        let mut printed: Vec<usize> = BACKGROUND_PROCESSES
            .iter()
            .map(|(.., lines)| lines.len() / 2)
            .collect();
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(1600));
        loop {
            tokio::select! {
                _ = interrupt.cancelled() => break,
                id = turn.background_stop_requested() => {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    listed.retain(|task| task.id != id);
                    turn.set_background_processes(listed.clone());
                    turn.set_background(listed.len());
                }
                _ = tick.tick() => {
                    for (ix, (id, .., lines)) in BACKGROUND_PROCESSES.iter().enumerate() {
                        if listed.iter().any(|task| task.id == *id) && printed[ix] < lines.len() {
                            let line = lines[printed[ix]];
                            printed[ix] += 1;
                            turn.append_background_output(id, &format!("{line}\n"));
                        }
                    }
                }
                message = steering.recv() => {
                    let Some(_) = message else { break };
                    let _ = tx.send(AgentEvent::Steered {
                        assistant_message_id: None,
                        next_assistant_message_id: None,
                    });
                    tokio::time::sleep(pause).await;
                    let running = listed.len();
                    let _ = tx.send(AgentEvent::TextDelta {
                        text: format!(
                            "Noted. {running} background process{} still running.",
                            if running == 1 { " is" } else { "es are" }
                        ),
                    });
                    let _ = tx.send(done());
                }
            }
        }
        turn.set_background_processes(Vec::new());
        turn.set_background(0);
        if interrupt.is_cancelled() {
            let _ = tx.send(AgentEvent::Done {
                status: DoneStatus::Interrupted,
                result: None,
                error: None,
                session_id: None,
            });
        }
    });
    futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|event| (Ok(event), rx))
    })
    .boxed()
}

/// What the long-tool runtime answers a prompt: its words, and the files it
/// came with as attachments (the images among them by name), so a test can
/// see what reached the agent as an attachment and not only as text.
fn heard(words: &str, attached: &[String]) -> String {
    if attached.is_empty() {
        return format!("Heard: {words}\n\n");
    }
    let names: Vec<&str> = attached
        .iter()
        .map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p))
        .collect();
    let images = attached
        .iter()
        .filter(|p| {
            let p = p.to_ascii_lowercase();
            [".png", ".jpg", ".jpeg", ".gif", ".webp"]
                .iter()
                .any(|e| p.ends_with(e))
        })
        .count();
    format!(
        "Heard: {words}\n\n[attachments: {} ({images} image{}): {}]\n\n",
        attached.len(),
        if images == 1 { "" } else { "s" },
        names.join(", ")
    )
}

/// `ZERON_MOCK_LONG_TOOL=1` (Unix: it runs `sleep`).
fn long_tool_mode() -> bool {
    cfg!(unix)
        && std::env::var("ZERON_MOCK_LONG_TOOL")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0")
}

/// `ZERON_MOCK_LONG_TOOL`: a runtime with real processes. Its first turn
/// starts a background `sleep 600` that outlives every turn, then runs a
/// foreground `sleep` (`ZERON_MOCK_TOOL_SECS`, 120 by default) as a
/// blocking command; so does any later prompt naming "long". A steer that
/// arrives while the command runs waits for it to finish (a step-boundary
/// agent's), then is answered `Heard: <prompt>`. A host stop
/// ([`crate::TurnControl::stop_turn`]) kills the foreground command alone
/// and ends the turn in place; a teardown takes everything. With
/// `ZERON_MOCK_PID_DIR`, the processes' ids go to `<first prompt>.bg.pid`
/// and `<first prompt>.fg.pid` there.
fn long_tool_run(
    first: String,
    first_attachments: Vec<String>,
    controls: RunControls,
) -> BoxStream<'static, Result<AgentEvent, HarnessError>> {
    let RunControls {
        mut steering,
        interrupt,
        turn,
        ..
    } = controls;
    let secs: u64 = std::env::var("ZERON_MOCK_TOOL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    let pid_dir = std::env::var_os("ZERON_MOCK_PID_DIR").map(std::path::PathBuf::from);
    // (Named for the runtime's first prompt, so runtimes don't share files.)
    let slug: String = first
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(40)
        .collect();
    let note_pid = move |name: &str, pid: Option<u32>| {
        if let (Some(dir), Some(pid)) = (&pid_dir, pid) {
            let _ = std::fs::write(dir.join(format!("{slug}.{name}")), pid.to_string());
        }
    };
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    tokio::spawn(async move {
        let done = |status: DoneStatus| AgentEvent::Done {
            status,
            result: None,
            error: None,
            session_id: None,
        };
        let say = |text: String| AgentEvent::TextDelta { text };
        let mut background: Option<tokio::process::Child> = None;
        let mut calls = 0usize;
        let mut prompt = Some((first, first_attachments));
        let mut first_turn = true;
        'turns: loop {
            // Parked between turns: the next prompt comes by the mailbox.
            let (words, attached) = match prompt.take() {
                Some(next) => next,
                None => {
                    tokio::select! {
                        _ = interrupt.cancelled() => break 'turns,
                        message = steering.recv() => {
                            let Some(message) = message else { break 'turns };
                            let _ = tx.send(AgentEvent::Steered {
                                assistant_message_id: None,
                                next_assistant_message_id: None,
                            });
                            (message.prompt, message.attachments)
                        }
                    }
                }
            };
            let long = first_turn || words.contains("long");
            if !first_turn {
                let _ = tx.send(say(heard(&words, &attached)));
            }
            if first_turn {
                first_turn = false;
                calls += 1;
                let id = format!("mock-long-bg-{calls}");
                let _ = tx.send(AgentEvent::ToolCall {
                    id: id.clone(),
                    call: zeron_proto::ToolCall::Exec {
                        command: "sleep 600 &".into(),
                    },
                });
                background = tokio::process::Command::new("sleep")
                    .arg("600")
                    .spawn()
                    .ok();
                let pid = background.as_ref().and_then(|c| c.id());
                note_pid("bg.pid", pid);
                let _ = tx.send(AgentEvent::ToolResult {
                    id,
                    is_error: false,
                    output: Some(format!(
                        "Command running in background, pid {}",
                        pid.unwrap_or(0)
                    )),
                    diff: None,
                    exit_code: None,
                });
                turn.set_background_processes(vec![zeron_proto::BackgroundTask {
                    id: "mock-long-bg".into(),
                    command: "sleep 600".into(),
                    description: Some("A background process that outlives the turn".into()),
                    started_at: chrono::Utc::now(),
                    ..Default::default()
                }]);
                turn.set_background(1);
            }
            if !long {
                let _ = tx.send(done(DoneStatus::Completed));
                continue;
            }
            calls += 1;
            let id = format!("mock-long-fg-{calls}");
            let _ = tx.send(AgentEvent::ToolCall {
                id: id.clone(),
                call: zeron_proto::ToolCall::Exec {
                    command: format!("sleep {secs}"),
                },
            });
            let Ok(mut child) = tokio::process::Command::new("sleep")
                .arg(secs.to_string())
                .spawn()
            else {
                let _ = tx.send(done(DoneStatus::Errored));
                continue;
            };
            note_pid("fg.pid", child.id());
            // Steers that come while the command runs wait for it.
            let mut held: Vec<(String, Vec<String>)> = Vec::new();
            let stopped = loop {
                tokio::select! {
                    _ = interrupt.cancelled() => {
                        let _ = child.kill().await;
                        break 'turns;
                    }
                    _ = turn.stop_requested() => {
                        let _ = child.kill().await;
                        break true;
                    }
                    _ = child.wait() => break false,
                    message = steering.recv() => match message {
                        Some(message) => held.push((message.prompt, message.attachments)),
                        None => {
                            let _ = child.kill().await;
                            break 'turns;
                        }
                    },
                }
            };
            let _ = tx.send(AgentEvent::ToolResult {
                id,
                is_error: stopped,
                output: Some(if stopped {
                    "Interrupted".into()
                } else {
                    String::new()
                }),
                diff: None,
                exit_code: (!stopped).then_some(0),
            });
            if stopped {
                // Steers the stopped turn held are cancelled with it, as
                // Claude's `cancel_queued` does.
                let _ = tx.send(done(DoneStatus::Interrupted));
                continue;
            }
            for (words, attached) in held {
                let _ = tx.send(AgentEvent::Steered {
                    assistant_message_id: None,
                    next_assistant_message_id: None,
                });
                let _ = tx.send(say(heard(&words, &attached)));
            }
            let _ = tx.send(say("The command finished.".into()));
            let _ = tx.send(done(DoneStatus::Completed));
        }
        if let Some(mut child) = background.take() {
            let _ = child.kill().await;
        }
        turn.set_background_processes(Vec::new());
        turn.set_background(0);
        if interrupt.is_cancelled() {
            let _ = tx.send(done(DoneStatus::Interrupted));
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

    #[test]
    fn the_mock_page_knob_sends_html_a_pick_page_or_a_path() {
        let (pick, _) = super::mock_page_input("pick", 600);
        assert_eq!(pick["title"], "Khung app");
        assert!(
            pick["html"]
                .as_str()
                .unwrap()
                .contains("data-zeron-pick=\"A2\"")
        );
        let (path, preview) = super::mock_page_input("path:out/tray.html", 420);
        assert_eq!(path["path"], "out/tray.html");
        assert!(path.get("html").is_none());
        assert_eq!(preview["path"], "out/tray.html");
        let (missing, _) = super::mock_page_input("/nope/page.html", 600);
        assert_eq!(missing["html"], "<p>mock page</p>");
    }

    #[test]
    fn error_prompt_names_the_failure() {
        assert_eq!(
            scripted_error("/error Not logged in"),
            Some("Not logged in")
        );
        assert_eq!(scripted_error("/error  "), None);
        assert_eq!(scripted_error("/errors"), None);
    }

    #[test]
    fn mock_limit_match_narrows_the_limit_to_matching_prompts() {
        assert!(limit_matches("anything", None));
        assert!(limit_matches("anything", Some("")));
        assert!(limit_matches(
            "Act as the test sub-agent for this task.\n\nprobe",
            Some("probe")
        ));
        assert!(!limit_matches("delegate the probe", Some("child-only")));
    }

    #[test]
    fn mock_delegate_fires_once_per_chat_and_never_in_its_children() {
        let one = r#"{"task":"say hi","role":"worker"}"#;
        assert_eq!(
            delegate_calls(one, "start", Some("chat-a")),
            vec![serde_json::json!({"task": "say hi", "role": "worker"})]
        );
        assert!(delegate_calls(one, "Delegated task t1 finished.", Some("chat-a")).is_empty());
        assert!(delegate_calls(one, "say hi", Some("chat-b")).is_empty());
        assert!(
            delegate_calls(
                one,
                "Act as the worker sub-agent for this task.\n\nsay hi",
                Some("chat-c")
            )
            .is_empty()
        );
        assert!(delegate_calls(one, "start", None).is_empty());

        let many =
            r#"[{"task":"probe the cache"},{"task":"draft notes","target":{"model":"haiku"}}]"#;
        assert_eq!(
            delegate_calls(many, "start", Some("chat-d")),
            vec![
                serde_json::json!({"task": "probe the cache"}),
                serde_json::json!({"task": "draft notes", "target": {"model": "haiku"}}),
            ]
        );
        assert!(delegate_calls("not json", "start", Some("chat-e")).is_empty());
        assert!(delegate_calls("[]", "start", Some("chat-f")).is_empty());
    }
}
