# Recon: /compact and /sidechat surfaces in zeron-fork

## 1. Run / session model

**Composer → harness path**

- Composer `send()` starts at `/Users/reinamaccredy/Code/zeron-fork/crates/ui/src/composer.rs:5952`. It mints `chat_id` (existing selection, or a client-minted UUID for the new-chat canvas), resolves model/reasoning/options from the pickers, and mints `message_id` (`composer.rs:6051`).
- Two send paths:
  - Busy chat → `methods::QUEUE_MESSAGE` with `"holdForTurnEnd": true` (`composer.rs:6490-6506`), returns a queue id.
  - Normal → builds `SessionCommandPayload::Run { request: RunRequest { … }, message_id }` and calls `methods::QUEUE_COMMAND` with `{chatId, command}` (`composer.rs:6509-6541`). Note `resume: None` is always sent from the UI (`composer.rs:6520`).
- `RunRequest` is defined at `/Users/reinamaccredy/Code/zeron-fork/crates/proto/src/agent.rs:93-126`: `prompt`, `harness: Option<HarnessId>`, `model`, `reasoning`, `model_options`, `cwd`, `sandbox`, `auto_approve`, `resume: Option<String>` ("Harness-native session id to resume, if any", `agent.rs:110-111`), `attachments`, `worktree`.
- The command ledger type is `SessionCommandPayload` at `/Users/reinamaccredy/Code/zeron-fork/crates/doc/src/commands.rs:39-57` — only four kinds: `Run`, `Steer`, `Interrupt`, `RespondInput` (`commands.rs:19-24`). There is **no** command kind that is not a transcript-appending turn.
- Host-side drain: `DocHost::drain_commands` (`/Users/reinamaccredy/Code/zeron-fork/crates/engine/src/doc_host.rs:3821`), evaluated via `evaluate_command` (`doc_host.rs:3550`, `doc_host.rs:3876`), `Run` executes at `doc_host.rs:4075` → `dispatch_with_source_context` (`doc_host.rs:4379`) → `sessions.dispatch(chat_id, harness, request, message_id)` (`doc_host.rs:4394`).

**The engine entry point**

- `SessionsEngine::dispatch` → `dispatch_inner` at `/Users/reinamaccredy/Code/zeron-fork/crates/engine/src/sessions.rs:326` / `:353`.
- Identity is the **chat id** (a string). One live `RunHandle` per chat id: `lock(&self.inner.runs).get(chat_id)` (`sessions.rs:366`), `runs.insert(chat_id.to_string(), RunHandle{…})` (`sessions.rs:487-500`).
- Routing rule: if a live run exists and is steerable with the same `RuntimeConfig`, the prompt goes into the steering mailbox as a warm dispatch (`sessions.rs:375-396`). Otherwise the live run is **interrupted and replaced** (`sessions.rs:427-437`: `self.interrupt(chat_id).await?`).
- The user entry is written to the doc immediately: `handle.write_user_message(&user_id, &request.prompt, now_ms())` (`sessions.rs:399` and `sessions.rs:442`).

**Harness session id storage and reuse (resume)**

- Engine-owned resume injection, callers always send `resume: None`: `sessions.rs:444-455` — `request.resume = self.inner.resume_for(chat_id, &request.cwd)`.
- `resume_for` (`sessions.rs:1065-1089`): live in-process cache → workspace chat row → journal scan. **Cwd-gated throughout** (`sessions.rs:1074`, `1082`, `1088`); an empty stored id is an explicit "do not resume" tombstone.
- `remember_harness_session` (`sessions.rs:1039-1056`) writes both the in-memory `HarnessSessionRef` and the durable row via `ws.set_chat_harness_session(chat_id, session_id, cwd)`.
- Written on `AgentEvent::SessionStarted` (`sessions.rs:2166-2173`) and on `AgentEvent::Done { session_id: Some(..) }` (`sessions.rs:2174-2179`), plus crash recovery (`sessions.rs:683-686`).
- Deliberate absence of a forget path: `sessions.rs:1058-1063` ("there is deliberately no `forget_harness_session` anymore").
- Journal fallback scan: `journal_harness_session` (`sessions.rs:1091-1124`).
- Durable row fields on the chat entity: `Chat.harness_session_id` and `Chat.harness_session_cwd` at `/Users/reinamaccredy/Code/zeron-fork/crates/proto/src/entities.rs:134-145`.

