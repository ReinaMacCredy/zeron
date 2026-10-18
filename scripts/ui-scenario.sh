#!/usr/bin/env bash
# Run one UI scenario (scripts/scenarios/README.md) against a fresh, isolated
# "Zeron Scenario" Dev bundle, then quit it. Exits with the scenario's code.
#
#   scripts/ui-scenario.sh <scenario.json> [--mock-script <path>] [--keep] [--port <n>]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
usage() {
  echo "usage: scripts/ui-scenario.sh <scenario.json> [--mock-script <path>] [--keep] [--port <n>]" >&2
  exit 2
}

SCENARIO=""
MOCK=""
KEEP=0
PORT=49880
while [[ $# -gt 0 ]]; do
  case "$1" in
    --mock-script) [[ $# -ge 2 ]] || usage; MOCK="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    --port) [[ $# -ge 2 ]] || usage; PORT="$2"; shift 2 ;;
    -*) usage ;;
    *) [[ -z "$SCENARIO" ]] || usage; SCENARIO="$1"; shift ;;
  esac
done
[[ -n "$SCENARIO" && -f "$SCENARIO" ]] || usage
SCENARIO="$(realpath "$SCENARIO")"
NAME="$(python3 -c 'import json, re, sys; print(re.sub(r"[^A-Za-z0-9_-]", "-", json.load(open(sys.argv[1]))["name"]))' "$SCENARIO")"
if [[ ! "$NAME" =~ [A-Za-z0-9_] ]]; then
  echo "the scenario \"name\" needs a letter, digit or underscore; it maps to \"$NAME\"" >&2
  exit 2
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
BASE="$ROOT/target/ui-scenario"
WORK="$BASE/$NAME"
APP_NAME="Zeron Scenario"
BUNDLE_ID="sh.zeron.app.dev.scenario"
APP="$BASE/bundle/$APP_NAME.app"
ZERON="$CARGO_TARGET_DIR/debug/zeron"

live_apps() {
  "$ZERON" ui list 2>/dev/null | jq -r '.[] | [.pid, (.bundlePath // "-"), .dataDir] | @tsv' || true
}

same_bundle() {
  [[ -n "$1" && -e "$APP" && "$(realpath "$1" 2>/dev/null)" == "$(realpath "$APP")" ]]
}

if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  echo "port $PORT already has a listener; the bundle would attach to that engine. Pass --port." >&2
  exit 2
fi
command -v cargo >/dev/null 2>&1 || PATH="$HOME/.cargo/bin:$PATH"
(cd "$ROOT" && cargo build -p zeron)
while IFS=$'\t' read -r pid bundle data; do
  if same_bundle "$bundle"; then
    echo "$APP_NAME is already running (pid $pid, data $data); quit it first" >&2
    exit 2
  fi
  if [[ -e "$WORK" && "$(realpath "$data" 2>/dev/null)/" == "$(realpath "$WORK")/"* ]]; then
    echo "pid $pid runs on $data; quit it before $WORK moves to the Trash" >&2
    exit 2
  fi
done < <(live_apps)
# The dcg guard blocks recursive deletes, so the previous run moves to the Trash.
if [[ -e "$WORK" ]]; then
  mv "$WORK" "$HOME/.Trash/ui-scenario-$NAME-$(date +%Y%m%d-%H%M%S)"
fi
mkdir -p "$WORK/data" "$WORK/project"
git init -q "$WORK/project"

export ZERON_DEV_ROOT="$BASE/bundle"
export ZERON_DEV_APP_NAME="$APP_NAME"
export ZERON_DEV_BUNDLE_ID="$BUNDLE_ID"
export ZERON_DEV_DATA_DIR="$WORK/data"
export ZERON_DEV_IPC_PORT="$PORT"
export ZERON_HARNESS="${ZERON_HARNESS:-mock}"
export ZERON_SCENARIO_PROJECT="$WORK/project"
if [[ -n "$MOCK" ]]; then
  export ZERON_MOCK_SCRIPT="$(realpath "$MOCK")"
fi

echo "scenario $NAME: building and launching $APP_NAME (port $PORT, log $WORK/launch.log)" >&2
"$ROOT/scripts/run-macos-dev.sh" >"$WORK/launch.log" 2>&1 &
LAUNCHER=$!
PID=""

stop_tree() {
  local children child
  children="$(pgrep -P "$1" || true)"
  kill "$1" 2>/dev/null || true
  for child in $children; do stop_tree "$child"; done
}

runs_bundle() {
  local exe app
  exe="$(ps -o comm= -p "$1" 2>/dev/null)" || return 1
  app="$(realpath "$APP" 2>/dev/null)" || return 1
  [[ "$(realpath "$exe" 2>/dev/null)" == "$app/"* ]]
}

finish() {
  if [[ -z "$PID" ]]; then
    stop_tree "$LAUNCHER"
    return 0
  fi
  if [[ "$KEEP" == 1 ]] || ! kill -0 "$PID" 2>/dev/null; then
    [[ "$KEEP" == 1 ]] && echo "kept $APP_NAME running (pid $PID)" >&2
    return 0
  fi
  if ! runs_bundle "$PID"; then
    echo "pid $PID no longer runs $APP; leaving it alone" >&2
    return 0
  fi
  osascript -e "tell application id \"$BUNDLE_ID\" to quit" >/dev/null 2>&1 || true
  for _ in $(seq 1 40); do
    kill -0 "$PID" 2>/dev/null || return 0
    sleep 0.25
  done
  if runs_bundle "$PID"; then
    kill "$PID" 2>/dev/null || true
  fi
}
trap finish EXIT

deadline=$((SECONDS + 600))
until [[ -n "$PID" ]] && (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; do
  if [[ -z "$PID" ]]; then
    while IFS=$'\t' read -r pid bundle _; do
      if same_bundle "$bundle"; then
        PID="$pid"
        break
      fi
    done < <(live_apps)
  fi
  if ! kill -0 "$LAUNCHER" 2>/dev/null; then
    echo "run-macos-dev.sh exited before $APP_NAME came up; tail of $WORK/launch.log:" >&2
    tail -20 "$WORK/launch.log" >&2
    exit 2
  fi
  if ((SECONDS > deadline)); then
    echo "timed out after 600 s waiting for $APP_NAME (descriptor pid: ${PID:-none}, port $PORT)" >&2
    exit 2
  fi
  sleep 1
done

echo "scenario $NAME: driving pid $PID, report $WORK/report" >&2
set +e
"$ZERON" ui scenario "$SCENARIO" --target "$PID" --out "$WORK/report"
CODE=$?
set -e
echo "report: $WORK/report/report.md" >&2
exit "$CODE"
