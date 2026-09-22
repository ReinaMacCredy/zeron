# Appshots

Appshots capture an application window on the desktop and stage it in a composer
for review before sending. Enable the feature in **Settings → Appshots**, choose
a hotkey and destination, and optionally enable capture sound. Invoke the hotkey
while another application is focused. Invoking it inside Zeron is ignored. No
message is sent automatically. A one-time introduction sheet on desktop routes
to the settings page.

## Hotkey

On macOS the hotkey is a double-modifier chord picked from a menu: both ⌘ keys
(default), both ⌥ keys, both ⇧ keys, or None. A listen-only session event tap on
`flagsChanged` tells the left and right keys apart by their device flags and
fires once per press while both are held; it needs **Input Monitoring**. Older
recorded key combos reload as the default chord. Linux keeps a recorded key
combo (default Ctrl+Alt+Space) and needs no extra permission.

## Settings page

Modeled on Synara's AppSnap panel: an introduction card, a Capture card (Enable
with a live status line, Hotkey, Destination, Capture sound with Preview), and
a macOS permissions card. Each permission row shows Granted/Denied/Not requested
with a Grant button that asks macOS, deep-links the pane, and expands numbered
steps that poll the grant every two seconds until it lands; a Recheck row
re-reads the state without prompting. Restore defaults sits in the page header
and each customized row carries a reset glyph.

Automatic destination follows Synara's affinity: the session interacted with
in the last minute wins, consecutive snaps stay with the previous snap's session,
and otherwise a fresh session opens with the capture attached. A capture landing
on the new-session canvas is keyed to that draft.

## Capture feedback

After the pixels are safe, macOS flashes a white, click-through panel over the
captured window (35 ms to 34 % opacity, 170 ms back to clear, skipped under
Reduce Motion), then plays the shutter cue, then brings Zeron forward and stages
the capture. Composer cards are fixed 208×128 chips with the capture contained
inside a 4 px inset, the app icon and "window title / app" on a bottom gradient,
and a corner remove button that brightens on hover.

## Platforms and permissions

- macOS uses ScreenCaptureKit with a bounded CoreGraphics fallback. Screen
  Recording permits pixels; optional Accessibility permission adds application
  text. The source accessibility window is retained before asynchronous capture.
- Linux supports X11 capture and compatible screenshot portals. Portal capture
  requires an advertised window target; cancellation ends the operation. Once a
  portal request starts, failure does not silently fall through to X11 capture.
  X11 text enrichment requires matching native window, process, title, and
  retained AT-SPI object identity. Portal captures omit accessibility text because
  the selected native window cannot currently be verified.
- iOS displays desktop Appshots received through the existing attachment
  transport. It does not capture other iOS applications. Windows capture is not
  implemented.

Optional accessibility collection has a 900 ms budget. Linux races enrichment
against the overall deadline; macOS applies the remaining timeout to each queried
AX element. If enrichment is unavailable, the screenshot remains usable.
Native fallback dimensions are checked before acquisition, actual dimensions
before encoding, and encoded bytes before copying into Rust.

## Presentation and delivery

Composer and transcript cards identify the application and window, show a
contained preview, and open the full image on activation. macOS uses the locally
installed application icon where available; other clients use a source icon.
Accessibility text is agent context, not visible message text.

Queued images use small cropped thumbnails. Narrow desktop windows show one
thumbnail with an additional-image count; desktop actions retain their fixed
slots when modifier hints appear. iOS uses one thumbnail, an image gallery, and
44-point edit/send/menu targets. Its queue scrolls after three rows.

Visible desktop queue thumbnails have their own bounded lifecycle, independent
of eviction from the full-image cache. Full images load when opened. Failed
previews require user activation to retry. iOS retains prepared thumbnails while
their rows are realized. Queue editing preserves the original Appshot context
and attachment references on both clients.

Capture and settings belong to the viewer's UI. Sending uses the existing
attachment upload/host acknowledgement flow and prompt context; no RPC or
persistent schema migration is introduced. Existing settings deserialize with
backward-compatible defaults. Appshots setup controls support Tab/Shift-Tab,
Enter/Space, accessible names and state, and visible keyboard focus.

## Verification and visual fixtures

The focused final suites passed on macOS (829 UI tests), native Linux (849 UI
tests), and iOS Simulator (107 unit tests plus one Appshots UI flow). App checks
passed on macOS and Linux. Native capture permission dialogs, real desktop
focus changes, interactive Linux portals, VoiceOver, and physical remote-device
delivery still require a live release pass; the fixtures do not establish them.

The opt-in `appshots-fixture` Rust example renders isolated native GPUI frames
from supplied PNG fixtures. Run it with an output directory and a directory
containing `wide.png`, `tall.png`, and `square.png` after building with
`cargo build -p zeron-ui --example appshots-fixture --features appshots-fixture`.
It uses a temporary data directory and an ephemeral IPC listener. iOS's
`-demo -appshots` fixture supplies neutral images; `AppshotUITests` verifies
portrait/landscape presentation, image opening, the gallery, and queue actions.
Neither fixture sends an agent prompt.

The initial whole-workspace review reported four failures, three reproduced on
an upstream baseline, plus existing upstream formatting differences. Focused
passing suites do not imply the global gate is clean. See the accompanying
review resolution and logs when preparing publication.
