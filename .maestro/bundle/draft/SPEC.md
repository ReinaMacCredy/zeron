# SPEC — draft

Original instruction (2026-09-17, Reina): "tôi muốn thảo luận với bạn để thêm
cái này từ bên t3 đó là draft" → four layers named in the discussion →
"y chang t3 là toàn bộ đúng chứ?" (confirmed: all four) → "okay" (open the
bundle, write SPEC, walk the decisions one at a time starting with the tab
strip). Design lane. Implementation is not yet requested.

Source of truth for "y chang T3": `~/Code/t3code` at 6c583620f (2026-09-09),
`apps/web/src/composerDraftStore.ts`, `apps/web/src/components/Sidebar.tsx`
(SidebarDraftBlock / SidebarDraftRow), `apps/web/src/routes/_chat.draft.$draftId.tsx`,
`apps/web/src/components/ChatView.logic.ts` (buildLocalDraftThread,
resolveBackgroundDraftWorkspaceOptions), `docs/user/thread-sidebar.md`
("Start in the background").

## Problem

Zeron keeps composer drafts only in memory (`crates/ui/src/composer.rs:4115`
`drafts: HashMap<String, String>`, plus `attachments` and `appshots` keyed the
same way; `""` = the single new-session canvas). They survive navigation and
die with the process. There is exactly one pre-session canvas: starting a
second new session overwrites the first one's text. Nothing in the sidebar
shows that a session has unsent text. Sending from the canvas is modal: the
canvas becomes the new chat and the user waits for it.

T3 has four stacked behaviors that Zeron lacks:

- **A. Persisted composer drafts.** Prompt, attachments, model/mode per thread
  key, written to device-local storage, restored after restart. Only entries
  with real content persist (`partializeComposerDraftStoreState`).
- **B. Unsent indicator.** A thread row whose composer holds unsent content
  shows an amber pen (`useThreadHasUnsentDraft`, `draftPenClassName`).
- **C. Draft sessions.** N pre-thread drafts at once, each with its own
  `DraftId`, pre-minted `threadId`, own route `/draft/$draftId`, and its own
  mutable routing config (project/environment, branch, worktree path, env
  mode, start-from-origin) that can change until the first send. A draft that
  has user content (`composerDraftHasUserContent`: text or any attachment;
  model/mode alone does not count) gets a sidebar row above the pinned block:
  project name, prompt preview, discard button, amber surface. The open
  draft's row is frozen at the moment it became the route (no per-keystroke
  repaint); a draft never navigated away from shows no row. On send the draft
  is promoted (`promotedTo`), the route is replaced by the real thread's
  route after the hero transition, the row disappears.
- **D. Cmd+Enter background submit.** Submit the draft as a new thread without
  navigating to it and immediately mint another draft that keeps the workspace
  mode and base branch (`resolveBackgroundDraftWorkspaceOptions`); with
  worktree mode every background submission creates its own worktree.

## Solution

Port all four, mapped onto Zeron's model:

| T3 | Zeron |
|---|---|
| project + environment | space (device, folder) — the canvas space picker already exists (`Pickers::render_new_thread_target_selectors`) |
| `/draft/$draftId` route | a draft identity alongside `NavEntry::Chat(String)`; today `""` is the only pre-session route (`shell.rs:552`, panel key `space-canvas:{space}` at `shell.rs:2316`) |
| draft store persisted in browser localStorage | device-local file beside `open_tabs` (`crates/ui/src/settings.rs:584` `UiSettings`), never the Loro doc — decision pending |
| pinned / active / settled sections, draft rows above pinned | one attention-sorted session list; draft rows at the top of it |
| no tab strip | tab strip + Ctrl+Tab switcher must each decide whether a draft appears — decision pending |
| pre-minted `threadId` on the draft | Zeron already mints the uuid at send (`shell.rs:3157`); C moves the mint to draft creation |
| `ComposerThreadDraftState` (prompt, images, files, persistedAttachments, terminalContexts, elementContexts, previewAnnotations, reviewComments, model, runtimeMode, interactionMode) | prompt, staged attachments, appshots (the composer entry) plus space, device, ref and checkout kind (the draft session). **Deviation as built:** harness, model, reasoning and permission-mode picks stay per canvas visit (the pickers reset them on every draft switch) and are not persisted per draft; T3 persists `modelSelectionByProvider` per draft. Follow-up if wanted. |
| files not finished uploading persist as metadata-only "needs reattach" markers | same: attachment refs whose upload is not committed persist as a marker row |

