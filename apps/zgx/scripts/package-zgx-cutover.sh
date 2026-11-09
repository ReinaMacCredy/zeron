#!/usr/bin/env bash
# Zeron.app with the zgx interface in it (apps/zgx built with its `cutover`
# feature, src/identity.rs): the Zeron app itself, not the beta beside it.
# Zeron.app's bundle id, name, icon and `zeron://` scheme; ~/.zeron and port
# 27654 as the engine's; it starts that engine when none answers and
# updates itself from the fork's releases. Nothing here publishes anything.
#
#   target/package-zgx/Zeron.app
#     Contents/MacOS/zeron           the zgx interface and `zeron`'s command line
#     Contents/MacOS/Zeron Engine    the engine (the root workspace's)
#     Contents/Resources/ui, plugins-builtin, cua-driver, zeron.icns, licenses
#   target/package-zgx/zeron-<version>-macos-<arch>-app.tar.gz   (the updater's)
#
# Usage: scripts/package-zgx-cutover.sh   (from apps/zgx or anywhere; macOS only)
#   ZGX_CUTOVER_ENGINE   a `zeron-engine` built already (else built here, release)
#   ZGX_CUTOVER_ZGX_BIN  the interface built already with --features cutover
#   ZGX_TARGET_DIR       where to build the interface (apps/zgx/target/package-zgx-cutover:
#                        a target dir of its own, so a Dev's release binary,
#                        apps/zgx/target/release/zeron-zgx, is never this build)
#   CARGO_FLAGS          extra cargo flags (CI: --locked)
# Signing as scripts/package-macos.sh: CODESIGN_IDENTITY; notarizing with
# NOTARY_KEY_PATH + NOTARY_KEY_ID + NOTARY_ISSUER_ID.

set -euo pipefail

ZGX="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(cd "$ZGX/../.." && pwd)"
command -v cargo >/dev/null 2>&1 || PATH="$HOME/.cargo/bin:$PATH"
die() { echo "$*" >&2; exit 1; }
[[ "$(uname -s)" == Darwin ]] || die "Zeron.app is packaged on macOS"

APP_NAME="Zeron"
BUNDLE_ID=sh.zeron.app
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "the workspace's version '$VERSION' isn't x.y.z"
ARCH="$(uname -m)"
OUT_DIR="$ROOT/target/package-zgx"
APP="$OUT_DIR/$APP_NAME.app"
APP_TARBALL="$OUT_DIR/zeron-$VERSION-macos-$ARCH-app.tar.gz"
ENTITLEMENTS="$ROOT/dist/macos/Zeron.entitlements"
read -r -a CARGO_EXTRA <<<"${CARGO_FLAGS:-}"
mkdir -p "$OUT_DIR"

ENGINE_BIN="${ZGX_CUTOVER_ENGINE:-}"
if [[ -z "$ENGINE_BIN" ]]; then
  (cd "$ROOT" && cargo build --release ${CARGO_EXTRA[@]+"${CARGO_EXTRA[@]}"} -p zeron --bin zeron-engine)
  ENGINE_BIN="$ROOT/target/release/zeron-engine"
fi
[[ -x "$ENGINE_BIN" ]] || die "no engine at $ENGINE_BIN"
[[ "$("$ENGINE_BIN" --version)" == "zeron-engine $VERSION" ]] || die "$ENGINE_BIN isn't the $VERSION engine"

ZGX_BIN="${ZGX_CUTOVER_ZGX_BIN:-}"
if [[ -z "$ZGX_BIN" ]]; then
  target_dir="${ZGX_TARGET_DIR:-$ZGX/target/package-zgx-cutover}"
  # (From its folder: the build's flags are its .cargo/config.toml.)
  (cd "$ZGX" && env -u ZERON_BETA_VERSION cargo build --release ${CARGO_EXTRA[@]+"${CARGO_EXTRA[@]}"} --features cutover --target-dir "$target_dir")
  ZGX_BIN="$target_dir/release/zeron-zgx"
