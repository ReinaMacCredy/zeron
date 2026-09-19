# NOTES — compact

Overwritten handoff, never appended; history lives in trace and decisions.

Base: b51493a4 (main)

## Current State

BUILT 2026-09-17 on feat/sidechat-compact: 9e3a22d5 (w6 core), 560815a7 (w7 drivers), db91f520 (w8 UI + edge test). Zeron Dev relaunched from d355e202. Unit/fixture verification PASS (VERIFY 15-16); live scenarios 1-4, 9, 13, 14 await the owner's manual pass. Full suite failures are environmental (w5 note). Not pushed. Handbacks: handback-w6/w7/w8.md in this dir.

Design locked on 2026-09-17 (d5 System boundary entry, d6 t3code-shaped capability, d8 UX). Recon report: `.maestro/recon-sidechat-compact.md`. Reference implementation: `~/Code/t3code` (`apps/server/src/provider/Services/ProviderAdapter.ts:35-43`, `apps/server/src/provider/Layers/ProviderService.ts:1702-1830`, `ClaudeAdapter.ts:3417-3440`, `CodexSessionRuntime.ts:2323-2327`, `OpenCodeAdapter.ts:3430-3490`, `apps/web/src/components/chat/ContextWindowMeter.tsx:135-158`, `ChatView.tsx:661-2950`). Agent SDK doc for `/compact` and `compact_boundary`: https://code.claude.com/docs/en/agent-sdk/slash-commands

Unverified, to settle during implementation: OpenCode summarize synchronicity (VERIFY 9); Cursor `/compress` through the shim (out of scope, capability `None`).

## Next Action

Owner runs the live VERIFY scenarios in Zeron Dev (Claude chat `/compact`, Codex, OpenCode, Cursor disabled reason, meter button). Then maestro-verify closes the bundle. Open follow-ups: iOS system-row renderer (SPEC slice 2), OpenCode 2.x summarize route unverified live (VERIFY 9), Codex turn/completed-before-compacted ordering (w7 residual risk).

## Authority

Transferred: none. The owner asked for a design; implementation was not requested.

Retained: implementation start, any push.

## Failed approaches

None yet.

## Do not repeat

- d4 and d7 in the store duplicate d6 and d5 (another writer, same day). Cite d5/d6/d8 only; do not draft a third copy.

- Do not pass `/compact` text through as a plain Run on Codex or OpenCode; they ignore it and there is no completion signal (d6 rejected alternative).
- Do not add a collapse/hidden flag to the doc schema for this feature (d5 rejected alternative).
