#!/usr/bin/env bash
# Seed a fresh worktree's target/debug and apps/zgx/target with copy-on-write
# clones of the main checkout's build artifacts, so third-party crates are not
# rebuilt, turn incremental compilation off there, and trash other worktrees'
# cold targets.
# Usage: scripts/seed-worktree-target.sh [main-checkout] [worktree]

set -euo pipefail

SCRIPTS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-${ZERON_PROJECT_ROOT:-$(git -C "$SCRIPTS" worktree list --porcelain | sed -n '1s/^worktree //p')}}"
WORKTREE="${2:-${ZERON_WORKTREE_PATH:-$PWD}}"

if [ "$(cd "$ROOT" && pwd -P)" = "$(cd "$WORKTREE" && pwd -P)" ]; then
  echo "seed: $WORKTREE is the main checkout; nothing to do"
  exit 0
fi

"$SCRIPTS/sweep-cold-targets.sh" || echo "seed: sweep failed; continuing"

# incremental/ is about 24G of a worktree's target/debug and 6G of its apps/zgx one.
if [ ! -e "$WORKTREE/.cargo/config.toml" ]; then
  mkdir -p "$WORKTREE/.cargo"
  printf '[build]\nincremental = false\n' >"$WORKTREE/.cargo/config.toml"
  echo "seed: wrote $WORKTREE/.cargo/config.toml (incremental off)"
fi

seed() {
  local src="$ROOT/$1" target="$WORKTREE/$1" profile
  if [ -e "$target" ]; then
    echo "seed: $target already exists; skipping"
    return
  fi
  # A plain copy is 100G+; only seed when clonefile can share blocks.
  if [ -d "$src" ] && [ "$(stat -f %d "$src")" != "$(stat -f %d "$WORKTREE")" ]; then
    echo "seed: $WORKTREE is on another volume; skipping $1"
    return
  fi
  local start=$SECONDS
  for profile in "${@:2}"; do
    [ -d "$src/$profile/deps" ] || continue
    mkdir -p "$target/$profile"
    # Fingerprints first: a unit the main checkout rebuilds mid-copy then looks stale, never fresh.
    for dir in .fingerprint build deps; do
      cp -cR "$src/$profile/$dir" "$target/$profile/"
    done
  done
  if [ ! -d "$target" ]; then
    echo "seed: $src has nothing built; skipping"
    return
  fi
  # cargo writes this tag only when it creates target/ itself, and `cargo clean` refuses a target without it.
  printf '%s\n' 'Signature: 8a477f597d28d172789f06886806bc55' \
    '# This file is a cache directory tag created by cargo.' \
    '# For information about cache directory tags see https://bford.info/cachedir/' \
    >"$target/CACHEDIR.TAG"
  echo "seed: cloned $src into $target in $((SECONDS - start))s"
}

seed target debug
seed apps/zgx/target debug release
