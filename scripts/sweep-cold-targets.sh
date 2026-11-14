#!/usr/bin/env bash
# Move every worktree's target/ and apps/zgx/target/ that no build has written
# to for some days into ~/.Trash. The main checkout's targets are never touched.
# Disk space comes back when the Trash is emptied.
# Usage: scripts/sweep-cold-targets.sh [days, default 3]

set -euo pipefail

DAYS="${1:-${ZERON_SWEEP_DAYS:-3}}"
SCRIPTS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TRASH="$HOME/.Trash"

git -C "$SCRIPTS" worktree list --porcelain | sed -n 's/^worktree //p' | sed 1d |
  while IFS= read -r worktree; do
    for rel in target apps/zgx/target; do
      target="$worktree/$rel"
      [ -d "$target" ] || continue
      if [ -n "$(find "$target" -maxdepth 3 -mtime "-$DAYS" -print -quit)" ]; then
        continue
      fi
      dest="$TRASH/$(basename "$worktree")-${rel//\//-}-$(date +%Y%m%d-%H%M%S)"
      mv "$target" "$dest"
      echo "sweep: $target idle over $DAYS days; moved to $dest"
    done
  done
