#!/usr/bin/env bash
# Runs apps/zgx on its own engine, port and data, for `zeron app launch --zgx`.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
command -v cargo >/dev/null 2>&1 || PATH="$HOME/.cargo/bin:$PATH"
DEV_ROOT="${ZERON_DEV_ROOT:-$ROOT/target/zgx-dev}"
[[ "$DEV_ROOT" == /* ]] || DEV_ROOT="$ROOT/$DEV_ROOT"
PORT="${ZERON_DEV_IPC_PORT:-49799}"
NAME="${ZERON_DEV_APP_NAME:-Zeron Beta Dev}"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
ZIG_BIN="${ZIG:-/opt/homebrew/opt/zig@0.16/bin/zig}"

CLEAN_ENV=(
  -u ZERON_APP -u ZERON_DATA_DIR -u ZERON_EDGE_URL -u ZERON_IPC_PORT
  -u ZERON_MANAGED_ENGINE -u ZERON_ORIGIN -u ZERON_WORKOS_CLIENT_ID
  -u ZERON_PROJECT_ROOT -u ZERON_IPC_TOKEN -u ZERON_IPC_TOKEN_FILE
  -u ZERON_RUN_SECRET
)

(cd "$ROOT" && scripts/gc-target.sh && cargo build -p zeron --bin zeron-engine)
(cd "$ROOT/apps/zgx" && env -u CARGO_TARGET_DIR "${CLEAN_ENV[@]}" ZIG="$ZIG_BIN" cargo build --release)

mkdir -p "$DEV_ROOT/data" "$DEV_ROOT/home"
cp -c "$TARGET/debug/zeron-engine" "$DEV_ROOT/zeron-engine"

MOCK=()
if [[ "${ZERON_HARNESS:-}" == mock ]]; then
  MOCK=(ZERON_ZGX_MOCK=1 ZERON_MOCK_DELAY_MS=300)
fi
[[ $# -gt 0 ]] || set -- main

exec env "${CLEAN_ENV[@]}" \
  ZERON_DEV_APP_NAME="$NAME" \
  ZERON_ZGX_ENGINE_PORT="$PORT" \
  ZERON_ZGX_ENGINE_DATA="$DEV_ROOT/data" \
  ZERON_ZGX_ENGINE_HOME="$DEV_ROOT/home" \
  ZERON_ZGX_ENGINE_BIN="$DEV_ROOT/zeron-engine" \
  ZERON_ZGX_EDGE=http://127.0.0.1:9 \
  ZERON_ZGX_NO_BROWSER=1 \
  ${MOCK[@]+"${MOCK[@]}"} \
  "$ROOT/apps/zgx/target/release/zeron-zgx" "$@" --inspect
