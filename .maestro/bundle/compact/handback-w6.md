# Handback — w6 compact core

## Status

DONE

## Claim

The engine has `SessionsEngine::compact(chat_id, focus)` plus `CompactChat` and
`HarnessCompaction` RPCs. On an idle chat it retires the parked warm child,
resumes the chat's own harness session with the harness's compaction command,
appends exactly one `MessageRole::System` entry
(`Context compacted · 22.8k → 2.3k tokens`), and resets `contextUsage` to the
post-compaction occupancy keeping the last known window. No user entry, no
assistant entry, nothing journaled. A second compaction, or one on a busy chat,
is refused. The claude normalizer turns `system`/`compact_boundary` into
`AgentEvent::Compacted`; nothing else in the captured compaction flow (the
second `init`, the two summary-carrying `user` frames) produces an event.

## Proof

```
$ cargo test -p zeron-harness --lib claude::normalize
test result: ok. 20 passed; 0 failed; 0 ignored; 131 filtered out

$ cargo test -p zeron-harness --test claude
test result: ok. 10 passed; 0 failed; 2 ignored; 0 filtered out

$ cargo test -p zeron-doc
test result: ok. 106 passed; 0 failed; 0 ignored     (lib)
test result: ok. 1 passed; 0 failed; 0 ignored       (integration)

$ cargo test -p zeron-engine --test compact
running 6 tests
test a_compaction_run_without_a_boundary_writes_nothing ... ok
test compaction_appends_one_system_entry_and_resets_the_meter ... ok
test focus_rides_the_command_text ... ok
test compaction_retires_a_parked_warm_child_and_resumes_its_session ... ok
test a_second_compaction_is_refused_while_one_is_pending ... ok
test compaction_is_refused_while_a_run_is_live ... ok
test result: ok. 6 passed; 0 failed; 0 ignored; 0 filtered out

$ cargo check --workspace --tests
Finished `dev` profile — 0 errors (pre-existing warnings only, in
preview/discovery.rs, sync, ui, engine/tests/restart_resume.rs)
```

Falsification: deleting `self.interrupt(chat_id)` from `compact` makes
`compaction_retires_a_parked_warm_child_and_resumes_its_session` fail with
`Other("Chat is busy")`; restored, all six pass.

## Files changed

- `crates/proto/src/agent.rs` — `AgentEvent::Compacted { pre_tokens, post_tokens }`; `Compaction { Native, SlashText(String), None }`.
- `crates/harness/src/lib.rs` — `Harness::compaction()` (default `None`) and `Harness::compact()` (default `Err(Protocol("compaction is not supported by this harness"))`).
- `crates/harness/src/claude/wire.rs` — `SystemFrame.compact_metadata` + `CompactMetadata { pre_tokens, post_tokens }`.
- `crates/harness/src/claude/normalize.rs` — `compact_boundary` → `Compacted`, ahead of the `init` gate; two unit tests fed by the captured frame lines.
- `crates/harness/src/claude/mod.rs` — `compaction()` returns `SlashText("/compact")`.
- `crates/doc/src/parts.rs` — `Compacted` among the not-persisted events.
- `crates/engine/src/doc_host.rs` — `ChatDocHandle::write_system_message`.
- `crates/engine/src/sessions.rs` — `CompactOutcome`, `compact`, `settle_compaction`, `compaction_request`, the `drain_compaction` sink, `compaction_note`/`format_tokens`, `Inner.compacting` + `Inner.compaction_timed_out`, and the completed-turn clear of the timeout marker.
- `crates/engine/src/rpc.rs` — `CompactChatParams`, the `COMPACT_CHAT` and `HARNESS_COMPACTION` handlers, both added to `forwardable`, 180s forward deadline for `COMPACT_CHAT`.
- `crates/rpc/src/lib.rs` — the two method constants.
- `crates/engine/tests/compact.rs` — new; six tests over a local `/compact`-capable `Harness` whose steerability and compaction run the test controls.

Not committed.

## Assumptions not verified

- A direct `dispatch` against a compacting chat interrupts the compaction and
  replaces it — existing "never two runtimes per chat" semantics. Not guarded
  in `dispatch_inner`; the UI never does it because the chat reads busy
  (`Composer::run_live` → `indicator_for` → session status, which the
  compaction holds at `Working` with a 15s `touch_session` heartbeat).
- `Compaction::Native` routing is wired and compiles but has no driver behind
  it (w7), so it is untested end to end.
- The `/compact` prompt shape was verified only against the sanitized frame
  capture in this bundle, not live from inside zeron.
- The compaction request falls back to the durable chat row (`cwd` + `config`)
  when `last_requests` is empty after an engine restart. Coded, not tested.

## Residual risks

- A compaction outliving the 120s timeout leaves the harness still compacting
  behind our back. The chat is marked so a retry is refused until its next
  completed turn, but that run's `Compacted` never lands, so the meter keeps
  the pre-compaction reading until the next `ContextUsage`.
- `MessageRole::System` still has no renderer on desktop, edge or iOS; the
  boundary entry draws as an assistant-styled row today. w8 / slice 2.
- Inherited from `dispatch_inner`, not introduced here: (a) drive_run's exit
  does `remove_run` then `set_status(Idle)` as two lock acquisitions and
  `interrupt()` polls `is_live` between them, so a compaction's
  `set_status(Working)` can land first and be flipped; (b) a dispatch landing
  between the `turn_in_flight` check and `interrupt()` gets its fresh turn
  killed. Both windows are microseconds.
- The drain breaks immediately on cancel/timeout rather than draining to `Done`
  the way drive_run does. Safe for claude: `run_session` is a spawned task that
  owns the child and reaps it on either `interrupt.cancelled()` or
  `event_tx.closed()` (`claude/mod.rs:487, :726, :743, :772`), so dropping the
  stream cannot orphan a process. Re-check this for any future driver whose
  teardown is stream-owned.

## Incidental findings

- `rustfmt <file>` follows `mod` declarations: formatting `harness/src/lib.rs`
  also reformatted `harness/src/acp/subagent.rs`. That file was restored from
  HEAD and the four spots in `claude/normalize.rs` that are not rustfmt-clean
  at HEAD were reverted, so the diff holds only intended lines.
- `crates/harness/src/claude/normalize.rs` is not rustfmt-clean at HEAD (4
  spots). Worth knowing before anyone runs a formatter over it.
- Capability is a new `HarnessCompaction` method rather than a reshaped
  `ListCommands`, whose bare-array result the composer already reads.
- Two untracked dirs appeared during the session that are not mine:
  `docs/research/chat-background-demo/`, `docs/research/fork-rewind/`.