**Per-harness resume mechanics**

| Harness | Resume mechanism | Citation |
|---|---|---|
| Claude Code | `--resume=<session_id>` CLI flag | `crates/harness/src/claude/mod.rs:199-200` |
| Codex | `thread/resume { threadId }` JSON-RPC, falling back to `thread/start` on failure | `crates/harness/src/codex/mod.rs:886-906` |
| Opencode | Sessions are durable server-side; resume = reuse the same session id after a `session_info` probe (`GET /session/{id}` or `/api/session/{id}`) | `crates/harness/src/opencode/mod.rs:1200-1212`, lookup at `:684-694` |
| ACP (Devin/Grok/Hermes/Pi) | `session/load { sessionId }`, fresh `session/new` fallback | `crates/harness/src/acp/mod.rs:1940-1969` |
| Cursor | `"resume": request.resume` passed into the pinned `@cursor/sdk` shim frame | `crates/harness/src/cursor/mod.rs:296` |

**Forking / branching**: none anywhere. No harness call site forks a session, and no `RunRequest` field expresses it. The only "isolated" concept is `WorktreeSpec` (`crates/proto/src/agent.rs:133-140`), which is a git worktree, not a conversation branch. Subagent docs (`subagent_doc_id`, `sessions.rs:1152-1168`) are separate *transcripts*, not forks of the harness session.

**Secondary run that uses session context**: **none exists.** `run_title` is the only secondary run, and it is explicitly *context-free*:

- `TitleGenerator::run_title_model` (`crates/engine/src/titles.rs:148`) runs in a **fresh tempdir** (`titles.rs:170`: "No repository instructions, files, or active coding-session context"; `let scratch = tempfile::tempdir()`), with `resume: None` (`titles.rs:203`), `SandboxLevel::ReadOnly`, `ReasoningLevel::Minimal` (`titles.rs:193-205`).
- Driver side strips everything: `ClaudeHarness::run_title` sets `request.resume = None`, clears worktree/attachments/model_options (`crates/harness/src/claude/mod.rs:401-410`) and adds `--system-prompt <TITLE_INSTRUCTIONS> --tools "" --strict-mcp-config --mcp-config {} --setting-sources ""` (`claude/mod.rs:415-424`). `CodexHarness::run_title` is the same shape (`crates/harness/src/codex/mod.rs:585-595`).
- The engine drains only `TextDelta` and **errors on any tool call** (`titles.rs:283-288`).
- `supports_titles` is only Codex, ClaudeCode, Mock (`crates/harness/src/lib.rs:306-311`). `TITLE_INSTRUCTIONS` at `lib.rs:303`.
- Title events never touch the doc or journal — `collect_text` consumes the stream itself (`titles.rs:261-312`).

So: the codebase has a "secondary run" *primitive* (`run_title` + `Harness::run_title`), but it is deliberately isolated from session context and only implemented by 2 of 5 harnesses.

## 2. Harness trait and per-harness capabilities

**Every method of `trait Harness`** (`/Users/reinamaccredy/Code/zeron-fork/crates/harness/src/lib.rs:62-112`):

| Method | Line | Default? |
|---|---|---|
| `fn id(&self) -> HarnessId` | 63 | required |
| `fn display_name(&self) -> &str` | 64 | required |
| `fn supports_steering(&self) -> bool` | 65 | required |
| `fn steering_mode(&self) -> SteeringMode` | 66 | required |
| `fn reasoning_levels(&self) -> &[ReasoningLevel]` | 67 | required |
| `fn installed(&self) -> bool` | 71 | default `true` |
| `fn deterministic_turn_end(&self) -> bool` | 79 | default `false` |
| `fn authoritative_prompt_end(&self) -> bool` | 85 | default = `deterministic_turn_end()` |
| `async fn models(&self) -> Result<Vec<Model>, …>` | 88 | required |
| `async fn commands(&self) -> Result<Vec<SlashCommand>, …>` | 91 | default `Ok(vec![])` |
| `async fn run_title(&self, RunRequest, RunControls) -> Result<BoxStream<…>, …>` | 96 | default: `Err(Protocol("title generation is not supported by this harness"))` |
| `async fn run(&self, RunRequest, RunControls) -> Result<BoxStream<…>, …>` | 107 | required |

