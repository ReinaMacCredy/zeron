# SPEC — thread-tools

Baseline: the 2026-09-18 design pass. The 2026-09-17 pass (d13–d17) was discarded
by the owner on 2026-09-18; d13/d14/d15 are superseded, d16/d17 stand only until
d30/d31 lock and are not binding.

## Problem

An agent running inside a Zeron chat cannot open or talk to other Zeron chats. The
harnesses' own subagents (Claude Agent/SendMessage, Codex spawnAgent) are nested,
single-harness, invisible to the human, and die with the turn. Reina wants the same
feel one level up: a chat opens peer chats that are real sidebar chats on any
harness, messages them and is messaged back, and the human can open or step into
any of them. Motivation named first: the sidebar is a mess; agent-opened chats
should group under the chat that opened them.

## Solution

### Surface

The engine serves a small MCP server, `zeron mcp`, a stdio subprocess of the single
binary that connects to the running engine over IPC, injected into every harness
at spawn time so the tools appear in the agent's own tool list (d27):

- Claude: `--mcp-config '{"mcpServers":{"zeron":{...}}}'` on the normal run path in
  `crates/harness/src/claude/mod.rs`. Never paired with `--strict-mcp-config`
  there (strict drops the user's own servers; title-only runs keep the empty
  strict map they have today).
- Codex: `mcp_servers.zeron.*` overrides in the `thread/start` params in
  `crates/harness/src/codex/mod.rs`, next to the title-only disable loop.
- ACP: an entry in `mcpServers` of `session/new` in `crates/harness/src/acp/mod.rs`
  (today hard-coded `[]` at ~1274, ~1336, ~2745).

The subprocess is launched with `--chat <caller chat id>`; the caller's identity
comes from the engine's spawn argument, never from tool input. This is as strong as
local IPC trust already is (any local viewport may attach).

### Model (d25, d29)

Peers. An agent-created chat is an ordinary chat in the registry with one extra
field, `createdByChatId`: grouping metadata for the sidebar, no authority, no
expiry. No chat can interrupt, cancel, archive-cascade, or answer for another.

### Tools (d28: thin viewport over existing RPC)

