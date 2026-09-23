//! select_text tool — exact-range text selection through `AXSelectedTextRange`.
//!
//! Writes a CFRange on the resolved element's `AXSelectedTextRange` attribute
//! and verifies the selection by reading the attribute back. There is no
//! keyboard, pointer, triple-click, or select-all fallback: a target whose
//! selection attribute is not settable — including most web content, which
//! Chromium and WebKit expose through `AXSelectedTextMarkerRange` rather than
//! the plain range attribute — is refused before dispatch.
//!
//! `start`/`length` are UTF-16 code-unit offsets, the same indexing
//! `AXSelectedTextRange`, Swift `String`, and JavaScript string offsets use.
//! A `length` of `0` places a collapsed caret at `start`. A range that runs
//! past the element's readable value is refused rather than clamped; when the
//! value is not readable, the post-write read-back is the sole arbiter.

use async_trait::async_trait;
use cua_driver_core::{
    protocol::ToolResult,
    tool::{Tool, ToolDef},
};
use serde_json::Value;
use std::sync::Arc;

use crate::apps;
use crate::ax::bindings::{
    copy_range_attr, copy_string_attr, copy_stringish_attr, is_attribute_settable, kAXErrorSuccess,
    set_range_attr, AXUIElementRef,
};
use crate::focus_guard;
use crate::input::ax_actions::{action_failure_result, AxActionFailure};
use crate::window_change_detector::WindowChangeDetector;

use super::ToolState;

pub struct SelectTextTool {
    state: Arc<ToolState>,
}

impl SelectTextTool {
    pub fn new(state: Arc<ToolState>) -> Self {
        Self { state }
    }
}

static DEF: std::sync::OnceLock<ToolDef> = std::sync::OnceLock::new();

fn def() -> &'static ToolDef {
    DEF.get_or_init(|| ToolDef {
        name: "select_text".into(),
        description:
            "Select an exact character range in a text element through \
             AXSelectedTextRange. Offsets count UTF-16 code units of the \
             element's complete, untrimmed AXValue (an emoji outside the Basic \
             Multilingual Plane counts as 2; a range that splits one is refused \
             with range_splits_character); `length` 0 places a collapsed caret at \
             `start`. When AXValue is readable the result carries the text that \
             range covers as `selected_text`. The selection is confirmed \
             only by an attribute read-back that reports the requested range. \
             \n\
             Elements without a settable AXSelectedTextRange — including most \
             web content, which exposes selection through \
             AXSelectedTextMarkerRange instead — are refused before dispatch. \
             No triple-click, select-all, key-event, or pointer fallback is \
             ever attempted."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["pid", "start", "length"],
            "properties": {
                "session": { "type": "string", "description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session." },
                "pid": { "type": "integer" },
                "window_id": {
                    "type": "integer",
                    "description": "CGWindowID for the window whose get_window_state produced the element_index. Required when element_index is used; optional when element_token is supplied (the token carries it)."
                },
                "element_index": cua_driver_core::tool_schema::element_index_schema(),
                "element_token": cua_driver_core::tool_schema::element_token_schema(),
                "snapshot_id": cua_driver_core::tool_schema::snapshot_id_schema(),
                "start": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Zero-based UTF-16 code-unit offset at which the selection starts."
                },
                "length": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Number of UTF-16 code units to select. 0 collapses to a caret at `start`."
                }
            },
            "additionalProperties": false
        }),
        read_only:   false,
        destructive: true,
        idempotent:  false,
        open_world:  true,
    })
}

#[async_trait]
impl Tool for SelectTextTool {
    fn def(&self) -> &ToolDef {
        def()
    }