Supporting types: `RunControls { request_input, steering, interrupt }` (`lib.rs:48-59`), `SteerMessage { prompt, message_id }` (`lib.rs:42-45`), `HarnessError` (`lib.rs:28-39`).

`commands()` is implemented by: claude (`claude/mod.rs:386`, via the CLI `initialize` control-request handshake), codex (`codex/mod.rs:570`, a `skills/list` probe), acp (`acp/mod.rs:1169`), opencode (`opencode/mod.rs:363`). **Cursor does not implement `commands()`** — it inherits the empty default.

**Existing compaction / summarize handling: NONE.**

A repo-wide search for `compact|summariz` across `crates`, `edge`, `apps` returns only:
- composer layout state ("compact↔expanded pill flip") in `crates/ui/src/composer.rs` (e.g. `:71`, `:118`, `:4107`) — unrelated;
- the string `/compact` used only as a *test fixture* for the slash tokenizer: `composer.rs:3911` (doc comment: "slash commands are whole-prompt prefixes (`/compact`, `/goal ship it`)"), `composer.rs:8511`, `composer.rs:8518`.
- `crates/engine/src/run_journal.rs:10`: "Bounded-window compaction is **deferred** (whole file kept for now, per M2 scope)" — journal file compaction, not context compaction.

No harness parses Claude's `system` `compact_boundary` subtype. `Frame::System` handling in `crates/harness/src/claude/normalize.rs:232` handles only `task_notification`, `task_started`, and `init`; everything else returns `Vec::new()` (`normalize.rs:275` region: `if f.subtype != "init" || self.saw_init { return Vec::new(); }`). No `thread/compact` or `context_compaction` RPC is sent by the codex driver.

**Token usage tracking and the context meter: yes, it exists.**

- Wire event: `AgentEvent::ContextUsage { tokens: Option<u64>, window: Option<u64> }` (`crates/proto/src/agent.rs:381-387`) — "Latest context occupancy, independent of cumulative billing usage. Missing fields preserve the previous measurement."
- Separate `AgentEvent::Usage { input_tokens, output_tokens }` is "Kept as a harness passthrough (rate-limit probes); **never persisted to docs**" (`agent.rs:388-393`).
- Replicated snapshot type `zeron_proto::ContextUsage { tokens, window }` with `fraction()` at `agent.rs:574-586`.
- Stored in the Loro doc `meta` map under key `contextUsage` as an atomic JSON string ("A single atomic value prevents tokens and capacity from tearing on sync"): `crates/doc/src/schema.rs:316-342`. Cleared by `clear_context_usage` (`schema.rs:344-348`).
- Engine writes it on every `ContextUsage` event: `crates/engine/src/sessions.rs:1946-1947`.
- **Reset semantics**: the doc's context usage is cleared at the start of a run *only when the run is not resuming*: `crates/engine/src/sessions.rs:1437-1439` (`if request.resume.is_none() { let _ = doc.clear_context_usage(); }`).
- The value rides the transcript delta frame: `TranscriptUpdate { frame, context_usage }` at `crates/doc/src/transcript_delta.rs:25`.
- UI: `AppState.context_usage` (`crates/ui/src/state.rs:660`), updated from the watch pump (`state.rs:2299-2307`), cleared on chat switch/reset (`state.rs:892`, `:1640`, `:1869`).
- **The meter**: a 16px ring + percentage chip with a hover card, `crates/ui/src/context_usage.rs:9-78`; thresholds `>=0.9` danger, `>=0.75` warning (`context_usage.rs:15-19`). Rendered in the composer control cluster: `crates/ui/src/composer.rs:7907` (read) and `:7939` (`crate::context_usage::render(usage, …)`). Tooltip text at `context_usage.rs:85-108` including "Context usage not reported by this harness yet".

**Which harnesses emit `ContextUsage`:**

| Harness | Source | Citation |
|---|---|---|
| Claude | sums `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` from the assistant frame's `usage` (tokens only, no window); window arrives on the result frame | `crates/harness/src/claude/normalize.rs:456-467`, `:639-645` |
| Claude window option | model option `contextWindow` (`"1m"` variant) | `crates/harness/src/claude/catalog.rs:89-92`, `crates/harness/src/claude/mod.rs:177` |
| Codex | `thread/tokenUsage/updated` notification | `crates/harness/src/codex/mod.rs:1131`, mapping at `crates/harness/src/codex/normalize.rs:159` |
| ACP | `usage_update` session update (`used` + one of `max/limit/size/contextWindow/context_window`) | `crates/harness/src/acp/normalize.rs:430-441` |
| Opencode | per-message token counts + a provider/model context-window catalog lookup | `crates/harness/src/opencode/mod.rs:1780-1794` |
| **Cursor** | **none** — zero `ContextUsage` occurrences in `crates/harness/src/cursor/mod.rs` | (grep negative) |

