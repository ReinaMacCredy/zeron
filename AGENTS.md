# Agent conventions for this fork

Local `main` is the integration branch; upstream is `origin` (zeronsh/zeron).
Never push, tag, or bump the version without Reina's explicit approval.

## Where to work

Every worktree carries its own 100G+ `target/`, so work in the main checkout
on `main` by default, whatever the size of the change. Take a worktree only
when another thread is already working in the main checkout: `git status
--short` shows changes you did not make, or Reina says other threads are
running. A thread that started on main stays there when others arrive; the
newcomers take worktrees.

- In the main checkout:
  - Commit only your own paths: `git commit -m "..." -- <paths>`. Never
    `git commit -a`, `git add .`, or `git add -A`.
  - Iterate with `cargo check -p <crate>`; run `cargo build` or `cargo test`
    only to verify.
  - Never switch branches here, and never revert, format, or stash files you
    did not change.
- In a worktree, `.maestro/worktree/<slug>` on its own branch:
  - Build with its own `CARGO_TARGET_DIR=$PWD/target`, never into the main
    checkout's `target/`.
  - Right after `git worktree add`, run `scripts/seed-worktree-target.sh`
    from the new worktree, before its first build: it APFS-clones the main
    checkout's `target/debug` (about 2-3 min, no extra disk) so only
    workspace crates rebuild. Worktrees Zeron creates get this from the
    project's setup Action.
- Visual proofs run in an isolated `target/macos-dev<N>/Zeron Dev <N>.app`
  (see `scripts/run-macos-dev.sh`), never in `/Applications/Zeron.app`, which
  hosts the running sessions. Inspect and drive it with `zeron ui` or the
  `ui_driver` and `ui_driver_drive` tools (`docs/ui-driver.md`), not
  computer use.

## Finishing a worktree: merge gate, then clean up

When the work is verified, run this gate from the main checkout and act on it
in the same hand-off, without waiting to be asked:

1. `git status --short` in the main checkout. Compare the tracked modified
   files against `git diff --name-only main...<branch>`. Any overlap means a
   peer is mid-edit in a file you touched: stop, report, leave the merge to
   Reina.
2. No overlap: `git merge --no-edit <branch>`. A conflict means abort
   (`git merge --abort`), report, stop.
3. Merge landed: build the crates you touched on `main` (`cargo build -p
   <crate>`; for `crates/ui`, `cargo test -p zeron-ui --lib --
   --test-threads=1`). A failure means revert the merge commit and report.
4. Green: `git worktree remove --force .maestro/worktree/<slug>` and
   `git branch -d <branch>`. Quit your `Zeron Dev <N>` and move its
   `target/macos-dev<N>` folder to `~/.Trash` (the dcg guard blocks recursive
   deletes; `mv` is the sanctioned path).
5. Report the merge commit, the check you ran, and that the running app still
   carries the old code until Reina rebuilds.

Branches the gate refused stay in their worktree; say so in the hand-off so
the next session does not mistake them for abandoned.

## Tests

- Verification is required; new tests are not. Spacing, color, size, copy,
  and layout tweaks are proven with a `Zeron Dev <N>` screenshot, never a new
  gpui fixture test that pins pixel bounds.
- Add a test only for behavior no existing test covers and that could
  plausibly regress; extend a nearby test before building a new fixture.

## Build hygiene

- `[profile.dev]` uses `debug = "line-tables-only"` because macOS keeps every
  codegen `.o` in `target/debug/deps` and Cargo never prunes stale hashes.
  Do not raise it back to full debuginfo in a shared checkout.
- Run `scripts/gc-target.sh` before a `cargo build` or `cargo test`, in the
  main checkout or a worktree. It `cargo clean -p`s each workspace crate whose
  artifacts passed 8G, so stale codegen stops piling up while third-party
  deps stay built. It waits on cargo's lock, so a peer's build is safe.
- Never `cargo fmt --all`; format only files you changed, with `rustfmt
  --edition 2024`.
