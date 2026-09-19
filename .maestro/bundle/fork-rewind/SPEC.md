# SPEC — fork-rewind

## Problem

A zeron chat is linear: every prompt appends to the same transcript and the same harness session (`crates/engine/src/sessions.rs` `dispatch_inner`, `resume_for`). There is no way to branch a conversation after a reply to try another direction, nor to go back to before a prompt and re-send it. Claude Code has `/branch` and `/rewind`, Codex has `thread/fork`, OpenCode has `session/fork` and `session/revert`; zeron exposes none of them (recon `.maestro/recon-sidechat-compact.md` §1 "Forking / branching: none anywhere").

## Solution

One engine primitive, two products (d9).

- **Anchors** (d11). Drivers emit `AgentEvent::Anchor { role, id, entry_id: None }` for the harness's own handle of a prompt or reply: Claude the message `uuid` of the echoed user frame (`--replay-user-messages`) and of each main-channel assistant frame; Codex the turn id from the `turn/start` response; OpenCode the message id. The run loop stamps `entry_id` (current user entry for `role: user`, current assistant entry for `role: assistant`) before journaling. Anchors never enter the doc.
- **Pending branch** (d9). `AgentEvent::SessionBranch { keep, drop }` is an engine-written journal record plus an in-memory map: the chat's next run must branch the resumed harness session so that history through `keep` survives and `drop` onward is discarded. `dispatch` copies it onto `RunRequest.branch`; the driver applies it (Claude `--resume <id> --fork-session --resume-session-at <keep>` plus `--resume-drops-turn <drop>` when known; Codex `thread/fork { threadId, lastTurnId: keep }`; OpenCode `POST /session/{id}/fork { messageID: keep }`); the run's `SessionStarted` records the new session id and clears the pending branch. `keep: None` means no history: the session is tombstoned and the run starts fresh.
- **Fork** (assistant row): `ForkChat { sourceChatId, entryId, chatId }` (host-side, relay-forwardable). Refused when the harness cannot branch, or the reply has no anchor. Creates a chat row in the source's space/cwd/checkout/config titled `Fork: <source title>`, seeds its doc with the source's raw entries up to the next user entry after `entryId`, remembers the source's harness session on the new chat, and records the pending branch `{ keep: anchor(entry), drop: anchor(next user entry) }`. Sidechat sources are allowed; the fork drops `sidechat_source_chat_id`, so it is a standalone chat.
- **Rewind** (user row): `RewindChat { chatId, entryId }` (host-side). Refused when the harness cannot branch, the chat has a live run, or the previous reply has no anchor. Deletes the doc's raw entries from `entryId` to the end, clears `contextUsage`, records the pending branch `{ keep: anchor(previous assistant entry), drop: anchor(entry) }` and replies with the removed prompt text, which the UI puts back into the composer. Rewinding before the first prompt tombstones the harness session.
- **UI** (d10, d12). Hover strip: user rows `timestamp · copy · rewind`, assistant rows `timestamp · copy · fork`. Either opens a centered palette-style modal; confirm fires the RPC. Fork selects the new chat; rewind restores the prompt. On harnesses whose `branching()` is false the actions render disabled with the reason. Capability rides the existing `HarnessCompaction` reply as `branching`.
- Every Claude run sets `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=true` (d10).

## Scope

- `crates/proto`: `AgentEvent::Anchor`, `AgentEvent::SessionBranch`, `AnchorRole`, `SessionBranch`, `RunRequest.branch`.
- `crates/harness`: `Harness::branching()` (default false); Claude (flags, env, uuid parse, anchors); Codex (`thread/fork`, turn anchors); OpenCode (`fork`, message anchors) if its driver exposes message ids without restructuring, else recorded as follow-up; Mock stays false.
- `crates/doc`: `SessionDoc::truncate_from(entry_id)`, `SessionDoc::raw_entries_until_next_user(entry_id)`; fold ignores the two new events.
- `crates/engine`: anchor stamping, pending-branch map + journal recovery, `dispatch` injection, fork/rewind operations, `ForkChat`/`RewindChat` RPCs (forwardable), `branching` on the capability reply.
- `crates/rpc`: two method constants.
- `crates/ui`: strip actions, `TranscriptEvent::Branch`, modal, RPC calls, composer prompt restore, capability cache.
- `edge/`, `apps/ios`: no change; docs shrink or appear through the ordinary row protocol.

Out of scope (recorded): fork into a new worktree; code restore; a per-entry anchor pre-check RPC; ACP/Cursor branching.

## Anti-goals

Each anti-goal gets a matching VERIFY.md check.

1. Fork never writes to the source chat's doc, row, harness session or context usage.
2. No doc schema field is added; anchors and pending branches live only in the journal and the run-time map.
3. A fork or rewind whose anchor is missing is refused with a reason, never applied at a guessed point.
4. Rewind is refused while the chat has a live run.
5. The doc's hidden state after a rewind is empty: the entries are deleted, not marked.
6. No harness other than Claude, Codex and OpenCode gains a branch call; ACP and Cursor report `branching: false`.

## Decisions

d9, d10, d11, d12

## Derived artifacts

None.

## Red tests

- engine: fork copies entries through the kept reply only, titles `Fork: …`, drops `sidechat_source_chat_id`, remembers the source session on the new chat, journals `SessionBranch { keep, drop }`; the source doc and row are byte-identical before and after.
- engine: rewind deletes entries from the prompt, clears context usage, returns the prompt text, journals the branch with `keep` = the previous reply's anchor; rewind before the first prompt tombstones the session.
- engine: the next dispatch after a pending branch hands the harness `RunRequest.branch`; a `SessionStarted` clears it; refusal paths (no anchor, live run, non-branching harness) return errors and leave the doc untouched.
- harness/claude: the arg builder emits `--fork-session --resume-session-at K --resume-drops-turn D` and the env var; the normalizer turns a replayed user frame and an assistant frame into `Anchor` events with their uuids.
- harness/codex: a branch request sends `thread/fork { threadId, lastTurnId }` instead of `thread/resume`.
- ui (`crates/ui/src/transcript.rs` tests): user rows carry the rewind action and assistant rows the fork action; a non-branching harness renders both disabled.
