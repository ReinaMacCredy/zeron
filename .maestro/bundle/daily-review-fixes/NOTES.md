# NOTES — daily-review-fixes

Overwritten handoff, never appended; history lives in trace and decisions.

Base: 2c68a1ce (main)

## Current State

Work:
- w40 [done] Fix daily review findings in drafts, tabs, capabilities and session caches
  evidence: local commit 3f5baaa7; VERIFY.md contains source, artifact and native draft journey evidence

## Next Action

None within authorized fix scope. No push or production installation performed.

## Authority

Transferred: completed local fix and scoped commit under user fix/resume request and AGENTS commit instruction.
Retained: no remote delivery authority; no production install requested.

## Failed approaches

- w40: failed: Full UI suite under default concurrent runner aborted SIGABRT without assertion; same built suite and current-source serial suite each passed 1068 tests. Repaired equivalent check uses --test-threads=1; no test skipped or weakened.
- w40: failed: Fresh-context reviews identified three introduced regressions: rewind branch cleared during session recovery, explicit usage Refresh served cache, and initial-canvas carry lost destination-only attachments. Adding focused repros and repairing before acceptance; other reviewed paths had no findings.

## Do not repeat

Do not claim parallel UI runner reliability or live new-engine acceptance from serial tests/native UI smoke. Preserve existing apps and unrelated research files.


Close-out 2026-09-17: commit 3f5baaa7 verified by 1452 tests passing, 1 existing ignored, clippy/build/diff check, three independent re-reviews and native draft journey with canonical readback. See VERIFY.md for artifact hashes and limitations. Rollback by reverting scoped commit. No pending delivery action in scope.