The meter is **desktop-only**: `contextUsage` has no occurrences in `edge/src/` or `apps/ios/`.

## 3. Transcript / doc model

**Storage** — a Loro CRDT doc per chat, `SessionDoc` (`/Users/reinamaccredy/Code/zeron-fork/crates/doc/src/schema.rs:291`). Container layout documented at `schema.rs:1-15`:
- `meta`: LoroMap `{ chatId, schemaVersion }` (+ `contextUsage`), host-only writer (`schema.rs:302-309`)
- `messages`: LoroList of LoroMap `{ id, role, parts, createdAt, deviceId, status?, continuationOf? }`
- `commands`: LoroList (the durable command ledger)
- `queue`: LoroMovableList

`SessionMessageEntry` at `schema.rs:43-56`. Entry roles: `MessageRole { User, Assistant, System }` at `schema.rs:34-40`.

**System entries exist in the schema but have no writer.** `MessageRole::System` appears exactly once outside the enum definition — the serializer at `schema.rs:708` (`MessageRole::System => "system"`). Nothing in `crates/engine`, `crates/ui`, or `crates/doc` ever constructs one. The wire type exists on the TS side too (`edge/src/session-doc/messages.ts:12`: `"user" | "assistant" | "system"`; writer discipline note at `messages.ts:124-126`: "the host is the sole writer of assistant/system entries") and on iOS (`apps/ios/Zeron/Models/Entities.swift:216`: `case user, assistant, system`). So a "context compacted here" boundary has a schema slot on all three platforms, with **no existing renderer** — iOS's `TranscriptRows.swift:118` only branches on `entry.role == .user`, and the desktop transcript has no `MessageRole::System` branch.

**Writers**: `SessionDoc::push_message` for complete entries (user/system/command inserts) at `schema.rs:357-369`; streaming assistant entries go through `SegmentWriter::begin/resume/sync/finish` (`schema.rs:998-1236`). `set_message_status` at `schema.rs:502`, `append_error_part` at `schema.rs:529`.

**Hiding / collapsing**: there is **no doc-level hidden or collapsed flag**. Message parts (`DocPartJson`, `schema.rs:60-117`) have no visibility field; entries have only `status` (`streaming | complete | aborted`) and `continuationOf`. Collapsing in the desktop UI is purely a view-layer affair: tool-group collapse (`crates/ui/src/transcript.rs:397` `tool_group_collapses`), long user bubbles (`transcript.rs:1203` `user_message_needs_collapse`, `:2650` per-entry expand state, `:4426` toggle) — none of it is persisted.

**Truncation that already exists**: `materialize_tail` returns only the last N joined entries plus `total_messages` (`schema.rs:1246-1266`), with `TAIL_MESSAGE_COUNT = 64` (`crates/doc/src/constants.rs:21`). `join_continuation_entries` (`schema.rs:962`) merges entries split by the segment byte cap. This tail is what the edge and mobile see.

**Second durable store**: the append-only JSONL run journal, one file per chat, `{data_dir}/journals/{chat_id}.jsonl`, `{"seq": n, "event": AgentEvent}` (`crates/engine/src/run_journal.rs:1-11`, API at `:100 append`, `:149 replay`, `:165 last_event`, `:197 discard`). Full tool output lives only here (`crates/doc/src/parts.rs:364-371`).

**AgentEvent → transcript entries**: the fold is `fold_event_into_parts` in `/Users/reinamaccredy/Code/zeron-fork/crates/doc/src/parts.rs`:

