# Agent conventions for this fork

Local `main` is the integration branch; upstream is `origin` (zeronsh/zeron).
Never push, tag, or bump the version without Reina's explicit approval.

## Where to work

Every worktree carries its own 100G+ `target/`, so work in the main checkout
on `main` by default, whatever the size of the change. Take a worktree only
when another thread is already working in the main checkout: `git status
--short` shows changes you did not make, or Reina says other threads are
running. A thread that started on main stays there when others arrive; the
newcomers take worktrees. A peer that has not edited yet does not show, so
run `git status --short` again before your first edit and before each
commit; changes you did not make in a path you touched mean move to a
worktree.

- In the main checkout:
  - Commit only your own paths: `git commit -m "..." -- <paths>`. Never
    `git commit -a`, `git add .`, or `git add -A`.
  - Iterate with `cargo check -p <crate>`; run `cargo build` or `cargo test`
    only to verify.
  - Never switch branches here, and never revert, format, or stash files you
    did not change.
  - When done, quit your `Zeron Dev <N>` and report that the running app
    carries the old code until Reina rebuilds.
- In a worktree, `.maestro/worktree/<slug>` on its own branch:
  - Build with its own `CARGO_TARGET_DIR=$PWD/target`, never into the main
    checkout's `target/`.
  - Before your first cargo command in a worktree, whoever created it, run
    `ls -ld target /Volumes/ZeronBuild`. If `target` is missing or the
    volume is not mounted (a restart unmounts it), run
    `scripts/seed-worktree-target.sh` from the worktree; running it twice is
    harmless. It links `target/` and
    `apps/zgx/target` to a 24G RAM disk (`/Volumes/ZeronBuild`, mounted by
    `scripts/ram-target.sh`), so checks and narrow tests write nothing to
    the SSD, writes a git-ignored `.cargo/config.toml` that turns
    incremental compilation off, and runs `scripts/sweep-cold-targets.sh`,
    which moves any on-disk worktree `target/` idle over 3 days to
    `~/.Trash`. The first check there is cold (about 4 min). Zeron does not
    seed the worktrees it creates.
  - The seed also empties a RAM target idle over a day and removes those of
    deleted worktrees.
  - On the RAM disk run only `cargo check -p <crate>` and narrow tests
    (`cargo test -p <crate> --lib <filter>`, `--test <file>`, a small
    crate): up to 8G a worktree, so three busy ones fit. Never
    `cargo test -p zeron-engine` without `--lib` or `--test` (72 test
    binaries, 18G), `cargo build -p zeron`, or a `Zeron Dev <N>` bundle
    there: they fill the disk for every worktree.
    For a visual proof that has to precede the merge, first run
    `scripts/seed-worktree-target.sh --ssd`, which replaces the links with
    clones of the main checkout's targets on disk. Whole suites and the app
    build run once, on `main`, in the merge gate.
- Visual proofs run in an isolated Dev app on its own engine, port and data
  (`scripts/run-zgx-dev.sh`, which `zeron app launch --zgx` runs), never in
  `/Applications/Zeron.app`, which hosts the running sessions. Pick an `N` no other bundle uses and set
  `ZERON_DEV_ROOT`, `ZERON_DEV_APP_NAME`, `ZERON_DEV_BUNDLE_ID` and
  `ZERON_DEV_IPC_PORT` from it (`docs/app-api.md`, "Launching a Dev
  bundle to drive"); the defaults are shared by every checkout. Inspect and
  drive it with `zeron app` or the `app_inspect` and `app_drive` tools
  (`docs/app-api.md`), not computer use. `zeron app launch --zgx --mock` does
  the same for the zgx app.

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
   <crate>`; for `apps/zgx`, `cargo test` there plus
   `scripts/ci/test-zgx.sh ui <zgx>` for the Luau UI suite) and run the filtered tests of the modules you changed,
   which the worktree skipped. A failure means revert the merge commit and
   report.
4. Green: `git worktree remove --force .maestro/worktree/<slug>` and
   `git branch -d <branch>`. Quit your Dev app and move its
   `ZERON_DEV_ROOT` folder to `~/.Trash` (the dcg guard blocks recursive
   deletes; `mv` is the sanctioned path). A worktree Zeron made for your
   chat (under `~/.zeron/worktrees/`) stays, with its branch: Zeron
   recreates it at the chat's next turn if it is gone, and its cleanup is
   Reina's call.
5. Report the merge commit, the check you ran, and that the running app still
   carries the old code until Reina rebuilds.

Branches the gate refused stay in their worktree; say so in the hand-off so
the next session does not mistake them for abandoned.

## Tests

- Verification is required; new tests are not. Spacing, color, size, copy,
  and layout tweaks are proven with a `Zeron Dev <N>` screenshot, never a new
  UI fixture test that pins pixel bounds.
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
- Never `cargo fmt --all` or `cargo fmt -p`; format only files you changed,
  each through stdin: `rustfmt --edition 2024 < f > f.tmp && mv f.tmp f`.
  `rustfmt <path>` also rewrites the file's `mod` children (`shell.rs`,
  `crates/engine/src/lib.rs`,
  `crates/engine/src/computer/mod.rs` and any other file with `mod x;`).
