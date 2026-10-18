#!/usr/bin/env bash
# Clean the workspace crates whose debug artifacts passed a size limit, keeping
# third-party deps built. Usage: scripts/gc-target.sh [GB-per-crate, default 8]

set -euo pipefail
shopt -s nullglob

LIMIT_GB="${1:-${ZERON_GC_CRATE_GB:-8}}"
TARGET="${CARGO_TARGET_DIR:-$PWD/target}"
DEBUG="$TARGET/debug"

if [ ! -d "$DEBUG/deps" ]; then
  echo "gc: $DEBUG/deps missing; nothing to do"
  exit 0
fi

over=()
for pkg in $(cargo metadata --no-deps --format-version 1 |
  python3 -c 'import json, sys; print("\n".join(p["name"] for p in json.load(sys.stdin)["packages"]))'); do
  crate="${pkg//-/_}"
  files=("$DEBUG/deps/$crate"-* "$DEBUG/deps/lib$crate"-* "$DEBUG/incremental/$crate"-*)
  [ "${#files[@]}" -gt 0 ] || continue
  kb=$(printf '%s\0' "${files[@]}" | xargs -0 du -sk | awk '{ kb += $1 } END { print kb + 0 }')
  if [ "$kb" -gt $((LIMIT_GB * 1024 * 1024)) ]; then
    echo "gc: $pkg $((kb / 1024 / 1024))G > ${LIMIT_GB}G"
    over+=(-p "$pkg")
  fi
done

if [ "${#over[@]}" -eq 0 ]; then
  echo "gc: no crate over ${LIMIT_GB}G in $DEBUG"
  exit 0
fi
cargo clean --target-dir "$TARGET" "${over[@]}"