fi
[[ "$("$ZGX_BIN" --version)" == "zeron $VERSION" ]] || die "$ZGX_BIN isn't the $VERSION interface (it says '$("$ZGX_BIN" --version)')"
# Production's edge only.
STAGING_MARKS='staging\.zeron\.sh|zeron-edge-staging|client_01KWD0E9SYPV2WE4QN9TP9DP0W'
for bin in "$ZGX_BIN" "$ENGINE_BIN"; do
  if LC_ALL=C grep -aqE "$STAGING_MARKS" "$bin"; then
    echo "$bin carries staging's edge or sign-in client" >&2
    exit 1
  fi
done

rm -rf "$APP" "$APP_TARBALL"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/cua-driver"
install -m 755 "$ZGX_BIN" "$APP/Contents/MacOS/zeron"
install -m 755 "$ENGINE_BIN" "$APP/Contents/MacOS/Zeron Engine"
DRIVER="$APP/Contents/Resources/cua-driver/cua-driver"
"$ROOT/scripts/build-cua-driver.sh" "$DRIVER" >/dev/null
mkdir -p "$APP/Contents/Resources/licenses/cua-driver" "$APP/Contents/Resources/licenses/fonts"
cp "$ROOT/vendor/cua-driver/LICENSE.md" "$ROOT/vendor/cua-driver/NOTICE.md" "$APP/Contents/Resources/licenses/cua-driver/"

# Zeron.app's own Info.plist (its id, name, `zeron://`), with zgx's floor, macOS 13.
PLIST="$APP/Contents/Info.plist"
sed "s/__VERSION__/$VERSION/" "$ROOT/dist/macos/Info.plist" >"$PLIST"
plutil -replace CFBundleIdentifier -string "$BUNDLE_ID" "$PLIST"
plutil -replace CFBundleName -string "$APP_NAME" "$PLIST"
plutil -replace CFBundleDisplayName -string "$APP_NAME" "$PLIST"
plutil -replace LSMinimumSystemVersion -string "13.0" "$PLIST"
plutil -convert xml1 "$PLIST"
plutil -lint "$PLIST" >/dev/null
/usr/libexec/PlistBuddy -c 'Print :CFBundleURLTypes:0:CFBundleURLSchemes:0' "$PLIST" | grep -qx zeron || die "the bundle lost zeron://"

rsync -a --exclude '.zgx/' --exclude '/tests/' --exclude '.lock' --exclude '.DS_Store' "$ZGX/ui/" "$APP/Contents/Resources/ui/"
rsync -a --exclude '.zgx/' --exclude '.DS_Store' "$ZGX/plugins-builtin/" "$APP/Contents/Resources/plugins-builtin/"
cp "$ROOT/crates/ui/assets/fonts/licenses/"* "$ZGX/fonts/"*-OFL.txt "$APP/Contents/Resources/licenses/fonts/"
cp "$ROOT/THIRD_PARTY_NOTICES.md" "$ROOT/LICENSE" "$APP/Contents/Resources/licenses/"
cp "$ROOT/crates/voice/NOTICE.md" "$APP/Contents/Resources/licenses/parakeet-v3.txt"
"$ROOT/scripts/macos-app-icon.sh" "$APP/Contents/Resources"

if [[ -n "${CODESIGN_IDENTITY:-}" ]]; then
  SIGN=(--options runtime --timestamp --sign "$CODESIGN_IDENTITY")
else
  SIGN=(--sign -)
  echo "warning: ad-hoc signed: macOS privacy grants don't survive a rebuild" >&2
fi
codesign --force "${SIGN[@]}" --identifier sh.zeron.cua-driver "$DRIVER"
codesign --force "${SIGN[@]}" --entitlements "$ENTITLEMENTS" --identifier sh.zeron.app.engine "$APP/Contents/MacOS/Zeron Engine"
codesign --force "${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$APP"
codesign --verify --strict --verbose=2 "$APP"

if [[ -n "${NOTARY_KEY_PATH:-}" && -n "${NOTARY_KEY_ID:-}" && -n "${NOTARY_ISSUER_ID:-}" ]]; then
  ZIP="$OUT_DIR/zeron-notarize.zip"
  ditto -c -k --keepParent "$APP" "$ZIP"
  xcrun notarytool submit "$ZIP" --key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID" --wait
  rm -f "$ZIP"
  xcrun stapler staple "$APP"
fi

tar -czf "$APP_TARBALL" -C "$OUT_DIR" "$APP_NAME.app"
echo "packaged: $APP_TARBALL"
