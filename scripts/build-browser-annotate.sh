#!/usr/bin/env bash
# Regenerates the committed bundles in crates/ui/src/browser/annotate; cargo only include_str!s them.
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../crates/ui/src/browser/annotate" && pwd)"
cd "$DIR"
bun install --frozen-lockfile
bun build overlay.ts --minify --format=iife --target=browser --outfile=annotate.generated.js
bun build hook.ts --minify --format=iife --target=browser --outfile=hook.generated.js
bun build grab.ts --minify --format=iife --target=browser --outfile=grab.generated.js
