# NOTES — sidechat

Overwritten handoff, never appended; history lives in trace and decisions.

Base: b51493a4 (main)

## Current State

BUILT 2026-09-17 on feat/sidechat-compact: dd4c3d7d (w9 engine), d355e202 (w10 UI). Zeron Dev relaunched from d355e202. Unit verification PASS (VERIFY 11); live scenarios 1, 2, 9, 10, 12, 13, 14 await the owner's manual pass. Deviation: the tab composer hides the model/harness pickers and the queue panel (w4 note), so VERIFY 10 is not reachable yet. Not pushed. Handbacks: handback-w9.md (reconstructed), handback-w10.md.

Design locked on 2026-09-17 (d1 root model, d2 sidebar nesting, d3 inherited permissions). Recon report: `.maestro/recon-sidechat-compact.md`. Reference implementation: `~/Code/astra` (`docs/user/sidechat.md`, `apps/server/src/orchestration/sidechatProviderInput.ts`, `apps/web/src/sidechat/`).

## Next Action

Owner runs the live VERIFY scenarios in Zeron Dev (picker row, bare `/sidechat`, `/sidechat <prompt>`, close → pill → reopen, sidebar nesting, source untouched). Then maestro-verify closes the bundle. Follow-ups: per-sidechat model picker (pickers rewire), iOS nested rendering, optimistic echo in the tab transcript, native session fork (d1 deferred).

## Authority

Transferred: none. The owner asked for a design ("design 2 cái đó với tôi"); implementation was not requested.

Retained: implementation start, any push.

## Failed approaches

None yet.

## Do not repeat

- Do not ask the owner whether t3code has a sidechat: it does not (0 hits, 0 commits). Astra is the reference.
- Do not reopen the /btw-overlay vs separate-chat fork; d1 settles it.