| AgentEvent | Result |
|---|---|
| `TextDelta` | appends to trailing `MessagePart::Text` |
| `ReasoningDelta` | appends to trailing `MessagePart::Reasoning`; empty deltas dropped (`parts.rs:301-316`) |
| `ToolCall` | upsert `MessagePart::Tool` (`parts.rs:317-342`) |
| `ToolResult` | marks `resolved`/`is_error`; **output and diff text are deliberately dropped** — "Tool OUTPUTS never enter the doc … Full text lives only in the host's run journal" (`parts.rs:343-378`, esp. `:364-375`) |
| `InputRequested` / `InputResolved` | `MessagePart::Input` create / resolve (`parts.rs:379-405`) |
| `Error`, `Done{error}` | `MessagePart::Error` (`parts.rs:406-421`) |
| `GeneratedImage` | `MessagePart::Image` |
| `Subagent{..}` | lifecycle only onto the parent's spawn chip; content goes to the subagent's own doc (`parts.rs:422-474`) |
| `SessionStarted`, `Steered`, `AssistantMessageCompleted`, `Usage`, **`ContextUsage`**, `AvailableCommands`, `UserMessage` | **not persisted** (`parts.rs:478-482`) |

`ContextUsage` is thus a `meta` side-channel, not a transcript entry.

## 4. UI: panels, panes, slash-command submission

**Shell routes and panes** — `/Users/reinamaccredy/Code/zeron-fork/crates/ui/src/shell.rs`:
- `enum Route { Chat, Settings(SettingsSection) }` (`shell.rs:445-448`).
- **Right pane** with tabbed surfaces: `enum RightSurface { Picker, Files, File(u64), Browser(u64), Diff(u64), Terminal(u64), Subagent(u64) }` (`shell.rs:466-478`). Geometry helpers: `right_pane_max_width` (`:454`), `right_pane_takeover_width` (`:460`, takeover may consume the whole chat column), `right_panel_content_width` (`:189`).
- Per-chat open flags: `struct ChatPanels { terminal_open, changes_open, right_active }` (`shell.rs:503-509`) in a chat-keyed `SessionPanels` map (`shell.rs:515`), in-memory only, default closed. Toggles at `shell.rs:2309 toggle_right_pane`, `:3374` terminal, `:8859 toggle_right_pane_expand`.
- Bottom terminal panel: `crates/ui/src/terminal/panel.rs` via `ToggleTerminal` (`shell.rs:57`, `:3351`).
- Left: sidebar (`SidebarPane`, `shell.rs:1268`), plus a scroll rail (`crates/ui/src/rail.rs:411 render_rail`) and history (`crates/ui/src/history.rs`).
- Popover/overlay primitives: `crates/ui/src/popover.rs` (`popover_card`, `filter_indices`, `menu_step`, `reset_menu_scroll`), notice chip `crates/ui/src/notice.rs:21-40`, command palette `crates/ui/src/shell/command_palette.rs:7-150`.

**The strongest existing precedent for a side-conversation panel is the subagent tab.** It hosts a *second full transcript entity over a different doc id, with no composer*:
- Open: `shell.rs:3040-3088` — opens the right pane if closed, creates `Transcript::for_doc(self.state.clone(), doc_id.clone(), !frozen, cx)` (`shell.rs:3060-3061`), subscribes to transcript events, either fetches a frozen blob or calls `state.watch_subagent_doc(doc_id, cx)` (`shell.rs:3069-3071`), then pushes `RightSurface::Subagent(id)` onto the chat's tab list and activates it.
- `struct SubagentTab { doc_id, title, transcript, _fetch, _events }` (`shell.rs:1255-1263`).
- Render: `shell.rs:8075-8090` (the pane hosts its own jump pill because "there is no composer stack to clear").
- Backing doc: `subagent_doc_id(chat_id, tool_use_id)` → `{chatId}--sub--{suffix}` (`crates/engine/src/sessions.rs:1148-1168`), written by `SubagentSink` (`sessions.rs:1174-1289`) through the same `SegmentWriter`.
- Transport: the UI subscribes to `methods::WATCH_DOC_MESSAGES` with `{"chatId": doc_id}` — the same RPC the main transcript uses (`crates/ui/src/state.rs:2271` main, `state.rs:2405-2407` subagent).

