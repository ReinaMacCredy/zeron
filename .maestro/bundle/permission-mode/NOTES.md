# NOTES — permission-mode

Overwritten handoff, never appended; history lives in trace and decisions.

Base: 4732b13f (main)

## Current State

Work:
- w19 [open] Permission mode chip: Ask / Approve for me / Full access shared by every harness, per-chat, next to the context ring
  evidence: none recorded

## Next Action

Driver: Claude Code (Fable 5.1), worktree `.maestro/worktree/permission-mode`,
branch `feat/permission-mode`, commit 581c4208 on top of base 4732b13f,
own `CARGO_TARGET_DIR=<worktree>/target`. Dev bundle: `target/macos-dev7/Zeron Dev 7.app`
(port 49783, ZERON_HARNESS=mock, chats `chat-pm` mock and `chat-claude-ask` claude-code/ask).

Reina still has to run the live checks the Little Snitch modal blocked (VERIFY rows 5b, 7, 9):
1. Dismiss or answer the Little Snitch prompt for the peer's Dev 6 opencode process (rule choice is hers).
2. In Zeron Dev 7, click the chip beside the ring, pick Ask; confirm the row persists after reopen.
3. On a Claude chat set to Ask, prompt a file write → an Allow/Deny question must appear; Deny must stop the tool.
4. On a Codex chat set to Approve for me, run a command outside the workspace → a yes/no question must appear
   (Auto uses `on-request`, which a user once reported as "asking at every step" — new behavior, watch it).
Then merge: main's `crates/engine/src/sessions.rs` carries peer WIP that will conflict on `RuntimeConfig`.

## Authority

Original instruction (2026-09-17): design discussion, then "build". Locked by Reina in chat: three
shared modes named after the ChatGPT picker, chip beside the context ring, default Full ("full như
hiện tại tôi muốn đổi thì tự tôi chọn"). Transferred: implement + verify on the worktree, local
commits on the feature branch. Retained by Reina: push, merge, Little Snitch rule choices, any
change to the default mode.

## Failed approaches

- Claude Full → `bypassPermissions`: wrong, zeron always ran `--permission-mode default` and
  auto-allowed in the handler; reverted to `default` so the unattended path is unchanged.
- Claude Auto → `--permission-mode auto`: the CLI downgrades it to `default` in print mode
  (init frame says `default`, Write asks). Auto maps to `acceptEdits`.
- CGEvent clicks on Dev 7: swallowed by the Little Snitch modal covering display 1.

## Do not repeat

- Do not edit harness sources while a `cargo build`/`cargo test` chain is running in the same
  target dir: cargo recompiles them mid-chain and fails on half-edits.
- Do not `cargo fmt --all`; format changed files through `rustfmt --edition 2024 < f`.
- `timeout` does not exist on this Mac; use `perl -e 'alarm shift; exec @ARGV' <secs> <cmd>`.
- Do not answer Little Snitch prompts; capture with `screencapture -l <winid>` and seed over RPC instead.

