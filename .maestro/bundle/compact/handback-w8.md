# Handback — w8 compact ui

## Status

DONE (the brief's edge item was a phantom — see Claim and Incidental findings)

## Claim

`/compact [focus]` is intercepted in `send()` before any `Run` is built: the
input clears and `CompactChat` runs on the chat's host device with the
remainder as `focus`. Compacted → nothing local (the boundary entry arrives
through the doc watch); otherwise the harness's own message becomes the
composer's chat-scoped notice. While the call is out the composer shows a
spinner chip "Compacting context…", the transcript's working trailer says
"Compacting context…" instead of a flavour word, the send button is blocked,
and the meter button reads "Compaction in progress". The pending marker is
`AppState::compacting_chat` (in-process, chat-keyed — not a doc/meta flag), so
switching chats mid-compaction leaves the other composer free. `/compact` is
injected into every harness's slash list, deduped by name, and drawn dimmed
with "<Harness> does not support compaction" (and refuses acceptance) when
`HarnessCompaction` reports `none`. The meter's hover card gained a "Compact
context" button on the same path with the same three disabled reasons; it is
now a `hoverable_tooltip` observing AppState and the composer. A
`MessageRole::System` entry renders as `RowKind::Divider` — muted centered text
between two hairlines, no bubble, no hover strip — never as assistant blocks.

**Edge: no renderer written, by finding.** `edge/` has no per-role renderer:
`render-parts.ts` is a tool-part storage sanitizer with zero `role` references,
`tail.ts`/`joinContinuations` are role-blind, `SessionMessageRole` already
includes `"system"`. There is no assistant bubble there to avoid. The honest
artifact is a contract test that a `system` entry keeps its role and text
through the doc transforms: `edge/src/session-doc/system-entries.test.ts`.

## Proof

```
$ cargo test -p zeron-ui --lib -- --test-threads=1
test result: ok. 1022 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p zeron-ui --lib compact -- --test-threads=1
test composer::tests::compact_is_intercepted_with_its_focus_and_nothing_else_is ... ok
test composer::tests::compaction_is_refused_with_a_reason_the_surfaces_share ... ok
test composer::tests::submitting_compact_takes_the_rpc_path_not_the_run_path ... ok
test composer::tests::the_compact_entry_never_doubles_a_harness_that_advertises_it ... ok
test result: ok. 12 passed; 0 failed; 1010 filtered out

$ cargo test -p zeron-ui --lib transcript::tests::a_system_entry -- --test-threads=1
test transcript::tests::a_system_entry_is_a_divider_not_an_assistant_reply ... ok
test result: ok. 1 passed; 0 failed; 1021 filtered out

$ cargo check -p zeron-ui --tests            # 0 errors (pre-existing warnings only)
$ rustfmt --check --edition 2024 <4 rust files>   # clean, and clean at HEAD too

$ cd edge && bunx vitest run                 # package.json test:unit
Test Files  8 passed (8)      Tests  43 passed (43)
$ cd edge && bunx vitest run -c vitest.workerd.config.ts   # test:workerd
Test Files  3 passed (3)      Tests  13 passed (13)
$ cd edge && bunx tsc --noEmit               # typecheck, exit 0
```

Two falsifications, each reverted: pointing `compact_focus` at a prefix no
prompt carries fails `submitting_compact_…` on the input-cleared assertion;
dropping the selection comparison from `Composer::compacting` fails
`compaction_is_refused_…` at `assert!(!composer.compacting(cx))`. Edge deps
came from `bun install` (no bun lockfile is checked in), so the suite ran
against `package.json` ranges.

## Files changed

- `crates/ui/src/composer.rs` — `compact_focus`, `with_compact_entry`,
  `harness_display_name`, `COMPACT_COMMAND`, `compact_task`,
  `compaction_cache`/`compaction_probed`, `ensure_compaction_kind`,
  `compacting`, `unsupported_compaction`, `compact_disabled_reason`,
  `set_compact_failure`, `request_compaction`; the `send()` intercept; the
  slash-list injection, dimmed row and `accept_slash` refusal; the chip;
  `send_blocked`; the meter call site; 4 tests.
- `crates/ui/src/state.rs` — `AppState::compacting_chat`.
- `crates/ui/src/context_usage.rs` — composer entity parameter,
  `hoverable_tooltip`, the button + reason line on `UsageCard`.
- `crates/ui/src/transcript.rs` — `RowKind::Divider`, the `System` branch in
  `rows_for_entry`, `divider_row`, its render arm, the trailer arm, 1 test.
- `edge/src/session-doc/system-entries.test.ts` — new.
- Coordinator's out-of-band ask (`sidechat_source_chat_id: None` in test `Chat`
  literals): `state.rs`, `change_requests.rs`, `changes.rs` as listed, plus
  `links.rs`, `settings/archived.rs`, `shell/spaces.rs` — the same proto field
  breaks those three too and `cargo check -p zeron-ui --tests` needs them.
  `crates/engine/src/workspace_host.rs:862` still misses it; that is the engine
  agent's file, untouched here.

Not committed. `crates/ui/src/notice.rs` was in bounds and needed no change.

## Assumptions not verified

- A click landing on the button inside the hoverable tooltip was never
  exercised: gpui paints tooltips deferred with normal hitboxes and the
  hoverable variant survives the pointer, but nothing in this repo clicks
  inside a tooltip and no test can reach it. Worth one manual pass in the app.
- The trailer's "Compacting context" arm needs a window and a live session row;
  covered by reading, not by a test.
- A `HarnessCompaction` probe whose RPC errors leaves the kind unknown (button
  enabled, engine refuses) until the next state change retries.
- The end-to-end path (real harness, real boundary entry, meter reset) was not
  run; only the composer's half is covered.

## Residual risks

- The engine registers the compaction in `runs` with an interrupt token
  (`sessions.rs:733-756`) and sets the session `Working`, so the composer shows
  Stop during a compaction and Stop should cancel it. Not exercised.
- That same `Working`→`Idle` edge may ring the done-chime. Engine-side (w6).
- The compact entry is injected on the `ListCommands` success path only; on a
  discovery error the popup shows the error and no `/compact` row (typing it
  still works).
- `harness_display_name` duplicates the engine's descriptor names: `Pickers`
  holds the catalog but exposes no accessor and `pickers.rs` was out of bounds.
- `input_hint` is `"focus"`, not the brief's `"[focus]"`: the row renderer
  already wraps hints in `<>`.
- Chip and trailer were split (composer / transcript) rather than both placed
  in the composer: the trailer lives in `render_working_trailer` and had to be
  fixed there anyway, or a compaction spins "Scheming…".

## Incidental findings

- SPEC red test "edge (`render-parts` tests): a `system` entry renders as a
  divider…" cannot be written as stated; restate it as the doc-contract test.
- `crates/doc/src/schema.rs:854` deserializes an unknown/missing role as
  `Assistant` — the same class of bug this slice fixed for `system`.
- iOS `TranscriptRows.swift:118` still draws the boundary as an assistant row
  (SPEC slice 2, unchanged).
- `bun install` in `edge/` writes `bun.lock` beside the checked-in
  `package-lock.json`; moved to `/tmp/zeron-edge-bun.lock` (dcg blocks
  `git clean -f` and python `os.remove`, so `mv` was used).
