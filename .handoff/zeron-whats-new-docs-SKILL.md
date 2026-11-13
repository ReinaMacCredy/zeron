---
name: zeron-whats-new-docs
description: Update Zeron's What's New entry, in-app Docs articles and their screenshots for a release or upstream merge. Use when a version lands (fork feature, upstream vX.Y.Z integration, iOS work) and the changelog, docs or images must catch up, or when asked to "update whats new", "update docs", "chụp lại ảnh docs".
---

# What's New and Docs for a Zeron release

Done means: the release's user-visible changes are in `crates/ui/whats_new.toml`,
every changed feature's article in `crates/ui/docs/` matches the running app,
every image is a fresh native-resolution capture registered in `IMAGES`
(`crates/docs/src/images.rs`), and the
tests below pass.

## 1. Collect what changed

- Fork side: `git log --first-parent <last-fork-tag>..main` on local main. The
  last fork release is `gh release list -R ReinaMacCredy/zeron --limit 1`.
- Upstream side: `gh release view vX.Y.Z -R zeronsh/zeron` for each upstream tag
  since that release.
- Keep only what a user can see or do. Internal refactors, tests and CI stay out.

## 2. whats_new.toml

- One `[[entry]]` per version: `version`, `date` ("Oct 20"), optional `hero`
  image or `hero_visual`, then `[[entry.features]]` with `id`, `title`,
  `description`, optional `image` or `visual`, optional `details`.
- Fork features first, then one `upstream-<x-y-z>` feature per upstream release
  summarizing it. `scripts/release-notes.py` drops `upstream-*` ids from the
  "In this fork" list because the upstream PR lines already cover them, so the
  id prefix matters.
- Sort each fork change into one of two kinds (Reina, 2026-10-25):
  - **New feature** (something the user could not do before): its own
    `[[entry.features]]` with a live animated `visual`. Build one in
    `crates/ui/src/shell/intros/release.rs` when none exists (register it in
    its `VISUALS` table), the way the 0.2.129 fork features got theirs.
  - **Fix or improvement** to something that already existed: no feature card,
    no visual. All of them go into one grouped "Improvements and fixes" block
    per release (`[[entry.improvements]]` with `text` and an optional
    `visual`), one short line each. Exception: when the fix or improvement is
    itself about motion (an animation that was fixed or reworked), its line in
    the block still shows that animation as a live visual.
- `visual` names a live gpui visual from `crates/ui/src/shell/intros.rs` or
  `intros/release.rs` (`VISUALS`); an unknown id fails the test. Do not invent a visual for an
  upstream feature; upstream features keep a static `image`.
- Titles are a plain sentence of what the user gets; descriptions say where to
  find it. No marketing words, no em dashes.

## 3. Docs articles

- Articles live in `crates/ui/docs/<id>.md`, are indexed in
  `crates/ui/docs/index.toml` (section, title, icon, blurb, actions) and must be
  listed in `crates/docs/src/lib.rs` `BODIES`. A new article needs all three.
- Before editing text, open the feature in a Dev bundle and check the wording
  against what is on screen: labels, menu items, order, colors. Fix text that the
  app contradicts; flag (do not silently delete) lines you cannot reproduce.
- `updates.md` gets a line for any new onboarding sheet or update surface.

## 4. Screenshots

- Capture from an isolated `Zeron Dev <N>` bundle with `ZERON_HARNESS=mock`
  (`AGENTS.md`, `docs/app-api.md` "Launching a Dev bundle to drive"), never
  `/Applications/Zeron.app`. Seed state with `ZERON_MOCK_SCRIPT` or scenario
  files; open dialogs with `ZERON_OPEN_DIALOG` (`whats-new-card`, `task-card`,
  `add-provider`, `model`, ...; grep the knob for
  the full list).
- Capture with `zeron app snapshot` with `"native": true` (no Screen Recording
  grant needed), or `screencapture`, so the file is native Retina resolution
  (2640x1760 for a full window).
  Never downscale; convert to `.jpg` only.
- Use neutral sample content ("Ship the shop", "MacBook Pro"). Open every image
  before committing and reject any with an email, a real path, a real repo name
  or a private thread.
- Images go in `crates/ui/assets/whats_new/` and must be added to `IMAGES` in
  `crates/docs/src/images.rs`; docs reference them by bare file name.
- iOS: capture from the iOS Simulator (`device_screenshot` or
  `xcrun simctl io booted screenshot`), then crop or set 2-3 phones side by side
  so it fits the docs frame.
- When done, quit the Dev bundle and `mv target/macos-dev<N> ~/.Trash`.

## 5. Verify

```
cargo test -p zeron-docs
cargo test -p zeron-ui --lib whats_new
cargo test -p zeron-ui --lib docs::
python3 scripts/release-notes.py vX.Y.Z /tmp/notes.md   # needs gh; check the body
```

The `zeron-ui` tests fail on an unregistered image, a broken article link or a
bad action, and the changelog test fails on a malformed entry. For a new
`visual`, open the What's New card and dialog in the Dev bundle and capture two
frames about a second apart to show it moves.

## 6. Hand off

Commit only these paths (`git commit -m ... -- <paths>`). Report the entry, the
articles touched, the images recaptured vs kept with the reason, the test
output, and any doc line left unverified. Pushing, tagging and creating the
GitHub release need Reina's explicit approval.
