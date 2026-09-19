# NOTES — draft

Overwritten handoff, never appended; history lives in trace and decisions.

Base: 8b6b7ab6 (main) → feature commit a2f47c0e on `feat/draft`, worktree
`.maestro/worktree/draft` (own `CARGO_TARGET_DIR=<worktree>/target`).
Driver: Claude (this session). Not merged, not pushed.

## Current State

All four layers built, verified and committed in one commit (a2f47c0e):
- w28 done — A persisted drafts (`crates/ui/src/draft_store.rs`, composer
  sync points) + B amber pen on session rows.
- w29 done — C draft sessions: `AppState.draft_sessions/active_draft`,
  draft = the chat id send mints (no re-key on promotion), tabs + Ctrl+Tab
  cards + sidebar rows (frozen while open, discard ✕), per-draft space /
  device / ref / checkout kind via `Pickers` observer + `record_draft_checkout`.
- w30 done — D Mod+Enter background start (`send(.., background)`,
  `AppState::start_draft_in_background`), Enter send behavior only (d23).
- Decisions d18–d24 locked (d24 supersedes d22 for attachments).
- Verification: `cargo test -p zeron-ui --lib -- --test-threads=1` → 1048
  passed; Dev 8 captures in `docs/research/draft/capture-*.png`; VERIFY.md
  rows 1–12.
- Rig left in place: `target/macos-dev8/Zeron Dev 8.app` (port 49790, mock
  harness, seeded data dir with space `space-draft`, chats chat-a/chat-b,
  drafts `draft-seed` + one typed draft). App is quit.

## Next Action

Reina reviews the diff and the captures; merging `feat/draft` into `main`
is her call. Known deviations from "y chang T3" to decide on or accept:
1. Harness / model / reasoning / permission-mode picks are per canvas visit,
   not per draft, and never persist (T3 persists modelSelection per draft).
2. Back/Forward to the canvas lands on a fresh draft, not the one left
   (`NavEntry::Chat("")` carries no draft id).
3. A background start that FAILS restores its text with `set_text` into the
   next draft's input, clobbering anything typed there in between.
4. Under the Mod+Enter send behavior there is no background chord (d23).
5. Image-attachment persistence is unit-tested only (no in-app capture).
6. Two capture knobs shipped in production code: `ZERON_DRAFT_TEXT`,
   `ZERON_OPEN_DIALOG=background-send` (same family as the existing knobs).

## Authority

Original: Reina 2026-09-17 "y chang t3 là toàn bộ đúng chứ?" (all four
layers) → "okay" (bundle + SPEC) → "làm đi" (build). Local commits on the
feature branch are authorized by standing feedback; push / merge / release
are NOT and were not done.

## Failed approaches

- Synthetic keyboard and pointer input (`/tmp/cgkey`, `/tmp/cgclick`) never
  reached Dev 8 even with the caret visible; replaced by the two capture
  knobs. One retry only, per the worktree memory.
- Reading `composer-drafts.json` 3s after launch missed the boot draft:
  boot sync ~2.5s + 300ms debounce. Read ≥7s after launch.
- First seeded text landed under key "" because the composer's observer
  had not yet swapped to the boot draft id; fixed by carrying "" text and
  attachments into the draft the key becomes (composer swap block).

## Do not repeat

- Do not `cargo fmt --all` or `rustfmt shell.rs` directly (mod children);
  format via stdin per file.
- Do not run `cargo test`/`build` while editing sources in the same target
  dir; the chain recompiles half-edited files.
- Do not `select_chat(Some(<draft id>))`; `select_existing_chat` now routes
  draft ids to `open_draft`, keep it that way.
- Do not kill any Zeron app other than `macos-dev8` (peers run dev..dev6;
  Reina's production app hosts this session).
