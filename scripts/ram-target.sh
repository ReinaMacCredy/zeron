#!/usr/bin/env bash
# Mount the RAM disk worktree targets live on, recreate the directories their
# target symlinks point to, drop the ones idle over a day or left by a removed
# worktree, and print the mount point.
# Usage: scripts/ram-target.sh [GB, default 24] | --path

set -euo pipefail
shopt -s nullglob

NAME=ZeronBuild
MOUNT="/Volumes/$NAME"
if [ "${1:-}" = --path ]; then
  echo "$MOUNT"
  exit 0
fi
GB="${1:-${ZERON_RAM_GB:-24}}"
SCRIPTS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCK="${TMPDIR:-/tmp}/zeron-ram-target.lock"

until mkdir "$LOCK" 2>/dev/null; do sleep 0.2; done
trap 'rmdir "$LOCK"' EXIT

if ! mount | grep -q " on $MOUNT "; then
  dev="$(hdiutil attach -nomount "ram://$((GB * 2097152))" | awk '{print $1}')"
  diskutil erasevolume APFS "$NAME" "$dev" >/dev/null
  touch "$MOUNT/.metadata_never_index"
  echo "ram: mounted ${GB}G at $MOUNT" >&2
fi

live=()
while IFS= read -r worktree; do
  for rel in target apps/zgx/target; do
    link="$worktree/$rel"
    [ -L "$link" ] || continue
    dest="$(readlink "$link")"
    case "$dest" in "$MOUNT"/?*/?*) ;; *) continue ;; esac
    live+=("$(dirname "$dest")")
    if [ -d "$dest" ] && [ -z "$(find "$dest" -maxdepth 3 -mtime -1 -print -quit)" ]; then
      rm -rf "$dest"
      echo "ram: $dest idle over a day; emptied" >&2
    fi
    if [ ! -d "$dest" ]; then
      mkdir -p "$dest"
      # cargo writes this tag only when it creates target/ itself, and `cargo clean` refuses a target without it.
      printf '%s\n' 'Signature: 8a477f597d28d172789f06886806bc55' \
        '# This file is a cache directory tag created by cargo.' >"$dest/CACHEDIR.TAG"
    fi
  done
done < <(git -C "$SCRIPTS" worktree list --porcelain | sed -n 's/^worktree //p' | sed 1d)

for dir in "$MOUNT"/*/; do
  dir="${dir%/}"
  case " ${live[*]-} " in *" $dir "*) continue ;; esac
  rm -rf "$dir"
  echo "ram: $dir has no worktree; removed" >&2
done

echo "$MOUNT"
