# NOTES — thread-tools

Overwritten handoff, never appended; history lives in trace and decisions.

Base: 1259573c (main)

## Current State

Baseline is the 2026-09-18 design pass. The owner discarded the 2026-09-17 pass
("lấy bây giờ làm chuẩn, bỏ hôm qua đi"): d13/d14/d15 superseded by d27/d28/d29;
d16/d17 will be superseded when d30/d31 lock. Binding today: d25 (peer model,
two-way messaging, no cancel, budgets, space scope), d26 (sidebar UX picked on
the demo), d27 (goal + MCP surface), d28 (thin viewport over existing RPC), d29
(`createdByChatId`, grouping only). SPEC and VERIFY rewritten from that
baseline. UX demo built and walked headlessly:
`docs/research/thread-tools/demo.html`. No code touched.

Recon facts already established (do not re-derive):
- Six of the seven owner-requested ops exist as RPC today
  (`crates/rpc/src/lib.rs` methods: Mutate createChat/renameChat/
  setChatArchived, QueueCommand, QueueMessage holdForTurnEnd, WatchChats,
  WatchSessions, WatchDocMessages). `crates/rpc/examples/e2e_driver.rs` ~152
  shows the exact createSpace/createChat/QueueCommand param shapes.
- Held queue rows are drained into a Run by the turn-end status watcher
  (`crates/engine/src/doc_host.rs` ~881, `drain_queue`).
- Sidechat nesting = 12px `↳` inset (`shell/spaces.rs` ~1588); the
  `nest_sidechats` rule renders an orphan flat. Sidechat link expires 24h and
  wraps the first turn in a transcript copy; not reusable for peers.
- No engine-served MCP exists. Claude title-only path pairs `--mcp-config` with
  `--strict-mcp-config` (claude/mod.rs ~464). Codex title-only disables every
  `mcp_servers.<name>` (codex/mod.rs ~956). ACP sends `mcpServers: []`
  (acp/mod.rs ~1274, ~1336, ~2745).
- `Session.status` is staleness-checked client-side against `updated_at`
  (`crates/proto/src/entities.rs` ~229).

## Slice 1 (w50) — DONE 2026-09-18, branch feat/thread-tools in
`.maestro/worktree/thread-tools` (own CARGO_TARGET_DIR), not merged, not pushed.

- `Chat.created_by_chat_id` (proto), `RawChat` + registry writers (doc),
  `MutateParams::CreateChat.created_by_chat_id` → `WorkspaceHost::create_chat_from`
  (engine; unknown creator refused; `create_chat` is the 5-arg wrapper so the
  30 existing callers stay untouched).
- Sidebar: `nest_linked_chats` (was `nest_sidechats`) nests both sidechats and
  created chats; a created chat lifts its creator's group to the earliest
  member position (d26 A); `render_chat_row` gained `created_by` and draws the
  `by <creator>` badge after the title (d26 B), truncated at 14 chars.
- Tests added: doc registry cross-device round-trip, RawChat compat decode,
  engine create/refuse/archive-no-cascade, ui lift+orphan ordering.
- Suites: zeron-ui lib 1113/1113 (`--test-threads=1`), doc/proto/sync/rpc green,
  engine green except the two known environmental failures on this Mac
  (`e2e generated_image…` passes with a real TMPDIR; `local_first
  online_runtime_shutdown…` fails while a Zeron app is running, pid 91008 on
  27654 today; diff touches no shutdown/preview/sync code).
- Not done: an in-app screenshot of the badge and group lift (needs a dev
  bundle build; the HTML demo stands in). d30 locked as its default (owner
  "duyệt" 2026-09-18); d31 still a draft with the default applying.

## Slice 2 (w51) — built 2026-09-18 on feat/thread-tools (see the commit).

- New crate `crates/mcp` (`zeron-mcp`): MCP stdio server, newline JSON-RPC
  2.0 (`initialize`, `ping`, `tools/list`, `tools/call`), tools
  `list_threads` (caller's device+space, non-archived, staleness-checked
  status), `read_thread` (bounded tail + `before` cursor, reasoning dropped,
  other parts named), `wait_threads` (returns on any listed thread leaving
  working, on a stale Working row via a 5s re-check, or on timeout; default
  600s, max 1800s). 7 unit tests over an in-memory `RpcService` fake.
- `zeron mcp --chat <id> --port <p>` (hidden clap subcommand) dials the IPC
  port and serves stdio; tracing goes to stderr for that command only.
- `zeron_harness::ToolServer {command, args}` + `RunControls.tool_server`;
  every harness destructure now ends in `..`. Claude's normal run path adds
  `--mcp-config <json>` (additive; title-only keeps its strict empty map).
  `SessionsEngine::set_tool_server_port` is called after a successful IPC
  bind in both `Engine::run` (headless) and the headed boot in
  `crates/ui/src/state.rs`; `Inner::tool_server_for` builds the spec from
  `current_exe()`.
- Proof: fixture test `thread_tools_ride_an_additive_mcp_config_on_a_normal_run`
  (fake-claude.sh `scenario:mcp` exits 1 unless the flag is right), and a live
  smoke of the built binary against the production engine (port 27654):
  initialize → tools/list → list_threads (18 rows) → read_thread (2 entries +
  cursor) → wait_threads (returned `status`, idle). Not yet: a real Claude
  turn calling the tools inside the app (needs a dev bundle).

## Slice 3 (w52) — built 2026-09-18 on feat/thread-tools (see the commit).

