#!/usr/bin/env bash
# Build the vendored cua-driver with its pinned toolchain.
# Usage: scripts/build-cua-driver.sh [output-path]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/vendor/cua-driver"
TOOLCHAIN="1.97.1"
TARGET="aarch64-apple-darwin"
OUT="${1:-$ROOT/target/cua-driver/cua-driver}"
TARGET_DIR="${CUA_DRIVER_TARGET_DIR:-$ROOT/target/cua-driver-build}"

# Homebrew cargo shadows the rustup proxy on some machines; go through rustup
# explicitly and assert the compiler, or the pin is silently ignored.
found="$(rustup run "$TOOLCHAIN" rustc --version)"
case "$found" in
  "rustc $TOOLCHAIN "*) ;;
  *) echo "cua-driver needs rustc $TOOLCHAIN, found: $found" >&2; exit 1 ;;
esac

# rust-lang/rust#157750: stripping misaligns proc-macro dylibs that macOS 27
# refuses to load.
export CARGO_PROFILE_RELEASE_STRIP="${CARGO_PROFILE_RELEASE_STRIP:-none}"
export CUA_DRIVER_RS_TELEMETRY_ENABLED=0

(cd "$SRC" && rustup run "$TOOLCHAIN" cargo build --release --locked \
  --target-dir "$TARGET_DIR" --target "$TARGET" -p cua-driver)

mkdir -p "$(dirname "$OUT")"
# Content copy, not clonefile: a cloned file keeps com.apple.provenance and
# Gatekeeper kills it at exec.
cat "$TARGET_DIR/$TARGET/release/cua-driver" > "$OUT"
chmod 755 "$OUT"
echo "$OUT"
