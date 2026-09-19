# SPEC — sidechat

## Problem

A user in a zeron chat wants to ask the agent about the current work (explain, compare, sanity-check) without that exchange continuing the chat's task or landing in its history. Today every prompt on a chat id is a turn on that chat: `dispatch_inner` interrupts and replaces the live run when the prompt cannot ride the steering mailbox (`crates/engine/src/sessions.rs:427-437`) and unconditionally writes a user entry (`sessions.rs:399`, `:442`). The only secondary run, `run_title`, is context-free by construction (`crates/engine/src/titles.rs:170`, `:193-205`). Zeron therefore has no way to ask a side question against a chat's context.

Reference: astra commit `1dc67604e` (guarded side conversations) and its follow-ups (dock in utility pane, status pill). t3code has no equivalent.

## Solution

A sidechat is an ordinary `Chat` that points at its source.

- `Chat` gains `sidechat_source_chat_id: Option<String>` (`crates/proto/src/entities.rs:110-161`), serde `default` + `skip_serializing_if`, so edge and iOS tolerate it as an unknown field until they render nesting.
- Invocation, two entry points. (a) A `Sidechat` row in the right pane picker (`RightSurface::Picker`, `crates/ui/src/shell.rs:466-478`) under Files / Browser / Terminal / Diffs / History: opens the chat's most recent sidechat as a `RightSurface::Sidechat` tab, or creates one with an empty first turn when none exists; closing the tab returns the pane to the picker. (b) `/sidechat [prompt]` typed in the source chat's composer opens the same tab: with a prompt, the sidechat's first turn starts immediately with it; bare `/sidechat` only opens the tab with an empty composer, exactly like the picker row. The composer intercepts it in `send()` (`crates/ui/src/composer.rs:5952`) before it becomes a `Run`; the text never reaches a harness. `/sidechat` is an app-level entry injected into the slash popup list (`SlashState`, `composer.rs:4026-4045`) for every harness, deduped by name against harness-advertised commands.
- Creation (engine, host-side): new chat in the same space, cwd, worktree, branch, harness, model, reasoning, sandbox and auto-approve as the source (decision d3: permissions inherit); title `Sidechat: <first 72 chars of prompt | source title>`; `sidechat_source_chat_id = source`. Creating a sidechat from a chat that already has `sidechat_source_chat_id` is refused (no nesting).
- Prompt wrapping (engine, at dispatch for a chat whose `sidechat_source_chat_id` is set): the doc's user entry is the typed text; the harness prompt is `<sidechat_boundary>` + (first turn only) `<sidechat_context>` bounded text copy of the source doc + `<latest_user_message>`. The copy walks the source doc's entries newest-first taking user/assistant text parts until a char budget is spent; tool outputs are not in the doc (`crates/doc/src/parts.rs:364-375`) and are not copied. Boundary text ported from astra `sidechatProviderInput.ts:3-4`. Later turns resume the sidechat's own harness session as any chat does (`sessions.rs:444-455`). Wrapping applies wherever the engine hands a prompt to the harness: the cold dispatch, the warm steering mailbox (`sessions.rs:375-396`), and messages released from the held queue (`QUEUE_MESSAGE` + `holdForTurnEnd`, `composer.rs:6490-6506`). "First turn" means the sidechat doc has no assistant entry with `status: complete`; an aborted or errored first turn re-sends the copy.
- The `/sidechat` interception in `send()` runs before the busy-chat branch, so a sidechat can be opened while the source chat is streaming; the source run is not interrupted.
- UI: new `RightSurface::Sidechat(u64)` tab in the right pane (`crates/ui/src/shell.rs:466-478`), built like `SubagentTab` (`shell.rs:1255-1263`, `:3040-3088`: `Transcript::for_doc` over the sidechat's doc id) plus a `Composer` bound to the sidechat chat id. Sidebar lists the sidechat nested under its source (decision d2). When the pane is closed the source composer shows a pill for the most recent sidechat that reopens it (astra `31a1c3213`); the sidechat also opens from the sidebar like any chat.
- Model picker on the sidechat changes only the sidechat.

## Scope

- `crates/proto`: `Chat.sidechat_source_chat_id`.
- `crates/engine`: create-sidechat method (RPC verb for the UI), nesting refusal, prompt wrapping at dispatch, first-turn detection (sidechat doc has no assistant entry yet).
- `crates/ui`: composer interception + app-level slash entry, `RightSurface::Sidechat`, sidechat tab with composer, sidebar nesting, reopen pill.
- `edge/`, `apps/ios`: no change in this slice; they receive the new field and show the sidechat as a flat chat. Nested rendering on iOS is a follow-up work item.
- Out of scope, recorded on d1: native session fork (Claude `--fork-session`, Codex `thread/fork`, OpenCode `session.fork`) as a later slice.

## Anti-goals

Each anti-goal gets a matching VERIFY.md check.

1. Sidechat activity never appends to, edits, or reorders the source chat's doc.
2. Sidechat runs never overwrite the source chat's `harness_session_id` / `harness_session_cwd` or its `contextUsage` meta.
3. The literal `/sidechat ...` text never reaches any harness as a prompt.
4. No harness driver gains a fork call in this slice.
5. A sidechat cannot spawn a sidechat.
6. No new doc schema field: the sidechat doc is a plain `SessionDoc`; only the workspace `Chat` row changes.

## Decisions

d1, d2, d3

## Derived artifacts

None; no decision above rests on a derived table or measurement.

## Red tests

- engine: dispatch on a chat with `sidechat_source_chat_id` writes the typed text as the user entry and sends the wrapped prompt (boundary + bounded copy) to the harness on the first turn; second turn sends boundary + latest message only, no copy.
- engine: bounded copy respects the char budget and drops tool parts; empty source yields no `<sidechat_context>` block.
- engine: create-sidechat from a sidechat returns an error; create-sidechat copies harness/model/sandbox/auto-approve/cwd from the source.
- engine: after a sidechat turn, the source chat's doc entry count, `harness_session_id` and `contextUsage` are unchanged.
- ui (`crates/ui/src/composer.rs` tests): `/sidechat compare` is intercepted in `send()` and never builds a `Run`; the popup lists `/sidechat` for a harness that advertises no commands and exactly once for one that advertises a same-named command.
- ui: sidebar places a sidechat under its source; a sidechat whose source is missing renders flat, not hidden.
