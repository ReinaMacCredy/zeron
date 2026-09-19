# Handback — w7 compact drivers

## Status

DONE

## Claim

Codex and OpenCode report `Compaction::Native` and implement `Harness::compact`.
ACP reports `SlashText("/compact")` only when its cached advertised command list
contains `compact`, else `Compaction::None`. Cursor is untouched and inherits the
trait default `None`. Each path has a fixture-driven test.

- **Codex**: refuses an empty `request.resume` with `Protocol("nothing to
  compact: …")`; otherwise resumes the thread (no fresh-start fallback) and
  issues `thread/compact/start { threadId }`. `thread/compacted` OR an
  `item/completed` whose item type contains "compact" is the boundary;
  `pre_tokens` is the last `thread/tokenUsage/updated` before it, `post_tokens`
  the first within a 2s grace after. Emits `Compacted` then `Done { Completed,
  session_id }`. `error`, `turn/failed`, `turn/aborted` and child EOF →
  `Done { Errored }`; interrupt → `Done { Interrupted }` with no `Compacted`.
  Foreign-`threadId` notifications are skipped. `run_session` gained an explicit
  `"thread/compacted" => {}` arm; child routing (`normalize.rs:696-700`) unchanged.
- **OpenCode**: session id from `request.resume`, provider/model from
  `request.model.split_once('/')`, both refused when absent; refused when
  `session_running` answers a definite `true`. POSTs `{providerID, modelID,
  auto: false}` to the summarize route with no per-call timeout, bounded by 600s
  and the interrupt, then `Compacted { None, None }` + `Done`.
- **ACP**: reads the `commands` `OnceCell` (filled from `initialize` /
  `available_commands_update`) without blocking.

## Proof

```
$ cargo test -p zeron-harness --test codex
test result: FAILED. 23 passed; 1 failed; 4 ignored
failures: models_discovers_visible_catalog_with_pagination
$ cargo test -p zeron-harness --test codex compact
test result: ok. 3 passed; 0 failed; 25 filtered out

$ cargo test -p zeron-harness --test opencode
test result: ok. 17 passed; 0 failed; 0 ignored

$ cargo test -p zeron-harness --test acp
test result: ok. 29 passed; 0 failed; 0 ignored

$ cargo test -p zeron-harness --lib
test result: FAILED. 150 passed; 1 failed
failures: shell_env::unix::tests::falls_back_when_interactive_attempt_hangs

$ cargo check --workspace --tests   # exit=0, warnings only (all pre-existing)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.50s
```

Both failures are **pre-existing and environmental**, reproduced at the base
commit with `crates/harness` stashed. New tests: three per native driver
(boundary+Done, refusal on a missing session/model, interrupt or busy-turn),
one for acp.

## Files changed

Under `/Users/reinamaccredy/Code/zeron-fork/crates/harness/`:
`src/{codex,opencode,acp}/mod.rs`, `tests/{codex,opencode,acp}.rs`,
`tests/fixtures/fake-codex.sh`, `tests/fixtures/fake-acp-nocompact.sh` (new).

One refactor was required: the child-spawn/stdio/`RpcClient` block was extracted
from `CodexHarness::run_with_mode` into `CodexHarness::spawn_app_server` + a
private `AppServer` struct so `compact` reuses it instead of copying ~50 lines.
No behaviour change; every existing codex test still passes. `rustfmt --edition
2024` pulled `src/acp/subagent.rs` in through `mod`; restored byte-for-byte from
HEAD, not in the change set. Nothing was committed; `crates/ui/` (dirty from the
concurrent agent) was not touched.

## Assumptions not verified

1. **OpenCode V2 summarize path is inferred.** Source:
   `/Users/reinamaccredy/Code/astra/node_modules/.pnpm/@opencode-ai+sdk@1.15.13/node_modules/@opencode-ai/sdk/dist/gen/types.gen.d.ts:2175`
   (`/session/{id}/summarize`, body `{providerID, modelID}`) and
   `dist/v2/gen/types.gen.d.ts:5650` (`/session/{sessionID}/summarize`, body
   `{providerID, modelID, auto?}`). Neither is opencode **2.x** — what zeron
   calls `Protocol::V2`, which prefixes `/api`. `/api/session/{id}/summarize`
   mirrors the driver's own V1→V2 convention but is unverified live; no 2.x SDK
   exists on this machine (installed binary is 1.18.18 = V1).
2. `thread/compact/start` is a **request**, answered independently of
   `thread/compacted` — from t3code `CodexSessionRuntime.ts:2325`. Not probed live.
3. The 2s post-boundary token grace is a guess; a slower real codex yields
   `post_tokens: None` and the plain "Context compacted" note. Degraded, not wrong.
4. `SUMMARIZE_TIMEOUT` (600s, t3code parity) is unreachable in practice — the
   engine's `COMPACT_TIMEOUT` (120s) fires first and cancels the interrupt token,
   which the driver's select already handles.
5. Codex compaction does not forward `serviceTier` on `thread/resume` (the run
   path does); the compaction pass runs on the account default tier.

## Residual risks

- **Deliberate deviation:** codex `compact()` does **not** fall back to
  `thread/start` when `thread/resume` fails. A fresh thread would compact nothing
  and `drain_compaction` would then `remember_harness_session` the new empty
  thread id, discarding the chat's real conversation. Resume failure surfaces as
  `Done { Errored }`.
- If a real codex wraps compaction in a turn and emits `turn/completed` **before**
  `thread/compacted`, the driver settles as "Nothing to compact" although the
  thread did compact. The fixture cannot express that ordering — VERIFY 8 live
  probe item.
- `compaction()` on ACP is cache-only by design, so a freshly restarted engine
  reports "does not support compaction" for an ACP agent until something warms
  `commands()`. Worth confirming against the UI slice's enable/disable logic.
- An interrupted opencode compaction drops the in-flight POST while the server
  keeps summarizing (t3code does the same), so the chat's meter can be stale
  afterwards.
- OpenCode's refusal probe uses `GET /session/status` (V1) / `/api/session/active`
  (V2); an unreachable route reads as "can't tell" and compaction proceeds, with
  the server's own 400 surfacing as `Done { Errored }`.

## Incidental findings

- `crates/harness/tests/codex.rs:630` hardcodes `/bin/false`, absent on this
  macOS (Darwin 27 ships only `/usr/bin/false`). Fails at HEAD.
- `crates/harness/src/shell_env.rs:330` unwraps a `None` in
  `falls_back_when_interactive_attempt_hangs`. Also fails at HEAD.
- Pre-existing `unused_assignments` warnings at `acp/mod.rs:2186` and `:2947`.
- All five ACP specs use `identity_transform` (`acp/mod.rs:121-123`, used at
  `:229/:304/:369/:425`), so an ACP `SlashText("/compact")` reaches the agent
  verbatim — no prefix corruption.
