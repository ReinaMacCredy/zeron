# NOTES — provider-switch

Overwritten handoff, never appended; history lives in trace and decisions.

Base: 1259573c (main)
Worktree: .maestro/worktree/provider-switch (branch feat/provider-switch, own CARGO_TARGET_DIR=<worktree>/target)
Mock: docs/research/provider-switch/demo.html (option 1 chosen by Reina 2026-09-18)

## Current State

Implemented option 1 (switch in place):
- proto: `AgentEvent::SessionTombstoned {}` journal-only record.
- engine: `SessionsEngine::switch_harness` (interrupt + tombstone) called from `Mutate setChatConfig`
  when the harness differs from the row; `Inner::harness_prompt` wraps the first run after a
  tombstone with `<provider_switch>` + `<previous_conversation>` (bounded_source_copy, 48k chars,
  system entries skipped, current user entry excluded); `anchors()` returns live/dead sets, fork
  refuses a dead reply, rewind onto a dead reply tombstones instead of branching.
- ui: harness rail unlocked on existing chats; `viewed_harness` browses a tab without committing;
  foreign pick -> `pick_foreign_model` -> dialog "Switch provider?" -> `setChatConfig` with
  harness+model+note "Provider changed from A to B."; empty chat switches without the dialog.
- tests: crates/engine/tests/provider_switch.rs (3), pickers gpui test + copy test.

## Next Action

MERGED to local main at af1ce76c (2026-09-18, --no-ff; 18889b9f feature + 87343596 chip fix). NOT pushed. Dev bundle 'Zeron Dev PS' (worktree target/macos-dev-ps, port 49900) still has the switched chat for a live check. Follow-ups flagged, not built: tag harness_session_id with its harness; enforce the lock engine-side for RunRequest.harness.

## Authority

Transferred: build option 1 as mocked (Reina "1", 2026-09-18).

Retained: push; any change to harness_session_id tagging or engine-side lock enforcement
(flagged follow-ups, not in this slice).

## Failed approaches

- Tagging the divider system entry with a kind: system entries are text-only parts; the journal
  record (`SessionTombstoned`) is the discriminator instead.

## Do not repeat

- Do not derive "needs replay" from `resume == None`: forks and cwd moves also start fresh and must
  not get a text handover. Only the explicit empty-string tombstone triggers it.
- A parked child's Done re-remembers the old session id: `switch_harness` must interrupt before it
  tombstones (same reason rewind does).