Layering: A is the storage substrate; B reads A; C introduces the draft
identity and sidebar rows and rides on A; D rides on C.

## Scope

- Slice 1 (A + B): persist the composer draft map to disk on change
  (debounced, like `ui-settings.json`); restore on boot; prune entries whose
  chat no longer exists; amber pen on sidebar rows with unsent content.
- Slice 2 (C): draft identity in the route model, draft rows in the sidebar,
  per-draft routing config, promotion on send, discard.
- Slice 3 (D): Cmd+Enter (mod-enter) on a draft submits in the background
  and mints the next draft with the same space, branch and worktree mode.
- Worktree `.maestro/worktree/draft`, branch `feat/draft`, base 8b6b7ab6.
  All three slices landed in one commit (the same files carry all three).

## Anti-goals

- Drafts entering the Loro-synced session doc or following the user across
  devices (VERIFY: the synced store's schema is unchanged; a draft never
  appears on a second device). Matches T3, where drafts are client-local.
- Model/mode-only drafts producing a sidebar row or a persisted entry
  (VERIFY: a draft with only a model pick has no row and is not on disk).
- The open draft's sidebar row repainting per keystroke (VERIFY: typing in
  the open draft does not change its row text; the row updates on the next
  navigation away).
- Sending from a draft changing the `Mutate createChat` / `Run` wire shape
  (VERIFY: a promoted draft's first turn produces the same RPC payloads as
  today's canvas send).
- Background submit creating a tab or stealing selection (VERIFY: after
  Cmd+Enter the selected route is a fresh draft; the new session is in the
  sidebar, tab presence per decision 1).
- Discarding a draft touching the engine (VERIFY: discard emits no RPC).

## Decisions

Locked 2026-09-17 (`maestro bundle show draft` renders each):

- d18 — tab strip yes, Ctrl+Tab switcher yes (Reina's pick; the two
  alternatives were tab-only and sidebar-row-only).
- d19 — own device-local `composer-drafts.json`, versioned, 300ms debounce
  ("như bên t3": T3 uses its own localStorage key, separate from settings).
- d20 — every canvas is a draft from birth; empty drafts get no row and no
  disk entry (T3 parity).
- d21 — draft rows at the top of the session list, space-filtered, open
  row frozen (T3 parity mapped onto the single attention-sorted list).
- d22 — persist prompt + committed attachment refs + appshot paths;
  uncommitted uploads persist as needs-reattach markers (T3 parity).
  As built: Zeron uploads only at send, so there is no pre-send ref and no
  "uncommitted upload" state; image bytes are written once under
  `{data_dir}/composer-drafts/<id>.<ext>` and the JSON carries the path.
  A missing file drops that attachment with a warning, never the draft.
- d23 — Mod+Enter on a draft starts the session in the background only
  under the Enter send behavior; under Mod+Enter the chord stays the plain
  send and no extra chord is added (the message-enter table's test forbids
  extra modifier bindings).

## Derived artifacts

`docs/research/draft/demo.html` — interactive mockup of the draft tab,
draft rows (frozen open row, discard), the unsent pen, the Ctrl+Tab card,
Cmd+Enter background submit, and a simulated restart for persistence.
Hand-written, no derivation.

`docs/research/draft/capture-*.png` — Zeron Dev 8 (isolated bundle,
`ZERON_HARNESS=mock`) captures of the built feature: restored rows + pen
after a restart, the switcher with draft cards, and the background start.
Produced with the capture knobs `ZERON_DRAFT_TEXT=<text>` (types into the
boot draft) and `ZERON_OPEN_DIALOG=background-send` (fires Mod+Enter 4s
later), added because synthetic CGEvent keys do not reach gpui on this rig.

## Red tests

- settings: composer-drafts file round-trips prompt + attachment refs; an
  entry with empty prompt and no attachments is not written; an entry whose
  chat id is gone is pruned on load (slice 1).
- shell: `sync_open_tabs` behavior for a draft id per decision 1 (slice 2).
- composer: draft swap between two draft ids keeps both texts; promotion
  moves the draft entry to the new chat id (slice 2).
- shell: Cmd+Enter from a draft leaves the route on a new draft carrying the
  previous branch and worktree mode (slice 3).