**Slash-command popup submission** — it is purely a text completion; there is **no local interception**:
- Tokenizer `slash_token` (`composer.rs:3913-3932`): only fires when the text *starts* with `/`, only while the cursor is inside the first token, and a query containing another `/` never triggers. Doc comment at `:3910-3912` explicitly cites `/compact` as the example.
- `SlashState` (`composer.rs:4026-4045`) caches per-harness command lists from `methods::LIST_COMMANDS` (`composer.rs:5418`), filtered locally.
- `accept_slash` (`composer.rs:5488-5510`) does exactly one thing: `input.replace_plain_token(token.range, &format!("/{}", command.name), cx)`. It **leaves text in the prompt**.
- The text then goes through the ordinary `send()` path → `SessionCommandPayload::Run { request: RunRequest { prompt: <the text> … } }` (`composer.rs:6509`). No shell/composer branch inspects the command name.
- **The single exception is opencode, driver-side**: `post_prompt` strips a leading `/`, and if the first token matches a known advertised command, it routes to `POST /session/{id}/command` (v1, body `{command, arguments}`) or `POST /api/session/{id}/command` (v2, body `{command, text}`) on a detached task instead of the prompt endpoint (`crates/harness/src/opencode/mod.rs:1901-1962`). Comment at `:1900-1904`: "the server does NOT parse slash text out of an ordinary prompt".
- The command palette (`crates/ui/src/shell/command_palette.rs`) is a separate Cmd-K surface over app actions and chats (`:91 command_entries`, `:140 activate_command`), unrelated to agent slash commands.

## 5. Sync / edge / iOS

- The transcript doc **does** sync. Two generations, per chat: `Chat.room_gen` (`crates/proto/src/entities.rs:155-160`) — `None`/1 = legacy loro room, 2 = the chat2 dumb relay; `Chat::on_chat2()` at `entities.rs:165`.
- Edge Durable Objects: `edge/src/chat-room.ts`, `edge/src/session-room.ts`, `edge/src/chat-log.ts`, `edge/src/chat-frames.ts`, blobs at `edge/src/blobs.ts`.
- The edge's TypeScript mirror of the doc schema lives at `edge/src/session-doc/`: `messages.ts`, `render-parts.ts`, `tail.ts`, `sidecar.ts`, `control-types.ts`, `constants.ts`, `index.ts`. The Rust schema header states the layout "MUST stay shape-compatible with the TS edge/tail materializer" (`crates/doc/src/schema.rs:3`).
- `SessionMessageRole = "user" | "assistant" | "system"` (`edge/src/session-doc/messages.ts:12`); `SessionMessageEntry` with `role`, `parts`, `tokens?`, `status?`, `continuationOf?` at `messages.ts:118-139`. Part decoding at `messages.ts:95-105` — unknown kinds fall back to `{kind:"text", text: p.text ?? ""}`.
- iOS mirrors it: `MessageRole { case user, assistant, system }` (`apps/ios/Zeron/Models/Entities.swift:216`), `var role: MessageRole` at `Entities.swift:280`. Rendering in `apps/ios/Zeron/Transcript/TranscriptRows.swift:118` branches only on `.user` vs everything-else — a `system` entry today renders as an assistant-styled row.
- **Context usage does not sync to edge or iOS**: no `contextUsage` / `context_usage` occurrence exists under `edge/src/` or `apps/ios/`. The meter is a desktop-only read of the `meta` map through `TranscriptUpdate.context_usage` (`crates/doc/src/transcript_delta.rs:25`).
- Host-side checkpointing/seeding of the room: `crates/engine/src/doc_host.rs:1350 spawn_chat2_join`, `:1933 spawn_chat2_checkpoint`, `:4475 save_snapshot`.
- Tool output/diff sidecars are keyed `{chatId}/{partId}` matching the edge's `/blob/{chatId}/{partId}` route (`crates/doc/src/parts.rs:486-512`).

## Constraints for the design

