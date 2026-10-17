#!/usr/bin/env bash
# Regenerates crates/ui/src/browser/annotate/annotate.generated.js; cargo only include_str!s the output.
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../crates/ui/src/browser/annotate" && pwd)"
cd "$DIR"
bun install --frozen-lockfile 2>/dev/null || bun install
bun build overlay.ts --minify --format=iife --target=browser --outfile=annotate.generated.js
