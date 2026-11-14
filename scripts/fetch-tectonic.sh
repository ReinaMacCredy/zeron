#!/usr/bin/env bash
# Fetch the pinned Tectonic release for this Mac, checked against its SHA-256
# (GitHub's asset digest for the tectonic@0.17.0 release), and its license.
# Usage: scripts/fetch-tectonic.sh [binary-path] [license-path]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="0.17.0"
LICENSE_SHA256="814a258f76e420b25cb3c07172eb2b3956f34cefbf0a650413b78e65c425f306"
case "$(uname -m)" in
  arm64) TARGET="aarch64-apple-darwin"; SHA256="a3f1cac7c5678f01661a92212f58480ae3b0634115d880dbc59e2953ded45667" ;;
  x86_64) TARGET="x86_64-apple-darwin"; SHA256="7c90ef5b6ddb1eb1937e4337add5237b79338e4b9676459fa91187d24d6cdf80" ;;
  *) echo "tectonic: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
OUT="${1:-$ROOT/target/tectonic/tectonic}"
LICENSE_OUT="${2:-$(dirname "$OUT")/LICENSE}"
CACHE="${TECTONIC_DOWNLOAD_DIR:-$ROOT/target/tectonic-download}"
RELEASE="https://github.com/tectonic-typesetting/tectonic/releases/download/tectonic%40$VERSION"
SOURCE="https://raw.githubusercontent.com/tectonic-typesetting/tectonic/tectonic%40$VERSION"

matches() { [[ -f "$2" ]] && echo "$1  $2" | shasum -a 256 -c - >/dev/null 2>&1; }
fetch() {
  local url="$1" file="$2" sum="$3"
  matches "$sum" "$file" && return 0
  curl -fsSL --retry 3 -o "$file.part" "$url"
  mv "$file.part" "$file"
  matches "$sum" "$file" || { echo "tectonic: $url doesn't match its pinned SHA-256" >&2; rm -f "$file"; exit 1; }
}

mkdir -p "$CACHE"
ARCHIVE="$CACHE/tectonic-$VERSION-$TARGET.tar.gz"
fetch "$RELEASE/tectonic-$VERSION-$TARGET.tar.gz" "$ARCHIVE" "$SHA256"
fetch "$SOURCE/LICENSE" "$CACHE/LICENSE-$VERSION" "$LICENSE_SHA256"

UNPACK="$(mktemp -d)"
trap 'rm -rf "$UNPACK"' EXIT
tar -xzf "$ARCHIVE" -C "$UNPACK" tectonic
mkdir -p "$(dirname "$OUT")"
# Content copy, not clonefile: a clone keeps com.apple.provenance and Gatekeeper kills it at exec.
cat "$UNPACK/tectonic" > "$OUT"
chmod 755 "$OUT"
mkdir -p "$(dirname "$LICENSE_OUT")"
cp "$CACHE/LICENSE-$VERSION" "$LICENSE_OUT"
"$OUT" --version >/dev/null
echo "$OUT"