1. **No Write/hidden/collapsed bit exists in the doc.** Entries carry only `status` (`streaming|complete|aborted`) and `continuationOf`. A "hide history above here" flag is a new schema field that must be additive and tolerated by the TS mirror (`edge/src/session-doc/messages.ts:118`) and iOS (`apps/ios/Zeron/Models/Entities.swift:280`).
2. **`MessageRole::System` is a live schema slot with zero writers and zero renderers** on all three platforms (`crates/doc/src/schema.rs:36-40` + `:708`; `messages.ts:12`; `Entities.swift:216`). A compaction boundary entry can use it, but every renderer needs a new branch; today iOS would render it as an assistant bubble.
3. **One runtime per chat id.** `dispatch_inner` interrupts and replaces the live run whenever the new request can't be routed into the existing mailbox (`crates/engine/src/sessions.rs:427-437`). Any side run keyed on the same `chat_id` would kill the main turn.
4. **There is no non-transcript-appending command kind.** `SessionCommandPayload` has exactly `Run | Steer | Interrupt | RespondInput` (`crates/doc/src/commands.rs:39-57`), and `dispatch` unconditionally writes a user entry to the doc (`sessions.rs:399`, `:442`).
5. **`run_title` is the only secondary-run primitive, and it is context-free by construction** (scratch tempdir, `resume: None`, tools disabled — `crates/engine/src/titles.rs:170,193-205`; `crates/harness/src/claude/mod.rs:401-424`), implemented only by Claude, Codex and Mock (`crates/harness/src/lib.rs:306-311`). Nothing today runs a turn *with* session context that skips the transcript.
6. **Resume is cwd-gated and last-writer-wins.** `resume_for` refuses a session created under a different cwd (`sessions.rs:1073-1089`); every `SessionStarted`/`Done` overwrites the stored id (`sessions.rs:2166-2179`). Any side run that produces its own `SessionStarted` on the same chat would clobber the main chat's resume pointer.
7. **No harness supports forking/branching a session.** All five resume paths append to the harness-side transcript (claude `--resume=`, codex `thread/resume`, opencode session id reuse, acp `session/load`, cursor `resume`). A side question sent on the resumed session will land in the harness's own history regardless of what zeron's doc shows.
8. **No compaction/summarize plumbing exists anywhere.** Claude's `system`/`compact_boundary` frames are swallowed by the catch-all in `crates/harness/src/claude/normalize.rs` (only `init`, `task_started`, `task_notification` are handled); no codex `thread/compact`-style RPC is issued; journal compaction is explicitly deferred (`crates/engine/src/run_journal.rs:10`).
9. **Context usage is a `meta` side-channel, not a transcript entry** (`crates/doc/src/parts.rs:478-482`; stored at `crates/doc/src/schema.rs:325-342`), and it is **cleared only when a run starts without `resume`** (`crates/engine/src/sessions.rs:1437-1439`). Because resume is always injected for an existing chat, a compaction would need an explicit `clear_context_usage` call to reset the meter.
10. **Cursor reports no context usage at all** and advertises no slash commands (no `ContextUsage` in `crates/harness/src/cursor/mod.rs`; no `commands()` impl). Any "compact when full" affordance is dark for cursor.
11. **Slash commands are inert text on the desktop side.** `accept_slash` only substitutes the token (`crates/ui/src/composer.rs:5488-5510`); the prompt ships as an ordinary `RunRequest`. Adding local interception means a new branch in `Composer::send` (`composer.rs:5952`) — and it must not break opencode's driver-side `/command` routing (`crates/harness/src/opencode/mod.rs:1901-1962`), which is the one place a `/name` is already special-cased.
12. **Two composer send paths must both be considered**: `QUEUE_MESSAGE` + `holdForTurnEnd` for busy chats (`composer.rs:6490-6506`) and `QUEUE_COMMAND` Run otherwise (`composer.rs:6509-6541`).
13. **A read-only side transcript pane already has a working template**: `RightSurface::Subagent(u64)` + `SubagentTab` + `Transcript::for_doc(state, doc_id, follow, cx)` over a `{chatId}--sub--{suffix}` doc, fed by `WATCH_DOC_MESSAGES` on that doc id (`crates/ui/src/shell.rs:466-478`, `:1255-1263`, `:3040-3088`, `:8075-8090`; `crates/engine/src/sessions.rs:1152-1289`; `crates/ui/src/state.rs:2405-2407`). It hosts **no composer** — a side-chat input is net-new.
14. **The mobile/edge view is the 64-entry tail** (`materialize_tail`, `crates/doc/src/schema.rs:1246-1266`; `TAIL_MESSAGE_COUNT = 64`, `crates/doc/src/constants.rs:21`). A design that hides history by *deleting* entries and one that hides it by *marking* them differ sharply here: marking leaves the tail count unchanged, deletion shifts what every mobile client sees.
15. **Tool outputs are already absent from the doc** (journal-only, `crates/doc/src/parts.rs:364-375`). A compaction that wants to summarize real tool output must read the run journal (`crates/engine/src/run_journal.rs:149 replay`), not the doc.
16. **Doc writer discipline**: the host is the sole writer of assistant/system entries and of outcomes; peers only insert their own entries (`crates/doc/src/commands.rs:1-9`; `edge/src/session-doc/messages.ts:119-126`). Both features must write from the host, not the composer's device.