# VERIFY — compact

Scenarios point at work acceptance/claims; do not restate them.

| # | Scenario | Work / acceptance | Result |
|---|----------|-------------------|--------|
| 1 | Claude chat at >20k tokens, type `/compact`: a `Context compacted · pre → post` divider appears, meter drops, next turn continues coherently | w5 acceptance: Claude | |
| 2 | Same on Codex: `thread/compact/start` observed in the driver log, divider appears, meter drops | w5 acceptance: Codex | |
| 3 | Same on OpenCode via `session.summarize` | w5 acceptance: OpenCode | |
| 4 | Cursor chat: `/compact` listed but disabled with "Cursor does not support compaction"; meter button likewise | w5 acceptance: unsupported harnesses show why; d8 | |
| 5 | Dump doc entries before/after compaction: all prior entries byte-identical, exactly one new `system` entry, no `user` entry containing `/compact` | anti-goals 1, 3 | |
| 6 | `rg -n 'run_title|TITLE_INSTRUCTIONS' crates/engine/src/compact*` (or the compact module) returns nothing; no new model call site outside the harness drivers | anti-goal 2 | |
| 7 | Trigger `/compact` twice quickly and once while a turn is streaming: second and third are refused with a notice; one divider in the end | anti-goal 4, red test engine | |
| 8 | Claude with two short turns: `/compact` ends with "Not enough messages to compact." as a notice, no divider, meter unchanged | SPEC Solution, no-Compacted path | |
| 9 | OpenCode: confirm whether `session.summarize` returns before the summary lands (probe the live server; if asynchronous, the driver must await the session event that t3code awaits) | SPEC Solution, unverified item | |
| 10 | `Chat` row before/after: `harness_session_cwd` unchanged; `harness_session_id` unchanged or equal to the id the harness emitted during the compaction run | anti-goal 5 | |
| 11 | `git diff --stat crates/ui/src/settings crates/engine/src/settings*` empty; no `autoCompact` string anywhere new | anti-goal 6 | |
| 12 | OpenCode: an advertised non-compact command (e.g. `/init`) still routes through `/session/{id}/command` | anti-goal 7 | |
| 13 | Meter hover card shows the `Compact context` button; clicking it behaves as scenario 1 | d8 | |
| 14 | Edge tail render and iOS transcript show the system entry without crashing (iOS may style it as assistant until slice 2) | SPEC Scope, edge/iOS | |
| 15 | `cargo test -p zeron-harness -p zeron-engine -p zeron-doc -p zeron-ui` green incl. red tests; `edge` test suite green | Definition of Done | PASS 2026-09-17 at d355e202: engine compact 6/6, sidechat 6/6, doc 107, harness codex compact 3/3 + opencode 17 + acp 29, ui lib 1035 (--test-threads=1), edge vitest 43+13 + tsc. Remaining failures are environmental (see w5 note). |
| 16 | Unit-level coverage of scenarios 5, 7, 8, 12 | red tests | PASS: engine tests `compaction_appends_one_system_entry_and_resets_the_meter`, `a_second_compaction_is_refused_while_one_is_pending`, `compaction_is_refused_while_a_run_is_live`, `a_compaction_run_without_a_boundary_writes_nothing`; composer tests `compact_is_intercepted_with_its_focus…`, `submitting_compact_takes_the_rpc_path_not_the_run_path`, `the_compact_entry_never_doubles…`; opencode `/command` routing tests untouched and green. Live scenarios 1-4, 9, 13, 14 remain for the owner's manual pass in Zeron Dev. |
