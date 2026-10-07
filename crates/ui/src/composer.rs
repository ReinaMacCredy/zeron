//! The composer: a hand-rolled multiline text input (adapted from gpui's
//! `examples/input.rs`), the compact↔expanded flip, the Send/Queue/Stop morph,
//! optimistic send with failure recovery, per-chat drafts, and the question
//! wizard that replaces the composer while a run awaits input.
//!
//! Pure decision logic (flip, auto-grow math, button morph, wizard reducer,
//! pending-input detection) lives in free functions/structs with unit tests;
//! the gpui element only feeds them measurements.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyTooltip, App, BorderStyle, Bounds, ClipboardEntry, ClipboardItem, Context, CursorStyle,
    DispatchPhase, ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, GlobalElementId, Hsla, KeyBinding, KeyDownEvent, KeyUpEvent, Keystroke, LayoutId,
    ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    PaintQuad, PathPromptOptions, Pixels, Point, Role, ScrollWheelEvent, SharedString, Style,
    StyledImage as _, Subscription, Task, TextRun, TextStyle, UTF16Selection, UnderlineStyle,
    Window, WrappedLine, actions, div, fill, img, point, prelude::*, px, quad, relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use zeron_doc::{MessagePart, MessageRole, SessionCommandPayload, SessionMessageEntry};
use zeron_proto::{
    FileSearchMatch, HarnessId, RunRequest, SandboxLevel, SlashCommand, SlashCommandKind,
    UserInputAnswer, UserInputQuestion, capabilities,
};
use zeron_rpc::{RpcError, methods};

use crate::appshots::{self, CapturedAppshot};
use crate::attachments::{self, StagedAttachment};
use crate::composer_markdown::{self, in_code};
use crate::motion;
use crate::notice::{NoticeChipIcon, notice_chip};
use crate::pickers::Pickers;
use crate::settings::{ComposerSendBehavior, FollowUpBehavior, platform_combo};
use crate::state::{AppState, Indicator};
use crate::theme::Theme;

mod ask_pill;
pub(crate) mod background;
mod chip;
pub use chip::ChipKind;
use chip::*;
pub(crate) use chip::{
    CHIP_PAD_FAMILY, ChipIcon, chip_icon, chip_pad_ranges, chip_text, paint_chip,
    text_chip_label_offset,
};

// ---------------------------------------------------------------------------
// Constants + pure decision logic
// ---------------------------------------------------------------------------

/// Expanded-mode textarea vertical padding: `pt-4 pb-1` (zeron composer.tsx
/// line 578) = 16 + 4.
pub const TEXTAREA_PAD_V: f32 = 20.0;
/// The expanded textarea BOX (content + padding) is clamped by the original's
/// auto-grow effect: `ta.style.height = Math.min(Math.max(scrollHeight, 76),
/// 260)` (zeron composer.tsx line 235). The 76px floor applies even when
/// empty — it's what makes the always-expanded new-chat composer tall.
pub const TEXTAREA_MIN: f32 = 76.0;
pub const TEXTAREA_MAX: f32 = 260.0;
/// Expanded actions row: 2px top + 32px picker + 8px bottom.
const ACTIONS_BOTTOM_PAD: f32 = 8.0;
pub const ACTIONS_ROW_HEIGHT: f32 = 2.0 + 32.0 + ACTIONS_BOTTOM_PAD;
/// The pill's 1px hairline, top + bottom (`rounded-[26px] border`).
pub const PILL_BORDER_V: f32 = 2.0;
/// Corner radius shared by the composer and the queue tray behind it.
pub(crate) const COMPOSER_RADIUS: f32 = 26.0;
/// Expanded composer bounds: 120px when empty, 304px at the content cap.
pub const COMPOSER_MIN_HEIGHT: f32 = TEXTAREA_MIN + ACTIONS_ROW_HEIGHT + PILL_BORDER_V;
pub const COMPOSER_MAX_HEIGHT: f32 = TEXTAREA_MAX + ACTIONS_ROW_HEIGHT + PILL_BORDER_V;
/// Compact pill, border-box: one-line textarea `py-3` (24) + one 22.75px line
/// (scrollHeight rounds to 47 in the original) + the 2px hairline = 49. The
/// compact cluster (`py-1.5` + h-8 = 44) is shorter, so the textarea wins.
pub const COMPACT_TOTAL_HEIGHT: f32 = 49.0;
/// `max-w-3xl`: outer width of the new-chat composer, also used when no shell
/// width is supplied. Established threads follow the conversation column.
pub const COMPOSER_MAX_WIDTH: f32 = 768.0;
/// The queue reads as a narrower tray emerging from behind the composer.
const QUEUE_SIDE_INSET: f32 = 16.0;
/// The composer covers the tray's lower padding so the queue reads as emerging
/// from behind it instead of as a separate rounded pill.
pub(crate) const QUEUE_COMPOSER_OVERLAP: f32 = 18.0;
/// The original floating selector rows use the same 20px chip height as the
/// established-thread footer. Their surrounding rows own no plate or border.
const NEW_THREAD_SELECTOR_ROW_HEIGHT: f32 = 20.0;
const RUNNING_TAB_FADE: motion::MotionSpec = motion::MotionSpec::new(200, motion::EASE_OUT);
// Accommodate the 24px usage indicator and PR badge without overflowing the
// row's equal 8px top/bottom gutters.
pub(crate) const SESSION_FOOTER_HEIGHT: f32 = 24.0;

/// Route chrome dissolves around the middle of the shared-element move. The
/// two ramps never overlap, which avoids duplicate picker ids/popovers while
/// still letting their surrounding geometry collapse continuously.
fn route_chrome_opacities(new_thread_chrome: f32) -> (f32, f32) {
    let new_thread = ((new_thread_chrome.clamp(0.0, 1.0) - 0.5) * 2.0).clamp(0.0, 1.0);
    let session = (((1.0 - new_thread_chrome.clamp(0.0, 1.0)) - 0.5) * 2.0).clamp(0.0, 1.0);
    (new_thread, session)
}
/// Ignore subpixel noise when the shell reports the conversation width.
const COMPOSER_WIDTH_EPSILON: f32 = 0.5;
/// Below this pill input width the composer always expands.
pub const MIN_COMPACT_INPUT_WIDTH: f32 = 200.0;
/// Input text metrics: `text-[14px] leading-relaxed` = 14 × 1.625 = 22.75.
pub const INPUT_LINE_HEIGHT: f32 = 22.75;
pub const INPUT_TEXT_SIZE: f32 = 14.0;
/// A compact ramp; the glyph-ascent inset keeps the clip edge invisible.
const INPUT_FADE_BAND: f32 = 12.0;
/// Single-select questions auto-advance after this long.
pub const AUTO_ADVANCE_MS: u64 = 220;
/// Drag-selection autoscroll runs at the display-friendly 60fps cadence.
pub const DRAG_SCROLL_FRAME_MS: u64 = 16;

/// Hysteresis slack for the expanded→compact flip: once expanded, the composer
/// only collapses when the text is comfortably narrower than the compact
/// capacity — expanding and collapsing share no boundary, so a width right at
/// the flip threshold can't oscillate between the two layouts.
pub const COLLAPSE_HYSTERESIS: f32 = 32.0;
/// During an interactive resize, collapsing back to the compact mode waits
/// until the measured widths have been stable this long. Expansion remains
/// immediate so a narrowing panel never traps the controls in a compact row.
pub const RESIZE_SETTLE_MS: u64 = 150;

/// Compact↔expanded flip with hysteresis. `capacity` is the *compact-mode*
/// input capacity (a layout-stable width: measured while compact, tracked by
/// container-width deltas while expanded — never the post-flip measured width,
/// which differs per mode and would feed back into the decision):
/// - a newline always expands;
/// - while `resizing`, an expanded composer stays expanded until sizes settle;
/// - a too-narrow pill (`capacity < MIN_COMPACT_INPUT_WIDTH`) always expands;
/// - compact expands only when `text_width > capacity`; expanded collapses
///   only when `text_width < capacity - COLLAPSE_HYSTERESIS`.
pub fn composer_flip(
    expanded: bool,
    text_width: f32,
    capacity: f32,
    has_newline: bool,
    resizing: bool,
) -> bool {
    if has_newline {
        return true;
    }
    if capacity < MIN_COMPACT_INPUT_WIDTH {
        return true;
    }
    if expanded {
        resizing || text_width >= capacity - COLLAPSE_HYSTERESIS
    } else {
        text_width > capacity
    }
}

fn composer_width_changed(previous: Option<f32>, current: f32) -> bool {
    previous.is_none_or(|previous| (current - previous).abs() > COMPOSER_WIDTH_EPSILON)
}

/// Caret blink half-period (standard textarea cadence: ~500ms on / 500ms off).
pub const CARET_BLINK_MS: u64 = 500;

/// Caret blink phase for a time since the last keystroke/caret move: solid
/// through the first half-period (typing bursts never blink — each keystroke
/// resets the phase), then alternating.
pub fn caret_visible(ms_since_activity: u64) -> bool {
    (ms_since_activity / CARET_BLINK_MS) % 2 == 0
}

/// Auto-grow: content height for a wrapped-line count.
pub fn input_content_height(wrapped_lines: usize) -> f32 {
    wrapped_lines.max(1) as f32 * INPUT_LINE_HEIGHT
}

/// Total expanded composer height (border-box) for a content height: the
/// textarea BOX (content + `pt-4 pb-1`) clamps to 76–260 exactly like the
/// original's auto-grow effect, then the actions row and the hairline
/// ride on top. Range 120–304.
pub fn composer_total_height(content_height: f32) -> f32 {
    (content_height + TEXTAREA_PAD_V).clamp(TEXTAREA_MIN, TEXTAREA_MAX)
        + ACTIONS_ROW_HEIGHT
        + PILL_BORDER_V
}

/// The question card's free-text field grows with the note up to the same
/// content cap as the expanded composer, then scrolls.
fn wizard_input_height(content_height: f32) -> f32 {
    content_height.clamp(INPUT_LINE_HEIGHT, TEXTAREA_MAX - TEXTAREA_PAD_V)
}

fn input_max_scroll(content_height: f32, viewport_height: f32) -> f32 {
    (content_height - viewport_height).max(0.0)
}

/// Only settled overflow gets a scroll fade. The animated viewport can be
/// smaller for a few frames while an otherwise fitting draft grows into it.
fn input_overflow_edges(
    content_height: f32,
    settled_height: f32,
    visible_height: f32,
    scroll_top: f32,
) -> (bool, bool) {
    if input_max_scroll(content_height, settled_height) <= 1.0 {
        return (false, false);
    }
    let max_scroll = input_max_scroll(content_height, visible_height);
    (scroll_top > 1.0, scroll_top < max_scroll - 1.0)
}

/// During the reveal, stop at a complete row boundary instead of slicing
/// glyphs with a moving clip. Scrolling offsets the row grid inside the box.
fn input_reveal_height(visible: f32, scroll: f32, line_height: f32, resizing: bool) -> f32 {
    if !resizing {
        return visible;
    }
    let row_end = ((scroll + visible + 0.001) / line_height).floor() * line_height;
    (row_end - scroll).clamp(0.0, visible)
}

/// Apply GPUI's wheel delta to a top-origin input offset. Positive deltas mean
/// scrolling toward the start, matching gpui's built-in list/div behavior.
fn input_scroll_offset(
    current: f32,
    delta_y: f32,
    content_height: f32,
    viewport_height: f32,
) -> f32 {
    (current - delta_y).clamp(0.0, input_max_scroll(content_height, viewport_height))
}

/// Minimally adjust the viewport so the caret row is fully visible.
fn input_scroll_offset_for_cursor(
    current: f32,
    cursor_top: f32,
    cursor_height: f32,
    content_height: f32,
    viewport_height: f32,
    settled_height: Option<f32>,
) -> f32 {
    // Resize the reveal, not the scroll position: existing text stays fixed
    // relative to the input origin throughout the height animation.
    let viewport_height = settled_height.unwrap_or(viewport_height);
    let mut next = current;
    if cursor_top < next {
        next = cursor_top;
    } else if cursor_top + cursor_height > next + viewport_height {
        next = cursor_top + cursor_height - viewport_height;
    }
    next.clamp(0.0, input_max_scroll(content_height, viewport_height))
}

/// Pointer selection granularity; multi-click drags retain the initial unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PressIntent {
    Word,
    Line,
    ExtendSelection,
    PlaceCaret,
}

fn press_intent(click_count: usize, shift: bool) -> PressIntent {
    if click_count >= 3 {
        PressIntent::Line
    } else if click_count == 2 {
        PressIntent::Word
    } else if shift {
        PressIntent::ExtendSelection
    } else {
        PressIntent::PlaceCaret
    }
}

fn word_range(text: &str, offset: usize) -> Range<usize> {
    let offset = offset.min(text.len());
    text.split_word_bound_indices()
        .find(|(at, word)| *at <= offset && offset < at + word.len())
        .map(|(at, word)| at..at + word.len())
        .unwrap_or(offset..offset)
}

/// Per-frame drag-selection scroll. Distance increases speed, capped at one
/// text row per frame so crossing the input boundary never causes a jump.
fn input_drag_scroll_delta(
    pointer_y: f32,
    viewport_top: f32,
    viewport_bottom: f32,
    line_height: f32,
) -> f32 {
    let distance = if pointer_y < viewport_top {
        pointer_y - viewport_top
    } else if pointer_y > viewport_bottom {
        pointer_y - viewport_bottom
    } else {
        return 0.0;
    };
    distance.signum() * (distance.abs() * 0.2).clamp(1.0, line_height)
}

/// Staged-attachment strip metrics (zeron attachment-ui.tsx AttachmentStrip:
/// `flex flex-wrap gap-2 px-4 pt-3`, `size-14` thumbs).
pub const STRIP_THUMB: f32 = 56.0;
pub const STRIP_GAP: f32 = 8.0;
pub const STRIP_PAD_TOP: f32 = 12.0;
pub const STRIP_PAD_X: f32 = 16.0;

/// Height the wrap strip adds to the pill for `count` staged thumbnails at an
/// `inner_width` pill content width (0 when empty). Mirrors flex-wrap: as many
/// 56px thumbs per row as fit with 8px gaps inside the 16px side insets.
pub fn attachment_strip_height(count: usize, inner_width: f32) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let usable = (inner_width - 2.0 * STRIP_PAD_X).max(STRIP_THUMB);
    let per_row = (((usable + STRIP_GAP) / (STRIP_THUMB + STRIP_GAP)).floor() as usize).max(1);
    let rows = count.div_ceil(per_row);
    STRIP_PAD_TOP + rows as f32 * STRIP_THUMB + (rows - 1) as f32 * STRIP_GAP
}

pub fn comment_strip_height(count: usize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    STRIP_PAD_TOP + crate::badges::BADGE_HEIGHT
}

/// Compact↔expanded flip morph (round 9): the flip used to snap between the
/// two pill layouts. The original has no height transition (its shell carries
/// only `transition-colors`), so this is a native nicety: ONE committed flip
/// starts exactly one 180ms ease-out morph ([`motion::COLLAPSE`]); the blank-
/// thread handoff swaps in the coordinated 420ms route-transition spec. Both use the
/// manual-drive pattern from shell.rs `WidthTween` — never `with_animation`,
/// whose element-id keying replays tweens on remount, round-6 §1–3.
///
/// The morph animates the pill's COMMITTED height: the flip commits its final
/// layout immediately (the input entity never remounts — the caret survives,
/// exactly as before) while the pill clips toward the live target. The pill's
/// bottom edge is stationary on screen, so the controls stay pinned to it
/// (constant screen-y; see the anchoring helpers below) and only the text
/// glides with the sweeping top edge. [`composer_flip`]'s hysteresis already
/// guarantees no oscillation at the boundary, and [`flip_morph_step`] never
/// restarts a morph while the committed mode holds. Reduced motion snaps: no
/// morph is ever created.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlipMorph {
    /// Rendered height when the flip committed — the animation's start point.
    pub from: f32,
    /// Commit time in ms on the caller's monotonic clock.
    pub start_ms: f32,
    /// Ordinary typing flips use the quick collapse spec; the first-send
    /// handoff uses the shell's longer coordinated route timeline.
    pub spec: motion::MotionSpec,
}

impl FlipMorph {
    fn collapse(from: f32, start_ms: f32) -> Self {
        Self {
            from,
            start_ms,
            spec: motion::COLLAPSE,
        }
    }

    fn new_thread_transition(from: f32, start_ms: f32) -> Self {
        Self {
            from,
            start_ms,
            spec: motion::NEW_THREAD_TRANSITION,
        }
    }

    /// Raw timeline position 0..1 over this morph's motion spec.
    fn raw(&self, now_ms: f32) -> f32 {
        let total = self.spec.total().as_secs_f32() * 1000.0;
        ((now_ms - self.start_ms) / total).clamp(0.0, 1.0)
    }

    /// Eased progress 0..1 — also drives the inner geometry handoff.
    pub fn progress(&self, now_ms: f32) -> f32 {
        self.spec.progress(self.raw(now_ms))
    }

    pub fn done(&self, now_ms: f32) -> bool {
        self.raw(now_ms) >= 1.0
    }

    /// Committed-height evaluation: eased lerp from the flip-time height to
    /// the LIVE target (auto-grow may move the target mid-morph — the morph
    /// tracks it instead of finishing on a stale height).
    pub fn height(&self, target: f32, now_ms: f32) -> f32 {
        motion::lerp(self.from, target, self.progress(now_ms))
    }
}

// -- morph anchoring (round-9 follow-up) ------------------------------------
// The pill sits at the BOTTOM of the shell column: growing it moves its TOP
// edge; the bottom edge is stationary on screen. The first morph cut anchored
// the pill's inner content to the top, so the actions/cluster (laid out at
// the inner bottom) rode the animating height up and down. The controls are
// therefore pinned to the stationary bottom edge (absolute bottom row when
// expanded, a bottom-justified row when compact) and only the TEXT glides
// with the sweeping top edge. The helpers below are the pure math.

/// Send/attach center sits 25px above the expanded pill's bottom (8px
/// padding + half the 32px control zone + 1px border), versus 24.5px in
/// compact. The morph glides this optical adjustment instead of snapping.
pub const CLUSTER_Y_DELTA: f32 =
    ACTIONS_BOTTOM_PAD + 16.0 + PILL_BORDER_V / 2.0 - COMPACT_TOTAL_HEIGHT / 2.0;

/// Attachment and Send share an outer inset: compact 8px, expanded 12px.
/// Keep both action groups aligned with the changing surface inset.
pub const CLUSTER_X_DELTA: f32 = 4.0;
/// Optical join between the picker group and the paperclip. This is tighter
/// than the structural spacing ladder because the narrow paperclip glyph
/// otherwise looks farther away than its hit target actually is.
pub const ACTION_UTILITY_GAP: f32 = 2.0;
/// Structural separation between utility actions and the primary Send action.
pub const ACTION_PRIMARY_GAP: f32 = Theme::SPACE_SM;

/// The shared outer inset for the in-flight morph: eases from the OLD mode's resting
/// inset to the committed mode's (compact 8 ↔ expanded 12).
pub fn morph_cluster_inset(expanded: bool, progress: f32) -> f32 {
    let (from, to) = if expanded {
        (8.0, 8.0 + CLUSTER_X_DELTA)
    } else {
        (8.0 + CLUSTER_X_DELTA, 8.0)
    };
    motion::lerp(from, to, progress)
}

/// Expanded text top padding across the morph: starts at the compact resting
/// inset (12 ≈ `py-3`) and eases to `pt-4` (16) — the first line glides with
/// the rising top edge instead of jumping at the commit.
pub fn morph_text_pad(progress: f32) -> f32 {
    motion::lerp(12.0, 16.0, progress)
}

/// Collapse-morph text glide: the committed compact row is bottom-anchored
/// (text resting top = 36px above the pill's outer bottom: 49 − 1 hairline −
/// 12 centering inset), while at the commit instant the text sat 17px below
/// the expanded pill's top (1 hairline + 16 `pt-4`) — i.e. `from − 17` above
/// the bottom. The decaying relative offset walks it down smoothly.
pub fn collapse_text_glide(from: f32, progress: f32) -> f32 {
    (from - 53.0).max(0.0) * (1.0 - progress)
}

/// The decaying [`CLUSTER_Y_DELTA`] offset for the in-flight morph.
/// Controls share this bottom anchor; the model's horizontal fade is applied
/// independently so its endpoint matches Attachment and Send.
pub fn morph_cluster_dy(progress: f32) -> f32 {
    CLUSTER_Y_DELTA * (1.0 - progress)
}

/// Session/route changes SNAP the composer (same rule as the header inset
/// tween, round 6: route swaps remount in the original — zero motion). The
/// nav-driven flip doesn't commit on the first render after a switch (the
/// draft swap has to be laid out and re-measured first), so a plain reset at
/// the nav instant leaks: `last_rendered_height` is repopulated before the
/// flip lands and the session change morphs 49↔124. Instead, every flip
/// committed within this wall-clock window of a navigation snaps. User-driven
/// flips need typing and can't land this fast after a switch.
pub const ROUTE_SNAP_MS: u64 = 250;

/// Advance the flip morph across one render pass. While the committed mode
/// holds, the morph is kept (a finished one clears) — same-mode renders can
/// NEVER restart the animation. A committed mode change starts one morph from
/// the last rendered height, which mid-flight is the CURRENT animated height,
/// so a reverse flip hands off seamlessly instead of popping to an endpoint.
/// Reduced motion (or a first paint with no measured height yet) snaps, and
/// `route_snap` (a session/route change within [`ROUTE_SNAP_MS`]) both blocks
/// arming AND kills anything in flight — navigation never animates the pill.
pub fn flip_morph_step(
    morph: Option<FlipMorph>,
    mode_changed: bool,
    last_height: f32,
    now_ms: f32,
    reduced_motion: bool,
    route_snap: bool,
) -> Option<FlipMorph> {
    if route_snap || reduced_motion {
        return None;
    }
    if !mode_changed {
        return morph.filter(|m| !m.done(now_ms));
    }
    if reduced_motion || last_height <= 0.0 {
        return None;
    }
    Some(FlipMorph::collapse(last_height, now_ms))
}

/// Engines at or above this version understand `pending://` attachment refs
/// and QueueCommand `transfers` (send-is-a-local-write attachments). Gated on
/// BOTH the local engine (an IPC daemon may be older than this UI) and, for
/// remotely-hosted chats, the host device's stamped registry version.
const QUEUED_ATTACHMENTS_MIN: (u64, u64, u64) = (0, 2, 12);

/// What the send button is right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendButtonMode {
    /// No live run: plain send.
    Send,
    /// Live run with text typed: queue for the next turn.
    Queue,
    /// Live run, nothing typed: red stop square.
    Stop,
    /// Stopped turn, nothing typed: play triangle that resumes it.
    Resume,
}

/// Per-chat bookkeeping for attachment chips. Numbers only grow within a draft,
/// so a removed attachment never renumbers or retargets an earlier mention.
#[derive(Default)]
struct AttachmentDraft {
    /// The highest number handed out so far.
    next: u32,
    /// Attachments whose last chip or tile was removed. Undo that brings their
    /// chip back restages the attachment from here.
    unstaged: Vec<StagedAttachment>,
}

/// A staged attachment offered by the `@` list.
struct AttachmentRow {
    index: u32,
    label: String,
    /// The picture, for an image.
    image: Option<std::sync::Arc<gpui::Image>>,
}

/// What the composer holds that a send could carry. A staged image or diff
/// comment counts: both synthesize their own prompt body, so either alone is
/// a legal send — and during a live run has to read as Queue, not Stop.
pub fn composer_has_content(text: &str, attachments: usize, comments: usize) -> bool {
    !text.trim().is_empty() || attachments > 0 || comments > 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModifiedSubmitTarget {
    SubmitContent,
    ActivateLatestQueued,
}

fn modified_submit_target(has_content: bool) -> ModifiedSubmitTarget {
    if has_content {
        ModifiedSubmitTarget::SubmitContent
    } else {
        ModifiedSubmitTarget::ActivateLatestQueued
    }
}

/// Synara's AppSnap chip: a fixed `w-52 h-32` card, the capture contained
/// inside a 4px inset, provenance on a bottom gradient.
pub const APPSHOT_TILE_WIDTH: f32 = 208.0;
pub const APPSHOT_TILE_HEIGHT: f32 = 128.0;
pub const APPSHOT_IMAGE_INSET: f32 = 4.0;

struct AppshotActionTooltip(SharedString);

impl Render for AppshotActionTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.surface_raised)
            .shadow_md()
            .text_size(px(11.0))
            .text_color(theme.text)
            .child(self.0.clone())
    }
}

/// Contain the capture inside the chip's inset box without cropping. Explicit
/// dimensions also bound the native image while decoding.
pub fn appshot_contained_size(dimensions: Option<(u32, u32)>) -> (f32, f32) {
    let box_width = APPSHOT_TILE_WIDTH - 2.0 * APPSHOT_IMAGE_INSET;
    let box_height = APPSHOT_TILE_HEIGHT - 2.0 * APPSHOT_IMAGE_INSET;
    let (width, height) = dimensions
        .filter(|(width, height)| *width > 0 && *height > 0)
        .unwrap_or((16, 10));
    let scale = (box_width / width as f32).min(box_height / height as f32);
    (width as f32 * scale, height as f32 * scale)
}

/// The chip's provenance line: the window title, then the app, unless the
/// title merely echoes the app's name ("ChatGPT / ChatGPT").
pub fn appshot_provenance(app_name: &str, window_title: Option<&str>) -> String {
    let app = app_name.trim();
    let app = if app.is_empty() { "Captured app" } else { app };
    match window_title
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        Some(title) if !title.eq_ignore_ascii_case(app) => format!("{title} / {app}"),
        _ => app.to_string(),
    }
}

pub fn appshot_strip_height(count: usize) -> f32 {
    if count == 0 {
        0.0
    } else {
        STRIP_PAD_TOP + APPSHOT_TILE_HEIGHT
    }
}

pub fn send_button_mode(run_live: bool, has_text: bool, stopped: bool) -> SendButtonMode {
    match (run_live, has_text) {
        (false, false) if stopped => SendButtonMode::Resume,
        (false, _) => SendButtonMode::Send,
        (true, true) => SendButtonMode::Queue,
        (true, false) => SendButtonMode::Stop,
    }
}

/// Whether a busy-turn submit asks the host to steer: the setting, flipped
/// for one message when the modifier chord submitted it.
pub fn follow_up_steers(behavior: FollowUpBehavior, inverted: bool) -> bool {
    (behavior == FollowUpBehavior::Steer) != inverted
}

/// The modifier chord only inverts the follow-up setting while plain Enter
/// is the send key; once the chord itself sends, it has nothing to invert.
pub fn modified_submit_inverts_follow_up(send_behavior: ComposerSendBehavior) -> bool {
    send_behavior == ComposerSendBehavior::Enter
}

/// Queue rows are represented by the queue panel until the host promotes them
/// into the transcript. They must never publish (or refresh) a local echo.
fn should_publish_optimistic_echo(queue: bool) -> bool {
    !queue
}

fn begin_interrupt(pending: &mut HashSet<String>, chat_id: &str) -> bool {
    pending.insert(chat_id.to_string())
}

fn retain_live_interrupts(pending: &mut HashSet<String>, mut is_live: impl FnMut(&str) -> bool) {
    pending.retain(|chat_id| is_live(chat_id));
}

fn interrupt_params(chat_id: &str) -> serde_json::Value {
    serde_json::json!({
        "chatId": chat_id,
        "command": { "kind": "interrupt" },
    })
}

fn escape_dismisses_completion(key: &str, completion_open: bool) -> bool {
    key == "escape" && completion_open
}

fn wizard_escape_goes_back(key: &str, input_focused: bool, input_empty: bool) -> bool {
    key == "escape" && (!input_focused || input_empty)
}

/// Find the unresolved input request the panel should serve, if any: an
/// unresolved input part on the LAST assistant entry — regardless of the
/// entry's run status. The question stays answerable until the user actually
/// answers it (user requirement): a run that died under its question (engine
/// restart reaping it) leaves an aborted entry whose answer the engine
/// delivers as a resumed turn (`RespondInput`'s dead-run fallback). A newer
/// assistant entry supersedes an unanswered question. Assistant-entry-scoped,
/// not last-entry: a steer prompt sent while the agent waits appends a USER
/// entry after the streaming assistant entry, and a last-entry-only read made
/// the QuestionPanel vanish exactly when the user typed (earlier forensics;
/// matches the original composer.tsx, which reads the live-assistant fold —
/// rebuilt from replay even after the run died).
pub fn pending_input_request(
    transcript: &[SessionMessageEntry],
) -> Option<(String, Vec<UserInputQuestion>)> {
    transcript
        .iter()
        .rev()
        .find(|entry| entry.role == MessageRole::Assistant)
        .and_then(|entry| {
            entry.parts.iter().find_map(|part| match part {
                MessagePart::Input {
                    request_id,
                    questions,
                    resolved: false,
                    ..
                } if !questions.is_empty()
                    && !questions.iter().any(UserInputQuestion::is_computer) =>
                {
                    Some((request_id.clone(), questions.clone()))
                }
                _ => None,
            })
        })
}

pub(crate) async fn respond_input(
    engine: &crate::state::EngineHandle,
    chat_id: &str,
    request_id: &str,
    answers: Vec<UserInputAnswer>,
) -> Result<(), String> {
    let command = SessionCommandPayload::RespondInput {
        request_id: request_id.to_owned(),
        answers,
    };
    let command = serde_json::to_value(&command).map_err(|err| err.to_string())?;
    engine
        .client()
        .call(
            methods::QUEUE_COMMAND,
            serde_json::json!({ "chatId": chat_id, "command": command }),
        )
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
}

/// Whether the transcript shows `request_id` explicitly resolved (here or on
/// another device) — the wizard latch's release condition.
pub fn input_request_resolved(transcript: &[SessionMessageEntry], request_id: &str) -> bool {
    transcript.iter().any(|entry| {
        entry.parts.iter().any(|part| {
            matches!(
                part,
                MessagePart::Input {
                    request_id: rid,
                    resolved: true,
                    ..
                } if rid == request_id
            )
        })
    })
}

// ---------------------------------------------------------------------------
// Question wizard (pure reducer)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WizardHost {
    Own,
    Ask,
}

impl WizardHost {
    fn hover_prefix(self) -> &'static str {
        match self {
            WizardHost::Own => "wizard-option",
            WizardHost::Ask => "ask-option",
        }
    }
}

/// Reducer outcome of a wizard interaction.
#[derive(Debug, Clone, PartialEq)]
pub enum WizardStep {
    Stay,
    /// Single-select landed — advance after [`AUTO_ADVANCE_MS`].
    AutoAdvance,
    /// All pages answered — submit these answers.
    Done(Vec<UserInputAnswer>),
}

/// Paged question state ("1/3"): single-select auto-advances, multi-select and
/// typed answers advance explicitly, number keys 1-9 select, Back pages back.
#[derive(Debug, Clone)]
pub struct Wizard {
    pub request_id: String,
    pub questions: Vec<UserInputQuestion>,
    pub page: usize,
    picked: Vec<Vec<usize>>,
    typed: Vec<String>,
}

impl Wizard {
    pub fn new(request_id: String, questions: Vec<UserInputQuestion>) -> Self {
        let n = questions.len();
        let typed = questions
            .iter()
            .map(|q| q.prefill.clone().unwrap_or_default())
            .collect();
        Self {
            request_id,
            questions,
            page: 0,
            picked: vec![Vec::new(); n],
            typed,
        }
    }

    pub fn counter(&self) -> String {
        format!("{}/{}", self.page + 1, self.questions.len().max(1))
    }

    pub fn current(&self) -> Option<&UserInputQuestion> {
        self.questions.get(self.page)
    }

    pub fn is_picked(&self, option_ix: usize) -> bool {
        self.picked
            .get(self.page)
            .is_some_and(|p| p.contains(&option_ix))
    }

    /// Whether the current page has any picked option.
    pub fn page_has_pick(&self) -> bool {
        self.picked.get(self.page).is_some_and(|p| !p.is_empty())
    }

    /// Click/tap an option.
    pub fn select(&mut self, option_ix: usize) -> WizardStep {
        let Some(question) = self.questions.get(self.page) else {
            return WizardStep::Stay;
        };
        if option_ix >= question.options.len() {
            return WizardStep::Stay;
        }
        let multi = question.multi_select;
        let Some(picked) = self.picked.get_mut(self.page) else {
            return WizardStep::Stay;
        };
        if multi {
            match picked.iter().position(|&p| p == option_ix) {
                Some(at) => {
                    picked.remove(at);
                }
                None => picked.push(option_ix),
            }
            WizardStep::Stay
        } else {
            *picked = vec![option_ix];
            WizardStep::AutoAdvance
        }
    }

    /// Number key 1-9.
    pub fn press_number(&mut self, number: usize) -> WizardStep {
        if number == 0 {
            return WizardStep::Stay;
        }
        self.select(number - 1)
    }

    pub fn set_typed(&mut self, text: String) {
        if let Some(slot) = self.typed.get_mut(self.page) {
            *slot = text;
        }
    }

    /// Explicit submit / auto-advance landing.
    pub fn advance(&mut self) -> WizardStep {
        if self.page + 1 < self.questions.len() {
            self.page += 1;
            WizardStep::Stay
        } else {
            WizardStep::Done(self.answers())
        }
    }

    /// Page back; false when already on the first page.
    pub fn back(&mut self) -> bool {
        if self.page > 0 {
            self.page -= 1;
            true
        } else {
            false
        }
    }

    /// Answers per question: free text overrides picked labels.
    pub fn answers(&self) -> Vec<UserInputAnswer> {
        self.questions
            .iter()
            .enumerate()
            .map(|(ix, q)| {
                let typed = self
                    .typed
                    .get(ix)
                    .map(|s| if q.multiline { s.as_str() } else { s.trim() })
                    .unwrap_or("");
                let labels = if !typed.is_empty() || q.multiline {
                    vec![typed.to_string()]
                } else {
                    self.picked
                        .get(ix)
                        .map(|picked| {
                            picked
                                .iter()
                                .filter_map(|&p| q.options.get(p).cloned())
                                .collect()
                        })
                        .unwrap_or_default()
                };
                UserInputAnswer {
                    question_id: q.id.clone(),
                    labels,
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Multiline text input (adapted from gpui examples/input.rs)
// ---------------------------------------------------------------------------

actions!(
    composer,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        Home,
        End,
        SelectHome,
        SelectEnd,
        DocStart,
        DocEnd,
        SelectDocStart,
        SelectDocEnd,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        DeleteWordLeft,
        DeleteWordRight,
        DeleteToLineStart,
        DeleteToLineEnd,
        Copy,
        Cut,
        Paste,
        Newline,
        MessageNewlineOrAccept,
        ModifiedSubmit,
        Submit,
        Undo,
        Redo,
        MentionTab,
        ToggleDictation,
        OutdentList,
    ]
);

/// How long a run of single-character edits keeps merging into one undo step.
/// A pause longer than this starts a fresh step, so undo rewinds in the
/// bursts the user actually typed rather than one character at a time.
const UNDO_COALESCE: Duration = Duration::from_millis(700);

/// Cap on retained undo steps — a long-lived composer must not grow forever.
const UNDO_LIMIT: usize = 200;

const MENTION_TOOLTIP_DELAY: Duration = Duration::from_millis(420);
const MENTION_TOOLTIP_HEIGHT: f32 = 24.0;
// Nonbreaking spaces give chips their side inset, shaped in Geist like the
// rest of the padding, while preserving the chip's atomic wrapping and
// source/caret projection.
const MENTION_SIDE_PAD: &str = "\u{00A0}";
/// A private URI scheme keeps file mentions distinguishable from ordinary
/// Markdown links pasted into the composer.
use zeron_proto::file_mentions::{FILE_MENTION_SCHEME, local_file_link, local_path_is_safe};
/// A thread mention names another chat by id; the agent reads it through
/// the `zeron` MCP server's `read_thread`, so the prompt carries the id
/// verbatim and the label is only the title at insert time.
const THREAD_MENTION_SCHEME: &str = "zeron-thread:";
const THREAD_ICON_SIZE: f32 = 14.0;

/// The mark painted in a thread chip's icon slot: the thread's current
/// harness brand, or a plain chat glyph when the chat is not in this
/// device's list (another device, archived, gone).
pub(crate) fn thread_chip_icon(chat: Option<&zeron_proto::Chat>) -> (&'static str, Option<Hsla>) {
    chat.and_then(|chat| crate::pickers::chat_harness_marks(chat).last().copied())
        .map(crate::pickers::harness_brand_icon)
        .unwrap_or((crate::icons::CHAT_ROUND_LINE, None))
}

/// The display sub-range a thread chip reserves for its mark, given the
/// chip's full projected range.
pub(crate) fn thread_icon_slot(chip: &Range<usize>) -> Range<usize> {
    let start = chip.start + MENTION_SIDE_PAD.len();
    start..start + CHIP_ICON_SLOT.len()
}

/// Paint a thread chip's mark at the head of its slot rect, vertically
/// centered on the line.
pub(crate) fn paint_thread_icon(
    slot: Bounds<Pixels>,
    icon: (&'static str, Option<Hsla>),
    fallback: Hsla,
    window: &mut Window,
    cx: &App,
) {
    let size = px(THREAD_ICON_SIZE);
    let origin = point(slot.left(), slot.top() + (slot.size.height - size) / 2.0);
    let _ = window.paint_svg(
        Bounds::new(origin, gpui::size(size, size)),
        SharedString::from(icon.0),
        None,
        gpui::TransformationMatrix::unit(),
        icon.1.unwrap_or(fallback),
        cx,
    );
}

/// A restorable point in the input's history: text plus where the caret and
/// selection sat when the edit landed.
#[derive(Clone)]
struct EditSnapshot {
    content: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
    caret_affinity: CaretAffinity,
}

/// A strict, local-only Markdown representation of a file mention. The
/// underlying prompt always contains this form; the editor projects it to a
/// chip for display without leaking a second data model into submission.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileMentionLink {
    range: Range<usize>,
    basename: String,
    path: String,
    kind: ChipKind,
    /// The draft number when this chip references a staged attachment.
    attachment: Option<u32>,
}

impl FileMentionLink {
    /// The character that marks this kind of reference.
    fn prefix(&self) -> char {
        match self.kind {
            ChipKind::Skill => '$',
            ChipKind::Command => '/',
            ChipKind::Thread => '#',
            _ => '@',
        }
    }

    fn is_dir(&self) -> bool {
        self.kind == ChipKind::Directory
    }
}

impl FileMentionLink {
    fn is_thread(&self) -> bool {
        self.kind == ChipKind::Thread
    }

    fn is_invocation(&self) -> bool {
        matches!(self.kind, ChipKind::Skill | ChipKind::Command)
    }

    fn in_accent_flow(&self) -> bool {
        self.kind.in_accent_flow()
    }
}

fn escape_mention_label(label: &str) -> String {
    label
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

fn label_close(text: &str, start: usize) -> Option<usize> {
    let mut escaped = false;
    for (at, ch) in text[start..].char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == ']' && text[start + at + 1..].starts_with('(') {
            return Some(start + at);
        }
    }
    None
}

fn unescape_mention_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn chat_id_is_safe(chat_id: &str) -> bool {
    !chat_id.is_empty()
        && chat_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
}

fn thread_link(chat_id: &str, title: &str) -> String {
    format!(
        "[{}]({THREAD_MENTION_SCHEME}{chat_id})",
        escape_mention_label(title)
    )
}

/// Build the text inserted when a workspace item is dropped at an arbitrary
/// selection. Unlike completion, a drop does not necessarily happen at a
/// token boundary, so it supplies its own leading separator when needed.
/// A reference must not introduce whitespace before closing Markdown syntax.
/// Existing horizontal whitespace is reused; line endings stay on their row.
fn reference_suffix(next: Option<char>) -> (&'static str, usize) {
    match next {
        Some('\n' | '\r') => ("", 0),
        Some(ch) if ch.is_whitespace() => ("", ch.len_utf8()),
        Some(')' | ']' | '}' | '*' | '_' | '~' | ',' | '.' | ';' | ':' | '!' | '?') => ("", 0),
        _ => (" ", 0),
    }
}

/// Place a reference link at an arbitrary selection, supplying the separators
/// a completion would already have had around it.
fn dropped_reference(content: &str, range: Range<usize>, link: &str) -> Option<(String, usize)> {
    if range.start > range.end || !content.is_char_boundary(range.start) {
        return None;
    }
    let suffix = content.get(range.end..)?;
    let prefix = if range.start > 0
        && content[..range.start]
            .chars()
            .next_back()
            .is_some_and(|ch| {
                !ch.is_whitespace() && !matches!(ch, '(' | '[' | '{' | '*' | '_' | '~')
            }) {
        " "
    } else {
        ""
    };
    let (trailing, advance) = reference_suffix(suffix.chars().next());
    let inserted = format!("{prefix}{link}{trailing}");
    let cursor_advance = inserted.len() + advance;
    Some((inserted, cursor_advance))
}

fn file_mention_links(text: &str) -> Vec<FileMentionLink> {
    zeron_proto::file_mentions::file_mention_links(text)
        .into_iter()
        .map(|link| FileMentionLink {
            range: link.range,
            basename: link.basename,
            path: link.path,
            kind: if link.is_dir {
                ChipKind::Directory
            } else {
                ChipKind::File
            },
            attachment: None,
        })
        .collect()
}

fn thread_mention_links(text: &str) -> Vec<FileMentionLink> {
    if !text.contains(THREAD_MENTION_SCHEME) {
        return Vec::new();
    }
    let mut links = Vec::new();
    let mut search = 0;
    while let Some(relative_start) = text[search..].find('[') {
        let start = search + relative_start;
        let Some(label_end) = label_close(text, start + 1) else {
            search = start + 1;
            continue;
        };
        let target_start = label_end + 2;
        let Some(relative_end) = text[target_start..].find(')') else {
            search = start + 1;
            continue;
        };
        let end = target_start + relative_end + 1;
        let label = &text[start + 1..label_end];
        if let Some(chat_id) = text[target_start..end - 1].strip_prefix(THREAD_MENTION_SCHEME) {
            let title = unescape_mention_label(label);
            if chat_id_is_safe(chat_id)
                && !title.trim().is_empty()
                && escape_mention_label(&title) == label
            {
                links.push(FileMentionLink {
                    range: start..end,
                    basename: title,
                    path: chat_id.to_string(),
                    kind: ChipKind::Thread,
                    attachment: None,
                });
            }
        }
        search = end;
    }
    links
}

#[derive(Debug, Clone, Default)]
struct TextProjection {
    display: String,
    mentions: Vec<(FileMentionLink, Range<usize>)>,
    mappings: Vec<(Range<usize>, Range<usize>)>,
}

/// A path alone is not enough: two identical relative paths can appear in a
/// draft, so the raw range remains part of the hover identity.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MentionTooltipTarget {
    range: Range<usize>,
    path: SharedString,
    /// Set for attachment chips. An image chip has no tooltip: a click opens
    /// the picture full size instead.
    attachment: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MentionTooltipPhase {
    Hidden,
    Waiting {
        target: MentionTooltipTarget,
        generation: u64,
    },
    Visible {
        target: MentionTooltipTarget,
        generation: u64,
    },
}

impl MentionTooltipPhase {
    fn target(&self) -> Option<&MentionTooltipTarget> {
        match self {
            Self::Hidden => None,
            Self::Waiting { target, .. } | Self::Visible { target, .. } => Some(target),
        }
    }
}

/// Pure tooltip lifecycle reducer. Motion within the same chip preserves both
/// waiting and visible phases, so normal pointer jitter cannot starve the
/// delay or flicker an already-visible tooltip.
fn mention_tooltip_reduce(
    phase: MentionTooltipPhase,
    pointer_target: Option<MentionTooltipTarget>,
    pointer_in_popup: bool,
    generation: u64,
) -> MentionTooltipPhase {
    match pointer_target {
        Some(target) if phase.target() == Some(&target) => phase,
        Some(target) => MentionTooltipPhase::Waiting { target, generation },
        None if pointer_in_popup && matches!(phase, MentionTooltipPhase::Visible { .. }) => phase,
        None => MentionTooltipPhase::Hidden,
    }
}

fn mention_tooltip_promote(
    phase: MentionTooltipPhase,
    generation: u64,
    target_is_live: bool,
) -> MentionTooltipPhase {
    match phase {
        MentionTooltipPhase::Waiting {
            target,
            generation: current,
        } if current == generation && target_is_live => MentionTooltipPhase::Visible {
            target,
            generation: current,
        },
        MentionTooltipPhase::Waiting {
            generation: current,
            ..
        } if current == generation => MentionTooltipPhase::Hidden,
        phase => phase,
    }
}

fn mention_tooltip_contains(in_chip: bool, in_popup: bool) -> bool {
    in_chip || in_popup
}

fn display_row_segments(
    range: Range<usize>,
    row_ends: impl IntoIterator<Item = usize>,
) -> Vec<(usize, usize, Range<usize>)> {
    let mut segments = Vec::new();
    let mut row_start = 0usize;
    for (row_ix, row_end) in row_ends.into_iter().enumerate() {
        let start = range.start.max(row_start);
        let end = range.end.min(row_end);
        if start < end {
            segments.push((row_ix, row_start, start..end));
        }
        row_start = row_end;
        if row_start >= range.end {
            break;
        }
    }
    segments
}

#[derive(Debug, Clone)]
struct MentionHit {
    target: MentionTooltipTarget,
    bounds: Bounds<Pixels>,
    anchor: Point<Pixels>,
}

/// A chip is a compact identity, with the complete path available in its tooltip.
/// Truncate by grapheme so emoji and combining marks remain intact.
fn compact_chip_label(label: &str) -> String {
    compact_chip_label_with_limit(label, 32)
}

fn compact_chip_label_with_limit(label: &str, limit: usize) -> String {
    let graphemes: Vec<_> = label.graphemes(true).collect();
    if graphemes.len() <= limit {
        return label.to_string();
    }
    format!(
        "{}…{}",
        graphemes[..limit - 14].concat(),
        graphemes[graphemes.len() - 12..].concat()
    )
}

/// Preserve distinctions established by path disambiguation when shortening.
fn compact_chip_labels(labels: &[String]) -> Vec<String> {
    let mut shortened: Vec<_> = labels
        .iter()
        .map(|label| compact_chip_label(label))
        .collect();
    let mut limit = 32usize;
    loop {
        let mut distinct: HashMap<&str, std::collections::HashSet<&str>> = HashMap::new();
        for (shown, full) in shortened.iter().zip(labels) {
            distinct.entry(shown).or_default().insert(full);
        }
        let ambiguous: Vec<_> = shortened
            .iter()
            .enumerate()
            .filter_map(|(ix, shown)| (distinct[shown.as_str()].len() > 1).then_some(ix))
            .collect();
        if ambiguous.is_empty() {
            return shortened;
        }
        limit = limit.saturating_mul(2);
        for ix in ambiguous {
            shortened[ix] = compact_chip_label_with_limit(&labels[ix], limit);
        }
    }
}

/// GPUI's text wrapper permits breaks at Unicode spacer glyphs. Reference
/// chips need stronger boundaries than ordinary prose, without changing ZUI.
fn wrap_reference_chips(line: &mut WrappedLine, chips: &[Range<usize>], width: Pixels) {
    if chips.is_empty() {
        return;
    }
    let glyphs: Vec<_> = line
        .runs()
        .iter()
        .enumerate()
        .flat_map(|(run_ix, run)| {
            run.glyphs.iter().enumerate().map(move |(glyph_ix, glyph)| {
                (
                    glyph.index,
                    glyph.position.x,
                    gpui::WrapBoundary { run_ix, glyph_ix },
                )
            })
        })
        .collect();
    let inside = |index| {
        chips
            .iter()
            .any(|range| range.start < index && index < range.end)
    };
    // Keep complete references together; oversized references can still wrap.
    let mut boundaries = line.wrap_boundaries.clone();
    boundaries.clear();
    let mut row = 0;
    while row < glyphs.len() {
        let limit = glyphs[row].1 + width;
        let overflow = (row..glyphs.len()).find(|&i| {
            glyphs
                .get(i + 1)
                .map_or(line.unwrapped_layout.width, |g| g.1)
                > limit
        });
        let Some(overflow) = overflow else {
            break;
        };
        // Prefer whitespace or a complete chip boundary. If an individual
        // token is wider than the viewport, retain the usual emergency wrap.
        let preferred = (row + 1..=overflow).rev().find(|&i| {
            let index = glyphs[i].0;
            !inside(index)
                && (chips.iter().any(|r| r.start == index || r.end == index)
                    || line.text[..index]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_whitespace))
        });
        let at = preferred
            .or_else(|| (row + 1..=overflow).rev().find(|&i| !inside(glyphs[i].0)))
            .unwrap_or(overflow.max(row + 1));
        if at >= glyphs.len() {
            break;
        }
        boundaries.push(glyphs[at].2);
        row = at;
    }
    let layout = gpui::WrappedLineLayout {
        unwrapped_layout: line.unwrapped_layout.clone(),
        wrap_boundaries: boundaries,
        wrap_width: line.wrap_width,
    };
    *std::ops::DerefMut::deref_mut(line) = std::sync::Arc::new(layout);
}

impl TextProjection {
    fn new(raw: &str) -> Self {
        Self::project(raw, None, false, None)
    }

    /// `live` names the attachments the draft still holds; a chip outside it is
    /// a dangling reference and shows as its plain label.
    fn rich(raw: &str, active: Option<Range<usize>>, live: Option<&HashSet<u32>>) -> Self {
        Self::project(raw, active, true, live)
    }

    fn project(
        raw: &str,
        active: Option<Range<usize>>,
        compact: bool,
        live: Option<&HashSet<u32>>,
    ) -> Self {
        let mut links = file_mention_links(raw);
        links.extend(thread_mention_links(raw));
        links.extend(
            zeron_proto::invocation::invocation_links(raw)
                .into_iter()
                .map(|(range, invocation)| FileMentionLink {
                    range,
                    basename: if compact && invocation.prefix() == '$' {
                        skill_display_name(invocation.name())
                    } else {
                        invocation.name().to_string()
                    },
                    path: invocation.detail(),
                    kind: if invocation.prefix() == '/' {
                        ChipKind::Command
                    } else {
                        ChipKind::Skill
                    },
                    attachment: None,
                }),
        );
        links.extend(
            zeron_proto::attachment_mentions::attachment_mentions(raw)
                .into_iter()
                .map(|mention| FileMentionLink {
                    range: mention.range,
                    // A file attachment's chip names the file, so its icon
                    // follows the name; an image has no path to follow.
                    path: if mention.is_image {
                        String::new()
                    } else {
                        mention.label.clone()
                    },
                    basename: mention.label,
                    kind: if mention.is_image {
                        ChipKind::Image
                    } else {
                        ChipKind::File
                    },
                    attachment: Some(mention.index),
                }),
        );
        links.sort_by_key(|link| link.range.start);
        let labels = mention_display_labels(&links);
        let labels = if compact {
            compact_chip_labels(&labels)
        } else {
            labels
        };
        let mut projection = Self::default();
        let mut edits: Vec<(Range<usize>, String, Option<FileMentionLink>)> = links
            .into_iter()
            .zip(labels)
            .map(|(link, label)| {
                let label = label.replace(' ', "\u{00A0}");
                if let (Some(index), Some(live)) = (link.attachment, live)
                    && !live.contains(&index)
                {
                    return (link.range.clone(), link.basename.clone(), None);
                }
                let pad = MENTION_SIDE_PAD;
                let display = if link.is_thread() {
                    format!("{pad}{CHIP_ICON_SLOT}{label}{pad}")
                } else if link.is_invocation() {
                    format!("{pad}{}{label}{pad}", link.prefix())
                } else {
                    format!("{pad}{CHIP_ICON_SLOT}{label}{CHIP_TRAILING_PAD}")
                };
                (link.range.clone(), display, Some(link))
            })
            .collect();
        if let Some(active) = active {
            let links: Vec<_> = edits.iter().map(|(range, _, _)| range.clone()).collect();
            for (range, replacement) in composer_markdown::decorations(raw, active) {
                let candidate = links.partition_point(|link| link.end <= range.start);
                if !links
                    .get(candidate)
                    .is_some_and(|link| link.start < range.end)
                {
                    edits.push((range, replacement, None));
                }
            }
        }
        edits.sort_by_key(|(r, _, _)| r.start);
        let mut raw_at = 0;
        for (range, replacement, link) in edits {
            if range.start < raw_at {
                continue;
            }
            projection.display.push_str(&raw[raw_at..range.start]);
            let start = projection.display.len();
            projection.display.push_str(&replacement);
            let display = start..projection.display.len();
            if let Some(link) = link {
                projection.mentions.push((link, display.clone()));
            }
            raw_at = range.end;
            projection.mappings.push((range, display));
        }
        projection.display.push_str(&raw[raw_at..]);
        projection
    }

    fn raw_to_display(&self, raw: usize) -> usize {
        let ix = self.mappings.partition_point(|(range, _)| range.end <= raw);
        if let Some((range, display)) = self.mappings.get(ix) {
            if raw > range.start {
                return display.start;
            }
        }
        let (raw_at, display_at) = ix
            .checked_sub(1)
            .map(|previous| (&self.mappings[previous].0, &self.mappings[previous].1))
            .map_or((0, 0), |(range, display)| (range.end, display.end));
        display_at + raw.saturating_sub(raw_at)
    }

    fn display_to_raw(&self, display_offset: usize) -> usize {
        // Equal zero-width boundaries are consumed together so clicks on the
        // first rendered character land after nested hidden opening markers.
        let ix = self
            .mappings
            .partition_point(|(_, display)| display.end <= display_offset);
        if let Some((range, display)) = self.mappings.get(ix) {
            if display_offset > display.start {
                return if display_offset - display.start < display.len() / 2 {
                    range.start
                } else {
                    range.end
                };
            }
        }
        let (raw_at, display_at) = ix
            .checked_sub(1)
            .map(|previous| (&self.mappings[previous].0, &self.mappings[previous].1))
            .map_or((0, 0), |(range, display)| (range.end, display.end));
        raw_at + display_offset.saturating_sub(display_at)
    }

    fn normalize_range(&self, range: Range<usize>) -> Range<usize> {
        if range.is_empty() {
            for (link, _) in &self.mentions {
                if link.range.start < range.start && range.start < link.range.end {
                    let midpoint = link.range.start + link.range.len() / 2;
                    let at = if range.start < midpoint {
                        link.range.start
                    } else {
                        link.range.end
                    };
                    return at..at;
                }
            }
            return range;
        }
        let mut normalized = range;
        for (link, _) in &self.mentions {
            if normalized.start < link.range.end && normalized.end > link.range.start {
                normalized.start = normalized.start.min(link.range.start);
                normalized.end = normalized.end.max(link.range.end);
            }
        }
        normalized
    }

    fn previous_boundary(&self, raw: usize) -> Option<usize> {
        self.mentions
            .iter()
            .find_map(|(link, _)| (raw == link.range.end).then_some(link.range.start))
    }

    fn next_boundary(&self, raw: usize) -> Option<usize> {
        self.mentions
            .iter()
            .find_map(|(link, _)| (raw == link.range.start).then_some(link.range.end))
    }
}

/// Basenames are compact in the common case. When the same basename appears
/// more than once, use the shortest unique path suffix so chips remain
/// distinguishable without always expanding to full paths.
fn mention_display_labels(links: &[FileMentionLink]) -> Vec<String> {
    // Only references with the same visible name can need disambiguation.
    // Repeated references share the result instead of rescanning the draft.
    let mut groups: HashMap<(char, &str), Vec<&FileMentionLink>> = HashMap::new();
    for link in links {
        groups
            .entry((link.prefix(), &link.basename))
            .or_default()
            .push(link);
    }
    let mut labels: HashMap<(char, &str, &str), String> = HashMap::new();
    links
        .iter()
        .map(|link| {
            if link.is_thread() {
                return link.basename.clone();
            }
            labels
                .entry((link.prefix(), &link.basename, &link.path))
                .or_insert_with(|| {
                    let duplicates: Vec<_> = groups[&(link.prefix(), link.basename.as_str())]
                        .iter()
                        .filter(|other| other.path != link.path)
                        .collect();
                    if duplicates.is_empty() {
                        return link.basename.clone();
                    }
                    let parts: Vec<_> = link.path.split('/').collect();
                    let suffix = (1..=parts.len())
                        .map(|count| parts[parts.len() - count..].join("/"))
                        .find(|suffix| {
                            if link.prefix() != '@' {
                                duplicates.iter().all(|other| !other.path.ends_with(suffix))
                            } else {
                                let suffix: Vec<_> = suffix.split('/').collect();
                                duplicates.iter().all(|other| {
                                    !other
                                        .path
                                        .split('/')
                                        .rev()
                                        .take(suffix.len())
                                        .eq(suffix.iter().rev().copied())
                                })
                            }
                        })
                        .unwrap_or_else(|| link.path.clone());
                    if link.prefix() == '@' {
                        suffix
                    } else {
                        format!("{} · {suffix}", link.basename)
                    }
                })
                .clone()
        })
        .collect()
}

/// One chip in a *sent* message: its byte range over the projected display
/// string (`@label` between side bearings). The transcript renders these
/// read-only — no editing state, no tooltip machinery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentMentionSpan {
    pub range: Range<usize>,
    /// Full workspace-relative path (labels can be shortened to basenames).
    pub path: SharedString,
    pub kind: ChipKind,
    /// The draft number of an attachment chip.
    pub attachment: Option<u32>,
    /// The upload an attachment chip names, once the transcript pairs them.
    pub upload: Option<SharedString>,
}

/// Cheap probe: whether `raw` could hold a file, thread, skill or attachment chip.
fn has_mention_scheme(raw: &str) -> bool {
    raw.contains(FILE_MENTION_SCHEME)
        || raw.contains(THREAD_MENTION_SCHEME)
        || raw.contains(zeron_proto::invocation::INVOCATION_SCHEME)
        || raw.contains(zeron_proto::attachment_mentions::IMAGE_MENTION_SCHEME)
        || raw.contains(zeron_proto::attachment_mentions::ATTACHMENT_MENTION_SCHEME)
}

/// Project a sent message's raw Markdown for transcript display: mention links
/// collapse to the same chip labels the composer shows, everything else passes
/// through untouched. `None` when the text has no valid mention — the
/// substring probe keeps ordinary prompts on the zero-allocation path, so this
/// is safe to call for every user row.
pub fn sent_mention_display(raw: &str) -> Option<(String, Vec<SentMentionSpan>)> {
    if !has_mention_scheme(raw) {
        return None;
    }
    let projection = TextProjection::new(raw);
    if projection.mentions.is_empty() {
        return None;
    }
    let spans = projection
        .mentions
        .iter()
        .map(|(link, display)| SentMentionSpan {
            range: display.clone(),
            path: SharedString::from(format!(
                "{}{}",
                link.path,
                if link.is_dir() { "/" } else { "" }
            )),
            kind: link.kind,
            attachment: link.attachment,
            upload: None,
        })
        .collect();
    Some((projection.display, spans))
}

/// Direction of the last edit — a run only merges with edits of its own kind.
#[derive(Clone, Copy, PartialEq)]
enum EditKind {
    Insert,
    Delete,
}

const GENERIC_COMPOSER_CONTEXT: &str = "Composer";
const MESSAGE_COMPOSER_CONTEXT: &str = "MessageComposer";
const PALETTE_SEARCH_CONTEXT: &str = "PaletteSearch";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageEnterBindingAction {
    Submit,
    ModifiedSubmit,
    NewlineOrAccept,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MessageEnterBinding {
    keystroke: String,
    action: MessageEnterBindingAction,
}

fn message_enter_bindings(
    behavior: ComposerSendBehavior,
    modifier_combo: &str,
) -> Vec<MessageEnterBinding> {
    match behavior {
        ComposerSendBehavior::Enter => vec![
            MessageEnterBinding {
                keystroke: "enter".into(),
                action: MessageEnterBindingAction::Submit,
            },
            MessageEnterBinding {
                keystroke: modifier_combo.into(),
                action: MessageEnterBindingAction::ModifiedSubmit,
            },
        ],
        ComposerSendBehavior::ModEnter => vec![
            MessageEnterBinding {
                keystroke: "enter".into(),
                action: MessageEnterBindingAction::NewlineOrAccept,
            },
            MessageEnterBinding {
                keystroke: modifier_combo.into(),
                action: MessageEnterBindingAction::ModifiedSubmit,
            },
        ],
    }
}

fn message_input_context(wizard_active: bool) -> &'static str {
    if wizard_active {
        GENERIC_COMPOSER_CONTEXT
    } else {
        MESSAGE_COMPOSER_CONTEXT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnterOutcome {
    AcceptCompletion,
    Submit,
    Newline,
}

fn enter_outcome(has_completion: bool, fallback: EnterOutcome) -> EnterOutcome {
    if has_completion {
        EnterOutcome::AcceptCompletion
    } else {
        fallback
    }
}

fn input_bindings(context: &'static str) -> Vec<KeyBinding> {
    let ctx = Some(context);
    let mut bindings = vec![
        KeyBinding::new("tab", MentionTab, ctx),
        KeyBinding::new("shift-tab", OutdentList, ctx),
        KeyBinding::new("shift-enter", Newline, ctx),
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("shift-backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("shift-up", SelectUp, ctx),
        KeyBinding::new("shift-down", SelectDown, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("shift-home", SelectHome, ctx),
        KeyBinding::new("shift-end", SelectEnd, ctx),
        // macOS line/document motion — a laptop keyboard has no home/end keys,
        // so Cmd+arrow is the only way users reach either edge.
        KeyBinding::new("cmd-left", Home, ctx),
        KeyBinding::new("cmd-right", End, ctx),
        KeyBinding::new("cmd-up", DocStart, ctx),
        KeyBinding::new("cmd-down", DocEnd, ctx),
        KeyBinding::new("shift-cmd-left", SelectHome, ctx),
        KeyBinding::new("shift-cmd-right", SelectEnd, ctx),
        KeyBinding::new("shift-cmd-up", SelectDocStart, ctx),
        KeyBinding::new("shift-cmd-down", SelectDocEnd, ctx),
        // Line-edge deletion (Cmd+Delete on macOS).
        KeyBinding::new("cmd-backspace", DeleteToLineStart, ctx),
        KeyBinding::new("cmd-delete", DeleteToLineEnd, ctx),
    ];
    for prefix in ["cmd", "ctrl"] {
        bindings.push(KeyBinding::new(&format!("{prefix}-z"), Undo, ctx));
        bindings.push(KeyBinding::new(&format!("shift-{prefix}-z"), Redo, ctx));
    }
    // Word-level editing: Option on macOS, Ctrl on Windows/Linux.
    let word_edit_prefix = if cfg!(target_os = "macos") {
        "alt"
    } else {
        "ctrl"
    };
    bindings.push(KeyBinding::new(
        &format!("{word_edit_prefix}-backspace"),
        DeleteWordLeft,
        ctx,
    ));
    bindings.push(KeyBinding::new(
        &format!("{word_edit_prefix}-delete"),
        DeleteWordRight,
        ctx,
    ));
    bindings.push(KeyBinding::new(
        &format!("{word_edit_prefix}-left"),
        WordLeft,
        ctx,
    ));
    bindings.push(KeyBinding::new(
        &format!("{word_edit_prefix}-right"),
        WordRight,
        ctx,
    ));
    bindings.push(KeyBinding::new(
        &format!("shift-{word_edit_prefix}-left"),
        SelectWordLeft,
        ctx,
    ));
    bindings.push(KeyBinding::new(
        &format!("shift-{word_edit_prefix}-right"),
        SelectWordRight,
        ctx,
    ));
    for prefix in ["cmd", "ctrl"] {
        bindings.push(KeyBinding::new(&format!("{prefix}-a"), SelectAll, ctx));
        bindings.push(KeyBinding::new(&format!("{prefix}-c"), Copy, ctx));
        bindings.push(KeyBinding::new(&format!("{prefix}-x"), Cut, ctx));
        bindings.push(KeyBinding::new(&format!("{prefix}-v"), Paste, ctx));
    }
    bindings
}

/// Bind the composer keymap. Call once at app boot.
pub fn init(cx: &mut App, send_behavior: ComposerSendBehavior) {
    let mut generic_bindings = input_bindings(GENERIC_COMPOSER_CONTEXT);
    generic_bindings.push(KeyBinding::new(
        "enter",
        Submit,
        Some(GENERIC_COMPOSER_CONTEXT),
    ));

    let mut message_bindings = input_bindings(MESSAGE_COMPOSER_CONTEXT);
    for binding in message_enter_bindings(send_behavior, &platform_combo("mod-enter")) {
        match binding.action {
            MessageEnterBindingAction::Submit => message_bindings.push(KeyBinding::new(
                &binding.keystroke,
                Submit,
                Some(MESSAGE_COMPOSER_CONTEXT),
            )),
            MessageEnterBindingAction::ModifiedSubmit => message_bindings.push(KeyBinding::new(
                &binding.keystroke,
                ModifiedSubmit,
                Some(MESSAGE_COMPOSER_CONTEXT),
            )),
            MessageEnterBindingAction::NewlineOrAccept => message_bindings.push(KeyBinding::new(
                &binding.keystroke,
                MessageNewlineOrAccept,
                Some(MESSAGE_COMPOSER_CONTEXT),
            )),
        }
    }

    let word_edit_prefix = if cfg!(target_os = "macos") {
        "alt"
    } else {
        "ctrl"
    };
    // Palette-search context: TEXT-EDITING keys only. gpui dispatches matched
    // keybindings BEFORE raw key listeners (window.rs `dispatch_key_event`),
    // so anything bound here can never reach a palette's `on_key_down` —
    // navigation keys (up/down/left/right/enter) are deliberately unbound and
    // bubble to the palette frame instead.
    let palette = Some(PALETTE_SEARCH_CONTEXT);
    let mut palette_bindings = vec![
        KeyBinding::new("backspace", Backspace, palette),
        KeyBinding::new("shift-backspace", Backspace, palette),
        KeyBinding::new("delete", Delete, palette),
        KeyBinding::new("home", Home, palette),
        KeyBinding::new("end", End, palette),
        KeyBinding::new("shift-left", SelectLeft, palette),
        KeyBinding::new("shift-right", SelectRight, palette),
        // Modifier-qualified motion is safe here: the palette's own navigation
        // uses BARE arrows/enter, which stay unbound and bubble to its frame.
        KeyBinding::new("cmd-left", Home, palette),
        KeyBinding::new("cmd-right", End, palette),
        KeyBinding::new("shift-cmd-left", SelectHome, palette),
        KeyBinding::new("shift-cmd-right", SelectEnd, palette),
        KeyBinding::new("cmd-backspace", DeleteToLineStart, palette),
    ];
    palette_bindings.push(KeyBinding::new(
        &format!("{word_edit_prefix}-backspace"),
        DeleteWordLeft,
        palette,
    ));
    palette_bindings.push(KeyBinding::new(
        &format!("{word_edit_prefix}-delete"),
        DeleteWordRight,
        palette,
    ));
    palette_bindings.push(KeyBinding::new(
        &format!("{word_edit_prefix}-left"),
        WordLeft,
        palette,
    ));
    palette_bindings.push(KeyBinding::new(
        &format!("{word_edit_prefix}-right"),
        WordRight,
        palette,
    ));
    palette_bindings.push(KeyBinding::new(
        &format!("shift-{word_edit_prefix}-left"),
        SelectWordLeft,
        palette,
    ));
    palette_bindings.push(KeyBinding::new(
        &format!("shift-{word_edit_prefix}-right"),
        SelectWordRight,
        palette,
    ));
    for prefix in ["cmd", "ctrl"] {
        palette_bindings.push(KeyBinding::new(&format!("{prefix}-a"), SelectAll, palette));
        palette_bindings.push(KeyBinding::new(&format!("{prefix}-c"), Copy, palette));
        palette_bindings.push(KeyBinding::new(&format!("{prefix}-x"), Cut, palette));
        palette_bindings.push(KeyBinding::new(&format!("{prefix}-v"), Paste, palette));
        palette_bindings.push(KeyBinding::new(&format!("{prefix}-z"), Undo, palette));
        palette_bindings.push(KeyBinding::new(&format!("shift-{prefix}-z"), Redo, palette));
    }
    cx.bind_keys(palette_bindings);
    cx.bind_keys(generic_bindings);
    cx.bind_keys(message_bindings);
}

/// Events the composer wrapper listens for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerInputEvent {
    Submitted,
    ModifiedSubmitted,
    Edited,
    CursorMoved,
    ViewportChanged,
    MentionNavigate(isize),
    MentionAccept,
    MentionDismiss,
    HistoryNavigate(isize),
    /// Images pasted from the clipboard (screenshots / copied image data) —
    /// the wrapper stages them as attachments (use-attachments.ts onPaste).
    PastedImages(Vec<gpui::Image>),
    /// File paths pasted from the clipboard (a file manager "Copy").
    PastedPaths(Vec<PathBuf>),
    PastedText {
        range: Range<usize>,
        revision: u64,
    },
    /// An image chip was clicked: open its picture full size.
    OpenAttachment(u32),
}

#[derive(Clone)]
enum DictationInputEvent {
    /// The dictation shortcut went down (auto-repeat is ignored)…
    Press,
    /// …and came back up, or focus left the editor while it was held.
    Release,
    Changed,
    Submit(u64),
}

/// A held dictation shortcut. `keystroke` is the binding, used to recognise
/// its release: the key coming up, or any of its modifiers being let go
/// (macOS does not deliver key-up for Command chords).
struct DictationKey {
    keystroke: Option<Keystroke>,
    _blur: Subscription,
}

/// Dictation is hold to talk, from the microphone (pointer or Enter/Space
/// while it is focused) and from the shortcut. Each releases only its own hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoldSource {
    Pointer,
    Button,
    Key,
}

struct DictationHold {
    source: HoldSource,
    /// When this hold started dictation; `None` when it took over a session
    /// started another way (assistive Click), which it then only finishes.
    started: Option<Instant>,
}

/// A release sooner than this is a click, not speech: explain hold to talk
/// instead of transcribing a fraction of a second of audio.
const DICTATION_TAP: Duration = Duration::from_millis(300);
impl EventEmitter<DictationInputEvent> for ComposerInput {}

/// The composer's morph into and out of dictation, and the clock's warning
/// window before the recording limit.
const VOICE_MORPH: Duration = Duration::from_millis(420);
const VOICE_CURVE: motion::CubicBezier = motion::CubicBezier::new(0.2, 0.0, 0.0, 1.0);
const VOICE_LIMIT_WARNING: Duration = Duration::from_secs(10);
/// Height of the voice track and its gap to the Stop control.
const VOICE_TRACK_HEIGHT: f32 = 32.0;
const VOICE_TRACK_GAP: f32 = 8.0;

/// Everything the voice track draws, captured from the dictation meter.
#[derive(Clone)]
struct VoiceFrame {
    label: String,
    mode: crate::dictation::waveform::Mode,
    bars: Vec<crate::dictation::Bar>,
    level: f32,
    elapsed: Option<Duration>,
}

/// Interruptible 0↔1 progress of the dictation morph: a retarget starts from
/// the current value, so toggling mid-flight reverses without a jump.
#[derive(Clone, Copy)]
struct VoiceTween {
    from: f32,
    to: f32,
    start: Instant,
}

impl Default for VoiceTween {
    fn default() -> Self {
        Self {
            from: 0.0,
            to: 0.0,
            start: Instant::now(),
        }
    }
}

impl VoiceTween {
    fn value(&self, now: Instant) -> f32 {
        let raw = now.saturating_duration_since(self.start).as_secs_f32()
            / (VOICE_MORPH.as_secs_f32() * motion::speed_scale());
        motion::lerp(self.from, self.to, VOICE_CURVE.eval(raw))
    }

    fn retarget(&mut self, to: f32, now: Instant, reduced: bool) {
        if self.to == to {
            return;
        }
        self.from = if reduced { to } else { self.value(now) };
        self.to = to;
        self.start = now;
    }

    fn settled(&self, now: Instant) -> bool {
        self.value(now) == self.to
    }
}

/// Shaping inputs excluding mutable viewport and selection geometry.
#[derive(Clone, PartialEq)]
struct InputLayoutKey {
    width: Pixels,
    font: gpui::Font,
    font_size: Pixels,
    color: gpui::Hsla,
    chip_family: SharedString,
    chip_color: gpui::Hsla,
    syntax: crate::theme::SyntaxPalette,
    marked_range: Option<Range<usize>>,
    placeholder: SharedString,
    mentions_enabled: bool,
    active_line: Option<Range<usize>>,
}

/// A soft-wrap boundary is both the previous row's end and the next row's
/// start. Keep the visual side of the caret separately from its source offset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CaretAffinity {
    Upstream,
    #[default]
    Downstream,
}

/// Multiline input entity: content + selection + IME marked text + measured
/// layout (wrapped lines) for mouse mapping and auto-grow.
pub struct ComposerInput {
    /// Key context for the binding map ("Composer", or "PaletteSearch" for
    /// palette filters whose navigation keys must bubble).
    key_context: &'static str,
    /// Cmd+C with no input selection copies the transcript selection. Only
    /// the message composer keeps focus while the user reads the transcript;
    /// dialog and palette fields copy only their own selection. A flag rather
    /// than `key_context`, which the question wizard swaps while it borrows
    /// the message input.
    copies_transcript_selection: bool,
    accessibility_role: Role,
    focus_handle: FocusHandle,
    content: String,
    edit_revision: u64,
    pub(crate) read_only: bool,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    caret_affinity: CaretAffinity,
    marked_range: Option<Range<usize>>,
    /// Retained through vertical moves across short rows; other moves reset it.
    preferred_column: Option<Pixels>,
    is_selecting: bool,
    drag_position: Option<Point<Pixels>>,
    drag_generation: u64,
    drag_unit: Option<(PressIntent, Range<usize>)>,
    drag_autoscroll_active: bool,
    /// Vertical scroll inside the input once content exceeds the max height.
    scroll_top: f32,
    /// Visible content budget supplied by the animated composer.
    viewport_height: Option<f32>,
    /// Final content budget, excluding temporary overflow during a resize.
    settled_viewport_height: Option<f32>,
    resizing: bool,
    overflow_top_padding: f32,
    needs_measure: bool,
    last_layout_key: Option<InputLayoutKey>,
    last_notified_layout: Option<(Pixels, f32)>,
    max_ascent: f32,
    #[cfg(test)]
    layout_rebuilds: usize,
    /// The runs of the last rebuilt layout; the test text system shapes
    /// every run alike, so tests check the requested fonts here.
    #[cfg(test)]
    last_runs: Vec<TextRun>,
    /// Normally keeps the caret visible through edits and rewraps. Manual
    /// wheel scrolling pauses it until the next caret move or edit.
    follow_cursor: bool,
    text_size: f32,
    configured_line_height: f32,
    single_line: bool,
    scroll_left: f32,
    // -- measured state (written during layout/paint) --
    last_lines: Vec<WrappedLine>,
    line_starts: Vec<usize>,
    line_indents: Vec<Pixels>,
    last_bounds: Option<Bounds<Pixels>>,
    line_height: Pixels,
    content_height: f32,
    max_line_width: f32,
    last_width: f32,
    /// Raw Markdown → chip display projection from the last layout pass.
    projection: TextProjection,
    syntax_source: String,
    syntax_spans: Vec<zeron_syntax::HighlightSpan>,
    syntax_task: Option<Task<()>>,
    /// Inline completion preview: painted in faint ink after the text while
    /// the caret sits at the end (palette tab-completion). Owned by the
    /// wrapper — it recomputes and re-sets this on every render pass, so the
    /// input never has to know what the completion means.
    ghost: Option<SharedString>,
    /// Where thread chips look up their brand mark; only the message
    /// composer sets it.
    thread_state: Option<Entity<AppState>>,
    /// File mentions are a composer feature, not a behavior of generic inputs
    /// (picker searches and rename fields also use this type).
    mentions_enabled: bool,
    /// Bumped once per `layout_text` pass — the flip logic uses it to apply at
    /// most one compact↔expanded flip per layout (a flip is only re-evaluated
    /// after the input has been measured in the new mode).
    layout_epoch: u64,
    display_is_placeholder: bool,
    /// Caret blink anchor: reset on every keystroke/caret move so the caret is
    /// solid while typing and blinks at [`CARET_BLINK_MS`] when idle.
    blink_anchor: Instant,
    /// Half-period repaint driver, alive only while the input is focused.
    blink_task: Option<Task<()>>,
    dictation: crate::dictation::Dictation,
    transcriber: Option<Box<dyn crate::dictation::Transcriber>>,
    dictation_task: Option<Task<()>>,
    /// The dictation shortcut while it is held down.
    dictation_key: Option<DictationKey>,
    // -- undo history --
    undo_stack: Vec<EditSnapshot>,
    redo_stack: Vec<EditSnapshot>,
    /// Kind, trailing offset, and time of the last edit — the merge test that
    /// decides whether the next edit extends the current undo step.
    last_edit: Option<(EditKind, usize, Instant)>,
    /// The wrapper owns mention state; this only redirects bound keys while a
    /// mention token is active, keeping input focus and native text editing.
    mention_open: bool,
    mention_has_selection: bool,
    prompt_history: bool,
    /// Last prepainted chip bounds; the paint-phase pointer listener uses
    /// these instead of attempting to infer text geometry from the cursor.
    mention_hits: Vec<MentionHit>,
    /// The image chip a click started on, and where.
    chip_press: Option<(u32, Point<Pixels>)>,
    /// The attachments the draft holds, by chip number, with each image's
    /// picture. `None` outside the composer, where every attachment link is
    /// shown as a chip.
    attachment_chips: Option<HashMap<u32, ChipAttachment>>,
    /// The draft the attachment chips belong to; copied chips carry it so a paste
    /// into another draft can tell it must not reuse their numbers.
    attachment_scope: String,
    mention_tooltip: MentionTooltipPhase,
    mention_tooltip_generation: u64,
    mention_tooltip_popup: Option<Bounds<Pixels>>,
    mention_tooltip_task: Option<Task<()>>,
    /// Created once when Waiting promotes; retaining this entity preserves
    /// GPUI's global animation state across prepaint frames.
    mention_tooltip_view: Option<Entity<MentionPathTooltip>>,
}

impl ComposerInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self::with_context(placeholder, GENERIC_COMPOSER_CONTEXT, cx)
    }

    /// An input in a custom KEY context — palettes use `"PaletteSearch"`,
    /// whose keymap binds only text-editing keys so navigation keys bubble to
    /// the surrounding frame (see `init`).
    pub fn with_context(
        placeholder: impl Into<SharedString>,
        key_context: &'static str,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            key_context,
            copies_transcript_selection: false,
            accessibility_role: Role::MultilineTextInput,
            focus_handle: cx.focus_handle(),
            content: String::new(),
            edit_revision: 0,
            read_only: false,
            placeholder: placeholder.into(),
            selected_range: 0..0,
            selection_reversed: false,
            caret_affinity: CaretAffinity::Downstream,
            marked_range: None,
            preferred_column: None,
            is_selecting: false,
            drag_position: None,
            drag_generation: 0,
            drag_unit: None,
            drag_autoscroll_active: false,
            scroll_top: 0.0,
            viewport_height: None,
            settled_viewport_height: None,
            resizing: false,
            overflow_top_padding: 0.0,
            needs_measure: true,
            last_layout_key: None,
            last_notified_layout: None,
            max_ascent: INPUT_TEXT_SIZE,
            #[cfg(test)]
            layout_rebuilds: 0,
            #[cfg(test)]
            last_runs: Vec::new(),
            follow_cursor: true,
            text_size: INPUT_TEXT_SIZE,
            configured_line_height: INPUT_LINE_HEIGHT,
            single_line: false,
            scroll_left: 0.0,
            last_lines: Vec::new(),
            line_starts: vec![0],
            line_indents: Vec::new(),
            last_bounds: None,
            line_height: px(INPUT_LINE_HEIGHT),
            content_height: INPUT_LINE_HEIGHT,
            max_line_width: 0.0,
            last_width: 0.0,
            projection: TextProjection::default(),
            syntax_source: String::new(),
            syntax_spans: Vec::new(),
            syntax_task: None,
            ghost: None,
            mentions_enabled: false,
            thread_state: None,
            layout_epoch: 0,
            display_is_placeholder: true,
            blink_anchor: Instant::now(),
            blink_task: None,
            dictation: Default::default(),
            transcriber: None,
            dictation_task: None,
            dictation_key: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit: None,
            mention_open: false,
            mention_has_selection: false,
            prompt_history: false,
            mention_hits: Vec::new(),
            chip_press: None,
            attachment_chips: None,
            attachment_scope: String::new(),
            mention_tooltip: MentionTooltipPhase::Hidden,
            mention_tooltip_generation: 0,
            mention_tooltip_popup: None,
            mention_tooltip_task: None,
            mention_tooltip_view: None,
        }
    }

    /// Override the text metrics for compact one-line surfaces such as
    /// toolbar searches without changing the main composer typography.
    pub fn with_text_metrics(mut self, text_size: f32, line_height: f32) -> Self {
        self.text_size = text_size;
        self.configured_line_height = line_height;
        self.line_height = px(line_height);
        self.content_height = line_height;
        self
    }

    /// Keep compact fields on one row and reveal the caret horizontally.
    pub fn with_single_line(mut self) -> Self {
        self.single_line = true;
        self
    }

    fn set_key_context(&mut self, key_context: &'static str, cx: &mut Context<Self>) {
        if self.key_context != key_context {
            self.key_context = key_context;
            cx.notify();
        }
    }

    pub fn with_accessibility_role(mut self, role: Role) -> Self {
        self.accessibility_role = role;
        self
    }

    /// Reset the caret blink phase (solid again) — called on every edit and
    /// caret move, matching textarea behavior.
    fn reset_blink(&mut self) {
        self.blink_anchor = Instant::now();
    }

    /// Caret paint gate: focused input in an active window, in the "on" blink
    /// phase. Also (re)arms the half-period repaint driver while focused, and
    /// drops it on blur so an unfocused input schedules no frames.
    fn caret_shown(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let focused = self.focus_handle.is_focused(window);
        if !focused || !window.is_window_active() {
            self.blink_task = None;
            return false;
        }
        if self.blink_task.is_none() {
            self.blink_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(CARET_BLINK_MS))
                        .await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        }
        caret_visible(self.blink_anchor.elapsed().as_millis() as u64)
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn set_mention_controls(
        &mut self,
        open: bool,
        has_selection: bool,
        cx: &mut Context<Self>,
    ) {
        if self.mention_open == open && self.mention_has_selection == has_selection {
            return;
        }
        self.mention_open = open;
        self.mention_has_selection = has_selection;
        cx.notify();
    }

    fn enable_mentions(&mut self) {
        self.mentions_enabled = true;
        self.refresh_projection();
    }

    fn editing_source_range(&self) -> Range<usize> {
        let selected = self.selected_range.clone();
        let range = self
            .marked_range
            .as_ref()
            .map_or(selected.clone(), |marked| {
                selected.start.min(marked.start)..selected.end.max(marked.end)
            });
        self.line_range_at(range.start).start..self.line_range_at(range.end).end
    }

    fn refresh_projection(&mut self) {
        self.projection = if self.mentions_enabled {
            let live: Option<HashSet<u32>> = self
                .attachment_chips
                .as_ref()
                .map(|images| images.keys().copied().collect());
            TextProjection::rich(
                &self.content,
                Some(self.editing_source_range()),
                live.as_ref(),
            )
        } else {
            TextProjection {
                display: self.content.clone(),
                mentions: Vec::new(),
                mappings: Vec::new(),
            }
        };
    }

    /// Replace a completed `@query` token as one non-coalescing undo step.
    pub fn replace_mention(
        &mut self,
        range: Range<usize>,
        path: &str,
        is_dir: bool,
        cx: &mut Context<Self>,
    ) {
        if !local_path_is_safe(path) {
            return;
        }
        self.replace_mention_link(range, &local_file_link(path, is_dir), cx);
    }

    /// Replace a completed `@query` token with a canonical reference link.
    pub(crate) fn replace_mention_link(
        &mut self,
        range: Range<usize>,
        link: &str,
        cx: &mut Context<Self>,
    ) {
        if self.read_only || self.marked_range.is_some() {
            return;
        }
        self.invalidate_mention_tooltip();
        let (trailing, advance) = reference_suffix(self.content[range.end..].chars().next());
        let inserted = format!("{link}{trailing}");
        self.record_edit(&range, &inserted);
        self.edit_revision = self.edit_revision.wrapping_add(1);
        self.content =
            self.content[..range.start].to_owned() + &inserted + &self.content[range.end..];
        let cursor = range.start + inserted.len() + advance;
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.preferred_column = None;
        self.refresh_projection();
        self.follow_cursor = true;
        self.reset_blink();
        self.needs_measure = true;
        cx.emit(ComposerInputEvent::Edited);
        cx.notify();
    }

    /// Insert a workspace reference at the current selection. Drag-and-drop
    /// uses the same strict local Markdown transport and projected chip as an
    /// `@` mention selected from completion.
    fn insert_dropped_mention(&mut self, path: &str, is_dir: bool, cx: &mut Context<Self>) -> bool {
        if !local_path_is_safe(path) {
            return false;
        }
        self.insert_reference(&local_file_link(path, is_dir), cx)
    }

    /// Delete every chip that mentions attachment `index`, along with one separator
    /// when the chip sat between spaces. Returns whether anything was removed.
    pub(crate) fn remove_attachment_chips(&mut self, index: u32, cx: &mut Context<Self>) -> bool {
        if self.read_only || self.marked_range.is_some() {
            return false;
        }
        let ranges: Vec<Range<usize>> =
            zeron_proto::attachment_mentions::attachment_mentions(&self.content)
                .into_iter()
                .filter(|mention| mention.index == index)
                .map(|mention| mention.range)
                .collect();
        if ranges.is_empty() {
            return false;
        }
        self.invalidate_mention_tooltip();
        for mut range in ranges.into_iter().rev() {
            let separated =
                range.start == 0 || self.content[..range.start].ends_with(char::is_whitespace);
            if separated && self.content[range.end..].starts_with(' ') {
                range.end += 1;
            }
            self.record_edit(&range, "");
            self.edit_revision = self.edit_revision.wrapping_add(1);
            self.content.replace_range(range.clone(), "");
            let shift = |offset: usize| {
                if offset >= range.end {
                    offset - range.len()
                } else {
                    offset.min(range.start)
                }
            };
            self.selected_range = shift(self.selected_range.start)..shift(self.selected_range.end);
        }
        self.last_edit = None;
        self.selection_reversed = false;
        self.preferred_column = None;
        self.refresh_projection();
        self.reset_blink();
        self.needs_measure = true;
        cx.emit(ComposerInputEvent::Edited);
        cx.notify();
        true
    }

    /// Tell the editor which staged attachments its chips may reference (`None`
    /// for one that is not an image). Chips for any other number show as plain
    /// text.
    pub(crate) fn set_attachment_chips(
        &mut self,
        scope: &str,
        attachments: HashMap<u32, Option<std::sync::Arc<gpui::Image>>>,
        cx: &mut Context<Self>,
    ) {
        let unchanged = self.attachment_scope == scope
            && self.attachment_chips.as_ref().is_some_and(|current| {
                current.len() == attachments.len()
                    && attachments.iter().all(|(index, image)| {
                        current.get(index).is_some_and(|known| {
                            known.image.as_ref().map(std::sync::Arc::as_ptr)
                                == image.as_ref().map(std::sync::Arc::as_ptr)
                        })
                    })
            });
        if unchanged {
            return;
        }
        self.attachment_scope = scope.to_string();
        self.attachment_chips = Some(
            attachments
                .into_iter()
                .map(|(index, image)| (index, ChipAttachment { image }))
                .collect(),
        );
        self.invalidate_mention_tooltip();
        self.refresh_projection();
        self.needs_measure = true;
        cx.notify();
    }

    /// Insert a canonical reference link at the selection, as one undo step.
    pub(crate) fn insert_reference(&mut self, link: &str, cx: &mut Context<Self>) -> bool {
        // The platform still owns the marked range during IME composition.
        // Inserting a chip into it would leave those offsets pointing inside
        // the new reference, and the next IME update could delete the chip.
        if self.read_only || self.marked_range.is_some() {
            return false;
        }
        let range = self.selected_range.clone();
        let Some((inserted, cursor_advance)) =
            dropped_reference(&self.content, range.clone(), link)
        else {
            return false;
        };
        self.invalidate_mention_tooltip();
        self.record_edit(&range, &inserted);
        self.edit_revision = self.edit_revision.wrapping_add(1);
        self.content =
            self.content[..range.start].to_owned() + &inserted + &self.content[range.end..];
        let cursor = range.start + cursor_advance;
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.preferred_column = None;
        self.refresh_projection();
        self.follow_cursor = true;
        self.reset_blink();
        self.needs_measure = true;
        cx.emit(ComposerInputEvent::Edited);
        cx.notify();
        true
    }

    /// Replace a completed plain-text token (slash commands) as one
    /// non-coalescing undo step. Unlike [`Self::replace_mention`], the
    /// replacement is ordinary text — no link, no chip projection.
    pub fn replace_plain_token(
        &mut self,
        range: Range<usize>,
        replacement: &str,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let (trailing, advance) = reference_suffix(self.content[range.end..].chars().next());
        let inserted = format!("{replacement}{trailing}");
        self.record_edit(&range, &inserted);
        self.edit_revision = self.edit_revision.wrapping_add(1);
        self.content =
            self.content[..range.start].to_owned() + &inserted + &self.content[range.end..];
        let cursor = range.start + inserted.len() + advance;
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.preferred_column = None;
        self.refresh_projection();
        self.follow_cursor = true;
        self.reset_blink();
        self.needs_measure = true;
        cx.emit(ComposerInputEvent::Edited);
        cx.notify();
    }

    fn remove_completion_token(&mut self, mut range: Range<usize>, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.content[..range.start].trim().is_empty()
            && self.content[range.end..].trim().is_empty()
        {
            range = 0..self.content.len();
        } else if (range.start == 0 || self.content[..range.start].ends_with(' '))
            && self.content[range.end..].starts_with(' ')
        {
            range.end += 1;
        }
        self.last_edit = None;
        self.record_edit(&range, "");
        self.last_edit = None;
        self.edit_revision = self.edit_revision.wrapping_add(1);
        self.content.replace_range(range.clone(), "");
        self.selected_range = range.start..range.start;
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.preferred_column = None;
        self.refresh_projection();
        self.follow_cursor = true;
        self.needs_measure = true;
        self.reset_blink();
        cx.emit(ComposerInputEvent::Edited);
        cx.notify();
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Set (or clear) the inline completion preview. Only paints while the
    /// caret sits at the end of a non-empty draft — see the prepaint gate.
    pub fn set_ghost(&mut self, ghost: Option<SharedString>, cx: &mut Context<Self>) {
        if self.ghost == ghost {
            return;
        }
        self.ghost = ghost;
        cx.notify();
    }

    pub fn has_newline(&self) -> bool {
        self.content.contains('\n')
    }

    /// Unwrapped width of the widest line — feeds the compact/expanded flip.
    pub fn measured_text_width(&self) -> f32 {
        self.max_line_width
    }

    pub fn measured_content_height(&self) -> f32 {
        self.content_height
    }

    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.cancel_dictation();
        self.invalidate_mention_tooltip();
        self.edit_revision = self.edit_revision.wrapping_add(1);
        self.content = text.into();
        if self.single_line {
            self.content = self.content.replace(['\r', '\n'], " ");
        }
        let end = self.content.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.preferred_column = None;
        self.marked_range = None;
        self.scroll_top = 0.0;
        self.scroll_left = 0.0;
        self.refresh_projection();
        self.follow_cursor = true;
        // Programmatic replacement (draft load, clear-on-submit) is a new
        // document, not an edit — undo must not reach back past it.
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.last_edit = None;
        self.reset_blink();
        self.needs_measure = true;
        cx.emit(ComposerInputEvent::Edited);
        cx.notify();
    }

    fn invalidate_mention_tooltip(&mut self) {
        self.mention_tooltip_generation = self.mention_tooltip_generation.wrapping_add(1);
        self.mention_tooltip = MentionTooltipPhase::Hidden;
        self.mention_tooltip_popup = None;
        self.mention_tooltip_task = None;
        self.mention_tooltip_view = None;
    }

    fn set_mention_hits(&mut self, hits: Vec<MentionHit>) {
        self.mention_hits = hits;
        let live = self
            .mention_tooltip
            .target()
            .is_none_or(|target| self.mention_hits.iter().any(|hit| &hit.target == target));
        if !live {
            self.invalidate_mention_tooltip();
        }
    }

    fn start_mention_tooltip_wait(&mut self, target: MentionTooltipTarget, cx: &mut Context<Self>) {
        self.mention_tooltip_generation = self.mention_tooltip_generation.wrapping_add(1);
        let generation = self.mention_tooltip_generation;
        self.mention_tooltip = MentionTooltipPhase::Waiting { target, generation };
        self.mention_tooltip_popup = None;
        self.mention_tooltip_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(MENTION_TOOLTIP_DELAY).await;
            this.update(cx, |input, cx| {
                let live = input.mention_tooltip.target().is_some_and(|target| {
                    input.mention_hits.iter().any(|hit| &hit.target == target)
                });
                let next = mention_tooltip_promote(input.mention_tooltip.clone(), generation, live);
                if next != input.mention_tooltip {
                    input.mention_tooltip = next;
                    input.mention_tooltip_task = None;
                    if let MentionTooltipPhase::Visible { target, generation } =
                        &input.mention_tooltip
                    {
                        input.mention_tooltip_view = Some(cx.new(|_| MentionPathTooltip {
                            path: target.path.clone(),
                            activation: *generation,
                        }));
                    }
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn is_image_chip(&self, target: &MentionTooltipTarget) -> bool {
        target
            .attachment
            .and_then(|index| self.attachment_chips.as_ref()?.get(&index))
            .is_some_and(|chip| chip.image.is_some())
    }

    /// The number of the image chip under `position`, if any.
    fn image_chip_at(&self, position: Point<Pixels>) -> Option<u32> {
        self.mention_hits
            .iter()
            .find(|hit| hit.bounds.contains(&position))
            .filter(|hit| self.is_image_chip(&hit.target))
            .and_then(|hit| hit.target.attachment)
    }

    fn on_mention_pointer_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.invalidate_mention_tooltip();
            return;
        }
        let target = self
            .mention_hits
            .iter()
            .find(|hit| hit.bounds.contains(&position))
            .filter(|hit| !self.is_image_chip(&hit.target))
            .map(|hit| hit.target.clone());
        let in_popup = self
            .mention_tooltip_popup
            .is_some_and(|popup| popup.contains(&position));
        let next_generation = self.mention_tooltip_generation.wrapping_add(1);
        let next = mention_tooltip_reduce(
            self.mention_tooltip.clone(),
            target.clone(),
            in_popup,
            next_generation,
        );
        if next == self.mention_tooltip {
            return;
        }
        match next {
            MentionTooltipPhase::Waiting { target, .. } => {
                self.start_mention_tooltip_wait(target, cx)
            }
            _ => {
                self.invalidate_mention_tooltip();
                self.mention_tooltip = next;
                cx.notify();
            }
        }
    }

    fn visible_mention_tooltip(
        &self,
    ) -> Option<(
        MentionTooltipTarget,
        Point<Pixels>,
        u64,
        Entity<MentionPathTooltip>,
    )> {
        let MentionTooltipPhase::Visible { target, generation } = &self.mention_tooltip else {
            return None;
        };
        self.mention_hits
            .iter()
            .find(|hit| hit.target == *target)
            .and_then(|hit| {
                let view = self.mention_tooltip_view.clone()?;
                Some((target.clone(), hit.anchor, *generation, view))
            })
    }

    fn check_mention_tooltip_visibility(
        &mut self,
        popup: Bounds<Pixels>,
        pointer: Point<Pixels>,
    ) -> bool {
        let Some((target, _, _, _)) = self.visible_mention_tooltip() else {
            return false;
        };
        let in_chip = self
            .mention_hits
            .iter()
            .any(|hit| hit.target == target && hit.bounds.contains(&pointer));
        if mention_tooltip_contains(in_chip, popup.contains(&pointer)) {
            self.mention_tooltip_popup = Some(popup);
            true
        } else {
            self.invalidate_mention_tooltip();
            false
        }
    }

    pub(crate) fn cancel_dictation(&mut self) {
        self.dictation.cancel();
        self.transcriber = None; // Drop signals capture cancellation and invalidates pending results.
        self.dictation_task = None;
    }

    fn begin_dictation(
        &mut self,
        transcriber: Box<dyn crate::dictation::Transcriber>,
        cx: &mut Context<Self>,
    ) {
        self.cancel_dictation();
        self.dictation.begin(
            &self.content,
            self.projection.normalize_range(self.selected_range.clone()),
        );
        self.transcriber = Some(transcriber);
        self.last_edit = None;
        let generation = self.dictation.generation;
        let started = Instant::now();
        self.dictation_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(40))
                    .await;
                if !this
                    .update(cx, |input, cx| {
                        let requesting =
                            input.dictation.phase == crate::dictation::Phase::Requesting;
                        let active = input.poll_dictation(generation, cx);
                        if requesting && input.dictation.phase == crate::dictation::Phase::Listening
                        {
                            tracing::debug!(
                                target: "zeron_ui::dictation",
                                activation_to_listening_ms = started.elapsed().as_millis(),
                                "Dictation capture ready"
                            );
                        }
                        active
                    })
                    .unwrap_or(false)
                {
                    break;
                }
            }
        }));
        cx.emit(DictationInputEvent::Changed);
        cx.notify();
    }

    pub(crate) fn finish_dictation(&mut self, send: bool, cx: &mut Context<Self>) -> bool {
        if !self.dictation.phase.active() {
            return false;
        }
        // Repeated clicks/Enter only mark one pending send and end audio once.
        if self.dictation.finish(send, Instant::now()) {
            if let Some(transcriber) = &mut self.transcriber {
                transcriber.finish();
            }
        }
        cx.emit(DictationInputEvent::Changed);
        cx.notify();
        true
    }

    fn complete_dictation(&mut self, phase: crate::dictation::Phase, cx: &mut Context<Self>) {
        let send = self.dictation.complete(phase);
        self.transcriber = None;
        self.last_edit = None;
        cx.emit(DictationInputEvent::Changed);
        if send {
            cx.emit(DictationInputEvent::Submit(self.dictation.generation));
        }
        cx.notify();
    }

    fn apply_dictation(&mut self, text: &str, cx: &mut Context<Self>) {
        let before = (!self.dictation.has_partial && !text.is_empty()).then(|| self.snapshot());
        if let Some(cursor) = self.dictation.replace(&mut self.content, text) {
            if let Some(before) = before {
                self.undo_stack.push(before);
                if self.undo_stack.len() > UNDO_LIMIT {
                    self.undo_stack.remove(0);
                }
                self.redo_stack.clear();
            }
            // Moving the selection explicitly stops dictation (see move_to /
            // select_to). The original selection collapses on the first result.
            self.selected_range = cursor..cursor;
            self.selection_reversed = false;
            self.caret_affinity = CaretAffinity::Downstream;
            self.preferred_column = None;
            self.edit_revision = self.edit_revision.wrapping_add(1);
            self.refresh_projection();
            self.invalidate_mention_tooltip();
            self.follow_cursor = true;
            self.reset_blink();
            self.needs_measure = true;
            cx.emit(ComposerInputEvent::Edited);
            cx.notify();
        }
    }

    fn poll_dictation(&mut self, generation: u64, cx: &mut Context<Self>) -> bool {
        use crate::dictation::{Event, Phase};
        if generation != self.dictation.generation || !self.dictation.phase.active() {
            return false;
        }
        while let Some(event) = self.transcriber.as_mut().and_then(|service| service.poll()) {
            match event {
                Event::Listening => {
                    if self.dictation.phase == Phase::Requesting {
                        self.dictation.phase = Phase::Listening;
                        self.dictation.meter.start(Instant::now());
                    }
                    cx.emit(DictationInputEvent::Changed);
                    cx.notify();
                }
                Event::Finalizing => {
                    self.finish_dictation(false, cx);
                }
                Event::Partial(text) => self.apply_dictation(&text, cx),
                Event::Final(text) => {
                    if text.trim().is_empty() && !self.dictation.has_partial {
                        self.complete_dictation(Phase::NoSpeech, cx);
                        return false;
                    }
                    self.apply_dictation(&text, cx);
                    self.complete_dictation(Phase::Idle, cx);
                    return false;
                }
                Event::Denied(message) => self.complete_dictation(Phase::Denied(message), cx),
                Event::Unavailable(message) => {
                    self.complete_dictation(Phase::Unavailable(message), cx)
                }
                Event::Failed(message) => self.complete_dictation(Phase::Failed(message), cx),
                Event::Cancelled => {
                    self.cancel_dictation();
                    cx.emit(DictationInputEvent::Changed);
                    cx.notify();
                }
            }
            if !self.dictation.phase.active() {
                self.transcriber = None;
                return false;
            }
        }
        if self.dictation.phase == Phase::Listening
            && let Some(service) = self.transcriber.as_mut()
        {
            let level = service.level();
            if self.dictation.meter.record(Instant::now(), level) {
                // The composer drives smooth frames itself; this keeps the
                // stepped reduced-motion waveform current.
                cx.emit(DictationInputEvent::Changed);
            }
        }
        if self.dictation.timed_out(Instant::now()) {
            self.complete_dictation(
                Phase::Failed(
                    "Dictation took too long. Your draft is safe. Wait a moment, then try a shorter recording.".into(),
                ),
                cx,
            );
            return false;
        }
        true
    }

    fn toggle_dictation_action(
        &mut self,
        _: &ToggleDictation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dictation_key.is_some() {
            return; // Auto-repeat while held.
        }
        if !crate::dictation::enabled(cx) && !self.dictation.phase.active() {
            // Off by default: let another binding of the same chord run.
            cx.propagate();
            return;
        }
        // Mirror `apply_keymap`, which binds the default for an unparseable combo.
        let parse = |combo: &str| Keystroke::parse(&crate::settings::platform_combo(combo)).ok();
        let keystroke = parse(&crate::settings::current(cx).keymap.toggle_dictation)
            .or_else(|| parse(crate::settings::ShortcutId::ToggleDictation.default_combo()));
        let blur = cx.on_blur(&self.focus_handle, window, |input, _, cx| {
            input.release_dictation_key(cx);
        });
        self.dictation_key = Some(DictationKey {
            keystroke,
            _blur: blur,
        });
        cx.emit(DictationInputEvent::Press);
    }

    fn release_dictation_key(&mut self, cx: &mut Context<Self>) {
        if self.dictation_key.take().is_some() {
            cx.emit(DictationInputEvent::Release);
        }
    }

    fn on_dictation_key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let released = self.dictation_key.as_ref().is_some_and(|held| {
            held.keystroke
                .as_ref()
                .is_none_or(|binding| binding.key.eq_ignore_ascii_case(&event.keystroke.key))
        });
        if released {
            self.release_dictation_key(cx);
        }
    }

    fn on_dictation_modifiers(
        &mut self,
        event: &ModifiersChangedEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let released = self
            .dictation_key
            .as_ref()
            .and_then(|held| held.keystroke.as_ref())
            .is_some_and(|binding| {
                let (bound, now) = (&binding.modifiers, &event.modifiers);
                (bound.platform && !now.platform)
                    || (bound.control && !now.control)
                    || (bound.alt && !now.alt)
                    || (bound.shift && !now.shift)
            });
        if released {
            self.release_dictation_key(cx);
        }
    }

    // ---- undo history ----

    fn snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            content: self.content.clone(),
            selected_range: self.selected_range.clone(),
            selection_reversed: self.selection_reversed,
            caret_affinity: self.caret_affinity,
        }
    }

    /// Called with the range about to be replaced, BEFORE the content changes,
    /// so the pushed snapshot is the pre-edit state.
    fn record_edit(&mut self, range: &Range<usize>, new_text: &str) {
        self.cancel_dictation();
        let kind = if new_text.is_empty() {
            EditKind::Delete
        } else {
            EditKind::Insert
        };
        // A run merges only while it stays single-character, contiguous with
        // the previous edit, of the same kind, and inside the idle window. A
        // pause, a word break, a paste, or a caret jump all break the run so
        // undo lands on a boundary the user recognizes.
        let coalescible = match kind {
            EditKind::Insert => range.is_empty() && new_text.chars().count() == 1,
            EditKind::Delete => self
                .content
                .get(range.clone())
                .is_some_and(|text| text.graphemes(true).count() == 1),
        };
        let mergeable = coalescible
            && match (kind, &self.last_edit) {
                (EditKind::Insert, Some((EditKind::Insert, at, when))) => {
                    range.is_empty()
                        && range.start == *at
                        && new_text.chars().count() == 1
                        && !new_text.starts_with(['\n', ' ', '\t'])
                        && when.elapsed() < UNDO_COALESCE
                }
                (EditKind::Delete, Some((EditKind::Delete, at, when))) => {
                    range.end == *at && when.elapsed() < UNDO_COALESCE
                }
                _ => false,
            };
        if !mergeable {
            self.undo_stack.push(self.snapshot());
            if self.undo_stack.len() > UNDO_LIMIT {
                self.undo_stack.remove(0);
            }
        }
        // Any fresh edit invalidates the redo branch.
        self.redo_stack.clear();
        let tail = match kind {
            EditKind::Insert => range.start + new_text.len(),
            EditKind::Delete => range.start,
        };
        self.last_edit = coalescible.then(|| (kind, tail, Instant::now()));
    }

    fn restore(&mut self, snapshot: EditSnapshot, cx: &mut Context<Self>) {
        self.cancel_dictation();
        self.invalidate_mention_tooltip();
        self.edit_revision = self.edit_revision.wrapping_add(1);
        self.content = snapshot.content;
        self.selected_range = snapshot.selected_range;
        self.selection_reversed = snapshot.selection_reversed;
        self.caret_affinity = snapshot.caret_affinity;
        self.preferred_column = None;
        self.marked_range = None;
        self.refresh_projection();
        self.follow_cursor = true;
        // Never merge a subsequent edit into a step that undo just crossed.
        self.last_edit = None;
        self.reset_blink();
        self.needs_measure = true;
        cx.emit(ComposerInputEvent::Edited);
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.dictation.phase.active() {
            cx.emit(DictationInputEvent::Changed);
            cx.notify();
        }
        self.cancel_dictation();
        if self.read_only {
            return;
        }
        let Some(previous) = self.undo_stack.pop() else {
            return;
        };
        self.redo_stack.push(self.snapshot());
        self.restore(previous, cx);
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.dictation.phase.active() {
            cx.emit(DictationInputEvent::Changed);
            cx.notify();
        }
        self.cancel_dictation();
        if self.read_only {
            return;
        }
        let Some(next) = self.redo_stack.pop() else {
            return;
        };
        self.undo_stack.push(self.snapshot());
        self.restore(next, cx);
    }

    // ---- editing ops ----

    pub fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.cancel_dictation();
        self.last_edit = None;
        let offset = self.projection.normalize_range(offset..offset).start;
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.preferred_column = None;
        self.follow_cursor = true;
        self.reset_blink();
        cx.emit(ComposerInputEvent::CursorMoved);
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.cancel_dictation();
        self.last_edit = None;
        self.extend_selection(offset, cx);
    }

    // Deletion extends the selection internally without breaking a typing run.
    fn extend_selection(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.caret_affinity = CaretAffinity::Downstream;
        self.preferred_column = None;
        let offset = self.projection.normalize_range(offset..offset).start;
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.follow_cursor = true;
        self.reset_blink();
        cx.emit(ComposerInputEvent::CursorMoved);
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        if let Some(boundary) = self.projection.previous_boundary(offset) {
            return boundary;
        }
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(ix, _)| (ix < offset).then_some(ix))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        if let Some(boundary) = self.projection.next_boundary(offset) {
            return boundary;
        }
        self.content
            .grapheme_indices(true)
            .find_map(|(ix, _)| (ix > offset).then_some(ix))
            .unwrap_or(self.content.len())
    }

    fn previous_word_boundary(&self, offset: usize) -> usize {
        if let Some(boundary) = self.projection.previous_boundary(offset) {
            return boundary;
        }
        self.content
            .split_word_bound_indices()
            .rev()
            .find_map(|(ix, word)| (ix < offset && !word.trim().is_empty()).then_some(ix))
            .unwrap_or(0)
    }

    fn next_word_boundary(&self, offset: usize) -> usize {
        if let Some(boundary) = self.projection.next_boundary(offset) {
            return boundary;
        }
        self.content
            .split_word_bound_indices()
            .find_map(|(ix, word)| {
                let end = ix + word.len();
                (end > offset && !word.trim().is_empty()).then_some(end)
            })
            .unwrap_or(self.content.len())
    }

    /// Byte range of the logical line containing `offset`.
    fn line_range_at(&self, offset: usize) -> Range<usize> {
        let mut offset = offset.min(self.content.len());
        while !self.content.is_char_boundary(offset) {
            offset -= 1;
        }
        let start = self.content[..offset]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let end = self.content[offset..]
            .find('\n')
            .map(|i| offset + i)
            .unwrap_or(self.content.len());
        start..end
    }

    /// Navigation stops before the whole line ending. The raw line range
    /// intentionally retains CR for full-line selection and block edits.
    fn line_content_end_at(&self, offset: usize) -> usize {
        let end = self.line_range_at(offset).end;
        if self.content.as_bytes().get(end) == Some(&b'\n')
            && end > 0
            && self.content.as_bytes()[end - 1] == b'\r'
        {
            end - 1
        } else {
            end
        }
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = self.previous_boundary(self.cursor_offset());
            if self.cursor_offset() == prev {
                return;
            }
            self.extend_selection(prev, cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.cursor_offset());
            if self.cursor_offset() == next {
                return;
            }
            self.extend_selection(next, cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = self.previous_boundary(self.cursor_offset());
            self.move_to(prev, cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.selected_range.end);
            self.move_to(next, cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if self.mention_has_selection {
            cx.emit(ComposerInputEvent::MentionNavigate(-1));
            return;
        }
        if self.prompt_history && self.on_edge_row(-1.0) {
            cx.emit(ComposerInputEvent::HistoryNavigate(-1));
            return;
        }
        self.move_vertical(-1.0, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if self.mention_has_selection {
            cx.emit(ComposerInputEvent::MentionNavigate(1));
            return;
        }
        if self.prompt_history && self.on_edge_row(1.0) {
            cx.emit(ComposerInputEvent::HistoryNavigate(1));
            return;
        }
        self.move_vertical(1.0, cx);
    }

    pub(crate) fn move_vertical(&mut self, dir: f32, cx: &mut Context<Self>) {
        if let Some((ix, affinity, column)) = self.vertical_target(dir) {
            self.move_to(ix, cx);
            self.caret_affinity = affinity;
            self.preferred_column = Some(column);
        }
    }

    fn on_edge_row(&self, dir: f32) -> bool {
        let Some(current) = self.cursor_point() else {
            return self.content.is_empty();
        };
        let target_y = f32::from(current.y) + dir * f32::from(self.line_height);
        target_y < 0.0 || target_y >= self.content_height
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((ix, affinity, column)) = self.vertical_target(-1.0) {
            self.select_to(ix, cx);
            self.caret_affinity = affinity;
            self.preferred_column = Some(column);
        }
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((ix, affinity, column)) = self.vertical_target(1.0) {
            self.select_to(ix, cx);
            self.caret_affinity = affinity;
            self.preferred_column = Some(column);
        }
    }

    /// Offset one wrapped line above/below the cursor, keeping its x column.
    /// Clamps to the document edges, matching the platform's behavior on the
    /// first and last line.
    fn vertical_target(&self, dir: f32) -> Option<(usize, CaretAffinity, Pixels)> {
        let current = self.cursor_point()?;
        let column = self.preferred_column.unwrap_or(current.x);
        let target_y = f32::from(current.y) + dir * f32::from(self.line_height);
        if target_y < 0.0 {
            return Some((0, CaretAffinity::Downstream, column));
        }
        if target_y >= self.content_height {
            return Some((self.content.len(), CaretAffinity::Upstream, column));
        }
        let (index, affinity) = self.caret_for_point(point(column, px(target_y)));
        Some((index, affinity, column))
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all_text(cx);
    }

    /// Select the whole draft — an inline rename opens with its name selected.
    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let line = self.line_range_at(self.cursor_offset());
        self.move_to(line.start, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_content_end_at(self.cursor_offset()), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        let line = self.line_range_at(self.cursor_offset());
        self.select_to(line.start, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line_content_end_at(self.cursor_offset()), cx);
    }

    fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn select_doc_start(&mut self, _: &SelectDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_doc_end(&mut self, _: &SelectDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let prev = self.previous_word_boundary(self.cursor_offset());
        self.move_to(prev, cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let next = self.next_word_boundary(self.cursor_offset());
        self.move_to(next, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let prev = self.previous_word_boundary(self.cursor_offset());
        self.select_to(prev, cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let next = self.next_word_boundary(self.cursor_offset());
        self.select_to(next, cx);
    }

    /// Opt/Cmd + Delete family. With a live selection these delete the
    /// selection only (platform behavior) — the extend runs off the cursor.
    fn delete_to(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            if self.cursor_offset() == offset {
                return;
            }
            self.extend_selection(offset, cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prev = self.previous_word_boundary(self.cursor_offset());
        self.delete_to(prev, window, cx);
    }

    fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = self.next_word_boundary(self.cursor_offset());
        self.delete_to(next, window, cx);
    }

    fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = self.line_range_at(self.cursor_offset()).start;
        self.delete_to(start, window, cx);
    }

    fn delete_to_line_end(
        &mut self,
        _: &DeleteToLineEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let end = self.line_content_end_at(self.cursor_offset());
        self.delete_to(end, window, cx);
    }

    fn clipboard_selection(&self) -> Option<(String, String)> {
        if self.selected_range.is_empty() {
            return None;
        }
        let selected = self.projection.normalize_range(self.selected_range.clone());
        let raw = self.content[selected.clone()].to_string();
        let mut text = String::new();
        let mut at = selected.start;
        // Use the document's actual chips: parsing a selected substring alone
        // would activate literal examples selected from inside code or images.
        for (link, _) in &self.projection.mentions {
            if link.range.start < selected.start || link.range.end > selected.end {
                continue;
            }
            text.push_str(&self.content[at..link.range.start]);
            text.push_str(&zeron_proto::invocation::invocation_prompt(
                &zeron_proto::file_mentions::file_mention_prompt(&self.content[link.range.clone()]),
            ));
            at = link.range.end;
        }
        text.push_str(&self.content[at..selected.end]);
        Some((raw, text))
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((raw, text)) = self.clipboard_selection() {
            cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
                text.clone(),
                serde_json::json!({
                    "zeronComposerV1": raw,
                    "text": text,
                    "zeronAttachmentScope": self.attachment_scope,
                }),
            ));
        } else if self.copies_transcript_selection
            && let Some(text) = crate::markdown::selection::selected_text()
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if let Some((raw, text)) = self.clipboard_selection() {
            cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
                text.clone(),
                serde_json::json!({
                    "zeronComposerV1": raw,
                    "text": text,
                    "zeronAttachmentScope": self.attachment_scope,
                }),
            ));

            self.last_edit = None;
            self.replace_text_in_range(None, "", window, cx);
            self.last_edit = None;
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        // Image data (or copied files) beats text — the original composer's
        // onPaste prevents the default text insert when `clipboardData.files`
        // is non-empty and stages the images instead.
        let mut images: Vec<gpui::Image> = Vec::new();
        let mut paths: Vec<PathBuf> = Vec::new();
        for entry in &item.entries {
            match entry {
                ClipboardEntry::Image(image) => images.push(image.clone()),
                ClipboardEntry::ExternalPaths(files) => {
                    paths.extend(files.paths().iter().cloned());
                }
                ClipboardEntry::String(_) => {}
            }
        }
        if !images.is_empty() {
            cx.emit(ComposerInputEvent::PastedImages(images));
            return;
        }
        if !paths.is_empty() {
            cx.emit(ComposerInputEvent::PastedPaths(paths));
            return;
        }
        if let Some(mut text) = item.text() {
            if self.mentions_enabled {
                if let Some(value) = item
                    .metadata()
                    .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
                {
                    if value.get("text").and_then(|v| v.as_str()) == Some(text.as_str()) {
                        if let Some(raw) = value.get("zeronComposerV1").and_then(|v| v.as_str()) {
                            // Chip numbers belong to one draft: chips copied from
                            // another chat would name the wrong attachment, so
                            // they paste as their plain labels instead.
                            let from_this_draft = value
                                .get("zeronAttachmentScope")
                                .and_then(|v| v.as_str())
                                .is_some_and(|scope| scope == self.attachment_scope);
                            if from_this_draft
                                || zeron_proto::attachment_mentions::attachment_mentions(raw)
                                    .is_empty()
                            {
                                text = raw.to_owned();
                            }
                        }
                    }
                }
            }
            // Clipboard operations remain separate undo steps, even a one-character paste.
            self.last_edit = None;
            self.replace_text_in_range(None, &text, window, cx);
            self.last_edit = None;
            if self.mentions_enabled && !self.single_line {
                let end = self.cursor_offset();
                cx.emit(ComposerInputEvent::PastedText {
                    range: end.saturating_sub(text.len())..end,
                    revision: self.edit_revision,
                });
            }
        }
    }

    fn apply_pasted_references(
        &mut self,
        original: &str,
        paste_end: usize,
        revision: u64,
        replacements: Vec<(Range<usize>, String)>,
        cx: &mut Context<Self>,
    ) {
        if self.read_only
            // A delayed paste lookup must not rewrite the range captured by
            // dictation and make its final transcript fail the draft guard.
            || self.dictation.phase.active()
            || self.edit_revision != revision
            || self.text() != original
            || self.marked_range.is_some()
            || self.selected_range != (paste_end..paste_end)
        {
            return;
        }
        self.edit_revision = self.edit_revision.wrapping_add(1);
        let mut cursor = paste_end;
        for (range, link) in replacements.into_iter().rev() {
            cursor = cursor - range.len() + link.len();
            self.content.replace_range(range, &link);
        }
        // Canonicalization belongs to the paste's existing undo step.
        self.selected_range = cursor..cursor;
        self.refresh_projection();
        self.needs_measure = true;
        self.last_edit = None;
        cx.emit(ComposerInputEvent::Edited);
        cx.notify();
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.mentions_enabled && self.selected_range.is_empty() && self.marked_range.is_none() {
            if let Some((range, inserted)) =
                composer_markdown::newline_edit(&self.content, self.cursor_offset())
            {
                let range = self.range_to_utf16(&range);
                self.replace_text_in_range(Some(range), &inserted, window, cx);
                return;
            }
        }
        let line_end = self.line_range_at(self.cursor_offset()).end;
        let newline = if self.line_content_end_at(self.cursor_offset()) < line_end {
            "\r\n"
        } else {
            "\n"
        };
        self.replace_text_in_range(None, newline, window, cx);
    }

    fn message_newline_or_accept(
        &mut self,
        _: &MessageNewlineOrAccept,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match enter_outcome(self.mention_has_selection, EnterOutcome::Newline) {
            EnterOutcome::AcceptCompletion => cx.emit(ComposerInputEvent::MentionAccept),
            EnterOutcome::Newline => self.newline(&Newline, window, cx),
            EnterOutcome::Submit => unreachable!("newline action cannot submit"),
        }
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        match enter_outcome(self.mention_has_selection, EnterOutcome::Submit) {
            EnterOutcome::AcceptCompletion => cx.emit(ComposerInputEvent::MentionAccept),
            EnterOutcome::Submit => cx.emit(ComposerInputEvent::Submitted),
            EnterOutcome::Newline => unreachable!("submit action cannot insert a newline"),
        }
    }

    fn modified_submit(&mut self, _: &ModifiedSubmit, _: &mut Window, cx: &mut Context<Self>) {
        match enter_outcome(self.mention_has_selection, EnterOutcome::Submit) {
            EnterOutcome::AcceptCompletion => cx.emit(ComposerInputEvent::MentionAccept),
            EnterOutcome::Submit => cx.emit(ComposerInputEvent::ModifiedSubmitted),
            EnterOutcome::Newline => unreachable!("submit action cannot insert a newline"),
        }
    }

    fn mention_tab(&mut self, _: &MentionTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.mention_has_selection {
            cx.emit(ComposerInputEvent::MentionAccept);
        } else if !self.indent_list(false, window, cx) {
            cx.propagate();
        }
    }

    fn outdent_list(&mut self, _: &OutdentList, window: &mut Window, cx: &mut Context<Self>) {
        if self.mention_open || !self.indent_list(true, window, cx) {
            cx.propagate();
        }
    }

    fn indent_list(&mut self, outdent: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.mentions_enabled
            || self.read_only
            || self.marked_range.is_some()
            || self.mention_open
        {
            return false;
        }
        let selection = self.selected_range.clone();
        let start = self.line_range_at(selection.start).start;
        let last = if !selection.is_empty() && self.content[..selection.end].ends_with('\n') {
            selection.end - 1
        } else {
            selection.end
        };
        let end = self.line_range_at(last).end;
        let block = &self.content[start..end];
        let mut replacement = String::new();
        let mut changes = Vec::new();
        let mut at = start;
        for (ix, line) in block.split('\n').enumerate() {
            let Some(prefix) = composer_markdown::list_prefix(line) else {
                return false;
            };
            if in_code(&self.content, at + prefix.indent) {
                return false;
            }
            if ix > 0 {
                replacement.push('\n');
            }
            let removed = if outdent {
                line[prefix.indent_start..prefix.indent]
                    .chars()
                    .take(2)
                    .map(char::len_utf8)
                    .sum()
            } else {
                0
            };
            let added = if outdent { 0 } else { 2 };
            replacement.push_str(&line[..prefix.indent_start]);
            if !outdent {
                replacement.push_str("  ");
            }
            replacement.push_str(&line[prefix.indent_start + removed..]);
            changes.push((at + prefix.indent_start, removed, added));
            at += line.len() + 1;
        }
        let remap = |offset: usize| -> usize {
            let mut result = offset as isize;
            for &(at, removed, added) in &changes {
                if offset >= at {
                    result += added as isize - removed.min(offset - at) as isize;
                }
            }
            result.max(0) as usize
        };
        let next = remap(selection.start)..remap(selection.end);
        let reversed = self.selection_reversed;
        let edit = self.range_to_utf16(&(start..end));
        self.last_edit = None;
        self.replace_text_in_range(Some(edit), &replacement, window, cx);
        self.selected_range = next;
        self.selection_reversed = reversed;
        self.refresh_projection();
        self.last_edit = None;
        cx.notify();
        true
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && self.dictation.phase.active() {
            self.cancel_dictation();
            cx.emit(DictationInputEvent::Changed);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if escape_dismisses_completion(&event.keystroke.key, self.mention_open) {
            cx.emit(ComposerInputEvent::MentionDismiss);
            cx.stop_propagation();
        }
    }

    // ---- geometry ----

    /// Content-local point for a byte index (y grows down from content top).
    fn point_for_index(&self, index: usize) -> Option<Point<Pixels>> {
        self.point_for_display_index(self.projection.raw_to_display(index))
    }

    fn cursor_point(&self) -> Option<Point<Pixels>> {
        self.point_for_display_index_with_affinity(
            self.projection.raw_to_display(self.cursor_offset()),
            self.caret_affinity,
        )
    }

    /// Content-local point for a shaped projection byte index. The icon layer
    /// uses this to occupy its explicit projection slot without inventing a
    /// second coordinate system beside the custom text editor.
    fn point_for_display_index(&self, index: usize) -> Option<Point<Pixels>> {
        self.point_for_display_index_with_affinity(index, CaretAffinity::Downstream)
    }

    fn point_for_display_index_with_affinity(
        &self,
        index: usize,
        affinity: CaretAffinity,
    ) -> Option<Point<Pixels>> {
        for (line_ix, line) in self.last_lines.iter().enumerate() {
            let line_start = *self.line_starts.get(line_ix)?;
            let line_len = line.len();
            if index < line_start {
                continue;
            }
            if index <= line_start + line_len {
                let relative_index = index - line_start;
                let local = if affinity == CaretAffinity::Downstream {
                    line.wrap_boundaries()
                        .iter()
                        .position(|boundary| {
                            line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index
                                == relative_index
                        })
                        .map(|row| point(px(0.0), self.line_height * (row + 1)))
                        .or_else(|| line.position_for_index(relative_index, self.line_height))?
                } else {
                    line.position_for_index(relative_index, self.line_height)?
                };
                let y_offset: f32 = self
                    .last_lines
                    .iter()
                    .take(line_ix)
                    .map(|l| f32::from(l.size(self.line_height).height))
                    .sum();
                let indent = if local.y > px(0.0) {
                    self.line_indents.get(line_ix).copied().unwrap_or_default()
                } else {
                    px(0.0)
                };
                return Some(point(local.x + indent, local.y + px(y_offset)));
            }
        }
        None
    }

    /// [`chip_label_offset`] for the chip label starting at projected byte
    /// `label`, from the shaped line holding it.
    fn chip_label_offset(
        &self,
        label: usize,
        row_top: Pixels,
        room: Pixels,
        window: &Window,
    ) -> Pixels {
        let line_ix = self
            .line_starts
            .partition_point(|start| *start <= label)
            .saturating_sub(1);
        match (self.last_lines.get(line_ix), self.line_starts.get(line_ix)) {
            (Some(line), Some(start)) => chip_label_offset(
                &line.unwrapped_layout,
                label - start,
                row_top,
                self.line_height,
                room,
                window,
            ),
            _ => px(0.0),
        }
    }

    /// Content-local boxes occupied by a projected byte range, split at every
    /// soft wrap. A caret exactly at a wrap boundary belongs visually to both
    /// rows in GPUI; using the explicit wrap indices lets the range's first
    /// glyph start at x=0 on the new row instead of inheriting the old row's
    /// end caret (which previously caused mention washes to be discarded).
    fn bounds_for_display_range(&self, range: Range<usize>) -> Vec<Bounds<Pixels>> {
        let mut bounds = Vec::new();
        let mut y_offset = px(0.0);
        for (line_ix, line) in self.last_lines.iter().enumerate() {
            let line_start = self.line_starts.get(line_ix).copied().unwrap_or(0);
            let local_start = range.start.saturating_sub(line_start).min(line.len());
            let local_end = range.end.saturating_sub(line_start).min(line.len());
            if local_start >= local_end
                || range.end <= line_start
                || range.start >= line_start + line.len()
            {
                y_offset += line.size(self.line_height).height;
                continue;
            }

            let row_ends = line
                .wrap_boundaries()
                .iter()
                .map(|boundary| line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index)
                .chain(std::iter::once(line.len()));
            for (row_ix, row_start, segment) in
                display_row_segments(local_start..local_end, row_ends)
            {
                let row_y = y_offset + self.line_height * row_ix;
                let start_x = if segment.start == row_start {
                    px(0.0)
                } else {
                    line.position_for_index(segment.start, self.line_height)
                        .map(|point| point.x)
                        .unwrap_or(px(0.0))
                };
                if let Some(end_point) = line.position_for_index(segment.end, self.line_height)
                    && end_point.x > start_x
                {
                    bounds.push(Bounds::new(
                        point(
                            start_x
                                + if row_ix > 0 {
                                    self.line_indents.get(line_ix).copied().unwrap_or_default()
                                } else {
                                    px(0.0)
                                },
                            row_y,
                        ),
                        size(end_point.x - start_x, self.line_height),
                    ));
                }
            }
            y_offset += line.size(self.line_height).height;
        }
        bounds
    }

    /// Byte index and visual side closest to a content-local point.
    fn caret_for_point(&self, position: Point<Pixels>) -> (usize, CaretAffinity) {
        if self.display_is_placeholder {
            return (0, CaretAffinity::Downstream);
        }
        let mut y = f32::from(position.y);
        if y < 0.0 {
            return (0, CaretAffinity::Downstream);
        }
        for (line_ix, line) in self.last_lines.iter().enumerate() {
            let height = f32::from(line.size(self.line_height).height);
            let line_start = self.line_starts.get(line_ix).copied().unwrap_or(0);
            if y < height || line_ix + 1 == self.last_lines.len() {
                let indent = if y >= f32::from(self.line_height) {
                    self.line_indents.get(line_ix).copied().unwrap_or_default()
                } else {
                    px(0.0)
                };
                let local = point(position.x - indent, px(y.min(height - 1.0).max(0.0)));
                let ix = line
                    .closest_index_for_position(local, self.line_height)
                    .unwrap_or_else(|ix| ix);
                let affinity = line
                    .wrap_boundaries()
                    .iter()
                    .position(|boundary| {
                        line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index == ix
                    })
                    .filter(|row| local.y < self.line_height * (row + 1))
                    .map_or(CaretAffinity::Downstream, |_| CaretAffinity::Upstream);
                let mut raw = self
                    .projection
                    .display_to_raw((line_start + ix).min(self.projection.display.len()));
                // CRLF is one newline grapheme. A click past the text on its
                // row must land before CR, never between its two bytes.
                if raw > 0
                    && self.content.as_bytes().get(raw) == Some(&b'\n')
                    && self.content.as_bytes()[raw - 1] == b'\r'
                {
                    raw -= 1;
                }
                return (raw, affinity);
            }
            y -= height;
        }
        (self.content.len(), CaretAffinity::Upstream)
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        self.caret_for_mouse_position(position).0
    }

    fn caret_for_mouse_position(&self, position: Point<Pixels>) -> (usize, CaretAffinity) {
        let Some(bounds) = self.last_bounds else {
            return (0, CaretAffinity::Downstream);
        };
        let local = point(
            position.x - bounds.left() + px(self.scroll_left),
            position.y - bounds.top() + px(self.scroll_top),
        );
        self.caret_for_point(local)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.invalidate_mention_tooltip();
        window.focus(&self.focus_handle, cx);
        let intent = press_intent(event.click_count, event.modifiers.shift);
        self.chip_press = (intent == PressIntent::PlaceCaret)
            .then(|| self.image_chip_at(event.position))
            .flatten()
            .map(|index| (index, event.position));
        self.is_selecting = true;
        self.drag_position = Some(event.position);
        self.drag_unit = None;
        self.drag_generation = self.drag_generation.wrapping_add(1);
        self.drag_autoscroll_active = false;
        match intent {
            PressIntent::Word | PressIntent::Line => {
                let index = self.index_for_mouse_position(event.position);
                let range = if intent == PressIntent::Word {
                    self.mention_hits
                        .iter()
                        .find(|hit| hit.bounds.contains(&event.position))
                        .map(|hit| hit.target.range.clone())
                        .unwrap_or_else(|| self.selection_unit(intent, index))
                } else {
                    self.selection_unit(intent, index)
                };
                self.move_to(range.start, cx);
                self.select_to(range.end, cx);
                self.drag_unit = Some((intent, range));
            }
            PressIntent::ExtendSelection => {
                let (index, affinity) = self.caret_for_mouse_position(event.position);
                self.select_to(index, cx);
                self.caret_affinity = affinity;
            }
            PressIntent::PlaceCaret => {
                let (index, affinity) = self.caret_for_mouse_position(event.position);
                self.move_to(index, cx);
                self.caret_affinity = affinity;
            }
        }
    }

    fn selection_unit(&self, intent: PressIntent, index: usize) -> Range<usize> {
        let range = if intent == PressIntent::Line {
            let mut range = self.line_range_at(index);
            if range.end < self.content.len() {
                range.end += 1;
            }
            range
        } else {
            // Hit testing snaps to chip edges; prefer the chip at that edge.
            if let Some((link, _)) = self
                .projection
                .mentions
                .iter()
                .find(|(link, _)| link.range.start <= index && index < link.range.end)
            {
                return link.range.clone();
            }
            word_range(&self.content, index)
        };
        self.projection.normalize_range(range)
    }

    fn drag_select_to(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some((intent, anchor)) = self.drag_unit.clone() {
            let range = self.selection_unit(intent, index);
            if range.start < anchor.start {
                self.move_to(anchor.end, cx);
                self.select_to(range.start, cx);
            } else {
                self.move_to(anchor.start, cx);
                self.select_to(range.end.max(anchor.end), cx);
            }
        } else {
            self.select_to(index, cx);
        }
    }

    fn drag_select_at(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let (index, affinity) = self.caret_for_mouse_position(position);
        self.drag_select_to(index, cx);
        if self.drag_unit.is_none() {
            self.caret_affinity = affinity;
        }
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        // A click that starts and ends on an image chip opens the picture, as
        // the thumbnails did; a drag across it only selects.
        if let Some((index, down)) = self.chip_press.take()
            && self.image_chip_at(event.position) == Some(index)
            && (event.position - down).magnitude() <= 4.0
        {
            cx.emit(ComposerInputEvent::OpenAttachment(index));
        }
        self.is_selecting = false;
        self.drag_position = None;
        self.drag_generation = self.drag_generation.wrapping_add(1);
        self.drag_autoscroll_active = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        self.on_mention_pointer_move(event.position, cx);
        if self.is_selecting {
            self.drag_position = Some(event.position);
            let position = self.drag_selection_position(event.position);
            self.drag_select_at(position, cx);
            if self.drag_scroll_delta(event.position) != 0.0 && !self.drag_autoscroll_active {
                self.start_drag_autoscroll(cx);
            }
        }
    }

    fn start_drag_autoscroll(&mut self, cx: &mut Context<Self>) {
        self.drag_autoscroll_active = true;
        let generation = self.drag_generation;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(DRAG_SCROLL_FRAME_MS))
                    .await;
                let keep_running = this
                    .update(cx, |input, cx| input.drag_autoscroll_tick(generation, cx))
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        })
        .detach();
    }

    fn drag_selection_position(&self, position: Point<Pixels>) -> Point<Pixels> {
        let Some(bounds) = self.last_bounds else {
            return position;
        };
        point(
            position.x.clamp(bounds.left(), bounds.right() - px(0.5)),
            position.y.clamp(bounds.top(), bounds.bottom() - px(0.5)),
        )
    }

    fn drag_scroll_delta(&self, position: Point<Pixels>) -> f32 {
        let Some(bounds) = self.last_bounds else {
            return 0.0;
        };
        input_drag_scroll_delta(
            f32::from(position.y),
            f32::from(bounds.top()),
            f32::from(bounds.bottom()),
            f32::from(self.line_height),
        )
    }

    fn drag_autoscroll_tick(&mut self, generation: u64, cx: &mut Context<Self>) -> bool {
        if !self.is_selecting || self.drag_generation != generation {
            return false;
        }
        let (Some(position), Some(bounds)) = (self.drag_position, self.last_bounds) else {
            self.drag_autoscroll_active = false;
            return false;
        };
        let delta = self.drag_scroll_delta(position);
        if delta == 0.0 {
            self.drag_autoscroll_active = false;
            return false;
        }
        let next = (self.scroll_top + delta).clamp(
            0.0,
            input_max_scroll(
                self.content_height,
                self.settled_viewport_height
                    .unwrap_or(f32::from(bounds.size.height)),
            ),
        );
        if next == self.scroll_top {
            self.drag_autoscroll_active = false;
            return false;
        }
        self.scroll_top = next;
        let edge_position = self.drag_selection_position(position);
        self.drag_select_at(edge_position, cx);
        // Selection motion normally resumes caret following. During an edge
        // drag the autoscroll loop owns the viewport instead.
        self.follow_cursor = false;
        true
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.last_bounds else {
            return;
        };
        let viewport_height = self
            .settled_viewport_height
            .unwrap_or(f32::from(bounds.size.height));
        let delta_y = f32::from(event.delta.pixel_delta(self.line_height).y);
        let next = input_scroll_offset(
            self.scroll_top,
            delta_y,
            self.content_height,
            viewport_height,
        );
        if next == self.scroll_top {
            // Overscroll guard: when the input itself is scrollable (content
            // taller than the viewport), swallow the wheel event even at the
            // scroll boundary so it never chains into the outer transcript
            // list (the native equivalent of `overscroll-behavior: contain`).
            if delta_y != 0.0 && input_max_scroll(self.content_height, viewport_height) > 0.0 {
                cx.stop_propagation();
            }
            return;
        }
        self.invalidate_mention_tooltip();
        self.scroll_top = next;
        self.follow_cursor = false;
        cx.stop_propagation();
        cx.emit(ComposerInputEvent::ViewportChanged);
        cx.notify();
    }

    // ---- utf16 mapping (IME) ----

    fn offset_from_utf16(&self, offset: usize) -> usize {
        Self::utf8_offset(&self.content, offset)
    }

    fn utf8_offset(text: &str, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in text.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    /// Shape the text at a width; store measured layout; return content height.
    /// Called from the element's measured-layout closure.
    fn layout_text(
        &mut self,
        width: Pixels,
        style: &TextStyle,
        window: &mut Window,
        cx: &App,
    ) -> f32 {
        let theme = Theme::of(cx);
        let key = InputLayoutKey {
            width,
            font: style.font(),
            font_size: style.font_size.to_pixels(window.rem_size()),
            color: style.color,
            chip_family: theme.font_mono.clone(),
            chip_color: theme.code_text,
            syntax: theme.syntax.clone(),
            marked_range: self.marked_range.clone(),
            placeholder: self.placeholder.clone(),
            mentions_enabled: self.mentions_enabled,
            active_line: self.mentions_enabled.then(|| self.editing_source_range()),
        };
        // Height-only animation, scrolling, selection and caret blinking do
        // not change shaping. Reuse the entity's single retained layout,
        // including the parent's early measurement of this same edit.
        if !self.needs_measure && self.last_layout_key.as_ref() == Some(&key) {
            self.layout_epoch += 1;
            return self.content_height;
        }
        #[cfg(test)]
        {
            self.layout_rebuilds += 1;
        }
        // Rebuild this even for an empty draft. Otherwise deleting the final
        // mention can leave its previous paint geometry alive while the
        // placeholder is already being shaped, tinting "Do anything" for a
        // frame (or longer when no subsequent layout is requested).
        self.refresh_projection();
        let (display, is_placeholder) = if self.content.is_empty() {
            (self.placeholder.clone(), true)
        } else {
            (SharedString::from(self.projection.display.clone()), false)
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        self.line_height = px(self.configured_line_height);

        // Inline code: the markdown renderer's recipe (mono font + the
        // spectrum's `code_text`) over the rounded `code_wash` beneath.
        // Thread mentions are plain accent text at medium weight.
        let (chip_font, chip_color, thread_font, thread_color) = {
            let theme = Theme::of(cx);
            let mut thread_font = style.font();
            thread_font.weight = gpui::FontWeight::MEDIUM;
            (
                gpui::font(theme.font_mono.clone()),
                theme.code_text,
                thread_font,
                theme.accent,
            )
        };
        let run_for = |len: usize, underline: bool, code: bool| TextRun {
            len,
            font: if code {
                chip_font.clone()
            } else {
                style.font()
            },
            color: if code { chip_color } else { style.color },
            // Rounded mention washes are painted explicitly beneath the text;
            // TextRun backgrounds are square and can disappear in wrapped runs.
            background_color: None,
            underline: underline.then_some(UnderlineStyle {
                color: Some(style.color),
                thickness: px(1.0),
                wavy: false,
            }),
            strikethrough: None,
        };
        let raw_faces = if self.mentions_enabled && !is_placeholder {
            composer_markdown::faces(&self.content)
        } else {
            Vec::new()
        };
        let faces = if self.mentions_enabled && !is_placeholder {
            raw_faces
                .iter()
                .cloned()
                .map(|(range, face)| {
                    (
                        self.projection.raw_to_display(range.start)
                            ..self.projection.raw_to_display(range.end),
                        face,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let syntax: Vec<_> = if self.mentions_enabled && self.syntax_source == self.content {
            self.syntax_spans
                .iter()
                .map(|span| {
                    (
                        self.projection.raw_to_display(span.range.start)
                            ..self.projection.raw_to_display(span.range.end),
                        span.kind,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let marked = self.marked_range.as_ref().map(|r| {
            self.projection.raw_to_display(r.start)..self.projection.raw_to_display(r.end)
        });
        let mut boundaries = vec![0, display.len()];
        for (range, _) in &syntax {
            boundaries.extend([range.start, range.end]);
        }
        for (range, _) in &faces {
            boundaries.extend([range.start, range.end]);
        }
        for (_, range) in &self.projection.mentions {
            boundaries.extend([range.start, range.end]);
            for pad in chip_pad_ranges(range) {
                boundaries.extend([pad.start, pad.end]);
            }
        }
        let owned_command = (self.mentions_enabled && !is_placeholder)
            .then(|| zeron_owned_command_range(&self.content))
            .flatten()
            .map(|range| {
                self.projection.raw_to_display(range.start)
                    ..self.projection.raw_to_display(range.end)
            });
        if let Some(range) = &owned_command {
            boundaries.extend([range.start, range.end]);
        }
        if let Some(range) = &marked {
            boundaries.extend([range.start, range.end]);
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut face_events: Vec<_> = faces
            .iter()
            .flat_map(|(range, face)| {
                [
                    (range.start, *face as usize, 1isize),
                    (range.end, *face as usize, -1isize),
                ]
            })
            .collect();
        face_events.sort_by_key(|event| event.0);
        let mut face_event = 0;
        let mut face_depth = [0isize; 4];
        let runs: Vec<TextRun> = boundaries
            .windows(2)
            .filter(|r| r[1] > r[0])
            .map(|r| {
                while face_event < face_events.len() && face_events[face_event].0 <= r[0] {
                    let (_, face, delta) = face_events[face_event];
                    face_depth[face] += delta;
                    face_event += 1;
                }
                let mention_ix = self
                    .projection
                    .mentions
                    .partition_point(|(_, range)| range.end <= r[0]);
                let chip_link = self
                    .projection
                    .mentions
                    .get(mention_ix)
                    .filter(|(_, range)| range.contains(&r[0]));
                let chip_range = chip_link.map(|(_, range)| range);
                let chip = chip_link.is_some();
                let code = face_depth[composer_markdown::Face::Code as usize] > 0;
                let mut run = run_for(
                    r[1] - r[0],
                    marked.as_ref().is_some_and(|range| range.contains(&r[0])),
                    code,
                );
                if chip_link.is_some_and(|(link, _)| link.in_accent_flow())
                    || (!chip && owned_command.as_ref().is_some_and(|c| c.contains(&r[0])))
                {
                    run.font = thread_font.clone();
                    run.color = thread_color;
                } else if chip_range.is_some_and(|range| {
                    chip_pad_ranges(range).iter().any(|pad| pad.contains(&r[0]))
                }) {
                    run.font.family = CHIP_PAD_FAMILY.into();
                }
                if !chip {
                    if face_depth[composer_markdown::Face::Bold as usize] > 0 {
                        run.font.weight = gpui::FontWeight::BOLD;
                    }
                    if face_depth[composer_markdown::Face::Italic as usize] > 0 {
                        run.font.style = gpui::FontStyle::Italic;
                    }
                    if code {
                        run.background_color = Some(theme.code_wash);
                    }
                    if face_depth[composer_markdown::Face::Strikethrough as usize] > 0 {
                        run.strikethrough = Some(gpui::StrikethroughStyle {
                            thickness: px(1.0),
                            color: Some(style.color),
                        });
                    }
                }
                if code && !chip {
                    run.color = style.color;
                    if let Some((_, kind)) = syntax
                        .get(syntax.partition_point(|(range, _)| range.end <= r[0]))
                        .filter(|(range, _)| range.contains(&r[0]))
                    {
                        run.color = Theme::of(cx).syntax.color(*kind);
                    }
                }
                run
            })
            .collect();
        #[cfg(test)]
        {
            self.last_runs = runs.clone();
        }

        // Each logical list line reserves its marker width for continuation
        // rows. Painting and hit testing apply the same continuation offset.
        let mut lines = Vec::new();
        let mut indents = Vec::new();
        let mut display_at = 0;
        let mut raw_at = 0;
        let mut run_at = 0;
        let run_ranges: Vec<_> = runs
            .iter()
            .map(|run| {
                let range = run_at..run_at + run.len;
                run_at = range.end;
                (range, run)
            })
            .collect();
        let code_ranges: Vec<_> = raw_faces
            .iter()
            .filter(|(_, face)| *face == composer_markdown::Face::Code)
            .map(|(range, _)| range)
            .collect();
        for text in display.split('\n') {
            let raw_line = self
                .content
                .get(raw_at..)
                .unwrap_or_default()
                .split('\n')
                .next()
                .unwrap_or_default();
            let mut indent = px(0.0);
            if self.mentions_enabled && !is_placeholder {
                if let Some(prefix) = composer_markdown::list_prefix(raw_line).filter(|prefix| {
                    let marker_at = raw_at + prefix.indent;
                    let code_ix = code_ranges.partition_point(|range| range.end <= marker_at);
                    !code_ranges
                        .get(code_ix)
                        .is_some_and(|range| range.contains(&marker_at))
                }) {
                    let end = self
                        .projection
                        .raw_to_display(raw_at + prefix.end)
                        .saturating_sub(display_at)
                        .min(text.len());
                    let marker: SharedString = text[..end].to_string().into();
                    let marker_run = run_for(marker.len(), false, false);
                    indent = window
                        .text_system()
                        .shape_line(marker, font_size, &[marker_run], None)
                        .width
                        .min(width * 0.4);
                }
            }
            let end = display_at + text.len();
            let first_run = run_ranges.partition_point(|(r, _)| r.end <= display_at);
            let line_runs: Vec<TextRun> = run_ranges[first_run..]
                .iter()
                .take_while(|(r, _)| r.start < end)
                .filter_map(|(range, run)| {
                    let len = range
                        .end
                        .min(end)
                        .saturating_sub(range.start.max(display_at));
                    (len > 0).then(|| TextRun {
                        len,
                        ..(*run).clone()
                    })
                })
                .collect();
            if let Ok(shaped) = window.text_system().shape_text(
                text.to_string().into(),
                font_size,
                &line_runs,
                (!self.single_line).then_some((width - indent).max(px(20.0))),
                None,
            ) {
                let chips: Vec<_> = self.projection.mentions[self
                    .projection
                    .mentions
                    .partition_point(|(_, r)| r.end <= display_at)..]
                    .iter()
                    .take_while(|(_, r)| r.start < end)
                    .filter(|(_, r)| r.start >= display_at && r.end <= end)
                    .map(|(_, r)| r.start - display_at..r.end - display_at)
                    .collect();
                for mut line in shaped {
                    if !self.single_line {
                        wrap_reference_chips(&mut line, &chips, (width - indent).max(px(20.0)));
                    }
                    lines.push(line);
                    indents.push(indent);
                }
            }
            display_at = end + 1;
            raw_at += raw_line.len() + 1;
        }
        self.line_indents = indents;

        // Logical line byte offsets (each shaped line covers one \n-split line).
        let mut line_starts = Vec::with_capacity(lines.len());
        let mut at = 0usize;
        for line in &lines {
            line_starts.push(at);
            at += line.len() + 1; // + '\n'
        }
        if line_starts.is_empty() {
            line_starts.push(0);
        }

        let content_height: f32 = lines
            .iter()
            .map(|l| f32::from(l.size(self.line_height).height))
            .sum();
        let max_line_width: f32 = lines
            .iter()
            .map(|l| f32::from(l.unwrapped_layout.width))
            .fold(0.0, f32::max);

        self.display_is_placeholder = is_placeholder;
        self.max_ascent = lines
            .iter()
            .map(|line| f32::from(line.unwrapped_layout.ascent))
            .fold(INPUT_TEXT_SIZE, f32::max);
        self.last_layout_key = Some(key);
        self.last_lines = lines;
        self.line_starts = line_starts;
        self.content_height = content_height.max(self.configured_line_height);
        self.max_line_width = if is_placeholder { 0.0 } else { max_line_width };
        self.last_width = f32::from(width);
        self.needs_measure = false;
        self.layout_epoch += 1;
        self.content_height
    }

    fn paint_bounds(&self, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        let visible = f32::from(bounds.size.height);
        let top_overflow = input_overflow_edges(
            self.content_height,
            self.settled_viewport_height.unwrap_or(visible),
            visible,
            self.scroll_top,
        )
        .0;
        let top_padding = if top_overflow {
            self.overflow_top_padding
        } else {
            0.0
        };
        let height = input_reveal_height(
            visible,
            self.scroll_top,
            f32::from(self.line_height),
            self.resizing,
        );
        Bounds::new(
            point(bounds.left(), bounds.top() - px(top_padding)),
            size(bounds.size.width, px(height + top_padding)),
        )
    }

    /// Keep the cursor visible when content exceeds the element height.
    fn clamp_scroll(&mut self, element_height: f32) -> bool {
        if self.single_line {
            let previous = self.scroll_left;
            let width = (self.last_width - 2.0).max(1.0);
            if let Some(cursor) = self.cursor_point() {
                let x = f32::from(cursor.x);
                self.scroll_left = self.scroll_left.min(x).max(x - width).max(0.0);
            }
            self.scroll_left = self.scroll_left.min((self.max_line_width - width).max(0.0));
            self.scroll_top = 0.0;
            return self.scroll_left != previous;
        }
        let previous = self.scroll_top;
        if self.follow_cursor {
            if let Some(cursor) = self.cursor_point() {
                self.scroll_top = input_scroll_offset_for_cursor(
                    self.scroll_top,
                    f32::from(cursor.y),
                    f32::from(self.line_height),
                    self.content_height,
                    element_height,
                    self.settled_viewport_height,
                );
            }
        }
        self.scroll_top = self.scroll_top.clamp(
            0.0,
            input_max_scroll(
                self.content_height,
                self.settled_viewport_height.unwrap_or(element_height),
            ),
        );
        self.scroll_top != previous
    }
}

impl EventEmitter<ComposerInputEvent> for ComposerInput {}

impl Focusable for ComposerInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for ComposerInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self
            .projection
            .normalize_range(self.range_from_utf16(&range_utf16));
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content.get(range)?.to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        self.selected_range = self.projection.normalize_range(self.selected_range.clone());
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked_range.take().is_some() {
            self.refresh_projection();
            self.needs_measure = true;
            self.last_edit = None;
            cx.emit(ComposerInputEvent::CursorMoved);
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        self.cancel_dictation();
        let single_line_text;
        let new_text = if self.single_line {
            single_line_text = new_text.replace(['\r', '\n'], " ");
            single_line_text.as_str()
        } else {
            new_text
        };
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let range = self.projection.normalize_range(range);
        self.invalidate_mention_tooltip();
        // An IME commit is the tail of a composition whose pre-composition
        // snapshot was already taken (`replace_and_mark_text_in_range`);
        // recording here would pin undo to the half-composed text instead.
        if self.marked_range.is_none() {
            self.record_edit(&range, new_text);
        }
        self.edit_revision = self.edit_revision.wrapping_add(1);
        self.content =
            self.content[0..range.start].to_owned() + new_text + &self.content[range.end..];
        let cursor = range.start + new_text.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.preferred_column = None;
        self.marked_range.take();
        self.refresh_projection();
        self.follow_cursor = true;
        self.reset_blink();
        self.needs_measure = true;
        cx.emit(ComposerInputEvent::Edited);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        self.cancel_dictation();
        let single_line_text;
        let new_text = if self.single_line {
            single_line_text = new_text.replace(['\r', '\n'], " ");
            single_line_text.as_str()
        } else {
            new_text
        };
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let range = self.projection.normalize_range(range);
        self.invalidate_mention_tooltip();
        // First keystroke of a composition: snapshot the text as it stood
        // before any of it existed, so one undo drops the whole composition.
        if self.marked_range.is_none() {
            self.undo_stack.push(self.snapshot());
            if self.undo_stack.len() > UNDO_LIMIT {
                self.undo_stack.remove(0);
            }
            self.redo_stack.clear();
            self.last_edit = None;
        }
        self.edit_revision = self.edit_revision.wrapping_add(1);
        self.content =
            self.content[0..range.start].to_owned() + new_text + &self.content[range.end..];
        if new_text.is_empty() {
            self.marked_range = None;
        } else {
            self.marked_range = Some(range.start..range.start + new_text.len());
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r| Self::utf8_offset(new_text, r.start)..Self::utf8_offset(new_text, r.end))
            .map(|new_range| new_range.start + range.start..new_range.end + range.start)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.preferred_column = None;
        self.refresh_projection();
        self.follow_cursor = true;
        self.reset_blink();
        self.needs_measure = true;
        cx.emit(ComposerInputEvent::Edited);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self
            .projection
            .normalize_range(self.range_from_utf16(&range_utf16));
        let start = if range.is_empty() && range.start == self.cursor_offset() {
            self.cursor_point()?
        } else {
            self.point_for_index(range.start)?
        };
        let origin = point(
            bounds.left() + start.x - px(self.scroll_left),
            bounds.top() + start.y - px(self.scroll_top),
        );
        Some(Bounds::new(origin, size(px(2.0), self.line_height)))
    }

    fn character_index_for_point(
        &mut self,
        point_in_window: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let index = self.index_for_mouse_position(point_in_window);
        Some(self.offset_to_utf16(index))
    }
}

/// The custom element: measured auto-grow layout + shaped-line painting.
struct ComposerTextElement {
    input: Entity<ComposerInput>,
    /// Max content height before internal scrolling kicks in.
    max_content_height: f32,
}

struct MentionPathTooltip {
    path: SharedString,
    /// Stable for one `Waiting → Visible` promotion; a later activation gets
    /// a new key and therefore exactly one fresh fade-in.
    activation: u64,
}

impl Render for MentionPathTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_popup();
        let card = crate::popover::popover_card(&theme)
            .max_w(px(480.0))
            .h(px(MENTION_TOOLTIP_HEIGHT))
            .flex()
            .items_center()
            .p_0()
            .px(px(8.0))
            .font_family(theme.font_mono.clone())
            .text_size(px(11.0))
            .text_color(theme.text_muted)
            .child(div().min_w_0().truncate().child(self.path.clone()));
        motion::fade_quick(
            ("file-mention-path-tooltip", self.activation),
            div().child(crate::frost::frosted(
                crate::popover::CARD_RADIUS,
                crate::frost::MENU_BLUR,
                card,
            )),
        )
    }
}

struct ComposerTextPrepaint {
    cursor: Option<PaintQuad>,
    /// Each chip row's bounds; the first row of a chip also carries its icon.
    mention_chips: Vec<(Bounds<Pixels>, Option<ChipIcon>)>,
    /// Thread chip marks: the slot rect plus the icon to paint in it.
    mention_icons: Vec<(Bounds<Pixels>, (&'static str, Option<Hsla>))>,
    mention_hits: Vec<MentionHit>,
    /// Image chips take the pointing hand: a click opens them.
    image_chip_hitboxes: Vec<gpui::Hitbox>,
    selection_quads: Vec<PaintQuad>,
    /// Completion preview: window-space origin of the end-of-text caret plus
    /// the suffix to paint there (shaped at paint time — it never joins the
    /// content's own layout, so hit-testing and the caret ignore it).
    ghost: Option<(Point<Pixels>, SharedString)>,
}

impl IntoElement for ComposerTextElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for ComposerTextElement {
    type RequestLayoutState = ();
    type PrepaintState = ComposerTextPrepaint;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        let input = self.input.clone();
        let text_style = window.text_style();
        let max_content = self.max_content_height;
        let layout_id =
            window.request_measured_layout(style, move |known, available, window, cx| {
                let width = known.width.unwrap_or(match available.width {
                    gpui::AvailableSpace::Definite(width) => width,
                    _ => px(320.0),
                });
                let content_height = input.update(cx, |input, cx| {
                    input.layout_text(width, &text_style, window, cx)
                });
                size(width, px(content_height.min(max_content)))
            });
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let text_style = window.text_style();
        self.input.update(cx, |input, cx| {
            // Intrinsic measurement may try several widths in one layout.
            // Only publish the resolved geometry, once: notifying for each
            // provisional width starts an endless measure/notify loop.
            input.layout_text(bounds.size.width, &text_style, window, cx);
            let layout = (bounds.size.width, input.content_height);
            let layout_changed = input.last_notified_layout != Some(layout);
            input.last_notified_layout = Some(layout);
            let scrolled = input.clamp_scroll(f32::from(bounds.size.height));
            input.last_bounds = Some(bounds);
            if scrolled || layout_changed {
                cx.emit(ComposerInputEvent::ViewportChanged);
            }
        });
        let input = self.input.read(cx);
        let paint_bounds = input.paint_bounds(bounds);
        let scroll = px(input.scroll_top);
        let origin = point(bounds.left() - px(input.scroll_left), bounds.top() - scroll);
        let selection_color = Theme::of(cx).selection;
        let caret_color = Theme::of(cx).caret;

        let mut mention_chips = Vec::new();
        let appearance = Theme::of(cx).appearance;
        let mut mention_hits = Vec::new();
        let mut mention_icons = Vec::new();
        for (mention, display) in &input.projection.mentions {
            if mention.is_thread() {
                let chat = input.thread_state.as_ref().and_then(|state| {
                    state
                        .read(cx)
                        .chats
                        .iter()
                        .find(|chat| chat.id == mention.path)
                });
                let icon = thread_chip_icon(chat);
                if let Some(slot) = input
                    .bounds_for_display_range(thread_icon_slot(display))
                    .into_iter()
                    .next()
                {
                    let slot = Bounds::new(
                        point(origin.x + slot.origin.x, origin.y + slot.origin.y),
                        slot.size,
                    );
                    mention_icons.push((slot, icon));
                }
            }
            let mut first_row = true;
            // Thread chips carry no hover popup: the title is already the
            // whole label, so only file chips register a tooltip target.
            let target = (!mention.is_thread()).then(|| MentionTooltipTarget {
                range: mention.range.clone(),
                path: if mention.attachment.is_some() {
                    SharedString::from(mention.basename.clone())
                } else {
                    SharedString::from(format!(
                        "{}{}",
                        mention.path,
                        if mention.is_dir() { "/" } else { "" }
                    ))
                },
                attachment: mention.attachment,
            });
            // The path tooltip stays flush so the pointer can move onto it.
            let (tooltip_height, tooltip_gap) = (MENTION_TOOLTIP_HEIGHT, 1.0);
            let label = chip_pad_ranges(display)[0].end;
            for local_bounds in input.bounds_for_display_range(display.clone()) {
                let row_top = origin.y + local_bounds.origin.y;
                // Centered on the label (see `paint_chip`).
                let label_offset = input.chip_label_offset(label, row_top, px(1.5), window);
                let chip_bounds = Bounds::new(
                    point(
                        origin.x + local_bounds.origin.x,
                        row_top + px(1.5) + label_offset,
                    ),
                    size(local_bounds.size.width, local_bounds.size.height - px(3.0)),
                );
                if !mention.in_accent_flow() {
                    mention_chips.push((
                        chip_bounds,
                        first_row.then(|| chip_icon(mention.kind, &mention.path, appearance)),
                    ));
                }
                first_row = false;
                let above_anchor = chip_bounds.top() - px(tooltip_height) - px(tooltip_gap);
                let anchor_y = if above_anchor >= px(0.0) {
                    above_anchor
                } else {
                    // GPUI positions at anchor + 1px; subtracting one keeps the
                    // below fallback flush so the pointer can enter the popup.
                    chip_bounds.bottom() - px(1.0)
                };
                let visible_bounds = chip_bounds.intersect(&paint_bounds);
                if visible_bounds.size.width == px(0.0) || visible_bounds.size.height == px(0.0) {
                    continue;
                }
                let Some(target) = target.clone() else {
                    continue;
                };
                mention_hits.push(MentionHit {
                    target,
                    bounds: visible_bounds,
                    // The fixed-height popup starts at anchor + 1px. Moving
                    // the anchor above the chip therefore yields conventional
                    // above-target placement without cursor tracking.
                    anchor: point(chip_bounds.left(), anchor_y),
                });
            }
        }
        let mut selection_quads = Vec::new();
        let mut cursor = None;
        if input.selected_range.is_empty() || input.display_is_placeholder {
            if let Some(p) = input.cursor_point() {
                cursor = Some(fill(
                    Bounds::new(
                        point(origin.x + p.x, origin.y + p.y),
                        size(px(2.0), input.line_height),
                    ),
                    caret_color,
                ));
            } else if input.display_is_placeholder {
                cursor = Some(fill(
                    Bounds::new(origin, size(px(2.0), input.line_height)),
                    caret_color,
                ));
            }
        } else if let (Some(start), Some(end)) = (
            input.point_for_index(input.selected_range.start),
            input.point_for_display_index_with_affinity(
                input.projection.raw_to_display(input.selected_range.end),
                CaretAffinity::Upstream,
            ),
        ) {
            let lh = input.line_height;
            if start.y == end.y {
                selection_quads.push(fill(
                    Bounds::from_corners(
                        point(origin.x + start.x, origin.y + start.y),
                        point(origin.x + end.x, origin.y + start.y + lh),
                    ),
                    selection_color,
                ));
            } else {
                // First visual row, full middle rows, last visual row.
                selection_quads.push(fill(
                    Bounds::from_corners(
                        point(origin.x + start.x, origin.y + start.y),
                        point(bounds.right(), origin.y + start.y + lh),
                    ),
                    selection_color,
                ));
                if end.y > start.y + lh {
                    selection_quads.push(fill(
                        Bounds::from_corners(
                            point(origin.x, origin.y + start.y + lh),
                            point(bounds.right(), origin.y + end.y),
                        ),
                        selection_color,
                    ));
                }
                selection_quads.push(fill(
                    Bounds::from_corners(
                        point(origin.x, origin.y + end.y),
                        point(origin.x + end.x, origin.y + end.y + lh),
                    ),
                    selection_color,
                ));
            }
        }
        let tooltip = input.visible_mention_tooltip();
        if let Some((_target, anchor, _activation, view)) = tooltip {
            let view = view.into();
            let input = self.input.clone();
            window.set_tooltip(AnyTooltip {
                view,
                mouse_position: anchor,
                check_visible_and_update: Rc::new(move |popup, window, cx| {
                    input.update(cx, |input, _| {
                        input.check_mention_tooltip_visibility(popup, window.mouse_position())
                    })
                }),
            });
        }
        // The ghost only shows where accepting it would insert: a collapsed
        // caret at the end of real (non-placeholder, non-IME) text.
        let ghost = input
            .ghost
            .clone()
            .filter(|g| {
                !g.is_empty()
                    && !input.display_is_placeholder
                    && input.marked_range.is_none()
                    && input.selected_range.is_empty()
                    && input.cursor_offset() == input.content.len()
            })
            .and_then(|g| {
                input
                    .point_for_index(input.content.len())
                    .map(|p| (point(origin.x + p.x, origin.y + p.y), g))
            });
        let image_chip_hitboxes = mention_hits
            .iter()
            .filter(|hit| input.is_image_chip(&hit.target))
            .map(|hit| window.insert_hitbox(hit.bounds, gpui::HitboxBehavior::Normal))
            .collect();
        ComposerTextPrepaint {
            cursor,
            mention_chips,
            mention_icons,
            mention_hits,
            image_chip_hitboxes,
            selection_quads,
            ghost,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        self.input.update(cx, |input, _| {
            input.set_mention_hits(prepaint.mention_hits.clone())
        });
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let input = self.input.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble {
                input.update(cx, |input, cx| input.on_mouse_move(event, cx));
            }
        });
        for hitbox in &prepaint.image_chip_hitboxes {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }

        // WrappedLine isn't Clone — temporarily take the shaped lines out of the
        // entity for painting, then put them back for mouse mapping.
        let (lines, indents, line_height, scroll, scroll_left) =
            self.input.update(cx, |input, _| {
                (
                    std::mem::take(&mut input.last_lines),
                    input.line_indents.clone(),
                    input.line_height,
                    input.scroll_top,
                    input.scroll_left,
                )
            });

        let paint_bounds = self.input.read(cx).paint_bounds(bounds);
        window.with_content_mask(
            Some(gpui::ContentMask {
                bounds: paint_bounds,
            }),
            |window| {
                let theme = Theme::of(cx).clone();
                for (chip, icon) in prepaint.mention_chips.drain(..) {
                    paint_chip(window, chip, icon.as_ref(), &theme, cx);
                }
                let icon_fallback = Theme::of(cx).text_muted;
                for (slot, icon) in prepaint.mention_icons.drain(..) {
                    paint_thread_icon(slot, icon, icon_fallback, window, cx);
                }
                for quad in prepaint.selection_quads.drain(..) {
                    window.paint_quad(quad);
                }
                let mut y = bounds.top() - px(scroll);
                for (line_ix, line) in lines.iter().enumerate() {
                    let height = line.size(line_height).height;
                    let indent = indents.get(line_ix).copied().unwrap_or_default();
                    if indent > px(0.0) && !line.wrap_boundaries().is_empty() {
                        for row in 0..=line.wrap_boundaries().len() {
                            let row_bounds = Bounds::new(
                                point(bounds.left(), y + line_height * row),
                                size(bounds.size.width, line_height),
                            );
                            window.with_content_mask(
                                Some(gpui::ContentMask { bounds: row_bounds }),
                                |window| {
                                    let _ = line.paint(
                                        point(
                                            bounds.left() - px(scroll_left)
                                                + if row > 0 { indent } else { px(0.0) },
                                            y,
                                        ),
                                        line_height,
                                        gpui::TextAlign::Left,
                                        Some(bounds),
                                        window,
                                        cx,
                                    );
                                },
                            );
                        }
                    } else {
                        let _ = line.paint(
                            point(bounds.left() - px(scroll_left), y),
                            line_height,
                            gpui::TextAlign::Left,
                            Some(bounds),
                            window,
                            cx,
                        );
                    }
                    y += height;
                }
                if let Some((ghost_origin, ghost)) = prepaint.ghost.take() {
                    let style = window.text_style();
                    let font_size = style.font_size.to_pixels(window.rem_size());
                    let run = TextRun {
                        len: ghost.len(),
                        font: style.font(),
                        color: Theme::of(cx).text_faint,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let line = window
                        .text_system()
                        .shape_line(ghost, font_size, &[run], None);
                    // (Clipping comes from the surrounding content mask.)
                    let _ = line.paint(
                        ghost_origin,
                        line_height,
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
                // Caret only when this input is actually focused in an active
                // window (Electron hides it on window deactivation too), and only
                // in the "on" blink phase — solid while typing, ~500ms blink idle.
                if self
                    .input
                    .update(cx, |input, cx| input.caret_shown(window, cx))
                    && let Some(cursor) = prepaint.cursor.take()
                {
                    window.paint_quad(cursor);
                }
            },
        );
        self.input.update(cx, |input, _| {
            input.last_lines = lines;
        });
    }
}

impl Render for ComposerInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.mentions_enabled && self.syntax_source != self.content {
            self.syntax_source = self.content.clone();
            self.syntax_spans.clear();
            let source = self.syntax_source.clone();
            // Keep grammar loading and parsing off the input/paint thread. A
            // dropped task and source check prevent stale edits recoloring text.
            self.syntax_task = Some(cx.spawn(async move |input, cx| {
                let (source, spans) = cx
                    .background_executor()
                    .spawn(async move {
                        let spans = composer_markdown::syntax_spans(&source);
                        (source, spans)
                    })
                    .await;
                input
                    .update(cx, |input, cx| {
                        if input.content == source {
                            input.syntax_spans = spans;
                            input.needs_measure = true;
                            cx.notify();
                        }
                    })
                    .ok();
            }));
        }
        let theme = Theme::of(cx);
        let popup_theme = theme.for_popup();
        let theme = if self.key_context == "PaletteSearch"
            || self.accessibility_role == gpui::Role::SearchInput
        {
            &popup_theme
        } else {
            theme
        };
        let text_color = if self.content.is_empty() {
            theme.text_faint
        } else {
            theme.text
        };
        let input = cx.entity().downgrade();
        div()
            .id(("composer-input", cx.entity_id()))
            .role(self.accessibility_role)
            .aria_label(self.placeholder.clone())
            .aria_placeholder(self.placeholder.clone())
            .when(!self.content.is_empty(), |this| {
                this.aria_value(self.content.clone())
            })
            .on_a11y_action(gpui::AccessibleAction::SetValue, move |data, _, cx| {
                let Some(gpui::accesskit::ActionData::Value(value)) = data else {
                    return;
                };
                let value = value.to_string();
                input
                    .update(cx, |input, cx| {
                        if !input.read_only {
                            input.set_text(value, cx);
                        }
                    })
                    .ok();
            })
            .key_context(self.key_context)
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::select_doc_start))
            .on_action(cx.listener(Self::select_doc_end))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::mention_tab))
            .on_action(cx.listener(Self::outdent_list))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::delete_to_line_start))
            .on_action(cx.listener(Self::delete_to_line_end))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::message_newline_or_accept))
            .on_action(cx.listener(Self::modified_submit))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::toggle_dictation_action))
            .on_key_up(cx.listener(Self::on_dictation_key_up))
            .on_modifiers_changed(cx.listener(Self::on_dictation_modifiers))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, window, _| {
                // Capture runs before the clicked control handles the press, so
                // another input can take focus normally during bubbling.
                if event.button == MouseButton::Left && this.focus_handle.is_focused(window) {
                    window.blur();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .w_full()
            .text_size(crate::typography::ui_rems(self.text_size))
            .line_height(crate::typography::ui_rems(self.configured_line_height))
            .text_color(text_color)
            .font_family(theme.font_sans.clone())
            .child({
                let input = cx.entity();
                let ascent = self.max_ascent;
                crate::edge_fade::edge_faded(
                    INPUT_FADE_BAND,
                    true,
                    true,
                    ComposerTextElement {
                        input: input.clone(),
                        max_content_height: self
                            .viewport_height
                            .unwrap_or(TEXTAREA_MAX - TEXTAREA_PAD_V),
                    },
                )
                // Fade through the existing top padding, like the transcript
                // scrolling under its chrome. Account for GPUI's baseline
                // sampling without consuming another inset inside the text box.
                .inset_top(ascent - self.overflow_top_padding)
                .fade_overflow_y_with(move |cx| {
                    let input = input.read(cx);
                    let visible_height = input
                        .last_bounds
                        .map_or(0.0, |bounds| f32::from(bounds.size.height));
                    input_overflow_edges(
                        input.content_height,
                        input
                            .settled_viewport_height
                            .unwrap_or(TEXTAREA_MAX - TEXTAREA_PAD_V),
                        visible_height,
                        input.scroll_top,
                    )
                })
            })
    }
}

// ---------------------------------------------------------------------------
// Composer wrapper
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct SidechatStaged {
    pub attachments: Vec<StagedAttachment>,
    pub appshots: Vec<CapturedAppshot>,
}

impl std::fmt::Debug for SidechatStaged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SidechatStaged")
            .field("attachments", &self.attachments.len())
            .field("appshots", &self.appshots.len())
            .finish()
    }
}

/// Events the shell listens for.
#[derive(Debug, Clone)]
pub enum ComposerEvent {
    WorkspaceCommand(WorkspaceCommand),
    /// Arm the shared-element transition before the draft route is replaced
    /// by the newly-created session. Emitting this before `select_chat` keeps
    /// the first destination frame on the same timeline as the source frame.
    NewThreadTransitionStarted,
    /// A prompt was sent optimistically — give the transcript its exact row
    /// identity so it can anchor the prompt at the top with the reply's
    /// reserved space below it.
    Sent {
        chat_id: String,
        message_id: String,
    },
    /// A new worktree's host-side setup attempt completed after its chat id
    /// was minted. The shell attaches an already-open terminal to that exact
    /// chat, even when the user has selected another chat in the meantime.
    WorktreeSetup {
        chat_id: String,
        setup_action: Option<zeron_proto::ProjectActionRun>,
        setup_error: Option<String>,
        target_device_id: Option<String>,
    },
    /// A locally-authored queue row was accepted. It is not a transcript send
    /// yet: the transcript remembers the stable id and promotes it to an
    /// own-turn anchor only when the host materializes the matching bubble.
    Queued {
        chat_id: String,
        message_id: String,
    },
    /// `/sidechat` (or the footer pill): open the source chat's side
    /// conversation in the right pane, creating it when there is none, and
    /// start its first turn with `prompt` when one was typed. The shell owns
    /// the tab, so the composer can only ask.
    OpenSidechat {
        source_chat_id: String,
        prompt: Option<String>,
        staged: SidechatStaged,
    },
    /// A chat key's unsent-content answer flipped (t3code
    /// `useThreadHasUnsentDraft`): the sidebar pen needs one repaint, not one
    /// per keystroke.
    UnsentChanged,
    ThreadMentionDropped,
    /// A draft was started in the background and the canvas now shows the
    /// next draft (d23); the shell refreshes its row snapshot and focus.
    BackgroundStarted,
    OpenComputerSetup {
        missing: Vec<zeron_proto::computer::ComputerPermission>,
    },
    OpenSubagent {
        chat_id: String,
        doc_id: String,
        title: String,
        frozen: bool,
    },
    OpenChat(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MentionToken {
    range: Range<usize>,
    query: String,
}

/// Refine a locally identified token using Markdown source ranges. This is
/// deliberately called only after a trigger has passed cheap boundary checks.
fn completion_markdown_end(
    text: &str,
    start: usize,
    cursor: usize,
    mut end: usize,
) -> Option<usize> {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let needs_quote_boundary = text[..start].ends_with('>');
    let line_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
    let mut paragraph_start = line_start;
    let mut token_is_prose = false;
    let mut quote_boundary = false;
    let parser = Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH);
    // Definitions have no body events, but their destinations are still link
    // syntax and must never be replaced by a nested canonical link.
    if parser
        .reference_definitions()
        .iter()
        .any(|(_, definition)| definition.span.contains(&start))
    {
        return None;
    }
    for (event, range) in parser.into_offset_iter() {
        if !range.contains(&start) {
            continue;
        }
        match event {
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => return None,
            Event::Start(Tag::Paragraph | Tag::Heading { .. } | Tag::Item) => {
                paragraph_start = range.start;
            }
            Event::Text(_) => token_is_prose = true,
            Event::Start(Tag::BlockQuote(_)) if needs_quote_boundary => {
                let prefix = &text[range.start.max(line_start)..start];
                quote_boundary |= prefix.chars().all(|ch| matches!(ch, '>' | ' ' | '\t'));
            }
            Event::Start(tag @ (Tag::Strong | Tag::Emphasis | Tag::Strikethrough)) => {
                let delimiter = match tag {
                    Tag::Strong => 2,
                    Tag::Emphasis => 1,
                    _ => text[range.clone()]
                        .bytes()
                        .take_while(|byte| *byte == b'~')
                        .count(),
                };
                let closing = range.end.saturating_sub(delimiter);
                if cursor > closing {
                    return None;
                }
                end = end.min(closing);
            }
            _ => {}
        }
    }
    if needs_quote_boundary && !quote_boundary {
        return None;
    }
    // Duplicate reference definitions are consumed without body events and
    // only the first definition is retained by reference_definitions(). Requiring
    // text also protects those later definitions and raw HTML without excluding
    // tight-list items, which omit paragraph events.
    if !token_is_prose {
        return None;
    }
    // The parser leaves an unfinished link as prose. Once its destination is
    // being authored, inserting a canonical link would create nested syntax.
    // An unfinished destination can continue across a soft line break. Stay
    // within its parsed paragraph so a malformed link in an earlier block does
    // not disable completion in later prose.
    let before = &text[paragraph_start..cursor];
    for (destination, _) in before.rmatch_indices("](") {
        if let Some(label) = before[..destination].rfind('[') {
            let escaped = before[..label]
                .bytes()
                .rev()
                .take_while(|byte| *byte == b'\\')
                .count()
                % 2
                == 1;
            let closing_escaped = before[..destination]
                .bytes()
                .rev()
                .take_while(|byte| *byte == b'\\')
                .count()
                % 2
                == 1;
            if !escaped && !closing_escaped && paragraph_start + destination + 2 <= start {
                let mut depth = 1usize;
                let mut chars = before[destination + 2..].chars();
                while let Some(ch) = chars.next() {
                    match ch {
                        '\\' => {
                            chars.next();
                        }
                        '(' => depth += 1,
                        ')' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if depth > 0 {
                    return None;
                }
            }
        }
    }
    (!in_code(text, cursor)).then_some(end)
}

/// The `@` must begin a token. This intentionally excludes `name@example.com`
/// and ordinary words while allowing punctuation such as `(@src`.
fn mention_token(text: &str, cursor: usize) -> Option<MentionToken> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let token_start = text[..cursor]
        .char_indices()
        .rev()
        .find_map(|(at, ch)| ch.is_whitespace().then_some(at + ch.len_utf8()))
        .unwrap_or(0);
    let Some(relative_at) = text[token_start..cursor].rfind('@') else {
        return None;
    };
    let at = token_start + relative_at;
    let valid_boundary = at == 0
        || text[..at]
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_whitespace() || matches!(ch, '(' | '[' | '{' | '>'));
    if text[at + 1..cursor].contains('@') || !valid_boundary {
        return None;
    }
    let closing = text[..at].chars().next_back().and_then(|ch| match ch {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    });
    let end = text[cursor..]
        .char_indices()
        .find_map(|(offset, ch)| {
            (ch.is_whitespace() || Some(ch) == closing).then_some(cursor + offset)
        })
        .unwrap_or(text.len());
    if closing.is_some_and(|ch| text[at + 1..cursor].contains(ch)) {
        return None;
    }
    let end = completion_markdown_end(text, at, cursor, end)?;
    Some(MentionToken {
        range: at..end,
        query: text[at + 1..cursor].to_string(),
    })
}

/// Invocation triggers share file completion's boundary rules, but never
/// interpret paths, currency amounts or code as invocations.
fn slash_token(text: &str, cursor: usize) -> Option<MentionToken> {
    invocation_token(text, cursor, '/')
}

fn invocation_token(text: &str, cursor: usize, prefix: char) -> Option<MentionToken> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let start = text[..cursor]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace() || matches!(c, '(' | '[' | '{' | '>'))
        .map_or(0, |(i, c)| i + c.len_utf8());
    if !text[start..cursor].starts_with(prefix) {
        return None;
    }
    let query = &text[start + 1..cursor];
    let name_grapheme = |grapheme: &str| {
        grapheme.chars().next().is_some_and(char::is_alphanumeric)
            || matches!(grapheme, "-" | "_" | ":" | ".")
    };
    if !query.graphemes(true).all(name_grapheme)
        || (prefix == '$' && query.starts_with(|c: char| c.is_numeric() || c.is_uppercase()))
    {
        return None;
    }
    // Scan from the token's start, including any combining marks after a caret
    // positioned between code points of the same grapheme.
    let end = text[start + 1..]
        .grapheme_indices(true)
        .find_map(|(offset, grapheme)| (!name_grapheme(grapheme)).then_some(start + 1 + offset))
        .unwrap_or(text.len());
    if text.get(end..end + 1) == Some("/") {
        return None;
    }
    let end = completion_markdown_end(text, start, cursor, end)?;
    Some(MentionToken {
        range: start..end,
        query: query.to_string(),
    })
}

/// Zeron's own slash command — the composer runs it, the prompt never
/// reaches the agent.
const COMPACT_COMMAND: &str = "compact";

/// Zeron's second own command — it opens a side conversation instead of
/// running a turn on this chat.
const SIDECHAT_COMMAND: &str = "sidechat";

/// `Some(argument)` when the prompt invokes zeron's own `command`: the first
/// token must be exactly `/command`, and the rest of the line is its argument.
fn app_command_argument(text: &str, command: &str) -> Option<Option<String>> {
    let rest = text.strip_prefix('/')?.strip_prefix(command)?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let argument = rest.trim();
    Some((!argument.is_empty()).then(|| argument.to_string()))
}

/// `Some(focus)` when the prompt is a `/compact` invocation: the first token
/// must be exactly `/compact`, and the rest of the line is the focus the
/// harness compacts around.
fn compact_focus(text: &str) -> Option<Option<String>> {
    app_command_argument(text, COMPACT_COMMAND)
}

/// `Some(prompt)` when the prompt is a `/sidechat` invocation: bare opens the
/// side conversation, and the rest of the line is its first turn.
fn sidechat_prompt(text: &str) -> Option<Option<String>> {
    app_command_argument(text, SIDECHAT_COMMAND)
}

/// Zeron's `/compact` rides every harness's list; one that advertises its own
/// keeps it, so the popup never shows the name twice.
fn with_compact_entry(mut commands: Vec<SlashCommand>) -> Vec<SlashCommand> {
    if commands.iter().any(|c| c.name == COMPACT_COMMAND) {
        return commands;
    }
    commands.insert(
        0,
        SlashCommand {
            name: COMPACT_COMMAND.into(),
            description: "Free up context by summarizing the conversation".into(),
            input_hint: Some("focus".into()),
            kind: SlashCommandKind::BuiltIn,
        },
    );
    commands
}

const COMPUTER_USE_COMMAND: &str = zeron_engine::computer::activation::COMPUTER_USE_SLASH_COMMAND;

/// Mirrors `Harness::computer_tools_reachable`: no Opencode, Cursor, Pi or Antigravity.
fn offers_computer_use(harness: HarnessId) -> bool {
    matches!(
        harness,
        HarnessId::ClaudeCode
            | HarnessId::Codex
            | HarnessId::Devin
            | HarnessId::Grok
            | HarnessId::Hermes
    )
}

/// Zeron owns `/computer-use` even where the harness advertises a native one.
fn with_computer_use_entry(
    harness: HarnessId,
    mut commands: Vec<SlashCommand>,
) -> Vec<SlashCommand> {
    if !offers_computer_use(harness) {
        return commands;
    }
    commands.retain(|c| c.name != COMPUTER_USE_COMMAND);
    commands.insert(
        0,
        SlashCommand {
            name: COMPUTER_USE_COMMAND.into(),
            description: "Use Zeron Computer for this request only".into(),
            input_hint: Some("request".into()),
            kind: SlashCommandKind::BuiltIn,
        },
    );
    commands
}

/// Remote hosts own their grants, so only a macOS UI on the chat's host checks them.
fn computer_preflight_applies(
    macos: bool,
    host_device: Option<&str>,
    local_device: Option<&str>,
) -> bool {
    macos && host_device.is_some() && host_device == local_device
}

fn computer_send_blockers(
    settings_enabled: bool,
    capabilities: crate::appshots::AppshotCapabilities,
) -> Vec<zeron_proto::computer::ComputerPermission> {
    if settings_enabled {
        return Vec::new();
    }
    crate::computer::cards::missing_computer_permissions(capabilities)
}

/// Same ride as `/compact`: zeron's `/sidechat` joins every harness's list
/// unless the harness already advertises that name.
fn with_sidechat_entry(mut commands: Vec<SlashCommand>) -> Vec<SlashCommand> {
    if commands.iter().any(|c| c.name == SIDECHAT_COMMAND) {
        return commands;
    }
    commands.insert(
        0,
        SlashCommand {
            name: SIDECHAT_COMMAND.into(),
            description: "Ask about this chat on the side, without touching it".into(),
            input_hint: Some("prompt".into()),
            kind: SlashCommandKind::BuiltIn,
        },
    );
    commands
}

// Skills complete under `$`, so `/` keeps only the agent's built-ins.
fn with_zeron_commands(
    harness: Option<HarnessId>,
    mut commands: Vec<SlashCommand>,
) -> Vec<SlashCommand> {
    commands.retain(|command| command.kind == SlashCommandKind::BuiltIn);
    let commands = match harness {
        Some(harness) => with_computer_use_entry(harness, commands),
        None => commands,
    };
    with_sidechat_entry(with_compact_entry(commands))
}

// Send intercepts these from the typed text, so they insert as plain `/name`.
fn zeron_owned_command(name: &str) -> bool {
    [COMPACT_COMMAND, SIDECHAT_COMMAND, COMPUTER_USE_COMMAND].contains(&name)
}

fn zeron_owned_command_range(raw: &str) -> Option<Range<usize>> {
    let (name, _) = zeron_proto::invocation::leading_command(raw)?;
    let end = name.as_ptr() as usize - raw.as_ptr() as usize + name.len();
    zeron_owned_command(name).then(|| end - name.len() - 1..end)
}

/// The pill names the sidechat once: the engine titles a sidechat
/// `Sidechat: <source>`, and the pill's own prefix would double it.
fn sidechat_pill_label(title: Option<&str>) -> String {
    let title = title.unwrap_or_default().trim();
    let body = title.strip_prefix("Sidechat:").unwrap_or(title).trim();
    if body.is_empty() {
        "Sidechat".to_string()
    } else {
        format!("Sidechat: {body}")
    }
}

/// The catalog's names live on the engine's harness descriptors, which the
/// composer never holds; this is the same list, for one sentence of copy.
pub(crate) fn harness_display_name(harness: HarnessId) -> &'static str {
    match harness {
        HarnessId::ClaudeCode => "Claude Code",
        HarnessId::Codex => "Codex",
        HarnessId::Cursor => "Cursor",
        HarnessId::Devin => "Devin",
        HarnessId::Grok => "Grok",
        HarnessId::Hermes => "Hermes",
        HarnessId::Pi => "Pi",
        HarnessId::Opencode => "OpenCode",
        HarnessId::Antigravity => "Antigravity",
        HarnessId::Mock => "Mock",
    }
}

// Claude's prompt cache lives at most an hour; the thresholds match Claude's own resume dialog.
const RESUME_COMPACTION_TOKENS: u64 = 100_000;
const RESUME_COMPACTION_IDLE_MINUTES: i64 = 70;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResumeCompaction {
    pub tokens: u64,
    pub idle_minutes: i64,
}

impl ResumeCompaction {
    pub(crate) fn idle_label(self) -> String {
        let minutes = self.idle_minutes;
        if minutes >= 24 * 60 {
            format!("{}d", minutes / (24 * 60))
        } else if minutes >= 60 && minutes % 60 != 0 {
            format!("{}h {}m", minutes / 60, minutes % 60)
        } else if minutes >= 60 {
            format!("{}h", minutes / 60)
        } else {
            format!("{minutes}m")
        }
    }
}

fn resume_compaction_due(
    harness: HarnessId,
    tokens: Option<u64>,
    last_message_at: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<ResumeCompaction> {
    if harness != HarnessId::ClaudeCode {
        return None;
    }
    let tokens = tokens.filter(|tokens| *tokens >= RESUME_COMPACTION_TOKENS)?;
    let idle_minutes = (now - last_message_at?).num_minutes();
    (idle_minutes >= RESUME_COMPACTION_IDLE_MINUTES).then_some(ResumeCompaction {
        tokens,
        idle_minutes,
    })
}

/// Pasted prose is eligible only at completion boundaries, never inside
/// Markdown code/links. Catalog matching below decides whether it is a reference.
fn pasted_reference_tokens(text: &str, pasted: Range<usize>) -> Vec<(char, MentionToken)> {
    if text.len() > 256 * 1024 || text.get(pasted.clone()).is_none() {
        return Vec::new();
    }
    let mut tokens = Vec::new();
    let mut attempts = 0;
    for (relative, prefix) in text[pasted.clone()].char_indices() {
        if !matches!(prefix, '@' | '$' | '/') {
            continue;
        }
        let start = pasted.start + relative;
        let mut end = start + 1;
        for ch in text[end..].chars() {
            if !(ch.is_alphanumeric()
                || matches!(ch, '_' | '-' | '.' | ':')
                || (prefix == '@' && ch == '/'))
            {
                break;
            }
            end += ch.len_utf8();
        }
        if end > pasted.end {
            continue;
        }
        // Do not chip a prefix of an email, URL query, or shell expression.
        if text[end..].starts_with(['@', '=', '\\'])
            || text[end..]
                .strip_prefix(['?', '#'])
                .is_some_and(|tail| tail.chars().next().is_some_and(|ch| !ch.is_whitespace()))
        {
            continue;
        }
        while end > start + 1 && matches!(text.as_bytes()[end - 1], b'.' | b':') {
            end -= 1;
        }
        if end == start + 1 {
            continue;
        }
        // Rejected code/link candidates still parse the whole document. Bound
        // attempts, not just accepted chips, so pasted examples cannot stall UI.
        if attempts == 32 {
            break;
        }
        attempts += 1;
        let token = if prefix == '@' {
            mention_token(text, end)
        } else {
            invocation_token(text, end, prefix)
        };
        if let Some(mut token) = token.filter(|token| token.range.start == start) {
            token.range.end = end;
            token.query = text[start + 1..end].to_owned();
            tokens.push((prefix, token));
        }
    }
    tokens
}

/// Interpret the trigger before discovery so a disabled $ stays ordinary text.
fn completion_trigger(
    text: &str,
    cursor: usize,
    preferences: crate::settings::SkillCompletionSettings,
) -> (Option<MentionToken>, bool, bool, bool) {
    let skill_token = preferences
        .dollar
        .then(|| invocation_token(text, cursor, '$'))
        .flatten();
    let skill = skill_token.is_some();
    let include_skills = skill || !preferences.separate_from_slash;
    let token = skill_token.or_else(|| slash_token(text, cursor));
    let commands_allowed = token.is_some() && !skill;
    (token, skill, include_skills, commands_allowed)
}

/// Human-readable presentation only; invocation names and paths stay canonical.
fn skill_display_name(name: &str) -> String {
    name.rsplit(':')
        .next()
        .unwrap_or(name)
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            let first = chars.next().unwrap();
            first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Commands implemented by Zeron, independently of the provider protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceCommand {
    Model,
    New,
    Resume,
    Settings,
    Diff,
    Files,
    Terminal,
    Rename,
    Stop,
}

impl WorkspaceCommand {
    fn catalog() -> &'static [(Self, &'static str, &'static str, bool)] {
        &[
            (
                Self::Model,
                "model",
                "Zeron: choose agent, model, and reasoning",
                false,
            ),
            (Self::New, "new", "Zeron: start a new conversation", false),
            (
                Self::Resume,
                "resume",
                "Zeron: search and open conversations",
                false,
            ),
            (Self::Settings, "settings", "Zeron: open settings", false),
            (Self::Diff, "diff", "Zeron: open changes", true),
            (Self::Files, "files", "Zeron: open project files", true),
            (Self::Terminal, "terminal", "Zeron: open a terminal", true),
            (
                Self::Rename,
                "rename",
                "Zeron: rename this conversation",
                true,
            ),
            (Self::Stop, "stop", "Zeron: stop the active run", true),
        ]
    }
}

fn with_workspace_commands(
    mut rows: Vec<InvocationCandidate>,
    in_chat: bool,
) -> Vec<InvocationCandidate> {
    rows.retain(|row| row.workspace_command.is_none());
    for &(command, name, description, needs_chat) in WorkspaceCommand::catalog() {
        if needs_chat && !in_chat {
            continue;
        }
        // Keep provider commands intact. Explicit Zeron names remain available
        // when a provider owns the unqualified name.
        let mut name = name.to_string();
        while rows.iter().any(|row| row.name == name) {
            name = format!("zeron:{name}");
        }
        rows.push(InvocationCandidate {
            invocation: zeron_proto::invocation::Invocation::Command { name: name.clone() },
            name,
            description: description.into(),
            input_hint: None,
            workspace_command: Some(command),
        });
    }
    rows
}

fn workspace_command_for_text(
    text: &str,
    rows: &[InvocationCandidate],
) -> Option<WorkspaceCommand> {
    let end = text.trim_end().len();
    let token = slash_token(text, end)?;
    if !text[..token.range.start].trim().is_empty() {
        return None;
    }
    rows.iter()
        .find(|row| row.name == token.query)?
        .workspace_command
}

#[derive(Debug, Clone)]
struct InvocationCandidate {
    workspace_command: Option<WorkspaceCommand>,
    name: String,
    description: String,
    input_hint: Option<String>,
    invocation: zeron_proto::invocation::Invocation,
}

fn invocation_insertion(
    invocation: &zeron_proto::invocation::Invocation,
    supported: bool,
) -> String {
    if !supported
        && matches!(
            invocation,
            zeron_proto::invocation::Invocation::Command { .. }
        )
    {
        invocation.prompt_text()
    } else {
        invocation.link()
    }
}

fn references_require_update(text: &str, supported: bool) -> bool {
    !supported
        && (!zeron_proto::invocation::invocation_links(text).is_empty()
            || !zeron_proto::file_mentions::file_mention_links(text).is_empty())
}

/// Slash-command completion state: like [`FileMentionState`] but the
/// candidate list is scoped to device, harness and workspace, then filtered
/// locally per keystroke. Commands and skills share focus and keyboard handling.
#[derive(Debug, Clone, Default)]
struct SlashState {
    token: Option<MentionToken>,
    /// Indices into the cached command list, filter-ranked for the query.
    filtered: Vec<usize>,
    active: Option<usize>,
    /// Harness the popup is showing commands for (cache key).
    harness: Option<HarnessId>,
    /// Shared identity for the command and skill catalogs. Switching trigger
    /// characters should not invalidate another catalog for the same provider.
    catalog_context: String,
    context: String,
    skill: bool,
    supported: bool,
    request: u64,
    loading: bool,
    error: Option<SharedString>,
    dismissed: Option<(Range<usize>, String)>,
}

#[derive(Debug, Clone, Default)]
struct FileMentionState {
    /// Workspace/device identity; unchanged text must still refresh after a checkout switch.
    context: String,
    token: Option<MentionToken>,
    /// Sidebar threads matching the token, filtered locally (no RPC) and
    /// listed above the file results; `active` indexes threads then files.
    threads: Vec<ThreadMatch>,
    results: Vec<FileSearchMatch>,
    active: Option<usize>,
    request: u64,
    loading: bool,
    /// Why the last search failed, for the popup. A failure MUST NOT render
    /// as "No matching files": cross-device searches fail for reasons the
    /// user can act on (host daemon too old for `SearchFiles`, device
    /// offline), and the empty state hid them (user report).
    error: Option<SharedString>,
    /// Full token text, not just the cursor-relative query: moving within a
    /// dismissed token keeps it closed, while any edit re-enables completion.
    dismissed: Option<(Range<usize>, String)>,
}

fn mention_response_is_current(state: &FileMentionState, request: u64) -> bool {
    state.request == request && state.token.is_some()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ThreadMatch {
    chat_id: String,
    title: String,
    marks: Vec<HarnessId>,
    project: String,
}

/// Where the prompt will run and read from: the device `read_thread`
/// resolves ids on, the project whose threads list first, and the chat
/// being composed for (which can't mention itself).
struct MentionScope<'a> {
    device_id: &'a str,
    space_id: Option<&'a str>,
    exclude: Option<&'a str>,
}

const THREAD_MENTION_LIMIT: usize = 6;

/// The sidebar title as the chip label; an untitled chat mentions as the
/// same placeholder the sidebar shows for it.
fn chat_mention_title(chat: &zeron_proto::Chat) -> String {
    crate::transcript::single_line(chat.title.as_deref().unwrap_or("New session"))
}

/// Threads whose title contains the query (case-insensitive) or whose id
/// starts with it, on the scope's device, minus the composed-for chat and
/// drafts (nothing to read). The scope's own project lists first, then the
/// rest, each in sidebar (recency) order. An empty query lists the first few.
fn thread_matches<'a>(
    chats: impl Iterator<Item = &'a zeron_proto::Chat>,
    query: &str,
    scope: MentionScope<'_>,
    is_draft: impl Fn(&str) -> bool,
    project_of: impl Fn(&zeron_proto::Chat) -> String,
) -> Vec<ThreadMatch> {
    let query = query.trim().to_lowercase();
    let mut matches: Vec<(bool, String, &zeron_proto::Chat)> = chats
        .filter(|chat| {
            chat.device_id == scope.device_id
                && Some(chat.id.as_str()) != scope.exclude
                && !is_draft(&chat.id)
        })
        .filter_map(|chat| {
            let title = chat_mention_title(chat);
            (query.is_empty()
                || title.to_lowercase().contains(&query)
                || chat.id.to_lowercase().starts_with(&query))
            .then(|| (chat.space_id.as_deref() != scope.space_id, title, chat))
        })
        .collect();
    matches.sort_by_key(|(other_project, _, _)| *other_project);
    matches
        .into_iter()
        .take(THREAD_MENTION_LIMIT)
        .map(|(_, title, chat)| ThreadMatch {
            chat_id: chat.id.clone(),
            title,
            marks: crate::pickers::chat_harness_marks(chat),
            project: project_of(chat),
        })
        .collect()
}

/// A failed file search, translated for the popup. `UnknownMethod` is the
/// version-skew case: `SearchFiles` shipped after v0.1.9, so a session hosted
/// by a device on an older daemon answers "unknown method" while the same
/// search works for local sessions.
fn mention_error_message(err: &RpcError) -> SharedString {
    match err {
        RpcError::UnknownMethod(_) => {
            "The session's device runs an older zeron — update it to search its files".into()
        }
        RpcError::Transport(_) | RpcError::Closed => "The session's device is unreachable".into(),
        RpcError::BadParams(_) | RpcError::Failed(_) => "File search failed".into(),
    }
}

/// A failed command discovery, translated for the popup.
fn invocation_candidates(
    commands: Vec<SlashCommand>,
    skills: Vec<zeron_proto::invocation::Skill>,
) -> Vec<InvocationCandidate> {
    use zeron_proto::invocation::{
        valid_invocation_name, valid_skill_command_name, valid_skill_path,
    };
    // A remote engine may use an older catalog decoder. Every visible choice
    // must still survive the local canonical-reference decoder unchanged.
    let skills: Vec<_> = skills
        .into_iter()
        .filter(|skill| {
            valid_invocation_name(&skill.name)
                && valid_skill_path(&skill.path)
                && skill
                    .command
                    .as_ref()
                    .is_none_or(|command| valid_skill_command_name(&command.name))
        })
        .collect();
    let skill_commands: std::collections::HashSet<_> = skills
        .iter()
        .filter_map(|skill| skill.command.as_ref().map(|command| command.name.as_str()))
        .collect();
    let commands: Vec<_> = commands
        .into_iter()
        .filter(|command| {
            valid_invocation_name(&command.name) && !skill_commands.contains(command.name.as_str())
        })
        .collect();
    commands
        .into_iter()
        .map(|c| InvocationCandidate {
            workspace_command: None,
            input_hint: c.input_hint,
            name: c.name.clone(),
            description: c.description,
            invocation: zeron_proto::invocation::Invocation::Command { name: c.name },
        })
        .chain(
            skills
                .into_iter()
                .filter(|s| s.enabled)
                .map(|s| InvocationCandidate {
                    workspace_command: None,
                    input_hint: None,
                    name: s.name.clone(),
                    description: if zeron_proto::invocation::native_skill_identity(&s.path) {
                        s.description.clone()
                    } else {
                        format!("{} — {}", s.description, s.path)
                    },
                    invocation: zeron_proto::invocation::Invocation::Skill {
                        name: s.name,
                        path: s.path,
                        command: s.command,
                    },
                }),
        )
        .collect()
}

const SLASH_PREFETCH_DELAY: Duration = Duration::from_millis(400);

fn slash_context(
    skill: bool,
    include_skills: bool,
    commands_allowed: bool,
    catalog_context: &str,
) -> String {
    format!(
        "{}:{include_skills}:{commands_allowed}:{catalog_context}",
        if skill { "skill" } else { "command" },
    )
}

async fn fetch_invocation_catalog(
    engine: crate::state::EngineHandle,
    params: serde_json::Value,
    harness: Option<HarnessId>,
    skill: bool,
    include_skills: bool,
    commands_allowed: bool,
) -> Result<(Vec<InvocationCandidate>, bool, Option<SharedString>), RpcError> {
    let commands = async {
        if skill || !commands_allowed {
            return Ok(Vec::new());
        }
        let value = engine
            .client()
            .call(methods::LIST_COMMANDS, params.clone())
            .await?;
        serde_json::from_value::<Vec<SlashCommand>>(value)
            .map(|commands| with_zeron_commands(harness, commands))
            .map_err(|e| RpcError::Failed(e.to_string()))
    };
    let skills = async {
        // Even separated slash menus need this metadata to exclude
        // provider commands that are actually skill aliases.
        let value = engine
            .client()
            .call(methods::LIST_SKILLS, params.clone())
            .await?;
        serde_json::from_value::<Option<Vec<zeron_proto::invocation::Skill>>>(value)
            .map_err(|e| RpcError::Failed(e.to_string()))
    };
    let (commands, skills) = futures::join!(commands, skills);
    merge_invocation_results(commands, skills, skill).map(|(mut rows, supported, warning)| {
        if !include_skills {
            rows.retain(|row| row.invocation.prefix() == '/');
        }
        (rows, supported, warning)
    })
}

fn merge_invocation_results(
    commands: Result<Vec<SlashCommand>, RpcError>,
    skills: Result<Option<Vec<zeron_proto::invocation::Skill>>, RpcError>,
    skill_only: bool,
) -> Result<(Vec<InvocationCandidate>, bool, Option<SharedString>), RpcError> {
    match (commands, skills) {
        (Ok(commands), Ok(skills)) => {
            let supported = !skill_only || skills.is_some();
            Ok((
                invocation_candidates(commands, skills.unwrap_or_default()),
                supported,
                None,
            ))
        }
        (Ok(commands), Err(error)) if !skill_only && !commands.is_empty() => Ok((
            invocation_candidates(commands, vec![]),
            true,
            Some(slash_error_message(&error, true)),
        )),
        (Err(error), Ok(Some(skills))) if skills.iter().any(|skill| skill.enabled) => Ok((
            invocation_candidates(vec![], skills),
            true,
            Some(slash_error_message(&error, false)),
        )),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

fn slash_error_message(err: &RpcError, skill: bool) -> SharedString {
    match err {
        RpcError::UnknownMethod(_) => {
            if skill {
                "Skills require an updated engine on the selected device. Restart that device’s Zeron after updating.".into()
            } else {
                "Commands require an updated engine on the selected device. Restart that device’s Zeron after updating.".into()
            }
        }
        RpcError::Transport(_) | RpcError::Closed => "The session's device is unreachable".into(),
        RpcError::BadParams(_) | RpcError::Failed(_) => {
            if skill {
                "Couldn't load this agent's skills".into()
            } else {
                "Couldn't load this agent's commands".into()
            }
        }
    }
}

struct PromptHistoryCursor {
    ix: usize,
    stash: String,
    shown: String,
}

fn sent_prompts(transcript: &[SessionMessageEntry]) -> Vec<String> {
    let mut prompts: Vec<String> = Vec::new();
    for entry in transcript.iter().rev() {
        if entry.role != MessageRole::User || entry.answers_request_id.is_some() {
            continue;
        }
        let text = entry
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = text.trim();
        if text.is_empty() || prompts.last().is_some_and(|last| last == text) {
            continue;
        }
        prompts.push(text.to_string());
    }
    prompts
}

pub struct Composer {
    pub(crate) state: Entity<AppState>,
    /// The chat this composer writes to, when it is NOT the selected one: a
    /// sidechat tab hosts its own composer under the source chat's route.
    /// Every `selected_chat` read goes through [`Self::bound_chat_id`], so
    /// this one field redirects the whole composer.
    pub(crate) chat_override: Option<String>,
    pub(crate) input: Entity<ComposerInput>,
    /// Draft displaced while a queued message occupies the composer.
    pub(crate) queue_edit_draft: Option<(String, Vec<StagedAttachment>, Vec<CapturedAppshot>)>,
    /// The displaced draft's chip numbering and undo stash. The queued
    /// message gets its own while it is edited, so its chips can never name
    /// the draft's attachments.
    queue_edit_attachment_draft: Option<AttachmentDraft>,
    /// Composer actions row plus the new-session floating target tab
    /// ([`Pickers::render_new_thread_target_selectors`]).
    pickers: Entity<Pickers>,
    /// Stale stored drafts are dropped once, after the first chats frame.
    drafts_pruned: bool,
    /// Whether `current_key` is a canvas key (empty or a draft session) —
    /// the kind flips without the key changing when a draft is promoted.
    current_is_canvas: bool,
    /// Staged-but-unsent attachments per chat key (use-attachments.ts `stash`):
    /// navigating away and back restores them; memory-only, like the original.
    pub(crate) attachments: HashMap<String, Vec<StagedAttachment>>,
    attachment_drafts: HashMap<String, AttachmentDraft>,
    /// Rich window captures keyed exactly like drafts and ordinary staged
    /// attachments. Each owns one screenshot that joins the existing upload
    /// path only at send time.
    pub(crate) appshots: HashMap<String, Vec<CapturedAppshot>>,
    appshot_entrances: HashMap<String, Instant>,
    /// The staged attachment being viewed full-size (click a thumbnail).
    preview: Option<attachments::PreviewImage>,
    /// Focused while the lightbox is open so Escape reaches it; the input
    /// gets focus back on close.
    preview_focus: FocusHandle,
    /// Focus grab deferred to the next render (open sites don't all have a
    /// `Window` — the `ZERON_ATTACH_PREVIEW` boot knob opens in `new`).
    preview_focus_pending: bool,
    /// In-flight file-picker prompt (paperclip).
    picker_task: Option<Task<()>>,
    mention_task: Option<Task<()>>,
    mention: FileMentionState,
    slash_task: Option<Task<()>>,
    slash_prefetch: Option<(String, Task<()>)>,
    slash: SlashState,
    /// Advertised invocations for the current device/harness/workspace.
    /// Invalidated on context changes; filtering stays local while typing.
    slash_cache: HashMap<String, Vec<InvocationCandidate>>,
    /// How each harness compacts (`native` / `slash` / `none`), cached like
    /// the command list; `compaction_probed` holds the ids already asked for.
    compaction_cache: HashMap<HarnessId, zeron_proto::CompactionKind>,
    compaction_probed: HashSet<HarnessId>,
    /// The `CompactChat` call in flight; the chat it belongs to lives on
    /// `AppState::compacting_chat`, which the transcript's trailer reads too.
    compact_task: Option<Task<()>>,
    /// Slash-popup row scroll — the stack overflows into a wheel/keyboard-
    /// scrollable list once it outgrows the card.
    slash_scroll: gpui::ScrollHandle,
    /// File-mention popup row scroll (same treatment).
    mention_scroll: gpui::ScrollHandle,
    /// Shared scrollbar hover/drag state for both popups' floating rails —
    /// they never show at once (mutually exclusive by token shape).
    popup_bar: crate::popover::MenuScrollbarState,
    pub(crate) current_key: String,
    sending: bool,
    /// Armed immediately before a blank-canvas send selects its minted chat.
    /// The state observer consumes it to distinguish that handoff from normal
    /// session navigation, which must continue to snap.
    launching_new_chat: bool,
    pub(crate) failure: Option<SharedString>,
    /// The chat key `failure` belongs to (`None` = global, e.g. "Engine not
    /// connected"). Chat-scoped failures survive navigation and render only
    /// under their own chat — a blanket clear-on-switch erased the one
    /// visible trace of a failed send (2026-08-19).
    failure_key: Option<String>,
    resume_compaction_kept: Option<(String, Option<chrono::DateTime<chrono::Utc>>)>,
    wizard: Option<Wizard>,
    wizard_focus: FocusHandle,
    /// Requests already answered locally (suppresses the panel until the doc
    /// frame marks them resolved).
    answered_requests: HashSet<String>,
    ask: ask_pill::AskPills,
    ask_input: Entity<ComposerInput>,
    ask_focus: FocusHandle,
    _ask_input_events: Subscription,
    advance_task: Option<Task<()>>,
    send_task: Option<Task<()>>,
    /// The queued message being edited in the composer (see
    /// [`Composer::begin_queue_edit`]).
    pub(crate) editing_queued: Option<String>,
    /// Host-issued generation protecting `editing_queued` from automatic
    /// delivery. The text buffer is kept until Finish receives an ACK.
    pub(crate) queue_edit_lease_id: Option<String>,
    pub(crate) queue_edit_base_text_hash: Option<String>,
    pub(crate) queue_edit_chat_id: Option<String>,
    pub(crate) queue_edit_host_device_id: Option<String>,
    pub(crate) queue_edit_instance_id: String,
    pub(crate) queue_edit_pending_id: Option<String>,
    pub(crate) queue_edit_finishing: bool,
    pub(crate) queue_edit_task: Option<Task<()>>,
    pub(crate) queue_edit_renew_task: Option<Task<()>>,
    background: background::BackgroundUi,
    /// Focus once on mount, navigation, or after opening/closing a queue edit.
    pub(crate) focus_pending: bool,
    prompt_history: Option<PromptHistoryCursor>,
    dictation_hold: Option<DictationHold>,
    /// Live drag over the queue panel: which row, and where it would land.
    pub(crate) queue_drag: Option<crate::queue::QueueDragState>,
    pub(crate) queue_scroll: gpui::ScrollHandle,
    pub(crate) queue_full_preview: Option<Task<()>>,
    pub(crate) queue_previews: HashMap<(String, String), crate::queue::QueuePreview>,
    /// Rows awaiting a host-authoritative removal acknowledgement. They stay
    /// visible but inert until the host wins the race against queue delivery.
    pub(crate) queue_removing: HashSet<String>,
    /// The agent's checklist tray: latest list of the selected chat, and the
    /// per-chat open/fold state (in memory, like the right-pane flags).
    pub(crate) todo_cache: crate::todo_panel::TodoCache,
    pub(crate) todo_panels: HashMap<String, crate::todo_panel::TodoPanelState>,
    pub(crate) todo_scroll: gpui::ScrollHandle,
    /// Whether the modifier overlay should currently reveal the queue hint.
    /// The shell owns modifier tracking and clears this on window deactivation.
    queue_shortcut_revealed: bool,
    /// Interrupt/answer commands get their own slot: assigning `send_task`
    /// DROPPED an in-flight send future mid-upload — no banner, no cleanup,
    /// `sending` stuck true forever (2026-08-19 incident, "press Stop while
    /// a send grinds" shape).
    action_task: Option<Task<()>>,
    computer_preflight: Option<Task<()>>,
    /// Chats whose durable Interrupt command has been accepted or is still
    /// being queued. Kept independently so stopping one chat cannot replace
    /// another chat's request when the user navigates quickly.
    interrupting: HashSet<String>,
    interrupt_tasks: HashMap<String, Task<()>>,
    // -- compact/expanded flip state (hysteresis; see `composer_flip`) --
    /// Current layout mode (persisted across frames — never derived fresh).
    expanded_mode: bool,
    /// `layout_epoch` of the measurement that caused the last flip: the flip is
    /// re-evaluated only after the input has been laid out in the new mode, so
    /// at most one flip can happen per layout pass.
    flip_epoch: u64,
    /// Compact-mode input capacity, learned while compact (layout-stable).
    compact_capacity: f32,
    /// Input width first measured after expanding — container-width deltas
    /// while expanded shift `compact_capacity` by the same amount.
    expanded_anchor: f32,
    /// Last input width seen in the current mode (resize detection).
    last_seen_width: f32,
    /// Stable outer composer width supplied by the shell. Unlike Taffy's
    /// provisional input measurements, this changes only when the actual
    /// conversation column changes and can safely drive a follow-up render.
    last_available_width: Option<f32>,
    /// Set while an interactive resize is in flight; collapse is deferred
    /// until widths have settled for [`RESIZE_SETTLE_MS`].
    width_changed_at: Option<Instant>,
    settle_task: Option<Task<()>>,
    /// In-flight compact↔expanded morph (one per committed flip; manual
    /// drive — see [`FlipMorph`]).
    flip_morph: Option<FlipMorph>,
    /// Pill height actually rendered last frame — a committed flip morphs
    /// from here, so mid-flight reversals hand off without a jump.
    last_rendered_height: f32,
    #[cfg(test)]
    model_bounds: Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    dock_frame: Option<crate::composer_dock::DockFrame>,
    running_tab: bool,
    running_tab_since: Option<Instant>,
    running_ticker: Option<Task<()>>,
    dock_reflow: crate::composer_dock::DockReflow,
    /// The shared clock owns this frame's height, including its final step.
    dock_height_changed: bool,
    dock_clearance_correction: f32,
    surface_bounds: crate::new_thread_background_mask::SurfaceBounds,
    last_target_height: f32,
    height_morph: Option<FlipMorph>,
    /// Monotonic clock anchor for the morph timeline.
    morph_clock: Instant,
    /// Set on every session/route change: flips committed before this instant
    /// SNAP instead of morphing (see [`ROUTE_SNAP_MS`]).
    route_snap_until: Option<Instant>,
    _observe: Subscription,
    _pickers_observe: Subscription,
    /// The footer's rings: plan usage of the session harness's live
    /// account, and context occupancy — each opening a popover.
    account_usage: Entity<crate::account_usage::AccountUsage>,
    /// A side chat's composer: its footer keeps only the context ring.
    side_chat: bool,
    pr_chat: bool,
    pr_context: Option<String>,
    idle_placeholder: SharedString,
    _picker_focus: Subscription,
    _input_events: Subscription,
    _dictation_events: Subscription,
    dictation_activation: Option<Subscription>,
    dictation_blur: Option<Subscription>,
    dictation_focus: FocusHandle,
    /// The last live frame of the voice pill, kept so it can settle away
    /// after the transcript lands instead of vanishing.
    voice_last: Option<VoiceFrame>,
    voice_tween: VoiceTween,
}

impl EventEmitter<ComposerEvent> for Composer {}

impl Composer {
    pub(crate) fn set_dock_frame(
        &mut self,
        frame: crate::composer_dock::DockFrame,
        cx: &mut Context<Self>,
    ) {
        let changed = self.dock_frame != Some(frame);
        self.dock_height_changed |= self
            .dock_frame
            .is_none_or(|previous| previous.amount != frame.amount);
        self.dock_frame = Some(frame);
        if frame.docked {
            self.running_tab = false;
            self.running_tab_since = None;
            self.running_ticker = None;
        }
        if frame.active {
            self.flip_morph = None;
            self.height_morph = None;
        }
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn dock_clearance_correction(&self) -> f32 {
        self.dock_clearance_correction
    }

    pub(crate) fn surface_bounds(&self) -> crate::new_thread_background_mask::SurfaceBounds {
        self.surface_bounds.clone()
    }

    fn set_running_tab(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.running_tab == on {
            return;
        }
        self.running_tab = on;
        self.running_tab_since = Some(Instant::now());
        self.running_ticker = on.then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            })
        });
        cx.notify();
    }

    fn running_reveal(&self, window: &mut Window, cx: &App) -> f32 {
        let target = if self.running_tab { 1.0 } else { 0.0 };
        let Some(since) = self.running_tab_since else {
            return target;
        };
        if motion::reduced_motion(cx) {
            return target;
        }
        let total = RUNNING_TAB_FADE.total().as_secs_f32() * motion::speed_scale();
        let raw = since.elapsed().as_secs_f32() / total.max(f32::EPSILON);
        if raw < 1.0 {
            window.request_animation_frame();
        }
        let progress = RUNNING_TAB_FADE.progress(raw);
        if self.running_tab {
            progress
        } else {
            1.0 - progress
        }
    }

    fn render_home_tabs(
        &self,
        theme: &Theme,
        running: usize,
        reveal: f32,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let tab = |id: &'static str, label: &'static str, count: Option<usize>, amount: f32| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .h(px(NEW_THREAD_SELECTOR_ROW_HEIGHT))
                .px(px(8.0))
                .flex()
                .items_center()
                .gap(px(5.0))
                .rounded(px(6.0))
                .text_size(crate::typography::ui_rems(12.0))
                .cursor_pointer()
                .role(Role::Tab)
                .aria_label(label)
                .bg(theme.wash(0.08 * amount))
                .text_color(theme.text_muted.blend(theme.text.opacity(amount)))
                .hover(|style| style.text_color(theme.text))
                .child(label)
                .when_some(count, |el, count| {
                    el.child(div().text_color(theme.text_faint).child(count.to_string()))
                })
        };
        div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(
                tab("composer-tab-new-task", "New task", None, 1.0 - reveal).on_click(cx.listener(
                    |this, _, window, cx| {
                        this.set_running_tab(false, cx);
                        window.focus(&this.input.focus_handle(cx), cx);
                    },
                )),
            )
            .child(
                tab(
                    "composer-tab-running",
                    "Running agents",
                    Some(running),
                    reveal,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_running_tab(true, cx))),
            )
    }

    fn render_running_list(&self, theme: &Theme, reveal: f32, cx: &mut Context<Self>) -> gpui::Div {
        let now = chrono::Utc::now();
        let rows: Vec<(String, String, i64)> = {
            let state = self.state.read(cx);
            crate::home::running_agents(state, now)
                .into_iter()
                .map(|(chat, started)| {
                    (
                        chat.id.clone(),
                        crate::home::chat_title(chat),
                        (now - started).num_seconds(),
                    )
                })
                .collect()
        };
        let entity = cx.entity_id();
        div()
            .absolute()
            .left_0()
            .right_0()
            .top(px(6.0 * (1.0 - reveal)))
            .bottom_0()
            .opacity(reveal)
            .when(self.running_tab, |el| el.occlude())
            .child(
                div()
                    .id("composer-running-list")
                    .size_full()
                    .overflow_y_scroll()
                    .p(px(8.0))
                    .when(rows.is_empty(), |el| {
                        el.child(
                            div()
                                .px(px(10.0))
                                .py(px(8.0))
                                .text_size(crate::typography::ui_rems(13.0))
                                .text_color(theme.text_faint)
                                .child("No agents running"),
                        )
                    })
                    .children(rows.into_iter().map(|(chat_id, title, secs)| {
                        let id: SharedString = format!("composer-running-{chat_id}").into();
                        div()
                            .id(id.clone())
                            .h(px(32.0))
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .rounded(px(10.0))
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.wash(0.05)))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(ComposerEvent::OpenChat(chat_id.clone()))
                            }))
                            .child(crate::loaders::mini_glyph_spinner(
                                format!("{id}-spin"),
                                2.0,
                                theme.glyph,
                                entity,
                                cx,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .text_size(crate::typography::ui_rems(13.0))
                                    .text_color(theme.text)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .font_family(theme.font_mono.clone())
                                    .text_size(crate::typography::ui_rems(11.0))
                                    .text_color(theme.text_faint)
                                    .child(crate::composer::background::elapsed_label(secs)),
                            )
                    })),
            )
    }

    /// The picker entity, for the shell's canvas target selectors.
    pub fn pickers(&self) -> &Entity<Pickers> {
        &self.pickers
    }

    /// Feed the shell's current outer width into responsive composer controls.
    /// During a route transition this is the interpolated width, including
    /// when returning from a wider conversation to the new-chat composer.
    pub fn set_available_width(&mut self, width: f32, cx: &mut Context<Self>) {
        let composer_width = width.max(0.0);
        if composer_width_changed(self.last_available_width, composer_width) {
            self.last_available_width = Some(composer_width);
            // The shell renders before this child, so this queues one more
            // pass after the input has been laid out at its final width. That
            // pass can consume the completed measurement without emitting an
            // event from inside Taffy's multi-pass measurement callback.
            cx.notify();
        }
    }

    pub(crate) fn set_queue_shortcut_revealed(&mut self, revealed: bool, cx: &mut Context<Self>) {
        if self.queue_shortcut_revealed != revealed {
            self.queue_shortcut_revealed = revealed;
            cx.notify();
        }
    }

    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        crate::draft_store::ensure(cx);
        cx.on_release(|this, cx| {
            this.input.update(cx, |input, _| input.cancel_dictation());
            this.release_queue_previews(cx);
        })
        .detach();
        let input = cx.new(|cx| {
            let mut input =
                ComposerInput::with_context("Do anything…", MESSAGE_COMPOSER_CONTEXT, cx);
            input.enable_mentions();
            input.prompt_history = true;
            input.thread_state = Some(state.clone());
            input.copies_transcript_selection = true;
            input
        });
        let pickers = cx.new(|cx| Pickers::new(state.clone(), cx));
        let composer = cx.entity().downgrade();
        let account_usage =
            cx.new(|cx| crate::account_usage::AccountUsage::new(state.clone(), composer, cx));
        // The footer toolbar (checkout kind + ref picker) is rendered INLINE
        // by the composer from picker state — a pickers-side notify (refs
        // loaded, popover toggled, pick made) must repaint the composer too.
        // The harness can change without an AppState notify (picker switch).
        let pickers_observe = cx.observe(&pickers, |this: &mut Self, _, cx| {
            this.ensure_compaction_kind(cx);
            this.prefetch_slash_catalog(cx);
            this.on_input_edited(cx);
            cx.notify();
        });
        let picker_focus = cx.subscribe(
            &pickers,
            |this: &mut Self, _, _: &crate::pickers::ReturnComposerFocus, cx| {
                this.focus_pending = true;
                cx.notify();
            },
        );
        let observe = cx.observe(&state, |this: &mut Self, _, cx| this.on_state_changed(cx));
        let input_events = cx.subscribe(&input, |this: &mut Self, _, event, cx| match event {
            ComposerInputEvent::Submitted => this.on_submit(cx),
            ComposerInputEvent::ModifiedSubmitted => this.on_modified_submit(cx),
            ComposerInputEvent::Edited | ComposerInputEvent::CursorMoved => {
                this.on_input_edited(cx)
            }
            ComposerInputEvent::ViewportChanged => cx.notify(),
            // The slash popup and the mention popup share the input's
            // completion key routing; they are mutually exclusive by token
            // shape (`/`, `$`, or `@` at a token boundary).
            ComposerInputEvent::MentionNavigate(delta) => {
                if this.slash.token.is_some() {
                    this.move_slash(*delta, cx)
                } else {
                    this.move_mention(*delta, cx)
                }
            }
            ComposerInputEvent::MentionAccept => {
                if this.slash.token.is_some() {
                    this.accept_slash(cx)
                } else {
                    this.accept_mention(cx)
                }
            }
            ComposerInputEvent::MentionDismiss => {
                if this.slash.token.is_some() {
                    this.dismiss_slash(cx)
                } else {
                    this.dismiss_mention(cx)
                }
            }
            ComposerInputEvent::HistoryNavigate(delta) => this.navigate_history(*delta, cx),
            ComposerInputEvent::OpenAttachment(index) => this.open_attachment(*index, cx),
            ComposerInputEvent::PastedImages(images) => {
                let images = images.clone();
                this.stage_in_background(
                    move || {
                        images
                            .into_iter()
                            .map(attachments::stage_clipboard_image)
                            .map(Ok)
                            .collect()
                    },
                    cx,
                );
            }
            ComposerInputEvent::PastedPaths(paths) => this.add_paths(paths.clone(), cx),
            ComposerInputEvent::PastedText { range, revision } => {
                this.resolve_pasted_references(range.clone(), *revision, cx);
            }
        });
        cx.observe_global::<crate::settings::SettingsStore>(|this, cx| {
            if !crate::settings::current(cx).dictation_enabled {
                this.input.update(cx, |input, _| input.cancel_dictation());
            }
            this.on_input_edited(cx);
        })
        .detach();
        if let Some(presence) = crate::computer::presence::map(cx) {
            cx.observe(&presence, |_, _, cx| cx.notify()).detach();
        }
        let dictation_events = cx.subscribe(&input, |this: &mut Self, _, event, cx| match event {
            DictationInputEvent::Press => this.press_dictation(HoldSource::Key, cx),
            DictationInputEvent::Release => this.release_dictation(HoldSource::Key, cx),
            DictationInputEvent::Changed => cx.notify(),
            DictationInputEvent::Submit(generation) => {
                if this.input.read(cx).dictation.generation == *generation
                    && composer_has_content(
                        this.input.read(cx).text(),
                        this.staged().len() + this.staged_appshots().len(),
                        this.staged_chip_count(cx),
                    )
                {
                    this.on_submit(cx);
                }
            }
        });
        let ask_input = cx.new(|cx| ComposerInput::new(ask_pill::PLACEHOLDER, cx));
        let ask_input_events =
            cx.subscribe(&ask_input, |this: &mut Self, _, event, cx| match event {
                ComposerInputEvent::Submitted | ComposerInputEvent::ModifiedSubmitted => {
                    this.ask_advance(cx)
                }
                ComposerInputEvent::Edited => cx.notify(),
                _ => {}
            });
        let current_key = state.read(cx).focused_id().unwrap_or_default();
        let current_is_canvas = state.read(cx).selected_chat.is_none();
        let mut composer = Self {
            state,
            chat_override: None,
            input,
            queue_edit_draft: None,
            queue_edit_attachment_draft: None,
            pickers,
            drafts_pruned: false,
            current_is_canvas,
            attachments: HashMap::new(),
            attachment_drafts: HashMap::new(),
            appshots: HashMap::new(),
            appshot_entrances: HashMap::new(),
            preview: None,
            preview_focus: cx.focus_handle(),
            preview_focus_pending: false,
            picker_task: None,
            mention_task: None,
            mention: FileMentionState::default(),
            slash_task: None,
            slash_prefetch: None,
            slash: SlashState::default(),
            slash_cache: HashMap::new(),
            compaction_cache: HashMap::new(),
            compaction_probed: HashSet::new(),
            compact_task: None,
            slash_scroll: gpui::ScrollHandle::new(),
            mention_scroll: gpui::ScrollHandle::new(),
            popup_bar: crate::popover::MenuScrollbarState::default(),
            current_key,
            sending: false,
            launching_new_chat: false,
            failure: None,
            wizard: None,
            wizard_focus: cx.focus_handle(),
            answered_requests: HashSet::new(),
            ask: Default::default(),
            ask_input,
            ask_focus: cx.focus_handle(),
            _ask_input_events: ask_input_events,
            failure_key: None,
            resume_compaction_kept: None,
            action_task: None,
            computer_preflight: None,
            advance_task: None,
            send_task: None,
            interrupting: HashSet::new(),
            interrupt_tasks: HashMap::new(),
            editing_queued: None,
            queue_edit_lease_id: None,
            queue_edit_base_text_hash: None,
            queue_edit_chat_id: None,
            queue_edit_host_device_id: None,
            queue_edit_instance_id: uuid::Uuid::new_v4().to_string(),
            queue_edit_pending_id: None,
            queue_edit_finishing: false,
            queue_edit_task: None,
            queue_edit_renew_task: None,
            background: Default::default(),
            focus_pending: true,
            prompt_history: None,
            dictation_hold: None,
            queue_drag: None,
            queue_scroll: gpui::ScrollHandle::new(),
            queue_full_preview: None,
            queue_previews: HashMap::new(),
            queue_removing: HashSet::new(),
            todo_cache: Default::default(),
            todo_panels: HashMap::new(),
            todo_scroll: gpui::ScrollHandle::new(),
            queue_shortcut_revealed: false,
            expanded_mode: false,
            flip_epoch: 0,
            compact_capacity: 0.0,
            expanded_anchor: 0.0,
            last_seen_width: 0.0,
            last_available_width: None,
            width_changed_at: None,
            settle_task: None,
            flip_morph: None,
            last_rendered_height: 0.0,
            #[cfg(test)]
            model_bounds: Default::default(),
            dock_frame: None,
            running_tab: false,
            running_tab_since: None,
            running_ticker: None,
            dock_reflow: Default::default(),
            dock_height_changed: false,
            dock_clearance_correction: 0.0,
            surface_bounds: Default::default(),
            last_target_height: 0.0,
            height_morph: None,
            morph_clock: Instant::now(),
            route_snap_until: None,
            _observe: observe,
            _pickers_observe: pickers_observe,
            account_usage,
            side_chat: false,
            pr_chat: false,
            pr_context: None,
            idle_placeholder: "Do anything…".into(),
            _picker_focus: picker_focus,
            _input_events: input_events,
            _dictation_events: dictation_events,
            dictation_activation: None,
            dictation_blur: None,
            dictation_focus: cx.focus_handle(),
            voice_last: None,
            voice_tween: VoiceTween::default(),
        };
        // Dev knob: pre-stage attachments (drop/paste can't be synthesized on
        // a rig) — `ZERON_ATTACH=/path/a.png[,/path/b.png]`, and
        // `ZERON_ATTACH_PREVIEW=1` boots with the first one's lightbox open.
        if let Ok(spec) = std::env::var("ZERON_ATTACH") {
            let staged: Vec<StagedAttachment> = spec
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .filter_map(|path| {
                    match attachments::stage_file(std::path::Path::new(path.trim())) {
                        Ok(att) => Some(att),
                        Err(err) => {
                            tracing::warn!(%path, error = %err, "ZERON_ATTACH stage failed");
                            None
                        }
                    }
                })
                .collect();
            if std::env::var("ZERON_ATTACH_PREVIEW").is_ok_and(|v| v == "1")
                && let Some(first) = staged.first()
            {
                composer.preview = first
                    .image()
                    .map(|image| attachments::PreviewImage::new(first.name.clone(), image.clone()));
                composer.preview_focus_pending = true;
            }
            if !staged.is_empty() {
                let key = composer.current_key.clone();
                composer.adopt_staged_attachments(key, staged, cx);
            }
        }
        composer.restore_drafts(cx);
        composer
    }

    /// The store owns inactive drafts; a composer restores its current editor.
    fn restore_drafts(&mut self, cx: &mut Context<Self>) {
        let entry = crate::draft_store::entry(&self.current_key, cx);
        self.attachments
            .entry(self.current_key.clone())
            .or_default()
            .extend(entry.attachments);
        self.appshots
            .entry(self.current_key.clone())
            .or_default()
            .extend(entry.appshots);
        self.apply_draft_text(cx);
    }

    /// Capture knob (`ZERON_DRAFT_TEXT`): type into the current key as a
    /// keystroke would, draft store included.
    pub(crate) fn seed_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.set_text(text, cx));
        self.sync_draft(cx);
        cx.notify();
    }

    fn apply_draft_text(&mut self, cx: &mut Context<Self>) {
        if self.input.read(cx).text().is_empty() {
            let text = crate::draft_store::entry(&self.current_key, cx).prompt;
            self.input.update(cx, |input, cx| input.set_text(text, cx));
        }
    }

    /// Mirror the current key's text and stage into the persisted store
    /// (d19). Cheap per keystroke — the disk write is debounced there — and
    /// the sidebar repaints only when the key's has-content answer flips.
    /// A queue edit displaces the draft; the store keeps the displaced one.
    fn sync_draft(&mut self, cx: &mut Context<Self>) {
        if self.editing_queued.is_some() || self.wizard.is_some() {
            return;
        }
        let entry = crate::draft_store::DraftEntry {
            prompt: self.input.read(cx).text().to_string(),
            attachments: self.staged().to_vec(),
            appshots: self.staged_appshots().to_vec(),
        };
        if crate::draft_store::set(&self.current_key, entry, cx) {
            cx.emit(ComposerEvent::UnsentChanged);
        }
    }

    /// Drop stored drafts of chats the doc no longer has. Judged once, after
    /// the first chats frame — the pre-sync empty list would erase them all.
    fn prune_drafts(&mut self, cx: &mut Context<Self>) {
        if self.drafts_pruned || self.chat_override.is_some() {
            return;
        }
        let state = self.state.read(cx);
        if !state.chats_synced {
            return;
        }
        let live: HashSet<String> = state
            .chats
            .iter()
            .map(|c| c.id.clone())
            .chain(state.draft_sessions.iter().map(|d| d.id.clone()))
            .collect();
        self.drafts_pruned = true;
        let keep = |key: &str| live.contains(key);
        self.attachments.retain(|key, _| keep(key));
        self.appshots.retain(|key, _| keep(key));
        crate::draft_store::prune(keep, cx);
    }

    /// A composer bound to `chat_id` instead of the selection — the sidechat
    /// tab's input. Its drafts, attachments and failures key off that chat,
    /// and its sends run on it.
    pub fn for_chat(state: Entity<AppState>, chat_id: String, cx: &mut Context<Self>) -> Self {
        let mut composer = Self::new(state, cx);
        composer.attachments.clear();
        composer.appshots.clear();
        composer.current_key = chat_id.clone();
        composer.current_is_canvas = false;
        composer
            .input
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        composer.restore_drafts(cx);
        composer
            .pickers
            .update(cx, |pickers, _| pickers.bind_chat(chat_id.clone()));
        composer.chat_override = Some(chat_id);
        composer
    }

    /// The chat every read in this composer means: the bound one, else the
    /// selection (`None` = the new-chat canvas).
    pub(crate) fn bound_chat_id(&self, cx: &App) -> Option<String> {
        self.chat_override
            .clone()
            .or_else(|| self.state.read(cx).selected_chat.clone())
    }

    fn bound_chat_row<'a>(&self, cx: &'a App) -> Option<&'a zeron_proto::Chat> {
        let state = self.state.read(cx);
        let id = self
            .chat_override
            .as_deref()
            .or(state.selected_chat.as_deref())?;
        state.chats.iter().find(|chat| chat.id == id)
    }

    /// Capture-knob passthrough (`ZERON_OPEN_DIALOG=model`): open the
    /// combined harness/model menu.
    pub fn open_model_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pickers
            .update(cx, |pickers, cx| pickers.open_model_menu(window, cx));
    }

    /// Capture-knob passthrough (`ZERON_OPEN_DIALOG=model-switch`).
    pub fn debug_open_model_switch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pickers.update(cx, |pickers, cx| {
            pickers.debug_open_model_switch(window, cx)
        });
    }

    pub fn is_sending(&self) -> bool {
        self.sending
    }

    pub(crate) fn can_edit_queue_in_composer(&self) -> bool {
        !self.sending && self.wizard.is_none()
    }

    // ---- attachment staging (use-attachments.ts) ----

    /// Staged attachments for the chat the composer is showing.
    pub(crate) fn staged(&self) -> &[StagedAttachment] {
        self.attachments
            .get(&self.current_key)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Staged attachments that get a tile above the input. One the draft
    /// mentions as a chip is already on screen there (hover previews it,
    /// deleting it unstages it), so only attachments without a chip are tiled.
    fn tiled_attachments<'a>(&'a self, cx: &App) -> Vec<&'a StagedAttachment> {
        let chipped: Vec<u32> = self
            .input
            .read(cx)
            .projection
            .mentions
            .iter()
            .filter_map(|(link, _)| link.attachment)
            .collect();
        self.staged()
            .iter()
            .filter(|att| att.mention.is_none_or(|index| !chipped.contains(&index)))
            .collect()
    }

    pub(crate) fn queue_preview_limit(&self) -> usize {
        if self.last_available_width.unwrap_or(COMPOSER_MAX_WIDTH) < 520.0 {
            1
        } else {
            2
        }
    }

    /// Open a staged image full size, from its chip (the tiles' click).
    fn open_attachment(&mut self, index: u32, cx: &mut Context<Self>) {
        let Some((name, image)) = self
            .staged()
            .iter()
            .filter(|att| att.mention == Some(index))
            .find_map(|att| Some((att.name.clone(), att.image()?.clone())))
        else {
            return;
        };
        let preview = attachments::PreviewImage::new(name, image);
        preview.viewer.reset();
        self.preview = Some(preview);
        self.preview_focus_pending = true;
        cx.notify();
    }

    pub(crate) fn show_queue_image(
        &mut self,
        preview: attachments::PreviewImage,
        cx: &mut Context<Self>,
    ) {
        self.preview = Some(preview);
        self.preview_focus_pending = true;
        cx.notify();
    }

    pub(crate) fn staged_appshots(&self) -> &[CapturedAppshot] {
        self.appshots
            .get(&self.current_key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn stage_appshot(&mut self, appshot: CapturedAppshot, cx: &mut Context<Self>) {
        self.stage_appshot_for(self.current_key.clone(), appshot, cx);
    }

    pub fn stage_appshot_for(
        &mut self,
        key: String,
        appshot: CapturedAppshot,
        cx: &mut Context<Self>,
    ) -> bool {
        let staged_bytes = self
            .appshots
            .get(&key)
            .into_iter()
            .flatten()
            .map(|shot| shot.screenshot.bytes().len() as u64)
            .sum::<u64>();
        let saved_bytes = self
            .queue_edit_draft
            .as_ref()
            .filter(|_| key == self.current_key)
            .map(|(_, _, shots)| {
                shots
                    .iter()
                    .map(|shot| shot.screenshot.bytes().len() as u64)
                    .sum::<u64>()
            })
            .unwrap_or_default();
        let staged_bytes = staged_bytes.saturating_add(saved_bytes);
        let incoming = appshot.screenshot.bytes().len() as u64;
        if incoming > attachments::MAX_ATTACHMENT_BYTES
            || staged_bytes.saturating_add(incoming) > appshots::MAX_STAGED_APPSHOT_BYTES
        {
            self.failure = Some(
                "Remove an Appshot before adding another (96 MB staged Appshot limit).".into(),
            );
            self.failure_key = Some(key);
            cx.notify();
            return false;
        }
        self.appshot_entrances
            .retain(|_, start| start.elapsed().as_secs_f32() < motion::speed_scale());
        if !motion::reduced_motion(cx) {
            self.appshot_entrances
                .insert(appshot.id.clone(), Instant::now());
        }
        // A capture completing while a queue edit is being saved belongs to
        // the displaced draft; it must not be lost or change the in-flight edit.
        if self.queue_edit_finishing && key == self.current_key {
            if let Some((_, _, saved_appshots)) = &mut self.queue_edit_draft {
                saved_appshots.push(appshot);
            } else {
                self.appshots.entry(key).or_default().push(appshot);
            }
        } else {
            // A key the composer has not switched to yet is reloaded from the
            // draft store on the switch, which would replace this bucket:
            // record the capture there too so the swap brings it back.
            if key != self.current_key {
                let mut entry = crate::draft_store::entry(&key, cx);
                entry.appshots.push(appshot.clone());
                crate::draft_store::set(&key, entry, cx);
            }
            self.appshots.entry(key).or_default().push(appshot);
        }
        self.failure = None;
        self.failure_key = None;
        self.sync_draft(cx);
        cx.notify();
        true
    }

    pub fn show_appshot_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.show_error(message, cx);
    }

    /// Mark this as a side chat's composer: below the input it shows only
    /// the context ring (no checkout/ref footer, no plan usage).
    pub(crate) fn set_side_chat(&mut self, cx: &mut Context<Self>) {
        self.side_chat = true;
        cx.notify();
    }

    pub(crate) fn set_pr_chat(&mut self, placeholder: &'static str, cx: &mut Context<Self>) {
        self.side_chat = true;
        self.pr_chat = true;
        self.pickers
            .update(cx, |pickers, cx| pickers.set_asks_only(cx));
        self.idle_placeholder = placeholder.into();
        if self.wizard.is_none() {
            self.input
                .update(cx, |input, cx| input.set_placeholder(placeholder, cx));
        }
        cx.notify();
    }

    pub(crate) fn set_pr_context(&mut self, block: Option<String>) {
        self.pr_context = block;
    }

    /// Whether the draft holds anything a close would lose: text, staged
    /// attachments or appshots, or staged review comments.
    pub(crate) fn has_draft(&self, cx: &App) -> bool {
        composer_has_content(
            self.input.read(cx).text(),
            self.staged().len() + self.staged_appshots().len(),
            self.staged_comments(cx).len(),
        )
    }

    #[cfg(test)]
    pub(crate) fn failure(&self) -> Option<&SharedString> {
        self.failure.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn input_text(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    #[cfg(test)]
    pub(crate) fn placeholder(&self, cx: &App) -> SharedString {
        self.input.read(cx).placeholder.clone()
    }

    /// The failure notice this draft renders. Chat-scoped failures render
    /// only under their own chat; a global failure (no key) renders everywhere.
    pub(crate) fn visible_failure(&self) -> Option<SharedString> {
        self.failure.clone().filter(|_| {
            self.failure_key
                .as_ref()
                .is_none_or(|key| *key == self.current_key)
        })
    }

    /// Show a dismissable failure chip for the current draft's session.
    pub(crate) fn show_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.failure = Some(message.into());
        self.failure_key = Some(self.current_key.clone());
        cx.notify();
    }

    fn add_staged(&mut self, staged: Vec<StagedAttachment>, cx: &mut Context<Self>) {
        if self.queue_edit_finishing {
            return;
        }
        if staged.is_empty() {
            return;
        }
        self.attachments
            .entry(self.current_key.clone())
            .or_default()
            .extend(staged);
        self.focus_pending = true;
        self.sync_draft(cx);
        cx.notify();
    }

    /// Run `stage` on the background executor — reading a file and converting
    /// a BMP are too slow for the UI thread — and add what it staged to the
    /// draft that is current now, even if the user has navigated away by the
    /// time it finishes. Failures surface in that draft's failure notice, all
    /// of them together so one refused file does not hide another.
    fn stage_in_background(
        &mut self,
        stage: impl FnOnce() -> Vec<Result<StagedAttachment, String>> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let key = self.current_key.clone();
        cx.spawn(async move |this, cx| {
            let results = cx.background_executor().spawn(async move { stage() }).await;
            this.update(cx, |this, cx| {
                let mut staged = Vec::new();
                let mut refused = Vec::new();
                for result in results {
                    match result {
                        Ok(att) => staged.push(att),
                        Err(message) => refused.push(message),
                    }
                }
                if !refused.is_empty() {
                    this.failure = Some(refused.join(" ").into());
                    this.failure_key = Some(key.clone());
                }
                if !staged.is_empty() && !this.queue_edit_finishing {
                    this.adopt_staged_attachments(key.clone(), staged, cx);
                    this.focus_pending = true;
                    if key == this.current_key {
                        this.sync_draft(cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Stage files (picker / drop / pasted paths): images with thumbnails, any
    /// other file as opaque bytes. Folders, read failures and oversize files
    /// surface in the failure notice.
    pub(crate) fn add_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.stage_in_background(
            move || {
                paths
                    .iter()
                    .map(|path| attachments::stage_file(path))
                    .collect()
            },
            cx,
        );
    }

    /// Add a file-tree or file-tab drop through the existing file-mention
    /// pipeline. This keeps the reference workspace-relative and therefore
    /// valid for local and remote sessions alike.
    pub(crate) fn add_workspace_path(
        &mut self,
        path: &str,
        is_directory: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !local_path_is_safe(path) {
            return;
        }
        self.insert_link(local_file_link(path, is_directory), window, cx);
    }

    /// A sidebar row or strip tab dropped on the composer becomes a thread
    /// mention chip. The open chat cannot mention itself (`read_thread` on
    /// the caller is meaningless), and a draft has no transcript to read.
    pub(crate) fn add_thread_mention(
        &mut self,
        chat_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((device, bound)) = self.mention_device(cx) else {
            return;
        };
        if bound.as_deref() == Some(chat_id) {
            self.failure = Some("This is the open thread; it can't mention itself.".into());
            self.failure_key = bound;
            cx.notify();
            return;
        }
        let state = self.state.read(cx);
        if state.is_draft(chat_id) || !chat_id_is_safe(chat_id) {
            return;
        }
        let Some(chat) = state
            .chats
            .iter()
            .find(|chat| chat.id == chat_id && chat.device_id == device)
        else {
            return;
        };
        let link = thread_link(chat_id, &chat_mention_title(chat));
        if self.insert_link(link, window, cx) {
            cx.emit(ComposerEvent::ThreadMentionDropped);
        }
    }

    /// The device a thread mention must live on, plus the chat it must not
    /// be: the bound chat's device, or the selected device for a draft.
    fn mention_device(&self, cx: &App) -> Option<(String, Option<String>)> {
        if let Some(chat) = self.bound_chat_row(cx) {
            return Some((chat.device_id.clone(), Some(chat.id.clone())));
        }
        self.state
            .read(cx)
            .selected_device
            .clone()
            .map(|device| (device, None))
    }

    /// The project whose threads the picker lists first: the composed-for
    /// chat's, else the one the new-session canvas would mint into.
    fn mention_space(&self, cx: &App) -> Option<String> {
        if let Some(chat) = self.bound_chat_row(cx) {
            return chat.space_id.clone();
        }
        self.state
            .read(cx)
            .selected_space_row()
            .map(|space| space.id.clone())
    }

    fn insert_link(&mut self, link: String, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // A drop while the `@` picker is open completes that token instead
        // of leaving a stray `@` beside the chip.
        let token = self.mention.token.clone();
        let inserted = self.input.update(cx, |input, cx| match token {
            Some(token) => {
                input.replace_mention_link(token.range, &link, cx);
                true
            }
            None => input.insert_reference(&link, cx),
        });
        if inserted {
            self.reset_mention(None, cx);
            self.reset_slash(None, cx);
            let focus = self.input.read(cx).focus_handle.clone();
            window.focus(&focus, cx);
            cx.notify();
        }
        inserted
    }

    /// The next chip number for a draft: above everything it holds or
    /// mentions, so a number is never reused.
    fn next_attachment_index(&mut self, key: &str, text: &str) -> u32 {
        let held = self
            .attachments
            .get(key)
            .into_iter()
            .flatten()
            .filter_map(|att| att.mention)
            .max();
        let mentioned = zeron_proto::attachment_mentions::attachment_mention_indices(text)
            .into_iter()
            .max();
        let draft = self.attachment_drafts.entry(key.to_string()).or_default();
        draft.next = draft
            .next
            .max(held.unwrap_or(0))
            .max(mentioned.unwrap_or(0))
            + 1;
        draft.next
    }

    /// Add newly staged files to a draft. Each is numbered and mentioned at the
    /// caret as one edit: images read "Image 1", "Image 2", ...; other files
    /// keep their name. A file the editor cannot take a chip for, or that was
    /// staged for a draft the user has since left, stays a plain attachment.
    fn adopt_staged_attachments(
        &mut self,
        key: String,
        mut staged: Vec<StagedAttachment>,
        cx: &mut Context<Self>,
    ) {
        if key != self.current_key {
            self.attachments.entry(key).or_default().extend(staged);
            return;
        }
        let text = self.input.read(cx).text().to_string();
        let mut links = Vec::with_capacity(staged.len());
        let mut indices = Vec::with_capacity(staged.len());
        for att in &mut staged {
            let index = self.next_attachment_index(&key, &text);
            let file_name = match att.image() {
                Some(image) => {
                    att.name = attachments::ensure_extension(
                        &zeron_proto::attachment_mentions::image_label(index),
                        image.format,
                    );
                    None
                }
                None => Some(att.name.as_str()),
            };
            links.push(zeron_proto::attachment_mentions::attachment_mention_link(
                index, file_name,
            ));
            indices.push(index);
        }
        // Chips are inserted before the attachments join the draft, so a
        // draft that cannot take them (read-only, composing) keeps the files
        // and their original names.
        let mentioned = self
            .input
            .update(cx, |input, cx| input.insert_reference(&links.join(" "), cx));
        if mentioned {
            for (att, index) in staged.iter_mut().zip(indices) {
                att.mention = Some(index);
            }
        }
        self.attachments.entry(key).or_default().extend(staged);
        self.sync_attachment_chips(cx);
        cx.notify();
    }

    /// Hand the editor the attachments its chips may reference.
    fn sync_attachment_chips(&mut self, cx: &mut Context<Self>) {
        let attachments: HashMap<u32, Option<std::sync::Arc<gpui::Image>>> = self
            .staged()
            .iter()
            .filter_map(|att| att.mention.map(|index| (index, att.image().cloned())))
            .collect();
        let scope = self.current_key.clone();
        self.input.update(cx, |input, cx| {
            input.set_attachment_chips(&scope, attachments, cx)
        });
    }

    /// Keep staged attachments and their chips in step: one whose last chip is
    /// gone leaves the draft (kept for undo), and one whose chip came back
    /// returns. Attachments the prompt never mentions are left alone.
    fn reconcile_attachment_mentions(&mut self, cx: &mut Context<Self>) {
        if self.wizard.is_none() && !self.queue_edit_finishing {
            let mentioned = zeron_proto::attachment_mentions::attachment_mention_indices(
                self.input.read(cx).text(),
            );
            let is_mentioned =
                |att: &StagedAttachment| att.mention.is_none_or(|index| mentioned.contains(&index));
            let key = self.current_key.clone();
            let draft = self.attachment_drafts.entry(key.clone()).or_default();
            let (restored, mut stash): (Vec<_>, Vec<_>) = std::mem::take(&mut draft.unstaged)
                .into_iter()
                .partition(is_mentioned);
            let (mut kept, dropped): (Vec<_>, Vec<_>) = self
                .attachments
                .remove(&key)
                .unwrap_or_default()
                .into_iter()
                .partition(is_mentioned);
            let changed = !restored.is_empty() || !dropped.is_empty();
            kept.extend(restored);
            kept.sort_by_key(|att| att.mention.unwrap_or(0));
            stash.extend(dropped);
            draft.unstaged = stash;
            if !kept.is_empty() {
                self.attachments.insert(key, kept);
            }
            if changed {
                cx.notify();
            }
        }
        self.sync_attachment_chips(cx);
    }

    /// Set the draft's chip numbering and undo stash aside while a queued
    /// message occupies the composer.
    pub(crate) fn displace_attachment_draft(&mut self) {
        self.queue_edit_attachment_draft = Some(
            self.attachment_drafts
                .remove(&self.current_key)
                .unwrap_or_default(),
        );
    }

    /// Bring back what [`Self::displace_attachment_draft`] set aside; the
    /// edit's own numbering and stash are dropped with it.
    pub(crate) fn restore_attachment_draft(&mut self) {
        if let Some(draft) = self.queue_edit_attachment_draft.take() {
            self.attachment_drafts
                .insert(self.current_key.clone(), draft);
        }
    }

    fn remove_attachment(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.queue_edit_finishing {
            return;
        }
        let mut mention = None;
        if let Some(list) = self.attachments.get_mut(&self.current_key) {
            if let Some(at) = list.iter().position(|a| a.id == id) {
                let removed = list.remove(at);
                if let Some(index) = removed.mention {
                    mention = Some(index);
                    // Undo of the chip removal below restages this image.
                    self.attachment_drafts
                        .entry(self.current_key.clone())
                        .or_default()
                        .unstaged
                        .push(removed);
                }
            }
            if list.is_empty() {
                self.attachments.remove(&self.current_key);
            }
        }
        if let Some(index) = mention {
            self.input
                .update(cx, |input, cx| input.remove_attachment_chips(index, cx));
        }
        self.sync_attachment_chips(cx);
        self.sync_draft(cx);
        cx.notify();
    }

    fn restore_failed_appshots(
        &mut self,
        sent: &[CapturedAppshot],
        failed_key: &str,
        restore_key: &str,
    ) {
        if sent.is_empty() {
            return;
        }
        let mut merged = sent.to_vec();
        for key in [failed_key, restore_key] {
            for shot in self.appshots.remove(key).unwrap_or_default() {
                if !merged.iter().any(|existing| existing.id == shot.id) {
                    merged.push(shot);
                }
            }
        }
        self.appshots.insert(restore_key.to_string(), merged);
    }

    fn remove_appshot(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.queue_edit_finishing {
            return;
        }
        if let Some(list) = self.appshots.get_mut(&self.current_key) {
            list.retain(|appshot| appshot.id != id);
            if list.is_empty() {
                self.appshots.remove(&self.current_key);
            }
        }
        self.sync_draft(cx);
        cx.notify();
    }

    /// Drop a deleted chat's per-chat composer state — staged attachments hold
    /// raw image bytes, and a deleted chat's stage could never be sent again.
    pub fn purge_chat(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.attachments.remove(chat_id);
        self.attachment_drafts.remove(chat_id);
        self.appshots.remove(chat_id);
        if crate::draft_store::remove(chat_id, cx) {
            cx.emit(ComposerEvent::UnsentChanged);
        }
        self.state.update(cx, |state, _| {
            state.purge_review_comments(chat_id);
            state.purge_quotes(chat_id);
        });
    }

    /// Staged in `AppState` because the changes pane writes them.
    fn staged_comments(&self, cx: &App) -> Vec<crate::comments::ReviewComment> {
        self.state
            .read(cx)
            .review_comments(&self.current_key)
            .to_vec()
    }

    /// Staged in `AppState` because the transcript's "Add to chat" writes them.
    fn staged_quotes(&self, cx: &App) -> Vec<crate::quotes::Quote> {
        self.state.read(cx).quotes(&self.current_key).to_vec()
    }

    fn staged_chip_count(&self, cx: &App) -> usize {
        self.staged_comments(cx).len() + self.staged_quotes(cx).len()
    }

    fn render_staged_chips(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let comments = self.staged_comments(cx).len();
        let quotes = self.staged_quotes(cx);
        if comments == 0 && quotes.is_empty() {
            return None;
        }
        let mut row = div()
            .flex()
            .flex_row()
            .gap(px(STRIP_GAP))
            .px(px(STRIP_PAD_X))
            .pt(px(STRIP_PAD_TOP));
        if comments > 0 {
            row = row.child(crate::badges::render(
                "composer-comments",
                &crate::badges::MessageBadge {
                    icon: crate::icons::CHAT_ROUND_LINE,
                    label: crate::comments::chip_label(comments).into(),
                    // The staged set is already on screen in the changes
                    // pane, so a hover card would only repeat it.
                    details: Vec::new(),
                },
                theme,
            ));
        }
        if !quotes.is_empty() {
            let details = quotes
                .iter()
                .map(|quote| crate::badges::BadgeDetail {
                    location: quote.location().into(),
                    tag: None,
                    body: quote.text.clone().into(),
                })
                .collect();
            let group: SharedString = "composer-quotes".into();
            let key = self.current_key.clone();
            row = row.child(
                div()
                    .group(group.clone())
                    .flex_none()
                    .relative()
                    .child(crate::badges::render(
                        "composer-quotes",
                        &crate::badges::MessageBadge {
                            icon: crate::icons::WRAP_TEXT,
                            label: crate::quotes::chip_label(quotes.len()).into(),
                            details,
                        },
                        theme,
                    ))
                    .child(
                        div()
                            .id("composer-quotes-remove")
                            .absolute()
                            .top(px(-6.0))
                            .right(px(-6.0))
                            .size(px(18.0))
                            .rounded_full()
                            .bg(theme.bg)
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .shadow_sm()
                            .opacity(0.0)
                            .group_hover(group, |s| s.opacity(1.0))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.state.update(cx, |state, cx| {
                                    state.purge_quotes(&key);
                                    cx.notify();
                                });
                                cx.notify();
                            }))
                            .child(
                                crate::icons::icon(crate::icons::CLOSE_CIRCLE)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted),
                            ),
                    ),
            );
        }
        Some(row)
    }

    /// The staged-thumbnail strip (attachment-ui.tsx AttachmentStrip):
    /// `flex flex-wrap gap-2 px-4 pt-3`, 56px rounded thumbs, a remove button
    /// revealed on hover, click opens the full-size preview. Attachments with
    /// a chip in the draft are left out.
    fn render_attachment_strip(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let staged = self.tiled_attachments(cx);
        if staged.is_empty() {
            return None;
        }
        let mut strip = div()
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(STRIP_GAP))
            .px(px(STRIP_PAD_X))
            .pt(px(STRIP_PAD_TOP));
        for (ix, att) in staged.iter().enumerate() {
            let group: SharedString = format!("composer-att-{}", att.id).into();
            let remove_id = att.id.clone();
            let remove_label: SharedString = format!("Remove {}", att.name).into();
            strip = strip.child(
                div()
                    .group(group.clone())
                    .flex_none()
                    .relative()
                    .child(self.attachment_tile(att, ix, cx))
                    // Own layer: inside the frosted pill everything shares one
                    // draw order and images render last, so without it the
                    // thumbnail paints OVER this button (user report).
                    .child(crate::frost::layered(
                        div()
                            .id(("composer-att-remove", ix))
                            .absolute()
                            .top(px(-6.0))
                            .right(px(-6.0))
                            .size(px(18.0))
                            .rounded_full()
                            .bg(theme.bg)
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .shadow_sm()
                            .opacity(0.0)
                            .group_hover(group, |s| s.opacity(1.0))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                // The button overhangs the thumbnail, whose
                                // hitbox is right underneath — don't let the
                                // same click also open the preview.
                                cx.stop_propagation();
                                this.remove_attachment(&remove_id, cx);
                            }))
                            .tooltip(move |_, cx| {
                                cx.new(|_| AppshotActionTooltip(remove_label.clone()))
                                    .into()
                            })
                            .child(
                                crate::icons::icon(crate::icons::CLOSE_CIRCLE)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted),
                            ),
                    )),
            );
        }
        Some(strip)
    }

    /// One staged attachment's tile: an image's thumbnail (a click opens the
    /// full-size preview) or a file's icon and name.
    fn attachment_tile(
        &self,
        att: &StagedAttachment,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let frame = div()
            .id(("composer-att-thumb", ix))
            .size(px(STRIP_THUMB))
            .rounded(px(8.0))
            .overflow_hidden()
            .border_1()
            .border_color(crate::theme::hairline(0.10));
        let Some(image) = att.image() else {
            let theme = Theme::of(cx);
            return frame
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(3.0))
                .px(px(4.0))
                .child(
                    crate::file_icons::icon(
                        crate::file_icons::FileIconIdentity::file(&att.name),
                        theme.appearance,
                    )
                    .size(px(24.0)),
                )
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_center()
                        .text_size(px(10.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(att.name.clone())),
                )
                .into_any_element();
        };
        let preview = attachments::PreviewImage::new(att.name.clone(), image.clone());
        frame
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                preview.viewer.reset();
                this.preview = Some(preview.clone());
                this.preview_focus_pending = true;
                cx.notify();
            }))
            .child(
                img(image.clone())
                    // EXPLICIT dims, not size_full: img layout honors the
                    // image's intrinsic aspect ratio over a percent height
                    // (gpui f8d8a90 repoint), so size_full let a tall photo
                    // grow past the frame — the rectangular overflow clip
                    // then squared the bottom corners (2026-08-19 report).
                    // 56−2 = frame minus its 1px borders.
                    .w(px(STRIP_THUMB - 2.0))
                    .h(px(STRIP_THUMB - 2.0))
                    // Own radii — the frame's rounding only clips
                    // rectangularly (7 = 8 - border).
                    .rounded(px(7.0))
                    .object_fit(ObjectFit::Cover),
            )
            .into_any_element()
    }

    fn render_appshot_strip(
        &self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let appshots = self.staged_appshots();
        if appshots.is_empty() {
            return None;
        }
        let mut strip = div()
            .id("composer-appshots-strip")
            .flex()
            .flex_row()
            .gap(px(STRIP_GAP))
            .px(px(STRIP_PAD_X))
            .pt(px(STRIP_PAD_TOP))
            .overflow_x_scroll();
        for (ix, appshot) in appshots.iter().enumerate() {
            let Some(screenshot) = appshot.screenshot.image().cloned() else {
                continue;
            };
            let group: SharedString = format!("composer-appshot-{}", appshot.id).into();
            let hover_key = group.to_string();
            let preview = crate::attachments::PreviewImage::new(
                appshot.screenshot.name.clone(),
                screenshot.clone(),
            );
            let preview_on_key = preview.clone();
            let preview_on_a11y = preview.clone();
            let composer_for_preview = cx.entity().downgrade();
            let remove_id = appshot.id.clone();
            let remove_on_key_id = remove_id.clone();
            let remove_on_a11y_id = remove_id.clone();
            let composer_for_remove = cx.entity().downgrade();
            let app_name = if appshot.app_name.trim().is_empty() {
                "Captured app".to_string()
            } else {
                appshot.app_name.trim().to_string()
            };
            let provenance: SharedString =
                appshot_provenance(&appshot.app_name, appshot.window_title.as_deref()).into();
            let preview_label: SharedString = format!("Preview Appshot from {app_name}").into();
            let remove_label: SharedString = format!("Remove Appshot from {app_name}").into();
            let preview_aria = preview_label.clone();
            let remove_aria = remove_label.clone();
            let tooltip_label = provenance.clone();
            let (image_width, image_height) = appshot_contained_size(appshot.screenshot_dimensions);
            // `transition-colors duration-150`: the border lifts from the light
            // hairline to the full border while hovered.
            let hover_t = motion::hover_t(&hover_key);
            let border = motion::mix(theme.border.opacity(0.55), theme.border, hover_t);
            let mut card = div()
                .id(("composer-appshot", ix))
                .group(group.clone())
                .relative()
                .w(px(APPSHOT_TILE_WIDTH))
                .h(px(APPSHOT_TILE_HEIGHT))
                .flex_none()
                .rounded(px(12.0))
                .border_1()
                .border_color(border)
                .bg(theme.surface_raised)
                .overflow_hidden()
                .cursor_pointer()
                .on_hover(motion::hover_listener(hover_key.clone()))
                .tooltip(move |_, cx| {
                    cx.new(|_| AppshotActionTooltip(tooltip_label.clone()))
                        .into()
                })
                .role(gpui::Role::Button)
                .aria_label(preview_aria)
                .tab_index(0)
                .focus_visible(|style| style.border_2().border_color(theme.accent))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        preview_on_key.viewer.reset();
                        this.preview = Some(preview_on_key.clone());
                        this.preview_focus_pending = true;
                        cx.notify();
                    }
                }))
                .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
                    composer_for_preview
                        .update(cx, |this, cx| {
                            preview_on_a11y.viewer.reset();
                            this.preview = Some(preview_on_a11y.clone());
                            this.preview_focus_pending = true;
                            cx.notify();
                        })
                        .ok();
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    preview.viewer.reset();
                    this.preview = Some(preview.clone());
                    this.preview_focus_pending = true;
                    cx.notify();
                }))
                .child(
                    div()
                        .id(("composer-appshot-preview", ix))
                        .size_full()
                        .p(px(APPSHOT_IMAGE_INSET))
                        .flex()
                        .items_center()
                        .justify_center()
                        .overflow_hidden()
                        .child(
                            div()
                                .w(px(image_width))
                                .h(px(image_height))
                                .flex_none()
                                .overflow_hidden()
                                .rounded(px(6.0))
                                .child(
                                    img(screenshot)
                                        .w(px(image_width))
                                        .h(px(image_height))
                                        .object_fit(ObjectFit::Contain),
                                ),
                        ),
                );
            // `bg-linear-to-t from-black/90 via-black/55 to-transparent` over
            // `pt-10 pb-2`: two stacked stops, since one gradient carries two.
            let scrim = |from: f32, to: f32| {
                div().w_full().flex_1().bg(gpui::linear_gradient(
                    180.0,
                    gpui::linear_color_stop(gpui::black().opacity(from), 0.0),
                    gpui::linear_color_stop(gpui::black().opacity(to), 1.0),
                ))
            };
            let source_icon = match &appshot.app_icon {
                Some(icon) => img(icon.clone())
                    .size(px(20.0))
                    .flex_none()
                    .rounded(px(5.0))
                    .object_fit(ObjectFit::Contain)
                    .into_any_element(),
                None => div()
                    .size(px(20.0))
                    .flex_none()
                    .rounded(px(5.0))
                    .bg(gpui::white().opacity(0.15))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        crate::icons::icon(crate::icons::MONITOR)
                            .size(px(12.0))
                            .text_color(gpui::white().opacity(0.85)),
                    )
                    .into_any_element(),
            };
            card = card.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(68.0))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .flex_col()
                            .child(scrim(0.0, 0.55))
                            .child(scrim(0.55, 0.9)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .px(px(10.0))
                            .pb(px(8.0))
                            .flex()
                            .items_end()
                            .gap(px(8.0))
                            .child(source_icon)
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .pb(px(1.0))
                                    .truncate()
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(gpui::white())
                                    .child(provenance),
                            ),
                    ),
            );
            // `AttachmentRemoveButton size="sm"`: a 14px solid disc at 70%,
            // full opacity while the card or the disc is hovered.
            let remove_opacity = motion::lerp(0.7, 1.0, hover_t);
            card = card.child(
                div()
                    .id(("composer-appshot-remove", ix))
                    .absolute()
                    .top(px(4.0))
                    .right(px(4.0))
                    .size(px(14.0))
                    .rounded_full()
                    .bg(theme.text.opacity(0.8))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .shadow_sm()
                    .opacity(remove_opacity)
                    .hover(|style| style.bg(theme.text))
                    .tooltip(move |_, cx| {
                        cx.new(|_| AppshotActionTooltip(remove_label.clone()))
                            .into()
                    })
                    .role(gpui::Role::Button)
                    .aria_label(remove_aria)
                    .tab_index(0)
                    .focus_visible(|style| style.opacity(1.0).border_1().border_color(theme.accent))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.remove_appshot(&remove_on_key_id, cx);
                        }
                    }))
                    .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
                        composer_for_remove
                            .update(cx, |this, cx| {
                                this.remove_appshot(&remove_on_a11y_id, cx);
                            })
                            .ok();
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.remove_appshot(&remove_id, cx);
                    }))
                    .child(
                        crate::icons::icon(crate::icons::CLOSE)
                            .size(px(10.0))
                            .text_color(theme.bg),
                    ),
            );
            // Entity-owned timestamps prevent the entrance replaying on route remount.
            if let Some(start) = self.appshot_entrances.get(&appshot.id) {
                let raw = (start.elapsed().as_secs_f32() / (0.24 * motion::speed_scale()))
                    .clamp(0.0, 1.0);
                if raw < 1.0 && !motion::reduced_motion(cx) {
                    let progress =
                        motion::MotionSpec::new(240, motion::EASE_OUT_EXPO).progress(raw);
                    card = card.opacity(progress).top(px(8.0 * (1.0 - progress)));
                    window.request_animation_frame();
                }
            }
            strip = strip.child(card);
        }
        Some(strip.into_any_element())
    }

    /// Paperclip: the native image picker (the original's hidden
    /// `<input type=file accept=image/* multiple>`).
    fn open_file_picker(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        self.picker_task = Some(cx.spawn(async move |this, cx| {
            let result = rx.await;
            this.update(cx, |composer, cx| {
                if let Ok(Ok(Some(paths))) = result {
                    composer.add_paths(paths, cx);
                }
                // Both Attach and Cancel return to the draft.
                composer.focus_pending = true;
                cx.notify();
            })
            .ok();
        }));
    }

    fn sync_mention_controls(&mut self, cx: &mut Context<Self>) {
        let open = self.mention.token.is_some() || self.slash.token.is_some();
        let has_selection = if self.slash.token.is_some() {
            self.slash.active.is_some()
        } else {
            self.mention.active.is_some()
        };
        self.input.update(cx, |input, cx| {
            input.set_mention_controls(open, has_selection, cx)
        });
    }

    /// Tear down the entire completion lifecycle. Advancing the generation is
    /// important even when the spawned task is dropped: an RPC response may
    /// already be queued for delivery on the UI executor.
    fn reset_mention(&mut self, dismissed: Option<(Range<usize>, String)>, cx: &mut Context<Self>) {
        let request = self.mention.request.wrapping_add(1);
        self.mention_task = None;
        self.mention = FileMentionState {
            context: self.mention.context.clone(),
            request,
            dismissed,
            ..FileMentionState::default()
        };
        self.sync_mention_controls(cx);
    }

    /// File mentions and provider catalogs share a workspace target. A new
    /// side chat inherits its parent's checkout but has no persisted row until
    /// its first send, so discovery must address the parent in the meantime.
    fn completion_workspace_params(&self, cx: &App) -> Option<serde_json::Value> {
        let state = self.state.read(cx);
        if let Some(chat) = self.bound_chat_row(cx) {
            let unsaved = state.side_chat_unsaved();
            if unsaved && chat.created_by_chat_id.is_none() {
                let mut params = serde_json::json!({
                    "spaceId": chat.space_id.clone()?,
                    "targetDeviceId": chat.device_id,
                    "cwd": chat.cwd,
                });
                let root = chat
                    .space_id
                    .as_deref()
                    .and_then(|id| state.space_row(id))
                    .map(|space| space.path.as_str());
                if let Some(cwd) = chat.cwd.as_deref().filter(|cwd| Some(*cwd) != root) {
                    params["path"] = cwd.into();
                }
                return Some(params);
            }
            let chat_id = chat
                .created_by_chat_id
                .as_deref()
                .filter(|_| unsaved)
                .unwrap_or(&chat.id);
            Some(serde_json::json!({
                "chatId": chat_id,
                "targetDeviceId": chat.device_id,
                // Include the inherited cwd in the cache identity, too.
                "cwd": chat.cwd,
            }))
        } else if let Some(space) = state.selected_space_row() {
            let mut params = serde_json::json!({
                "spaceId": space.id,
                "targetDeviceId": space.device_id,
                "cwd": space.path,
            });
            if let crate::pickers::CheckoutPlan::ReuseWorktree { path, .. } =
                self.pickers.read(cx).checkout_plan()
            {
                params["path"] = path.into();
            }
            Some(params)
        } else {
            None
        }
    }

    fn file_search_params(&self, query: &str, cx: &App) -> Option<serde_json::Value> {
        let mut params = self.completion_workspace_params(cx)?;
        params["query"] = query.into();
        Some(params)
    }

    fn catalog_params(&self, cx: &App) -> serde_json::Value {
        let mut params = self.completion_workspace_params(cx).unwrap_or_else(|| {
            let mut params = serde_json::json!({});
            if let Some(device) = self.state.read(cx).effective_device_id() {
                params["targetDeviceId"] = device.into();
            }
            params
        });
        // The workspace can come from the parent; the agent always comes from
        // this composer's selection, including changes before the first send.
        params["harness"] = serde_json::json!(self.pickers.read(cx).resolved(cx).harness);
        params
    }

    fn completion_connection_context(&self, cx: &App) -> String {
        let state = self.state.read(cx);
        let engine = state
            .engine()
            .map(|engine| engine.client() as *const _ as usize);
        let target = self
            .bound_chat_row(cx)
            .map(|chat| chat.device_id.clone())
            .or_else(|| {
                state
                    .selected_space_row()
                    .map(|space| space.device_id.clone())
            })
            .or_else(|| state.effective_device_id());
        let online = target
            .as_deref()
            .is_none_or(|device| state.device_online(device, chrono::Utc::now()));
        format!(
            "{engine:?}:{:?}:{:?}:{online}",
            state.connection, state.connectivity.state
        )
    }

    fn resolve_pasted_references(
        &mut self,
        range: Range<usize>,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        let original = self.input.read(cx).text().to_owned();
        let tokens = pasted_reference_tokens(&original, range.clone());
        if tokens.is_empty() {
            return;
        }
        let params = self.catalog_params(cx);
        let file_params = self.file_search_params("", cx);
        if !params["targetDeviceId"].as_str().is_some_and(|target| {
            self.state
                .read(cx)
                .device_supports(target, capabilities::COMPOSER_REFERENCES_V1)
        }) {
            return;
        }
        let harness = self.pickers.read(cx).resolved(cx).harness;
        let preferences =
            crate::settings::current(cx).skill_completion(harness.unwrap_or(HarnessId::Codex));
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let context = format!(
            "{}:{params}:{preferences:?}",
            self.completion_connection_context(cx)
        );
        cx.spawn(async move |this, cx| {
            let mut candidates = Vec::new();
            if tokens.iter().any(|(prefix, _)| *prefix != '@') && harness.is_some() {
                let commands = async {
                    engine
                        .client()
                        .call(methods::LIST_COMMANDS, params.clone())
                        .await
                        .ok()
                        .and_then(|v| serde_json::from_value::<Vec<SlashCommand>>(v).ok())
                        .unwrap_or_default()
                };
                let skills = async {
                    engine
                        .client()
                        .call(methods::LIST_SKILLS, params.clone())
                        .await
                        .ok()
                        .and_then(|v| {
                            serde_json::from_value::<Option<Vec<zeron_proto::invocation::Skill>>>(v)
                                .ok()
                        })
                        .flatten()
                        .unwrap_or_default()
                };
                let (commands, skills) = futures::join!(commands, skills);
                candidates = invocation_candidates(commands, skills);
            }
            let mut replacements = Vec::new();
            let mut files = HashMap::new();
            for (prefix, token) in tokens {
                if this
                    .update(cx, |this, cx| this.input.read(cx).edit_revision != revision)
                    .unwrap_or(true)
                {
                    return;
                }
                let replacement = if prefix == '@' {
                    if !files.contains_key(&token.query) {
                        let Some(mut search) = file_params.clone() else {
                            continue;
                        };
                        search["query"] = token.query.clone().into();
                        let results = engine
                            .client()
                            .call(methods::SEARCH_FILES, search)
                            .await
                            .ok()
                            .and_then(|v| serde_json::from_value::<Vec<FileSearchMatch>>(v).ok())
                            .unwrap_or_default();
                        let exact: Vec<_> = results
                            .into_iter()
                            .filter(|file| {
                                file.path.trim_end_matches('/') == token.query.trim_end_matches('/')
                                    && local_path_is_safe(&file.path)
                            })
                            .collect();
                        files.insert(
                            token.query.clone(),
                            (exact.len() == 1)
                                .then(|| local_file_link(&exact[0].path, exact[0].is_dir)),
                        );
                    }
                    files.get(&token.query).cloned().flatten()
                } else if prefix == '$' && !preferences.dollar {
                    None
                } else {
                    let exact: Vec<_> = candidates
                        .iter()
                        .filter(|candidate| {
                            candidate.invocation.prefix() == prefix && candidate.name == token.query
                        })
                        .collect();
                    (exact.len() == 1).then(|| exact[0].invocation.link())
                };
                if let Some(replacement) = replacement {
                    replacements.push((token.range, replacement));
                }
            }
            if replacements.is_empty() {
                return;
            }
            this.update(cx, |this, cx| {
                let harness = this.pickers.read(cx).resolved(cx).harness;
                let preferences = crate::settings::current(cx)
                    .skill_completion(harness.unwrap_or(HarnessId::Codex));
                let current = format!(
                    "{}:{}:{preferences:?}",
                    this.completion_connection_context(cx),
                    this.catalog_params(cx)
                );
                if current != context {
                    return;
                }
                this.input.update(cx, |input, cx| {
                    input.apply_pasted_references(&original, range.end, revision, replacements, cx);
                });
            })
            .ok();
        })
        .detach();
    }

    fn on_input_edited(&mut self, cx: &mut Context<Self>) {
        if self
            .prompt_history
            .as_ref()
            .is_some_and(|cursor| cursor.shown != self.input.read(cx).text())
        {
            self.prompt_history = None;
        }
        self.reconcile_attachment_mentions(cx);
        self.sync_draft(cx);
        if self.wizard.is_some() {
            if self.mention.token.is_some() || self.mention_task.is_some() {
                self.reset_mention(None, cx);
            }
            if self.slash.token.is_some() || self.slash_task.is_some() {
                self.reset_slash(None, cx);
            }
            return;
        }
        let input = self.input.read(cx);
        if !input.selected_range.is_empty() || input.marked_range.is_some() {
            if self.mention.token.is_some() {
                self.reset_mention(None, cx);
            }
            if self.slash.token.is_some() {
                self.reset_slash(None, cx);
            }
            return;
        }
        let (text, cursor) = (input.text().to_string(), input.cursor_offset());
        self.update_slash(&text, cursor, cx);
        let token = mention_token(&text, cursor);
        let context = self
            .file_search_params("", cx)
            .map(|params| format!("{}:{params}", self.completion_connection_context(cx)))
            .unwrap_or_default();
        if self.mention.context != context {
            self.reset_mention(None, cx);
            self.mention.context = context;
        }
        let still_dismissed = token.as_ref().is_some_and(|token| {
            self.mention
                .dismissed
                .as_ref()
                .is_some_and(|(range, value)| {
                    token.range == *range && text.get(range.clone()) == Some(value.as_str())
                })
        });
        if still_dismissed {
            self.mention.token = None;
            self.mention_task = None;
            self.sync_mention_controls(cx);
            cx.notify();
            return;
        }
        self.mention.dismissed = None;
        if token == self.mention.token {
            self.sync_mention_controls(cx);
            cx.notify();
            return;
        }
        self.mention.request = self.mention.request.wrapping_add(1);
        self.mention_task = None;
        // Refining an open menu keeps the stale rows visible until the new
        // response lands — clearing here made the popup bounce through the
        // skeleton (and a different height) on every keystroke.
        let refining = self.mention.token.is_some() && token.is_some();
        self.mention.token = token.clone();
        if !refining {
            self.mention.results.clear();
            self.mention.active = (!self.mention_attachment_matches().is_empty()).then_some(0);
            // Fresh open: the row stack restarts at the top.
            crate::popover::reset_menu_scroll(&self.mention_scroll, &mut self.popup_bar);
        }
        self.mention.error = None;
        self.mention.loading = token.is_some();
        self.mention.threads = match &token {
            Some(token) => match self.mention_device(cx) {
                Some((device, exclude)) => {
                    let space = self.mention_space(cx);
                    let state = self.state.read(cx);
                    thread_matches(
                        state.visible_chats(),
                        &token.query,
                        MentionScope {
                            device_id: &device,
                            space_id: space.as_deref(),
                            exclude: exclude.as_deref(),
                        },
                        |id| state.is_draft(id),
                        |chat| match (state.space_for_chat(chat), chat.space_id.as_deref()) {
                            (Some(space), _) => space.display_name().to_string(),
                            (None, None) => "~".to_string(),
                            (None, Some(_)) => "?".to_string(),
                        },
                    )
                }
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        // Threads are known before the file search answers, so the keyboard
        // cursor lands on the first thread now instead of on the first file
        // later (and stays within range when a refinement shrinks the list).
        let count = self.mention_item_count();
        self.mention.active = match self.mention.active {
            Some(active) if active < count => Some(active),
            _ => (count > 0).then_some(0),
        };
        self.sync_mention_controls(cx);
        let Some(token) = token else {
            cx.notify();
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.mention.loading = false;
            cx.notify();
            return;
        };
        let Some(params) = self.file_search_params(&token.query, cx) else {
            self.mention.loading = false;
            cx.notify();
            return;
        };
        let request = self.mention.request;
        self.mention_task = Some(cx.spawn(async move |this, cx| {
            // A short debounce prevents one full workspace walk per keystroke
            // during normal typing. The generation check below still guards
            // requests that were already in flight when the query changed.
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            let mut result = engine
                .client()
                .call(methods::SEARCH_FILES, params.clone())
                .await;
            if matches!(result, Err(RpcError::Transport(_)) | Err(RpcError::Closed)) {
                // One retry rides out a cold relay dial to the host device
                // (the diffs pane retries forever; a keystroke-scoped search
                // gets a single second chance).
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                result = engine.client().call(methods::SEARCH_FILES, params).await;
            }
            this.update(cx, |composer, cx| {
                if !mention_response_is_current(&composer.mention, request) {
                    return;
                }
                composer.mention.loading = false;
                match result {
                    Ok(value) => match serde_json::from_value::<Vec<FileSearchMatch>>(value) {
                        Ok(mut results) => {
                            // Search results can include legal filesystem names
                            // that the canonical reference format cannot encode.
                            results.retain(|result| local_path_is_safe(&result.path));
                            composer.mention.error = None;
                            composer.mention.results = results;
                            let count = composer.mention_item_count();
                            composer.mention.active = match composer.mention.active {
                                Some(active) if active < count => Some(active),
                                _ => (count > 0).then_some(0),
                            };
                            // New result set: the row stack restarts at the top.
                            crate::popover::reset_menu_scroll(
                                &composer.mention_scroll,
                                &mut composer.popup_bar,
                            );
                        }
                        Err(err) => tracing::warn!(%err, "file mention response decode failed"),
                    },
                    Err(err) => {
                        tracing::warn!(%err, "file mention search failed");
                        composer.mention.results.clear();
                        let count = composer.mention_item_count();
                        composer.mention.active = composer
                            .mention
                            .active
                            .filter(|active| *active < count)
                            .or((count > 0).then_some(0));
                        composer.mention.error = Some(mention_error_message(&err));
                    }
                }
                composer.sync_mention_controls(cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn move_mention(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.mention.active =
            crate::popover::menu_step(self.mention.active, self.mention_item_count(), delta);
        if let Some(active) = self.mention.active {
            // Keep the keyboard cursor visible in the scrolled row stack; the
            // section headings occupy list slots of their own.
            let threads = self.mention.threads.len();
            let list_ix = if threads == 0 {
                active
            } else if active < threads {
                active + 1
            } else {
                active + 2
            };
            self.mention_scroll.scroll_to_item(list_ix);
        }
        self.sync_mention_controls(cx);
        cx.notify();
    }

    fn dismiss_mention(&mut self, cx: &mut Context<Self>) {
        let dismissed = self.mention.token.as_ref().and_then(|token| {
            self.input
                .read(cx)
                .text()
                .get(token.range.clone())
                .map(|text| (token.range.clone(), text.to_string()))
        });
        self.reset_mention(dismissed, cx);
        cx.notify();
    }

    /// Staged attachments that match the typed `@` query, pinned above file
    /// results. Matching ignores case and spaces, so `@im`, `@image2` and
    /// `@2` all find "Image 2", and `@note` finds "notes.md".
    fn mention_item_count(&self) -> usize {
        self.mention.threads.len()
            + self.mention_attachment_matches().len()
            + self.mention.results.len()
    }

    fn mention_attachment_matches(&self) -> Vec<AttachmentRow> {
        let Some(token) = self.mention.token.as_ref() else {
            return Vec::new();
        };
        let normalize = |text: &str| -> String {
            text.chars()
                .filter(|c| !c.is_whitespace())
                .flat_map(char::to_lowercase)
                .collect()
        };
        let wanted = normalize(&token.query);
        let mut rows: Vec<_> = self
            .staged()
            .iter()
            .filter_map(|att| {
                let index = att.mention?;
                let label = if att.image().is_some() {
                    zeron_proto::attachment_mentions::image_label(index)
                } else {
                    att.name.clone()
                };
                normalize(&label).contains(&wanted).then(|| AttachmentRow {
                    index,
                    label,
                    image: att.image().cloned(),
                })
            })
            .collect();
        rows.sort_by_key(|row| row.index);
        rows
    }

    fn accept_mention(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.mention.token.clone() else {
            return;
        };
        let Some(active) = self.mention.active else {
            return;
        };
        let threads = self.mention.threads.len();
        let attachments = self.mention_attachment_matches();
        let link = if let Some(thread) = self.mention.threads.get(active) {
            thread_link(&thread.chat_id, &thread.title)
        } else if let Some(row) = attachments.get(active - threads) {
            zeron_proto::attachment_mentions::attachment_mention_link(
                row.index,
                row.image.is_none().then_some(row.label.as_str()),
            )
        } else if let Some(result) = self
            .mention
            .results
            .get(active - threads - attachments.len())
        {
            local_file_link(&result.path, result.is_dir)
        } else {
            return;
        };
        self.input.update(cx, |input, cx| {
            input.replace_mention_link(token.range, &link, cx)
        });
        self.reset_mention(None, cx);
        cx.notify();
    }

    fn render_file_mention_popup(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let theme = &theme.for_popup();
        let token = self.mention.token.as_ref()?;
        let mut card = crate::popover::completion_card(theme)
            // Completion choices belong to the input. Keep it focused until
            // mouse-up can accept a choice (or while dragging the scrollbar).
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.input.focus_handle(cx), cx);
                }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_mention(cx)));
        let thread_count = self.mention.threads.len();
        let attachment_rows = self.mention_attachment_matches();
        let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(
            thread_count + attachment_rows.len() + self.mention.results.len() + 2,
        );
        if thread_count > 0 {
            rows.push(crate::popover::menu_heading(theme, "Threads").into_any_element());
        }
        let statuses: Vec<(zeron_proto::ChatIndicator, Option<usize>)> = {
            let now = chrono::Utc::now();
            let state = self.state.read(cx);
            self.mention
                .threads
                .iter()
                .map(|thread| {
                    let status = state
                        .chats
                        .iter()
                        .find(|chat| chat.id == thread.chat_id)
                        .map_or(zeron_proto::ChatIndicator::Idle, |chat| {
                            state.display_status_for(chat, now)
                        });
                    let ring = background::sidebar_ring(status, state.session_for(&thread.chat_id));
                    (status, ring)
                })
                .collect()
        };
        for (ix, thread) in self.mention.threads.iter().enumerate() {
            let selected = self.mention.active == Some(ix);
            let (status, ring) = statuses[ix];
            let status_color = crate::shell::status_dot_color(status, theme);
            let status_label = match status {
                zeron_proto::ChatIndicator::Working => Some("Working"),
                zeron_proto::ChatIndicator::AwaitingInput => Some("Input"),
                zeron_proto::ChatIndicator::Errored => Some("Failed"),
                zeron_proto::ChatIndicator::Completed => Some("Done"),
                zeron_proto::ChatIndicator::Idle => None,
            };
            let status_badge = match (status_label, ring) {
                (None, Some(count)) => Some(background::sidebar_ring_mark(count, theme)),
                (None, None) => None,
                (Some(label), _) => Some({
                    let glyph: gpui::AnyElement = match (ring, status) {
                        (Some(count), _) => {
                            background::sidebar_ring_mark(count, theme).into_any_element()
                        }
                        (None, zeron_proto::ChatIndicator::Working) => {
                            crate::loaders::mini_glyph_spinner(
                                format!("thread-mention-working-{}", thread.chat_id),
                                2.0,
                                theme.glyph,
                                cx.entity_id(),
                                cx,
                            )
                            .into_any_element()
                        }
                        (None, zeron_proto::ChatIndicator::Completed) => {
                            crate::icons::icon(crate::icons::CHECK)
                                .size(px(11.0))
                                .flex_none()
                                .text_color(status_color)
                                .into_any_element()
                        }
                        _ => div()
                            .size(px(6.0))
                            .flex_none()
                            .rounded_full()
                            .bg(status_color)
                            .into_any_element(),
                    };
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .child(glyph)
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(10.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(status_color)
                                .child(SharedString::from(label)),
                        )
                }),
            };
            rows.push(
                crate::popover::menu_row(theme, selected, format!("thread-mention-result-{ix}"))
                    .id(("thread-mention-result", ix))
                    .debug_selector(|| format!("thread-mention-result-{ix}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.mention.active = Some(ix);
                        this.accept_mention(cx);
                    }))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(crate::pickers::harness_mark_stack(
                                &thread.marks,
                                14.0,
                                theme.text_muted,
                                1.0,
                            ))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .overflow_hidden()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .text_color(theme.text)
                                    .child(SharedString::from(thread.title.clone())),
                            )
                            .children(status_badge)
                            .child(
                                div()
                                    .flex_none()
                                    .max_w(px(160.0))
                                    .overflow_hidden()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(thread.project.clone())),
                            ),
                    )
                    .into_any_element(),
            );
        }
        // The file half keeps its own states under the thread rows: a
        // skeleton while the search is out, the error, or the empty note
        // (only when no thread matched either).
        let mut status: Option<gpui::AnyElement> = None;
        if self.mention.loading && self.mention.results.is_empty() && attachment_rows.is_empty() {
            status = Some(crate::popover::skeleton_rows(
                "file-mention-loading",
                theme,
                if thread_count > 0 { 1 } else { 3 },
                cx.entity_id(),
                cx,
            ));
        } else if let Some(error) = self
            .mention
            .error
            .clone()
            .filter(|_| attachment_rows.is_empty())
        {
            status = Some(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.danger_muted)
                    .child(error)
                    .into_any_element(),
            );
        } else if self.mention.results.is_empty() && attachment_rows.is_empty() {
            if thread_count == 0 {
                status = Some(
                    div()
                        .px(px(12.0))
                        .py(px(10.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(if token.query.is_empty() {
                            "No files available"
                        } else {
                            "No matching files or threads"
                        })
                        .into_any_element(),
                );
            }
        } else {
            if thread_count > 0 {
                rows.push(crate::popover::menu_heading(theme, "Files").into_any_element());
            }
            // Staged attachments stay pinned above every file result.
            for (attachment_ix, row) in attachment_rows.iter().enumerate() {
                let ix = thread_count + attachment_ix;
                let selected = self.mention.active == Some(ix);
                let (icon, detail) = match &row.image {
                    Some(image) => (
                        img(image.clone())
                            .size(px(16.0))
                            .rounded(px(3.0))
                            .object_fit(ObjectFit::Cover)
                            .into_any_element(),
                        "Attached image",
                    ),
                    None => (
                        crate::file_icons::icon(
                            crate::file_icons::FileIconIdentity::file(&row.label),
                            theme.appearance,
                        )
                        .size(px(16.0))
                        .into_any_element(),
                        "Attached file",
                    ),
                };
                rows.push(
                    crate::popover::menu_row(
                        theme,
                        selected,
                        format!("attachment-mention-result-{ix}"),
                    )
                    .id(("attachment-mention-result", ix))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.mention.active = Some(ix);
                        this.accept_mention(cx);
                    }))
                    .child(crate::popover::completion_row_content(
                        theme,
                        icon,
                        row.label.clone().into(),
                        detail.into(),
                    ))
                    .into_any_element(),
                );
            }
            let attachment_count = attachment_rows.len();
            for (file_ix, result) in self.mention.results.iter().enumerate() {
                let ix = thread_count + attachment_count + file_ix;
                let selected = self.mention.active == Some(ix);
                let (directory, name) = match result.path.rsplit_once('/') {
                    Some((directory, name)) => (directory.to_string(), name.to_string()),
                    None => (String::new(), result.path.clone()),
                };
                rows.push(
                    crate::popover::menu_row(theme, selected, format!("file-mention-result-{ix}"))
                        .id(("file-mention-result", ix))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mention.active = Some(ix);
                            this.accept_mention(cx);
                        }))
                        .child(crate::popover::completion_row_content(
                            theme,
                            crate::file_icons::icon(
                                if result.is_dir {
                                    crate::file_icons::FileIconIdentity::directory(
                                        &result.path,
                                        false,
                                    )
                                } else {
                                    crate::file_icons::FileIconIdentity::file(&result.path)
                                },
                                theme.appearance,
                            )
                            .size(px(16.0))
                            .into_any_element(),
                            name.into(),
                            directory.into(),
                        ))
                        .into_any_element(),
                );
            }
        }
        if !rows.is_empty() {
            // Overflowing rows wheel-scroll inside a bounded viewport; the
            // floating rail mirrors the model-list scrollbar treatment.
            card = card.child(
                crate::popover::menu_scroll_host("mention-scroll-host")
                    .on_hover(cx.listener(Self::on_popup_list_hover))
                    .child(crate::popover::completion_list(
                        "mention-list",
                        &self.mention_scroll,
                        rows,
                    ))
                    .children(crate::popover::rail(self, "mention-scrollbar", theme, cx)),
            );
        }
        card = card.children(status);
        Some(crate::popover::full_width_menu_above(
            "file-mention-popup",
            card.into_any_element(),
            None,
        ))
    }

    fn render_input_with_completion(&self) -> gpui::Div {
        div().relative().child(self.input.clone())
    }

    // ---- slash commands ---------------------------------------------------

    /// Track the `/` token on every edit: open/refresh the popup, fetch the
    /// harness's command list on each open, filter locally per keystroke.
    fn update_slash(&mut self, text: &str, cursor: usize, cx: &mut Context<Self>) {
        let harness = self.pickers.read(cx).resolved(cx).harness;
        let preferences =
            crate::settings::current(cx).skill_completion(harness.unwrap_or(HarnessId::Codex));
        let (token, skill, include_skills, commands_allowed) =
            completion_trigger(text, cursor, preferences);
        if token.is_none() {
            // Leaving a token must not replace the catalog identity with the
            // idle (commands_allowed=false) context and evict the warm cache.
            self.reset_slash(None, cx);
            return;
        }
        let params = self.catalog_params(cx);
        let catalog_context = format!(
            "{preferences:?}:{}:{params}",
            self.completion_connection_context(cx),
        );
        let context = slash_context(skill, include_skills, commands_allowed, &catalog_context);
        let context_changed = self.slash.context != context;
        if !context_changed
            && token.as_ref().is_some_and(|token| {
                self.slash.dismissed.as_ref().is_some_and(|(range, value)| {
                    token.range == *range && text.get(range.clone()) == Some(value.as_str())
                })
            })
        {
            return;
        }
        if !context_changed && token == self.slash.token {
            return;
        }
        let refresh_catalog = context_changed || self.slash.token.is_none();
        self.slash.dismissed = None;
        if context_changed {
            self.slash.request = self.slash.request.wrapping_add(1);
            self.slash_task = None;
            self.slash.loading = false;
            if self.slash.catalog_context != catalog_context {
                self.slash_cache.clear();
            } else if self.slash.error.is_some() || !self.slash.supported {
                self.slash_cache.remove(&self.slash.context);
            }
        }
        self.slash.catalog_context = catalog_context;
        self.slash.context = context.clone();
        self.slash.harness = harness;
        self.slash.skill = skill;
        if context_changed {
            self.slash.supported = true;
        }
        self.slash.token = token;
        if context_changed {
            self.slash.error = None;
        }
        if harness.is_none() && !skill && commands_allowed {
            self.slash_cache.insert(
                context.clone(),
                with_workspace_commands(vec![], self.state.read(cx).selected_chat.is_some()),
            );
        }
        if harness.is_none()
            || (self.slash_cache.contains_key(&context) && !refresh_catalog)
            || self.slash.loading
        {
            self.refilter_slash(cx);
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            if !skill && commands_allowed {
                self.slash_cache.insert(
                    context,
                    with_workspace_commands(vec![], self.state.read(cx).selected_chat.is_some()),
                );
                self.slash.error = Some("Agent command discovery requires a connection".into());
                self.refilter_slash(cx);
            }
            return;
        };
        self.slash.request = self.slash.request.wrapping_add(1);
        let request = self.slash.request;
        self.slash.loading = true;
        self.slash.error = None;
        self.refilter_slash(cx);
        self.slash_task = Some(cx.spawn(async move |this, cx| {
            let result = fetch_invocation_catalog(
                engine,
                params,
                harness,
                skill,
                include_skills,
                commands_allowed,
            )
            .await;
            this.update(cx, |composer, cx| {
                if composer.slash.request != request || composer.slash.context != context {
                    return;
                }
                composer.slash.loading = false;
                let decoded = result.map(|(candidates, supported, warning)| {
                    composer.slash.supported = supported;
                    composer.slash.error = warning;
                    candidates
                });
                match decoded {
                    Ok(candidates) => {
                        let candidates = if !skill && commands_allowed {
                            with_workspace_commands(
                                candidates,
                                composer.state.read(cx).selected_chat.is_some(),
                            )
                        } else {
                            candidates
                        };
                        composer.slash_cache.insert(context, candidates);
                    }
                    Err(err) => {
                        composer.slash_cache.remove(&context);
                        composer.slash.error = Some(slash_error_message(&err, skill));
                        if !skill && commands_allowed {
                            composer.slash_cache.insert(
                                context,
                                with_workspace_commands(
                                    vec![],
                                    composer.state.read(cx).selected_chat.is_some(),
                                ),
                            );
                        }
                    }
                }
                composer.refilter_slash(cx);
            })
            .ok();
        }));
        cx.notify();
    }

    fn prefetch_slash_catalog(&mut self, cx: &mut Context<Self>) {
        if self.slash.token.is_some() {
            return;
        }
        let Some(harness) = self.pickers.read(cx).resolved(cx).harness else {
            return;
        };
        let preferences = crate::settings::current(cx).skill_completion(harness);
        let params = self.catalog_params(cx);
        let catalog_context = format!(
            "{preferences:?}:{}:{params}",
            self.completion_connection_context(cx),
        );
        if self.slash.catalog_context == catalog_context
            || self
                .slash_prefetch
                .as_ref()
                .is_some_and(|(pending, _)| *pending == catalog_context)
        {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let triggers: Vec<_> = ["/", "$"]
            .into_iter()
            .filter_map(|trigger| {
                let (token, skill, include_skills, commands_allowed) =
                    completion_trigger(trigger, 1, preferences);
                token.map(|_| (skill, include_skills, commands_allowed))
            })
            .collect();
        let context = catalog_context.clone();
        let task = cx.spawn(async move |this, cx| {
            // Clicking through chats replaces this task before any agent is spawned.
            cx.background_executor().timer(SLASH_PREFETCH_DELAY).await;
            let results = futures::future::join_all(triggers.iter().map(
                |&(skill, include_skills, commands_allowed)| {
                    fetch_invocation_catalog(
                        engine.clone(),
                        params.clone(),
                        Some(harness),
                        skill,
                        include_skills,
                        commands_allowed,
                    )
                },
            ))
            .await;
            this.update(cx, |composer, cx| {
                if composer
                    .slash_prefetch
                    .take_if(|(pending, _)| *pending == context)
                    .is_none()
                    || composer.slash.token.is_some()
                {
                    return;
                }
                if composer.slash.catalog_context != context {
                    composer.slash_cache.clear();
                    composer.slash.catalog_context = context.clone();
                }
                let chat_selected = composer.state.read(cx).selected_chat.is_some();
                for ((skill, include_skills, commands_allowed), result) in
                    triggers.into_iter().zip(results)
                {
                    let Ok((candidates, _, _)) = result else {
                        continue;
                    };
                    let candidates = if !skill && commands_allowed {
                        with_workspace_commands(candidates, chat_selected)
                    } else {
                        candidates
                    };
                    composer.slash_cache.insert(
                        slash_context(skill, include_skills, commands_allowed, &context),
                        candidates,
                    );
                }
            })
            .ok();
        });
        self.slash_prefetch = Some((catalog_context, task));
    }

    /// Re-rank the cached list for the current query (pure local filter).
    fn refilter_slash(&mut self, cx: &mut Context<Self>) {
        let query = self
            .slash
            .token
            .as_ref()
            .map(|t| t.query.clone())
            .unwrap_or_default();
        let commands = self
            .slash_cache
            .get(&self.slash.context)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
        self.slash.filtered = crate::popover::filter_indices(&query, &names);
        self.slash.active = (!self.slash.filtered.is_empty()).then_some(0);
        // A fresh query/reopen restarts the row stack at the top.
        crate::popover::reset_menu_scroll(&self.slash_scroll, &mut self.popup_bar);
        self.sync_mention_controls(cx);
        cx.notify();
    }

    fn move_slash(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.slash.active =
            crate::popover::menu_step(self.slash.active, self.slash.filtered.len(), delta);
        if let Some(active) = self.slash.active {
            // Keep the keyboard cursor visible in the scrolled row stack.
            self.slash_scroll.scroll_to_item(active);
        }
        self.sync_mention_controls(cx);
        cx.notify();
    }

    fn dismiss_slash(&mut self, cx: &mut Context<Self>) {
        let dismissed = self.slash.token.as_ref().and_then(|token| {
            self.input
                .read(cx)
                .text()
                .get(token.range.clone())
                .map(|text| (token.range.clone(), text.to_string()))
        });
        self.reset_slash(dismissed, cx);
        cx.notify();
    }

    fn accept_slash(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.slash.token.clone() else {
            return;
        };
        let Some(command) = self
            .slash
            .active
            .and_then(|active| self.slash.filtered.get(active))
            .and_then(|&ix| {
                self.slash_cache
                    .get(&self.slash.context)
                    .and_then(|c| c.get(ix))
            })
            .cloned()
        else {
            return;
        };
        // A command the harness cannot run is shown, not offered.
        if command.name == COMPACT_COMMAND && self.unsupported_compaction(cx).is_some() {
            return;
        }
        if let Some(action) = command.workspace_command {
            self.execute_workspace_command(action, token.range, cx);
            return;
        }
        let insertion = if zeron_owned_command(&command.name) {
            format!("/{}", command.name)
        } else {
            invocation_insertion(&command.invocation, self.reference_delivery_supported(cx))
        };
        self.input.update(cx, |input, cx| {
            input.replace_plain_token(token.range, &insertion, cx)
        });
        self.reset_slash(None, cx);
        cx.notify();
    }

    /// Tear down the slash completion (mirrors [`Self::reset_mention`]).
    fn reset_slash(&mut self, dismissed: Option<(Range<usize>, String)>, cx: &mut Context<Self>) {
        // Partial discovery is useful now, but reopening must retry the failed provider call.
        if self.slash.error.is_some() {
            self.slash_cache.remove(&self.slash.context);
        }
        let request = self.slash.request.wrapping_add(1);
        self.slash_task = None;
        self.slash = SlashState {
            request,
            dismissed,
            harness: self.slash.harness,
            catalog_context: self.slash.catalog_context.clone(),
            context: self.slash.context.clone(),
            supported: self.slash.supported,
            ..SlashState::default()
        };
        self.sync_mention_controls(cx);
    }

    // ---- context compaction ------------------------------------------------

    /// One `HarnessCompaction` per harness, cached like the command list —
    /// the meter's button needs the answer before any popup has opened.
    fn ensure_compaction_kind(&mut self, cx: &mut Context<Self>) {
        let Some(harness) = self.pickers.read(cx).resolved(cx).harness else {
            return;
        };
        if !self.compaction_probed.insert(harness) {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.compaction_probed.remove(&harness);
            return;
        };
        let target = self
            .bound_chat_row(cx)
            .map(|chat| chat.device_id.clone())
            .or_else(|| {
                self.state
                    .read(cx)
                    .selected_space_row()
                    .map(|s| s.device_id.clone())
            });
        cx.spawn(async move |this, cx| {
            let params = zeron_proto::HarnessCompactionParams {
                harness,
                target_device_id: target,
            };
            let result = engine
                .client()
                .call(
                    methods::HARNESS_COMPACTION,
                    serde_json::to_value(params).unwrap(),
                )
                .await
                .and_then(|value| {
                    serde_json::from_value::<zeron_proto::HarnessCompactionResult>(value)
                        .map_err(|err| RpcError::Failed(err.to_string()))
                });
            this.update(cx, |composer, cx| {
                match result {
                    Ok(capabilities) => {
                        composer
                            .compaction_cache
                            .insert(harness, capabilities.compaction);
                        composer.state.update(cx, |state, _| {
                            state
                                .harness_branching
                                .insert(harness, capabilities.branching);
                        });
                    }
                    Err(err) => {
                        tracing::debug!(%err, "harness compaction probe failed");
                        // Unknown stays unknown; the next state change retries.
                        composer.compaction_probed.remove(&harness);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Why this harness can never compact, if it cannot.
    fn unsupported_compaction(&self, cx: &App) -> Option<SharedString> {
        let harness = self.pickers.read(cx).resolved(cx).harness?;
        (self.compaction_cache.get(&harness).copied() == Some(zeron_proto::CompactionKind::None))
            .then(|| {
                format!(
                    "{} does not support compaction",
                    harness_display_name(harness)
                )
                .into()
            })
    }

    /// This composer's chat is compacting. Another chat's compaction must not
    /// lock this one's composer.
    pub(crate) fn compacting(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        state.compacting_chat.is_some() && state.compacting_chat == self.bound_chat_id(cx)
    }

    /// Why compaction is refused right now, if it is — shared by the typed
    /// command and the context meter's button.
    pub(crate) fn compact_disabled_reason(&self, cx: &App) -> Option<SharedString> {
        if self.compacting(cx) {
            return Some("Compaction in progress".into());
        }
        if self.run_live(cx) {
            return Some("Wait for the current turn to finish".into());
        }
        self.unsupported_compaction(cx)
    }

    pub(crate) fn resume_compaction(&self, cx: &App) -> Option<ResumeCompaction> {
        if self.compact_disabled_reason(cx).is_some() {
            return None;
        }
        let chat = self.bound_chat_row(cx)?;
        if self
            .resume_compaction_kept
            .as_ref()
            .is_some_and(|(id, at)| *id == chat.id && *at == chat.last_message_at)
        {
            return None;
        }
        let harness = self.pickers.read(cx).resolved(cx).harness?;
        let tokens = self
            .state
            .read(cx)
            .context_usage_for(Some(&chat.id))
            .and_then(|usage| usage.tokens);
        resume_compaction_due(harness, tokens, chat.last_message_at, chrono::Utc::now())
    }

    fn keep_full_history(&mut self, cx: &mut Context<Self>) {
        self.resume_compaction_kept = self
            .bound_chat_row(cx)
            .map(|chat| (chat.id.clone(), chat.last_message_at));
        cx.notify();
    }

    fn set_chat_failure(&mut self, message: SharedString, cx: &mut Context<Self>) {
        self.failure = Some(message);
        self.failure_key = Some(self.current_key.clone());
        cx.notify();
    }

    /// `CompactChat` on the selected chat: the harness compacts its own
    /// context and the host appends the boundary entry. No `Run` is built and
    /// no user entry is written, so nothing shows in the transcript until the
    /// call returns.
    pub(crate) fn request_compaction(&mut self, focus: Option<String>, cx: &mut Context<Self>) {
        if let Some(reason) = self.compact_disabled_reason(cx) {
            self.set_chat_failure(reason, cx);
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.set_chat_failure("Engine not connected".into(), cx);
            return;
        };
        let Some((chat_id, host_device_id)) = self
            .bound_chat_row(cx)
            .map(|chat| (chat.id.clone(), chat.device_id.clone()))
        else {
            self.set_chat_failure("Start a chat before compacting its context.".into(), cx);
            return;
        };
        let mut params = serde_json::json!({
            "chatId": chat_id,
            "targetDeviceId": host_device_id,
        });
        if let (Some(focus), Some(object)) = (focus, params.as_object_mut()) {
            object.insert("focus".into(), focus.into());
        }
        self.failure = None;
        let pending = chat_id.clone();
        self.state.update(cx, |state, cx| {
            state.compacting_chat = Some(pending);
            cx.notify();
        });
        cx.notify();
        let key = self.current_key.clone();
        let pending = chat_id.clone();
        self.compact_task = Some(cx.spawn(async move |this, cx| {
            // Outlives the engine's own 120s compaction guard, so the reply
            // carries the harness's outcome rather than a local timeout.
            let result = attachments::call_with_timeout(
                &engine,
                cx.background_executor(),
                methods::COMPACT_CHAT,
                params,
                Duration::from_secs(180),
            )
            .await;
            this.update(cx, |composer, cx| {
                composer.state.update(cx, |state, cx| {
                    // Only ours: a retargeted compaction owns the marker now.
                    if state.compacting_chat.as_deref() == Some(pending.as_str()) {
                        state.compacting_chat = None;
                        cx.notify();
                    }
                });
                // Compacted: the boundary entry arrives through the doc watch.
                let message = match result {
                    Ok(value) => (value.get("compacted") != Some(&serde_json::Value::Bool(true)))
                        .then(|| {
                            value
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("Nothing to compact")
                                .to_string()
                        }),
                    Err(err) => Some(format!("Compaction failed: {err}")),
                };
                if let Some(message) = message {
                    composer.failure = Some(message.into());
                    composer.failure_key = Some(key.clone());
                }
                cx.notify();
            })
            .ok();
        }));
    }

    // ---- sidechats ---------------------------------------------------------

    /// Ask the shell to open this chat's side conversation — from `/sidechat`
    /// or from the footer pill. The shell owns the right pane, the creation
    /// RPC and the first turn; the composer only names the source.
    pub(crate) fn request_sidechat(
        &mut self,
        prompt: Option<String>,
        carry_staged: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((source_chat_id, nested)) = self
            .bound_chat_row(cx)
            .map(|chat| (chat.id.clone(), chat.sidechat_source_chat_id.is_some()))
        else {
            self.set_chat_failure("Start a chat before opening a sidechat.".into(), cx);
            return;
        };
        if nested {
            self.set_chat_failure("Nested sidechats are not supported".into(), cx);
            return;
        }
        self.failure = None;
        let staged = if carry_staged {
            SidechatStaged {
                attachments: self
                    .attachments
                    .remove(&self.current_key)
                    .unwrap_or_default(),
                appshots: self.appshots.remove(&self.current_key).unwrap_or_default(),
            }
        } else {
            SidechatStaged::default()
        };
        cx.emit(ComposerEvent::OpenSidechat {
            source_chat_id,
            prompt,
            staged,
        });
        cx.notify();
    }

    pub(crate) fn stage_carried(&mut self, staged: SidechatStaged, cx: &mut Context<Self>) {
        self.add_staged(staged.attachments, cx);
        for appshot in staged.appshots {
            self.stage_appshot(appshot, cx);
        }
    }

    /// The reopen pill: this chat's newest sidechat, while its tab is closed.
    fn render_sidechat_pill(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if self.chat_override.is_some() {
            return None;
        }
        let chat_id = self.bound_chat_id(cx)?;
        let state = self.state.read(cx);
        let sidechat = state.latest_sidechat_of(&chat_id)?;
        if state.open_sidechat_tabs.contains(&sidechat.id) {
            return None;
        }
        let label: SharedString = sidechat_pill_label(sidechat.title.as_deref()).into();
        Some(
            div()
                .id("composer-sidechat-pill")
                .flex_none()
                .flex()
                .items_center()
                .gap(px(5.0))
                .h(px(24.0))
                .px(px(6.0))
                .rounded(px(6.0))
                .text_size(px(11.0))
                .text_color(theme.text_muted)
                .hover(|s| s.bg(crate::theme::ink(0.05)))
                .cursor_pointer()
                .child(
                    crate::icons::icon(crate::icons::CHAT_ROUND_LINE)
                        .size(px(12.0))
                        .text_color(theme.text_muted),
                )
                .child(label)
                .on_click(cx.listener(|this, _, _, cx| this.request_sidechat(None, false, cx)))
                .into_any_element(),
        )
    }

    fn render_slash_popup(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let theme = &theme.for_popup();
        // Only while a slash token is active.
        self.slash.token.as_ref()?;
        let commands = self
            .slash_cache
            .get(&self.slash.context)
            .map(Vec::as_slice)
            .unwrap_or_default();
        // Full pill width at the mention card's height budget — both composer
        // completions share the same surface shape.
        let mut card = crate::popover::completion_card(theme)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.input.focus_handle(cx), cx);
                }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_slash(cx)));
        if let Some(error) = self.slash.error.clone() {
            card = card.child(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.danger_muted)
                    .child(error),
            );
        }
        if self.slash.loading && commands.is_empty() {
            card = card.child(crate::popover::skeleton_rows(
                "slash-loading",
                theme,
                3,
                cx.entity_id(),
                cx,
            ));
        } else if self.slash.filtered.is_empty() && self.slash.error.is_none() {
            card = card.child(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(if commands.is_empty() {
                        if self.slash.skill {
                            if self.slash.supported {
                                "No skills available for this project"
                            } else {
                                "This agent does not advertise skills"
                            }
                        } else if !crate::settings::current(cx)
                            .skill_completion(self.slash.harness.unwrap_or(HarnessId::Codex))
                            .separate_from_slash
                        {
                            "No commands or skills available"
                        } else {
                            "No slash commands available in this integration"
                        }
                    } else {
                        if self.slash.skill {
                            "No matching skills"
                        } else if !crate::settings::current(cx)
                            .skill_completion(self.slash.harness.unwrap_or(HarnessId::Codex))
                            .separate_from_slash
                        {
                            "No matching commands or skills"
                        } else {
                            "No matching commands"
                        }
                    }),
            );
        } else {
            let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(self.slash.filtered.len());
            let unsupported = self.unsupported_compaction(cx);
            for (row_ix, &cmd_ix) in self.slash.filtered.iter().enumerate() {
                let Some(command) = commands.get(cmd_ix) else {
                    continue;
                };
                let selected = self.slash.active == Some(row_ix);
                let name: SharedString = if command.invocation.prefix() == '$' {
                    skill_display_name(&command.name)
                } else {
                    format!("/{}", command.name)
                }
                .into();
                // The reason replaces the description rather than hiding the
                // row: the entry exists on every harness, so its absence
                // would read as a bug.
                let blocked = (command.name == COMPACT_COMMAND)
                    .then(|| unsupported.clone())
                    .flatten();
                let description: SharedString = match blocked {
                    Some(reason) => reason,
                    None => {
                        let mut description = command.description.clone();
                        if let Some(hint) = &command.input_hint {
                            if description.is_empty() {
                                description = format!("<{hint}>");
                            } else {
                                description = format!("{description} · <{hint}>");
                            }
                        }
                        description.into()
                    }
                };
                let disabled = command.name == COMPACT_COMMAND && unsupported.is_some();
                rows.push(
                    crate::popover::menu_row(theme, selected, format!("slash-result-{row_ix}"))
                        .id(("slash-result", row_ix))
                        .when(disabled, |row| row.opacity(0.45))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.slash.active = Some(row_ix);
                            this.accept_slash(cx);
                        }))
                        .child(crate::popover::completion_row_content(
                            theme,
                            crate::icons::icon(if command.invocation.prefix() == '$' {
                                crate::icons::MAGIC_STICK_3
                            } else {
                                crate::icons::COMMAND
                            })
                            .size(px(16.0))
                            .text_color(theme.text_muted)
                            .into_any_element(),
                            name,
                            description,
                        ))
                        .into_any_element(),
                );
            }
            // Overflowing rows wheel-scroll inside a bounded viewport; the
            // floating rail mirrors the model-list scrollbar treatment.
            card = card.child(
                crate::popover::menu_scroll_host("slash-scroll-host")
                    .on_hover(cx.listener(Self::on_popup_list_hover))
                    .child(crate::popover::completion_list(
                        "slash-list",
                        &self.slash_scroll,
                        rows,
                    ))
                    .children(crate::popover::rail(self, "slash-scrollbar", theme, cx)),
            );
        }
        // Full pill width above the composer, matching the file-mention popup.
        Some(crate::popover::full_width_menu_above(
            "slash-popup",
            card.into_any_element(),
            None,
        ))
    }

    /// The popup whose rows a scrollbar drag is moving — the tokens are
    /// mutually exclusive, so at most one exists.
    fn active_popup_scroll(&self) -> Option<gpui::ScrollHandle> {
        if self.slash.token.is_some() {
            Some(self.slash_scroll.clone())
        } else if self.mention.token.is_some() {
            Some(self.mention_scroll.clone())
        } else {
            None
        }
    }

    fn on_popup_list_hover(
        &mut self,
        hovered: &bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.popup_bar.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    fn on_state_changed(&mut self, cx: &mut Context<Self>) {
        self.sync_asks(cx);
        self.ensure_compaction_kind(cx);
        self.prefetch_slash_catalog(cx);
        self.prune_drafts(cx);
        {
            let state = self.state.read(cx);
            let now = chrono::Utc::now();
            retain_live_interrupts(&mut self.interrupting, |chat_id| {
                matches!(
                    state.indicator_for(chat_id, now),
                    Indicator::Working | Indicator::AwaitingInput
                )
            });
            self.queue_removing
                .retain(|id| state.queue.iter().any(|item| item.id == *id));
        }
        self.interrupt_tasks
            .retain(|chat_id, _| self.interrupting.contains(chat_id));

        let editing_id = self.editing_queued.clone();
        let key = self
            .chat_override
            .clone()
            .or_else(|| self.state.read(cx).focused_id())
            .unwrap_or_default();
        let (pending, edited_row_exists) = {
            let s = self.state.read(cx);
            (
                // `state.transcript` and `state.queue` belong to the SELECTED
                // chat; a bound composer would otherwise answer the source
                // chat's question from the sidechat's input.
                self.chat_override
                    .is_none()
                    .then(|| pending_input_request(&s.transcript))
                    .flatten(),
                editing_id
                    .as_ref()
                    .is_none_or(|id| s.queue.iter().any(|item| item.id == *id)),
            )
        };

        // A queue edit belongs to exactly one visible row. Navigation or a
        // remote drain/removal cancels it instead of leaving a focused but
        // unmounted editor entity behind.
        if key != self.current_key && self.editing_queued.is_some() {
            self.clear_queue_edit(cx);
        } else if !edited_row_exists && self.editing_queued.is_some() && !self.queue_edit_finishing
        {
            self.failure =
                Some("The queued message was removed; your edit remains in the composer".into());
            // Recover both drafts when another device removes the reserved row.
            if let Some((draft, mut attachments, mut appshots)) = self.queue_edit_draft.take() {
                appshots.extend(self.appshots.remove(&self.current_key).unwrap_or_default());
                self.appshots.insert(self.current_key.clone(), appshots);
                // The edit's chip numbers would collide with the draft's: its
                // chips become their labels and its attachments plain tiles.
                let edited = zeron_proto::attachment_mentions::attachment_mention_prompt(
                    self.input.read(cx).text(),
                );
                let text = [draft, edited]
                    .into_iter()
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                self.input.update(cx, |input, cx| input.set_text(text, cx));
                attachments.extend(
                    self.attachments
                        .remove(&self.current_key)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|att| StagedAttachment {
                            mention: None,
                            ..att
                        }),
                );
                self.attachments
                    .insert(self.current_key.clone(), attachments);
            }
            self.clear_queue_edit(cx);
        }

        // Draft swap on chat navigation — the input entity itself survives.
        // A promoted draft keeps its key (the chat was minted with the
        // draft's id), so the canvas→session morph also fires on the key's
        // CHANGE OF KIND with the text left alone (send already cleared it).
        let now_canvas = key.is_empty() || self.state.read(cx).is_draft(&key);
        let promoted = key == self.current_key && self.current_is_canvas && !now_canvas;
        if key != self.current_key || promoted {
            if key != self.current_key && !now_canvas && !self.current_is_canvas {
                // Switching between established chats still snaps to the new
                // draft; a tail from a hero transition belongs to its old chat.
                self.dock_reflow = Default::default();
            }
            let new_thread_launch =
                self.launching_new_chat && self.current_is_canvas && !now_canvas;
            let returning_to_new_thread = !self.current_is_canvas && now_canvas;
            self.launching_new_chat = false;
            self.current_is_canvas = now_canvas;
            let draft = if promoted {
                self.input.read(cx).text().to_string()
            } else {
                let old_text = self.input.read(cx).text().to_string();
                // Text typed on the pre-sync canvas (key "") belongs to the
                // draft that key becomes once the boot frames land (d20).
                let target_draft = crate::draft_store::entry(&key, cx);
                let carry = self.current_key.is_empty()
                    && now_canvas
                    && !key.is_empty()
                    && target_draft.prompt.is_empty();
                if carry {
                    self.attachments
                        .insert(key.clone(), target_draft.attachments);
                    self.appshots.insert(key.clone(), target_draft.appshots);
                    if let Some(staged) = self.attachments.remove("") {
                        self.attachments
                            .entry(key.clone())
                            .or_default()
                            .extend(staged);
                    }
                    if let Some(shots) = self.appshots.remove("") {
                        self.appshots.entry(key.clone()).or_default().extend(shots);
                    }
                    crate::draft_store::remove("", cx);
                    old_text
                } else {
                    let state = self.state.read(cx);
                    let keep = !state.chats_synced
                        || self.current_key.is_empty()
                        || state.is_draft(&self.current_key)
                        || state.chats.iter().any(|chat| chat.id == self.current_key)
                        || state.send_pending(&self.current_key, chrono::Utc::now());
                    if keep {
                        self.sync_draft(cx);
                    }
                    self.attachments
                        .insert(key.clone(), target_draft.attachments);
                    self.appshots.insert(key.clone(), target_draft.appshots);
                    target_draft.prompt
                }
            };
            self.current_key = key;
            self.prompt_history = None;
            // `failure` deliberately survives navigation: chat-scoped
            // failures render only under their own chat (see `failure_key`),
            // so switching away and back must not erase the one visible
            // trace of a failed send.
            self.wizard = None;
            // Attachments stay stashed under their chat key (the map swap IS
            // the navigation); only the transient chrome resets.
            self.preview = None;
            self.reset_mention(None, cx);
            self.reset_slash(None, cx);
            // Route changes snap (round 5/6): a mode difference between the
            // old and new session's composer must not glide across
            // navigation. Killing the in-flight morph here isn't enough —
            // the nav-driven flip only commits AFTER the swapped draft has
            // been re-measured, one or two renders later, so the whole
            // window snaps (see ROUTE_SNAP_MS).
            self.height_morph = None;
            self.last_target_height = 0.0;
            if (new_thread_launch || returning_to_new_thread)
                && !motion::reduced_motion(cx)
                && self.last_rendered_height > 0.0
            {
                // Both directions share one timeline. The blank canvas is
                // always expanded; an established session begins compact.
                self.expanded_mode = returning_to_new_thread;
                let now_ms =
                    self.morph_clock.elapsed().as_secs_f32() * 1000.0 / motion::speed_scale();
                self.flip_morph = Some(FlipMorph::new_thread_transition(
                    self.last_rendered_height,
                    now_ms,
                ));
                self.route_snap_until = None;
            } else {
                self.flip_morph = None;
                self.last_rendered_height = 0.0;
                self.route_snap_until = Some(Instant::now() + Duration::from_millis(ROUTE_SNAP_MS));
            }
            self.input.update(cx, |input, cx| input.set_text(draft, cx));
            self.sync_draft(cx);
        }

        // A pending agent question must not take over an active queue edit.
        if self.editing_queued.is_some() {
            self.on_input_edited(cx);
            cx.notify();
            return;
        }
        // Question panel lifecycle (wizard state cached per request id).
        match pending {
            Some((request_id, questions)) if !self.answered_requests.contains(&request_id) => {
                let same = self
                    .wizard
                    .as_ref()
                    .is_some_and(|w| w.request_id == request_id);
                if !same {
                    self.input.update(cx, |input, _| input.cancel_dictation());
                    if self.wizard.is_none() {
                        self.sync_draft(cx);
                    }
                    self.reset_mention(None, cx);
                    let prefill = questions
                        .first()
                        .and_then(|q| q.prefill.clone())
                        .unwrap_or_default();
                    self.wizard = Some(Wizard::new(request_id, questions));
                    self.input
                        .update(cx, |input, cx| input.set_text(prefill, cx));
                    self.advance_task = None;
                    // The shared input becomes the panel's free-text override.
                    self.input.update(cx, |input, cx| {
                        input.set_placeholder(ask_pill::PLACEHOLDER, cx)
                    });
                }
            }
            _ => {
                if let Some(wizard) = self.wizard.as_ref() {
                    // LATCH (original composer.tsx `inputLatch`): a transient
                    // fold/sync blip — or a steer appended behind the
                    // streaming entry — must not unmount the panel and lose
                    // the user's picks. Release only on explicit resolution
                    // (here or on another device) or when a NON-EMPTY
                    // transcript shows the question superseded (a newer
                    // assistant entry took over). Never on run death: the
                    // question stays answerable until answered — the engine
                    // delivers a dead run's answer as a resumed turn.
                    let transcript = self.state.read(cx).transcript.clone();
                    let released = input_request_resolved(&transcript, &wizard.request_id)
                        || (!transcript.is_empty()
                            && !self.answered_requests.contains(&wizard.request_id));
                    if released {
                        self.wizard = None;
                        self.advance_task = None;
                        let draft = crate::draft_store::entry(&self.current_key, cx).prompt;
                        let placeholder = self.idle_placeholder.clone();
                        self.input.update(cx, |input, cx| {
                            input.set_text(draft, cx);
                            input.set_placeholder(placeholder, cx);
                        });
                    }
                }
            }
        }
        let input_context = message_input_context(self.wizard.is_some());
        self.input
            .update(cx, |input, cx| input.set_key_context(input_context, cx));
        self.on_input_edited(cx);
        cx.notify();
    }

    pub(crate) fn run_live(&self, cx: &App) -> bool {
        let Some(chat_id) = self.bound_chat_id(cx) else {
            return false;
        };
        self.state
            .read(cx)
            .turn_in_flight(&chat_id, chrono::Utc::now())
    }

    fn reference_delivery_supported(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        let target = self
            .bound_chat_row(cx)
            .map(|chat| chat.device_id.clone())
            .or_else(|| state.effective_device_id());
        target.is_some_and(|device| {
            state.device_supports(&device, capabilities::COMPOSER_REFERENCES_V1)
        })
    }

    /// Check before consuming drafts, attachments, or an edited queue row.
    pub(crate) fn check_reference_delivery(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if references_require_update(text, self.reference_delivery_supported(cx)) {
            self.failure = Some("Update the selected device’s Zeron to send file, command, or skill references. Your draft is preserved.".into());
            self.failure_key = Some(self.current_key.clone());
            cx.notify();
            return false;
        }
        true
    }

    /// New chats need a runnable agent, but may target the device's home
    /// directory without a project. Existing chats carry their own run config.
    fn send_blocked(&self, cx: &App) -> bool {
        if self.queue_edit_finishing || self.compacting(cx) {
            return true;
        }
        let state = self.state.read(cx);
        if state.review_comment_flush_pending(&self.current_key) {
            return true;
        }
        if self.bound_chat_id(cx).is_some() {
            return false;
        }
        // New-chat canvas: needs a runnable agent. The
        // no-agents check only fires once the catalog is loaded — offline
        // and still-loading states must not block (the harness resolves from
        // the remembered default and the engine reports real failures).
        self.pickers.read(cx).no_agents_available()
    }

    fn button_mode(&self, cx: &App) -> SendButtonMode {
        if self.editing_queued.is_some() {
            return SendButtonMode::Send;
        }
        let has_text = self.input.read(cx).dictation.phase.active()
            || composer_has_content(
                self.input.read(cx).text(),
                self.staged().len() + self.staged_appshots().len(),
                self.staged_chip_count(cx),
            );
        send_button_mode(self.run_live(cx), has_text, self.turn_stopped(cx))
    }

    /// The bound chat's last turn stopped short and nothing is running:
    /// the send circle turns into the resume triangle.
    fn turn_stopped(&self, cx: &App) -> bool {
        let Some(chat_id) = self.bound_chat_id(cx) else {
            return false;
        };
        let state = self.state.read(cx);
        let entries = if self.chat_override.is_some() {
            state.sub_transcript(&chat_id)
        } else {
            &state.transcript
        };
        crate::transcript::resume_offer(entries, state.indicator_for(&chat_id, chrono::Utc::now()))
    }

    fn execute_workspace_command(
        &mut self,
        command: WorkspaceCommand,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) {
        // Actions consume only their trigger. Draft text, attachments and queued
        // edits remain in the composer; selecting a command never sends them.
        self.input
            .update(cx, |input, cx| input.remove_completion_token(range, cx));
        self.reset_slash(None, cx);
        self.failure = None;
        self.failure_key = None;
        cx.emit(ComposerEvent::WorkspaceCommand(command));
        cx.notify();
    }

    fn navigate_history(&mut self, delta: isize, cx: &mut Context<Self>) {
        let target = if self.wizard.is_some()
            || self.editing_queued.is_some()
            || self.chat_override.is_some()
            || self.input.read(cx).read_only
        {
            None
        } else {
            match (&self.prompt_history, delta < 0) {
                (None, true) => Some(0),
                (Some(cursor), true) => Some(cursor.ix + 1),
                (Some(cursor), false) => cursor.ix.checked_sub(1),
                (None, false) => None,
            }
        };
        let prompts = target
            .map(|_| sent_prompts(&self.state.read(cx).transcript))
            .unwrap_or_default();
        match target.and_then(|ix| prompts.get(ix).map(|prompt| (ix, prompt.clone()))) {
            Some((ix, prompt)) => {
                let stash = match self.prompt_history.take() {
                    Some(cursor) => cursor.stash,
                    None => self.input.read(cx).text().to_string(),
                };
                self.prompt_history = Some(PromptHistoryCursor {
                    ix,
                    stash,
                    shown: prompt.clone(),
                });
                self.input
                    .update(cx, |input, cx| input.set_text(prompt, cx));
            }
            None if delta > 0 && self.prompt_history.is_some() => {
                let stash = self.prompt_history.take().map(|cursor| cursor.stash);
                self.input.update(cx, |input, cx| {
                    input.set_text(stash.unwrap_or_default(), cx)
                });
            }
            None => self
                .input
                .update(cx, |input, cx| input.move_vertical(delta as f32, cx)),
        }
    }

    fn on_submit(&mut self, cx: &mut Context<Self>) {
        if self
            .input
            .update(cx, |input, cx| input.finish_dictation(true, cx))
        {
            return;
        }
        self.submit(false, cx);
    }

    fn submit(&mut self, invert_follow_up: bool, cx: &mut Context<Self>) {
        if self.commit_queue_edit(cx) {
            return;
        }
        if self.wizard.is_some() {
            // Enter inside the panel's free-text input submits the page.
            let typed = self.input.read(cx).text().to_string();
            if let Some(w) = self.wizard.as_mut() {
                w.set_typed(typed);
            }
            self.wizard_advance(cx);
            return;
        }
        // Leading indentation distinguishes literal Markdown from native commands
        // and skill invocations. Only the empty-content check may trim the draft.
        let text = self.input.read(cx).text().to_string();
        if let Some(action) = self
            .slash_cache
            .get(&self.slash.context)
            .and_then(|rows| workspace_command_for_text(self.input.read(cx).text(), rows))
        {
            self.execute_workspace_command(action, 0..self.input.read(cx).text().len(), cx);
            return;
        }
        let no_content = !composer_has_content(
            &text,
            self.staged().len() + self.staged_appshots().len(),
            self.staged_chip_count(cx),
        );
        match self.button_mode(cx) {
            // Enter never stops a run: Stop mode implies an empty composer,
            // so a stray extra Enter right after sending landed an interrupt
            // on the just-dispatched prompt and the agent ate it silently
            // (issue #406). Stop stays on the button — and on Esc when
            // escape_stops_active_agent is enabled.
            SendButtonMode::Stop | SendButtonMode::Resume => {}
            _ if no_content => {}
            _ if self.send_blocked(cx) => {}
            SendButtonMode::Send => self.send(text, false, false, false, cx),
            // Busy: the row joins the queue; whether it also steers the live
            // turn is the follow-up setting, inverted by the modifier chord.
            SendButtonMode::Queue => {
                let steer = follow_up_steers(
                    crate::settings::current(cx).follow_up_behavior,
                    invert_follow_up,
                );
                self.send(text, true, false, steer, cx)
            }
        }
    }

    /// Send `prompt` as this composer's turn without it ever being typed —
    /// the sidechat tab's first turn, handed over by `/sidechat <prompt>`.
    /// Queues instead when the bound chat is already busy, like a submit.
    pub(crate) fn send_prompt(&mut self, prompt: String, cx: &mut Context<Self>) {
        let queue = matches!(self.button_mode(cx), SendButtonMode::Queue);
        self.send(prompt, queue, false, false, cx);
    }

    /// The play button on a stopped turn: send the same prompt the user
    /// would type, under the guards a typed submit runs.
    fn resume(&mut self, cx: &mut Context<Self>) {
        if self.send_blocked(cx) {
            return;
        }
        self.send_prompt(crate::transcript::RESUME_PROMPT.to_string(), cx);
    }

    /// Cmd/Ctrl+Enter remains an ordinary submit while the composer carries
    /// content. With a truly empty composer it instead activates the most
    /// recently queued row, and never turns an empty chord into Stop.
    pub(crate) fn on_modified_submit(&mut self, cx: &mut Context<Self>) {
        if self
            .input
            .update(cx, |input, cx| input.finish_dictation(true, cx))
        {
            return;
        }
        if self.commit_queue_edit(cx) {
            return;
        }
        let text = self.input.read(cx).text().to_string();
        let has_content = composer_has_content(
            &text,
            self.staged().len() + self.staged_appshots().len(),
            self.staged_chip_count(cx),
        );
        // On a draft, the modifier chord starts the session in the
        // background and leaves a fresh draft here (t3code "Start in the
        // background", d23) — only where the chord is not the plain send.
        if has_content
            && self.wizard.is_none()
            && self.background_submit_available(cx)
            && !self.send_blocked(cx)
            && matches!(self.button_mode(cx), SendButtonMode::Send)
        {
            self.send(text, false, true, false, cx);
            return;
        }
        match modified_submit_target(has_content) {
            ModifiedSubmitTarget::SubmitContent => {
                let invert = modified_submit_inverts_follow_up(
                    crate::settings::current(cx).composer_send_behavior,
                );
                self.submit(invert, cx)
            }
            ModifiedSubmitTarget::ActivateLatestQueued => self.activate_latest_queued(cx),
        }
    }

    /// Mod+Enter is the background start only on the main composer's draft
    /// and only under the Enter send behavior; under Mod+Enter it IS the
    /// send, and the message-enter table adds no further modifier chords.
    fn background_submit_available(&self, cx: &App) -> bool {
        self.chat_override.is_none()
            && self.bound_chat_id(cx).is_none()
            && self.state.read(cx).active_draft.is_some()
            && crate::settings::current(cx).composer_send_behavior == ComposerSendBehavior::Enter
    }

    fn send(
        &mut self,
        text: String,
        queue: bool,
        background: bool,
        steer: bool,
        cx: &mut Context<Self>,
    ) {
        match zeron_engine::computer::activation::parse_computer_invocation(&text) {
            Some(prompt) if prompt.is_empty() => {
                self.failure = Some("Add a request after /computer-use.".into());
                self.failure_key = self.bound_chat_id(cx);
                cx.notify();
            }
            Some(_) if self.computer_preflight.is_some() => {}
            Some(_)
                if computer_preflight_applies(
                    cfg!(target_os = "macos"),
                    self.send_host_device(cx).as_deref(),
                    self.state.read(cx).local_device_id.as_deref(),
                ) =>
            {
                self.preflight_computer_send(text, queue, background, steer, cx)
            }
            _ => self.send_now(text, queue, background, steer, cx),
        }
    }

    fn computer_effort_hint(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        if !computer_preflight_applies(
            cfg!(target_os = "macos"),
            self.send_host_device(cx).as_deref(),
            state.local_device_id.as_deref(),
        ) {
            return false;
        }
        let active = state
            .computer_settings
            .as_ref()
            .is_some_and(|settings| settings.computer_control_enabled)
            || zeron_engine::computer::activation::parse_computer_invocation(
                self.input.read(cx).text(),
            )
            .is_some();
        active
            && self.pickers.read(cx).computer_effort_hint_applies(cx)
            && !crate::settings::current(cx).computer_effort_hint_dismissed
    }

    fn send_host_device(&self, cx: &App) -> Option<String> {
        match self.bound_chat_row(cx) {
            Some(chat) => Some(chat.device_id.clone()),
            None => {
                let state = self.state.read(cx);
                state
                    .effective_device_id()
                    .or_else(|| state.local_device_id.clone())
            }
        }
    }

    /// Granting never sends: a missing grant opens the guide and keeps the draft.
    fn preflight_computer_send(
        &mut self,
        text: String,
        queue: bool,
        background: bool,
        steer: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return self.send_now(text, queue, background, steer, cx);
        };
        let draft = self.input.read(cx).text().to_string();
        let chat = self.bound_chat_id(cx);
        self.computer_preflight = Some(cx.spawn(async move |this, cx| {
            let call = engine
                .client()
                .call(methods::GET_COMPUTER_SETTINGS, serde_json::json!({}));
            let deadline = cx.background_executor().timer(Duration::from_secs(5));
            futures::pin_mut!(call);
            let settings = match futures::future::select(call, deadline).await {
                futures::future::Either::Left((reply, _)) => reply.ok(),
                futures::future::Either::Right(_) => None,
            };
            let settings_enabled = settings
                .and_then(|value| {
                    serde_json::from_value::<zeron_proto::computer::ComputerSettings>(value).ok()
                })
                .is_some_and(|settings| settings.computer_control_enabled);
            this.update(cx, |composer, cx| {
                composer.computer_preflight = None;
                if composer.input.read(cx).text() != draft || composer.bound_chat_id(cx) != chat {
                    return;
                }
                let missing =
                    computer_send_blockers(settings_enabled, crate::appshots::capabilities());
                if missing.is_empty() {
                    composer.send_now(text, queue, background, steer, cx);
                } else {
                    cx.emit(ComposerEvent::OpenComputerSetup { missing });
                }
            })
            .ok();
        }));
    }

    pub(crate) fn enable_computer_use(&mut self, cx: &mut Context<Self>) {
        let draft = self.input.read(cx).text().to_string();
        if zeron_engine::computer::activation::parse_computer_invocation(&draft).is_none() {
            let next = if draft.trim().is_empty() {
                format!("/{COMPUTER_USE_COMMAND} ")
            } else {
                format!("/{COMPUTER_USE_COMMAND} {draft}")
            };
            self.input.update(cx, |input, cx| input.set_text(next, cx));
            self.sync_draft(cx);
        }
        cx.notify();
    }

    fn render_computer_strip(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let chat_id = self.bound_chat_id(cx)?;
        let presence = crate::computer::presence::for_chat(&chat_id, cx)?;
        let show_esc = crate::computer::strip::shows_stop(&presence)
            && !crate::computer::cards::missing_computer_permissions(
                crate::appshots::capabilities(),
            )
            .contains(&zeron_proto::computer::ComputerPermission::InputMonitoring);
        let state = self.state.clone();
        let on_answer = {
            let (state, chat_id) = (state.clone(), chat_id.clone());
            Box::new(
                move |input_id: String,
                      answers: Vec<UserInputAnswer>,
                      _: &mut Window,
                      cx: &mut App| {
                    crate::computer::status_menu::answer_pending(
                        &state,
                        chat_id.clone(),
                        input_id,
                        answers,
                        cx,
                    );
                },
            )
        };
        let on_stop = Box::new(move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
            let Some(engine) = state.read(cx).engine().cloned() else {
                return;
            };
            let chat_id = chat_id.clone();
            cx.spawn(async move |_| {
                if let Err(error) = engine
                    .client()
                    .call(
                        methods::COMPUTER_REVOKE_DESKTOP,
                        serde_json::json!({ "chatId": chat_id }),
                    )
                    .await
                {
                    tracing::warn!(target: "desktop_cua", "revoke desktop: {error}");
                }
            })
            .detach();
        });
        Some(
            div()
                .mx(px(14.0))
                .mb(px(-(Theme::SPACE_SM + 1.0)))
                .child(crate::computer::strip::computer_strip(
                    &presence, show_esc, on_stop, on_answer, theme,
                ))
                .into_any_element(),
        )
    }

    /// Queue a Run doc command with an optimistic echo — or, with the agent
    /// busy, park the message on the chat's pending queue instead. New chats
    /// thread the picked config in: worktree creation (when the isolated toggle
    /// is on), `Mutate createChat` with the `ChatConfig` + cwd, and the model /
    /// reasoning / options on the Run request itself (§1.7).
    fn send_now(
        &mut self,
        text: String,
        queue: bool,
        background: bool,
        steer: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .bound_chat_row(cx)
            .is_some_and(|chat| chat.sidechat_expired(chrono::Utc::now()))
        {
            self.failure =
                Some("Sidechat expired after 24 hours. Create a new sidechat to continue.".into());
            self.failure_key = self.bound_chat_id(cx);
            cx.notify();
            return;
        }
        if !self.check_reference_delivery(&text, cx) {
            return;
        }
        // `/compact` is zeron's own command: it becomes an RPC, never a turn,
        // so it never builds a `Run` and never writes a user entry.
        let plain = zeron_proto::invocation::invocation_prompt(&text);
        if let Some(focus) = compact_focus(&plain) {
            self.input.update(cx, |input, cx| input.set_text("", cx));
            self.sync_draft(cx);
            self.request_compaction(focus, cx);
            return;
        }
        // `/sidechat` is zeron's own too: it opens a side conversation and
        // never runs a turn on THIS chat.
        if let Some(prompt) = sidechat_prompt(&plain) {
            self.input.update(cx, |input, cx| input.set_text("", cx));
            self.request_sidechat(prompt, true, cx);
            self.sync_draft(cx);
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.failure = Some("Engine not connected".into());
            self.failure_key = None; // global — meaningful on every chat
            cx.notify();
            return;
        };
        // Chat id: existing selection, or client-minted for the new-chat canvas
        // (the chat then appears from the doc host once the doc materializes).
        // Chat id: existing selection, or the draft's pre-minted id (d20) so
        // the tab and every per-key map carry straight over to the session.
        let (chat_id, is_new) = match self.bound_chat_id(cx) {
            Some(id) => (id, false),
            None => (
                self.state
                    .read(cx)
                    .active_draft
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                true,
            ),
        };
        if !is_new {
            crate::computer::preview::clear_revoked(&chat_id, cx);
        }
        // Where the new session runs (Current checkout / reuse an existing
        // worktree / fresh worktree off the picked base) — resolved NOW so
        // the async block needs no picker access.
        let plan = self.pickers.read(cx).checkout_plan();
        // Fully-resolved model/reasoning/options — concrete values (chat config
        // or defaults), so the engine never has to guess a "default".
        let mut resolved = self.pickers.read(cx).resolved(cx);
        // The pickers follow the SELECTION; a bound composer runs its own
        // chat's configuration instead of the source chat's.
        if self.chat_override.is_some()
            && let Some(config) = self.bound_chat_row(cx).and_then(|chat| chat.config.clone())
        {
            resolved.harness = Some(config.harness);
            resolved.model = config.model;
            resolved.reasoning = config.reasoning;
            resolved.model_options = config.model_options;
            resolved.permission_mode = config.permission_mode;
        }
        if self.pr_chat {
            let Some(harness) = resolved.harness else {
                self.failure = Some("Pick an agent before sending.".into());
                self.failure_key = self.bound_chat_id(cx);
                cx.notify();
                return;
            };
            if !zeron_harness::asks_before_running_tools(harness) {
                self.failure = Some(
                    "This agent can't ask before running commands. Pick Claude Code or Codex for this chat."
                        .into(),
                );
                self.failure_key = self.bound_chat_id(cx);
                cx.notify();
                return;
            }
            resolved.permission_mode = zeron_proto::PermissionMode::Ask;
        }
        let existing_cwd = self.bound_chat_row(cx).and_then(|c| c.cwd.clone());
        // The PROJECT fixes the new chat's device + base folder — sessions are
        // minted onto the project's device, not necessarily this one. With no
        // project ("Don't work in a project") the composer's device pick is
        // the host and the session runs from `~` there.
        let space = self.state.read(cx).selected_space_row().cloned();
        let local_device_id = self.state.read(cx).local_device_id.clone();
        let target_device_id = self.state.read(cx).effective_device_id();
        let device_id = if is_new {
            target_device_id
                .clone()
                .unwrap_or_else(|| "local".to_string())
        } else {
            self.bound_chat_row(cx)
                .map(|c| c.device_id.clone())
                .or_else(|| local_device_id.clone())
                .unwrap_or_else(|| "local".to_string())
        };
        // Uploads/read-backs target the chat's HOST device (forwardable RPCs);
        // for a new chat that's the target device (None when it's local).
        let host_device_id = if is_new {
            target_device_id
                .clone()
                .filter(|id| local_device_id.as_deref() != Some(id.as_str()))
        } else {
            self.bound_chat_row(cx).map(|c| c.device_id.clone())
        };
        let space_id = space.as_ref().map(|s| s.id.clone());
        let space_path = space.as_ref().map(|s| s.path.clone());
        let create_side_chat = self.state.update(cx, |state, _| {
            if state.side_chat_unsaved()
                && let Some(mut config) = resolved.chat_config()
            {
                // Freeze the same resolved provider settings in createChat and
                // Run, including defaults learned since the harness was picked.
                if let Some(existing) = state.selected_chat_row().and_then(|c| c.config.as_ref()) {
                    config.sandbox = existing.sandbox;
                }
                state.apply_chat_config(&chat_id, config);
            }
            state.unsaved_side_chat_create(&chat_id)
        });
        if queue && !is_new {
            let capability = if self.staged().is_empty() && self.staged_appshots().is_empty() {
                capabilities::MESSAGE_QUEUE_V1
            } else {
                capabilities::MESSAGE_QUEUE_ATTACHMENTS_V1
            };
            if !engine.engine_info().supports(capability)
                || !self.state.read(cx).chat_host_supports(&chat_id, capability)
            {
                self.failure =
                    Some("Update the chat's engine to queue messages during a response.".into());
                cx.notify();
                return;
            }
        }
        // Snapshot-and-clear NOW (use-attachments.ts takeAttachments): the
        // strip empties the instant you hit send; a failure hands the files
        // back into the chat's stash.
        let ordinary_staged = self
            .attachments
            .remove(&self.current_key)
            .unwrap_or_default();
        self.attachment_drafts.remove(&self.current_key);
        let staged_appshots = self.appshots.remove(&self.current_key).unwrap_or_default();
        let mut staged = ordinary_staged.clone();
        staged.extend(
            staged_appshots
                .iter()
                .map(|appshot| appshot.screenshot.clone()),
        );
        // `typed` keeps the user's own words for the failure hand-back below:
        // restoring the folded prompt would paste the comment block into the
        // input as literal text.
        let key = self.current_key.clone();
        let comments = self.state.update(cx, |state, cx| {
            let taken = state.take_review_comments(&key);
            if !taken.is_empty() {
                cx.notify();
            }
            taken
        });
        let quotes = self.state.update(cx, |state, cx| {
            let taken = state.take_quotes(&key);
            if !taken.is_empty() {
                cx.notify();
            }
            taken
        });
        let typed = text.clone();
        // A chip for an attachment that is no longer staged must not leave as a link.
        let attached: Vec<u32> = ordinary_staged
            .iter()
            .filter_map(|att| att.mention)
            .collect();
        let text = zeron_proto::attachment_mentions::demote_unattached_mentions(&text, &attached);
        let text = crate::quotes::with_quotes(&text, &quotes);
        let text = crate::comments::with_comments(&text, &comments);
        let text = match &self.pr_context {
            Some(block) if create_side_chat.is_some() => format!("{text}{block}"),
            _ => text,
        };
        self.preview = None;
        let message_id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().timestamp_millis();
        // Existing busy chats always queue; compatibility was checked before
        // taking the draft, attachments, or review comments.
        let queue = queue && !is_new;
        let clean_queue_attachment_text = staged.is_empty()
            || (engine
                .engine_info()
                .supports(capabilities::MESSAGE_QUEUE_CLEAN_ATTACHMENT_TEXT_V1)
                && self.state.read(cx).chat_host_supports(
                    &chat_id,
                    capabilities::MESSAGE_QUEUE_CLEAN_ATTACHMENT_TEXT_V1,
                ));

        // Queued-attachment flow (durable-by-design): stage the bytes on the
        // LOCAL engine, queue the command immediately with `pending://` refs,
        // and let the engine push the bytes to a remote host afterwards —
        // staging must never gate the queue (2026-08-19 incident: a send
        // died with a zombie peer link because the upload sat in front of
        // QueueCommand). Requires every engine involved to understand the
        // ref scheme — the local engine (an IPC daemon may be older than
        // this UI) and, for remotely-hosted chats, the host; anything older
        // keeps the legacy blocking upload.
        let host_is_remote = host_device_id
            .as_deref()
            .is_some_and(|id| local_device_id.as_deref() != Some(id));
        // Queue rows do not carry upstream's attachment-transfer escort, so
        // they retain the proven host-upload path and store absolute refs.
        // Appshot XML attributes require escaped final paths. The engine's plain
        // string replacement of pending refs cannot safely rewrite those, so
        // rich captures use the existing upload-before-send path.
        let queued_flow = !queue && staged_appshots.is_empty() && !staged.is_empty() && {
            let state = self.state.read(cx);
            let local_ok = local_device_id
                .as_deref()
                .is_some_and(|id| state.device_version_at_least(id, QUEUED_ATTACHMENTS_MIN));
            let host_ok = !host_is_remote
                || host_device_id
                    .as_deref()
                    .is_some_and(|id| state.device_version_at_least(id, QUEUED_ATTACHMENTS_MIN));
            local_ok && host_ok
        };
        // Upload identities minted NOW: in the queued flow the `pending://`
        // ref IS the persisted transport until the host rewrites it, so the
        // id must exist before any bytes move.
        let upload_ids: Vec<String> = staged
            .iter()
            .map(|_| uuid::Uuid::new_v4().to_string())
            .collect();
        // The echo carries attachment refs from the first frame, so photos
        // render while the send is still pending. Queued flow: the refs are
        // the real `pending://` identities (stable — no post-upload refresh).
        // Legacy flow: synthetic `pending/…` paths that the post-upload
        // refresh replaces with the host's absolute paths. Either way the
        // staged bytes are seeded into the transcript cache under every
        // device key the transcript consults.
        let echo_paths: Vec<String> = if queued_flow {
            staged
                .iter()
                .zip(&upload_ids)
                .map(|(att, id)| format!("pending://{id}/{}", att.name))
                .collect()
        } else {
            staged
                .iter()
                .map(|att| format!("pending/{}/{}", att.id, att.name))
                .collect()
        };
        let echo_appshot_paths: HashMap<String, String> = staged
            .iter()
            .zip(&echo_paths)
            .map(|(attachment, path)| (attachment.id.clone(), path.clone()))
            .collect();
        let echo_text = attachments::with_attachments(
            &appshots::with_appshots(&text, &staged_appshots, &echo_appshot_paths),
            &echo_paths,
        );
        // Queued flow also seeds the UPLOAD ALIAS: the host rewrites the
        // persisted ref to `{its uploads dir}/{id8}-{name}` — an absolute
        // path the sender can't predict, but whose id8 it minted. The alias
        // keeps the thumbnail on the already-local bytes through that
        // rewrite instead of blanking into a reload skeleton.
        if queued_flow {
            for (upload_id, att) in upload_ids.iter().zip(&staged) {
                attachments::seed_staged_alias(&device_id, upload_id, att);
                if let Some(local) = local_device_id.as_deref()
                    && local != device_id
                {
                    attachments::seed_staged_alias(local, upload_id, att);
                }
            }
        }
        for (path, att) in echo_paths.iter().zip(&staged) {
            attachments::seed_staged(&device_id, path, att);
            if let Some(local) = local_device_id.as_deref()
                && local != device_id
            {
                attachments::seed_staged(local, path, att);
            }
        }

        // Optimistic echo (client-minted id doubles as the persisted message id,
        // so the doc frame dedups it away).
        let echo = SessionMessageEntry {
            id: message_id.clone(),
            role: zeron_doc::MessageRole::User,
            parts: vec![MessagePart::Text {
                id: "t0".into(),
                text: echo_text.clone(),
            }],
            created_at,
            device_id: "local".into(),
            status: None,
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        };
        // Background start (d23): the session is minted and run without
        // becoming the selection; the canvas moves on to a fresh draft that
        // keeps this one's target, ref and checkout kind.
        let background = background && is_new;
        self.launching_new_chat = is_new && !background;
        if is_new && !background {
            cx.emit(ComposerEvent::NewThreadTransitionStarted);
        }
        // A queued message is not in the transcript yet — the queue panel is
        // its echo, and it gets a real bubble when the host sends it.
        self.state.update(cx, |s, cx| {
            if background {
                s.start_draft_in_background(&chat_id, cx);
            } else if is_new {
                s.select_chat(Some(chat_id.clone()), cx);
            }
            if should_publish_optimistic_echo(queue) {
                s.push_echo(&chat_id, echo);
                // Working overlay until the host executes the queued command —
                // without it a remote send flashed Completed (and could ring
                // the done-chime) in the queue→drain→sync gap.
                s.begin_pending_send(&chat_id, &message_id, chrono::Utc::now());
            }
            cx.notify();
        });

        self.input.update(cx, |input, cx| input.set_text("", cx));
        self.sync_draft(cx);
        self.failure = None;
        self.sending = true;
        // A queued row is represented by the queue panel, not the transcript.
        // Claiming an own-turn anchor for it here would replace the live
        // turn's runway with an id that has no transcript row yet. A
        // background start has no transcript on screen to anchor.
        if should_publish_optimistic_echo(queue) && !background {
            cx.emit(ComposerEvent::Sent {
                chat_id: chat_id.clone(),
                message_id: message_id.clone(),
            });
        }
        if background {
            cx.emit(ComposerEvent::BackgroundStarted);
        }
        cx.notify();

        let restore_text = typed;
        let err_chat_id = chat_id.clone();
        let err_message_id = message_id.clone();
        self.send_task = Some(cx.spawn(async move |this, cx| {
            let result: Result<Option<String>, String> = async {
                // Attachments stage FIRST — before the chat row or anything
                // else exists. Staging is chat-independent (keyed by
                // uploadId), and ordering it first makes a new-chat send
                // atomic: a staging failure aborts with NOTHING created,
                // instead of stranding a just-minted empty chat (v0.2.12
                // "failed to stage → empty transcript" report).
                //
                // Queued flow: commit the bytes to the LOCAL engine's uploads
                // dir (fast, offline-safe) — the queued command carries the
                // `pending://` refs and the engine delivers the bytes to a
                // remote host afterwards, retrying until they land. Legacy
                // flow (old engines): stage on the host device up front,
                // bounded by a total budget so a degraded link fails the send
                // loudly instead of grinding through silent per-chunk retries
                // for minutes.
                let mut content = text.clone();
                let mut attachment_paths: Vec<String> = Vec::new();
                let mut transfers: Vec<serde_json::Value> = Vec::new();
                if !staged.is_empty() && queued_flow {
                    // Local staging is disk-speed; publish progress anyway so
                    // huge files still narrate.
                    let progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                    let total: u64 = staged.iter().map(|a| a.bytes().len() as u64).sum();
                    {
                        let progress = progress.clone();
                        this.update(cx, |composer, cx| {
                            composer.state.update(cx, |s, cx| {
                                s.begin_upload_progress(total, progress);
                                cx.notify();
                            });
                        })
                        .ok();
                    }
                    for (att, upload_id) in staged.iter().zip(&upload_ids) {
                        if let Err(err) = attachments::upload_attachment(
                            &engine,
                            cx.background_executor(),
                            None,
                            upload_id,
                            att,
                            Some(progress.clone()),
                        )
                        .await
                        {
                            tracing::warn!(name = %att.name, error = %err, "local attachment stage failed");
                            return Err("Couldn't stage the attachment locally.".to_string());
                        }
                        transfers.push(serde_json::json!({
                            "uploadId": upload_id,
                            "fileName": att.name,
                        }));
                    }
                    // The echo refs ARE the persisted refs — no refresh pass.
                    attachment_paths = echo_paths.clone();
                    content = echo_text.clone();
                } else if !staged.is_empty() {
                    let progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                    let total: u64 = staged.iter().map(|a| a.bytes().len() as u64).sum();
                    {
                        let progress = progress.clone();
                        this.update(cx, |composer, cx| {
                            composer.state.update(cx, |s, cx| {
                                s.begin_upload_progress(total, progress);
                                cx.notify();
                            });
                        })
                        .ok();
                    }
                    for (att, upload_id) in staged.iter().zip(&upload_ids) {
                        match attachments::upload_attachment(
                            &engine,
                            cx.background_executor(),
                            host_device_id.as_deref(),
                            upload_id,
                            att,
                            Some(progress.clone()),
                        )
                        .await
                        {
                            Ok(path) => attachment_paths.push(path),
                            Err(err) => {
                                tracing::warn!(name = %att.name, error = %err, "attachment upload failed");
                                return Err(
                                    "Couldn't upload the attachment — the device may be offline."
                                        .to_string(),
                                );
                            }
                        }
                    }
                    // Seed the transcript cache from local bytes so the sent
                    // bubble's thumbnails never round-trip (seedTranscript-
                    // Attachment in the original send path).
                    let seed_device = host_device_id.clone().unwrap_or_else(|| device_id.clone());
                    for (path, att) in attachment_paths.iter().zip(&staged) {
                        attachments::seed_staged(&seed_device, path, att);
                        if seed_device != device_id {
                            attachments::seed_staged(&device_id, path, att);
                        }
                    }
                    let appshot_paths: HashMap<String, String> = staged
                        .iter()
                        .zip(&attachment_paths)
                        .map(|(attachment, path)| (attachment.id.clone(), path.clone()))
                        .collect();
                    content = attachments::with_attachments(
                        &appshots::with_appshots(&text, &staged_appshots, &appshot_paths),
                        &attachment_paths,
                    );
                    // A normal send already has an optimistic echo: refresh it
                    // in place with the uploaded refs so its thumbnails never
                    // flicker. A queued message has no transcript echo at all;
                    // its queue row is the only representation until dispatch.
                    if should_publish_optimistic_echo(queue) {
                        let refreshed = SessionMessageEntry {
                            id: message_id.clone(),
                            role: zeron_doc::MessageRole::User,
                            parts: vec![MessagePart::Text {
                                id: "t0".into(),
                                text: content.clone(),
                            }],
                            created_at,
                            device_id: "local".into(),
                            status: None,
                            continuation_of: None,
                            from_chat_id: None,
                            fork_import: false,
                            answers_request_id: None,
                            duration_ms: None,
                        };
                        let echo_chat_id = chat_id.clone();
                        this.update(cx, |composer, cx| {
                            composer.state.update(cx, |s, cx| {
                                s.remove_echo(&echo_chat_id, &message_id);
                                s.push_echo(&echo_chat_id, refreshed);
                                cx.notify();
                            });
                        })
                        .ok();
                    }
                }

                // Resolve the working directory: existing chats keep theirs;
                // new chats run per the checkout plan (t3code env-mode): the
                // space's folder as-is, an EXISTING worktree of the picked ref
                // (a plain cwd override — multiple sessions share one
                // worktree), or a fresh isolated worktree created off the
                // picked base ref (CreateWorktree on send, targeted at the
                // space's device; the RPC relay-forwards).
                let mut cwd = if is_new {
                    // Project-less sessions run from the host's home dir —
                    // "~" is expanded on the host when the run spawns.
                    space_path.clone().or_else(|| Some("~".to_string()))
                } else {
                    existing_cwd
                }
                .unwrap_or_else(|| ".".to_string());
                let mut worktree_cwd: Option<String> = None;
                // Fresh-worktree plans ride the QUEUED Run command (a
                // WorktreeSpec the HOST materializes at drain time) instead of
                // a blocking CreateWorktree relay RPC here: the RPC had no
                // timeout, so a lost relay frame wedged the send on "Sending…"
                // forever while the session ran remotely anyway (2026-08-18).
                let mut run_worktree: Option<zeron_proto::WorktreeSpec> = None;
                // The picked ref rides createChat so the session footer names
                // it from the first frame (it read "Select ref" until the
                // host's diff reconciler got around to stamping the branch).
                let mut chat_branch: Option<String> = None;
                if is_new && space_path.is_some() {
                    match &plan {
                        crate::pickers::CheckoutPlan::CurrentCheckout { branch } => {
                            chat_branch = branch.clone();
                        }
                        crate::pickers::CheckoutPlan::ReuseWorktree { path, branch } => {
                            cwd = path.clone();
                            worktree_cwd = Some(path.clone());
                            chat_branch = Some(branch.clone());
                        }
                        crate::pickers::CheckoutPlan::NewWorktree { base } => {
                            // Footer shows the base until the host stamps the
                            // actual zeron/<name> branch post-creation. cwd
                            // stays the repo folder — an old host that doesn't
                            // know the spec degrades to the main checkout
                            // instead of failing the run.
                            chat_branch = base.clone();
                            if let Some(repo_path) = &space_path {
                                // A remote repo's branch list loads over the
                                // relay — on a bad link it may never arrive
                                // and the picker has no base. That must NOT
                                // silently drop the isolation the user picked
                                // (2026-08-19: "New worktree" ran in the main
                                // checkout): default to HEAD, which git — any
                                // host version — resolves as the repo's
                                // current checkout state.
                                let base =
                                    base.clone().unwrap_or_else(|| "HEAD".to_string());
                                run_worktree = Some(zeron_proto::WorktreeSpec {
                                    repo_path: repo_path.clone(),
                                    base,
                                    space_id: space_id.clone(),
                                });
                            }
                        }
                    }
                }

                // Best-effort Mutate createChat with the picked config: the
                // engine resolves device + cwd from the PROJECT row when one
                // is picked; project-less chats name the host device outright
                // (idempotent; the doc host would materialize the chat on
                // first command anyway, so failures are non-fatal).
                if is_new {
                    let mut mutate = serde_json::json!({
                        "op": "createChat",
                        "chatId": chat_id,
                    });
                    if let Some(object) = mutate.as_object_mut() {
                        match &space_id {
                            Some(space_id) => {
                                object.insert(
                                    "spaceId".into(),
                                    serde_json::Value::String(space_id.clone()),
                                );
                            }
                            None => {
                                object.insert(
                                    "deviceId".into(),
                                    serde_json::Value::String(device_id.clone()),
                                );
                            }
                        }
                    }
                    if let Some(object) = mutate.as_object_mut() {
                        if let Some(worktree_cwd) = &worktree_cwd {
                            object.insert(
                                "cwd".into(),
                                serde_json::Value::String(worktree_cwd.clone()),
                            );
                        }
                        if let Some(branch) = &chat_branch {
                            object.insert(
                                "branch".into(),
                                serde_json::Value::String(branch.clone()),
                            );
                        }
                        if let Some(config) = resolved.chat_config()
                            && let Ok(config) = serde_json::to_value(&config)
                        {
                            object.insert("config".into(), config);
                        }
                    }
                    if let Err(err) = attachments::call_with_timeout(
                        &engine,
                        cx.background_executor(),
                        methods::MUTATE,
                        mutate,
                        std::time::Duration::from_secs(30),
                    )
                    .await
                    {
                        tracing::warn!(error = %err, "CreateChat mutate unavailable; doc host will materialize the chat");
                    }
                }
                // A hand-started side chat is minted by its first send. Unlike
                // a fresh session this one must land: the doc host would
                // materialize it without its parent link.
                if let Some(create) = create_side_chat {
                    if let Err(err) = attachments::call_with_timeout(
                        &engine,
                        cx.background_executor(),
                        methods::MUTATE,
                        create,
                        std::time::Duration::from_secs(30),
                    )
                    .await
                    {
                        tracing::warn!(error = %err, "side chat createChat failed");
                        return Err("Couldn't create the side chat.".to_string());
                    }
                    let saved = chat_id.clone();
                    this.update(cx, |composer, cx| {
                        composer
                            .state
                            .update(cx, |s, cx| s.side_chat_saved(&saved, cx));
                    })
                    .ok();
                }

                if queue {
                    // A queue row is editable UI state, so its text must stay
                    // free of the internal attachment-path trailer. The host
                    // rebuilds that transport when it promotes the row.
                    let appshot_paths = staged.iter().zip(&attachment_paths)
                        .map(|(attachment, path)| (attachment.id.clone(), path.clone())).collect();
                    let queue_body = appshots::with_appshots(&text, &staged_appshots, &appshot_paths);
                    let queue_text = if !clean_queue_attachment_text {
                        content.as_str()
                    } else if queue_body.trim().is_empty() && !attachment_paths.is_empty() {
                        attachments::attachment_only_text(&attachment_paths)
                    } else {
                        queue_body.as_str()
                    };
                    let params = serde_json::json!({
                        "chatId": chat_id,
                        "text": queue_text,
                        "attachments": attachment_paths,
                        "holdForTurnEnd": true,
                        "steerNow": steer,
                    });
                    let reply = engine
                        .client()
                        .call(methods::QUEUE_MESSAGE, params)
                        .await
                        .map_err(|e| format!("Send failed: {e}"))?;
                    let queue_id = reply
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| "Send failed: queue did not return an id".to_string())?;
                    return Ok(Some(queue_id.to_string()));
                }

                let expects_setup_handoff = run_worktree
                    .as_ref()
                    .and_then(|spec| spec.space_id.as_ref())
                    .is_some();
                let command = SessionCommandPayload::Run {
                    request: RunRequest {
                        mcp: None,
                        prompt: content.clone(),
                        harness: resolved.harness,
                        model: resolved.model.clone(),
                        reasoning: resolved.reasoning,
                        model_options: resolved.model_options.clone(),
                        cwd,
                        sandbox: SandboxLevel::WorkspaceWrite,
                        permission_mode: resolved.permission_mode,
                        auto_approve: false,
                        resume: None,
                        attachments: attachment_paths,
                        worktree: run_worktree,
                        branch: None,
                        forked_at: None,
                        computer_control: false,
                    },
                    message_id: message_id.clone(),
                };
                let command = serde_json::to_value(&command)
                    .map_err(|e| format!("Send failed: {e}"))?;
                let mut params = serde_json::json!({ "chatId": chat_id, "command": command });
                if !transfers.is_empty() {
                    params["transfers"] = serde_json::Value::Array(transfers);
                }
                // Deadline-bounded: QueueCommand is a local write (in-process
                // or IPC), but a deferred engine handle can park forever.
                let queued = attachments::call_with_timeout(
                    &engine,
                    cx.background_executor(),
                    methods::QUEUE_COMMAND,
                    params,
                    std::time::Duration::from_secs(30),
                )
                .await
                .map_err(|e| format!("Send failed: {e}"))?;
                if expects_setup_handoff
                    && let Some(command_id) = queued
                        .get("commandId")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                {
                    let poll_engine = engine.clone();
                    let poll_chat_id = chat_id.clone();
                    let poll_target_device_id = host_device_id.clone();
                    this.update(cx, |_, cx| {
                        cx.spawn(async move |this, cx| {
                            for _ in 0..480 {
                                let mut params = serde_json::json!({
                                    "chatId": poll_chat_id,
                                    "commandId": command_id,
                                });
                                if let (Some(target), Some(object)) =
                                    (&poll_target_device_id, params.as_object_mut())
                                {
                                    object.insert(
                                        "targetDeviceId".into(),
                                        serde_json::Value::String(target.clone()),
                                    );
                                }
                                match attachments::call_with_timeout(
                                    &poll_engine,
                                    cx.background_executor(),
                                    methods::TAKE_PROJECT_ACTION_SETUP,
                                    params,
                                    std::time::Duration::from_secs(10),
                                )
                                .await
                                {
                                    Ok(value) if value.get("ready").and_then(|v| v.as_bool()) == Some(true) => {
                                        let setup_action = value
                                            .get("setupAction")
                                            .cloned()
                                            .filter(|value| !value.is_null())
                                            .and_then(|value| serde_json::from_value(value).ok());
                                        let setup_error = value
                                            .get("setupError")
                                            .and_then(|value| value.as_str())
                                            .map(str::to_string);
                                        this.update(cx, |_, cx| {
                                            cx.emit(ComposerEvent::WorktreeSetup {
                                                chat_id: poll_chat_id.clone(),
                                                setup_action,
                                                setup_error,
                                                target_device_id: poll_target_device_id.clone(),
                                            });
                                        })
                                        .ok();
                                        return;
                                    }
                                    Err(error) if error.starts_with("unknown method: ") => return,
                                    Ok(_) | Err(_) => {}
                                }
                                cx.background_executor()
                                    .timer(Duration::from_millis(250))
                                    .await;
                            }
                            tracing::warn!(
                                chat = %poll_chat_id,
                                command = %command_id,
                                "worktree setup handoff timed out"
                            );
                        })
                        .detach();
                    })
                    .ok();
                }
                Ok(None)
            }
            .await;
            if result.is_err() && is_new {
                // A failed new-chat send must not strand a just-minted empty
                // chat in the sidebar (v0.2.12 "empty transcript" report).
                // Staging now runs before CreateChat, so usually nothing was
                // created — but a post-mutate failure (QueueCommand) still
                // leaves a row. Best-effort delete; a no-op if the chat was
                // never materialized.
                let _ = attachments::call_with_timeout(
                    &engine,
                    cx.background_executor(),
                    methods::MUTATE,
                    serde_json::json!({ "op": "deleteChat", "chatId": err_chat_id }),
                    std::time::Duration::from_secs(5),
                )
                .await;
            }
            this.update(cx, |composer, cx| {
                composer.sending = false;
                composer
                    .state
                    .update(cx, |s, _| s.end_upload_progress());
                if let Ok(Some(message_id)) = &result {
                    cx.emit(ComposerEvent::Queued {
                        chat_id: err_chat_id.clone(),
                        message_id: message_id.clone(),
                    });
                }
                if let Err(message) = result {
                    // Failure: red banner, echo removed, prompt back in the
                    // draft, staged files back in the stash. A failed NEW
                    // chat restores to the CANVAS (key "") and navigates back
                    // there — the minted chat is gone (deleted above), so
                    // nothing may restore under its key.
                    composer.failure = Some(message.into());
                    let restore_key = composer.state.update(cx, |s, cx| {
                        s.remove_echo(&err_chat_id, &err_message_id);
                        s.end_pending_send(&err_chat_id, &err_message_id);
                        if is_new && s.selected_chat.as_deref() == Some(err_chat_id.as_str()) {
                            // Back to the canvas — a fresh draft (the failed
                            // one was promoted away); the navigation
                            // draft-swap loads the restored text below.
                            s.select_chat(None, cx);
                        }
                        let restore_key = if is_new {
                            s.ensure_draft(cx)
                        } else {
                            err_chat_id.clone()
                        };
                        for comment in &comments {
                            s.add_review_comment(&restore_key, comment.clone());
                        }
                        for quote in &quotes {
                            s.add_quote(&restore_key, quote.clone());
                        }
                        cx.notify();
                        restore_key
                    });
                    composer.failure_key = Some(restore_key.clone());
                    if composer.current_key == restore_key {
                        composer.input.update(cx, |input, cx| input.set_text(restore_text.clone(), cx));
                    }
                    if !ordinary_staged.is_empty() {
                        // Merge by id (stashAttachments): files the user staged
                        // while the send was in flight survive the hand-back —
                        // draining the minted chat's slot too when the restore
                        // target is the canvas.
                        let mut merged = ordinary_staged.clone();
                        for key in [err_chat_id.clone(), restore_key.clone()] {
                            if let Some(slot) = composer.attachments.get_mut(&key) {
                                let fresh: Vec<_> = slot
                                    .drain(..)
                                    .filter(|e| !merged.iter().any(|f| f.id == e.id))
                                    .collect();
                                merged.extend(fresh);
                            }
                        }
                        composer.attachments.insert(restore_key.clone(), merged);
                    }
                    composer.restore_failed_appshots(&staged_appshots, &err_chat_id, &restore_key);
                    if crate::draft_store::set(&restore_key, crate::draft_store::DraftEntry {
                        prompt: restore_text,
                        attachments: composer.attachments.get(&restore_key).cloned().unwrap_or_default(),
                        appshots: composer.appshots.get(&restore_key).cloned().unwrap_or_default(),
                    }, cx) {
                        cx.emit(ComposerEvent::UnsentChanged);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    pub(crate) fn type_into(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.input.focus_handle(cx), cx);
        self.input.update(cx, |input, cx| {
            input.replace_text_in_range(None, text, window, cx);
        });
    }

    pub(crate) fn interrupt_selected(&mut self, cx: &mut Context<Self>) {
        let Some(chat_id) = self.bound_chat_id(cx) else {
            return;
        };
        self.interrupt_chat(chat_id, cx);
    }

    pub(crate) fn interrupt_chat(&mut self, chat_id: String, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if !begin_interrupt(&mut self.interrupting, &chat_id) {
            return;
        }
        let params = interrupt_params(&chat_id);
        let task_chat_id = chat_id.clone();
        let failure_chat = chat_id.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = engine.client().call(methods::QUEUE_COMMAND, params).await;
            if let Err(err) = result {
                this.update(cx, |composer, cx| {
                    composer.interrupting.remove(&task_chat_id);
                    composer.failure = Some(format!("Stop failed: {err}").into());
                    composer.failure_key = Some(failure_chat);
                    cx.notify();
                })
                .ok();
            }
        });
        self.interrupt_tasks.insert(chat_id, task);
    }

    pub(crate) fn is_interrupting(&self, chat_id: &str) -> bool {
        self.interrupting.contains(chat_id)
    }

    // ---- wizard glue ----

    fn host_input(&self, host: WizardHost) -> &Entity<ComposerInput> {
        match host {
            WizardHost::Own => &self.input,
            WizardHost::Ask => &self.ask_input,
        }
    }

    fn host_advance(&mut self, host: WizardHost, cx: &mut Context<Self>) {
        match host {
            WizardHost::Own => self.wizard_advance(cx),
            WizardHost::Ask => self.ask_advance(cx),
        }
    }

    fn wizard_select(&mut self, host: WizardHost, option_ix: usize, cx: &mut Context<Self>) {
        let Some(wizard) = self.host_wizard(host) else {
            return;
        };
        let step = wizard.select(option_ix);
        let has_pick = wizard.page_has_pick();
        self.host_input(host).update(cx, |input, cx| {
            input.set_placeholder(ask_pill::placeholder(has_pick), cx)
        });
        match step {
            WizardStep::AutoAdvance => self.schedule_auto_advance(host, cx),
            WizardStep::Done(answers) => match host {
                WizardHost::Own => self.wizard_finish(answers, cx),
                WizardHost::Ask => self.ask_finish(answers, cx),
            },
            WizardStep::Stay => {}
        }
        cx.notify();
    }

    fn schedule_auto_advance(&mut self, host: WizardHost, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(AUTO_ADVANCE_MS))
                .await;
            this.update(cx, |composer, cx| composer.host_advance(host, cx))
                .ok();
        });
        match host {
            WizardHost::Own => self.advance_task = Some(task),
            WizardHost::Ask => self.ask.advance_task = Some(task),
        }
    }

    fn wizard_advance(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).text().to_string();
        let Some(wizard) = self.wizard.as_mut() else {
            return;
        };
        wizard.set_typed(text);
        match wizard.advance() {
            WizardStep::Done(answers) => self.wizard_finish(answers, cx),
            _ => {
                let text = wizard.typed.get(wizard.page).cloned().unwrap_or_default();
                self.input.update(cx, |input, cx| input.set_text(text, cx));
                cx.notify();
            }
        }
    }

    fn wizard_back(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).text().to_string();
        if let Some(wizard) = self.wizard.as_mut() {
            wizard.set_typed(text);
            wizard.back();
            let text = wizard.typed.get(wizard.page).cloned().unwrap_or_default();
            self.input.update(cx, |input, cx| input.set_text(text, cx));
            cx.notify();
        }
    }

    /// Submit RespondInput and retire the panel.
    fn wizard_finish(&mut self, answers: Vec<UserInputAnswer>, cx: &mut Context<Self>) {
        let Some(wizard) = self.wizard.take() else {
            return;
        };
        self.advance_task = None;
        self.answered_requests.insert(wizard.request_id.clone());
        let draft = crate::draft_store::entry(&self.current_key, cx).prompt;
        let placeholder = self.idle_placeholder.clone();
        self.input.update(cx, |input, cx| {
            input.set_text(draft, cx);
            // The panel borrowed the composer input; hand back its identity.
            input.set_placeholder(placeholder, cx);
            input.set_key_context(MESSAGE_COMPOSER_CONTEXT, cx);
        });
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(chat_id) = self.bound_chat_id(cx) else {
            return;
        };
        let request_id = wizard.request_id.clone();
        let failure_chat = chat_id.clone();
        // `action_task`, NOT `send_task` — see `interrupt`.
        self.action_task = Some(cx.spawn(async move |this, cx| {
            let result = respond_input(&engine, &chat_id, &request_id, answers).await;
            if let Err(err) = result {
                this.update(cx, |composer, cx| {
                    composer.failure = Some(format!("Answer failed: {err}").into());
                    composer.failure_key = Some(failure_chat);
                    // The answer never left this device — put the panel back.
                    composer.answered_requests.remove(&request_id);
                    cx.notify();
                })
                .ok();
                return;
            }
            // Safety net against a dead-looking session: the command queued,
            // but the host may still REJECT it (e.g. the run's resolver is
            // gone). If the very same request is still the live pending input
            // once the host has had ample time to execute and the resolved
            // flag to sync back, the answer demonstrably didn't take —
            // un-hide the panel instead of leaving the question unanswerable.
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |composer, cx| {
                let transcript = composer.state.read(cx).transcript.clone();
                let still_pending = pending_input_request(&transcript)
                    .is_some_and(|(pending_id, _)| pending_id == request_id);
                if still_pending && composer.answered_requests.remove(&request_id) {
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    fn on_wizard_key(
        &mut self,
        host: WizardHost,
        event: &KeyDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        // Keys bubbling out of the free-text input must not double-handle:
        // digits select options only while the input is empty, and Enter is the
        // input's own Submit action when it has focus.
        let input_focused = self
            .host_input(host)
            .read(cx)
            .focus_handle
            .is_focused(window);
        let input_empty = self.host_input(host).read(cx).is_empty();
        let key = event.keystroke.key.as_str();
        // A BARE digit picks an option. With a modifier held the keystroke
        // belongs to an app shortcut — ⌘1..⌘9 jump to a sidebar row — and the
        // panel must not also consume it as a selection.
        if let Ok(digit) = key.parse::<usize>()
            && (1..=9).contains(&digit)
            && !event.keystroke.modifiers.modified()
        {
            if !input_focused || input_empty {
                self.wizard_select(host, digit - 1, cx);
                // Consumed as a selection: stop the platform from also
                // inserting the digit into the focused free-text input.
                cx.stop_propagation();
            }
        } else if key == "enter" {
            if !input_focused {
                self.host_advance(host, cx);
                cx.stop_propagation();
            }
        } else if key == "escape" {
            match host {
                WizardHost::Own => {
                    if wizard_escape_goes_back(key, input_focused, input_empty) {
                        self.wizard_back(cx);
                    }
                }
                WizardHost::Ask => self.close_ask_tray(cx),
            }
            cx.stop_propagation();
        }
    }

    // ---- render pieces ----

    /// The agent-asked-a-question panel (zeron question-panel.tsx), rendered in
    /// place of the composer: the same floating-pill chrome (`rounded-[26px]
    /// border-white/[0.08] bg-white/[0.03] shadow-xl`), uppercase header +
    /// "1/3" counter chip, option rows with number kbd chips, a free-text
    /// override over a hairline, and Back / Next-Submit footer.
    fn render_wizard(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::of(cx).clone();
        let Some(wizard) = self.wizard.clone() else {
            return gpui::Empty.into_any_element();
        };
        let input = self.input.clone();
        let Some((body, footer)) = self.wizard_body(&wizard, &input, WizardHost::Own, cx) else {
            return gpui::Empty.into_any_element();
        };

        // Stands in for the composer pill, so it is the same frosted surface:
        // without the backdrop blur the translucent fill let the transcript
        // show through unblurred.
        let panel = div()
            .id("question-panel")
            .track_focus(&self.wizard_focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_wizard_key(WizardHost::Own, event, window, cx)
            }))
            .occlude()
            .rounded(px(COMPOSER_RADIUS))
            .border_1()
            .border_color(theme.border)
            .bg(theme.composer_surface_bg())
            .when(!theme.is_frost(), |el| el.shadow_lg())
            .flex()
            .flex_col()
            .child(body)
            .child(footer);
        crate::frost::frosted(COMPOSER_RADIUS, crate::frost::MENU_BLUR, panel).into_any_element()
    }

    fn wizard_body(
        &self,
        wizard: &Wizard,
        input: &Entity<ComposerInput>,
        host: WizardHost,
        cx: &mut Context<Self>,
    ) -> Option<(gpui::Div, gpui::Div)> {
        let theme = Theme::of(cx).clone();
        let counter = wizard.counter();
        let question = wizard.current()?;
        let page = wizard.page;
        let last = page + 1 >= wizard.questions.len();
        let typed_empty = input.read(cx).is_empty();
        let can_advance = wizard.page_has_pick() || !typed_empty || question.multiline;
        let hover_prefix = host.hover_prefix();

        let options = question.options.iter().enumerate().map(|(ix, label)| {
            // Selection reads on the row only while no typed override exists
            // (typed answers win — zeron question-panel.tsx `isSel`).
            let picked = wizard.is_picked(ix) && typed_empty;
            div()
                .id(("wizard-option", ix))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .px(px(14.0))
                .py(px(10.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(if picked {
                    crate::theme::ink(0.16)
                } else {
                    gpui::transparent_black()
                })
                // zeron question-panel.tsx option rows: `transition-colors`.
                .bg(if picked {
                    crate::theme::ink(0.09)
                } else {
                    motion::hover_blend(
                        &format!("{hover_prefix}-{ix}"),
                        crate::theme::ink(0.025),
                        crate::theme::ink(0.06),
                    )
                })
                .on_hover(motion::hover_listener(format!("{hover_prefix}-{ix}")))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.wizard_select(host, ix, cx)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(crate::typography::ui_rems(13.5))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(if picked {
                            theme.text
                        } else {
                            theme.text.opacity(0.9)
                        })
                        .child(SharedString::from(label.clone())),
                )
                .when(ix < 9, |el| {
                    el.child(
                        // Number kbd chip: `size-[22px] rounded-md text-[11px]`.
                        div()
                            .flex_none()
                            .size(px(22.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .bg(if picked {
                                crate::theme::ink(0.16)
                            } else {
                                crate::theme::ink(0.05)
                            })
                            .text_size(crate::typography::ui_rems(11.0))
                            .text_color(if picked {
                                theme.text
                            } else {
                                theme.text_muted.opacity(0.6)
                            })
                            .child(SharedString::from(format!("{}", ix + 1))),
                    )
                })
        });

        let body = div()
            .px(px(16.0))
            .pt(px(16.0))
            .flex()
            .flex_col()
            // Header: tracked uppercase + counter chip when paged.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(10.5))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text_muted.opacity(0.6))
                            .child(SharedString::from(crate::popover::tracked_upper(
                                &question.header,
                            ))),
                    )
                    .when(wizard.questions.len() > 1, |el| {
                        el.child(
                            div()
                                .h(px(20.0))
                                .px(px(6.0))
                                .flex()
                                .items_center()
                                .rounded(px(6.0))
                                .bg(crate::theme::ink(0.06))
                                .text_size(crate::typography::ui_rems(10.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text_muted.opacity(0.6))
                                .child(SharedString::from(counter)),
                        )
                    }),
            )
            .child(
                div()
                    .mt(px(6.0))
                    .text_size(crate::typography::ui_rems(15.0))
                    .line_height(px(20.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(SharedString::from(question.question.clone())),
            )
            .when(question.multi_select, |el| {
                el.child(
                    div()
                        .mt(px(4.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted.opacity(0.65))
                        .child(SharedString::from("Select one or more options.")),
                )
            })
            .child(
                div()
                    .mt(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .children(options),
            )
            // Free-text override over a hairline.
            .child(
                div()
                    .mt(px(12.0))
                    .border_t_1()
                    .border_color(crate::theme::hairline(0.06))
                    .pt(px(12.0))
                    .pb(px(4.0))
                    .px(px(4.0))
                    .child(input.clone()),
            );
        let footer = div()
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .px(px(16.0))
            .pb(px(16.0))
            .pt(px(4.0))
            .child(if page > 0 {
                crate::popover::btn_ghost(&theme, "Back", "wizard-back")
                    .id("wizard-back")
                    .on_click(cx.listener(move |this, _, _, cx| match host {
                        WizardHost::Own => this.wizard_back(cx),
                        WizardHost::Ask => this.ask_back(cx),
                    }))
                    .into_any_element()
            } else {
                gpui::Empty.into_any_element()
            })
            .child(
                crate::popover::btn_primary(&theme, if last { "Submit" } else { "Next" })
                    .id("wizard-submit")
                    .px(px(16.0))
                    .when(!can_advance, |el| el.opacity(0.4))
                    .on_click(cx.listener(move |this, _, _, cx| this.host_advance(host, cx))),
            );
        Some((body, footer))
    }

    /// `refocus` returns focus to the editor afterwards. Enter/Space on the
    /// focused microphone keeps focus there so its key-up ends the hold.
    fn toggle_dictation(&mut self, refocus: bool, cx: &mut Context<Self>) {
        use crate::dictation::Phase;
        if !crate::dictation::enabled(cx) && !self.input.read(cx).dictation.phase.active() {
            return;
        }
        // A queue row still acquiring its edit lease is about to replace the
        // draft, which would discard the dictation.
        if self.wizard.is_some()
            || self.queue_edit_finishing
            || self.queue_edit_pending_id.is_some()
            || self.sending
            || self.input.read(cx).read_only
        {
            return;
        }
        self.input
            .update(cx, |input, cx| match input.dictation.phase {
                Phase::Requesting => {
                    input.cancel_dictation();
                    cx.emit(DictationInputEvent::Changed);
                    cx.notify();
                }
                Phase::Listening => {
                    input.finish_dictation(false, cx);
                }
                // The voice track's Cancel stops transcription.
                Phase::Finalizing => {}
                _ if input.marked_range.is_some() => {
                    input.dictation.phase = Phase::Unavailable(
                        "Finish composing the current character before starting dictation.".into(),
                    );
                    cx.emit(DictationInputEvent::Changed);
                }
                _ => {
                    if let Some(service) = crate::dictation::start(cx) {
                        input.begin_dictation(service, cx);
                    }
                }
            });
        self.focus_pending |= refocus;
        cx.notify();
    }

    /// Hold to talk: the microphone or the shortcut going down starts
    /// dictation. Pressing during a session started by assistive Click takes
    /// it over, so releasing finishes it.
    fn press_dictation(&mut self, source: HoldSource, cx: &mut Context<Self>) {
        use crate::dictation::Phase;
        let phase = self.input.read(cx).dictation.phase.clone();
        // Repeats from the same source are ignored. Another source takes the
        // session over, so a hold whose release went missing (focus moved
        // within the composer) can always be ended.
        if phase.active()
            && self
                .dictation_hold
                .as_ref()
                .is_some_and(|hold| hold.source == source)
        {
            return;
        }
        self.dictation_hold = None;
        match phase {
            Phase::Finalizing => {}
            Phase::Requesting | Phase::Listening => {
                self.dictation_hold = Some(DictationHold {
                    source,
                    started: None,
                });
            }
            _ => {
                self.toggle_dictation(source != HoldSource::Button, cx);
                if self.input.read(cx).dictation.phase.active() {
                    self.dictation_hold = Some(DictationHold {
                        source,
                        started: Some(Instant::now()),
                    });
                }
            }
        }
    }

    /// Letting go transcribes what was said. A tap explains hold to talk,
    /// and a release that only answered the permission prompt records nothing.
    fn release_dictation(&mut self, source: HoldSource, cx: &mut Context<Self>) {
        use crate::dictation::Phase;
        if self
            .dictation_hold
            .as_ref()
            .is_none_or(|hold| hold.source != source)
        {
            return;
        }
        let tapped = self
            .dictation_hold
            .take()
            .and_then(|hold| hold.started)
            .is_some_and(|started| started.elapsed() < DICTATION_TAP);
        self.input
            .update(cx, |input, cx| match input.dictation.phase {
                Phase::Requesting if crate::dictation::permission_pending() => {
                    input.cancel_dictation();
                    cx.emit(DictationInputEvent::Changed);
                    cx.notify();
                }
                Phase::Requesting | Phase::Listening if tapped => {
                    input.cancel_dictation();
                    input.dictation.phase = Phase::Tapped;
                    cx.emit(DictationInputEvent::Changed);
                    cx.notify();
                }
                Phase::Requesting | Phase::Listening => {
                    input.finish_dictation(false, cx);
                }
                _ => {}
            });
        // Only the pointer moved focus to the microphone. A key hold already
        // sits in the editor (or focus has left it on purpose), and Enter/Space
        // keeps the microphone focused.
        self.focus_pending |= source == HoldSource::Pointer;
        cx.notify();
    }

    /// Microphone at rest; the one Stop control while dictating. `t` morphs
    /// the mic glyph out and the accent glass plate, stop square or spinner
    /// in; the glow follows the live voice level.
    fn render_dictation_button(
        &self,
        t: f32,
        frame: Option<&VoiceFrame>,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        use crate::dictation::{Phase, glass, waveform::Mode};
        if !crate::dictation::enabled(cx) || self.wizard.is_some() {
            return None;
        }
        let theme = Theme::of(cx).clone();
        let phase = self.input.read(cx).dictation.phase.clone();
        let active = phase.active();
        let label = phase.action_label().to_owned();
        let live = frame.is_some_and(|f| f.mode == Mode::Live);
        let glow = frame.map_or(0.0, |f| f.level * f.level.sqrt());
        // Spinner while the microphone opens or the model transcribes; the
        // stop square once the voice is live (and while retracting from it).
        let busy = matches!(phase, Phase::Requesting | Phase::Finalizing)
            || (!active && frame.is_some_and(|f| f.mode != Mode::Live));
        let tooltip = label.clone();
        let composer = cx.entity().downgrade();
        // Per-composer key: the main and sidechat composers share this
        // render, and the hover fade store is keyed by string.
        let hover_key: SharedString = format!("composer-dictation-{}", cx.entity_id()).into();
        let centered = || {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
        };
        let button = div()
            .id("composer-dictation")
            .debug_selector(|| "composer-dictation".into())
            .relative()
            .size(px(28.0))
            .flex_none()
            .rounded_full()
            .role(Role::Button)
            .aria_label(label)
            .aria_toggled(if active {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .tab_index(0)
            .cursor_pointer()
            // At rest it is a sibling of the paperclip: the same eased ink
            // wash. Live, it is the accent plate and dims like Send.
            .on_hover(motion::hover_listener(hover_key.clone()))
            .when(t <= 0.0, |el| {
                el.bg(motion::hover_blend(
                    &hover_key,
                    gpui::transparent_black(),
                    crate::theme::ink(0.10),
                ))
            })
            .when(t > 0.0, |el| {
                glass::accent(el, &theme, t, glow).hover(|style| style.opacity(0.85))
            })
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .tooltip(crate::settings::widgets::text_tooltip(tooltip))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.press_dictation(HoldSource::Pointer, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.release_dictation(HoldSource::Pointer, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.release_dictation(HoldSource::Pointer, cx)),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    if !event.is_held {
                        this.press_dictation(HoldSource::Button, cx);
                    }
                }
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    this.release_dictation(HoldSource::Button, cx);
                }
            }))
            // Assistive technology cannot hold: Click starts, then finishes.
            .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
                composer
                    .update(cx, |this, cx| this.toggle_dictation(true, cx))
                    .ok();
            })
            .when(t < 1.0, |el| {
                let scale = 1.0 - 0.75 * t;
                el.child(
                    centered().child(
                        crate::icons::icon(crate::icons::MICROPHONE)
                            .size(px(18.0))
                            .text_color(theme.text_muted.opacity(1.0 - t))
                            .with_transformation(gpui::Transformation::scale(gpui::size(
                                scale, scale,
                            ))),
                    ),
                )
            })
            .when(t > 0.0 && busy, |el| {
                el.child(
                    centered()
                        .opacity(t)
                        .child(crate::loaders::mini_mono_spinner(
                            "dictation-progress",
                            2.0,
                            theme.on_accent,
                            cx.entity_id(),
                            cx,
                        )),
                )
            })
            .when(t > 0.0 && !busy, |el| {
                // Square grows from a quarter of its size, as the mic shrinks.
                let side = 9.0 * (0.25 + 0.75 * t) * if live { 1.0 + 0.08 * glow } else { 1.0 };
                el.child(
                    centered().child(
                        div()
                            .size(px(side))
                            .rounded(px(2.5))
                            .bg(theme.on_accent.opacity(t)),
                    ),
                )
            });
        Some(button.into_any_element())
    }

    fn dismiss_dictation(&mut self, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.cancel_dictation();
            cx.emit(DictationInputEvent::Changed);
            cx.notify();
        });
        self.focus_pending = true;
        cx.notify();
    }

    fn dictation_action_button(
        &self,
        active: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = Theme::of(cx);
        let action_label = if active {
            "Cancel dictation"
        } else {
            "Dismiss dictation message"
        };
        let composer = cx.entity().downgrade();
        div()
            .id("dictation-dismiss")
            .debug_selector(|| "dictation-dismiss".into())
            .role(Role::Button)
            .aria_label(action_label)
            .tab_index(0)
            .flex_none()
            .h(px(24.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .rounded_full()
            .text_color(theme.text_muted)
            .cursor_pointer()
            .hover(|s| s.bg(theme.surface_raised_hover).text_color(theme.text))
            .focus_visible(|s| s.border_1().border_color(theme.accent))
            .on_click(cx.listener(|this, _, _, cx| this.dismiss_dictation(cx)))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    this.dismiss_dictation(cx);
                }
            }))
            .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
                composer
                    .update(cx, |this, cx| this.dismiss_dictation(cx))
                    .ok();
            })
    }

    fn render_dictation_status(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        // Live dictation renders inside the composer; only outcomes that
        // need reading (no speech, errors) appear below it.
        let phase = self.input.read(cx).dictation.phase.clone();
        if phase.active() {
            return None;
        }
        let (title, detail) = phase.status()?;
        let (title, detail) = (title.to_owned(), detail.to_owned());
        let phase = &phase;
        let failed = matches!(
            phase,
            crate::dictation::Phase::Denied(_)
                | crate::dictation::Phase::Unavailable(_)
                | crate::dictation::Phase::Failed(_)
        );
        let theme = Theme::of(cx);
        Some(
            motion::fade_in(
                "dictation-message-enter",
                div()
                    .id("dictation-status")
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .text_size(px(12.0))
                    .line_height(px(18.0))
                    .child(
                        div()
                            .flex_none()
                            .mt(px(6.0))
                            .size(px(6.0))
                            .rounded_full()
                            .bg(if failed {
                                theme.warning
                            } else {
                                theme.text_faint
                            }),
                    )
                    .child(
                        div()
                            .id("dictation-live-status")
                            .role(Role::Status)
                            .aria_label(format!("{title}. {detail}"))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_wrap()
                            .gap_x(px(8.0))
                            .child(div().text_color(theme.text).child(title))
                            .child(div().min_w_0().text_color(theme.text_muted).child(detail)),
                    )
                    .child(
                        self.dictation_action_button(false, cx)
                            .mt(px(-3.0))
                            .child("Dismiss"),
                    ),
            )
            .into_any_element(),
        )
    }

    /// Advances the dictation morph. Returns its eased progress and the frame
    /// to draw: live while dictating, the last live frame while retracting.
    fn update_voice(&mut self, window: &mut Window, cx: &App) -> (f32, Option<VoiceFrame>) {
        use crate::dictation::{Phase, waveform::Mode};
        let now = Instant::now();
        let reduced = motion::reduced_motion(cx);
        let dictation = &self.input.read(cx).dictation;
        let active = dictation.phase.active();
        self.voice_tween
            .retarget(if active { 1.0 } else { 0.0 }, now, reduced);
        let t = self.voice_tween.value(now);
        if active {
            let meter = &dictation.meter;
            let mode = match dictation.phase {
                Phase::Listening => Mode::Live,
                Phase::Finalizing if meter.since_start(now).is_some() => Mode::Processing,
                _ => Mode::Waiting,
            };
            let label = dictation
                .phase
                .status()
                .map(|(title, detail)| format!("{title}. {detail}"))
                .unwrap_or_default();
            self.voice_last = Some(VoiceFrame {
                label,
                mode,
                bars: meter.bars(now, !reduced),
                level: if mode == Mode::Live {
                    meter.level(now)
                } else {
                    0.0
                },
                elapsed: meter.since_start(now).map(|_| meter.elapsed(now)),
            });
        } else if t <= 0.0 {
            self.voice_last = None;
        }
        if !reduced && (active || !self.voice_tween.settled(now)) {
            // Bars scroll, the glow follows the voice, and the morph advances.
            window.request_animation_frame();
        }
        (t, self.voice_last.clone())
    }

    /// The waveform track that unrolls from Stop across the action row. `t`
    /// is the morph progress; the box keeps its final geometry and occludes
    /// the controls fading out beneath it.
    fn render_voice_track(
        &self,
        t: f32,
        frame: &VoiceFrame,
        left: f32,
        right: f32,
        top: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        use crate::dictation::{glass, waveform};
        let theme = Theme::of(cx).clone();
        let animate = !motion::reduced_motion(cx);
        let active = self.input.read(cx).dictation.phase.active();
        let live = frame.mode == waveform::Mode::Live && active;
        let clock = frame.elapsed.map(|elapsed| {
            let limit = Duration::from_secs(zeron_voice::MAX_SECONDS as u64);
            div()
                .flex_none()
                .font_family(theme.font_mono.clone())
                .text_size(px(11.0))
                .opacity(t)
                .text_color(if live && elapsed + VOICE_LIMIT_WARNING >= limit {
                    theme.warning
                } else if live {
                    theme.text_muted
                } else {
                    theme.text_faint
                })
                .child(waveform::clock(elapsed))
        });
        let track = glass::light(
            div()
                .id("dictation-live-status")
                .role(Role::Status)
                .aria_label(frame.label.clone())
                .h_full()
                .w(gpui::relative(t.max(0.0)))
                .min_w(px(VOICE_TRACK_HEIGHT * t.min(1.0)))
                .rounded_full()
                .overflow_hidden()
                .flex()
                .items_center()
                .gap(px(10.0))
                .pl(px(12.0))
                .pr(px(12.0)),
            &theme,
            t,
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h(px(16.0))
                .flex()
                .child(waveform::waveform(
                    frame.bars.clone(),
                    frame.mode,
                    waveform::Paint {
                        ink: theme.text.opacity(0.8),
                        quiet: theme.text_faint.opacity(0.5),
                    },
                    waveform::Phase {
                        intro: t,
                        collapse: if active { 0.0 } else { 1.0 - t },
                    },
                    animate,
                )),
        )
        .children(clock);
        div()
            .id("dictation-track")
            .absolute()
            .left(px(left))
            .right(px(right))
            .top(px(top))
            .h(px(VOICE_TRACK_HEIGHT))
            .flex()
            .justify_end()
            .occlude()
            .child(track)
            .into_any_element()
    }

    fn render_send_button(
        &mut self,
        mode: SendButtonMode,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::of(cx);
        // Zeron composer-actions.tsx: a size-7 filled circle — up-arrow to
        // send/queue, a dark rounded square on the same light circle to stop.
        match mode {
            SendButtonMode::Stop => div()
                .id("composer-stop")
                .size(px(28.0))
                .flex_none()
                .rounded_full()
                .bg(theme.text)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.opacity(0.85))
                .on_click(cx.listener(|this, _, _, cx| this.interrupt_selected(cx)))
                .tooltip(crate::settings::widgets::text_tooltip("Stop"))
                .child(div().size(px(11.0)).rounded(px(3.0)).bg(theme.bg))
                .into_any_element(),
            SendButtonMode::Resume => div()
                .id("composer-resume")
                .size(px(28.0))
                .flex_none()
                .rounded_full()
                .bg(theme.text)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.opacity(0.85))
                .on_click(cx.listener(|this, _, _, cx| this.resume(cx)))
                .child(
                    crate::icons::icon(crate::icons::PLAY)
                        .size(px(12.0))
                        .ml(px(1.0))
                        .text_color(theme.bg),
                )
                .into_any_element(),
            SendButtonMode::Send | SendButtonMode::Queue => {
                // Share the submission guard with Enter, including pending
                // edits and the new-session runnable-agent check.
                let blocked = self.send_blocked(cx);
                div()
                    .id("composer-send")
                    .debug_selector(|| "composer-send".into())
                    .size(px(28.0))
                    .flex_none()
                    .rounded_full()
                    .bg(theme.text)
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(blocked, |el| el.opacity(0.35))
                    .when(!blocked, |el| {
                        el.cursor_pointer()
                            .hover(|s| s.opacity(0.85))
                            .on_click(cx.listener(|this, _, _, cx| this.on_submit(cx)))
                    })
                    .tooltip(crate::settings::widgets::text_tooltip(
                        if mode == SendButtonMode::Queue {
                            "Queue message"
                        } else {
                            "Send message"
                        },
                    ))
                    .child(
                        crate::icons::icon(crate::icons::ARROW_UP)
                            .size(px(14.0))
                            .text_color(theme.bg),
                    )
                    .into_any_element()
            }
        }
    }
}

/// The completion popups' floating rails run through
/// [`crate::popover::rail`]: the shared `popup_bar` state plus whichever
/// popup's rows are mounted — see [`Composer::active_popup_scroll`].
impl crate::popover::ScrollRailHost for Composer {
    fn rail_bar(&mut self) -> &mut crate::popover::MenuScrollbarState {
        &mut self.popup_bar
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.active_popup_scroll()
    }
}

/// Focus lands on the prompt input (window-level focus fallbacks — e.g. after
/// the focused terminal panel is hidden — route here).
impl Focusable for Composer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.dictation_blur.is_none() {
            // Stop, Send and keyboard navigation within the composer must not
            // discard capture. Only leaving the whole composer invalidates it.
            let focus = self.dictation_focus.clone();
            self.dictation_blur = Some(cx.on_focus_out(&focus, window, |this, _, _, cx| {
                this.input.update(cx, |input, cx| {
                    if !input.dictation.phase.active() {
                        return;
                    }
                    if input.dictation.phase == crate::dictation::Phase::Requesting
                        && crate::dictation::permission_pending()
                    {
                        return;
                    }
                    input.cancel_dictation();
                    cx.emit(DictationInputEvent::Changed);
                    cx.notify();
                });
            }));
        }
        if self.dictation_activation.is_none() {
            self.dictation_activation =
                Some(cx.observe_window_activation(window, |this, window, cx| {
                    if !window.is_window_active() {
                        this.input.update(cx, |input, cx| {
                            if !input.dictation.phase.active() {
                                return;
                            }
                            // The permission bridge checks the originating key
                            // window before starting any capture after a prompt.
                            if input.dictation.phase == crate::dictation::Phase::Requesting
                                && crate::dictation::permission_pending()
                            {
                                return;
                            }
                            input.cancel_dictation();
                            cx.notify();
                        });
                        cx.notify();
                    }
                }));
        }
        if self.focus_pending {
            self.focus_pending = false;
            let focus = self.input.focus_handle(cx);
            window.focus(&focus, cx);
        }
        let theme = Theme::of(cx).clone();
        if self
            .bound_chat_row(cx)
            .is_some_and(|chat| chat.sidechat_expired(chrono::Utc::now()))
        {
            let stop = self
                .run_live(cx)
                .then(|| self.render_send_button(SendButtonMode::Stop, cx));
            return div()
                .p(px(12.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(
                    div()
                        .id("expired-sidechat")
                        .debug_selector(|| "expired-sidechat".into())
                        .flex_1()
                        .child("Expired after 24 hours. Create a new sidechat to continue."),
                )
                .children(stop);
        }
        let wizard_active = self.wizard.is_some();
        if self.mention.token.is_some()
            && (wizard_active || !self.input.focus_handle(cx).is_focused(window))
        {
            self.reset_mention(None, cx);
        }
        if self.slash.token.is_some()
            && (wizard_active || !self.input.focus_handle(cx).is_focused(window))
        {
            self.reset_slash(None, cx);
        }
        let mode = self.button_mode(cx);
        // Shape the current draft before sizing the pill. Waiting for the child
        // layout leaves the parent using the previous edit's height.
        self.input.update(cx, |input, cx| {
            if input.needs_measure && input.last_width > 0.0 {
                let mut style = window.text_style();
                style.font_family = theme.font_sans.clone();
                style.font_size = crate::typography::ui_rems(INPUT_TEXT_SIZE).into();
                style.color = if input.content.is_empty() {
                    theme.text_faint
                } else {
                    theme.text
                };
                input.layout_text(px(input.last_width), &style, window, cx);
            }
        });
        let (text_width, has_newline, content_height, last_width, epoch) = {
            let input = self.input.read(cx);
            (
                input.measured_text_width(),
                input.has_newline(),
                input.measured_content_height(),
                input.last_width,
                input.layout_epoch,
            )
        };
        let now = Instant::now();
        // Only measurements taken *after* the last flip may drive the next one
        // (at most one flip per layout pass — a flip invalidates the widths).
        let measured_since_flip = epoch > self.flip_epoch && last_width > 0.0;
        if measured_since_flip {
            // A same-mode width change is an interactive window/pane resize:
            // defer collapse until sizes settle for RESIZE_SETTLE_MS. Expansion
            // remains live so compact controls never squeeze the input away.
            if self.last_seen_width > 0.0 && (last_width - self.last_seen_width).abs() > 0.5 {
                self.width_changed_at = Some(now);
            }
            self.last_seen_width = last_width;
            if self.expanded_mode {
                if self.expanded_anchor <= 0.0 {
                    self.expanded_anchor = last_width;
                }
            } else {
                // The compact pill's content box is the layout-stable capacity
                // both thresholds measure against.
                self.compact_capacity = last_width - 8.0;
            }
        }
        let resizing = self
            .width_changed_at
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(RESIZE_SETTLE_MS));
        if resizing && self.settle_task.is_none() {
            // Re-evaluate once the settle window has passed.
            self.settle_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(RESIZE_SETTLE_MS + 20))
                    .await;
                this.update(cx, |composer, cx| {
                    composer.settle_task = None;
                    cx.notify();
                })
                .ok();
            }));
        }
        // Layout-stable compact capacity: measured directly while compact;
        // while expanded, the learned value shifted by any container resize
        // (the expanded input width tracks the container 1:1).
        let capacity = if !self.expanded_mode {
            if last_width > 0.0 {
                last_width - 8.0
            } else {
                f32::MAX // before first measure default to compact
            }
        } else if self.compact_capacity > 0.0 {
            if self.expanded_anchor > 0.0 && last_width > 0.0 {
                self.compact_capacity + (last_width - self.expanded_anchor)
            } else {
                self.compact_capacity
            }
        } else {
            f32::MAX
        };
        let next = composer_flip(
            self.expanded_mode,
            text_width,
            capacity,
            has_newline,
            resizing,
        );
        let committed_flip = next != self.expanded_mode && measured_since_flip;
        if committed_flip {
            self.expanded_mode = next;
            self.flip_epoch = epoch;
            self.expanded_anchor = 0.0;
            // The mode change moves the input width; don't read that jump as
            // an interactive resize.
            self.last_seen_width = 0.0;
        }
        // New chats render expanded regardless of `expanded_mode` (see below),
        // so a mode flip there changes nothing visible — never morph it.
        let new_chat = self.bound_chat_id(cx).is_none();
        // Morph clock in ms; dividing by the measurement knob stretches the
        // timeline exactly like shell.rs eval_tween's scaled duration.
        let now_ms = self.morph_clock.elapsed().as_secs_f32() * 1000.0 / motion::speed_scale();
        let route_snap = self
            .route_snap_until
            .is_some_and(|until| Instant::now() < until);
        let dock_height_changed = std::mem::take(&mut self.dock_height_changed);
        self.flip_morph = if dock_height_changed
            || self.dock_frame.is_some_and(|frame| frame.active)
            || self.dock_reflow.active()
        {
            None
        } else {
            flip_morph_step(
                self.flip_morph,
                committed_flip && !new_chat,
                self.last_rendered_height,
                now_ms,
                motion::reduced_motion(cx),
                route_snap,
            )
        };
        let expanded = self.expanded_mode;

        let failure = self.visible_failure();
        // Composer honesty: when the target's delivery path is degraded, say
        // UP FRONT that a send will queue (a durable local write delivered on
        // reconnect) instead of letting the button imply instant delivery.
        let queue_notice: Option<(SharedString, bool)> = {
            use zeron_proto::ConnectivityState as S;
            let bound_chat = self.bound_chat_id(cx);
            let state = self.state.read(cx);
            let degraded = match bound_chat.as_deref() {
                Some(id) => state.chat_delivery_degraded(id),
                None => {
                    // New-chat canvas: judge by the picked target device.
                    let remote_target = state
                        .effective_device_id()
                        .is_some_and(|id| state.local_device_id.as_deref() != Some(id.as_str()));
                    remote_target
                        && (matches!(state.connectivity.state, S::Offline | S::Reconnecting)
                            || state
                                .effective_device_id()
                                .is_some_and(|id| !state.device_online(&id, chrono::Utc::now())))
                }
            };
            let offline = state.connectivity.state == S::Offline;
            degraded.then(|| {
                let text: SharedString = if offline {
                    "Offline — messages will send when you're back online.".into()
                } else {
                    "Messages will send once the connection recovers.".into()
                };
                (text, offline)
            })
        };
        let effort_hint = self.computer_effort_hint(cx);
        let resume_compaction = self.resume_compaction(cx);
        // The shell owns the width and its route animation. A route-dependent
        // cap here would cut a wide composer before its return glide finishes.
        let container = div()
            .w_full()
            .max_w(px(self.last_available_width.unwrap_or(COMPOSER_MAX_WIDTH)))
            .mx_auto()
            .flex()
            .flex_col()
            .gap(px(Theme::SPACE_SM))
            .px(px(Theme::SPACE_LG))
            .pb(px(Theme::SPACE_LG))
            .when_some(failure, |el, message| {
                // Amber with "Warning" for the offline-ish case (engine not
                // connected), red with "Error" for send/run failures. Click
                // dismisses.
                let offline = message.as_ref() == "Engine not connected";
                el.child(
                    notice_chip(
                        &theme,
                        offline,
                        if offline { "Warning" } else { "Error" },
                        message,
                        NoticeChipIcon::Plain,
                    )
                    .id("composer-failure")
                    .mx(px(4.0))
                    .mt(px(6.0))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.failure = None;
                        this.failure_key = None;
                        cx.notify();
                    })),
                )
            })
            .when(self.compacting(cx), |el| {
                // The compaction writes nothing until it lands, so this chip
                // is the only sign the chat is doing anything.
                el.child(
                    div()
                        .mx(px(8.0))
                        .mt(px(6.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(crate::loaders::mini_mono_spinner(
                            "composer-compacting",
                            2.0,
                            theme.text_muted,
                            cx.entity_id(),
                            cx,
                        ))
                        .child("Compacting context…"),
                )
            })
            .when_some(resume_compaction, |el, offer| {
                let compact = cx.entity().downgrade();
                let keep = compact.clone();
                el.child(
                    div()
                        .mx(px(4.0))
                        .child(crate::computer::cards::computer_card(
                            crate::computer::cards::CardTone::Info,
                            crate::icons::COLLAPSE_ARROWS,
                            "Resume with less context".into(),
                            vec![
                                format!(
                                    "{} tokens from {} ago \u{b7} cache expired",
                                    crate::pickers::compact_tokens(offer.tokens),
                                    offer.idle_label()
                                )
                                .into(),
                            ],
                            vec![
                                crate::computer::cards::CardAction {
                                    id: "composer-resume-compact".into(),
                                    label: "Compact".into(),
                                    primary: true,
                                    on_click: Box::new(move |_, _, cx| {
                                        compact
                                            .update(cx, |composer, cx| {
                                                composer.request_compaction(None, cx)
                                            })
                                            .ok();
                                    }),
                                },
                                crate::computer::cards::CardAction {
                                    id: "composer-resume-keep".into(),
                                    label: "Keep full history".into(),
                                    primary: false,
                                    on_click: Box::new(move |_, _, cx| {
                                        keep.update(cx, |composer, cx| {
                                            composer.keep_full_history(cx)
                                        })
                                        .ok();
                                    }),
                                },
                            ],
                            None,
                            &theme,
                        )),
                )
            })
            .when(effort_hint, |el| {
                let composer = cx.entity().downgrade();
                let dismiss = |cx: &mut App| {
                    crate::settings::update(crate::settings::SavePolicy::Debounced, cx, |s| {
                        s.computer_effort_hint_dismissed = true
                    });
                    cx.refresh_windows();
                };
                el.child(
                    div()
                        .mx(px(4.0))
                        .child(crate::computer::cards::computer_card(
                            crate::computer::cards::CardTone::Info,
                            crate::icons::CURSOR,
                            "Desktop actions are faster at Medium effort".into(),
                            Vec::new(),
                            vec![
                                crate::computer::cards::CardAction {
                                    id: "composer-computer-effort-apply".into(),
                                    label: "Use Medium".into(),
                                    primary: true,
                                    on_click: Box::new(move |_, _, cx| {
                                        composer
                                            .update(cx, |composer, cx| {
                                                composer.pickers.update(cx, |pickers, cx| {
                                                    pickers.apply_computer_effort_hint(cx)
                                                })
                                            })
                                            .ok();
                                        dismiss(cx);
                                    }),
                                },
                                crate::computer::cards::CardAction {
                                    id: "composer-computer-effort-dismiss".into(),
                                    label: "Dismiss tip".into(),
                                    primary: false,
                                    on_click: Box::new(move |_, _, cx| dismiss(cx)),
                                },
                            ],
                            None,
                            &theme,
                        )),
                )
            })
            .when_some(queue_notice, |el, (notice, offline)| {
                // Not a warning box (v0.2.12 feedback: the amber Notice read
                // as an error and flashed on every blip — pre-grace). One
                // quiet caption line, amber dot only for hard offline; it
                // clears itself the moment the path heals.
                let dot = if offline {
                    theme.warning
                } else {
                    theme.text_faint
                };
                el.child(crate::motion::fade_in(
                    "composer-queue-notice",
                    div()
                        .id("composer-queue-notice")
                        .mx(px(8.0))
                        .mt(px(6.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(11.0))
                        .line_height(px(14.0))
                        .text_color(theme.text_faint)
                        .child(div().size(px(5.0)).rounded_full().bg(dot))
                        .child(div().min_w_0().truncate().child(notice)),
                ))
            });

        // With Steer as the default, an agent that only takes input between
        // turns still queues; without this line the queue read as a dropped
        // steer (user report: "my steer didn't apply until grok finished").
        let steer_queues = mode == SendButtonMode::Queue
            && crate::settings::current(cx).follow_up_behavior == FollowUpBehavior::Steer
            && self.pickers.read(cx).resolved_steering_mode(cx)
                == Some(zeron_proto::SteeringMode::TurnBoundary);
        let container = container.when(steer_queues, |el| {
            el.child(
                div()
                    .mx(px(8.0))
                    .mt(px(6.0))
                    .text_size(px(11.0))
                    .line_height(px(14.0))
                    .text_color(theme.text_faint)
                    .child("This agent can't be steered mid-turn. Your message will be queued and sent when the current turn finishes."),
            )
        });

        if wizard_active {
            // The card returns before the pill sizing below, so the input
            // would keep the compact pill's one-line viewport and a wrapped
            // note would vanish under the scroll fades.
            let height = wizard_input_height(content_height);
            self.input.update(cx, |input, cx| {
                if input.viewport_height != Some(height)
                    || input.settled_viewport_height != Some(height)
                    || input.resizing
                    || input.overflow_top_padding != 0.0
                {
                    input.resizing = false;
                    input.overflow_top_padding = 0.0;
                    input.viewport_height = Some(height);
                    input.settled_viewport_height = Some(height);
                    cx.notify();
                }
            });
            let wizard = self.render_wizard(cx);
            return container
                .children(self.render_computer_strip(&theme, cx))
                .child(motion::fade_quick("composer-wizard", div().child(wizard)));
        }

        // What is waiting to be sent, stacked directly above the box it was
        // typed in — the queue is a property of this composer, not a panel
        // somewhere else.
        let show_queue_latest_shortcut = self.queue_shortcut_revealed
            && self.editing_queued.is_none()
            && !self.pickers.read(cx).is_open()
            && !composer_has_content(
                self.input.read(cx).text(),
                self.staged().len() + self.staged_appshots().len(),
                self.staged_chip_count(cx),
            );
        let container = container
            .when_some(self.render_corner(window, cx), |el, corner| {
                el.child(corner)
            })
            .children(self.render_computer_strip(&theme, cx));
        // The checklist tray stacks above the queue (or directly above the
        // composer), one step narrower than what follows it.
        let has_queue = !self.state.read(cx).queue.is_empty();
        let container = container.when_some(
            self.render_todo_panel(has_queue, window, cx),
            |el, panel| el.child(motion::fade_quick("composer-todo", div().child(panel))),
        );
        let container = container.when_some(
            self.render_queue_panel(show_queue_latest_shortcut, window, cx),
            |el, panel| {
                el.child(motion::fade_quick(
                    "composer-queue",
                    div()
                        .mx(px(QUEUE_SIDE_INSET))
                        // Cancel the column gap, then tuck the tray one pixel
                        // behind the composer painted after it.
                        .mb(px(-(Theme::SPACE_SM + QUEUE_COMPOSER_OVERLAP)))
                        .child(panel),
                ))
            },
        );
        // Escape backs out of a queue-row edit (the row keeps its old text).
        // Bound here rather than in the input: the input's own Escape belongs
        // to the mention/slash popups, which outrank this while they're open.
        let container = container.when(self.editing_queued.is_some(), |el| {
            el.on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape"
                    && this.mention.token.is_none()
                    && this.slash.token.is_none()
                    && this.cancel_queue_edit(cx)
                {
                    cx.stop_propagation();
                }
            }))
        });

        // Route coordination must not force an established thread into the
        // two-row layout. Short drafts keep the original skinny composer.
        let session_expanded = expanded;
        let expanded = expanded || new_chat;
        let dock_amount = self.dock_frame.map_or(0.0, |frame| frame.amount);
        let mut dock_layout = crate::composer_dock::DockLayout {
            hero_height: composer_total_height(content_height),
            thread_height: if session_expanded {
                (content_height + TEXTAREA_PAD_V).clamp(TEXTAREA_MIN - 16.0, TEXTAREA_MAX)
                    + ACTIONS_ROW_HEIGHT
                    + PILL_BORDER_V
            } else {
                COMPACT_TOTAL_HEIGHT
            },
            extra_height: 0.0,
            compact: !session_expanded,
        };

        // Committed-height morph: the layout below is already the NEW mode's;
        // only the pill's height (and the entrance fade/text glide driven by
        // `morph_t`) animates. Steady state renders exactly the target.
        // Staged attachments add the wrap strip's height to the pill in BOTH
        // modes (attachment-ui.tsx AttachmentStrip sits above the input row).
        let staged_count = self.tiled_attachments(cx).len();
        // The input width excludes the inline controls in compact mode.
        // Wrap against the pill's content width in both modes, accounting
        // for the outer container padding and the pill's 1px borders.
        let strip_width_hint =
            self.last_available_width.unwrap_or(COMPOSER_MAX_WIDTH) - 2.0 * Theme::SPACE_LG - 2.0;
        let appshot_count = self.staged_appshots().len();
        let strip_h = attachment_strip_height(staged_count, strip_width_hint);
        let comment_strip_h = comment_strip_height(self.staged_chip_count(cx));
        let base_height = if self.dock_frame.is_some() {
            dock_layout.height(dock_amount)
        } else if expanded {
            composer_total_height(content_height)
        } else {
            COMPACT_TOTAL_HEIGHT
        };
        let target_height =
            base_height + strip_h + appshot_strip_height(appshot_count) + comment_strip_h;
        dock_layout.extra_height = strip_h + appshot_strip_height(appshot_count) + comment_strip_h;
        let dock_owns_layout = self.dock_frame.is_some_and(|frame| frame.active)
            || dock_height_changed
            || self.dock_reflow.active();
        let (reflow_height, dock_compact_amount) =
            self.dock_frame.map_or((0.0, dock_amount), |frame| {
                self.dock_reflow
                    .sample(dock_layout, frame, motion::reduced_motion(cx), now)
            });
        let coordinated_route_morph = self
            .flip_morph
            .filter(|m| m.spec == motion::NEW_THREAD_TRANSITION && !m.done(now_ms));
        // The route state commits before its shared-element animation begins.
        // Reconstruct the departing chrome at t=0, then progressively trade
        // it for the destination chrome so neither route changes the outer
        // composer geometry in a single frame.
        let new_thread_chrome = self
            .dock_frame
            .map(|frame| frame.selectors())
            .unwrap_or_else(|| {
                coordinated_route_morph.map_or_else(
                    || if new_chat { 1.0 } else { 0.0 },
                    |morph| {
                        let progress = morph.progress(now_ms);
                        if new_chat { progress } else { 1.0 - progress }
                    },
                )
            });
        let (new_thread_chrome_opacity, session_chrome_opacity) = self.dock_frame.map_or_else(
            || route_chrome_opacities(new_thread_chrome),
            |frame| (frame.selectors(), frame.footer()),
        );
        self.height_morph = if dock_owns_layout {
            None
        } else if coordinated_route_morph.is_some() {
            coordinated_route_morph
        } else {
            flip_morph_step(
                self.height_morph,
                (target_height - self.last_target_height).abs() > 0.5,
                self.last_rendered_height,
                now_ms,
                motion::reduced_motion(cx),
                route_snap,
            )
        };
        self.last_target_height = target_height;
        let pill_height = self
            .height_morph
            .map_or(target_height, |m| m.height(target_height, now_ms))
            + reflow_height;
        if self.height_morph.is_some() || self.dock_reflow.active() {
            window.request_animation_frame();
        }
        let (_, morph_t, morphing) = match self.flip_morph {
            Some(m) if !m.done(now_ms) => {
                (m.height(target_height, now_ms), m.progress(now_ms), true)
            }
            _ => (target_height, 1.0, false),
        };
        if !morphing {
            self.flip_morph = None;
        } else {
            // Manual tween drive: keep frames coming (shell.rs motion_active).
            window.request_animation_frame();
        }
        self.last_rendered_height = pill_height;
        self.dock_clearance_correction = self.dock_frame.map_or(0.0, |frame| {
            dock_layout.height(if frame.docked { 1.0 } else { 0.0 }) - pill_height
        });
        // Route morphs use the dock's reversible clock; typing flips keep
        // their existing local clock once the composer reaches its dock.
        let layout_morph_t = if dock_owns_layout {
            if expanded {
                1.0 - dock_compact_amount
            } else {
                dock_compact_amount
            }
        } else {
            morph_t
        };
        let text_pt = morph_text_pad(layout_morph_t);
        let surface_radius = COMPOSER_RADIUS - 4.0 * dock_amount;
        let route_to_single_line =
            self.dock_frame.is_some_and(|frame| frame.active) && !session_expanded;
        let textarea_height = (pill_height
            - strip_h
            - appshot_strip_height(appshot_count)
            - comment_strip_h
            - PILL_BORDER_V
            - ACTIONS_ROW_HEIGHT)
            .max(if route_to_single_line {
                INPUT_LINE_HEIGHT + text_pt + 4.0
            } else {
                0.0
            });
        self.input.update(cx, |input, cx| {
            let height = if expanded {
                (textarea_height - text_pt - 4.0).max(0.0)
            } else {
                INPUT_LINE_HEIGHT
            };
            let settled_height = if expanded {
                (base_height - PILL_BORDER_V - ACTIONS_ROW_HEIGHT - TEXTAREA_PAD_V).max(
                    if route_to_single_line {
                        INPUT_LINE_HEIGHT
                    } else {
                        0.0
                    },
                )
            } else {
                INPUT_LINE_HEIGHT
            };
            let resizing = self.height_morph.is_some() || self.dock_reflow.active();
            let top_padding = if expanded { text_pt } else { 0.0 };
            if input.viewport_height != Some(height)
                || input.settled_viewport_height != Some(settled_height)
                || input.resizing != resizing
                || input.overflow_top_padding != top_padding
            {
                input.resizing = resizing;
                input.overflow_top_padding = top_padding;
                input.viewport_height = Some(height);
                input.settled_viewport_height = Some(settled_height);
                cx.notify();
            }
        });

        let send_button = self.render_send_button(mode, cx);
        let (voice_t, voice_frame) = self.update_voice(window, cx);
        let dictating = self.input.read(cx).dictation.phase.active();
        let microphone = self.render_dictation_button(voice_t, voice_frame.as_ref(), cx);
        let attach_action = cx.entity().downgrade();
        let attach_hover_key: SharedString = format!("composer-attach-{}", cx.entity_id()).into();
        // Attach button — opens the native image picker (the original's hidden
        // `<input type=file accept="image/*" multiple>`); paste/drop also feed
        // the same strip. The leading utility group owns the spacing between
        // this button and the model picker.
        let attach = div()
            .id("composer-attach")
            .size(px(28.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            // zeron composer-actions.tsx attach: `transition-colors`.
            .bg(motion::hover_blend(
                &attach_hover_key,
                gpui::transparent_black(),
                crate::theme::ink(0.10),
            ))
            .on_hover(motion::hover_listener(attach_hover_key))
            .relative()
            // While dictating, the attachment slot becomes Cancel: the
            // paperclip and the cross swap with a scale and fade. The action
            // follows the dictation phase, never the animation.
            .role(Role::Button)
            .aria_label(if dictating {
                "Cancel dictation"
            } else {
                "Attach images"
            })
            .tab_index(0)
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .when(dictating, |el| {
                el.debug_selector(|| "dictation-dismiss".into())
            })
            .tooltip(crate::settings::widgets::text_tooltip(if dictating {
                "Cancel dictation"
            } else {
                "Attach images"
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.input.read(cx).dictation.phase.active() {
                    this.dismiss_dictation(cx);
                } else {
                    this.open_file_picker(cx);
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    if this.input.read(cx).dictation.phase.active() {
                        this.dismiss_dictation(cx);
                    } else {
                        this.open_file_picker(cx);
                    }
                }
            }))
            .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
                attach_action
                    .update(cx, |this, cx| {
                        if this.input.read(cx).dictation.phase.active() {
                            this.dismiss_dictation(cx);
                        } else {
                            this.open_file_picker(cx);
                        }
                    })
                    .ok();
            })
            .when(voice_t < 1.0, |el| {
                let scale = 1.0 - 0.75 * voice_t;
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            crate::icons::icon(crate::icons::PAPERCLIP)
                                // Its painted bounds are centered in the 24px viewbox;
                                // a larger glyph balances the brand icon without moving
                                // it off-center inside the unchanged 28px hit target.
                                .size(px(18.0))
                                .text_color(theme.text_muted.opacity(1.0 - voice_t))
                                .with_transformation(gpui::Transformation::scale(gpui::size(
                                    scale, scale,
                                ))),
                        ),
                )
            })
            .when(voice_t > 0.0, |el| {
                let scale = 0.25 + 0.75 * voice_t;
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            crate::icons::icon(crate::icons::CLOSE)
                                .size(px(16.0))
                                .text_color(theme.text_muted.opacity(voice_t))
                                .with_transformation(gpui::Transformation::scale(gpui::size(
                                    scale, scale,
                                ))),
                        ),
                )
            });
        // Staged-thumbnail strip (attachment-ui.tsx AttachmentStrip), above
        // the input inside the pill in both modes.
        let strip = self.render_attachment_strip(&theme, cx);
        let appshot_strip = self.render_appshot_strip(&theme, window, cx);
        let comments_chip = self.render_staged_chips(&theme, cx);

        let pill_border = theme.composer_surface_border();
        // Compensate for the transcript canvas beneath the frosted surface.
        // Keep the opaque fallback when frost is disabled or unsupported.
        let pill = div()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    // Padding and action controls are part of the text composer.
                    // Open menus keep their own keyboard/search focus.
                    if !this.pickers.read(cx).is_open() && !this.running_tab {
                        window.focus(&this.input.focus_handle(cx), cx);
                        window.prevent_default();
                    }
                }),
            )
            .rounded(px(surface_radius))
            .border_1()
            .border_color(pill_border)
            .bg(theme.composer_surface_bg())
            .when(!theme.is_frost(), |el| el.shadow_lg());
        // The pill's bottom edge is stationary on screen (the composer sits at
        // the bottom of the shell column; growth moves the TOP edge), so the
        // controls pin to the bottom and only the text glides with the reveal
        // while the model selector stays in the right action group.
        let running_reveal = if self.dock_frame.is_some() {
            self.running_reveal(window, cx)
        } else {
            0.0
        };
        let prompt_opacity = 1.0 - running_reveal;
        let cluster_dy = morph_cluster_dy(layout_morph_t);
        let action_inset = morph_cluster_inset(expanded, layout_morph_t);
        let surface_width = self.last_available_width.map_or_else(
            || {
                self.surface_bounds
                    .get()
                    .map_or(strip_width_hint + PILL_BORDER_V, |bounds| {
                        f32::from(bounds.size.width)
                    })
            },
            |width| (width - 2.0 * Theme::SPACE_LG).max(0.0),
        );
        let model_picker = div()
            .min_w_0()
            .max_w(px(surface_width * 0.45))
            .relative()
            .child(self.pickers.clone());
        #[cfg(test)]
        let model_picker = {
            let measured_model_bounds = self.model_bounds.clone();
            model_picker.child(
                gpui::canvas(
                    move |bounds, _, _| measured_model_bounds.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
        };
        // The model chip already contributes 6px of trailing padding. Remove
        // that from its gap to the narrow microphone glyph so the visible
        // label–mic and mic–Send spaces balance without shrinking hit targets.
        let model_action_gap = if microphone.is_some() {
            ACTION_PRIMARY_GAP - 6.0
        } else {
            ACTION_PRIMARY_GAP
        };
        // The voice track spans from the attachment slot to Stop, over the
        // model selector (and, compact, the text), which fade beneath it.
        let voice_track = voice_frame
            .as_ref()
            .filter(|_| voice_t > 0.0 && microphone.is_some())
            .map(|frame| {
                // Expanded: the row's 2px top pad; compact: centred in the
                // 47px line, riding the same cluster glide as the buttons.
                let top = if expanded {
                    2.0
                } else {
                    (COMPACT_TOTAL_HEIGHT - PILL_BORDER_V - VOICE_TRACK_HEIGHT) / 2.0 - cluster_dy
                };
                self.render_voice_track(
                    voice_t,
                    frame,
                    action_inset + 28.0 + VOICE_TRACK_GAP,
                    action_inset + 28.0 + ACTION_PRIMARY_GAP + 28.0 + VOICE_TRACK_GAP,
                    top,
                    cx,
                )
            });
        let beneath_voice = 1.0 - voice_t.clamp(0.0, 1.0);
        let model_picker = model_picker.opacity(beneath_voice);
        let body = if expanded {
            // Expanded: textarea on top (`px-4 pb-1 pt-4`), actions row
            // (8px bottom + 2px top, 32px chips → 42px) ABSOLUTE at the pill's
            // stationary bottom — constant screen-y through the morph, with
            // the compact↔expanded centering delta gliding out. The
            // text viewport follows the animated height so it cannot paint
            // over the controls. Its width stays fixed (no tween rewraps);
            // top padding eases 12→16. Attachment and Send stay on the bottom
            // anchor with the model selector always beside Send.
            pill.h(px(pill_height))
                .overflow_hidden()
                .relative()
                .flex()
                .flex_col()
                .children(comments_chip)
                .children(appshot_strip)
                .children(strip)
                .child(
                    div()
                        .h(px(textarea_height))
                        .flex_none()
                        .overflow_hidden()
                        .px(px(16.0))
                        .pt(px(text_pt))
                        .pb(px(4.0))
                        .opacity(prompt_opacity)
                        .child(self.render_input_with_completion()),
                )
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .opacity(prompt_opacity)
                        .bottom(px(-cluster_dy))
                        .h(px(ACTIONS_ROW_HEIGHT))
                        .flex()
                        .flex_row()
                        .items_center()
                        // Attachment stays left; model, microphone and Send stay right.
                        .gap(px(model_action_gap))
                        .px(px(action_inset))
                        .pt(px(2.0))
                        .pb(px(ACTIONS_BOTTOM_PAD))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(ACTION_UTILITY_GAP))
                                .child(attach),
                        )
                        .child(model_picker)
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(px(ACTION_PRIMARY_GAP))
                                .children(microphone)
                                .child(send_button),
                        )
                        .children(voice_track),
                )
                .when(running_reveal > 0.0, |el| {
                    el.child(self.render_running_list(&theme, running_reveal, cx))
                })
        } else {
            // Compact pill: attachment on the left, input in the middle,
            // then model, microphone and Send on the right, all on one 47px line.
            // The row is BOTTOM-justified: during the collapse morph the pill
            // top sweeps down over a stationary row, the text walks down from
            // its expanded resting place via a decaying relative offset, and
            // attachment/Send hold their spots (the centering delta gliding
            // in). The model selector remains beside microphone and Send.
            let text_glide = if dock_owns_layout {
                collapse_text_glide(dock_layout.hero_height, dock_compact_amount)
            } else {
                match self.flip_morph {
                    Some(m) if morphing => collapse_text_glide(m.from, morph_t),
                    _ => 0.0,
                }
            };
            pill.h(px(pill_height))
                .overflow_hidden()
                .flex()
                .flex_col()
                .justify_end()
                .children(comments_chip)
                .children(appshot_strip)
                .children(strip)
                .child(
                    div()
                        .h(px(COMPACT_TOTAL_HEIGHT - PILL_BORDER_V))
                        .relative()
                        .flex()
                        .flex_row()
                        .items_center()
                        .child(
                            div()
                                .flex_none()
                                .pl(px(action_inset))
                                .relative()
                                .top(px(-cluster_dy))
                                .flex()
                                .items_center()
                                .gap(px(ACTION_UTILITY_GAP))
                                .child(attach),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .px(px(8.0))
                                .relative()
                                .top(px(-text_glide))
                                // The draft waits under the voice track and
                                // fades back as the transcript lands in it.
                                .opacity(beneath_voice)
                                .child(self.render_input_with_completion()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .max_w(px(surface_width * 0.45))
                                .relative()
                                .top(px(-cluster_dy))
                                .child(model_picker),
                        )
                        .child(
                            div()
                                .flex_none()
                                .pl(px(model_action_gap))
                                .pr(px(action_inset))
                                .relative()
                                .top(px(-cluster_dy))
                                .flex()
                                .items_center()
                                .gap(px(ACTION_PRIMARY_GAP))
                                .children(microphone)
                                .child(send_button),
                        )
                        .children(voice_track),
                )
        };
        let new_thread_target_selectors = (new_thread_chrome_opacity > 0.0).then(|| {
            self.pickers.update(cx, |pickers, cx| {
                pickers.render_new_thread_target_selectors(cx)
            })
        });
        let new_thread_git_selectors = (new_thread_chrome_opacity > 0.0)
            .then(|| {
                self.pickers.update(cx, |pickers, cx| {
                    pickers.render_new_thread_git_selectors(cx)
                })
            })
            .flatten();
        let has_new_thread_git_selectors = self
            .state
            .read(cx)
            .selected_space_row()
            .is_some_and(|space| space.git_detected);
        // The file dropzone lives in the shell (the whole conversation column,
        // not just the pill — shell.rs `chat-dropzone`); drops land back here
        // via `add_paths`.
        // Frosted: the pill backdrop-blurs the transcript scrolling under it
        // (the popover glass treatment; radius matches the pill's rounding).
        // The shell keeps this entity under one parent on both routes. The
        // surface itself never fades, and frost follows the same morph radius.
        // A session dragged from the sidebar or the tab strip drops here as
        // a thread mention; the ring only shows while such a drag hovers the
        // pill (typed `drag_over`, so file drags and pane resizes never light
        // it). The shell's split halves stop above the pill so this listener
        // is the one that receives the drop.
        let drop_ring = {
            let accent = theme.accent;
            div()
                .absolute()
                .inset_0()
                .rounded(px(surface_radius))
                .border_1()
                .border_dashed()
                .border_color(accent)
                .opacity(0.0)
                .drag_over::<crate::shell::SidebarSessionDrag>(|style, _, _, _| style.opacity(1.0))
                .drag_over::<crate::shell::TabDragPayload>(|style, _, _, _| style.opacity(1.0))
        };
        let pill_surface = div()
            .relative()
            .id("composer-surface")
            .debug_selector(|| "composer-surface".to_string())
            .on_drop::<crate::shell::SidebarSessionDrag>(cx.listener(
                |this, payload: &crate::shell::SidebarSessionDrag, window, cx| {
                    this.add_thread_mention(&payload.chat_id, window, cx);
                },
            ))
            .on_drop::<crate::shell::TabDragPayload>(cx.listener(
                |this, payload: &crate::shell::TabDragPayload, window, cx| {
                    this.add_thread_mention(&payload.chat_id, window, cx);
                },
            ))
            .child(crate::frost::frosted(surface_radius, 16.0, body))
            .child({
                let measured = self.surface_bounds.clone();
                // All prepaint completes before any paint. The background
                // reads this cell during paint, never last frame's geometry.
                gpui::canvas(
                    move |bounds, _, _| measured.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0()
            })
            .child(drop_ring)
            // Both completion popups span the full pill width above it —
            // the file-mention and slash tokens are mutually exclusive.
            .children(self.render_file_mention_popup(&theme, cx))
            .children(self.render_slash_popup(&theme, cx));
        // Restore the original chip-only selector treatment: destination at
        // the top-right, no surrounding surface. Cancel the column gap as the
        // row collapses so the pill never jumps at the route boundary.
        let container = if self.dock_frame.is_some() {
            let running =
                crate::home::running_agents(self.state.read(cx), chrono::Utc::now()).len();
            let tabs = (new_thread_chrome_opacity > 0.0)
                .then(|| self.render_home_tabs(&theme, running, running_reveal, cx));
            // Floating selectors share the surface's origin and never change its height.
            container.relative().child(
                div()
                    .id("dock-target-selectors")
                    .absolute()
                    .top(px(-28.0))
                    .left(px(Theme::SPACE_LG + 4.0))
                    .right(px(Theme::SPACE_LG + 10.0))
                    .h(px(NEW_THREAD_SELECTOR_ROW_HEIGHT))
                    .flex()
                    .items_start()
                    .justify_between()
                    .opacity(new_thread_chrome_opacity)
                    .children(tabs)
                    .child(div().flex().children(new_thread_target_selectors)),
            )
        } else if new_thread_chrome > 0.0 {
            container.child(
                div()
                    .w_full()
                    .h(px(NEW_THREAD_SELECTOR_ROW_HEIGHT * new_thread_chrome))
                    .mb(px(-Theme::SPACE_SM * (1.0 - new_thread_chrome)))
                    .px(px(10.0))
                    .flex()
                    .items_start()
                    .justify_end()
                    .opacity(new_thread_chrome_opacity)
                    .children(new_thread_target_selectors),
            )
        } else {
            container
        };
        let dictation_status = self.render_dictation_status(cx);
        let container = container.child(
            div()
                .track_focus(&self.dictation_focus)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape"
                        && this.input.read(cx).dictation.phase.active()
                    {
                        this.dismiss_dictation(cx);
                        cx.stop_propagation();
                    }
                }))
                .flex()
                .flex_col()
                .gap(px(Theme::SPACE_SM))
                .child(pill_surface)
                .children(dictation_status),
        );

        // The lower slot keeps a stable footprint for Git projects while its
        // old floating checkout/ref controls dissolve into the session footer.
        // Non-Git sessions grow the slot continuously from zero.
        let session_chrome = 1.0 - new_thread_chrome;
        let bottom_slot = if has_new_thread_git_selectors || self.dock_frame.is_some() {
            1.0
        } else {
            session_chrome
        };
        let container = if bottom_slot > 0.0 {
            // The pickers carry the composer's chat binding, so a bound
            // composer's footer reads and writes ITS chat (split partner,
            // sidechat), never the selection's.
            let footer = (session_chrome_opacity > 0.0 && !self.side_chat).then(|| {
                self.pickers
                    .update(cx, |pickers, cx| pickers.render_footer(cx))
            });
            let sidechat_pill = self.render_sidechat_pill(&theme, cx);
            let permission_chip = (session_chrome_opacity > 0.0 && !self.pr_chat).then(|| {
                self.pickers
                    .update(cx, |pickers, cx| pickers.render_permission_chip(cx))
            });
            if session_chrome_opacity > 0.0 {
                let harness = (!self.side_chat)
                    .then(|| self.pickers.read(cx).resolved(cx).harness)
                    .flatten();
                let target = self
                    .bound_chat_row(cx)
                    .map(|chat| chat.device_id.clone())
                    .filter(|device| self.state.read(cx).local_device_id.as_ref() != Some(device));
                let chat = self.bound_chat_id(cx);
                self.account_usage
                    .update(cx, |usage, cx| usage.track(harness, target, chat, cx));
            }
            container.child(
                div()
                    .w_full()
                    .h(px(SESSION_FOOTER_HEIGHT * bottom_slot))
                    .mt(px(-Theme::SPACE_SM * (1.0 - bottom_slot)))
                    .mb(px(-Theme::SPACE_SM * bottom_slot))
                    .relative()
                    .when(new_thread_chrome_opacity > 0.0, |slot| {
                        slot.child(
                            div()
                                .absolute()
                                .inset_0()
                                .px(px(10.0))
                                .flex()
                                .items_center()
                                .opacity(new_thread_chrome_opacity)
                                .children(new_thread_git_selectors),
                        )
                    })
                    .when(session_chrome_opacity > 0.0, |slot| {
                        slot.child(
                            div()
                                .absolute()
                                .inset_0()
                                .w_full()
                                .h(px(SESSION_FOOTER_HEIGHT))
                                .flex()
                                .items_center()
                                .opacity(session_chrome_opacity)
                                .child(div().flex_1().min_w_0().children(footer.flatten()))
                                .child(
                                    // The footer row's own 4px gap: the PR badge
                                    // ends flush with the row, so the rings keep
                                    // their distance here.
                                    div()
                                        .flex_none()
                                        .pl(px(4.0))
                                        .pr(px(10.0))
                                        .flex()
                                        .items_center()
                                        .gap(px(4.0))
                                        .children(sidechat_pill)
                                        .children(permission_chip)
                                        .child(self.account_usage.clone()),
                                ),
                        )
                    }),
            )
        } else {
            container
        };
        // Full-size preview of a staged thumbnail (AttachmentPreviewDialog).
        if let Some(preview) = self.preview.clone() {
            if std::mem::take(&mut self.preview_focus_pending) {
                window.focus(&self.preview_focus, cx);
            }
            let weak = cx.weak_entity();
            return container.child(attachments::lightbox(
                window,
                &preview,
                &self.preview_focus,
                move |window, cx| {
                    // Hand focus back to the input so typing (and the next
                    // Escape) lands where it did before the lightbox opened.
                    if let Ok(input_focus) = weak.update(cx, |this, cx| {
                        this.preview = None;
                        cx.notify();
                        this.input.read(cx).focus_handle.clone()
                    }) {
                        window.focus(&input_focus, cx);
                    }
                },
                cx,
            ));
        }
        container
    }
}

#[cfg(test)]
mod modal_selection_tests;

#[cfg(test)]
mod attachment_chip_tests;

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn composer_focus_window(
        cx: &mut gpui::TestAppContext,
    ) -> (tempfile::TempDir, gpui::WindowHandle<Composer>) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Composer::new(state, cx)
        });
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        (dir, window)
    }

    fn with_composer_input(
        cx: &mut gpui::TestAppContext,
        test: impl FnOnce(&mut ComposerInput, &mut Window, &mut Context<ComposerInput>),
    ) {
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, window, cx| {
                composer
                    .input
                    .update(cx, |input, cx| test(input, window, cx));
            })
            .unwrap();
    }

    #[gpui::test]
    fn shift_backspace_deletes_text_and_selection_in_all_input_contexts(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            init(cx, ComposerSendBehavior::default());
        });
        for context in [
            GENERIC_COMPOSER_CONTEXT,
            MESSAGE_COMPOSER_CONTEXT,
            PALETTE_SEARCH_CONTEXT,
        ] {
            let handle = cx.add_window(|window, cx| {
                let mut input = ComposerInput::with_context("", context, cx);
                input.set_text("café", cx);
                window.focus(&input.focus_handle, cx);
                input
            });
            cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
                .unwrap();
            cx.simulate_keystrokes(handle.into(), "shift-backspace");
            handle
                .update(cx, |input, _, cx| {
                    assert_eq!(input.text(), "caf", "{context}");
                    input.set_text("hello world", cx);
                    input.move_to(6, cx);
                    input.extend_selection(11, cx);
                })
                .unwrap();
            cx.simulate_keystrokes(handle.into(), "shift-backspace");
            handle
                .update(cx, |input, _, cx| {
                    assert_eq!(input.text(), "hello ", "{context}");
                    input.set_text("", cx);
                })
                .unwrap();
            cx.simulate_keystrokes(handle.into(), "shift-backspace");
            assert_eq!(
                handle
                    .read_with(cx, |input, _| input.text().to_owned())
                    .unwrap(),
                ""
            );
        }
    }

    #[gpui::test]
    fn dock_morph_restores_skinny_height_with_a_continuous_editor_origin(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_dir, handle) = composer_focus_window(cx);
        let input = handle
            .read_with(cx, |composer, _| composer.input.clone())
            .unwrap();
        // The trigger stays beside Send whichever picker the chip opens.
        for compact_picker in [false, true] {
            cx.update(|cx| {
                crate::settings::update(crate::settings::SavePolicy::Debounced, cx, |settings| {
                    settings.compact_model_picker = compact_picker;
                });
            });
            for thread_width in [436.0, 592.0, 768.0, 1232.0] {
                for docked in [true, false] {
                    let amounts = if docked {
                        [0.0, 0.2, 0.6, 0.98, 1.0]
                    } else {
                        [1.0, 0.98, 0.6, 0.2, 0.0]
                    };
                    for amount in amounts {
                        let outer_width = motion::lerp(COMPOSER_MAX_WIDTH, thread_width, amount);
                        cx.update(|cx| {
                        handle
                            .update(cx, |composer, window, cx| {
                                window.resize(size(px(outer_width), px(800.0)));
                                composer.set_available_width(outer_width, cx);
                                composer.state.update(cx, |state, _| {
                                    state.selected_chat = docked.then(|| "chat".into());
                                });
                                composer.on_state_changed(cx);
                                composer
                                    .input
                                    .update(cx, |input, cx| input.set_text("Hi", cx));
                                composer.expanded_mode = false;
                                let mut frame = crate::composer_dock::DockFrame::settled(docked);
                                frame.amount = amount;
                                frame.active = amount != if docked { 1.0 } else { 0.0 };
                                composer.set_dock_frame(frame, cx);
                            })
                            .unwrap();
                        cx.update_window(handle.into(), |_, window, cx| {
                            window.refresh();
                            window.draw(cx).clear();
                        })
                        .unwrap();
                        // Inspect the first painted frame before TestAppContext
                        // flushes effects and automatically draws dirty views.
                        handle.read_with(cx, |composer, cx| {
                        assert_eq!(composer.input, input);
                        let surface = composer.surface_bounds.get().unwrap();
                        assert!((f32::from(surface.size.width) - (outer_width - 2.0 * Theme::SPACE_LG)).abs() <= 1.0,
                            "surface width clipped: docked={docked}, amount={amount}, outer={outer_width}, surface={surface:?}");
                        let origin = input.read(cx).last_bounds.unwrap().origin;
                        assert!((f32::from(origin.y - surface.top()) - (17.0 - 4.0 * amount)).abs() <= 1.0,
                            "editor jumped: docked={docked}, amount={amount}, origin={origin:?}, surface={surface:?}");
                        let model = composer.model_bounds.get().unwrap();
                        let inset = motion::lerp(12.0, 8.0, amount);
                        let expected_right = surface.right() - px(1.0 + inset + 28.0 + ACTION_PRIMARY_GAP);
                        assert!((f32::from(model.right() - expected_right)).abs() <= 1.0,
                            "model must stay right aligned: compact_picker={compact_picker}, docked={docked}, amount={amount}, actual={model:?}, expected={expected_right:?}");
                        let expected = if docked { COMPACT_TOTAL_HEIGHT } else { COMPOSER_MIN_HEIGHT };
                        assert!((composer.last_rendered_height + composer.dock_clearance_correction - expected).abs() < 0.1);
                        assert!((composer.last_rendered_height - motion::lerp(COMPOSER_MIN_HEIGHT, COMPACT_TOTAL_HEIGHT, amount)).abs() < 0.1);
                            }).unwrap();
                        });
                    }
                }
            }
        }
    }

    #[gpui::test]
    fn dock_reflow_paints_attachment_wrap_without_a_height_jump(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD,
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aF1cAAAAASUVORK5CYII=").unwrap();
        handle
            .update(cx, |composer, _, cx| {
                composer
                    .state
                    .update(cx, |state, _| state.selected_chat = Some("chat".into()));
                composer.on_state_changed(cx);
                composer.attachments.insert(
                    composer.current_key.clone(),
                    (0..10)
                        .map(|i| {
                            attachments::stage_png_bytes(format!("image-{i}.png"), png.clone())
                        })
                        .collect(),
                );
                composer.set_available_width(698.0, cx);
                composer.set_dock_frame(crate::composer_dock::DockFrame::settled(true), cx);
                composer
                    .input
                    .update(cx, |input, cx| input.set_text("Hi", cx));
            })
            .unwrap();
        let mut frame = crate::composer_dock::DockFrame::settled(true);
        frame.active = true;
        frame.amount = 0.4;
        handle
            .update(cx, |composer, _, cx| composer.set_dock_frame(frame, cx))
            .unwrap();
        let before = handle
            .read_with(cx, |composer, _| composer.last_rendered_height)
            .unwrap();
        // Both widths are within the same route frame: only the thumbnail row
        // changes. Inspect before GPUI's automatic follow-up paints can hide it.
        cx.update(|cx| {
            handle
                .update(cx, |composer, _, cx| {
                    composer.set_available_width(697.0, cx)
                })
                .unwrap();
            cx.update_window(handle.into(), |_, window, cx| {
                window.refresh();
                window.draw(cx).clear();
            })
            .unwrap();
            handle
                .read_with(cx, |composer, cx| {
                    assert_eq!(composer.staged().len(), 10);
                    assert_eq!(attachment_strip_height(10, 698.0 - 34.0), 68.0);
                    assert_eq!(attachment_strip_height(10, 697.0 - 34.0), 132.0);
                    let surface = composer.surface_bounds.get().unwrap();
                    assert!(
                        (f32::from(surface.size.height) - before).abs() <= 1.0,
                        "attachment reflow jumped from {before} to {:?}",
                        surface.size.height
                    );
                    assert!(composer.dock_reflow.active());
                    assert!(composer.height_morph.is_none());
                    assert!(
                        (composer.last_rendered_height + composer.dock_clearance_correction
                            - (COMPACT_TOTAL_HEIGHT + 132.0))
                            .abs()
                            < 0.1,
                        "transcript clearance must reserve the destination height"
                    );
                    assert_eq!(composer.input.read(cx).text(), "Hi");
                })
                .unwrap();
        });
    }

    #[gpui::test]
    fn dock_reflow_keeps_text_growth_and_model_controls_continuous(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, _, cx| {
                composer
                    .state
                    .update(cx, |state, _| state.selected_chat = Some("chat".into()));
                composer.on_state_changed(cx);
                composer.set_available_width(592.0, cx);
                composer.set_dock_frame(crate::composer_dock::DockFrame::settled(true), cx);
                composer
                    .input
                    .update(cx, |input, cx| input.set_text("Hi", cx));
            })
            .unwrap();
        let mut frame = crate::composer_dock::DockFrame::settled(true);
        frame.active = true;
        frame.amount = 0.7;
        handle
            .update(cx, |composer, _, cx| composer.set_dock_frame(frame, cx))
            .unwrap();
        let (before, model_before) = handle
            .read_with(cx, |composer, _| {
                (
                    composer.last_rendered_height,
                    composer.model_bounds.get().unwrap().left(),
                )
            })
            .unwrap();
        cx.update(|cx| {
            handle
                .update(cx, |composer, _, cx| {
                    composer
                        .input
                        .update(cx, |input, cx| input.set_text("One\nTwo\nThree\nFour", cx));
                })
                .unwrap();
            cx.update_window(handle.into(), |_, window, cx| {
                window.refresh();
                window.draw(cx).clear();
            })
            .unwrap();
            handle
                .read_with(cx, |composer, _| {
                    assert!(composer.expanded_mode);
                    assert!(composer.dock_reflow.active());
                    assert!(composer.flip_morph.is_none());
                    assert!(
                        (composer.last_rendered_height - before).abs() <= 1.0,
                        "text growth jumped from {before} to {}",
                        composer.last_rendered_height
                    );
                    assert!(
                        (f32::from(composer.model_bounds.get().unwrap().left() - model_before))
                            .abs()
                            <= 1.0,
                        "model selector jumped on the compact/expanded commit"
                    );
                })
                .unwrap();
        });
    }

    #[gpui::test]
    fn conversation_resize_reflows_the_live_draft_and_preserves_the_flip(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_dir, handle) = composer_focus_window(cx);
        let draft = "A draft near the compact boundary. ".repeat(2);
        handle
            .update(cx, |composer, window, cx| {
                composer
                    .state
                    .update(cx, |state, _| state.selected_chat = Some("chat".into()));
                composer.on_state_changed(cx);
                composer.route_snap_until = None;
                composer.set_dock_frame(crate::composer_dock::DockFrame::settled(true), cx);
                composer.set_available_width(1232.0, cx);
                window.resize(size(px(1232.0), px(800.0)));
            })
            .unwrap();
        let draw = |cx: &mut gpui::TestAppContext| {
            cx.update_window(handle.into(), |_, window, cx| {
                window.draw(cx).clear();
            })
            .unwrap();
        };
        for _ in 0..3 {
            draw(cx);
        }
        // Finish measuring the wide input before typing; the hero's previous
        // width must not accidentally put this fixture into the resize hold.
        handle
            .update(cx, |composer, _, cx| {
                composer.width_changed_at = None;
                composer.input.update(cx, |input, cx| {
                    input.set_text(draft.clone(), cx);
                    input.selected_range = 2..8;
                });
            })
            .unwrap();
        for _ in 0..3 {
            draw(cx);
        }
        let input = handle
            .read_with(cx, |composer, cx| {
                assert!(
                    !composer.expanded_mode,
                    "draft must fit the wide compact input: text={}, capacity={}",
                    composer.input.read(cx).measured_text_width(),
                    composer.compact_capacity,
                );
                assert!(composer.input.read(cx).measured_text_width() < composer.compact_capacity);
                composer.input.clone()
            })
            .unwrap();

        handle
            .update(cx, |composer, window, cx| {
                composer.set_available_width(592.0, cx);
                window.resize(size(px(592.0), px(800.0)));
            })
            .unwrap();
        for _ in 0..3 {
            draw(cx);
        }
        handle
            .update(cx, |composer, _, _| {
                assert!(
                    composer.expanded_mode,
                    "narrowing must expand the overflowing draft immediately"
                );
                assert!(
                    composer.flip_morph.is_some(),
                    "resize must retain the height/controls morph"
                );
                assert!(composer.last_target_height > COMPACT_TOTAL_HEIGHT);
                composer.morph_clock -= Duration::from_secs(1);
            })
            .unwrap();
        draw(cx);
        handle
            .read_with(cx, |composer, _| {
                let surface = composer.surface_bounds.get().unwrap();
                let model = composer.model_bounds.get().unwrap();
                let right = surface.right() - px(1.0 + 12.0 + 28.0 + ACTION_PRIMARY_GAP);
                assert!(
                    (f32::from(model.right() - right)).abs() <= 1.0,
                    "expanded controls must remain on the right"
                );
            })
            .unwrap();

        handle
            .update(cx, |composer, window, cx| {
                composer.set_available_width(1232.0, cx);
                window.resize(size(px(1232.0), px(800.0)));
            })
            .unwrap();
        for _ in 0..3 {
            draw(cx);
        }
        handle
            .update(cx, |composer, _, _| {
                assert!(
                    composer.expanded_mode,
                    "widening must wait for resize to settle before collapsing"
                );
                composer.width_changed_at =
                    Some(Instant::now() - Duration::from_millis(RESIZE_SETTLE_MS + 1));
            })
            .unwrap();
        draw(cx);
        handle
            .read_with(cx, |composer, cx| {
                assert!(!composer.expanded_mode);
                assert!(composer.flip_morph.is_some());
                assert_eq!(composer.last_target_height, COMPACT_TOTAL_HEIGHT);
                assert_eq!(composer.input, input);
                assert_eq!(input.read(cx).text(), draft);
                assert_eq!(input.read(cx).selected_range, 2..8);
            })
            .unwrap();
    }

    #[gpui::test]
    fn composer_padding_and_file_prompt_restore_focus(cx: &mut gpui::TestAppContext) {
        let (dir, handle) = composer_focus_window(cx);
        let image_path = dir.path().join("attachment.png");
        let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD,
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aF1cAAAAASUVORK5CYII=").unwrap();
        std::fs::write(&image_path, png).unwrap();
        let input = handle
            .read_with(cx, |composer, _| composer.input.clone())
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.blur();
            window.draw(cx).clear();
            let bounds = input.read(cx).last_bounds.unwrap();
            // Click padding immediately left of the actual editor.
            let position = point(bounds.left() - px(4.0), bounds.center().y);
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    click_count: 1,
                    ..Default::default()
                }),
                cx,
            );
            assert!(input.read(cx).focus_handle.is_focused(window));
            window.dispatch_event(
                gpui::PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        for accepted in [false, true] {
            handle
                .update(cx, |composer, window, cx| {
                    composer
                        .input
                        .update(cx, |input, cx| input.set_text("Keep this draft", cx));
                    window.blur();
                    composer.open_file_picker(cx);
                })
                .unwrap();
            assert!(cx.did_prompt_for_paths());
            let path = image_path.clone();
            cx.simulate_path_prompt_response(move |_| accepted.then(|| vec![path]));
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| {
                window.draw(cx).clear();
                assert!(input.read(cx).focus_handle.is_focused(window));
                // An attached image is mentioned at the caret.
                assert_eq!(
                    input.read(cx).text(),
                    if accepted {
                        "Keep this draft [Image 1](zeron-image:1) "
                    } else {
                        "Keep this draft"
                    }
                );
            })
            .unwrap();
            assert_eq!(
                handle
                    .read_with(cx, |composer, _| composer.staged().len())
                    .unwrap(),
                usize::from(accepted)
            );
        }
        // The same staging path handles external file drops.
        handle
            .update(cx, |composer, window, cx| {
                window.blur();
                composer.add_paths(vec![image_path], cx);
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            assert!(input.read(cx).focus_handle.is_focused(window));
        })
        .unwrap();
    }

    #[gpui::test]
    fn staged_files_land_in_the_draft_they_were_added_to(cx: &mut gpui::TestAppContext) {
        let (dir, handle) = composer_focus_window(cx);
        let bmp = dir.path().join("shot.bmp");
        image::RgbImage::new(1, 1).save(&bmp).unwrap();
        handle
            .update(cx, |composer, _, cx| {
                composer.current_key = "chat-a".into();
                composer.add_paths(vec![bmp, dir.path().join("notes.txt")], cx);
                // Staging runs in the background; navigating meanwhile must
                // not move the attachment into the other chat's draft.
                composer.current_key = "chat-b".into();
                assert!(composer.attachments.values().all(Vec::is_empty));
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .read_with(cx, |composer, _| {
                let names: Vec<_> = composer.attachments["chat-a"]
                    .iter()
                    .map(|att| att.name.as_str())
                    .collect();
                assert_eq!(names, ["shot.png"]);
                assert!(!composer.attachments.contains_key("chat-b"));
            })
            .unwrap();
    }

    #[gpui::test]
    fn composer_picker_escape_restores_focus_but_click_away_does_not(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_dir, handle) = composer_focus_window(cx);
        let input = handle
            .read_with(cx, |composer, _| composer.input.clone())
            .unwrap();
        for escape in [true, false] {
            handle
                .update(cx, |composer, window, cx| {
                    composer
                        .pickers
                        .update(cx, |pickers, cx| pickers.open_model_menu(window, cx));
                })
                .unwrap();
            cx.update_window(handle.into(), |_, window, cx| {
                window.draw(cx).clear();
                assert!(!input.read(cx).focus_handle.is_focused(window));
            })
            .unwrap();
            if escape {
                cx.simulate_keystrokes(handle.into(), "escape");
            } else {
                cx.update_window(handle.into(), |_, window, cx| {
                    window.dispatch_event(
                        gpui::PlatformInput::MouseDown(MouseDownEvent {
                            button: MouseButton::Left,
                            position: point(px(5.0), window.viewport_size().height - px(1.0)),
                            click_count: 1,
                            ..Default::default()
                        }),
                        cx,
                    );
                })
                .unwrap();
            }
            cx.update_window(handle.into(), |_, window, cx| {
                window.draw(cx).clear();
                assert_eq!(input.read(cx).focus_handle.is_focused(window), escape);
            })
            .unwrap();
        }
    }

    #[gpui::test]
    fn inputs_release_focus_on_click_away(cx: &mut gpui::TestAppContext) {
        struct Inputs(Vec<Entity<ComposerInput>>);
        impl Render for Inputs {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().flex().flex_col().children(
                    self.0
                        .iter()
                        .map(|input| div().w(px(200.0)).child(input.clone())),
                )
            }
        }
        cx.update(|cx| cx.set_global(Theme::dark()));
        let host = cx.add_window(|_, cx| {
            Inputs(vec![
                cx.new(|cx| ComposerInput::new("Composer", cx)),
                cx.new(|cx| ComposerInput::new("URL", cx).with_single_line()),
            ])
        });
        cx.run_until_parked();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        let inputs = host.read_with(cx, |host, _| host.0.clone()).unwrap();
        // Visit both fields and return to the first: click-away capture must
        // never clear focus acquired by the clicked input during bubbling.
        for index in [0, 1, 0] {
            let position =
                inputs[index].read_with(cx, |input, _| input.last_bounds.unwrap().center());
            cx.update_window(host.into(), |_, window, cx| {
                window.dispatch_event(
                    gpui::PlatformInput::MouseDown(MouseDownEvent {
                        button: MouseButton::Left,
                        position,
                        click_count: 1,
                        ..Default::default()
                    }),
                    cx,
                );
                assert!(inputs[index].read(cx).focus_handle.is_focused(window));
                window.dispatch_event(
                    gpui::PlatformInput::MouseUp(MouseUpEvent {
                        button: MouseButton::Left,
                        position,
                        ..Default::default()
                    }),
                    cx,
                );
            })
            .unwrap();
        }
        cx.update_window(host.into(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position: point(px(400.0), px(300.0)),
                    click_count: 1,
                    ..Default::default()
                }),
                cx,
            );
            assert!(window.focused(cx).is_none());
        })
        .unwrap();
    }

    #[test]
    fn workspace_commands_preserve_native_commands_and_avoid_collisions() {
        let native = invocation_candidates(
            vec![
                SlashCommand {
                    name: "model".into(),
                    description: "Native model command".into(),
                    input_hint: Some("model id".into()),
                    kind: Default::default(),
                },
                SlashCommand {
                    name: "zeron:model".into(),
                    description: "Plugin command".into(),
                    input_hint: None,
                    kind: Default::default(),
                },
            ],
            vec![],
        );
        let rows = with_workspace_commands(native, true);
        assert_eq!(rows.len(), 11);
        assert!(rows[0].workspace_command.is_none());
        assert_eq!(rows[0].input_hint.as_deref(), Some("model id"));
        assert_eq!(workspace_command_for_text("/model", &rows), None);
        assert_eq!(workspace_command_for_text("/zeron:model", &rows), None);
        assert_eq!(
            workspace_command_for_text("/zeron:zeron:model", &rows),
            Some(WorkspaceCommand::Model)
        );
        assert_eq!(with_workspace_commands(rows, true).len(), 11);
        let draft_rows = with_workspace_commands(vec![], false);
        assert_eq!(draft_rows.len(), 4);
        assert_eq!(workspace_command_for_text("/diff", &draft_rows), None);
        assert_eq!(
            workspace_command_for_text("/model  ", &draft_rows),
            Some(WorkspaceCommand::Model)
        );
        for literal in [
            "    /model",
            "`/model`",
            "```\n/model\n```",
            "please /model",
            "/model extra",
            "/model/path",
        ] {
            assert_eq!(
                workspace_command_for_text(literal, &draft_rows),
                None,
                "{literal:?}"
            );
        }
    }

    #[gpui::test]
    fn workspace_command_submission_is_local_and_preserves_drafts(cx: &mut gpui::TestAppContext) {
        let state = cx.new(|_| AppState::new());
        let composer = cx.new(|cx| Composer::new(state, cx));
        let actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let captured = actions.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&composer, move |_, event: &ComposerEvent, _| {
                if let ComposerEvent::WorkspaceCommand(command) = event {
                    captured.borrow_mut().push(*command);
                }
            })
        });
        composer.update(cx, |composer, cx| {
            composer
                .input
                .update(cx, |input, cx| input.set_text("/model", cx));
            composer.update_slash("/model", 6, cx);
            composer.on_submit(cx);
            assert!(composer.input.read(cx).text().is_empty());
            assert!(
                composer.failure.is_none(),
                "must not require an engine or create a run"
            );
        });
        assert_eq!(&*actions.borrow(), &[WorkspaceCommand::Model]);
        composer.update(cx, |composer, cx| {
            composer
                .input
                .update(cx, |input, cx| input.set_text("/model keep this draft", cx));
            composer.update_slash("/model keep this draft", 6, cx);
            composer.accept_slash(cx);
            assert_eq!(composer.input.read(cx).text(), "keep this draft");
            assert!(composer.failure.is_none());
        });
        assert_eq!(actions.borrow().len(), 2);
    }

    #[gpui::test]
    fn inline_slash_completion_distinguishes_actions_from_references(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, window, cx| {
                let attachment = attachments::stage_png_bytes("draft.png".into(), Vec::new());
                let attachment_id = attachment.id.clone();
                composer
                    .attachments
                    .insert(composer.current_key.clone(), vec![attachment]);
                composer.editing_queued = Some("queued-draft".into());
                for draft in [
                    "café /model keep this",
                    "first\n/model\nlast",
                    "(/model) text",
                ] {
                    let cursor = draft.find("/model").unwrap() + 6;
                    composer
                        .input
                        .update(cx, |input, cx| input.set_text(draft, cx));
                    composer.update_slash(draft, cursor, cx);
                    assert!(composer.slash.token.is_some());
                    composer.accept_slash(cx);
                    let expected = draft.replace("/model ", "").replace("/model", "");
                    assert_eq!(composer.input.read(cx).text(), expected);
                    assert!(composer.failure.is_none());
                    assert_eq!(composer.staged()[0].id, attachment_id);
                    assert_eq!(composer.editing_queued.as_deref(), Some("queued-draft"));
                    composer.input.update(cx, |input, cx| {
                        input.undo(&Undo, window, cx);
                        assert_eq!(input.text(), draft, "action removal is undoable");
                    });
                }
                let skill = zeron_proto::invocation::Invocation::Skill {
                    name: "review".into(),
                    path: "/repo/SKILL.md".into(),
                    command: None,
                };
                for invocation in [
                    zeron_proto::invocation::Invocation::Command {
                        name: "review".into(),
                    },
                    skill,
                ] {
                    let draft = "café /rev after";
                    composer
                        .input
                        .update(cx, |input, cx| input.set_text(draft, cx));
                    composer.update_slash(draft, "café /rev".len(), cx);
                    composer.slash_cache.insert(
                        composer.slash.context.clone(),
                        vec![InvocationCandidate {
                            name: "review".into(),
                            description: String::new(),
                            input_hint: None,
                            workspace_command: None,
                            invocation: invocation.clone(),
                        }],
                    );
                    composer.refilter_slash(cx);
                    composer.accept_slash(cx);
                    assert_eq!(
                        composer.input.read(cx).text(),
                        format!(
                            "café {} after",
                            match &invocation {
                                zeron_proto::invocation::Invocation::Command { .. } =>
                                    "/review".to_string(),
                                _ => invocation.link(),
                            }
                        )
                    );
                    assert!(composer.failure.is_none());
                }
            })
            .unwrap();
    }

    #[gpui::test]
    fn projectless_composer_allows_send_and_enter_submission(cx: &mut gpui::TestAppContext) {
        let state = cx.new(|_| AppState::new());
        let composer = cx.new(|cx| Composer::new(state.clone(), cx));
        // A device with no projects is a valid home-directory target too.
        composer.update(cx, |composer, cx| assert!(!composer.send_blocked(cx)));
        state.update(cx, |state, cx| state.select_space(None, cx));
        composer.update(cx, |composer, cx| {
            composer.input.update(cx, |input, cx| {
                input.set_text("Hello without a project", cx);
            });
            assert_eq!(composer.button_mode(cx), SendButtonMode::Send);
            assert!(
                !composer.send_blocked(cx),
                "The Send button must be enabled without a project"
            );
            composer.on_submit(cx);
            // With no engine attached, reaching the normal send error proves
            // Enter dispatched instead of silently stopping at the UI gate.
            assert_eq!(composer.failure.as_deref(), Some("Engine not connected"));
            composer.queue_edit_finishing = true;
            assert!(
                composer.send_blocked(cx),
                "Pending edits must still block submission"
            );
        });
    }

    #[gpui::test]
    fn submission_preserves_literal_command_and_skill_indentation(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let skill = zeron_proto::invocation::Invocation::Skill {
            name: "review".into(),
            path: "/skills/review/SKILL.md".into(),
            command: None,
        }
        .link();
        for editing_queue in [false, true] {
            for raw in [
                "    /review".to_string(),
                "\t/compact".into(),
                "\u{a0}/review".into(),
                format!("    {skill}"),
                "Keep this hard break  ".into(),
                " \t\n ".into(),
            ] {
                let (out, mut requests) = tokio::sync::mpsc::channel::<String>(64);
                let (_replies, inbound) = tokio::sync::mpsc::channel::<String>(64);
                let state = cx.new(|_| AppState::new());
                state.update(cx, |state, _| {
                    state.set_test_engine(crate::state::EngineHandle::from_test_client(
                        zeron_rpc::RpcClient::new(out, inbound),
                    ));
                    state.selected_chat = Some("literal-draft".into());
                });
                let composer = cx.new(|cx| Composer::new(state, cx));
                composer.update(cx, |composer, cx| {
                    composer
                        .input
                        .update(cx, |input, cx| input.set_text(&raw, cx));
                    if editing_queue {
                        composer.editing_queued = Some("queued-row".into());
                        composer.queue_edit_lease_id = Some("lease".into());
                        composer.queue_edit_chat_id = Some("literal-draft".into());
                        composer.queue_edit_host_device_id = Some("host".into());
                    }
                    composer.on_submit(cx);
                });
                cx.run_until_parked();
                let mut submitted = None;
                let mut discarded = false;
                while let Ok(frame) = requests.try_recv() {
                    let frame: zeron_rpc::ClientFrame = serde_json::from_str(&frame).unwrap();
                    if frame.method.as_deref() == Some(methods::FINISH_QUEUED_MESSAGE_EDIT) {
                        discarded = frame.params["action"] == "discard";
                        submitted = frame.params["text"].as_str().map(str::to_owned);
                    }
                    if frame.method.as_deref() == Some(methods::QUEUE_COMMAND) {
                        submitted = Some(
                            frame.params["command"]["request"]["prompt"]
                                .as_str()
                                .unwrap()
                                .to_owned(),
                        );
                    }
                }
                if raw.trim().is_empty() {
                    assert!(submitted.is_none(), "whitespace alone must not submit");
                    assert_eq!(discarded, editing_queue);
                } else {
                    let submitted = submitted.expect("submission must reach the engine RPC");
                    assert_eq!(submitted, raw);
                    assert!(zeron_proto::invocation::leading_command(&submitted).is_none());
                    assert!(zeron_proto::invocation::invocation_links(&submitted).is_empty());
                }
            }
        }
    }

    /// Issue #406: Enter submits — it must never stop a run. Stop mode only
    /// exists on a live run with an EMPTY composer, so a habitual
    /// double-Enter after sending interrupted the just-dispatched prompt
    /// and the agent ate it silently. Keyboard stop is the Esc setting's
    /// job; Enter on an empty composer is a no-op.
    #[gpui::test]
    fn enter_on_empty_composer_during_a_live_run_never_interrupts(cx: &mut gpui::TestAppContext) {
        // RpcClient::new spawns its reader on tokio — give the test a reactor.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        // The client's write end: any RPC Enter dispatches lands here.
        let (out, mut server_in) = tokio::sync::mpsc::channel::<String>(16);
        let (_server_out, inbound) = tokio::sync::mpsc::channel::<String>(16);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(
                zeron_rpc::RpcClient::new(out, inbound),
            ));
            state.selected_chat = Some("c".into());
            // A send in flight reads as Working — the double-Enter window.
            state.begin_pending_send("c", "m1", chrono::Utc::now());
        });
        let composer = cx.new(|cx| Composer::new(state, cx));
        composer.update(cx, |composer, cx| {
            assert_eq!(composer.button_mode(cx), SendButtonMode::Stop);
            composer.on_submit(cx);
            // interrupt_chat marks the chat before its RPC even flies.
            assert!(!composer.interrupting.contains("c"));
        });
        // If Enter had dispatched an interrupt, the spawned call would have
        // written its frame into the channel by the time the executor parks.
        cx.run_until_parked();
        assert!(server_in.try_recv().is_err());
    }

    #[gpui::test]
    fn inline_reference_text_preserves_selection_and_ime(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        let raw = format!(
            "{} {} {}",
            local_file_link("src/main.rs", false),
            zeron_proto::invocation::Invocation::Skill {
                command: None,
                name: "review".into(),
                path: "/repo/SKILL.md".into(),
            }
            .link(),
            zeron_proto::invocation::Invocation::Command {
                name: "help".into()
            }
            .link(),
        );
        let input = handle
            .read_with(cx, |composer, _| composer.input.clone())
            .unwrap();
        input.update(cx, |input, cx| input.set_text(&raw, cx));
        cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        input.update(cx, |input, _| {
            assert_eq!(input.projection.mentions.len(), 3);
            for (mention, display) in &input.projection.mentions {
                let label = &input.projection.display[display.clone()];
                if mention.is_invocation() {
                    assert!(label.starts_with(&format!("{MENTION_SIDE_PAD}{}", mention.prefix())));
                    assert!(label.ends_with(MENTION_SIDE_PAD));
                } else {
                    assert!(label.starts_with(&format!("{MENTION_SIDE_PAD}{CHIP_ICON_SLOT}")));
                    assert!(label.ends_with(CHIP_TRAILING_PAD));
                }
                let shell = input.bounds_for_display_range(display.clone());
                assert_eq!(shell.len(), 1);
                assert!(shell[0].size.width > px(0.0));
                let start = display.start + MENTION_SIDE_PAD.len();
                assert_eq!(input.projection.display_to_raw(start), mention.range.start);
                assert_eq!(
                    input
                        .projection
                        .normalize_range(mention.range.start + 1..mention.range.end - 1),
                    mention.range,
                );
            }
            let display = input.projection.display.clone();
            input.marked_range = Some(raw.len()..raw.len());
            input.refresh_projection();
            assert_eq!(
                input.projection.display, display,
                "IME must retain reference text"
            );
            assert_eq!(input.text(), raw);
        });
    }

    #[gpui::test]
    fn inline_chips_wrap_as_units(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle.update(cx, |composer, window, cx| {
            let file = local_file_link("src/composer.rs", false);
            let skill = zeron_proto::invocation::Invocation::Skill { name: "review-changes".into(), path: "/repo/SKILL.md".into(), command: None }.link();
            let raw = format!("Review {file} with {skill} and enough trailing prose to need more than one additional row of wrapping.");
            composer.input.update(cx, |input, cx| {
                input.set_text(&raw, cx);
                for width in [240., 300., 360.] {
                    input.layout_text(px(width), &window.text_style(), window, cx);
                    for (_, display) in &input.projection.mentions {
                        assert_eq!(input.bounds_for_display_range(display.clone()).len(), 1, "chip split at {width}");

                    }
                }
                assert_eq!(input.text(), raw);
            });
        }).unwrap();
    }

    #[gpui::test]
    fn crlf_navigation_deletion_and_newlines_keep_the_pair_intact(cx: &mut gpui::TestAppContext) {
        with_composer_input(cx, |input, window, cx| {
            for (first, continued) in [
                ("plain café", ""),
                ("- item", "- "),
                ("- [x] item", "- [ ] "),
            ] {
                let raw = format!("{first}\r\nnext");
                input.set_text(&raw, cx);
                input.move_to(0, cx);
                input.end(&End, window, cx);
                assert_eq!(input.cursor_offset(), first.len());
                input.layout_text(px(600.0), &window.text_style(), window, cx);
                assert_eq!(
                    input.caret_for_point(point(px(590.0), px(1.0))).0,
                    first.len()
                );
                assert_eq!(
                    input.selection_unit(PressIntent::Line, 0),
                    0..first.len() + 2
                );
                input.move_to(0, cx);
                input.select_end(&SelectEnd, window, cx);
                assert_eq!(input.selected_range, 0..first.len());
                input.move_to(0, cx);
                input.end(&End, window, cx);
                input.backspace(&Backspace, window, cx);
                let previous = first.grapheme_indices(true).last().unwrap().0;
                assert_eq!(input.text(), format!("{}\r\nnext", &first[..previous]));
                input.undo(&Undo, window, cx);
                input.move_to(first.len(), cx);
                input.delete(&Delete, window, cx);
                assert_eq!(input.text(), format!("{first}next"));
                input.undo(&Undo, window, cx);
                input.move_to(first.len() + 2, cx);
                input.backspace(&Backspace, window, cx);
                assert_eq!(input.text(), format!("{first}next"));
                input.undo(&Undo, window, cx);
                input.move_to(0, cx);
                input.end(&End, window, cx);
                input.newline(&Newline, window, cx);
                assert_eq!(input.text(), format!("{first}\r\n{continued}\r\nnext"));
                input.undo(&Undo, window, cx);
                input.move_to(2, cx);
                input.delete_to_line_end(&DeleteToLineEnd, window, cx);
                assert_eq!(input.text(), format!("{}\r\nnext", &first[..2]));
                input.undo(&Undo, window, cx);
                assert_eq!(input.text(), raw);
            }
            input.set_text("\r\nnext", cx);
            input.move_to(0, cx);
            input.end(&End, window, cx);
            assert_eq!(input.cursor_offset(), 0);
            input.backspace(&Backspace, window, cx);
            assert_eq!(input.text(), "\r\nnext");
            input.delete(&Delete, window, cx);
            assert_eq!(input.text(), "next");
        });
    }

    #[gpui::test]
    fn soft_wrap_carets_preserve_pointer_keyboard_and_ime_affinity(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle.update(cx, |composer, window, cx| {
            composer.input.update(cx, |input, cx| {
                input.set_text("Café and emoji 🦀 wrap across several visual rows with enough text to keep moving downward.", cx);
                input.layout_text(px(140.0), &window.text_style(), window, cx);
                let line = &input.last_lines[0];
                assert!(line.wrap_boundaries().len() >= 2);
                let boundary = line.wrap_boundaries()[0];
                let offset = line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index;
                let upstream = input.point_for_display_index_with_affinity(offset, CaretAffinity::Upstream).unwrap();
                let downstream = input.point_for_display_index(offset).unwrap();
                assert_eq!(upstream.y, px(0.0));
                assert_eq!(downstream, point(px(0.0), input.line_height));
                assert!(upstream.x > downstream.x);
                let raw_offset = input.projection.display_to_raw(offset);
                assert_eq!(input.caret_for_point(upstream + point(px(20.0), px(1.0))), (raw_offset, CaretAffinity::Upstream));
                assert_eq!(input.caret_for_point(downstream + point(px(0.0), px(1.0))), (raw_offset, CaretAffinity::Downstream));
                let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(140.0), px(200.0)));
                input.last_bounds = Some(bounds);
                for (local, affinity) in [(upstream, CaretAffinity::Upstream), (downstream, CaretAffinity::Downstream)] {
                    input.on_mouse_down(&MouseDownEvent {
                        button: MouseButton::Left,
                        position: bounds.origin + local + point(px(0.0), px(1.0)),
                        click_count: 1,
                        ..Default::default()
                    }, window, cx);
                    assert_eq!(input.cursor_offset(), raw_offset);
                    assert_eq!(input.caret_affinity, affinity);
                    assert_eq!(input.cursor_point(), Some(local));
                    let utf16 = input.offset_to_utf16(raw_offset);
                    let ime = input.bounds_for_range(utf16..utf16, bounds, window, cx).unwrap();
                    assert_eq!(ime.origin, bounds.origin + local);
                    input.replace_text_in_range(None, "x", window, cx);
                    input.undo(&Undo, window, cx);
                    input.layout_text(px(140.0), &window.text_style(), window, cx);
                    assert_eq!(input.cursor_offset(), raw_offset);
                    assert_eq!(input.caret_affinity, affinity);
                    assert_eq!(input.cursor_point(), Some(local));
                }
                input.move_to(0, cx);
                input.down(&Down, window, cx);
                assert_eq!(input.cursor_point(), Some(downstream));
                input.down(&Down, window, cx);
                assert_eq!(input.cursor_point().unwrap().y, input.line_height * 2);
                input.up(&Up, window, cx);
                assert_eq!(input.cursor_point(), Some(downstream));
                input.up(&Up, window, cx);
                assert_eq!(input.cursor_offset(), 0);
                input.select_down(&SelectDown, window, cx);
                assert_eq!(input.selected_range, 0..raw_offset);
                assert_eq!(input.cursor_point(), Some(downstream));
                input.select_up(&SelectUp, window, cx);
                assert_eq!(input.selected_range, 0..0);
            });
        }).unwrap();
    }

    #[gpui::test]
    fn vertical_navigation_retains_column_across_short_and_wrapped_rows(
        cx: &mut gpui::TestAppContext,
    ) {
        with_composer_input(cx, |input, window, cx| {
            let long = "abcdefghijklmnopqrstuvwx";
            let raw = format!("{long}\nx\n{long}");
            let last_start = long.len() + 3;
            for width in [1000.0, 140.0] {
                input.set_text(&raw, cx);
                input.layout_text(px(width), &window.text_style(), window, cx);
                input.move_to(7, cx);
                let start = input.cursor_point().unwrap();
                let target = input.point_for_index(last_start + 7).unwrap();
                let rows =
                    (f32::from(target.y - start.y) / f32::from(input.line_height)).round() as usize;
                assert!(rows >= 2);
                for _ in 0..rows {
                    input.down(&Down, window, cx);
                }
                assert_eq!(
                    input.cursor_offset(),
                    last_start + 7,
                    "column lost at width {width}"
                );
                assert_eq!(input.preferred_column, Some(start.x));
                for _ in 0..rows {
                    input.up(&Up, window, cx);
                }
                assert_eq!(input.cursor_offset(), 7);
                for _ in 0..rows {
                    input.select_down(&SelectDown, window, cx);
                }
                assert_eq!(input.selected_range, 7..last_start + 7);
                for _ in 0..rows {
                    input.select_up(&SelectUp, window, cx);
                }
                assert_eq!(input.selected_range, 7..7);
                input.right(&Right, window, cx);
                assert_eq!(input.preferred_column, None);
                input.down(&Down, window, cx);
                assert!(input.preferred_column.is_some());
                input.replace_text_in_range(None, "é", window, cx);
                assert_eq!(input.preferred_column, None);
                input.undo(&Undo, window, cx);
                assert_eq!(input.text(), raw);
            }
        });
    }

    #[gpui::test]
    fn wrapped_rich_lines_keep_caret_and_range_geometry_aligned(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle.update(cx, |composer, window, cx| {
            composer.input.update(cx, |input, cx| {
                let file = local_file_link("src/composer.rs", false);
                let raw = format!("- First **bold** item with café and enough words to wrap over several visual rows\n- Second item with {file} and more trailing content to wrap\n\nDone");
                input.set_text(&raw, cx);
                input.layout_text(px(190.0), &window.text_style(), window, cx);
                let mut y_offset = px(0.0);
                let mut checked = 0;
                for (line_ix, line) in input.last_lines.iter().enumerate() {
                    let line_start = input.line_starts[line_ix];
                    for (row, boundary) in line.wrap_boundaries().iter().enumerate() {
                        let local_offset = line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index;
                        let display_offset = line_start + local_offset;
                        let raw_offset = input.projection.display_to_raw(display_offset);
                        let downstream = input.point_for_display_index(display_offset).unwrap();
                        assert_eq!(downstream.y, y_offset + input.line_height * (row + 1));
                        assert_eq!(downstream.x, input.line_indents[line_ix]);
                        assert_eq!(input.caret_for_point(downstream + point(px(0.0), px(1.0))), (raw_offset, CaretAffinity::Downstream));
                        let upstream = input.point_for_display_index_with_affinity(display_offset, CaretAffinity::Upstream).unwrap();
                        assert_eq!(upstream.y + input.line_height, downstream.y);
                        assert_eq!(input.caret_for_point(upstream + point(px(20.0), px(1.0))), (raw_offset, CaretAffinity::Upstream));
                        let end = line_start + line.text[local_offset..].char_indices().nth(1).map_or(line.len(), |(offset, _)| local_offset + offset);
                        let boxes = input.bounds_for_display_range(display_offset..end);
                        assert!(!boxes.is_empty());
                        assert_eq!(boxes[0].origin, downstream);
                        checked += 1;
                    }
                    y_offset += line.size(input.line_height).height;
                }
                assert!(checked >= 3);
                assert_eq!(input.text(), raw);
            });
        }).unwrap();
    }

    #[test]
    fn hidden_markdown_hit_testing_targets_visible_text() {
        let raw = "***café***\nactive";
        let projection = TextProjection::rich(raw, Some(12..raw.len()), None);
        assert_eq!(projection.display_to_raw(0), 3);
        assert_eq!(
            &raw[projection.display_to_raw(0)..projection.display_to_raw(1)],
            "c"
        );
        for (display, _) in projection.display.char_indices() {
            assert!(raw.is_char_boundary(projection.display_to_raw(display)));
        }
    }

    #[test]
    fn long_chip_labels_keep_graphemes_and_full_source_identity() {
        let label = "ä".repeat(48) + ".rs";
        let shown = compact_chip_label(&label);
        assert_eq!(shown.graphemes(true).count(), 31);
        assert!(shown.ends_with(".rs"));
        let raw = local_file_link(&format!("src/{label}"), false);
        let projected = TextProjection::rich(&raw, Some(0..raw.len()), None);
        assert_eq!(projected.mentions[0].0.range, 0..raw.len());
        assert!(projected.mentions[0].0.path.ends_with(&label));
        assert!(projected.display.contains('…'));
        assert_eq!(projected.normalize_range(1..raw.len() - 1), 0..raw.len());
    }

    #[test]
    fn chip_labels_stay_compact_for_repeats_and_distinct_after_truncation() {
        let path = "src/components/field.rs";
        let raw = format!(
            "{} {}",
            local_file_link(path, false),
            local_file_link(path, false)
        );
        assert_eq!(
            mention_display_labels(&file_mention_links(&raw)),
            ["field.rs", "field.rs"]
        );
        let left = format!("{}alpha{}", "x".repeat(20), "tail".repeat(8));
        let right = format!("{}bravo{}", "x".repeat(20), "tail".repeat(8));
        assert_eq!(compact_chip_label(&left), compact_chip_label(&right));
        let shown = compact_chip_labels(&[left.clone(), right, left]);
        assert_ne!(shown[0], shown[1]);
        assert_eq!(shown[0], shown[2]);
    }

    #[test]
    fn pasted_references_only_recognize_complete_prose_tokens() {
        let text = "Use @README.md, ($framer) and /review. café@example.com $12.50 /usr/bin x/review `@code.rs` \\@escaped.rs [@label](url) https://host/@remote";
        let tokens = pasted_reference_tokens(text, 0..text.len());
        assert_eq!(
            tokens
                .iter()
                .map(|(prefix, token)| (*prefix, token.query.as_str()))
                .collect::<Vec<_>>(),
            vec![('@', "README.md"), ('$', "framer"), ('/', "review")]
        );
        for text in [
            "```\n@README.md\n```",
            "    @README.md",
            "<user@example.com>",
            "$HOME/bin",
            "[x](https://host/@file)",
        ] {
            assert!(
                pasted_reference_tokens(text, 0..text.len()).is_empty(),
                "{text}"
            );
        }
        let text = "prefix@README.md suffix";
        assert!(pasted_reference_tokens(text, 6..15).is_empty());
        let text = "@README.md";
        assert!(
            pasted_reference_tokens(text, 0..5).is_empty(),
            "partial pasted token"
        );
        // Invalid candidates consume the same work budget as accepted ones.
        // Do not repeatedly parse a large pasted block of literal examples.
        let text = format!("```\n{}\n```\n$review", "@example ".repeat(4_000));
        assert!(pasted_reference_tokens(&text, 0..text.len()).is_empty());
    }

    #[gpui::test]
    fn projectless_paste_discovers_invocations_and_rejects_changed_targets(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let directory = tempfile::tempdir().unwrap();
        crate::settings::composer::ComposerDefaults {
            harness: Some(HarnessId::Codex),
            ..Default::default()
        }
        .save(directory.path())
        .unwrap();
        cx.update(|cx| crate::settings::init(Default::default(), directory.path(), cx));
        for change_target in [false, true] {
            let (out, mut requests) = tokio::sync::mpsc::channel(64);
            let (replies, inbound) = tokio::sync::mpsc::channel(64);
            let state = cx.new(|_| AppState::new());
            state.update(cx, |state, _| {
                state.data_dir = Some(directory.path().to_path_buf());
                state.selected_device = Some("peer".into());
                state.devices = vec![
                    serde_json::from_value(serde_json::json!({
                        "id": "peer", "name": "Peer", "platform": "linux",
                        "capabilities": [capabilities::COMPOSER_REFERENCES_V1]
                    }))
                    .unwrap(),
                ];
                state.set_test_engine(crate::state::EngineHandle::from_test_client(
                    zeron_rpc::RpcClient::new(out, inbound),
                ));
            });
            let composer = cx.new(|cx| Composer::new(state.clone(), cx));
            let raw = "$review /compact @README.md";
            composer.update(cx, |composer, cx| {
                assert!(composer.file_search_params("", cx).is_none());
                composer
                    .input
                    .update(cx, |input, cx| input.set_text(raw, cx));
                let revision = composer.input.read(cx).edit_revision;
                composer.resolve_pasted_references(0..raw.len(), revision, cx);
            });
            cx.run_until_parked();
            if change_target {
                state.update(cx, |state, _| state.selected_device = Some("other".into()));
            }
            let mut methods = Vec::new();
            while let Ok(frame) = requests.try_recv() {
                let frame: zeron_rpc::ClientFrame = serde_json::from_str(&frame).unwrap();
                assert_eq!(frame.params["targetDeviceId"], "peer");
                assert!(frame.params.get("cwd").is_none());
                let value = match frame.method.as_deref() {
                    Some(methods::LIST_COMMANDS) => {
                        serde_json::json!([{"name":"compact", "description":"Compact"}])
                    }
                    Some(methods::LIST_SKILLS) => {
                        serde_json::json!([{"name":"review", "path":"/skills/SKILL.md", "description":"Review", "enabled":true}])
                    }
                    method => panic!("unexpected projectless request: {method:?}"),
                };
                methods.push(frame.method.unwrap());
                replies
                    .try_send(
                        serde_json::to_string(&zeron_rpc::ServerFrame {
                            id: frame.id,
                            ok: Some(value),
                            ..Default::default()
                        })
                        .unwrap(),
                    )
                    .unwrap();
            }
            methods.sort();
            let mut expected = vec![methods::LIST_COMMANDS, methods::LIST_SKILLS];
            expected.sort();
            assert_eq!(methods, expected);
            runtime.block_on(async { tokio::task::yield_now().await });
            cx.run_until_parked();
            composer.read_with(cx, |composer, cx| {
                let text = composer.input.read(cx).text();
                if change_target {
                    assert_eq!(text, raw);
                } else {
                    assert_eq!(zeron_proto::invocation::invocation_links(text).len(), 2);
                    assert!(text.ends_with(" @README.md"));
                }
            });
        }
    }

    /// A hand-started side chat discovers commands, skills and files through
    /// its parent without writing a row. Its first send creates it before the
    /// run and switches subsequent discovery to its own persisted identity.
    #[gpui::test]
    fn unsaved_side_chat_is_created_by_its_first_send(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let directory = tempfile::tempdir().unwrap();
        cx.update(|cx| crate::settings::init(Default::default(), directory.path(), cx));
        let (out, mut requests) = tokio::sync::mpsc::channel::<String>(256);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(256);
        let parent = cx.new(|_| AppState::new());
        parent.update(cx, |state, _| {
            state.data_dir = Some(directory.path().to_path_buf());
            state.chats = vec![
                serde_json::from_value(serde_json::json!({
                    "id": "main", "deviceId": "local", "cwd": "/tmp/main",
                    "archived": false, "createdAt": chrono::Utc::now(),
                    "config": { "harness": "codex", "sandbox": "workspace-write" },
                }))
                .unwrap(),
            ];
            state.set_test_engine(crate::state::EngineHandle::from_test_client(
                zeron_rpc::RpcClient::new(out, inbound),
            ));
        });
        let chat: zeron_proto::Chat = serde_json::from_value(serde_json::json!({
            "id": "side", "createdByChatId": "main", "deviceId": "local", "cwd": "/tmp/main",
            "archived": false, "createdAt": chrono::Utc::now(),
            "config": { "harness": "codex", "model": "child-model", "reasoning": "low",
                "sandbox": "workspace-write" },
        }))
        .unwrap();
        let side = cx.new(|cx| AppState::side_chat_state(&parent, chat, true, cx));
        let mut drain = || {
            let mut frames = Vec::new();
            while let Ok(frame) = requests.try_recv() {
                frames.push(serde_json::from_str::<zeron_rpc::ClientFrame>(&frame).unwrap());
            }
            frames
        };
        let touches_side = |frame: &zeron_rpc::ClientFrame| frame.params["chatId"] == "side";
        cx.run_until_parked();
        assert!(!drain().iter().any(touches_side));

        let composer = cx.new(|cx| Composer::new(side.clone(), cx));
        // Discovery must work before createChat, including a different agent
        // from the parent's. Opening any completion must remain read-only.
        // Ends on the inherited harness, so the first send below mints the
        // fixture's config.
        let inherited = side
            .read_with(cx, |state, _| {
                state.selected_chat_row().and_then(|c| c.config.clone())
            })
            .unwrap();
        for harness in [HarnessId::ClaudeCode, HarnessId::Codex] {
            side.update(cx, |state, cx| {
                let mut config = inherited.clone();
                config.harness = harness;
                state.apply_chat_config("side", config);
                cx.notify();
            });
            cx.run_until_parked();
            for trigger in ["/", "$", "@"] {
                if trigger == "$" && harness != HarnessId::Codex {
                    continue;
                }
                composer.update(cx, |composer, cx| {
                    composer
                        .input
                        .update(cx, |input, cx| input.set_text(trigger, cx));
                });
                cx.run_until_parked();
                cx.executor().advance_clock(Duration::from_millis(100));
                cx.run_until_parked();
                let mut discovery = Vec::new();
                for frame in drain() {
                    assert_ne!(frame.method.as_deref(), Some(methods::MUTATE));
                    assert!(
                        !touches_side(&frame),
                        "unsaved chat must not be queried: {frame:?}"
                    );
                    let value = match frame.method.as_deref() {
                        Some(methods::LIST_COMMANDS) => serde_json::json!([
                            { "name": "compact", "description": "Compact" }
                        ]),
                        Some(methods::LIST_SKILLS) => serde_json::json!([
                            { "name": "review", "path": "/skills/SKILL.md",
                              "description": "Review", "enabled": true }
                        ]),
                        Some(methods::SEARCH_FILES) => serde_json::json!([
                            { "path": "README.md", "isDir": false }
                        ]),
                        _ => continue,
                    };
                    assert_eq!(frame.params["chatId"], "main");
                    assert_eq!(frame.params["targetDeviceId"], "local");
                    assert_eq!(frame.params["cwd"], "/tmp/main");
                    if frame.method.as_deref() != Some(methods::SEARCH_FILES) {
                        assert_eq!(frame.params["harness"], serde_json::json!(harness));
                    }
                    discovery.push(frame.method.clone().unwrap());
                    replies
                        .try_send(
                            serde_json::to_string(&zeron_rpc::ServerFrame {
                                id: frame.id,
                                ok: Some(value),
                                ..Default::default()
                            })
                            .unwrap(),
                        )
                        .unwrap();
                }
                let expected = match trigger {
                    "/" => vec![methods::LIST_COMMANDS, methods::LIST_SKILLS],
                    "$" => vec![methods::LIST_SKILLS],
                    _ => vec![methods::SEARCH_FILES],
                };
                discovery.sort();
                assert_eq!(discovery, expected);
                runtime.block_on(async { tokio::task::yield_now().await });
                cx.run_until_parked();
                composer.read_with(cx, |composer, _| {
                    if trigger == "@" {
                        assert!(!composer.mention.loading);
                        assert!(composer.mention.error.is_none());
                        assert_eq!(composer.mention.results[0].path, "README.md");
                    } else {
                        assert!(!composer.slash.loading);
                        assert!(composer.slash.error.is_none());
                        let expected = if trigger == "$" { "review" } else { "compact" };
                        assert!(
                            composer.slash_cache[&composer.slash.context]
                                .iter()
                                .any(|row| row.name == expected)
                        );
                    }
                });
                assert!(side.read_with(cx, |state, _| state.side_chat_unsaved()));
                assert_eq!(parent.read_with(cx, |state, _| state.chats.len()), 1);
            }
        }
        composer.update(cx, |composer, cx| {
            composer
                .input
                .update(cx, |input, cx| input.set_text("hello", cx));
            composer.on_submit(cx);
        });
        cx.run_until_parked();
        let sent: Vec<_> = drain().into_iter().filter(touches_side).collect();
        let [create] = sent.as_slice() else {
            panic!("expected only createChat, got {sent:?}");
        };
        assert_eq!(create.method.as_deref(), Some(methods::MUTATE));
        assert_eq!(create.params["op"], "createChat");
        assert_eq!(create.params["createdByChatId"], "main");
        assert_eq!(create.params["cwd"], "/tmp/main");
        assert_eq!(create.params["config"]["harness"], "codex");
        assert_eq!(create.params["config"]["model"], "child-model");
        replies
            .try_send(
                serde_json::to_string(&zeron_rpc::ServerFrame {
                    id: create.id,
                    ok: Some(serde_json::json!({})),
                    ..Default::default()
                })
                .unwrap(),
            )
            .unwrap();
        runtime.block_on(async { tokio::task::yield_now().await });
        cx.run_until_parked();
        let after: Vec<_> = drain().into_iter().filter(touches_side).collect();
        let called = |method: &str| after.iter().any(|f| f.method.as_deref() == Some(method));
        assert!(called(methods::WATCH_DOC_MESSAGES), "{after:?}");
        assert!(called(methods::QUEUE_COMMAND), "{after:?}");
        let run = after
            .iter()
            .find(|f| f.method.as_deref() == Some(methods::QUEUE_COMMAND))
            .unwrap();
        let request = &run.params["command"]["request"];
        for field in ["harness", "model", "reasoning", "modelOptions"] {
            assert_eq!(request[field], create.params["config"][field], "{field}");
        }
        assert_eq!(request["cwd"], "/tmp/main");
        assert_eq!(request["resume"], serde_json::Value::Null);
        assert!(!after.iter().any(|f| f.params["op"] == "createChat"));
        assert!(!side.read_with(cx, |state, _| state.side_chat_unsaved()));
        composer.update(cx, |composer, cx| {
            assert_eq!(
                composer.file_search_params("src", cx).unwrap()["chatId"],
                "side"
            );
            assert_eq!(composer.catalog_params(cx)["chatId"], "side");
            composer
                .input
                .update(cx, |input, cx| input.set_text("/", cx));
        });
        cx.run_until_parked();
        let catalogs: Vec<_> = drain()
            .into_iter()
            .filter(|frame| {
                matches!(
                    frame.method.as_deref(),
                    Some(methods::LIST_COMMANDS | methods::LIST_SKILLS)
                )
            })
            .collect();
        assert_eq!(catalogs.len(), 2);
        assert!(catalogs.iter().all(touches_side));
    }

    #[gpui::test]
    fn pasted_reference_resolution_is_one_undo_step_and_rejects_stale_results(
        cx: &mut gpui::TestAppContext,
    ) {
        with_composer_input(cx, |input, window, cx| {
            input.set_text("Before ", cx);
            cx.write_to_clipboard(ClipboardItem::new_string("@README.md".into()));
            input.paste(&Paste, window, cx);
            let original = input.text().to_owned();
            let revision = input.edit_revision;
            input.layout_text(px(400.0), &window.text_style(), window, cx);
            assert_eq!(
                input.edit_revision, revision,
                "layout must not invalidate discovery"
            );
            let replacement = vec![(7..original.len(), local_file_link("README.md", false))];
            input.apply_pasted_references(
                &original,
                original.len(),
                revision,
                replacement.clone(),
                cx,
            );
            assert_eq!(input.projection.mentions.len(), 1);
            let canonical = input.text().to_owned();
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "Before ");
            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), canonical);
            input.set_text(&original, cx);
            input.apply_pasted_references(&original, original.len(), revision, replacement, cx);
            assert_eq!(
                input.text(),
                original,
                "stale discovery cannot change a restored draft"
            );
        });
    }

    #[gpui::test]
    fn message_composer_still_copies_transcript_selection(cx: &mut gpui::TestAppContext) {
        let _selection = crate::markdown::selection::test_state_lock();
        with_composer_input(cx, |input, window, cx| {
            let key = "message-composer-copy-test";
            let text = "Selected transcript text";
            input.set_text("Unselected draft", cx);
            assert_eq!(input.key_context, MESSAGE_COMPOSER_CONTEXT);
            assert!(input.selected_range.is_empty());
            crate::markdown::selection::begin_with_span(key, text, 0..text.len());
            crate::markdown::selection::end_active_drag();
            input.copy(&Copy, window, cx);
            let copied = cx.read_from_clipboard().and_then(|item| item.text());
            // The question wizard borrows this input under the generic
            // context; it is still the message composer.
            cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
            input.set_key_context(message_input_context(true), cx);
            input.copy(&Copy, window, cx);
            let copied_in_wizard = cx.read_from_clipboard().and_then(|item| item.text());
            input.set_key_context(MESSAGE_COMPOSER_CONTEXT, cx);
            crate::markdown::selection::clear_if_owner(key);
            assert_eq!(copied.as_deref(), Some(text));
            assert_eq!(copied_in_wizard.as_deref(), Some(text));
        });
    }

    #[gpui::test]
    fn clipboard_is_readable_outside_zeron_and_lossless_inside(cx: &mut gpui::TestAppContext) {
        with_composer_input(cx, |input, window, cx| {
            let raw = format!(
                "**Check** {} with {}",
                local_file_link("src/café.rs", false),
                zeron_proto::invocation::Invocation::Skill {
                    name: "review".into(),
                    path: "/repo/SKILL.md".into(),
                    command: None
                }
                .link()
            );
            input.set_text(&raw, cx);
            input.select_all(&SelectAll, window, cx);
            input.copy(&Copy, window, cx);
            let copied = cx.read_from_clipboard().unwrap().text().unwrap();
            assert_eq!(
                copied,
                "**Check** [café.rs](src/caf%C3%A9.rs) with [$review](/repo/SKILL.md)"
            );
            input.set_text("", cx);
            input.paste(&Paste, window, cx);
            assert_eq!(input.text(), raw);
            assert_eq!(input.projection.mentions.len(), 2);
            input.set_text("x", cx);
            cx.write_to_clipboard(ClipboardItem::new_string("a".into()));
            input.paste(&Paste, window, cx);
            input.replace_text_in_range(None, "b", window, cx);
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "xa");
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "x");
        });
    }

    #[gpui::test]
    fn dense_rich_draft_keeps_offsets_and_reuses_unchanged_layout(cx: &mut gpui::TestAppContext) {
        with_composer_input(cx, |input, window, cx| {
            let raw = "- **café** _text_\n".repeat(1000) + "active";
            input.set_text(&raw, cx);
            input.layout_text(px(320.), &window.text_style(), window, cx);
            assert!(input.projection.display.starts_with("• café text\n"));
            assert_eq!(input.last_lines.len(), 1001);
            let mut previous = 0;
            for (offset, _) in raw.char_indices() {
                let display = input.projection.raw_to_display(offset);
                assert!(display >= previous && display <= input.projection.display.len());
                assert!(input.projection.display.is_char_boundary(display));
                previous = display;
            }
            for (offset, _) in input.projection.display.char_indices() {
                assert!(raw.is_char_boundary(input.projection.display_to_raw(offset)));
            }
            let rebuilt = input.layout_rebuilds;
            input.layout_text(px(320.), &window.text_style(), window, cx);
            assert_eq!(input.layout_rebuilds, rebuilt);
        });
    }

    #[gpui::test]
    fn rich_edit_state_is_current_before_the_next_layout(cx: &mut gpui::TestAppContext) {
        with_composer_input(cx, |input, window, cx| {
            let draft = "**first**\n_second_\nlast";
            input.set_text(draft, cx);
            assert_eq!(input.projection.display, "first\nsecond\nlast");
            input.replace_text_in_range(None, "\n**next**", window, cx);
            assert_eq!(input.projection.display, "first\nsecond\nlast\n**next**");
            input.undo(&Undo, window, cx);
            assert_eq!(input.projection.display, "first\nsecond\nlast");
            input.redo(&Redo, window, cx);
            assert_eq!(input.projection.display, "first\nsecond\nlast\n**next**");
        });
    }

    #[gpui::test]
    fn ime_uses_replacement_relative_utf16_and_restores_markdown_on_unmark(
        cx: &mut gpui::TestAppContext,
    ) {
        with_composer_input(cx, |input, window, cx| {
            let before = "**bold**\n😀x";
            input.set_text(before, cx);
            let start = before.len() - 1;
            input.selected_range = start..before.len();
            input.selection_reversed = true;
            input.replace_and_mark_text_in_range(None, "あいう", Some(1..2), window, cx);
            assert_eq!(input.selected_range, start + 3..start + 6);
            assert!(!input.selection_reversed);
            assert_eq!(&input.text()[input.selected_range.clone()], "い");
            assert!(
                input.projection.display.starts_with("bold\n"),
                "composition must not reveal unrelated syntax"
            );
            input.layout_text(px(320.), &window.text_style(), window, cx);
            assert!(!input.needs_measure);
            input.unmark_text(window, cx);
            assert!(input.needs_measure);
            assert!(input.projection.display.starts_with("bold\n"));
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), before);
            assert_eq!(input.selected_range, start..before.len());
            assert!(input.selection_reversed);
        });
    }

    #[gpui::test]
    fn bulk_edits_and_following_typing_are_separate_undo_steps(cx: &mut gpui::TestAppContext) {
        with_composer_input(cx, |input, window, cx| {
            input.set_text("", cx);
            input.replace_text_in_range(None, "paste", window, cx);
            input.replace_text_in_range(None, "d", window, cx);
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "paste");
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "");
            input.set_text("@a", cx);
            input.replace_mention(0..2, "src/a.rs", false, cx);
            let chip = input.text().to_owned();
            input.replace_text_in_range(None, "x", window, cx);
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), chip);
        });
    }

    #[gpui::test]
    fn inserting_references_preserves_surrounding_markdown_and_punctuation(
        cx: &mut gpui::TestAppContext,
    ) {
        with_composer_input(cx, |input, window, cx| {
            for raw in [
                "**see @src**",
                "_see @src_",
                "~~see @src~~",
                "See (@src)",
                "- @src\r\nnext",
            ] {
                let start = raw.find("@src").unwrap();
                let token = mention_token(raw, start + 4).unwrap();
                input.set_text(raw, cx);
                let file = local_file_link("src/main.rs", false);
                input.replace_mention(token.range, "src/main.rs", false, cx);
                assert_eq!(input.text(), raw.replacen("@src", &file, 1));
                assert_eq!(input.projection.mentions.len(), 1);
                input.undo(&Undo, window, cx);
                assert_eq!(input.text(), raw);
                let source = raw.replace("@src", "$review");
                input.set_text(&source, cx);
                let skill = zeron_proto::invocation::Invocation::Skill {
                    name: "review".into(),
                    path: "/repo/SKILL.md".into(),
                    command: None,
                }
                .link();
                let token = invocation_token(&source, start + 7, '$').unwrap();
                input.replace_plain_token(token.range, &skill, cx);
                assert_eq!(input.text(), source.replacen("$review", &skill, 1));
                assert_eq!(input.projection.mentions.len(), 1);
            }
            input.set_text("**selected**", cx);
            input.selected_range = 2..10;
            for outside in ["/tmp/file.rs", "../file.rs"] {
                assert!(!input.insert_dropped_mention(outside, false, cx));
            }
            assert!(input.insert_dropped_mention("src/main.rs", false, cx));
            assert_eq!(
                input.text(),
                format!("**{}**", local_file_link("src/main.rs", false))
            );
            assert!(
                composer_markdown::faces(input.text())
                    .iter()
                    .any(|(_, face)| *face == composer_markdown::Face::Bold)
            );
        });
    }

    #[gpui::test]
    fn completion_rejects_paths_that_cannot_round_trip_as_chips(cx: &mut gpui::TestAppContext) {
        let input = cx.new(|cx| ComposerInput::new("Draft", cx));
        input.update(cx, |input, cx| {
            input.enable_mentions();
            input.set_text("See @src", cx);
            for path in [
                "src/a\nb.rs",
                "src/a\rb.rs",
                "src/a\tb.rs",
                "",
                "../secret",
                "https://example.com",
            ] {
                input.replace_mention(4..8, path, false, cx);
                assert_eq!(input.text(), "See @src", "{path:?}");
                assert!(input.undo_stack.is_empty());
            }
            input.replace_mention(4..8, "src/café [draft].rs", false, cx);
            assert_eq!(input.projection.mentions.len(), 1);
            assert_eq!(input.projection.mentions[0].0.path, "src/café [draft].rs");
        });
    }

    #[gpui::test]
    fn quoted_list_indentation_preserves_containers_and_selection(cx: &mut gpui::TestAppContext) {
        with_composer_input(cx, |input, window, cx| {
            for original in ["> - café\n> - [x] done", "> > - café\r\n> > - [x] done"] {
                input.set_text(original, cx);
                input.selected_range = 0..original.len();
                input.selection_reversed = true;
                input.refresh_projection();
                assert!(input.indent_list(false, window, cx));
                let indented = original.replace("- ", "  - ");
                assert_eq!(input.text(), indented);
                assert_eq!(input.selected_range, 0..indented.len());
                assert!(input.selection_reversed);
                assert!(input.indent_list(true, window, cx));
                assert_eq!(input.text(), original);
                assert_eq!(input.selected_range, 0..original.len());
                assert!(input.selection_reversed);
                input.undo(&Undo, window, cx);
                assert_eq!(input.text(), indented);
                input.undo(&Undo, window, cx);
                assert_eq!(input.text(), original);
            }
            input.set_text("> ```\n> - literal\n> ```", cx);
            input.move_to(input.text().find("literal").unwrap(), cx);
            assert!(!input.indent_list(false, window, cx));
            input.set_text(
                ">     - literal code that wraps over several rows\n\nactive",
                cx,
            );
            input.layout_text(px(160.), &window.text_style(), window, cx);
            assert_eq!(input.line_indents[0], px(0.));
            assert!(input.projection.display.starts_with(">     - literal"));
        });
    }

    #[gpui::test]
    fn dropped_chip_replacement_rebuilds_same_length_layout(cx: &mut gpui::TestAppContext) {
        with_composer_input(cx, |input, window, cx| {
            input.set_text(local_file_link("src/a.rs", false), cx);
            input.layout_text(px(320.), &window.text_style(), window, cx);
            let rebuilds = input.layout_rebuilds;
            input.selected_range = 0..input.content.len();
            assert!(input.insert_dropped_mention("src/b.rs", false, cx));
            assert!(input.needs_measure);
            input.layout_text(px(320.), &window.text_style(), window, cx);
            assert_eq!(input.layout_rebuilds, rebuilds + 1);
            assert!(input.projection.display.contains("b.rs"));
            input.set_text("    - code\n\nactive", cx);
            input.layout_text(px(160.), &window.text_style(), window, cx);
            assert_eq!(input.line_indents[0], px(0.));
            input.move_to(7, cx);
            assert!(!input.indent_list(false, window, cx));
            assert_eq!(input.text(), "    - code\n\nactive");
        });
    }

    #[gpui::test]
    fn multiline_selection_exposes_all_selected_markdown_source(cx: &mut gpui::TestAppContext) {
        let input = cx.new(|cx| ComposerInput::new("Draft", cx));
        input.update(cx, |input, cx| {
            input.enable_mentions();
            input.set_text("**first**\n_second_\nactive", cx);
            input.selected_range = 2..18;
            input.refresh_projection();
            assert!(input.projection.display.starts_with("**first**\n_second_"));
            input.selection_reversed = true;
            input.refresh_projection();
            assert!(input.projection.display.starts_with("**first**\n_second_"));
            input.selected_range = input.content.len()..input.content.len();
            input.refresh_projection();
            assert!(input.projection.display.starts_with("first\nsecond"));
        });
    }

    #[test]
    fn rich_projection_keeps_unicode_offsets_and_atomic_invocations() {
        let invocation = zeron_proto::invocation::Invocation::Skill {
            command: None,
            name: "bla-bla:bla-bla".into(),
            path: "/repo/SKILL.md".into(),
        };
        let raw = format!("**café** {} end\nactive", invocation.link());
        let active = raw.rfind('\n').unwrap() + 1..raw.len();
        let projection = TextProjection::rich(&raw, Some(active), None);
        assert!(projection.display.starts_with("café"));
        assert!(projection.display.contains("$Bla\u{00A0}Bla"));
        assert_eq!(projection.mentions.len(), 1);
        let (link, display) = &projection.mentions[0];
        assert_eq!(projection.raw_to_display(link.range.start), display.start);
        assert_eq!(projection.display_to_raw(display.end), link.range.end);
        assert_eq!(
            projection.normalize_range(link.range.start + 1..link.range.end - 1),
            link.range
        );
        assert_eq!(
            projection.previous_boundary(link.range.end),
            Some(link.range.start)
        );
        let (sent, _) = sent_mention_display(&raw).unwrap();
        assert!(sent.contains("$bla-bla:bla-bla"));
        assert!(sent.starts_with("**café**"));
    }

    #[test]
    fn invocation_completion_rejects_code_paths_currency_and_escapes() {
        for text in [
            "cost $100",
            "echo $HOME",
            "set $Path",
            "word$review",
            "\\$review",
            "`$review",
            "```\n$review",
            "path/$review",
        ] {
            assert!(invocation_token(text, text.len(), '$').is_none(), "{text}");
        }
        for text in [
            "use $review",
            "- $review",
            "($review",
            "first\n$review",
            "hello\u{00a0}$review",
        ] {
            assert_eq!(
                invocation_token(text, text.len(), '$').unwrap().query,
                "review"
            );
        }
        assert!(slash_token("try /usr/bin", 12).is_none());
        assert!(mention_token("`@file", 6).is_none());
    }

    #[gpui::test]
    fn rich_editor_selection_lists_and_undo(cx: &mut gpui::TestAppContext) {
        with_composer_input(cx, |input, window, cx| {
            input.set_text("hello café world\n- item", cx);
            assert_eq!(input.selection_unit(PressIntent::Word, 8), 6..11);
            input.drag_unit = Some((PressIntent::Word, 6..11));
            input.drag_select_to(2, cx);
            assert_eq!(input.selected_range, 0..11);
            assert!(input.selection_reversed);
            input.drag_select_to(14, cx);
            assert_eq!(input.selected_range, 6..17);
            input.move_to(input.content.len(), cx);
            input.newline(&Newline, window, cx);
            assert!(input.content.ends_with("\n- "));
            input.undo(&Undo, window, cx);
            assert!(input.content.ends_with("\n- item"));
            input.redo(&Redo, window, cx);
            input.newline(&Newline, window, cx);
            assert!(input.content.ends_with("item\n"));
        });
    }

    #[gpui::test]
    fn rendered_word_selection_uses_pointer_hit_testing(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        let input = handle
            .read_with(cx, |composer, _| composer.input.clone())
            .unwrap();
        input.update(cx, |input, cx| input.set_text("hello café world", cx));
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            let position = {
                let editor = input.read(cx);
                let local = editor.point_for_index(8).unwrap();
                editor.last_bounds.unwrap().origin + local + point(px(1.0), px(5.0))
            };
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    click_count: 2,
                    ..Default::default()
                }),
                cx,
            );
            assert_eq!(input.read(cx).selected_range, 6..11);
            window.dispatch_event(
                gpui::PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    ..Default::default()
                }),
                cx,
            );
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    click_count: 3,
                    ..Default::default()
                }),
                cx,
            );
            assert_eq!(input.read(cx).selected_range, 0..17);
        })
        .unwrap();
    }

    #[test]
    fn multi_click_selects_words_and_lines() {
        assert_eq!(press_intent(1, false), PressIntent::PlaceCaret);
        assert_eq!(press_intent(1, true), PressIntent::ExtendSelection);
        assert_eq!(press_intent(2, false), PressIntent::Word);
        assert_eq!(press_intent(3, false), PressIntent::Line);
        assert_eq!(word_range("hello café world", 8), 6..11);
        assert_eq!(word_range("hello world", 3), 0..5);
    }

    #[test]
    fn message_enter_bindings_cover_both_platform_modifiers() {
        for modifier in ["cmd-enter", "ctrl-enter"] {
            for (behavior, action) in [
                (
                    ComposerSendBehavior::Enter,
                    MessageEnterBindingAction::Submit,
                ),
                (
                    ComposerSendBehavior::ModEnter,
                    MessageEnterBindingAction::NewlineOrAccept,
                ),
            ] {
                assert_eq!(
                    message_enter_bindings(behavior, modifier),
                    vec![
                        MessageEnterBinding {
                            keystroke: "enter".into(),
                            action
                        },
                        MessageEnterBinding {
                            keystroke: modifier.into(),
                            action: MessageEnterBindingAction::ModifiedSubmit,
                        },
                    ]
                );
            }
        }
    }

    #[test]
    fn enter_accepts_a_completion_before_submit_or_newline() {
        assert_eq!(
            enter_outcome(true, EnterOutcome::Submit),
            EnterOutcome::AcceptCompletion
        );
        assert_eq!(
            enter_outcome(true, EnterOutcome::Newline),
            EnterOutcome::AcceptCompletion
        );
        assert_eq!(
            enter_outcome(false, EnterOutcome::Submit),
            EnterOutcome::Submit
        );
        assert_eq!(
            enter_outcome(false, EnterOutcome::Newline),
            EnterOutcome::Newline
        );
    }

    #[test]
    fn wizard_borrows_the_generic_enter_context_only_while_active() {
        assert_eq!(message_input_context(false), MESSAGE_COMPOSER_CONTEXT);
        assert_eq!(message_input_context(true), GENERIC_COMPOSER_CONTEXT);
    }

    #[test]
    fn stable_outer_width_only_schedules_reflow_on_real_changes() {
        assert!(composer_width_changed(None, 400.0));
        assert!(!composer_width_changed(Some(400.0), 400.0));
        assert!(!composer_width_changed(Some(400.0), 400.5));
        assert!(composer_width_changed(Some(400.0), 400.51));
    }

    fn tooltip_target(range: Range<usize>, path: &str) -> MentionTooltipTarget {
        MentionTooltipTarget {
            range,
            path: path.into(),
            attachment: None,
        }
    }

    #[test]
    fn mention_tooltip_wait_survives_pointer_jitter_and_promotes_once() {
        let target = tooltip_target(3..20, "src/composer.rs");
        let waiting = MentionTooltipPhase::Waiting {
            target: target.clone(),
            generation: 1,
        };
        let restarted = mention_tooltip_reduce(waiting.clone(), Some(target.clone()), false, 2);
        assert_eq!(restarted, waiting);
        assert!(matches!(
            restarted,
            MentionTooltipPhase::Waiting { generation: 1, .. }
        ));
        assert_eq!(
            mention_tooltip_promote(restarted.clone(), 2, true),
            restarted,
            "a stale timer must not reveal the tooltip"
        );
        let visible = mention_tooltip_promote(restarted, 1, true);
        assert!(matches!(
            visible,
            MentionTooltipPhase::Visible { generation: 1, .. }
        ));
        assert_eq!(
            mention_tooltip_reduce(visible.clone(), Some(target), false, 3),
            visible,
            "one visible activation keeps its presentation generation stable"
        );
    }

    #[test]
    fn mention_tooltip_changes_target_and_cancels_disappeared_target() {
        let first = tooltip_target(0..10, "src/a.rs");
        let second = tooltip_target(20..30, "src/a.rs");
        let visible = MentionTooltipPhase::Visible {
            target: first,
            generation: 4,
        };
        assert!(matches!(
            mention_tooltip_reduce(visible, Some(second), false, 5),
            MentionTooltipPhase::Waiting { generation: 5, .. }
        ));
        assert_eq!(
            mention_tooltip_promote(
                MentionTooltipPhase::Waiting {
                    target: tooltip_target(20..30, "src/a.rs"),
                    generation: 5,
                },
                5,
                false,
            ),
            MentionTooltipPhase::Hidden
        );
    }

    #[test]
    fn mention_tooltip_stays_visible_over_chip_or_popup_only() {
        assert!(mention_tooltip_contains(true, false));
        assert!(mention_tooltip_contains(false, true));
        assert!(!mention_tooltip_contains(false, false));
    }

    #[test]
    fn mention_wash_moves_wholly_to_the_next_visual_row_at_a_wrap() {
        assert_eq!(
            display_row_segments(12..24, [12, 40]),
            vec![(1, 12, 12..24)]
        );
        assert_eq!(
            display_row_segments(8..24, [12, 40]),
            vec![(0, 0, 8..12), (1, 12, 12..24)]
        );
    }

    #[test]
    fn mention_token_requires_a_token_boundary_and_tracks_full_token() {
        assert_eq!(
            mention_token("Fix @src/com", 12),
            Some(MentionToken {
                range: 4..12,
                query: "src/com".into(),
            })
        );
        assert!(mention_token("mail@example.com", 16).is_none());
        assert!(mention_token("word@file", 9).is_none());
        assert!(mention_token("path/@file", 10).is_none());
        assert_eq!(
            mention_token("See (@lib", 9).map(|token| token.range),
            Some(5..9)
        );
    }

    #[gpui::test]
    fn the_first_slash_opens_on_a_prefetched_catalog(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _runtime = runtime.enter();
        let directory = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            crate::settings::init(crate::settings::UiSettings::default(), directory.path(), cx);
        });
        let (out, mut requests) = tokio::sync::mpsc::channel(64);
        let (replies, inbound) = tokio::sync::mpsc::channel(64);
        let state = cx.new(|_| AppState::new());
        let harness = crate::settings::SKILL_COMPLETION_HARNESSES[0].0;
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(
                zeron_rpc::RpcClient::new(out, inbound),
            ));
            state.chats = vec![
                serde_json::from_value(serde_json::json!({
                    "id": "chat", "deviceId": "host", "archived": false,
                    "cwd": "/checkout", "createdAt": chrono::Utc::now(),
                    "config": { "harness": harness, "sandbox": "workspace-write" }
                }))
                .unwrap(),
            ];
            state.selected_chat = Some("chat".into());
        });
        let composer = cx.new(|cx| Composer::new(state.clone(), cx));
        let catalog_frames = |requests: &mut tokio::sync::mpsc::Receiver<String>| {
            let mut frames = Vec::new();
            while let Ok(frame) = requests.try_recv() {
                let frame: zeron_rpc::ClientFrame = serde_json::from_str(&frame).unwrap();
                if matches!(
                    frame.method.as_deref(),
                    Some(methods::LIST_COMMANDS | methods::LIST_SKILLS)
                ) {
                    frames.push(frame);
                }
            }
            frames
        };
        state.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert!(catalog_frames(&mut requests).is_empty(), "debounced");
        cx.executor().advance_clock(SLASH_PREFETCH_DELAY);
        cx.run_until_parked();
        let frames = catalog_frames(&mut requests);
        assert!(!frames.is_empty());
        for frame in frames {
            let value = if frame.method.as_deref() == Some(methods::LIST_COMMANDS) {
                serde_json::json!([{ "name": "review", "description": "Review" }])
            } else {
                serde_json::json!([])
            };
            replies
                .try_send(
                    serde_json::to_string(&zeron_rpc::ServerFrame {
                        id: frame.id,
                        ok: Some(value),
                        ..Default::default()
                    })
                    .unwrap(),
                )
                .unwrap();
        }
        runtime.block_on(async { tokio::task::yield_now().await });
        cx.run_until_parked();
        composer.update(cx, |composer, cx| {
            composer
                .input
                .update(cx, |input, cx| input.set_text("/", cx));
        });
        composer.read_with(cx, |composer, _| {
            let rows = composer.slash_cache.get(&composer.slash.context).unwrap();
            assert!(
                composer
                    .slash
                    .filtered
                    .iter()
                    .any(|&index| rows[index].name == "review"),
                "rows are there before the refresh answers"
            );
        });
    }

    #[gpui::test]
    fn delayed_catalogs_cannot_replace_current_harness_or_preferences(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _runtime = runtime.enter();
        let directory = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            crate::settings::init(crate::settings::UiSettings::default(), directory.path(), cx);
        });
        let (out, mut requests) = tokio::sync::mpsc::channel(64);
        let (replies, inbound) = tokio::sync::mpsc::channel(64);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(
                zeron_rpc::RpcClient::new(out, inbound),
            ));
            state.chats = crate::settings::SKILL_COMPLETION_HARNESSES
                .iter()
                .enumerate()
                .map(|(index, (harness, _))| {
                    serde_json::from_value(serde_json::json!({
                        "id": format!("chat-{index}"), "deviceId": "host", "archived": false,
                        "cwd": format!("/worktree-{index}"), "createdAt": chrono::Utc::now(),
                        "config": { "harness": harness, "sandbox": "workspace-write" }
                    }))
                    .unwrap()
                })
                .collect();
        });
        let composer = cx.new(|cx| Composer::new(state.clone(), cx));
        let mut delayed = Vec::new();
        for (index, (harness, _)) in crate::settings::SKILL_COMPLETION_HARNESSES
            .iter()
            .enumerate()
        {
            state.update(cx, |state, cx| {
                state.selected_chat = Some(format!("chat-{index}"));
                cx.notify();
            });
            cx.run_until_parked();
            composer.update(cx, |composer, cx| {
                composer
                    .input
                    .update(cx, |input, cx| input.set_text("/", cx));
            });
            cx.run_until_parked();
            let mut batch = Vec::new();
            while let Ok(frame) = requests.try_recv() {
                let frame: zeron_rpc::ClientFrame = serde_json::from_str(&frame).unwrap();
                if matches!(
                    frame.method.as_deref(),
                    Some(methods::LIST_COMMANDS | methods::LIST_SKILLS)
                ) {
                    assert_eq!(
                        frame.params["harness"],
                        serde_json::to_value(harness).unwrap()
                    );
                    assert_eq!(frame.params["cwd"], format!("/worktree-{index}"));
                    batch.push(frame);
                }
            }
            assert_eq!(batch.len(), 2, "catalog requests for {harness:?}");
            delayed.extend(batch);
        }
        // Change the active checkout and slash separation while its original
        // catalog is still in flight. The token itself remains unchanged.
        state.update(cx, |state, cx| {
            state.chats.last_mut().unwrap().cwd = Some("/current-checkout".into());
            cx.notify();
        });
        cx.update(|cx| {
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                settings.skill_completion_by_harness.insert(
                    HarnessId::Opencode,
                    crate::settings::SkillCompletionSettings {
                        dollar: true,
                        separate_from_slash: true,
                    },
                );
            });
        });
        cx.run_until_parked();
        let mut current = Vec::new();
        while let Ok(frame) = requests.try_recv() {
            let frame: zeron_rpc::ClientFrame = serde_json::from_str(&frame).unwrap();
            if matches!(
                frame.method.as_deref(),
                Some(methods::LIST_COMMANDS | methods::LIST_SKILLS)
            ) {
                assert_eq!(frame.params["cwd"], "/current-checkout");
                current.push(frame);
            }
        }
        assert_eq!(current.len(), 2);
        let respond = |frames: Vec<zeron_rpc::ClientFrame>, name: &str| {
            for frame in frames {
                let value = if frame.method.as_deref() == Some(methods::LIST_COMMANDS) {
                    serde_json::json!([{ "name": name, "description": "Provider command" }])
                } else {
                    serde_json::json!([{ "name": format!("{name}-skill"), "path": "/skills/SKILL.md", "description": "Skill", "enabled": true }])
                };
                replies
                    .try_send(
                        serde_json::to_string(&zeron_rpc::ServerFrame {
                            id: frame.id,
                            ok: Some(value),
                            ..Default::default()
                        })
                        .unwrap(),
                    )
                    .unwrap();
            }
            runtime.block_on(async { tokio::task::yield_now().await });
        };
        respond(current, "current-command");
        cx.run_until_parked();
        let visible_names = |composer: &Composer| {
            let rows = composer.slash_cache.get(&composer.slash.context).unwrap();
            composer
                .slash
                .filtered
                .iter()
                .map(|&index| rows[index].name.clone())
                .collect::<Vec<_>>()
        };
        let current_names = composer.read_with(cx, |composer, _| {
            assert!(!composer.slash.loading);
            let names = visible_names(composer);
            assert!(names.iter().any(|name| name == "current-command"));
            assert!(
                !names.iter().any(|name| name.ends_with("-skill")),
                "separated slash menu must exclude skills"
            );
            names
        });
        // Every abandoned harness now replies after the visible current result.
        delayed.reverse();
        respond(delayed, "stale-command");
        cx.run_until_parked();
        composer.read_with(cx, |composer, _| {
            assert_eq!(visible_names(composer), current_names);
            assert_eq!(composer.slash.harness, Some(HarnessId::Opencode));
        });

        // Acceptance, ordinary prose and changing trigger characters must all
        // retain catalogs scoped to this same device/harness/checkout.
        composer.update(cx, |composer, cx| composer.update_slash("$", 1, cx));
        cx.run_until_parked();
        let mut skill_requests = Vec::new();
        while let Ok(frame) = requests.try_recv() {
            let frame: zeron_rpc::ClientFrame = serde_json::from_str(&frame).unwrap();
            if matches!(
                frame.method.as_deref(),
                Some(methods::LIST_COMMANDS | methods::LIST_SKILLS)
            ) {
                assert_eq!(frame.method.as_deref(), Some(methods::LIST_SKILLS));
                skill_requests.push(frame);
            }
        }
        assert_eq!(skill_requests.len(), 1);
        respond(skill_requests, "current");
        cx.run_until_parked();
        let mut names = ["current-skill".to_string(), "current-command".to_string()];
        for (index, token) in ["$", "/", "$", "/"].into_iter().enumerate() {
            let kind = usize::from(token == "/");
            composer.update(cx, |composer, cx| {
                composer.reset_slash(None, cx);
                composer.update_slash("ordinary prose ", 15, cx);
                composer.update_slash(token, token.len(), cx);
                assert!(composer.slash.loading, "each open refreshes the catalog");
                assert!(
                    visible_names(composer).contains(&names[kind]),
                    "warm rows stay visible"
                );
                let query = format!("{token}refreshed");
                composer.update_slash(&query, query.len(), cx);
            });
            cx.run_until_parked();
            let mut refresh = Vec::new();
            while let Ok(frame) = requests.try_recv() {
                let frame: zeron_rpc::ClientFrame = serde_json::from_str(&frame).unwrap();
                if matches!(
                    frame.method.as_deref(),
                    Some(methods::LIST_COMMANDS | methods::LIST_SKILLS)
                ) {
                    refresh.push(frame);
                }
            }
            assert_eq!(
                refresh.len(),
                if token == "$" { 1 } else { 2 },
                "typing shares the open's request"
            );
            let fresh = format!("refreshed-{index}");
            respond(refresh, &fresh);
            cx.run_until_parked();
            let expected = if token == "$" {
                format!("{fresh}-skill")
            } else {
                fresh
            };
            composer.read_with(cx, |composer, _| {
                assert!(!composer.slash.loading);
                assert_eq!(visible_names(composer), [expected.clone()]);
                assert!(
                    !composer.slash_cache[&composer.slash.context]
                        .iter()
                        .any(|row| row.name == names[kind]),
                    "removed catalog entries must disappear"
                );
            });
            names[kind] = expected;
        }
    }

    #[gpui::test]
    fn reconnect_reconsiders_unchanged_completion_tokens(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _runtime = runtime.enter();
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, _, cx| {
                composer
                    .input
                    .update(cx, |input, cx| input.set_text("/model", cx));
                composer.on_input_edited(cx);
                let old_context = composer.slash.context.clone();
                let old_request = composer.slash.request;
                composer.state.update(cx, |state, _| {
                    state.connection = crate::state::ConnectionStatus::Ready
                });
                composer.on_state_changed(cx);
                assert_ne!(composer.slash.context, old_context);
                assert!(composer.slash.request > old_request);
                assert!(composer.slash.token.is_some());
                let context = composer.completion_connection_context(cx);
                let (out, _server) = tokio::sync::mpsc::channel(4);
                let (_incoming, inbound) = tokio::sync::mpsc::channel(4);
                composer.state.update(cx, |state, _| {
                    state.set_test_engine(crate::state::EngineHandle::from_test_client(
                        zeron_rpc::RpcClient::new(out, inbound),
                    ))
                });
                assert_ne!(composer.completion_connection_context(cx), context);
            })
            .unwrap();
    }

    #[gpui::test]
    fn explicit_navigation_breaks_undo_runs_but_backspace_remains_coalesced(
        cx: &mut gpui::TestAppContext,
    ) {
        with_composer_input(cx, |input, window, cx| {
            input.set_text("", cx);
            input.replace_text_in_range(None, "a", window, cx);
            input.move_to(0, cx);
            input.move_to(1, cx);
            input.replace_text_in_range(None, "b", window, cx);
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "a");
            input.set_text("abcd", cx);
            input.backspace(&Backspace, window, cx);
            input.backspace(&Backspace, window, cx);
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "abcd");
        });
    }

    #[gpui::test]
    fn unmarking_ime_reopens_completion_without_another_keystroke(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, window, cx| {
                composer.input.update(cx, |input, cx| {
                    input.set_text("", cx);
                    input.replace_and_mark_text_in_range(None, "/model", None, window, cx);
                });
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |composer, window, cx| {
                assert!(composer.slash.token.is_none());
                composer
                    .input
                    .update(cx, |input, cx| input.unmark_text(window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .read_with(cx, |composer, _| assert!(composer.slash.token.is_some()))
            .unwrap();
    }

    #[gpui::test]
    fn copying_part_of_a_literal_example_preserves_its_source(cx: &mut gpui::TestAppContext) {
        let input = cx.new(|cx| ComposerInput::new("Draft", cx));
        input.update(cx, |input, cx| {
            input.enable_mentions();
            for reference in [
                local_file_link("src/a.rs", false),
                zeron_proto::invocation::Invocation::Command {
                    name: "review".into(),
                }
                .link(),
            ] {
                for document in [
                    format!("`{reference}`"),
                    format!("```\n{reference}\n```"),
                    format!("![example {reference}](image.png)"),
                ] {
                    input.set_text(&document, cx);
                    let start = document.find(&reference).unwrap();
                    input.selected_range = start..start + reference.len();
                    assert_eq!(
                        input.clipboard_selection(),
                        Some((reference.clone(), reference.clone()))
                    );
                }
            }
        });
    }

    #[gpui::test]
    fn changing_slash_skill_preference_invalidates_open_completion(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, _, cx| {
                composer
                    .input
                    .update(cx, |input, cx| input.set_text("/review", cx));
                composer.on_input_edited(cx);
                composer
                    .slash_cache
                    .insert(composer.slash.context.clone(), vec![]);
            })
            .unwrap();
        let (context, request) = handle
            .read_with(cx, |composer, _| {
                (composer.slash.context.clone(), composer.slash.request)
            })
            .unwrap();
        cx.update(|cx| {
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |s| {
                s.skills_in_slash_menu = true
            });
        });
        cx.run_until_parked();
        handle
            .read_with(cx, |composer, _| {
                assert_ne!(composer.slash.context, context);
                assert!(
                    composer.slash.request > request,
                    "late responses must be rejected"
                );
                assert!(!composer.slash_cache.contains_key(&context));
                assert!(
                    composer
                        .slash_cache
                        .values()
                        .flatten()
                        .all(|row| row.workspace_command.is_some())
                );
                assert_eq!(composer.slash.token.as_ref().unwrap().query, "review");
            })
            .unwrap();
    }

    #[gpui::test]
    fn unchanged_mention_refreshes_when_its_workspace_changes(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, _, cx| {
                composer
                    .input
                    .update(cx, |input, cx| input.set_text("@src", cx));
                composer.on_input_edited(cx);
                composer.mention.context = "departing-worktree".into();
                composer.mention.token = mention_token("@src", 4);
                let old_request = composer.mention.request;
                composer.on_input_edited(cx);
                assert_ne!(composer.mention.context, "departing-worktree");
                assert!(composer.mention.request > old_request);
                assert!(!mention_response_is_current(&composer.mention, old_request));
                let context = composer.mention.context.clone();
                composer.reset_mention(Some((0..4, "@src".into())), cx);
                assert_eq!(composer.mention.context, context);
                composer.on_input_edited(cx);
                assert!(
                    composer.mention.token.is_none(),
                    "Escape must stay dismissed in the same workspace"
                );
            })
            .unwrap();
    }

    fn completion_token(text: &str, cursor: usize, prefix: char) -> Option<MentionToken> {
        if prefix == '@' {
            mention_token(text, cursor)
        } else {
            invocation_token(text, cursor, prefix)
        }
    }

    #[test]
    fn completion_respects_markdown_containers_and_destinations() {
        for prefix in ['@', '$', '/'] {
            let token = |text: &str, cursor| completion_token(text, cursor, prefix);
            for container in [">", ">>", "> >", "- >", "> first\n>"] {
                let text = format!("{container}{prefix}review");
                let found = token(&text, text.len()).unwrap();
                assert_eq!(found.range, container.len()..text.len(), "{text:?}");
            }
            for literal in ["comparison >", "> comparison >", "    >", "\\>"] {
                let text = format!("{literal}{prefix}review");
                assert!(token(&text, text.len()).is_none(), "{text:?}");
            }
            for link in [
                format!("[label]({prefix}review)"),
                format!("[label]({prefix}review"),
                format!("![label]({prefix}review)"),
                format!("[{prefix}review](https://example.com)"),
            ] {
                let cursor = link.find("review").unwrap() + "review".len();
                assert!(token(&link, cursor).is_none(), "{link:?}");
            }
            let after_link = format!("[label](url) ({prefix}review");
            assert!(token(&after_link, after_link.len()).is_some());
        }
        let canonical = zeron_proto::invocation::Invocation::Command {
            name: "review".into(),
        }
        .link();
        assert!(slash_token(&canonical, canonical.find("review").unwrap() + 6).is_none());
    }

    #[test]
    fn completion_keeps_multiline_and_reference_destinations_literal() {
        for prefix in ['@', '$', '/'] {
            let token = |text: &str, cursor| completion_token(text, cursor, prefix);
            for source in [
                format!("[label](\n{prefix}review"),
                format!("[label](foo\n {prefix}review"),
                format!("[label](url \"title\n {prefix}review"),
                format!("[id]: {prefix}review"),
                format!("[id]:\n  {prefix}review"),
                format!("> [id]: {prefix}review"),
                format!("[id]: url\n[id]: {prefix}review"),
                format!("[id]: url\n[id]:\n  {prefix}review"),
                format!("[outer]([inner\\]({prefix}review"),
                format!("<!-- {prefix}review -->"),
            ] {
                let cursor = source.find("review").unwrap() + "review".len();
                assert!(token(&source, cursor).is_none(), "{source:?}");
            }
            for source in [
                format!("[label](unfinished\n\n{prefix}review"),
                format!("[label](unfinished\n- {prefix}review"),
                format!("[id]: url\n\n{prefix}review"),
                format!("[label\\]({prefix}review"),
                format!("[label](url)\n{prefix}review"),
                format!("- first\n- {prefix}review"),
                format!("- first\n\n- {prefix}review"),
                format!("- first\n  - {prefix}review"),
                format!("> - {prefix}review"),
                prefix.to_string(),
            ] {
                assert!(token(&source, source.len()).is_some(), "{source:?}");
            }
        }
    }

    #[test]
    fn completion_closes_at_parsed_emphasis_delimiters() {
        for prefix in ['@', '$', '/'] {
            let token = |text: &str, cursor| completion_token(text, cursor, prefix);
            for delimiter in ["_", "**", "***", "~~"] {
                let source = format!("{delimiter}see {prefix}review{delimiter}");
                let closing = source.len() - delimiter.len();
                assert_eq!(token(&source, closing).unwrap().range.end, closing);
                for cursor in closing + 1..=source.len() {
                    assert!(token(&source, cursor).is_none(), "{source:?}@{cursor}");
                }
            }
        }
    }

    #[test]
    fn invocation_tokens_preserve_unicode_graphemes() {
        for name in ["réview", "re\u{301}view", "確認", "レビュー"] {
            for prefix in ['$', '/'] {
                let text = format!("please {prefix}{name}");
                let token = invocation_token(&text, text.len(), prefix).unwrap();
                assert_eq!(token.query, name);
                assert_eq!(&text[token.range], format!("{prefix}{name}"));
            }
        }
        let text = "$re\u{301}view";
        let token = invocation_token(text, 3, '$').unwrap();
        assert_eq!(token.query, "re");
        assert_eq!(token.range, 0..text.len());
        for text in ["$🧑", "$٣", "$12", "$re/view", "$\u{301}"] {
            assert!(
                invocation_token(text, text.len(), '$').is_none(),
                "{text:?}"
            );
        }
        assert!(invocation_token("$é", 2, '$').is_none());
    }

    #[test]
    fn file_completion_retains_parsed_emphasis_closing_delimiters() {
        for (text, suffix) in [
            ("**see @src**", "**"),
            ("_see @src_", "_"),
            ("~~see @src~~", "~~"),
            ("***see @src***", "***"),
        ] {
            let cursor = text.find("@src").unwrap() + 4;
            let token = mention_token(text, cursor).unwrap();
            assert_eq!(&text[token.range.end..], suffix, "{text:?}");
            assert_eq!(&text[token.range], "@src");
        }
        assert_eq!(
            mention_token("@file*name.rs", 13).unwrap().query,
            "file*name.rs"
        );
    }

    #[test]
    fn file_completion_preserves_surrounding_delimiters() {
        for (text, end) in [("(@src)", 5), ("[@src]", 5), ("{@src}", 5)] {
            let token = mention_token(text, end).unwrap();
            assert_eq!(token.range, 1..end);
            assert_eq!(token.query, "src");
            assert!(mention_token(text, text.len()).is_none());
        }
        assert_eq!(mention_token("@src/(draft).rs", 15).unwrap().range, 0..15);
        assert!(mention_token("\\@src", 5).is_none());
    }

    #[test]
    fn legacy_host_commands_remain_literal_and_saved_references_need_an_update() {
        use zeron_proto::invocation::Invocation;
        let command = Invocation::Command {
            name: "compact".into(),
        };
        assert_eq!(invocation_insertion(&command, false), "/compact");
        assert_eq!(invocation_insertion(&command, true), command.link());
        let skill = Invocation::Skill {
            name: "review".into(),
            path: "/repo/SKILL.md".into(),
            command: None,
        };
        for reference in [
            command.link(),
            skill.link(),
            local_file_link("src/lib.rs", false),
        ] {
            assert!(references_require_update(&reference, false));
            assert!(!references_require_update(&reference, true));
            for literal in [
                format!("`{reference}`"),
                format!("    {reference}"),
                format!("\\{reference}"),
            ] {
                assert!(!references_require_update(&literal, false));
            }
        }
        assert!(!references_require_update("/compact", false));
    }

    #[gpui::test]
    fn unsupported_host_preserves_reference_drafts_and_queue_edits(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, _, cx| {
                let draft = zeron_proto::invocation::Invocation::Command {
                    name: "compact".into(),
                }
                .link();
                composer
                    .input
                    .update(cx, |input, cx| input.set_text(draft.clone(), cx));
                let attachment = attachments::stage_png_bytes("draft.png".into(), Vec::new());
                let attachment_id = attachment.id.clone();
                composer
                    .attachments
                    .insert(composer.current_key.clone(), vec![attachment]);
                composer.state.update(cx, |state, _| {
                    state.selected_device = Some("peer".into());
                    state.devices =
                        vec![serde_json::from_value(serde_json::json!({
                    "id": "peer", "name": "Peer", "platform": "linux", "capabilities": []
                })).unwrap()];
                });
                assert!(!composer.reference_delivery_supported(cx));
                composer.send(draft.clone(), false, false, false, cx);
                assert!(
                    composer
                        .failure
                        .as_deref()
                        .unwrap()
                        .contains("Update the selected device")
                );
                assert_eq!(composer.input.read(cx).text(), draft);
                assert_eq!(composer.staged()[0].id, attachment_id);
                composer.editing_queued = Some("queued-draft".into());
                assert!(composer.commit_queue_edit(cx));
                assert_eq!(composer.editing_queued.as_deref(), Some("queued-draft"));
                assert!(!composer.queue_edit_finishing);
                assert_eq!(composer.input.read(cx).text(), draft);
                composer.state.update(cx, |state, _| {
                    state.devices[0]
                        .capabilities
                        .push(capabilities::COMPOSER_REFERENCES_V1.into());
                });
                assert!(composer.reference_delivery_supported(cx));
                assert!(composer.check_reference_delivery(&draft, cx));
            })
            .unwrap();
    }

    #[test]
    fn partial_discovery_keeps_commands_and_skills_independently() {
        let command = SlashCommand {
            name: "review".into(),
            description: String::new(),
            input_hint: None,
            kind: Default::default(),
        };
        let skill = zeron_proto::invocation::Skill {
            command: None,
            name: "review".into(),
            path: "/repo/SKILL.md".into(),
            description: String::new(),
            enabled: true,
        };
        let (rows, supported, warning) = merge_invocation_results(
            Ok(vec![command]),
            Err(RpcError::UnknownMethod("ListSkills".into())),
            false,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].invocation.prefix(), '/');
        assert!(supported && warning.is_some());
        assert_eq!(invocation_insertion(&rows[0].invocation, false), "/review");
        let (rows, _, warning) = merge_invocation_results(
            Err(RpcError::Failed("commands unavailable".into())),
            Ok(Some(vec![skill])),
            false,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].invocation.prefix(), '$');
        assert!(warning.is_some());
        assert!(merge_invocation_results(Ok(vec![]), Err(RpcError::Closed), true).is_err());
        let (rows, supported, warning) =
            merge_invocation_results(Ok(vec![]), Ok(None), true).unwrap();
        assert!(rows.is_empty() && !supported && warning.is_none());
    }

    #[gpui::test]
    fn commands_complete_inline_and_partial_discovery_retries(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, _, cx| {
                for text in [
                    "https://example/review",
                    "/repo/review/",
                    "`/review",
                    "```\n/review",
                ] {
                    composer.update_slash(text, text.len(), cx);
                    assert!(composer.slash.token.is_none(), "{text}");
                }
                for text in [
                    "/review",
                    "  /review",
                    "please $review",
                    "please /review",
                    "(/review",
                    "first\n/review",
                ] {
                    composer.update_slash(text, text.len(), cx);
                    assert!(composer.slash.token.is_some(), "{text}");
                }
                composer
                    .slash_cache
                    .insert(composer.slash.context.clone(), vec![]);
                composer.slash.error = Some("Skills unavailable".into());
                composer.reset_slash(None, cx);
                assert!(composer.slash_cache.is_empty());
            })
            .unwrap();
    }

    #[test]
    fn every_harness_respects_both_skill_completion_toggles() {
        use crate::settings::SkillCompletionSettings;
        for (harness, _) in crate::settings::SKILL_COMPLETION_HARNESSES {
            for dollar in [false, true] {
                for separate_from_slash in [false, true] {
                    let preferences = SkillCompletionSettings {
                        dollar,
                        separate_from_slash,
                    };
                    let (token, skill, include, _) = completion_trigger("$review", 7, preferences);
                    assert_eq!(token.is_some(), dollar, "{harness:?}");
                    assert_eq!(skill, dollar);
                    assert_eq!(include, dollar || !separate_from_slash);
                    let (token, skill, include, commands) =
                        completion_trigger("/review", 7, preferences);
                    assert!(token.is_some() && !skill && commands);
                    assert_eq!(include, !separate_from_slash);
                    let (token, _, _, commands) =
                        completion_trigger("please /review", 14, preferences);
                    assert!(token.is_some() && commands);
                }
            }
        }
    }

    #[test]
    fn every_harness_catalog_only_offers_round_trippable_references() {
        use zeron_proto::invocation::{Skill, SkillCommand, invocation_links};
        for (harness, _) in crate::settings::SKILL_COMPLETION_HARNESSES {
            let commands = ["review", "bad\ncommand", "two words"]
                .into_iter()
                .map(|name| SlashCommand {
                    name: name.into(),
                    description: String::new(),
                    input_hint: None,
                    kind: Default::default(),
                })
                .collect();
            let skills = [
                ("审查-é", "/repo/skill dir/SKILL.md", None),
                ("bad name", "/repo/SKILL.md", None),
                ("review", "/repo/bad\npath/SKILL.md", Some("review")),
                ("broken-native", "harness-skill:probe", Some("bad command")),
            ]
            .into_iter()
            .map(|(name, path, command)| Skill {
                name: name.into(),
                path: path.into(),
                description: String::new(),
                enabled: true,
                command: command.map(|name| SkillCommand {
                    name: name.into(),
                    harness,
                }),
            })
            .collect();
            let rows = invocation_candidates(commands, skills);
            assert_eq!(
                rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
                ["review", "审查-é"],
                "{harness:?}"
            );
            for row in rows {
                assert_eq!(
                    invocation_links(&row.invocation.link())[0].1,
                    row.invocation
                );
            }
        }
    }

    #[test]
    fn separated_native_skills_are_not_left_in_the_command_catalog() {
        use zeron_proto::invocation::{Skill, SkillCommand};
        for (harness, _) in crate::settings::SKILL_COMPLETION_HARNESSES {
            let commands = vec![
                SlashCommand {
                    name: "review".into(),
                    description: String::new(),
                    input_hint: None,
                    kind: Default::default(),
                },
                SlashCommand {
                    name: "compact".into(),
                    description: String::new(),
                    input_hint: None,
                    kind: Default::default(),
                },
            ];
            let skills = vec![Skill {
                name: "review".into(),
                path: "/repo/SKILL.md".into(),
                description: String::new(),
                enabled: true,
                command: Some(SkillCommand {
                    name: "review".into(),
                    harness,
                }),
            }];
            let rows = invocation_candidates(commands, skills);
            assert_eq!(rows.len(), 2);
            assert_eq!(
                rows.iter()
                    .filter(|row| row.invocation.prefix() == '/')
                    .map(|row| row.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["compact"]
            );
            assert_eq!(rows[1].invocation.prefix(), '$');
        }
    }

    #[test]
    fn combined_invocations_preserve_skill_identity_and_command_collisions() {
        use zeron_proto::invocation::{Invocation, Skill};
        let commands = vec![SlashCommand {
            name: "review".into(),
            description: "Command".into(),
            input_hint: None,
            kind: Default::default(),
        }];
        let skills = vec![
            Skill {
                command: None,
                name: "review".into(),
                path: "/a/SKILL.md".into(),
                description: "A".into(),
                enabled: true,
            },
            Skill {
                command: None,
                name: "review".into(),
                path: "/b/SKILL.md".into(),
                description: "B".into(),
                enabled: true,
            },
            Skill {
                command: None,
                name: "hidden".into(),
                path: "/c/SKILL.md".into(),
                description: String::new(),
                enabled: false,
            },
        ];
        assert_eq!(invocation_candidates(commands.clone(), vec![]).len(), 1);
        let dollar = invocation_candidates(vec![], skills.clone());
        assert_eq!(dollar.len(), 2);
        let slash = invocation_candidates(commands, skills);
        assert_eq!(slash.len(), 3);
        assert!(matches!(slash[0].invocation, Invocation::Command { .. }));
        for candidate in &slash[1..] {
            assert!(matches!(candidate.invocation, Invocation::Skill { .. }));
            assert!(candidate.invocation.prompt_text().contains("SKILL.md"));
        }
        assert_ne!(slash[1].invocation, slash[2].invocation);
    }

    #[test]
    fn slash_token_opens_at_prose_boundaries() {
        assert_eq!(
            slash_token("/comp", 5),
            Some(MentionToken {
                range: 0..5,
                query: "comp".into(),
            })
        );
        // Token range spans the whole command word even mid-cursor.
        assert_eq!(
            slash_token("/compact now", 3),
            Some(MentionToken {
                range: 0..8,
                query: "co".into(),
            })
        );
        // Not at offset 0 → prose, not a command.
        assert_eq!(slash_token("run /compact", 12).unwrap().range, 4..12);
        // Cursor past the command word (typing the argument) → closed.
        assert!(slash_token("/goal ship it", 10).is_none());
        // A typed absolute path is not a command.
        assert!(slash_token("/usr/bin", 8).is_none());
        // Bare "/" with cursor at 0 → closed; cursor after it → open-all.
        assert!(slash_token("/", 0).is_none());
        assert_eq!(slash_token("/", 1).map(|t| t.query), Some(String::new()));
    }

    #[test]
    fn slash_rows_keep_built_ins_and_zeron_commands() {
        let command = |name: &str, kind: SlashCommandKind| SlashCommand {
            name: name.into(),
            description: String::new(),
            input_hint: None,
            kind,
        };
        let names: Vec<String> = with_zeron_commands(
            Some(HarnessId::ClaudeCode),
            vec![
                command("maestro-work", SlashCommandKind::Skill),
                command("model", SlashCommandKind::BuiltIn),
            ],
        )
        .into_iter()
        .map(|command| command.name)
        .collect();
        assert_eq!(
            names,
            vec![
                SIDECHAT_COMMAND,
                COMPACT_COMMAND,
                COMPUTER_USE_COMMAND,
                "model"
            ]
        );
        assert!(zeron_owned_command(COMPACT_COMMAND));
        assert!(!zeron_owned_command("model"));
    }

    #[test]
    fn a_leading_zeron_command_is_the_only_plain_text_painted_as_a_command() {
        assert_eq!(zeron_owned_command_range("/sidechat á"), Some(0..9));
        assert_eq!(zeron_owned_command_range("\n  /compact"), Some(3..11));
        assert_eq!(zeron_owned_command_range("/sidechats"), None);
        assert_eq!(zeron_owned_command_range("/model"), None);
        assert_eq!(zeron_owned_command_range("ask in a /sidechat"), None);
    }

    #[test]
    fn sent_commands_and_skills_echo_as_invocations() {
        let command = zeron_proto::invocation::Invocation::Command {
            name: "review".into(),
        }
        .link();
        let (display, spans) = sent_mention_display(&format!("{command} now")).unwrap();
        assert!(display.contains("/review"));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, ChipKind::Command);
    }

    #[test]
    fn compact_is_intercepted_with_its_focus_and_nothing_else_is() {
        assert_eq!(compact_focus("/compact"), Some(None));
        assert_eq!(
            compact_focus("/compact focus on auth"),
            Some(Some("focus on auth".into()))
        );
        // A longer name that merely starts with it is another command.
        assert!(compact_focus("/compaction").is_none());
        assert!(compact_focus("/compact-now").is_none());
        // Not the first token → an ordinary prompt, sent as a turn.
        assert!(compact_focus("please /compact").is_none());
        assert!(compact_focus("/goal ship it").is_none());
    }

    #[test]
    fn the_compact_entry_never_doubles_a_harness_that_advertises_it() {
        let advertised = |name: &str| SlashCommand {
            name: name.into(),
            description: "harness copy".into(),
            input_hint: None,
            kind: SlashCommandKind::BuiltIn,
        };
        let names = |commands: &[SlashCommand]| -> Vec<String> {
            commands.iter().map(|c| c.name.clone()).collect()
        };
        // Harness advertises it: exactly one row, the harness's own.
        let merged = with_compact_entry(vec![advertised("compact"), advertised("clear")]);
        assert_eq!(names(&merged), vec!["compact", "clear"]);
        assert_eq!(merged[0].description, "harness copy");
        // Harness does not (cursor advertises nothing at all): zeron's rides.
        let merged = with_compact_entry(Vec::new());
        assert_eq!(names(&merged), vec!["compact"]);
        assert_eq!(
            merged[0].description,
            "Free up context by summarizing the conversation"
        );
    }

    #[gpui::test]
    fn compaction_is_refused_with_a_reason_the_surfaces_share(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, _, cx| {
                assert!(composer.compact_disabled_reason(cx).is_none());
                // A harness that reports no compaction names itself.
                composer.state.update(cx, |state, _| {
                    state.chats = vec![zeron_proto::Chat {
                        id: "c1".into(),
                        device_id: "dev".into(),
                        title: None,
                        archived: false,
                        cwd: None,
                        branch: None,
                        checkout_id: None,
                        source_context: None,
                        config: Some(zeron_proto::ChatConfig {
                            harness: HarnessId::Cursor,
                            model: None,
                            reasoning: None,
                            model_options: serde_json::Map::new(),
                            sandbox: SandboxLevel::WorkspaceWrite,
                            permission_mode: zeron_proto::PermissionMode::Full,
                        }),
                        last_message_preview: None,
                        last_message_at: None,
                        created_at: chrono::Utc::now(),
                        harness_session_id: None,
                        harness_session_cwd: None,
                        space_id: None,
                        last_seen_at: None,
                        room_gen: None,
                        sidechat_source_chat_id: None,
                        created_by_chat_id: None,
                        previous_harnesses: Vec::new(),
                        delegation: None,
                    }];
                    state.selected_chat = Some("c1".into());
                });
                composer
                    .compaction_cache
                    .insert(HarnessId::Cursor, zeron_proto::CompactionKind::None);
                assert_eq!(
                    composer.compact_disabled_reason(cx).as_deref(),
                    Some("Cursor does not support compaction")
                );
                assert_eq!(
                    composer.unsupported_compaction(cx).as_deref(),
                    Some("Cursor does not support compaction")
                );
                // A pending compaction outranks it, and blocks the send button.
                composer.state.update(cx, |state, _| {
                    state.compacting_chat = Some("c1".into());
                });
                assert_eq!(
                    composer.compact_disabled_reason(cx).as_deref(),
                    Some("Compaction in progress")
                );
                assert!(composer.send_blocked(cx));
                // Another chat's compaction never locks this one's composer.
                composer.state.update(cx, |state, _| {
                    state.selected_chat = Some("c2".into());
                });
                assert!(!composer.compacting(cx));
                assert!(!composer.send_blocked(cx));
                assert!(composer.compact_disabled_reason(cx).is_none());
            })
            .unwrap();
    }

    #[test]
    fn resume_compaction_is_due_only_for_a_large_idle_claude_chat() {
        let now = chrono::Utc::now();
        let ago = |minutes| Some(now - chrono::Duration::minutes(minutes));
        let due = |harness, tokens, at| resume_compaction_due(harness, tokens, at, now);
        assert_eq!(
            due(HarnessId::ClaudeCode, Some(184_312), ago(145)),
            Some(ResumeCompaction {
                tokens: 184_312,
                idle_minutes: 145
            })
        );
        assert!(due(HarnessId::ClaudeCode, Some(100_000), ago(70)).is_some());
        assert!(due(HarnessId::ClaudeCode, Some(99_999), ago(145)).is_none());
        assert!(due(HarnessId::ClaudeCode, Some(184_312), ago(69)).is_none());
        assert!(due(HarnessId::ClaudeCode, None, ago(145)).is_none());
        assert!(due(HarnessId::ClaudeCode, Some(184_312), None).is_none());
        assert!(due(HarnessId::Codex, Some(184_312), ago(145)).is_none());
    }

    #[test]
    fn resume_compaction_idle_label_reads_in_hours_and_days() {
        let label = |idle_minutes| {
            ResumeCompaction {
                tokens: 0,
                idle_minutes,
            }
            .idle_label()
        };
        assert_eq!(label(45), "45m");
        assert_eq!(label(120), "2h");
        assert_eq!(label(145), "2h 25m");
        assert_eq!(label(3 * 24 * 60 + 5), "3d");
    }

    #[gpui::test]
    fn arrow_keys_walk_the_chats_sent_prompts_and_restore_the_draft(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        let entry = |id: &str, role: MessageRole, text: &str| SessionMessageEntry {
            id: id.into(),
            role,
            parts: vec![MessagePart::Text {
                id: "t0".into(),
                text: text.into(),
            }],
            created_at: 0,
            device_id: "dev".into(),
            status: None,
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        };
        handle
            .update(cx, |composer, _, cx| {
                composer.state.update(cx, |state, _| {
                    state.chats = vec![chat_row("c1", None)];
                    state.selected_chat = Some("c1".into());
                    state.transcript = vec![
                        entry("u1", MessageRole::User, "first"),
                        entry("a1", MessageRole::Assistant, "reply"),
                        entry("u2", MessageRole::User, "second"),
                    ];
                });
            })
            .unwrap();
        cx.run_until_parked();
        let settle = |cx: &mut gpui::TestAppContext| {
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
                .unwrap();
        };
        handle
            .update(cx, |composer, _, cx| {
                composer
                    .input
                    .update(cx, |input, cx| input.set_text("draft", cx));
            })
            .unwrap();
        settle(cx);
        let press = |cx: &mut gpui::TestAppContext, up: bool| -> String {
            handle
                .update(cx, |composer, window, cx| {
                    composer.input.update(cx, |input, cx| {
                        if up {
                            input.up(&Up, window, cx)
                        } else {
                            input.down(&Down, window, cx)
                        }
                    });
                })
                .unwrap();
            settle(cx);
            handle
                .update(cx, |composer, _, cx| {
                    composer.input.read(cx).text().to_string()
                })
                .unwrap()
        };
        assert_eq!(press(cx, true), "second");
        assert_eq!(press(cx, true), "first");
        assert_eq!(press(cx, true), "first");
        assert_eq!(press(cx, false), "second");
        assert_eq!(press(cx, false), "draft");
        assert_eq!(press(cx, false), "draft");

        assert_eq!(press(cx, true), "second");
        handle
            .update(cx, |composer, _, cx| {
                composer
                    .input
                    .update(cx, |input, cx| input.set_text("second, edited", cx));
            })
            .unwrap();
        settle(cx);
        assert_eq!(press(cx, false), "second, edited");
    }

    #[gpui::test]
    fn keeping_full_history_hides_the_offer_until_the_chat_moves_on(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, _, cx| {
                let set_last_message =
                    |composer: &mut Composer, minutes: i64, cx: &mut Context<Composer>| {
                        composer.state.update(cx, |state, _| {
                            let mut chat = chat_row("c1", None);
                            chat.config = Some(zeron_proto::ChatConfig {
                                harness: HarnessId::ClaudeCode,
                                model: None,
                                reasoning: None,
                                model_options: serde_json::Map::new(),
                                sandbox: SandboxLevel::WorkspaceWrite,
                                permission_mode: zeron_proto::PermissionMode::Full,
                            });
                            chat.last_message_at =
                                Some(chrono::Utc::now() - chrono::Duration::minutes(minutes));
                            state.chats = vec![chat];
                            state.selected_chat = Some("c1".into());
                            state.context_usage = Some(zeron_proto::ContextUsage {
                                tokens: Some(184_312),
                                window: Some(1_000_000),
                            });
                        });
                    };
                set_last_message(composer, 145, cx);
                let offer = composer
                    .resume_compaction(cx)
                    .expect("offer for an idle chat");
                assert_eq!(offer.tokens, 184_312);

                composer.keep_full_history(cx);
                assert!(composer.resume_compaction(cx).is_none());

                set_last_message(composer, 90, cx);
                assert!(composer.resume_compaction(cx).is_some());

                composer.state.update(cx, |state, _| {
                    state.compacting_chat = Some("c1".into());
                });
                assert!(composer.resume_compaction(cx).is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn submitting_compact_takes_the_rpc_path_not_the_run_path(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        // Without an engine both paths fail, and they fail differently: the
        // compaction owns the draft (cleared, chat-scoped notice) while a Run
        // hands it back untouched under the global one.
        for (text, intercepted) in [("/compact focus on auth", true), ("compact this", false)] {
            handle
                .update(cx, |composer, _, cx| {
                    composer
                        .input
                        .update(cx, |input, cx| input.set_text(text, cx));
                    composer.on_submit(cx);
                    assert_eq!(composer.input.read(cx).text().is_empty(), intercepted);
                    assert_eq!(composer.failure_key.is_some(), intercepted);
                    // Neither builds a Run here, but only the Run path echoes.
                    assert!(composer.state.read(cx).transcript.is_empty());
                    assert!(!composer.sending);
                    composer.failure = None;
                    composer.failure_key = None;
                })
                .unwrap();
        }
    }

    fn chat_row(id: &str, sidechat_of: Option<&str>) -> zeron_proto::Chat {
        zeron_proto::Chat {
            id: id.into(),
            device_id: "dev".into(),
            title: Some(format!("Sidechat: {id}")),
            archived: false,
            cwd: None,
            branch: None,
            checkout_id: None,
            source_context: None,
            config: None,
            last_message_preview: None,
            last_message_at: None,
            created_at: chrono::Utc::now(),
            harness_session_id: None,
            harness_session_cwd: None,
            space_id: None,
            last_seen_at: None,
            room_gen: None,
            sidechat_source_chat_id: sidechat_of.map(str::to_string),
            created_by_chat_id: None,
            previous_harnesses: Vec::new(),
            delegation: None,
        }
    }

    #[test]
    fn sidechat_is_intercepted_with_its_prompt_and_nothing_else_is() {
        assert_eq!(sidechat_prompt("/sidechat"), Some(None));
        assert_eq!(
            sidechat_prompt("/sidechat compare the two"),
            Some(Some("compare the two".into()))
        );
        // A longer name that merely starts with it is another command.
        assert!(sidechat_prompt("/sidechats").is_none());
        assert!(sidechat_prompt("/sidechat-now").is_none());
        // Not the first token → an ordinary prompt, sent as a turn.
        assert!(sidechat_prompt("ask in a /sidechat").is_none());
        assert!(sidechat_prompt("/compact").is_none());
    }

    #[test]
    fn the_sidechat_entry_never_doubles_a_harness_that_advertises_it() {
        let advertised = |name: &str| SlashCommand {
            name: name.into(),
            description: "harness copy".into(),
            input_hint: None,
            kind: SlashCommandKind::BuiltIn,
        };
        let names = |commands: &[SlashCommand]| -> Vec<String> {
            commands.iter().map(|c| c.name.clone()).collect()
        };
        let merged = with_sidechat_entry(vec![advertised("sidechat"), advertised("clear")]);
        assert_eq!(names(&merged), vec!["sidechat", "clear"]);
        assert_eq!(merged[0].description, "harness copy");
        // The popup's real list: both zeron commands ride, each exactly once.
        let merged = with_sidechat_entry(with_compact_entry(Vec::new()));
        assert_eq!(names(&merged), vec!["sidechat", "compact"]);
        assert_eq!(
            merged[0].description,
            "Ask about this chat on the side, without touching it"
        );
    }

    #[test]
    fn the_pill_names_the_sidechat_once() {
        assert_eq!(
            sidechat_pill_label(Some("Sidechat: Fix the parser")),
            "Sidechat: Fix the parser"
        );
        assert_eq!(
            sidechat_pill_label(Some("Fix the parser")),
            "Sidechat: Fix the parser"
        );
        assert_eq!(sidechat_pill_label(None), "Sidechat");
        assert_eq!(sidechat_pill_label(Some("Sidechat:")), "Sidechat");
    }

    #[gpui::test]
    fn expired_sidechat_keeps_the_draft_and_rejects_sending(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, _, cx| {
                composer.state.update(cx, |state, _| {
                    let mut side = chat_row("side", Some("src"));
                    side.created_at = chrono::Utc::now() - chrono::Duration::hours(24);
                    state.chats = vec![side];
                    state.selected_chat = Some("side".into());
                });
                composer
                    .input
                    .update(cx, |input, cx| input.set_text("keep my draft", cx));
                composer.send_prompt("after expiry".into(), cx);
                assert_eq!(composer.input.read(cx).text(), "keep my draft");
                assert!(
                    composer
                        .failure
                        .as_deref()
                        .unwrap()
                        .starts_with("Sidechat expired")
                );
                assert!(!composer.sending);
                assert!(composer.state.read(cx).transcript.is_empty());
            })
            .unwrap();
    }

    #[gpui::test]
    fn submitting_sidechat_asks_for_the_tab_and_never_builds_a_run(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        let composer = handle.root(cx).unwrap();
        let seen: std::rc::Rc<std::cell::RefCell<Vec<(String, Option<String>, Vec<String>)>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let seen = seen.clone();
            cx.subscribe(&composer, move |_, event: &ComposerEvent, _| {
                if let ComposerEvent::OpenSidechat {
                    source_chat_id,
                    prompt,
                    staged,
                } = event
                {
                    seen.borrow_mut().push((
                        source_chat_id.clone(),
                        prompt.clone(),
                        staged.attachments.iter().map(|a| a.id.clone()).collect(),
                    ));
                }
            })
            .detach();
        });
        cx.run_until_parked();
        handle
            .update(cx, |composer, _, cx| {
                composer.state.update(cx, |state, _| {
                    state.chats = vec![chat_row("src", None), chat_row("side", Some("src"))];
                    state.selected_chat = Some("src".into());
                });
                for (text, image) in [("/sidechat", None), ("/sidechat compare", Some("shot"))] {
                    if let Some(id) = image {
                        composer.add_staged(
                            vec![StagedAttachment {
                                id: id.into(),
                                name: "shot.png".into(),
                                content: crate::attachments::AttachmentContent::Image(
                                    std::sync::Arc::new(gpui::Image::from_bytes(
                                        gpui::ImageFormat::Png,
                                        vec![1, 2, 3],
                                    )),
                                ),
                                mention: None,
                            }],
                            cx,
                        );
                    }
                    composer
                        .input
                        .update(cx, |input, cx| input.set_text(text, cx));
                    composer.on_submit(cx);
                    // Intercepted: draft cleared, no echo, no send in flight.
                    assert!(composer.input.read(cx).text().is_empty());
                    assert!(composer.staged().is_empty());
                    assert!(composer.state.read(cx).transcript.is_empty());
                    assert!(!composer.sending);
                    assert!(composer.failure.is_none());
                }
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            seen.borrow().as_slice(),
            [
                ("src".to_string(), None, vec![]),
                (
                    "src".to_string(),
                    Some("compare".to_string()),
                    vec!["shot".to_string()]
                )
            ]
        );
    }

    #[gpui::test]
    fn a_sidechat_refuses_to_open_one_of_its_own(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        let composer = handle.root(cx).unwrap();
        let seen = std::rc::Rc::new(std::cell::Cell::new(0usize));
        cx.update(|cx| {
            let seen = seen.clone();
            cx.subscribe(&composer, move |_, event: &ComposerEvent, _| {
                if matches!(event, ComposerEvent::OpenSidechat { .. }) {
                    seen.set(seen.get() + 1);
                }
            })
            .detach();
        });
        cx.run_until_parked();
        handle
            .update(cx, |composer, _, cx| {
                composer.state.update(cx, |state, _| {
                    state.chats = vec![chat_row("src", None), chat_row("side", Some("src"))];
                    state.selected_chat = Some("side".into());
                });
                composer
                    .input
                    .update(cx, |input, cx| input.set_text("/sidechat compare", cx));
                composer.on_submit(cx);
                assert_eq!(
                    composer.failure.as_deref(),
                    Some("Nested sidechats are not supported")
                );
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(seen.get(), 0);
    }

    #[gpui::test]
    fn initial_canvas_carry_preserves_an_attachment_only_draft(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::draft_store::ensure(cx);
            crate::draft_store::set(
                "draft-target",
                crate::draft_store::DraftEntry {
                    attachments: vec![StagedAttachment {
                        id: "image".into(),
                        name: "image.png".into(),
                        content: crate::attachments::AttachmentContent::Image(std::sync::Arc::new(
                            gpui::Image::from_bytes(
                                gpui::ImageFormat::Png,
                                include_bytes!("../../../dist/macos/icon-1024.png").to_vec(),
                            ),
                        )),
                        mention: None,
                    }],
                    ..Default::default()
                },
                cx,
            );
        });
        let (_dir, main) = composer_focus_window(cx);
        main.update(cx, |composer, _, cx| {
            assert!(composer.current_key.is_empty());
            composer.seed_text("typed on initial canvas".into(), cx);
            composer.state.update(cx, |state, _| {
                state.draft_sessions.push(crate::draft_store::DraftSession {
                    id: "draft-target".into(),
                    ..Default::default()
                });
                state.active_draft = Some("draft-target".into());
            });
            composer.on_state_changed(cx);
            assert_eq!(composer.input.read(cx).text(), "typed on initial canvas");
            assert_eq!(composer.staged().len(), 1);
            assert_eq!(
                crate::draft_store::entry("draft-target", cx)
                    .attachments
                    .len(),
                1
            );
        })
        .unwrap();
    }

    #[gpui::test]
    fn navigation_restores_the_latest_draft_from_another_composer(cx: &mut gpui::TestAppContext) {
        let (_dir, main) = composer_focus_window(cx);
        let state = main
            .read_with(cx, |composer, _| composer.state.clone())
            .unwrap();
        main.update(cx, |composer, _, cx| {
            state.update(cx, |state, _| {
                state.chats = vec![chat_row("a", None), chat_row("b", None)];
                state.selected_chat = Some("a".into());
            });
            composer.on_state_changed(cx);
            composer.seed_text("old draft".into(), cx);
            state.update(cx, |state, _| state.selected_chat = Some("b".into()));
            composer.on_state_changed(cx);
            composer.seed_text("other chat".into(), cx);
        })
        .unwrap();
        let bound = cx.new(|cx| Composer::for_chat(state.clone(), "a".into(), cx));
        bound.update(cx, |composer, cx| {
            assert_eq!(composer.input.read(cx).text(), "old draft");
            composer.seed_text("edited in another pane".into(), cx);
        });
        main.update(cx, |composer, _, cx| {
            state.update(cx, |state, _| state.selected_chat = Some("a".into()));
            composer.on_state_changed(cx);
            assert_eq!(composer.input.read(cx).text(), "edited in another pane");
            assert_eq!(crate::draft_store::entry("b", cx).prompt, "other chat");
        })
        .unwrap();
    }

    #[gpui::test]
    fn a_bound_composer_reads_its_own_chat_not_the_selection(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        let state = handle
            .read_with(cx, |composer, _| composer.state.clone())
            .unwrap();
        let window = cx.add_window(|_, cx| Composer::for_chat(state.clone(), "side".into(), cx));
        window
            .update(cx, |composer, _, cx| {
                composer.state.update(cx, |state, _| {
                    let config = |model: &str| {
                        Some(zeron_proto::ChatConfig {
                            harness: zeron_proto::HarnessId::Mock,
                            model: Some(model.into()),
                            reasoning: None,
                            model_options: Default::default(),
                            sandbox: zeron_proto::SandboxLevel::WorkspaceWrite,
                            permission_mode: zeron_proto::PermissionMode::Full,
                        })
                    };
                    let mut src = chat_row("src", None);
                    src.config = config("src-model");
                    let mut side = chat_row("side", Some("src"));
                    side.config = config("side-model");
                    state.chats = vec![src, side];
                    state.selected_chat = Some("src".into());
                });
                assert_eq!(composer.bound_chat_id(cx).as_deref(), Some("side"));
                assert_eq!(
                    composer.pickers.read(cx).resolved(cx).model.as_deref(),
                    Some("side-model"),
                    "the sidechat's pickers resolve its own config, not the source chat's"
                );
                assert_eq!(composer.current_key, "side");
                assert_eq!(
                    composer.bound_chat_row(cx).map(|chat| chat.id.as_str()),
                    Some("side")
                );
                // A chat-navigation notify must not re-key it to the selection.
                composer.on_state_changed(cx);
                assert_eq!(composer.current_key, "side");
            })
            .unwrap();
    }

    #[test]
    fn dismissed_mentions_reject_stale_responses() {
        let mut state = FileMentionState {
            token: mention_token("@src", 4),
            request: 7,
            ..FileMentionState::default()
        };
        assert!(mention_response_is_current(&state, 7));
        state.request += 1;
        state.token = None;
        assert!(!mention_response_is_current(&state, 7));
        assert!(!mention_response_is_current(&state, 8));
    }

    #[test]
    fn file_mentions_serialize_to_strict_local_markdown() {
        let raw = local_file_link("src/a file#[x].rs", false);
        assert_eq!(
            raw,
            "[a file#\\[x\\].rs](zeron-file:src/a%20file%23%5Bx%5D.rs)"
        );
        let links = file_mention_links(&raw);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].path, "src/a file#[x].rs");
        assert_eq!(links[0].basename, "a file#[x].rs");
        assert!(!links[0].is_dir());

        let folder = local_file_link("src/components", true);
        assert_eq!(folder, "[components](zeron-file:src/components/)");
        let links = file_mention_links(&folder);
        assert_eq!(links[0].path, "src/components");
        assert!(links[0].is_dir());
    }

    #[test]
    fn dropped_mentions_are_separated_from_surrounding_text() {
        let link = local_file_link("src/lib.rs", false);
        let (inserted, cursor_advance) =
            dropped_reference("fixnow", 3..3, &link).expect("valid drop");
        assert_eq!(inserted, " [lib.rs](zeron-file:src/lib.rs) ");
        assert_eq!(cursor_advance, inserted.len());

        let link = local_file_link("src/components", true);
        let (inserted, cursor_advance) =
            dropped_reference("fix now", 3..3, &link).expect("valid drop");
        assert_eq!(inserted, " [components](zeron-file:src/components/)");
        assert_eq!(cursor_advance, inserted.len() + 1);
    }

    #[test]
    fn file_mentions_reject_external_or_noncanonical_markdown() {
        assert!(file_mention_links("[site](https://example.com/a)").is_empty());
        assert!(file_mention_links("[a.rs](../a.rs)").is_empty());
        assert!(file_mention_links("[a.rs](src/a file.rs)").is_empty());
        assert!(file_mention_links("[other](src/a.rs)").is_empty());
        assert!(file_mention_links("[a.rs](src/a.rs)").is_empty());
        assert!(file_mention_links("[a.rs](src%5Cfake%5Ca.rs)").is_empty());
        assert!(file_mention_links("[a.rs](src/a%0A.rs)").is_empty());
    }

    #[test]
    fn duplicate_mention_basenames_use_unique_suffixes() {
        let raw = format!(
            "{} {}",
            local_file_link("src/one/mod.rs", false),
            local_file_link("src/two/mod.rs", false)
        );
        let projection = TextProjection::new(&raw);
        assert!(projection.display.contains("one/mod.rs"));
        assert!(projection.display.contains("two/mod.rs"));
    }

    #[test]
    fn mention_suffixes_compare_path_components() {
        let links = vec![
            FileMentionLink {
                range: 0..0,
                basename: "mod.rs".into(),
                path: "foo/mod.rs".into(),
                kind: ChipKind::File,
                attachment: None,
            },
            FileMentionLink {
                range: 0..0,
                basename: "oomod.rs".into(),
                path: "bar/oomod.rs".into(),
                kind: ChipKind::File,
                attachment: None,
            },
        ];
        assert_eq!(
            mention_display_labels(&links),
            vec!["mod.rs".to_string(), "oomod.rs".to_string()]
        );
    }

    #[test]
    fn projection_maps_and_expands_atomic_chip_ranges() {
        let raw = format!("open {} now", local_file_link("src/composer.rs", false));
        let projection = TextProjection::new(&raw);
        let (link, chip) = &projection.mentions[0];
        assert_eq!(
            &projection.display[chip.clone()],
            format!("{MENTION_SIDE_PAD}{CHIP_ICON_SLOT}composer.rs{CHIP_TRAILING_PAD}")
        );
        assert_eq!(projection.display_to_raw(chip.start + 1), link.range.start);
        assert_eq!(projection.display_to_raw(chip.end - 1), link.range.end);
        assert_eq!(
            projection.previous_boundary(link.range.end),
            Some(link.range.start)
        );
        assert_eq!(
            projection.next_boundary(link.range.start),
            Some(link.range.end)
        );
        assert_eq!(
            projection.normalize_range(link.range.start + 2..link.range.end - 2),
            link.range
        );
    }

    #[test]
    fn sent_mention_display_projects_chips_for_the_transcript() {
        let raw = format!(
            "check {} and {}",
            local_file_link("src/composer.rs", false),
            local_file_link("src/components", true)
        );
        let (display, spans) = sent_mention_display(&raw).expect("mentions project");
        assert!(!display.contains(FILE_MENTION_SCHEME));
        assert!(display.contains("composer.rs"));
        assert!(display.contains("components"));
        assert_eq!(spans.len(), 2);
        assert_eq!(
            &display[spans[0].range.clone()],
            format!("{MENTION_SIDE_PAD}{CHIP_ICON_SLOT}composer.rs{CHIP_TRAILING_PAD}")
        );
        assert_eq!(spans[0].kind, ChipKind::File);
        assert_eq!(spans[1].kind, ChipKind::Directory);
        assert_eq!(spans[0].path.as_ref(), "src/composer.rs");
        assert_eq!(spans[1].path.as_ref(), "src/components/");
    }

    #[test]
    fn thread_mentions_serialize_and_project_to_a_title_chip() {
        let raw = thread_link("2f9e1d58-aa", "Fix [PR] rate limits");
        assert_eq!(raw, "[Fix \\[PR\\] rate limits](zeron-thread:2f9e1d58-aa)");
        let links = thread_mention_links(&raw);
        assert_eq!(
            links,
            vec![FileMentionLink {
                range: 0..raw.len(),
                basename: "Fix [PR] rate limits".into(),
                path: "2f9e1d58-aa".into(),
                kind: ChipKind::Thread,
                attachment: None,
            }]
        );
        let projection = TextProjection::new(&raw);
        assert_eq!(
            &projection.display[projection.mentions[0].1.clone()],
            format!("\u{00A0}{CHIP_ICON_SLOT}Fix\u{00A0}[PR]\u{00A0}rate\u{00A0}limits\u{00A0}")
        );
        let (display, spans) = sent_mention_display(&format!("see {raw} first")).unwrap();
        assert!(!display.contains(THREAD_MENTION_SCHEME));
        assert_eq!(spans[0].kind, ChipKind::Thread);
        assert_eq!(spans[0].path.as_ref(), "2f9e1d58-aa");
        // Unsafe ids, empty labels, and labels that do not round-trip are
        // ordinary Markdown, not chips.
        assert!(thread_mention_links("[t](zeron-thread:../x)").is_empty());
        assert!(thread_mention_links("[](zeron-thread:abc)").is_empty());
        assert!(thread_mention_links("[a]b](zeron-thread:abc)").is_empty());
        assert_eq!(sent_mention_display("what is a zeron-thread: link?"), None);
    }

    #[test]
    fn thread_matches_filter_by_title_or_id_prefix_and_skip_self_and_drafts() {
        let chat = |id: &str, title: Option<&str>| zeron_proto::Chat {
            id: id.into(),
            device_id: "dev".into(),
            title: title.map(str::to_string),
            archived: false,
            cwd: None,
            branch: None,
            checkout_id: None,
            source_context: None,
            config: None,
            last_message_preview: None,
            last_message_at: None,
            created_at: chrono::Utc::now(),
            harness_session_id: None,
            harness_session_cwd: None,
            space_id: None,
            last_seen_at: None,
            room_gen: None,
            sidechat_source_chat_id: None,
            created_by_chat_id: None,
            previous_harnesses: Vec::new(),
            delegation: None,
        };
        let mut chats = vec![
            chat("a1", Some("Fix PR fetching")),
            chat("b2", Some("Minimize slider")),
            chat("c3", None),
            chat("d4", Some("Fix Claude context")),
            chat("e5", Some("Fix on the laptop")),
            chat("f6", Some("Fix the sidebar")),
        ];
        chats[4].device_id = "laptop".into();
        chats[0].space_id = Some("other".into());
        chats[5].space_id = Some("mine".into());
        let scope = |space_id| MentionScope {
            device_id: "dev",
            space_id,
            exclude: Some("d4"),
        };
        let project_of = |chat: &zeron_proto::Chat| match chat.space_id.as_deref() {
            Some("mine") => "zeron".to_string(),
            Some("other") => "synara".to_string(),
            _ => "~".to_string(),
        };
        let titles = |query: &str| {
            thread_matches(
                chats.iter(),
                query,
                scope(Some("mine")),
                |id| id == "b2",
                project_of,
            )
            .into_iter()
            .map(|m| format!("{} · {}", m.title, m.project))
            .collect::<Vec<_>>()
        };
        // The composer's own project first, then the rest in sidebar order.
        assert_eq!(
            titles(""),
            vec![
                "Fix the sidebar · zeron",
                "Fix PR fetching · synara",
                "New session · ~"
            ]
        );
        assert_eq!(
            titles("fix"),
            vec!["Fix the sidebar · zeron", "Fix PR fetching · synara"]
        );
        assert_eq!(titles("C3"), vec!["New session · ~"]);
        assert!(titles("zzz").is_empty());
        // A project-less composer puts project-less threads first.
        let ids: Vec<String> =
            thread_matches(chats.iter(), "", scope(None), |id| id == "b2", project_of)
                .into_iter()
                .map(|m| m.chat_id)
                .collect();
        assert_eq!(ids, vec!["c3", "a1", "f6"]);
    }

    /// The padding pinned to Geist is exactly the chip's NBSP padding, so the
    /// label keeps the interface font and the insets don't follow it.
    #[test]
    fn chip_pad_ranges_cover_only_the_padding() {
        let raw = format!(
            "open {} now",
            zeron_proto::attachment_mentions::attachment_mention_link(1, None)
        );
        let (display, spans) = sent_mention_display(&raw).expect("image chip projects");
        let [lead, trail] = chip_pad_ranges(&spans[0].range);
        assert_eq!(
            &display[lead.clone()],
            format!("{MENTION_SIDE_PAD}{CHIP_ICON_SLOT}")
        );
        assert_eq!(&display[trail.clone()], CHIP_TRAILING_PAD);
        assert_eq!(&display[lead.end..trail.start], "Image\u{a0}1");
    }

    /// The composer shapes a chip's padding in Geist and everything else,
    /// the chip's label included, in the interface font.
    #[gpui::test]
    fn composer_chip_padding_is_shaped_in_geist(cx: &mut gpui::TestAppContext) {
        with_composer_input(cx, |input, window, cx| {
            let raw = format!("open {} now", local_file_link("src/composer.rs", false));
            input.set_text(&raw, cx);
            let mut style = window.text_style();
            style.font_family = "Geist Mono".into();
            input.layout_text(px(600.0), &style, window, cx);
            let (_, chip) = input.projection.mentions[0].clone();
            let pads = chip_pad_ranges(&chip);
            let mut at = 0;
            let mut pinned = 0;
            for run in &input.last_runs {
                let range = at..at + run.len;
                at = range.end;
                if pads.iter().any(|pad| pad.contains(&range.start)) {
                    assert_eq!(run.font.family.as_ref(), CHIP_PAD_FAMILY, "{range:?}");
                    assert!(pads.iter().any(|pad| pad.end >= range.end), "{range:?}");
                    pinned += run.len;
                } else {
                    assert_eq!(run.font.family.as_ref(), "Geist Mono", "{range:?}");
                }
            }
            assert_eq!(pinned, pads.iter().map(|pad| pad.len()).sum::<usize>());
        });
    }

    /// The queue's family overrides are every chip's padding, in order and
    /// never empty, as `with_font_family_overrides` requires.
    #[test]
    fn queue_chip_overrides_are_the_padding_in_order() {
        let raw = format!(
            "{} then {}",
            local_file_link("a.rs", false),
            zeron_proto::attachment_mentions::attachment_mention_link(1, None),
        );
        let (_, spans) = sent_mention_display(&raw).expect("chips project");
        let overrides = chip_pad_overrides(&spans);
        let expected: Vec<_> = spans
            .iter()
            .flat_map(|span| chip_pad_ranges(&span.range))
            .collect();
        assert_eq!(overrides, expected);
        assert!(
            overrides
                .windows(2)
                .all(|pair| pair[0].end <= pair[1].start)
        );
    }

    /// A chip centers on its label's cap height: the row's middle for Geist,
    /// and a nudge toward the label for faces whose caps sit higher or lower.
    #[test]
    fn chip_centering_follows_the_label_face() {
        // A 14 px label on a 22 px row whose top is at y = 100, at 1x.
        let center = |ascent: f32, descent: f32, cap: f32| {
            let em = |units: f32| units / 1000.0 * 14.0;
            label_center_offset(100.0, 22.0, em(ascent), em(descent), em(cap), 1.0)
        };
        // Geist, alone or as the padding beside a Geist Mono label.
        assert_eq!(center(1005.0, 295.0, 710.0), 0.0);
        // A label with much shorter caps sits lower, so the pill follows.
        assert_eq!(center(1005.0, 295.0, 560.0), 1.0);
        // A tall face lowers the baseline past the next pixel.
        assert_eq!(center(1200.0, 295.0, 710.0), 1.0);
        // Without a cap height there is nothing to center on.
        assert_eq!(center(1005.0, 295.0, 0.0), 0.0);
        // The baseline snaps to a pixel where the row lands, so the same
        // face can need the pill a pixel lower on a row a little further down.
        assert_eq!(
            label_center_offset(100.0, 22.0, 14.966, 4.13, 10.0, 1.0),
            0.0
        );
        assert_eq!(
            label_center_offset(100.2, 22.0, 14.966, 4.13, 10.0, 1.0),
            1.0
        );
    }

    /// Ordinary prompts must stay on the zero-cost path, including ones that
    /// merely *talk about* the scheme without containing a valid mention.
    #[test]
    fn sent_mention_display_leaves_plain_prompts_untouched() {
        assert_eq!(sent_mention_display("fix the composer"), None);
        assert_eq!(
            sent_mention_display("what is a zeron-file: link?"),
            None,
            "scheme substring without a valid mention link"
        );
        assert_eq!(
            sent_mention_display("[a.rs](zeron-file:../a.rs)"),
            None,
            "a hostile path never becomes a chip in the transcript either"
        );
    }

    fn question(id: &str, options: &[&str], multi: bool) -> UserInputQuestion {
        UserInputQuestion {
            id: id.into(),
            header: "Header".into(),
            question: format!("Question {id}"),
            options: options.iter().map(|s| s.to_string()).collect(),
            prefill: None,
            multiline: false,
            multi_select: multi,
            kind: None,
        }
    }

    #[test]
    fn flip_decision() {
        // Fits in the pill → compact stays compact.
        assert!(!composer_flip(false, 150.0, 300.0, false, false));
        // Overflow → expand.
        assert!(composer_flip(false, 320.0, 300.0, false, false));
        // Newline always expands (either mode, even mid-resize).
        assert!(composer_flip(false, 10.0, 300.0, true, false));
        assert!(composer_flip(true, 10.0, 300.0, true, true));
        // Narrow column (< MIN_COMPACT_INPUT_WIDTH) always expands.
        assert!(composer_flip(false, 10.0, 199.0, false, false));
        assert!(!composer_flip(false, 10.0, 200.0, false, false));
    }

    #[test]
    fn flip_hysteresis_band_prevents_oscillation() {
        let cap = 300.0;
        // Text just over capacity expands…
        assert!(composer_flip(false, cap + 1.0, cap, false, false));
        // …and the SAME width, now expanded, does NOT collapse back — the
        // collapse threshold sits COLLAPSE_HYSTERESIS below the expand one.
        assert!(composer_flip(true, cap + 1.0, cap, false, false));
        // Anywhere inside the band the two modes are both stable (no width in
        // (cap - 32, cap] flips in either direction).
        let in_band = cap - COLLAPSE_HYSTERESIS + 1.0;
        assert!(!composer_flip(false, in_band, cap, false, false));
        assert!(composer_flip(true, in_band, cap, false, false));
        // Comfortably under the band → collapses.
        assert!(!composer_flip(
            true,
            cap - COLLAPSE_HYSTERESIS - 1.0,
            cap,
            false,
            false
        ));
    }

    #[test]
    fn resize_expands_live_but_defers_collapse() {
        // A compact composer expands immediately as its text or controls stop
        // fitting, even while the divider is moving.
        assert!(composer_flip(false, 500.0, 300.0, false, true));
        assert!(composer_flip(false, 10.0, 150.0, false, true));
        // An expanded composer waits for the drag to settle before collapsing,
        // avoiding mode chatter while the user reverses direction.
        assert!(composer_flip(true, 0.0, 300.0, false, true));
        // Once settled, the same wide layout may collapse.
        assert!(composer_flip(false, 500.0, 300.0, false, false));
        assert!(!composer_flip(true, 0.0, 300.0, false, false));
        assert!(composer_flip(false, 10.0, 150.0, false, false));
    }

    #[test]
    fn caret_blink_phase() {
        // Solid through the first half-period (typing burst never blinks).
        assert!(caret_visible(0));
        assert!(caret_visible(CARET_BLINK_MS - 1));
        // Off for the second half-period, back on for the third.
        assert!(!caret_visible(CARET_BLINK_MS));
        assert!(!caret_visible(2 * CARET_BLINK_MS - 1));
        assert!(caret_visible(2 * CARET_BLINK_MS));
    }

    #[test]
    fn auto_grow_math() {
        // The source heights (zeron composer.tsx line 235 clamp, composer-
        // actions.tsx row, 1px hairlines): 76+46+2 empty … 260+46+2 capped.
        assert_eq!(COMPOSER_MIN_HEIGHT, 120.0);
        assert_eq!(COMPOSER_MAX_HEIGHT, 304.0);
        // One line sits at the floor: the textarea BOX (content + `pt-4 pb-1`)
        // clamps UP to 76 exactly like `Math.max(scrollHeight, 76)` — this is
        // what makes the always-expanded new-chat composer 120px tall.
        assert_eq!(
            composer_total_height(input_content_height(1)),
            COMPOSER_MIN_HEIGHT
        );
        // Growth is linear once the textarea box exceeds its 76px floor.
        let h4 = composer_total_height(input_content_height(4));
        assert_eq!(
            h4,
            4.0 * INPUT_LINE_HEIGHT + TEXTAREA_PAD_V + ACTIONS_ROW_HEIGHT + PILL_BORDER_V
        );
        // Caps at a 260px textarea box (zeron max-h-[260px] / the JS clamp).
        assert_eq!(
            composer_total_height(input_content_height(100)),
            COMPOSER_MAX_HEIGHT
        );
        // Zero lines still measures one.
        assert_eq!(input_content_height(0), INPUT_LINE_HEIGHT);
    }

    #[test]
    fn appshot_strip_height_tracks_cards() {
        assert_eq!(appshot_strip_height(0), 0.0);
        assert_eq!(appshot_strip_height(1), STRIP_PAD_TOP + APPSHOT_TILE_HEIGHT);
        assert_eq!(appshot_strip_height(2), appshot_strip_height(1));
    }

    #[test]
    fn appshot_images_fit_the_chip_inset_without_losing_aspect_ratio() {
        // The inset box is 200×120: landscape fills the width, portrait the height.
        let landscape = appshot_contained_size(Some((1600, 900)));
        assert_eq!(landscape, (200.0, 112.5));
        let portrait = appshot_contained_size(Some((900, 1600)));
        assert!((portrait.0 - 67.5).abs() < 0.01);
        assert!((portrait.1 - 120.0).abs() < 0.01);
        let square = appshot_contained_size(Some((1000, 1000)));
        assert!((square.0 - 120.0).abs() < 0.01 && (square.1 - 120.0).abs() < 0.01);
        // Panoramas fit without distortion.
        assert_eq!(appshot_contained_size(Some((4000, 1000))), (200.0, 50.0));
        let fallback = appshot_contained_size(None);
        assert_eq!(fallback, (192.0, 120.0));
    }

    #[test]
    fn appshot_provenance_leads_with_the_window_title_unless_it_echoes_the_app() {
        assert_eq!(
            appshot_provenance("Safari", Some("API documentation")),
            "API documentation / Safari"
        );
        assert_eq!(appshot_provenance("ChatGPT", Some("chatgpt")), "ChatGPT");
        assert_eq!(appshot_provenance("Terminal", Some("  ")), "Terminal");
        assert_eq!(appshot_provenance("", None), "Captured app");
    }

    #[test]
    fn input_wheel_scroll_uses_gpui_direction_and_clamps() {
        // Positive wheel delta moves toward the start; negative moves down.
        assert_eq!(input_scroll_offset(40.0, 20.0, 200.0, 100.0), 20.0);
        assert_eq!(input_scroll_offset(40.0, -30.0, 200.0, 100.0), 70.0);
        // Neither edge can be overscrolled.
        assert_eq!(input_scroll_offset(10.0, 50.0, 200.0, 100.0), 0.0);
        assert_eq!(input_scroll_offset(90.0, -50.0, 200.0, 100.0), 100.0);
        // Short content has no internal scroll range.
        assert_eq!(input_scroll_offset(20.0, -50.0, 80.0, 100.0), 0.0);
    }

    #[test]
    fn input_scroll_reveals_only_when_caret_leaves_viewport() {
        // A visible caret preserves the user's viewport.
        assert_eq!(
            input_scroll_offset_for_cursor(40.0, 60.0, 20.0, 300.0, 100.0, None),
            40.0
        );
        // Moving above or below reveals the row with the smallest adjustment.
        assert_eq!(
            input_scroll_offset_for_cursor(80.0, 30.0, 20.0, 300.0, 100.0, None),
            30.0
        );
        assert_eq!(
            input_scroll_offset_for_cursor(20.0, 130.0, 20.0, 300.0, 100.0, None),
            50.0
        );
        // Revealing the final row clamps exactly to the content end.
        assert_eq!(
            input_scroll_offset_for_cursor(0.0, 290.0, 20.0, 300.0, 100.0, None),
            200.0
        );
    }

    #[test]
    fn input_drag_autoscroll_is_edge_proportional_and_capped() {
        let top = 100.0;
        let bottom = 300.0;
        let line = INPUT_LINE_HEIGHT;
        assert_eq!(input_drag_scroll_delta(200.0, top, bottom, line), 0.0);
        assert_eq!(input_drag_scroll_delta(90.0, top, bottom, line), -2.0);
        assert_eq!(input_drag_scroll_delta(315.0, top, bottom, line), 3.0);
        assert_eq!(input_drag_scroll_delta(-100.0, top, bottom, line), -line);
        assert_eq!(input_drag_scroll_delta(500.0, top, bottom, line), line);
    }

    /// One frame short of the full morph timeline (never rounds up to done).
    const ALMOST: f32 = 179.0;

    #[test]
    fn flip_morph_starts_once_per_committed_flip() {
        // No committed flip → no morph.
        assert_eq!(flip_morph_step(None, false, 49.0, 0.0, false, false), None);
        // A committed flip starts one, from the last rendered height…
        let m = flip_morph_step(None, true, 49.0, 100.0, false, false).unwrap();
        assert_eq!(m.from, 49.0);
        assert_eq!(m.start_ms, 100.0);
        // …and same-mode renders keep it UNCHANGED (no restart at the
        // boundary, whatever the heights are doing).
        assert_eq!(
            flip_morph_step(Some(m), false, 80.0, 150.0, false, false),
            Some(m)
        );
        // A finished morph clears on the next same-mode render.
        assert_eq!(
            flip_morph_step(Some(m), false, 124.0, 100.0 + ALMOST, false, false),
            Some(m)
        );
        assert_eq!(
            flip_morph_step(Some(m), false, 124.0, 300.0, false, false),
            None
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolved_layout_does_not_keep_notifying_on_repaint() {
        use std::cell::Cell;
        gpui_platform::headless().run(|cx| {
            cx.set_global(Theme::dark());
            let handle = cx.open_window(gpui::WindowOptions::default(), |_, cx| {
                cx.new(|cx| {
                    let mut input = ComposerInput::new("Draft", cx);
                    input.set_text("A long line whose wrapping differs between provisional and resolved widths.\n".repeat(100), cx);
                    input
                })
            }).unwrap();
            let changes = Rc::new(Cell::new(0));
            let observed = changes.clone();
            let subscription = cx.subscribe(&handle.entity(cx).unwrap(), move |_, event, _| {
                if matches!(event, ComposerInputEvent::ViewportChanged) {
                    observed.set(observed.get() + 1);
                }
            });
            cx.spawn(async move |cx| {
                let _subscription = subscription;
                cx.update(|cx| {
                    handle.update(cx, |input, _, _| input.last_notified_layout = None).unwrap();
                    cx.update_window(handle.into(), |_, window, cx| { window.refresh(); let _ = window.draw(cx); }).unwrap();
                });
                let settled = changes.get();
                assert!(settled > 0, "the first resolved layout must be published");
                for _ in 0..30 {
                    cx.update(|cx| {
                        cx.update_window(handle.into(), |_, window, cx| { window.refresh(); let _ = window.draw(cx); }).unwrap();
                    });
                }
                assert_eq!(changes.get(), settled, "unchanged draws must not schedule more layout");
                cx.update(|cx| cx.quit());
            }).detach();
        });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn single_line_address_reveals_caret_and_maps_scrolled_pointer() {
        gpui_platform::headless().run(|cx| {
            cx.set_global(Theme::dark());
            let handle = cx
                .open_window(gpui::WindowOptions::default(), |_, cx| {
                    cx.new(|cx| {
                        ComposerInput::new("Address", cx)
                            .with_single_line()
                            .with_text_metrics(11.0, 16.0)
                    })
                })
                .unwrap();
            handle
                .update(cx, |input, window, cx| {
                    let style = window.text_style();
                    input.set_text(
                        "http://device.a-very-long-project-name.localhost:7331/path",
                        cx,
                    );
                    input.layout_text(px(100.0), &style, window, cx);
                    assert_eq!(input.content_height, 16.0, "long hostnames must not wrap");
                    input.clamp_scroll(16.0);
                    assert!(input.scroll_left > 0.0);
                    let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(100.0), px(16.0)));
                    input.last_bounds = Some(bounds);
                    let caret = input
                        .bounds_for_range(
                            input.content.len()..input.content.len(),
                            bounds,
                            window,
                            cx,
                        )
                        .unwrap();
                    assert!(caret.left() >= bounds.left() && caret.right() <= bounds.right());
                    assert_eq!(
                        input.index_for_mouse_position(caret.origin),
                        input.content.len()
                    );
                    input.selected_range = 0..0;
                    input.clamp_scroll(16.0);
                    assert_eq!(input.scroll_left, 0.0, "Home must reveal the URL start");
                    input.replace_text_in_range(None, "one\r\ntwo", window, cx);
                    assert!(!input.content.contains(['\r', '\n']));
                    input.set_text("short", cx);
                    input.layout_text(px(100.0), &style, window, cx);
                    input.clamp_scroll(16.0);
                    assert_eq!(
                        input.scroll_left, 0.0,
                        "short replacement must reset scrolling"
                    );
                })
                .unwrap();
            cx.spawn(async move |cx| {
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn layout_cache_reuses_resize_frames_and_invalidates_text_inputs() {
        gpui_platform::headless().run(|cx| {
            cx.set_global(Theme::dark());
            let handle = cx
                .open_window(gpui::WindowOptions::default(), |_, cx| {
                    cx.new(|cx| ComposerInput::new("Draft", cx))
                })
                .unwrap();
            handle
                .update(cx, |input, window, cx| {
                    input.layout_rebuilds = 0; // Exclude the window's initial placeholder paint.
                    let mut style = window.text_style();
                    style.font_size = px(INPUT_TEXT_SIZE).into();
                    input.set_text(
                        "A wrapped draft with enough text to measure.\n".repeat(100),
                        cx,
                    );
                    input.layout_text(px(400.0), &style, window, cx);
                    assert_eq!(input.layout_rebuilds, 1);
                    let height = input.content_height;
                    for frame in 0..120 {
                        input.viewport_height = Some(40.0 + frame as f32);
                        input.scroll_top = frame as f32;
                        input.selected_range = 2..8;
                        assert_eq!(input.layout_text(px(400.0), &style, window, cx), height);
                    }
                    assert_eq!(
                        input.layout_rebuilds, 1,
                        "resize/scroll/selection must reuse shaping"
                    );
                    input.set_text("Edited draft", cx);
                    input.layout_text(px(400.0), &style, window, cx);
                    assert_eq!(input.layout_rebuilds, 2);
                    input.layout_text(px(200.0), &style, window, cx);
                    assert_eq!(input.layout_rebuilds, 3, "width changes must rewrap");
                    style.font_size = px(18.0).into();
                    input.layout_text(px(200.0), &style, window, cx);
                    assert_eq!(input.layout_rebuilds, 4);
                    input.marked_range = Some(0..2);
                    input.layout_text(px(200.0), &style, window, cx);
                    assert_eq!(
                        input.layout_rebuilds, 5,
                        "IME marking must repaint decoration"
                    );
                    input.unmark_text(window, cx);
                    input.layout_text(px(200.0), &style, window, cx);
                    assert_eq!(input.layout_rebuilds, 6, "IME unmark must also invalidate");
                    input.set_text("", cx);
                    input.layout_text(px(200.0), &style, window, cx);
                    input.set_placeholder("New placeholder", cx);
                    input.layout_text(px(200.0), &style, window, cx);
                    assert_eq!(input.layout_rebuilds, 8);
                    style.color = gpui::rgb(0xff0000).into();
                    input.layout_text(px(200.0), &style, window, cx);
                    assert_eq!(input.layout_rebuilds, 9);
                    input.enable_mentions();
                    input.layout_text(px(200.0), &style, window, cx);
                    assert_eq!(input.layout_rebuilds, 10);
                    cx.set_global(Theme::light());
                    input.layout_text(px(200.0), &style, window, cx);
                    assert_eq!(input.layout_rebuilds, 11, "mention colors follow the theme");
                })
                .unwrap();
            cx.spawn(async move |cx| {
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    }

    #[test]
    fn resize_reveals_only_complete_rows() {
        for visible in [0.0, 5.0, 22.0, 22.75, 30.0, 45.5, 70.0, 150.0] {
            let height = input_reveal_height(visible, 0.0, INPUT_LINE_HEIGHT, true);
            assert!(height <= visible);
            assert_eq!(height % INPUT_LINE_HEIGHT, 0.0);
        }
        // The row grid moves with scrolling; the clip still ends between rows.
        assert_eq!(input_reveal_height(39.0, 7.0, 20.0, true), 33.0);
        // Normal overflow scrolling keeps its full viewport and existing fades.
        assert_eq!(input_reveal_height(39.0, 7.0, 20.0, false), 39.0);
        assert_eq!(input_reveal_height(100.0, 0.0, 20.0, true), 100.0);
    }

    #[test]
    fn resize_keeps_text_anchored_to_the_input_origin() {
        // A fitting draft grows from one row to seven. Caret-follow must
        // never temporarily scroll earlier lines through the top clip.
        for visible in [0.0, 22.75, 60.0, 110.0, 159.25] {
            assert_eq!(
                input_scroll_offset_for_cursor(0.0, 136.5, 22.75, 159.25, visible, Some(159.25),),
                0.0
            );
        }
        // A genuinely overflowing draft keeps the same caret-follow offset
        // through every frame of the reveal, rather than chasing its height.
        for visible in [30.0, 100.0, 180.0, 240.0] {
            assert_eq!(
                input_scroll_offset_for_cursor(160.0, 377.25, 22.75, 400.0, visible, Some(240.0),),
                160.0
            );
        }
        // Deleting back to a fitting draft resets scroll immediately, even
        // while the old, larger viewport is still shrinking.
        assert_eq!(
            input_scroll_offset_for_cursor(160.0, 77.25, 22.75, 100.0, 240.0, Some(100.0),),
            0.0
        );
    }

    #[test]
    fn scroll_fade_ignores_temporary_resize_overflow() {
        for visible_height in [0.0, 20.0, 60.0, 100.0, 160.0] {
            let scroll = input_max_scroll(160.0, visible_height);
            assert_eq!(
                input_overflow_edges(160.0, 160.0, visible_height, scroll),
                (false, false)
            );
        }
        // Deleting a capped draft disables fading immediately, even while
        // its scroll position and outer height are still settling.
        assert_eq!(
            input_overflow_edges(100.0, 100.0, 240.0, 80.0),
            (false, false)
        );
    }

    #[test]
    fn scroll_fade_tracks_real_overflow_edges() {
        for (scroll, top, bottom) in [(0.0, false, true), (80.0, true, true), (160.0, true, false)]
        {
            assert_eq!(
                input_overflow_edges(400.0, 240.0, 240.0, scroll),
                (top, bottom)
            );
        }
    }

    #[test]
    fn content_resize_retargets_from_visible_height_and_settles() {
        let start = composer_total_height(input_content_height(3));
        let target = composer_total_height(input_content_height(6));
        let grow = flip_morph_step(None, true, start, 0.0, false, false).unwrap();
        let visible = grow.height(target, 60.0);
        assert!(visible > start && visible < target);
        // A delete during growth reverses from what is on screen, with no snap.
        let shrink = flip_morph_step(Some(grow), true, visible, 60.0, false, false).unwrap();
        assert_eq!(shrink.height(start, 60.0), visible);
        assert!(shrink.height(start, 120.0) < visible);
        assert_eq!(shrink.height(start, 240.0), start);
        assert_eq!(
            flip_morph_step(Some(shrink), false, start, 240.0, false, false),
            None
        );
        // Toggling reduced motion also cancels an already running resize.
        assert_eq!(
            flip_morph_step(Some(grow), false, visible, 60.0, true, false),
            None
        );
    }

    #[test]
    fn flip_morph_height_ramps_monotonically_to_target() {
        let m = FlipMorph {
            from: 49.0,
            start_ms: 0.0,
            spec: motion::COLLAPSE,
        };
        // Starts exactly at the committed height…
        let mut prev = m.height(124.0, 0.0);
        assert_eq!(prev, 49.0);
        // …ramps without ever moving backwards…
        for step in 1..=18 {
            let h = m.height(124.0, step as f32 * 10.0);
            assert!(h >= prev, "height regressed at {step}: {h} < {prev}");
            prev = h;
        }
        // …and lands exactly on the target when done (and stays there).
        assert_eq!(m.height(124.0, 180.0), 124.0);
        assert!(m.done(180.0));
        assert_eq!(m.height(124.0, 500.0), 124.0);
        // Collapse runs the same ramp downward.
        assert!(m.height(124.0, 90.0) > 49.0);
        let down = FlipMorph {
            from: 124.0,
            start_ms: 0.0,
            spec: motion::COLLAPSE,
        };
        assert!(down.height(49.0, 90.0) < 124.0);
        assert!(down.height(49.0, 90.0) > 49.0);
    }

    #[test]
    fn flip_morph_reverse_hands_off_from_current_height() {
        let m = FlipMorph {
            from: 49.0,
            start_ms: 0.0,
            spec: motion::COLLAPSE,
        };
        let mid = m.height(124.0, 90.0);
        assert!(mid > 49.0 && mid < 124.0);
        // A reverse flip mid-flight commits a new morph FROM the animated
        // height — continuous at the handoff, no pop to an endpoint.
        let rev = flip_morph_step(Some(m), true, mid, 90.0, false, false).unwrap();
        assert_eq!(rev.from, mid);
        assert_eq!(rev.height(49.0, 90.0), mid);
    }

    #[test]
    fn flip_morph_snaps_for_reduced_motion_and_first_paint() {
        // Reduced motion never creates a morph (the flip just snaps)…
        assert_eq!(flip_morph_step(None, true, 49.0, 0.0, true, false), None);
        // …and neither does a flip before anything was ever rendered.
        assert_eq!(flip_morph_step(None, true, 0.0, 0.0, false, false), None);
    }

    #[test]
    fn route_change_never_arms_the_morph() {
        // A flip committed inside the route-snap window must NOT animate —
        // switching sessions (chat↔chat or chat↔new-session) snaps the
        // composer straight to the target mode, like the header (round 6).
        assert_eq!(flip_morph_step(None, true, 49.0, 0.0, false, true), None);
        // The route change also kills anything already in flight…
        let m = FlipMorph {
            from: 49.0,
            start_ms: 0.0,
            spec: motion::COLLAPSE,
        };
        assert_eq!(
            flip_morph_step(Some(m), false, 80.0, 50.0, false, true),
            None
        );
        assert_eq!(
            flip_morph_step(Some(m), true, 80.0, 50.0, false, true),
            None
        );
        // …while outside the window the same flip animates as usual.
        let armed = flip_morph_step(None, true, 49.0, 300.0, false, false).unwrap();
        assert_eq!(armed.from, 49.0);
    }

    #[test]
    fn morph_anchoring_holds_controls_and_glides_text() {
        // Steady state (progress 1): no offsets, everything at rest.
        assert_eq!(morph_cluster_dy(1.0), 0.0);
        assert_eq!(morph_text_pad(1.0), 16.0);
        assert_eq!(collapse_text_glide(124.0, 1.0), 0.0);
        // At the commit instant the pieces start from the OLD mode's resting
        // geometry: text pad at the compact 12px inset, cluster displaced by
        // exactly the 4.5px centering delta.
        assert_eq!(morph_text_pad(0.0), 12.0);
        assert_eq!(morph_cluster_dy(0.0), CLUSTER_Y_DELTA);
        // Collapse glide: starts where the expanded text sat (17px below the
        // committed pill top → `from − 53` above the compact resting spot)…
        assert_eq!(collapse_text_glide(124.0, 0.0), 71.0);
        // …decays monotonically to zero…
        let mut prev = collapse_text_glide(124.0, 0.0);
        for step in 1..=10 {
            let g = collapse_text_glide(124.0, step as f32 / 10.0);
            assert!(g <= prev, "glide regressed at {step}");
            prev = g;
        }
        // …and can't go negative on shallow mid-flight reversals.
        assert_eq!(collapse_text_glide(50.0, 0.0), 0.0);
    }

    #[test]
    fn cluster_inset_glides_between_the_source_endpoints() {
        assert_eq!(ACTION_UTILITY_GAP, 2.0);
        assert_eq!(ACTION_PRIMARY_GAP, Theme::SPACE_SM);
        assert!(ACTION_UTILITY_GAP < ACTION_PRIMARY_GAP);
        // The morph starts from the OLD mode's resting inset (no sideways
        // step at the commit) and eases to the committed mode's…
        assert_eq!(morph_cluster_inset(true, 0.0), 8.0); // expand: from compact pr-2
        assert_eq!(morph_cluster_inset(true, 1.0), 12.0); // …to expanded px-3
        assert_eq!(morph_cluster_inset(false, 0.0), 12.0); // collapse: from px-3
        assert_eq!(morph_cluster_inset(false, 1.0), 8.0); // …to pr-2
        // …monotonically, bounded by the 4px source delta.
        let mut prev = morph_cluster_inset(true, 0.0);
        for step in 1..=10 {
            let v = morph_cluster_inset(true, step as f32 / 10.0);
            assert!(v >= prev && v <= 8.0 + CLUSTER_X_DELTA);
            prev = v;
        }
        // Internal group spacing is shared between modes — only this wrapper
        // inset may differ across the flip.
    }

    #[test]
    fn flip_morph_tracks_live_target_and_drives_fade() {
        let m = FlipMorph {
            from: 49.0,
            start_ms: 0.0,
            spec: motion::COLLAPSE,
        };
        // Auto-grow can move the target mid-morph: evaluation tracks the
        // live value instead of finishing on a stale height.
        assert!(m.height(159.0, 90.0) > m.height(124.0, 90.0));
        // The eased progress is the actions-row fade: 0 at commit, 1 at rest.
        assert_eq!(m.progress(0.0), 0.0);
        assert_eq!(m.progress(180.0), 1.0);
        let mid = m.progress(90.0);
        assert!(mid > 0.0 && mid < 1.0);
    }

    #[test]
    fn new_thread_route_changes_use_the_coordinated_timeline() {
        let m = FlipMorph::new_thread_transition(124.0, 0.0);
        assert_eq!(m.spec, motion::NEW_THREAD_TRANSITION);
        assert_eq!(m.height(49.0, 0.0), 124.0);
        assert!(m.height(49.0, 250.0) < 124.0);
        assert!(m.height(49.0, 250.0) > 49.0);
        assert_eq!(m.height(49.0, 420.0), 49.0);
        let reverse = FlipMorph::new_thread_transition(49.0, 0.0);
        assert_eq!(reverse.height(124.0, 0.0), 49.0);
        assert_eq!(reverse.height(124.0, 420.0), 124.0);
    }

    #[test]
    fn new_thread_selectors_restore_the_compact_floating_row() {
        assert_eq!(NEW_THREAD_SELECTOR_ROW_HEIGHT, 20.0);
        assert_eq!(SESSION_FOOTER_HEIGHT, 24.0);
    }

    #[test]
    fn route_chrome_crossfade_never_duplicates_picker_controls() {
        assert_eq!(route_chrome_opacities(1.0), (1.0, 0.0));
        assert_eq!(route_chrome_opacities(0.5), (0.0, 0.0));
        assert_eq!(route_chrome_opacities(0.0), (0.0, 1.0));
        for step in 0..=20 {
            let (new_thread, session) = route_chrome_opacities(step as f32 / 20.0);
            assert!(new_thread == 0.0 || session == 0.0);
        }
    }

    #[test]
    fn staged_comments_alone_are_content() {
        assert!(!composer_has_content("   ", 0, 0));
        assert!(composer_has_content("hi", 0, 0));
        assert!(composer_has_content("", 1, 0));
        assert!(composer_has_content("", 0, 1));
    }

    #[test]
    fn modified_submit_sends_content_and_activates_latest_queue_row_when_empty() {
        assert_eq!(
            modified_submit_target(composer_has_content("message", 0, 0)),
            ModifiedSubmitTarget::SubmitContent
        );
        assert_eq!(
            modified_submit_target(composer_has_content("", 1, 0)),
            ModifiedSubmitTarget::SubmitContent
        );
        assert_eq!(
            modified_submit_target(composer_has_content("", 0, 1)),
            ModifiedSubmitTarget::SubmitContent
        );
        assert_eq!(
            modified_submit_target(composer_has_content("  ", 0, 0)),
            ModifiedSubmitTarget::ActivateLatestQueued
        );
    }

    #[test]
    fn follow_up_setting_decides_steering_and_the_chord_flips_it_once() {
        assert!(!follow_up_steers(FollowUpBehavior::Queue, false));
        assert!(follow_up_steers(FollowUpBehavior::Queue, true));
        assert!(follow_up_steers(FollowUpBehavior::Steer, false));
        assert!(!follow_up_steers(FollowUpBehavior::Steer, true));
    }

    #[test]
    fn the_chord_only_inverts_while_enter_is_the_send_key() {
        assert!(modified_submit_inverts_follow_up(
            ComposerSendBehavior::Enter
        ));
        assert!(!modified_submit_inverts_follow_up(
            ComposerSendBehavior::ModEnter
        ));
    }

    #[test]
    fn a_comment_only_stage_queues_during_a_live_run() {
        let live = true;
        let comment_only = composer_has_content("", 0, 2);
        assert_eq!(
            send_button_mode(live, comment_only, false),
            SendButtonMode::Queue,
            "comment-only submit must queue without interrupting the run"
        );
        // Nothing staged at all is still the stop square.
        assert_eq!(
            send_button_mode(live, composer_has_content("", 0, 0), false),
            SendButtonMode::Stop
        );
    }

    #[test]
    fn send_button_morph() {
        assert_eq!(send_button_mode(false, false, false), SendButtonMode::Send);
        assert_eq!(send_button_mode(false, true, false), SendButtonMode::Send);
        assert_eq!(send_button_mode(true, true, false), SendButtonMode::Queue);
        assert_eq!(send_button_mode(true, false, false), SendButtonMode::Stop);
        // A stopped turn owns the empty circle; typing or a live run wins.
        assert_eq!(send_button_mode(false, false, true), SendButtonMode::Resume);
        assert_eq!(send_button_mode(false, true, true), SendButtonMode::Send);
        assert_eq!(send_button_mode(true, false, true), SendButtonMode::Stop);
    }

    #[test]
    fn queued_submit_does_not_publish_an_optimistic_transcript_echo() {
        assert!(should_publish_optimistic_echo(false));
        assert!(!should_publish_optimistic_echo(true));
    }

    #[test]
    fn interrupt_tracking_is_idempotent_per_chat() {
        let mut pending = HashSet::new();
        assert!(begin_interrupt(&mut pending, "chat-a"));
        assert!(!begin_interrupt(&mut pending, "chat-a"));
        assert!(begin_interrupt(&mut pending, "chat-b"));
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn interrupt_tracking_releases_only_settled_chats() {
        let mut pending = HashSet::from(["chat-a".to_string(), "chat-b".to_string()]);
        retain_live_interrupts(&mut pending, |chat_id| chat_id == "chat-b");
        assert_eq!(pending, HashSet::from(["chat-b".to_string()]));
        assert!(begin_interrupt(&mut pending, "chat-a"));
    }

    #[test]
    fn interrupt_payload_keeps_the_captured_chat() {
        let params = interrupt_params("chat-a");
        assert_eq!(params["chatId"], "chat-a");
        assert_eq!(params["command"]["kind"], "interrupt");
    }

    #[test]
    fn escape_consumers_keep_completion_and_wizard_priority() {
        assert!(escape_dismisses_completion("escape", true));
        assert!(!escape_dismisses_completion("escape", false));
        assert!(!escape_dismisses_completion("enter", true));

        assert!(wizard_escape_goes_back("escape", false, false));
        assert!(wizard_escape_goes_back("escape", true, true));
        assert!(!wizard_escape_goes_back("escape", true, false));
        assert!(!wizard_escape_goes_back("enter", false, true));
    }

    #[test]
    fn wizard_single_select_auto_advances_and_completes() {
        let mut w = Wizard::new(
            "req".into(),
            vec![
                question("q1", &["a", "b"], false),
                question("q2", &["x"], false),
            ],
        );
        assert_eq!(w.counter(), "1/2");
        assert_eq!(w.select(1), WizardStep::AutoAdvance);
        assert!(w.is_picked(1));
        assert_eq!(w.advance(), WizardStep::Stay);
        assert_eq!(w.counter(), "2/2");
        assert_eq!(w.select(0), WizardStep::AutoAdvance);
        let WizardStep::Done(answers) = w.advance() else {
            panic!("expected Done")
        };
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].labels, vec!["b"]);
        assert_eq!(answers[1].labels, vec!["x"]);
    }

    #[test]
    fn wizard_multi_select_toggles_and_stays() {
        let mut w = Wizard::new("req".into(), vec![question("q", &["a", "b", "c"], true)]);
        assert_eq!(w.select(0), WizardStep::Stay);
        assert_eq!(w.select(2), WizardStep::Stay);
        assert!(w.is_picked(0) && w.is_picked(2));
        // Toggle off.
        assert_eq!(w.select(0), WizardStep::Stay);
        assert!(!w.is_picked(0));
        let WizardStep::Done(answers) = w.advance() else {
            panic!()
        };
        assert_eq!(answers[0].labels, vec!["c"]);
    }

    #[test]
    fn wizard_editor_preserves_prefill_whitespace_and_empty_edits() {
        let mut q = question("editor", &[], false);
        q.multiline = true;
        q.prefill = Some("  first\nsecond\n".into());
        let mut w = Wizard::new("req".into(), vec![q]);
        assert_eq!(w.answers()[0].labels, vec!["  first\nsecond\n"]);
        w.set_typed(String::new());
        assert_eq!(w.answers()[0].labels, vec![""]);
    }

    #[test]
    fn wizard_number_keys_and_bounds() {
        let mut w = Wizard::new("req".into(), vec![question("q", &["a", "b"], false)]);
        assert_eq!(w.press_number(9), WizardStep::Stay, "out of range ignored");
        assert_eq!(w.press_number(0), WizardStep::Stay);
        assert_eq!(w.press_number(2), WizardStep::AutoAdvance);
        assert!(w.is_picked(1));
        assert_eq!(w.select(5), WizardStep::Stay, "bad option ix ignored");
    }

    #[test]
    fn wizard_typed_answer_overrides_and_back_pages() {
        let mut w = Wizard::new(
            "req".into(),
            vec![
                question("q1", &["a"], false),
                question("q2", &["x", "y"], false),
            ],
        );
        w.select(0);
        w.advance();
        assert_eq!(w.page, 1);
        assert!(w.back());
        assert_eq!(w.page, 0);
        assert!(!w.back(), "already at first page");
        w.advance();
        w.set_typed("  custom answer  ".into());
        let WizardStep::Done(answers) = w.advance() else {
            panic!()
        };
        assert_eq!(answers[0].labels, vec!["a"]);
        assert_eq!(
            answers[1].labels,
            vec!["custom answer"],
            "typed overrides picked, trimmed"
        );
    }

    #[gpui::test]
    fn wizard_note_viewport_grows_with_wrapped_lines(cx: &mut gpui::TestAppContext) {
        let (_dir, handle) = composer_focus_window(cx);
        handle
            .update(cx, |composer, _, cx| {
                composer.state.update(cx, |state, _| {
                    state.selected_chat = Some("chat".into());
                });
                composer.on_state_changed(cx);
                composer.expanded_mode = false;
                composer
                    .input
                    .update(cx, |input, cx| input.set_text("Hi", cx));
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        handle
            .read_with(cx, |composer, cx| {
                assert_eq!(
                    composer.input.read(cx).viewport_height,
                    Some(INPUT_LINE_HEIGHT),
                    "compact pill starts one line tall"
                );
            })
            .unwrap();
        handle
            .update(cx, |composer, _, cx| {
                composer.wizard = Some(Wizard::new(
                    "req".into(),
                    vec![question("q", &["a", "b"], false)],
                ));
                composer
                    .input
                    .update(cx, |input, cx| input.set_text("one\ntwo\nthree\nfour", cx));
            })
            .unwrap();
        for _ in 0..2 {
            cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
                .unwrap();
        }
        handle
            .read_with(cx, |composer, cx| {
                let input = composer.input.read(cx);
                let expected = wizard_input_height(input.content_height);
                assert!(expected >= 4.0 * INPUT_LINE_HEIGHT - 0.1);
                assert_eq!(input.viewport_height, Some(expected));
                assert_eq!(input.settled_viewport_height, Some(expected));
                assert_eq!(input.overflow_top_padding, 0.0);
            })
            .unwrap();
        assert_eq!(wizard_input_height(0.0), INPUT_LINE_HEIGHT);
        assert_eq!(wizard_input_height(1000.0), TEXTAREA_MAX - TEXTAREA_PAD_V);
    }

    #[test]
    fn a_consent_card_never_becomes_the_question_wizard() {
        let consent = UserInputQuestion {
            kind: Some(zeron_proto::UserInputKind::Consent {
                scope: zeron_proto::computer::ComputerConsentScope::Task,
                app: None,
                tool: None,
                params: None,
            }),
            ..question("consent", &["Allow", "Decline"], false)
        };
        let part = |request_id: &str, question: UserInputQuestion| MessagePart::Input {
            id: request_id.into(),
            request_id: request_id.into(),
            questions: vec![question],
            resolved: false,
            answers: None,
        };
        let entry = |parts: Vec<MessagePart>| SessionMessageEntry {
            id: "m".into(),
            role: MessageRole::Assistant,
            parts,
            created_at: 0,
            device_id: "d".into(),
            status: Some(zeron_doc::MessageStatus::Streaming),
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        };
        assert!(pending_input_request(&[entry(vec![part("c1", consent.clone())])]).is_none());
        let foreground = UserInputQuestion {
            kind: Some(zeron_proto::UserInputKind::Foreground { app: None }),
            ..question("foreground", &["Allow for this turn", "No"], false)
        };
        assert!(pending_input_request(&[entry(vec![part("f1", foreground)])]).is_none());
        let undecodable = MessagePart::Input {
            id: "u1".into(),
            request_id: "u1".into(),
            questions: Vec::new(),
            resolved: false,
            answers: None,
        };
        assert!(
            pending_input_request(&[entry(vec![undecodable])]).is_none(),
            "an Input part with no decodable questions never takes the composer"
        );
        assert_eq!(
            pending_input_request(&[entry(vec![
                part("c1", consent),
                part("r1", question("q", &["a"], false)),
            ])])
            .map(|(id, _)| id)
            .as_deref(),
            Some("r1")
        );
    }

    #[gpui::test]
    fn composition_drops_and_atomic_chip_edits_preserve_rich_draft_history(
        cx: &mut gpui::TestAppContext,
    ) {
        fn assert_ranges(input: &ComposerInput) {
            for range in std::iter::once(&input.selected_range).chain(input.marked_range.iter()) {
                assert!(range.start <= range.end && range.end <= input.content.len());
                assert!(input.content.is_char_boundary(range.start));
                assert!(input.content.is_char_boundary(range.end));
            }
            for (chip, display) in &input.projection.mentions {
                assert!(input.content.get(chip.range.clone()).is_some());
                assert!(input.projection.display.get(display.clone()).is_some());
            }
        }
        with_composer_input(cx, |input, window, cx| {
            let chip = local_file_link("src/café.rs", false);
            let raw = format!("**bold** e\u{301} 👨‍👩‍👧‍👦 {chip}\n`{chip}`\nactive ");
            input.set_text(&raw, cx);
            assert_eq!(input.projection.mentions.len(), 1);
            input.replace_and_mark_text_in_range(None, "かな", Some(0..1), window, cx);
            let composed = input.text().to_owned();
            let selection = input.selected_range.clone();
            let marked = input.marked_range.clone();
            let history = input.undo_stack.len();
            assert!(!input.insert_dropped_mention("src/dropped.rs", false, cx));
            assert_eq!(input.text(), composed);
            assert_eq!(input.selected_range, selection);
            assert_eq!(input.marked_range, marked);
            assert_eq!(input.undo_stack.len(), history);
            assert_ranges(input);
            input.replace_and_mark_text_in_range(None, "かんじ", Some(1..2), window, cx);
            assert_eq!(&input.content[input.selected_range.clone()], "ん");
            assert_ranges(input);
            input.replace_text_in_range(None, "漢字", window, cx);
            let committed = format!("{raw}漢字");
            assert_eq!(input.text(), committed);
            assert!(input.marked_range.is_none());
            assert_ranges(input);
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), raw);
            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), committed);
            assert_ranges(input);
            let link = input.projection.mentions[0].0.range.clone();
            let partial = input.range_to_utf16(&(link.start + 1..link.end - 1));
            input.replace_text_in_range(Some(partial), "🦀", window, cx);
            assert!(input.projection.mentions.is_empty());
            assert!(input.text().contains(&format!("`{chip}`")));
            assert_ranges(input);
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), committed);
            assert_eq!(input.projection.mentions.len(), 1);
            input.move_to(input.content.len(), cx);
            assert!(input.insert_dropped_mention("src/dropped.rs", false, cx));
            assert_eq!(input.projection.mentions.len(), 2);
            assert_ranges(input);
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), committed);
            assert_ranges(input);
        });
    }

    #[gpui::test]
    fn wizard_restores_displaced_draft_after_submit_navigation_and_timeout(
        cx: &mut gpui::TestAppContext,
    ) {
        let state = cx.new(|_| AppState::new());
        let composer = cx.new(|cx| Composer::new(state.clone(), cx));
        let transcript = |id: &str, resolved: bool| {
            let mut q = question("editor", &[], false);
            q.prefill = Some("  initial\ntext\n".into());
            q.multiline = true;
            vec![SessionMessageEntry {
                id: "assistant".into(),
                role: MessageRole::Assistant,
                parts: vec![MessagePart::Input {
                    id: "input".into(),
                    request_id: id.into(),
                    questions: vec![q],
                    resolved,
                    answers: None,
                }],
                created_at: 0,
                device_id: "device".into(),
                status: Some(zeron_doc::MessageStatus::Streaming),
                continuation_of: None,
                duration_ms: None,
                from_chat_id: None,
                fork_import: false,
                answers_request_id: None,
            }]
        };
        state.update(cx, |s, _| s.selected_chat = Some("a".into()));
        composer.update(cx, |c, cx| {
            c.on_state_changed(cx);
            c.input
                .update(cx, |i, cx| i.set_text("ordinary draft a", cx));
        });
        state.update(cx, |s, _| s.transcript = transcript("submit", false));
        composer.update(cx, |c, cx| {
            c.on_state_changed(cx);
            assert_eq!(c.input.read(cx).text(), "  initial\ntext\n");
            c.input.update(cx, |i, cx| i.set_text("answer", cx));
            c.wizard_advance(cx);
            assert_eq!(c.input.read(cx).text(), "ordinary draft a");
        });
        state.update(cx, |s, _| s.transcript = transcript("expires", false));
        composer.update(cx, |c, cx| {
            c.on_state_changed(cx);
            c.input.update(cx, |i, cx| i.set_text("partial answer", cx));
        });
        state.update(cx, |s, _| {
            s.selected_chat = Some("b".into());
            s.transcript.clear();
        });
        composer.update(cx, |c, cx| {
            c.on_state_changed(cx);
            assert!(c.input.read(cx).text().is_empty());
            c.input
                .update(cx, |i, cx| i.set_text("ordinary draft b", cx));
        });
        state.update(cx, |s, _| {
            s.selected_chat = Some("a".into());
            s.transcript = transcript("expires", false);
        });
        composer.update(cx, |c, cx| c.on_state_changed(cx));
        state.update(cx, |s, _| s.transcript = transcript("expires", true));
        composer.update(cx, |c, cx| {
            c.on_state_changed(cx);
            assert!(c.wizard.is_none());
            assert_eq!(c.input.read(cx).text(), "ordinary draft a");
            assert_eq!(
                crate::draft_store::entry("b", cx).prompt,
                "ordinary draft b"
            );
        });
    }

    #[test]
    fn pending_input_detection() {
        use zeron_doc::MessageStatus;
        let input_part = MessagePart::Input {
            id: "in-r1".into(),
            request_id: "r1".into(),
            questions: vec![question("q", &["a"], false)],
            resolved: false,
            answers: None,
        };
        let entry = |status: Option<MessageStatus>, parts: Vec<MessagePart>| SessionMessageEntry {
            id: "m".into(),
            role: MessageRole::Assistant,
            parts,
            created_at: 0,
            device_id: "d".into(),
            status,
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        };
        // Streaming entry with unresolved input → panel.
        let t = vec![entry(
            Some(MessageStatus::Streaming),
            vec![input_part.clone()],
        )];
        assert_eq!(
            pending_input_request(&t).map(|(id, _)| id),
            Some("r1".into())
        );
        // DEAD entry with an unresolved input STILL gets the panel: the
        // question stays answerable until answered (the engine delivers the
        // answer as a resumed turn), so a run reaped under its question —
        // engine restart — must not orphan it (user report).
        let t = vec![entry(
            Some(MessageStatus::Aborted),
            vec![input_part.clone()],
        )];
        assert_eq!(
            pending_input_request(&t).map(|(id, _)| id),
            Some("r1".into())
        );
        // A NEWER assistant entry supersedes an unanswered question.
        let t = vec![
            entry(Some(MessageStatus::Aborted), vec![input_part.clone()]),
            SessionMessageEntry {
                id: "m2".into(),
                role: MessageRole::Assistant,
                parts: vec![MessagePart::Text {
                    id: "t2".into(),
                    text: "moved on".into(),
                }],
                created_at: 2,
                device_id: "d".into(),
                status: Some(MessageStatus::Complete),
                continuation_of: None,
                from_chat_id: None,
                fork_import: false,
                answers_request_id: None,
                duration_ms: None,
            },
        ];
        assert!(pending_input_request(&t).is_none());
        // Resolved part → no panel.
        let resolved = MessagePart::Input {
            id: "in-r1".into(),
            request_id: "r1".into(),
            questions: vec![],
            resolved: true,
            answers: None,
        };
        let t = vec![entry(
            Some(MessageStatus::Streaming),
            vec![resolved.clone()],
        )];
        assert!(pending_input_request(&t).is_none());
        assert!(pending_input_request(&[]).is_none());

        // Regression (user forensics): a steer prompt appends a USER entry
        // AFTER the streaming assistant entry — the question must still be
        // found (a last-entry-only read vanished the panel exactly when the
        // user typed, bricking the answer flow).
        let user_echo = SessionMessageEntry {
            id: "u2".into(),
            role: MessageRole::User,
            parts: vec![MessagePart::Text {
                id: "t".into(),
                text: "I answered".into(),
            }],
            created_at: 1,
            device_id: "d".into(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        };
        let t = vec![
            entry(Some(MessageStatus::Streaming), vec![input_part.clone()]),
            user_echo,
        ];
        assert_eq!(
            pending_input_request(&t).map(|(id, _)| id),
            Some("r1".into()),
            "question survives entries appended behind the streaming entry"
        );

        // Latch release: only an explicitly resolved matching part releases.
        assert!(!input_request_resolved(&t, "r1"));
        let t = vec![entry(Some(MessageStatus::Streaming), vec![resolved])];
        assert!(input_request_resolved(&t, "r1"));
        assert!(!input_request_resolved(&t, "other"));
    }
}

#[cfg(test)]
mod appshot_rebase_tests {
    use super::*;
    use gpui::{AppContext, TestAppContext};

    #[gpui::test]
    fn appshot_only_draft_counts_as_content_and_removal_clears_it(cx: &mut TestAppContext) {
        let state = cx.new(|_| AppState::new());
        let composer = cx.new(|cx| Composer::new(state, cx));
        composer.update(cx, |composer, cx| {
            let shot = appshots::tests::shot();
            composer.stage_appshot(shot.clone(), cx);
            assert!(composer_has_content(
                "",
                composer.staged().len() + composer.staged_appshots().len(),
                0
            ));
            composer.remove_appshot(&shot.id, cx);
            assert!(composer.staged_appshots().is_empty());
            assert!(!composer_has_content(
                "",
                composer.staged().len() + composer.staged_appshots().len(),
                0
            ));
        });
    }

    #[gpui::test]
    fn failed_send_restores_complete_appshots_without_duplicates(cx: &mut TestAppContext) {
        let state = cx.new(|_| AppState::new());
        let composer = cx.new(|cx| Composer::new(state, cx));
        composer.update(cx, |composer, cx| {
            let original = appshots::tests::shot();
            let mut fresh = original.clone();
            fresh.id = "fresh".into();
            composer.stage_appshot_for("minted".into(), original.clone(), cx);
            composer.stage_appshot(fresh, cx);
            composer.restore_failed_appshots(&[original.clone()], "minted", "");
            assert_eq!(composer.staged_appshots().len(), 2);
            assert_eq!(
                composer.staged_appshots()[0].accessibility,
                original.accessibility
            );
            assert_eq!(
                composer.staged_appshots()[0].screenshot.id,
                original.screenshot.id
            );
            assert_eq!(composer.staged_appshots()[1].id, "fresh");
            assert!(!composer.appshots.contains_key("minted"));
        });
    }

    #[gpui::test]
    fn remote_queue_removal_recovers_both_appshot_drafts(cx: &mut TestAppContext) {
        let state = cx.new(|_| AppState::new());
        let composer = cx.new(|cx| Composer::new(state, cx));
        composer.update(cx, |composer, cx| {
            let original = appshots::tests::shot();
            let mut edited = original.clone();
            edited.id = "edited".into();
            composer.stage_appshot(edited, cx);
            composer.queue_edit_draft = Some(("original".into(), vec![], vec![original]));
            composer.editing_queued = Some("removed-row".into());
            composer
                .input
                .update(cx, |input, cx| input.set_text("edited", cx));
            composer.on_state_changed(cx);
            assert!(composer.editing_queued.is_none());
            assert_eq!(composer.staged_appshots().len(), 2);
            assert_eq!(composer.input.read(cx).text(), "original\n\nedited");
        });
    }
}

#[cfg(feature = "appshots-fixture")]
impl Composer {
    pub fn fixture_clear_appshots(&mut self, cx: &mut Context<Self>) {
        self.appshots.clear();
        cx.notify();
    }
    pub fn fixture_type(&mut self, text: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.set_text(text, cx));
        self.focus_pending = true;
        self.on_input_edited(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod computer_use_tests {
    use super::*;
    use gpui::TestAppContext;
    use zeron_proto::computer::ComputerPermission;

    fn command(name: &str) -> SlashCommand {
        SlashCommand {
            name: name.into(),
            description: "native".into(),
            input_hint: None,
            kind: SlashCommandKind::BuiltIn,
        }
    }

    #[test]
    fn computer_use_is_offered_where_the_catalog_exists_and_wins_a_native_collision() {
        let merged = with_computer_use_entry(
            HarnessId::Codex,
            vec![command("computer-use"), command("clear")],
        );
        let names: Vec<&str> = merged.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["computer-use", "clear"]);
        assert_eq!(
            merged[0].description,
            "Use Zeron Computer for this request only"
        );
        for harness in [
            HarnessId::Opencode,
            HarnessId::Cursor,
            HarnessId::Pi,
            HarnessId::Antigravity,
        ] {
            assert!(
                with_computer_use_entry(harness, vec![command("clear")])
                    .iter()
                    .all(|c| c.name != "computer-use")
            );
        }
        let all = with_sidechat_entry(with_compact_entry(with_computer_use_entry(
            HarnessId::ClaudeCode,
            Vec::new(),
        )));
        let names: Vec<&str> = all.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["sidechat", "compact", "computer-use"]);
    }

    #[test]
    fn the_preflight_runs_only_in_a_mac_ui_on_the_chats_host() {
        assert!(computer_preflight_applies(true, Some("mac"), Some("mac")));
        assert!(!computer_preflight_applies(false, Some("mac"), Some("mac")));
        assert!(!computer_preflight_applies(
            true,
            Some("other"),
            Some("mac")
        ));
        assert!(!computer_preflight_applies(true, Some("mac"), None));
        assert!(!computer_preflight_applies(true, None, None));
    }

    #[test]
    fn settings_on_sends_skip_the_grant_check() {
        use crate::appshots::{
            AppshotCapabilities, AppshotPlatform, CapabilityState, CaptureTarget,
        };
        let none_granted = AppshotCapabilities {
            platform: AppshotPlatform::MacOs,
            global_shortcut: CapabilityState::Ready,
            window_capture: CapabilityState::PermissionRequired,
            application_text: CapabilityState::PermissionRequired,
            input_monitoring: CapabilityState::Ready,
            target: CaptureTarget::ActiveWindow,
        };
        assert!(computer_send_blockers(true, none_granted).is_empty());
        assert_eq!(
            computer_send_blockers(false, none_granted),
            vec![
                ComputerPermission::Accessibility,
                ComputerPermission::ScreenRecording
            ]
        );
    }

    fn chat(device: &str) -> zeron_proto::Chat {
        serde_json::from_value(serde_json::json!({
            "id": "c",
            "deviceId": device,
            "archived": false,
            "createdAt": "2026-09-23T00:00:00Z"
        }))
        .expect("chat row")
    }

    struct Wire {
        frames: tokio::sync::mpsc::Receiver<String>,
    }

    impl Wire {
        fn methods(&mut self, cx: &mut TestAppContext) -> Vec<String> {
            cx.run_until_parked();
            let mut methods = Vec::new();
            while let Ok(frame) = self.frames.try_recv() {
                let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
                methods.push(frame["method"].as_str().unwrap_or_default().to_owned());
            }
            methods
        }
    }

    fn composer_on(cx: &mut TestAppContext, host: &str) -> (Entity<Composer>, Wire) {
        let (out, frames) = tokio::sync::mpsc::channel::<String>(16);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(16);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(
                zeron_rpc::RpcClient::new(out, inbound),
            ));
            state.chats = vec![chat(host)];
            state.selected_chat = Some("c".into());
            state.local_device_id = Some("mac".into());
        });
        // A closed inbound fails pending calls from a tokio thread.
        std::mem::forget(replies);
        let wire = Wire { frames };
        (cx.new(|cx| Composer::new(state, cx)), wire)
    }

    #[gpui::test]
    fn an_empty_computer_use_stays_in_the_composer(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (composer, mut wire) = composer_on(cx, "mac");
        composer.update(cx, |composer, cx| {
            composer
                .input
                .update(cx, |input, cx| input.set_text("/computer-use", cx));
            composer.send("/computer-use".into(), false, false, false, cx);
            assert_eq!(
                composer.failure.as_deref(),
                Some("Add a request after /computer-use.")
            );
            assert_eq!(composer.input.read(cx).text(), "/computer-use");
        });
        assert!(wire.methods(cx).is_empty());
    }

    #[gpui::test]
    fn a_local_computer_use_send_waits_on_the_settings_read(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (composer, mut wire) = composer_on(cx, "mac");
        let text = "/computer-use open Calculator";
        composer.update(cx, |composer, cx| {
            composer
                .input
                .update(cx, |input, cx| input.set_text(text, cx));
            composer.send(text.into(), false, false, false, cx);
            composer.send(text.into(), false, false, false, cx);
        });
        let methods = wire.methods(cx);
        if cfg!(target_os = "macos") {
            assert_eq!(methods, vec![methods::GET_COMPUTER_SETTINGS.to_string()]);
            composer.update(cx, |composer, cx| {
                assert_eq!(composer.input.read(cx).text(), text, "the draft waits");
            });
        } else {
            assert!(methods.contains(&methods::QUEUE_COMMAND.to_string()));
        }
    }

    #[gpui::test]
    fn a_remote_chat_sends_without_the_preflight(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (composer, mut wire) = composer_on(cx, "other-mac");
        let text = "/computer-use open Calculator";
        composer.update(cx, |composer, cx| {
            composer
                .input
                .update(cx, |input, cx| input.set_text(text, cx));
            composer.send(text.into(), false, false, false, cx);
        });
        let methods = wire.methods(cx);
        assert!(!methods.contains(&methods::GET_COMPUTER_SETTINGS.to_string()));
        assert!(methods.contains(&methods::QUEUE_COMMAND.to_string()));
    }

    #[gpui::test]
    fn the_strip_shows_while_this_chat_drives_and_stop_calls_revoke_desktop(
        cx: &mut TestAppContext,
    ) {
        use crate::computer::preview::{self, tests::thread_state};
        use zeron_proto::computer::ComputerEvent;

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
            preview::init(cx);
        });
        let (out, frames) = tokio::sync::mpsc::channel::<String>(16);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(16);
        std::mem::forget(replies);
        let mut wire = Wire { frames };
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(
                zeron_rpc::RpcClient::new(out, inbound),
            ));
            state.chats = vec![chat("mac")];
            state.selected_chat = Some("c".into());
            state.local_device_id = Some("mac".into());
        });
        let (composer, cx) = cx.add_window_view(|_, cx| Composer::new(state, cx));
        let store = cx.update(|_, cx| preview::store(cx).unwrap());
        let apply = |cx: &mut gpui::VisualTestContext, event: ComputerEvent| {
            cx.update(|_, cx| preview::update_store(&store, cx, |store| store.apply_event(&event)));
            cx.run_until_parked();
        };
        let strip_text = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, cx| {
                crate::computer::presence::for_chat("c", cx)
                    .map(|presence| crate::computer::strip::strip_copy(&presence).text())
            })
        };

        apply(
            cx,
            ComputerEvent::ThreadState {
                state: Box::new(thread_state("other", true)),
            },
        );
        assert!(
            cx.debug_bounds("computer-strip").is_none(),
            "another chat's drive never shows here"
        );

        let mut typing = thread_state("c", true);
        typing.activity = Some("Typing".into());
        apply(
            cx,
            ComputerEvent::ThreadState {
                state: Box::new(typing),
            },
        );
        apply(
            cx,
            ComputerEvent::Action {
                window_id: Some("w1".into()),
                delivery: None,
                action: "computer_type_text".into(),
                ok: true,
                message: None,
                thread_id: Some("c".into()),
                restored_window_id: None,
                restore_status: None,
                masked: false,
            },
        );
        assert!(cx.debug_bounds("computer-strip").is_some());
        assert_eq!(strip_text(cx).as_deref(), Some("Calculator · Typing"));
        wire.methods(cx);
        let stop = cx
            .debug_bounds("computer-strip-stop")
            .expect("Stop while driving");
        cx.simulate_click(stop.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        let mut revokes = Vec::new();
        while let Ok(frame) = wire.frames.try_recv() {
            let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
            if frame["method"] == methods::COMPUTER_REVOKE_DESKTOP {
                revokes.push(frame["params"].clone());
            }
        }
        assert_eq!(revokes, vec![serde_json::json!({ "chatId": "c" })]);

        let mut revoked = thread_state("c", false);
        revoked.input_stopped = Some(true);
        apply(
            cx,
            ComputerEvent::ThreadState {
                state: Box::new(revoked),
            },
        );
        assert_eq!(
            strip_text(cx).as_deref(),
            Some("Desktop access revoked · the agent is summing up")
        );
        assert!(cx.debug_bounds("computer-strip-stop").is_none());
        apply(
            cx,
            ComputerEvent::ThreadState {
                state: Box::new(thread_state("c", false)),
            },
        );
        assert_eq!(strip_text(cx).as_deref(), Some("Desktop access revoked"));
        assert!(cx.debug_bounds("computer-strip").is_some());

        composer.update(cx, |composer, cx| {
            composer
                .input
                .update(cx, |input, cx| input.set_text("next", cx));
            composer.send("next".into(), false, false, false, cx);
        });
        cx.run_until_parked();
        assert_eq!(strip_text(cx), None, "a send clears the latch");
        assert!(cx.debug_bounds("computer-strip").is_none());
    }

    #[gpui::test]
    fn the_foreground_row_answers_through_respond_input_and_guards_itself(cx: &mut TestAppContext) {
        use crate::computer::preview::{self, tests::thread_state};
        use zeron_proto::computer::{ComputerEvent, ComputerPendingKind, ComputerPendingRequest};

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
            preview::init(cx);
        });
        let (out, frames) = tokio::sync::mpsc::channel::<String>(16);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(16);
        std::mem::forget(replies);
        let mut wire = Wire { frames };
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(
                zeron_rpc::RpcClient::new(out, inbound),
            ));
            state.chats = vec![chat("mac")];
            state.selected_chat = Some("c".into());
            state.local_device_id = Some("mac".into());
        });
        let (_composer, cx) = cx.add_window_view(|_, cx| Composer::new(state, cx));
        let store = cx.update(|_, cx| preview::store(cx).unwrap());
        let apply = |cx: &mut gpui::VisualTestContext, event: ComputerEvent| {
            cx.update(|_, cx| preview::update_store(&store, cx, |store| store.apply_event(&event)));
            cx.run_until_parked();
        };

        let mut asking = thread_state("c", false);
        asking.pending_request = Some(ComputerPendingRequest {
            input_id: "computer-foreground-7".into(),
            question_id: "foreground".into(),
            kind: ComputerPendingKind::Foreground {
                app: Some("Ghostty".into()),
            },
        });
        apply(
            cx,
            ComputerEvent::ThreadState {
                state: Box::new(asking),
            },
        );
        assert!(cx.debug_bounds("computer-strip").is_some());
        assert!(cx.debug_bounds("computer-strip-stop").is_none());
        assert!(
            crate::computer::protected_surfaces::any_showing(),
            "the agent can never click its own Allow"
        );
        wire.methods(cx);
        let allow = cx
            .debug_bounds("computer-strip-allow-foreground")
            .expect("Allow for this turn");
        cx.simulate_click(allow.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        let mut answers = Vec::new();
        while let Ok(frame) = wire.frames.try_recv() {
            let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
            if frame["method"] == methods::QUEUE_COMMAND {
                answers.push(frame["params"].clone());
            }
        }
        assert_eq!(answers.len(), 1, "{answers:?}");
        assert_eq!(answers[0]["chatId"], serde_json::json!("c"));
        let command = serde_json::to_value(SessionCommandPayload::RespondInput {
            request_id: "computer-foreground-7".into(),
            answers: vec![UserInputAnswer {
                question_id: "foreground".into(),
                labels: vec!["Allow for this turn".into()],
            }],
        })
        .unwrap();
        assert_eq!(answers[0]["command"], command);

        apply(
            cx,
            ComputerEvent::ThreadState {
                state: Box::new(thread_state("c", true)),
            },
        );
        assert!(cx.debug_bounds("computer-strip-allow-foreground").is_none());
        assert!(!crate::computer::protected_surfaces::any_showing());
    }

    #[gpui::test]
    fn the_strip_consent_is_a_protected_surface(cx: &mut TestAppContext) {
        use crate::computer::preview::{self, tests::thread_state};
        use zeron_proto::computer::{ComputerConsentScope, ComputerEvent};

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
            preview::init(cx);
        });
        let (out, frames) = tokio::sync::mpsc::channel::<String>(16);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(16);
        std::mem::forget(replies);
        let mut wire = Wire { frames };
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(
                zeron_rpc::RpcClient::new(out, inbound),
            ));
            state.chats = vec![chat("mac")];
            state.selected_chat = Some("c".into());
            state.local_device_id = Some("mac".into());
        });
        let (_composer, cx) = cx.add_window_view(|_, cx| Composer::new(state, cx));
        let store = cx.update(|_, cx| preview::store(cx).unwrap());
        let apply = |cx: &mut gpui::VisualTestContext, event: ComputerEvent| {
            cx.update(|_, cx| preview::update_store(&store, cx, |store| store.apply_event(&event)));
            cx.run_until_parked();
        };

        let mut asking = thread_state("c", false);
        asking.pending_request = Some(crate::computer::strip::tests::consent_pending(
            ComputerConsentScope::Task,
            Some("Ghostty"),
            true,
        ));
        apply(
            cx,
            ComputerEvent::ThreadState {
                state: Box::new(asking),
            },
        );
        assert!(cx.debug_bounds("computer-strip").is_some());
        assert!(cx.debug_bounds("computer-strip-stop").is_none());
        assert!(cx.debug_bounds("computer-strip-allow-consent").is_some());
        assert!(cx.debug_bounds("computer-strip-decline-consent").is_some());
        assert!(
            crate::computer::protected_surfaces::any_showing(),
            "the agent can never click its own Allow"
        );
        wire.methods(cx);
        let always = cx
            .debug_bounds("computer-strip-always-consent")
            .expect("Always allow Ghostty");
        cx.simulate_click(always.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        let mut answers = Vec::new();
        while let Ok(frame) = wire.frames.try_recv() {
            let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
            if frame["method"] == methods::QUEUE_COMMAND {
                answers.push(frame["params"].clone());
            }
        }
        assert_eq!(answers.len(), 1, "{answers:?}");
        assert_eq!(answers[0]["chatId"], serde_json::json!("c"));
        let command = serde_json::to_value(SessionCommandPayload::RespondInput {
            request_id: "computer-consent-1".into(),
            answers: vec![UserInputAnswer {
                question_id: "consent".into(),
                labels: vec!["Always allow Ghostty".into()],
            }],
        })
        .unwrap();
        assert_eq!(answers[0]["command"], command);

        apply(
            cx,
            ComputerEvent::ThreadState {
                state: Box::new(thread_state("c", true)),
            },
        );
        assert!(cx.debug_bounds("computer-strip-allow-consent").is_none());
        assert!(!crate::computer::protected_surfaces::any_showing());
    }

    #[gpui::test]
    fn enable_prefixes_the_draft_once(cx: &mut TestAppContext) {
        let state = cx.new(|_| AppState::new());
        let composer = cx.new(|cx| Composer::new(state, cx));
        composer.update(cx, |composer, cx| {
            composer.enable_computer_use(cx);
            assert_eq!(composer.input.read(cx).text(), "/computer-use ");
            composer
                .input
                .update(cx, |input, cx| input.set_text("open Calculator", cx));
            composer.enable_computer_use(cx);
            assert_eq!(
                composer.input.read(cx).text(),
                "/computer-use open Calculator"
            );
            composer.enable_computer_use(cx);
            assert_eq!(
                composer.input.read(cx).text(),
                "/computer-use open Calculator"
            );
        });
    }
}

#[cfg(test)]
#[path = "composer_dictation_tests.rs"]
mod dictation_tests;
