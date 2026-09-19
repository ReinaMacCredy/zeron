# SPEC — daily-review-fixes

## Problem
User requested `fix` for eight daily-review findings, then `resume`. Base: 2c68a1ce.

## Solution
Preserve existing behavior and wire/disk formats while correcting bulk-close persistence, moving draft disk work off the app thread, using typed compaction capabilities, indexing tab rendering, respecting usage cache/in-flight work, caching branch misses and sidechat context with invalidation, and consolidating proven lifecycle/draft duplication.

## Scope
crates/ui draft_store/composer/state/pickers/shell/lib, crates/engine sessions/agent_accounts/rpc, crates/doc transcript metadata reads, shared RPC DTOs and focused regression tests. Only edit portions necessary for the supplied findings.

## Anti-goals
No upstream merge, no version bump, no changes to existing research demos, no push/release, no provider network probes against real credentials in tests.

## Decisions
Use source-confirmed contracts; keep persistence version and RPC method names/JSON representations compatible.

## Red tests
Bulk close must preserve hidden idle IDs. Draft writes must retain latest snapshot on quit/failure and never overlap out of order. Cache invalidation must preserve fork/rewind and sidechat context semantics. Reuse existing lifecycle/queue/compact/draft suites.