    async fn invoke(&self, args: Value) -> ToolResult {
        use cua_driver_core::tool_args::ArgsExt;
        let pid = match args.require_i32("pid") {
            Ok(v) => v,
            Err(e) => return e,
        };
        let start = match args.require_u64("start") {
            Ok(v) => v,
            Err(e) => return e,
        };
        let length = match args.require_u64("length") {
            Ok(v) => v,
            Err(e) => return e,
        };
        // CFIndex (the range field width) is i64; refuse a range that cannot
        // be represented instead of truncating it into a different selection.
        let (start, length) = match start
            .checked_add(length)
            .filter(|end| *end <= i64::MAX as u64)
        {
            Some(_) => (start as i64, length as i64),
            None => {
                return ToolResult::error(
                    "select_text start + length exceeds the CFIndex width of \
                     AXSelectedTextRange; the request is refused rather than truncated.",
                )
            }
        };

        // Element addressing follows set_value exactly: element_token or
        // element_index + window_id, resolved through the live token cache so
        // a stale snapshot row is refused before any AX traffic.
        let element_token_arg = args.opt_str("element_token");
        let window_id_arg = args.opt_u64("window_id");
        let element_index_arg = args.opt_u64("element_index").map(|v| v as usize);
        let resolved = match self.state.element_cache.resolve_element_args(
            pid,
            element_index_arg,
            element_token_arg.as_deref(),
            args.opt_str("snapshot_id").as_deref(),
            window_id_arg,
            "select_text",
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let (element_index, window_id, element_guard) = match resolved {
            cua_driver_core::element_token::ResolvedElement::None => {
                return ToolResult::error(
                    "select_text requires element_index (+ window_id) or element_token to \
                     address the target element.",
                )
            }
            cua_driver_core::element_token::ResolvedElement::Element {
                window_id: Some(wid),
                element_index: idx,
                element,
                ..
            } => match u32::try_from(wid) {
                Ok(wid) => (idx, wid, element),
                Err(_) => return ToolResult::error("window_id is out of range for macOS."),
            },
            cua_driver_core::element_token::ResolvedElement::Element {
                window_id: None, ..
            } => {
                return ToolResult::error(
                    "select_text requires window_id when element_index is used \
                 (omit only when supplying element_token, which carries it).",
                )
            }
        };

        let element_ptr = element_guard.as_ptr();

        // Always-background semantic AX mutation. Re-prove that the retained
        // element still belongs to the requested exact window immediately
        // before any cursor or AX work.
        let _mutation_lease = match super::gate_background_window_action(
            pid,
            window_id,
            Some(element_ptr),
            cua_driver_core::background_input::BackgroundAction::AxSemantic,
        )
        .await
        {
            Ok(lease) => lease,
            Err(refusal_result) => return refusal_result,
        };

        let cursor_key = super::cursor_tools::resolve_cursor_key(&args);
        let center_guard = element_guard.clone();
        if let Ok(Some((screen_x, screen_y))) = tokio::task::spawn_blocking(move || unsafe {
            crate::ax::bindings::element_screen_center(center_guard.as_ptr() as AXUIElementRef)
        })
        .await
        {
            crate::cursor::overlay::send_command(
                cursor_key.clone(),
                cursor_overlay::OverlayCommand::PinAbove(window_id as u64),
            );
            crate::cursor::overlay::animate_cursor_to(cursor_key.clone(), screen_x, screen_y).await;
            self.state
                .cursor_registry
                .update_position(&cursor_key, screen_x, screen_y);
        }

        // An AXSelectedTextRange read-back is not ground truth for web
        // content either: an AXWebArea can echo the write through
        // accessibility while the renderer never selects. Reuse type_text's
        // bounded ancestor check so native browser chrome stays trusted but
        // rendered content is always reported as unverified.
        let ax_echo_surface = super::type_text::target_in_web_area(
            pid,
            Some((element_ptr, Some(element_index))),
            Some(window_id),
        );

        // Selection writes can cause reflex activations in some apps; apply
        // the same focus-suppression wrap set_value uses around AX writes.
        let prior_front = apps::frontmost_pid();
        let snapshot = WindowChangeDetector::snapshot(prior_front);

        let result = focus_guard::with_focus_suppressed(
            Some(pid),
            prior_front,
            "select_text.AXSelectedTextRange",
            || async move {
                tokio::task::spawn_blocking(move || {
                    select_text_blocking(element_ptr, element_index, start, length)
                })
                .await
            },
        )
        .await;

        let changes = snapshot.detect_async().await;

        match result {
            Ok(Ok(mut outcome)) => {
                apply_surface_trust(&mut outcome, ax_echo_surface);
                apply_verification_label(&mut outcome);
                let mut msg = outcome.detail;
                msg.push_str(&changes.result_suffix());
                let verified = outcome.verified.unwrap_or(false);
                let mut structured = serde_json::json!({
                    "path": "ax",
                    "verified": verified,
                    "effect": if verified {
                        "confirmed"
                    } else if outcome.verified == Some(false) {
                        "unconfirmed"
                    } else {
                        "unverifiable"
                    },
                });
                if verified {
                    structured["evidence"] = serde_json::json!([{ "kind": "value_readback" }]);
                }
                if ax_echo_surface {
                    structured["observation_required"] = serde_json::json!(true);
                }
                if let Some(text) = outcome.selected_text {
                    structured["selected_text"] = serde_json::json!(text);
                }
                ToolResult::text(msg).with_structured(structured)
            }
            Ok(Err(e)) => action_failure_result(e, "select_text failed"),
            Err(e) => action_failure_result(e.into(), "select_text task failed"),
        }
    }
}

// ── Blocking implementation (runs on spawn_blocking thread) ─────────────────

/// Outcome of a `select_text` write.
///
/// `verified` is `Some(true)` only when the post-write
/// `AXSelectedTextRange` read-back reports exactly the requested range —
/// a successful `AXUIElementSetAttributeValue` return code is not by itself
/// evidence that the selection landed. `Some(false)` means a read-back ran
/// and returned a different range; `None` means the attribute could not be
/// read back at all.
struct SelectTextOutcome {
    detail: String,
    verified: Option<bool>,
    selected_text: Option<String>,
}

fn apply_surface_trust(outcome: &mut SelectTextOutcome, ax_echo_surface: bool) {
    if ax_echo_surface && outcome.verified == Some(true) {
        outcome.verified = Some(false);
        outcome.detail.push_str(
            " AXSelectedTextRange read-back is not trusted for web content; \
             verify the renderer via screenshot or use the browser page tools.",
        );
    }
}

fn apply_verification_label(outcome: &mut SelectTextOutcome) {
    if outcome.verified != Some(true) {
        if let Some(rest) = outcome.detail.strip_prefix("✅ Selected") {
            outcome.detail = format!("📨 Sent (unverified){rest}");
        }
    }
}

fn select_text_blocking(
    element_ptr: usize,
    element_index: usize,
    start: i64,
    length: i64,
) -> anyhow::Result<SelectTextOutcome> {
    let element = element_ptr as AXUIElementRef;

    let role = unsafe { copy_string_attr(element, "AXRole") }.unwrap_or_default();

    // Gate on settability up front: a CFRange write to an element without a
    // settable selection range is refused rather than attempted and reported
    // uncertain. Elements that expose selection only through
    // AXSelectedTextMarkerRange fail this check, which is the documented
    // refusal for most web content.
    if !unsafe { is_attribute_settable(element, "AXSelectedTextRange") } {
        return Err(AxActionFailure::refused(
            "AXSelectedTextRange is not settable on this element. Elements that expose \
             selection only through AXSelectedTextMarkerRange — most web content — are \
             not supported, and no keyboard, pointer, or select-all fallback exists.",
        )
        .into());
    }

    // Bounds-check against the element's readable value: a range past the end
    // is a bad request, refused before dispatch rather than a write the app
    // may silently clamp. When AXValue is not readable the post-write
    // read-back is the sole arbiter and a mismatch reports unconfirmed.
    let selected_text = unsafe { copy_stringish_attr(element, "AXValue") }
        .and_then(|value| value.string_value)
        .map(|value| range_text(&value, start, length))
        .transpose()?;

    (|| -> anyhow::Result<()> {
        crate::input::ax_actions::ensure_ax_action_enabled(element_ptr, "AXSelectedTextRange")?;
        crate::input::cancellation::gate().check_target()
    })()
    .map_err(AxActionFailure::before_dispatch)?;

    let error = unsafe { set_range_attr(element, "AXSelectedTextRange", start, length) };
    if error != kAXErrorSuccess {
        return Err(AxActionFailure::uncertain(format!(
            "AXSelectedTextRange write returned {error} after dispatch; no alternate \
             write was submitted. Observe fresh state; do not replay."
        ))
        .into());
    }

    let after = unsafe { copy_range_attr(element, "AXSelectedTextRange") };
    let verified = after.map(|(location, len)| location == start && len == length);
    let suffix = match verified {
        Some(true) => "",
        Some(false) => " Read-back reported a different range; observe fresh state; do not replay.",
        None => " Selection range is not readable through AX; observe fresh state; do not replay.",
    };
    Ok(SelectTextOutcome {
        detail: format!(
            "✅ Selected {length} characters at {start} on [{element_index}] {role}.{suffix}"
        ),
        verified,
        selected_text,
    })
}

fn range_text(value: &str, start: i64, length: i64) -> anyhow::Result<String> {
    let units: Vec<u16> = value.encode_utf16().collect();
    let count = units.len() as i64;
    let end = start.checked_add(length);
    if start < 0 || length < 0 || start > count || end.is_none_or(|end| end > count) {
        return Err(AxActionFailure::refused(format!(
            "range start={start} length={length} exceeds the element's {count} \
             UTF-16 code units; the request is refused rather than clamped."
        ))
        .into());
    }
    let (start, end) = (start as usize, (start + length) as usize);
    let splits = |at: usize| {
        units
            .get(at)
            .is_some_and(|unit| (0xDC00..0xE000).contains(unit))
    };
    if splits(start) || splits(end) {
        return Err(AxActionFailure::refused_with_code(
            "range_splits_character",
            "The range starts or ends inside a character that takes two UTF-16 code units \
             (most emoji and other characters outside the Basic Multilingual Plane count \
             as 2); nothing was selected. Recount the offsets in UTF-16 code units.",
        )
        .into());
    }
    Ok(String::from_utf16_lossy(&units[start..end]))
}

#[cfg(test)]
mod tests {
    use super::{range_text, AxActionFailure};

    #[test]
    fn a_range_is_read_from_the_untrimmed_value_in_utf16_units() {
        assert_eq!(range_text("\n\nTotal: 10", 9, 2).unwrap(), "10");
        assert_eq!(range_text("a🦀b", 1, 2).unwrap(), "🦀");
        assert_eq!(range_text("a🦀b", 3, 1).unwrap(), "b");
        assert_eq!(range_text("abc", 3, 0).unwrap(), "");
        assert!(range_text("abc", 1, i64::MAX).is_err());
        assert!(range_text("abc", -1, 1).is_err());
    }

    #[test]
    fn a_range_that_splits_a_surrogate_pair_or_overruns_is_refused() {
        for (start, length) in [(2, 1), (1, 1), (0, 5), (5, 0)] {
            let error = range_text("a🦀b", start, length).unwrap_err();
            assert!(!error.downcast_ref::<AxActionFailure>().unwrap().attempted);
        }
        let error = range_text("a🦀b", 2, 1).unwrap_err();
        assert_eq!(
            error.downcast_ref::<AxActionFailure>().unwrap().code(),
            Some("range_splits_character")
        );
    }
}