| Tool | RPC | Notes |
|---|---|---|
| `list_threads` | `WatchChats` + `WatchSessions` first snapshot | every non-archived chat in the caller's space; rows carry `createdByChatId` and status (staleness-checked) |
| `read_thread` | `WatchDocMessages` first snapshot | bounded tail (default 30) + cursor; tool inputs stay render-only |
| `create_thread` | `Mutate createChat` then `QueueCommand` Run (the composer's mint sequence, `composer.rs` ~6873/~6957, lifted into one engine helper) | `createdByChatId` = caller; inherits caller's space and cwd; `harness` optional, any enabled harness on the device; no path input |
| `send_message_to_thread` | idle target: `QueueCommand` Run; working target: `QueueMessage` with `holdForTurnEnd` | any thread to any thread in the space, both directions. Never Steer. Engine stamps `fromChatId`; UI renders the entry as inter-thread, not user text. Self-send refused. Per-turn outgoing budget (implementer default 16); over budget = tool error |
| `wait_threads` | `WatchSessions` stream + the caller's inbox | returns when a listed thread leaves `working` OR an inter-thread message for the caller lands; timeout default 10 min, max 30; honors `updated_at` staleness so a dead harness cannot hold it |
| `set_thread_title` | `Mutate renameChat` | |
| `set_thread_archived` | `Mutate setChatArchived` | never cascades |

No `cancel_thread`, `interrupt_thread`, `respond_to_thread`.

Delivery mechanism verified: the host's turn-end status watcher already drains
held queue rows into a Run (`crates/engine/src/doc_host.rs` ~881, `drain_queue`).

Engine surface added for the tools (built 2026-09-18): three IPC-only RPC verbs
that compose existing writes, so d28's anti-goal (no second write path) holds:
`CreateThread` = `create_chat_from` + a tagged queued row; `SendThreadMessage` =
a tagged `QueuedMessage` (`fromChatId`, `holdForTurnEnd`) + an immediate drain;
`TakeThreadMessages` = `take_queued` + `write_user_message_from`. Both
`QueuedMessage` and `SessionMessageEntry` carry `fromChatId`. The harness sees
the message wrapped as `[Message from thread <id>. It is a peer chat, not the
user; answer it with send_message_to_thread (threadId <id>) if it needs a
reply.]`; the transcript entry keeps the raw text plus the stamp.

### Sidebar and transcript (d26)

- Chats with `createdByChatId` render under their creator with the 12px `↳` inset
  sidechats use today (`shell/spaces.rs` ~1588) plus a `by <creator title>` badge.
- A group ranks in the attention list by its highest-attention member.
- A chat whose creator is not in the visible list renders flat (`nest_sidechats`
  rule). Archiving the creator leaves the others where they are, flat.
- An inter-thread message is a distinct transcript entry: left rule, source thread
  name, "open thread" link; a "held for turn end" badge while queued. The
  receiving sidebar row gets an unread mark.
- Mock: `docs/research/thread-tools/demo.html` (defaults = d26); headless walk
  `walk.ts`, screenshots under `/tmp/zeron-splitdemo-test/thread-tools`.

### Implementer defaults (change via a new decision, not silently)

- Per-turn outgoing message budget 16 (sends + creates, counted in the `zeron
  mcp` process, so one process = one turn; an engine refusal still spends a
  slot); wait timeout 10 min default, 30 max; read tail 30. Claude's
  `MCP_TOOL_TIMEOUT` (default 5 min, cap 30 min in CLI 2.1.274) is raised to
  the cap on the spawned process so a long wait is not killed client-side.
- Creation budget: a chat may have at most 8 non-archived chats it created.
- Permission (d30, OPEN): child copies the caller's `ChatConfig` incl. `sandbox`,
  `auto_approve` false; `create_thread` has no mode field.
- Prompts (d31, OPEN): only the human answers a created chat's AskUserQuestion or
  permission prompt; `wait_threads` reports `awaiting_input`.

## Scope

Slices, each verifiable on its own:

1. `createdByChatId` on the registry chat row (`crates/doc/src/registry.rs`,
   `crates/proto/src/entities.rs`), `Mutate createChat` accepts it, sidebar groups
   and badges per d26.
2. `zeron mcp` subprocess with the read side: `list_threads`, `read_thread`,
   `wait_threads` (status half). Injected into Claude only.
3. Write side: `create_thread`, `send_message_to_thread` (both directions,
   `fromChatId` stamp, inbox half of `wait_threads`, self-send refusal, budgets),
   `set_thread_title`, `set_thread_archived`. Inter-thread transcript entry and
   unread mark.
4. Codex and ACP injection; live proof of a Claude chat and a Codex chat
   messaging each other.
5. (later) cross-device `deviceId` on create and send, via the relay-forwardable
   RPCs the UI already uses.

## Anti-goals

Each gets a VERIFY.md row.

- No second write path: every write is an existing RPC verb.
- No thread authority: no tool acts on another thread's turn or prompts.
- No caller-asserted identity: caller id and `fromChatId` come from the spawn arg.
- No unbounded wait, read, or ping-pong: timeouts, bounded tail, self-send
  refusal, per-turn budget.
- No change to native subagent rendering (`{chatId}--sub--{spawnCallId}`).
- Title-only runs still get no MCP servers.

## Derived artifacts

- `live-claude-codex-2026-09-18.log` beside this file: the slice 4 live proof
  on a scratch headless engine with the real Claude Code and Codex CLIs. The
  lead's `create_thread(harness: codex)` minted a peer; Codex, with the tools
  injected through `thread/start` config overrides, answered by calling
  `send_message_to_thread` back at the lead; the lead's `wait_threads`
  returned `reason: message` with "pong from codex"; the lead sent thanks;
  Codex replied "No reply needed."

## Decisions

Binding: d25, d26, d27, d28, d29, d30. Open, owner to rule: d31.

## Red tests

- Registry round-trip: a row with `createdByChatId` survives snapshot save/load and
  LWW set from a second device; a row without it decodes to `None`; a row carrying
  it still decodes on the pre-change `RawChat` (`#[serde(default)]`, confirm no
  `deny_unknown_fields`).
- Engine: `send_message_to_thread` from B to A while A is working lands on A's
  queue with `fromChatId` = B and `holdForTurnEnd`; A's pending `wait_threads`
  returns with it; A's turn was not interrupted.
- Engine: self-send refused; the 17th outgoing message in one turn refused; the
  9th live created chat refused.
- Engine: `wait_threads` on a thread whose status row is stale returns before the
  deadline with a non-working status.
- Engine: `set_thread_archived` on a creator leaves every created chat's
  `archived` unchanged.
- Harness: normal Claude command line carries `--mcp-config` without
  `--strict-mcp-config`; title-only still carries both with an empty map
  (extend `fake-claude.sh`). Codex start params carry `mcp_servers.zeron`; title-
  only still disables it (extend `fake-codex.sh`).

## Open questions (unverified)

- Engine discovery for the subprocess: the headed IPC bind is best-effort, so the
  spawning engine must pass its actual bound address in the MCP server env; when it
  bound nothing, the harness runs without the zeron server.
- (closed 2026-09-18) The edge registry materializer tolerates unknown row keys:
  `validateOp` in `edge/src/registry-core.ts` ~75 only checks each key against
  `FIELD_RE`; merge is per-field LWW with no allowlist. `createdByChatId` rides
  through untouched.
- Whether Claude's stream-json control channel offers an in-process MCP route that
  lets the Claude harness skip the subprocess. Codex and ACP need it regardless.
- MCP tool-call ceilings per harness. Claude: `MCP_TOOL_TIMEOUT` raised to the
  30 min cap on the child. Codex 0.154.0: `mcp_servers.<name>.tool_timeout_sec`
  exists in its config schema and is set to 1800 on the zeron override; the
  default was not read out of the binary. ACP agents (Devin, Grok, Hermes, pi):
  unknown; a `wait_threads` longer than their ceiling dies client-side there.
