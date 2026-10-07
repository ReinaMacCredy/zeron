# Agent conventions for this fork

Local `main` is the integration branch; upstream is `origin` (zeronsh/zeron).
Never push, tag, or bump the version without Reina's explicit approval.

## Worktrees

- One branch per unit of work, in `.maestro/worktree/<slug>` with its own
  `CARGO_TARGET_DIR=$PWD/target`. Never build a worktree into the main
  checkout's `target/`.
- Right after `git worktree add`, run `scripts/seed-worktree-target.sh` from
  the new worktree, before its first build: it APFS-clones the main checkout's
  `target/debug` (about 2-3 min, no extra disk) so only workspace crates
  rebuild. Worktrees Zeron creates get this from the project's setup Action.
- Peers edit the main checkout live. Never revert, format, or stash files
  there that you did not change.
- Visual proofs run in an isolated `target/macos-dev<N>/Zeron Dev <N>.app`
  (see `scripts/run-macos-dev.sh`), never in `/Applications/Zeron.app`, which
  hosts the running sessions.

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

## Build hygiene

- `[profile.dev]` uses `debug = "line-tables-only"` because macOS keeps every
  codegen `.o` in `target/debug/deps` and Cargo never prunes stale hashes.
  Do not raise it back to full debuginfo in a shared checkout.
- Never `cargo fmt --all`; format only files you changed, with `rustfmt
  --edition 2024`.
