# NOTES — fork-rewind

Overwritten handoff, never appended; history lives in trace and decisions.

Base: 08c0cc06 (feat/sidechat-compact tip at merge time; design base was db91f520)
Driver: Claude Code

## Current State

MERGED 2026-09-17: fork/rewind commit 3247bbf2 (rebased onto d6a88754, the peer's 1M-context change committed from the shared checkout) is the tip of both `feat/sidechat-compact` and `main`. Worktrees chat-background, ui-polish, fork-rewind removed and their branches deleted (all merged). Dev bundles 1-5 quit. Not pushed. A release build + `scripts/package-macos.sh` (Apple Development identity) is being installed over `/Applications/Zeron.app` (previous 0.2.71 upstream-signed bundle kept in ~/.Trash).

Work:
- w15 done; evidence in VERIFY.md rows 1-12, live Claude probe `/tmp/fork-rewind-probe2.log` PASS.

Decisions: d9, d10, d11, d12 (locked). d9's "pending harness_fork on the chat row" wording predates d11; the built shape is the `AgentEvent::BranchPending` journal record plus an in-memory map.

## Next Action

Owner-side: allow or deny Little Snitch for Dev 5 and click through fork/rewind live (VERIFY rows 2, 7, 10 are engine/unit-proven, not clicked). Then push `feat/sidechat-compact` when wanted.

Follow-ups (not started, each a new work item): OpenCode branching (`POST /session/{id}/fork`, message-id anchors from the driver's `message.updated` feed); fork into a new worktree (blocked by the cwd-gated resume, d10); code restore via `--rewind-files` (the env var is already set on every Claude run); a `ListBranchPoints` RPC to dim actions per entry instead of refusing at request time (d11).

## Authority

Transferred: none.

Retained by the owner: push; any rule choice in Little Snitch for the dev bundle. Original instruction: "Duyệt build đi" (2026-09-17, after the HTML demo at docs/research/fork-rewind/demo.html) and "khi nào xong merge nha" (merge on completion). The merge grant stands; the fast-forward is blocked only by the peer's dirty main checkout (see Current State). Push was never granted.

## Failed approaches

- Sharing the main checkout's `target/` from the worktree: a base-check `cargo test` run in the main checkout rebuilt `zeron_proto` there and the worktree's engine then compiled against it (variants "not found"). Fixed by a worktree-private `CARGO_TARGET_DIR`; recorded in the Claude auto-memory gotchas.
- First live probe showed `prompt_anchor=None`: the driver sends prompts as a plain string and the `--replay-user-messages` echo keeps that shape, so the "has a text block" filter saw nothing. Fixed in the normalizer (string content counts), unit-tested, probe re-run PASS with `--resume-drops-turn` set.

## Do not repeat

- Do not run `cargo fmt --all` or `rustfmt` on shell.rs/transcript.rs directly (their `mod` children are HEAD-dirty); format those through stdin.
- Do not attribute these four failures to feature work; they fail at base on this Mac: codex `models_discovers_visible_catalog_with_pagination`, harness `shell_env…falls_back_when_interactive_attempt_hangs`, e2e `generated_image…`, local_first `online_runtime_shutdown…` (running Zeron apps probe the test's edge port).
- Do not drive the dev bundle through System Events by app name (every instance is process `zeron`); use `/tmp/cgtools/cg` (CGEvent move/click/type/key) after `open`-ing the bundle, and leave Little Snitch rule choices to the owner.