- `QueuedMessage.from_chat_id` and `SessionMessageEntry.from_chat_id`
  (`fromChatId` in both Loro maps; salvage reader too). Engine:
  `DocHost::queue_thread_message` (peer check, tagged held row, immediate
  drain so an idle target is Working before the call returns),
  `take_thread_messages` (tagged rows → transcript entries, marker bumped),
  `create_thread` (creator's config/cwd, harness override, 8-live budget,
  first turn is a message from the creator); `dispatch_queued` writes the
  tagged entry first and frames only the harness prompt. RPC verbs
  `CreateThread`, `SendThreadMessage`, `TakeThreadMessages`.
- MCP tools now: list, read, create, send, wait (status + inbox halves; takes
  the inbox rows), set_thread_title, set_thread_archived; peer scope on every
  target; send budget 16 per process. Claude child gets
  `MCP_TOOL_TIMEOUT=1800000` (CLI default 5 min would kill a long wait).
- UI: a user entry with `from_chat_id` renders left-aligned, accent-washed
  with a left rule under a "↳ from <title>" line (`transcript.rs`
  `RowKind::User.from_chat_id`). The unseen dot rides the existing
  `last_message_at` bump. The "held for turn end" badge on a queued
  inter-thread row (d26 C) is NOT built.
- Tests: mcp 11 (fake RpcService), engine `tests/thread_tools.rs` 6
  (recording harness), doc round-trips unchanged. Live: scratch headless
  engine (`ZERON_DATA_DIR=/tmp/zeron-tt-lab ZERON_IPC_PORT=27777
  ZERON_HARNESS=mock`, driver `/tmp/zeron-tt-lab/mcp_drive.py`) proved
  lead → create → wait → read → send → wait → rename, then created → lead
  with the lead's tail showing `from=<created id>`. Never point the write
  tools at the production engine on 27654.

## Slice 4 (w53) — built 2026-09-18 on feat/thread-tools (see the commit).

- Codex: `thread/start` config overrides `mcp_servers.zeron.command/args`
  plus `tool_timeout_sec: 1800` (dotted keys merge over the user's table;
  title-only untouched). ACP:
  `session/new` `mcpServers: [{name: zeron, command, args, env: []}]` on the
  run path only. Fixture tests in `codex.rs` / `acp.rs` (+ `fake-codex.sh`
  `scenario:mcp`, `fake-acp.sh` ZERON_TOOLS gate).
- d26 C: a queued row with `from_chat_id` shows "↳ from <title> · held for
  turn end" under its text (`crates/ui/src/queue.rs`).
- LIVE PROOF (real CLIs, scratch headless engine on 27778, log
  `live-claude-codex-2026-09-18.log` beside SPEC): Claude created a Codex
  peer, Codex used `send_message_to_thread` to answer "pong from codex", the
  lead's `wait_threads` returned `reason: message`, thanks went back, Codex
  replied "No reply needed." Claude Code defers MCP tools, so the lead spent
  one `ToolSearch` call loading the zeron schemas first.
- Known failing on this Mac at base, not from this branch: codex
  `approvals_round_trip_as_input_requests`, `approval_no_answer_becomes_decline`,
  `models_discovers_visible_catalog_with_pagination`. REPRODUCED at base
  af1ce76c in a clean detached worktree with its own target dir
  (`cargo test -p zeron-harness --test codex`: 22 passed, 3 failed, the same
  three). They pass inside the full `-p zeron-harness` run and fail when the
  codex binary runs alone, so they are order/env dependent.
- Still not done: in-app screenshots (sidebar group/badge, "↳ from" entry,
  held badge). Needs a dev bundle; the HTML demo stands in.

## Merged 2026-09-18

`feat/thread-tools` (10 commits, 83585e00..ce36b897) merged into local main
as 75a41214 on the owner's word ("commit và merge toàn bộ vào main"), no
conflicts against the peer's 5aede303. NOT pushed. Worktree removed, branch
deleted; the Dev TT bundle moved to `target/macos-dev-tt` (data
`/tmp/zeron-dev-tt/data`, IPC 27690). Late additions after the slice 4
report: `create_thread` picks harness/model/reasoning (6d104729, sub-agent),
peer message rendered as the C3 rule ("From <thread>", advisor glyph, name
opens the thread; 249f6ba4, ce36b897), Codex `tool_timeout_sec` (4d3d211e).

## Next Action

Push when the owner says so. d31 (who answers a created chat's prompts) is
still an owner draft; the shipped default is human only. Open owner call
from the sub-agent: whether a harness switch on `create_thread` should
clear the inherited model/reasoning.

## Authority

Transferred: implementer may set the budgets, wait timeout and read tail
defaults listed in SPEC; changing d25–d29 needs the owner; d30/d31 defaults
apply until the owner rules.

Retained: cross-device slice 5 scope; any tool that acts on another thread;
pushing or tagging.

## Failed approaches

None yet.

## Do not repeat

- Do not treat d13–d17 as binding; the owner discarded that pass.
- Do not name the field `parentChatId`; today's name is `createdByChatId` and
  it carries no authority.
- Do not reuse `sidechatSourceChatId` as the link (expiry + transcript copy).
- Do not add `cancel_thread` or any tool that acts on another thread's turn.
- Do not Steer a working target on `send_message_to_thread`; hold for turn end.
- Do not pair `--mcp-config` with `--strict-mcp-config` on the normal Claude
  run path; it would drop the user's own MCP servers.
