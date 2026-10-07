#!/usr/bin/env bash
# Seed a fresh worktree's target/debug with copy-on-write clones of the main
# checkout's build artifacts, so third-party crates are not rebuilt.
# Usage: scripts/seed-worktree-target.sh [main-checkout] [worktree]

set -euo pipefail

ROOT="${1:-${ZERON_PROJECT_ROOT:-$(git -C "$(dirname "${BASH_SOURCE[0]}")" worktree list --porcelain | sed -n '1s/^worktree //p')}}"
WORKTREE="${2:-${ZERON_WORKTREE_PATH:-$PWD}}"
SRC="$ROOT/target/debug"
DST="$WORKTREE/target/debug"

if [ "$(cd "$ROOT" && pwd -P)" = "$(cd "$WORKTREE" && pwd -P)" ]; then
  echo "seed: $WORKTREE is the main checkout; nothing to do"
  exit 0
fi
if [ ! -d "$SRC/deps" ]; then
  echo "seed: $SRC/deps missing; nothing to seed"
  exit 0
fi
if [ -e "$WORKTREE/target" ]; then
  echo "seed: $WORKTREE/target already exists; skipping"
  exit 0
fi
# A plain copy of target/debug is 100G+; only seed when clonefile can share blocks.
if [ "$(stat -f %d "$SRC")" != "$(stat -f %d "$WORKTREE")" ]; then
  echo "seed: $WORKTREE is on another volume; skipping"
  exit 0
fi

mkdir -p "$DST"
start=$SECONDS
# Fingerprints first: a unit the main checkout rebuilds mid-copy then looks stale, never fresh.
for dir in .fingerprint build deps; do
  cp -cR "$SRC/$dir" "$DST/"
done
echo "seed: cloned $SRC into $DST in $((SECONDS - start))s"
