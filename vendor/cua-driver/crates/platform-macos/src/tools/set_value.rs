//! set_value tool — matches the Swift reference in SetValueTool.swift.
//!
//! Two modes, determined by the element's AXRole:
//!
//! * **AXPopUpButton**: Find the child option whose AXTitle or AXValue matches
//!   `value` (case-insensitive) and AXPress it directly.  The native macOS popup
//!   menu is never opened. A popup without an exact AX child is refused; it
//!   never mutates an unrelated frontmost browser document.
//!
//! * **Everything else**: Write `AXValue` directly (sliders, steppers, native
//!   text fields that expose a settable AXValue).

use async_trait::async_trait;
use cua_driver_core::{
    protocol::ToolResult,
    tool::{Tool, ToolDef},
};
use serde_json::Value;
use std::sync::Arc;

use crate::apps;
use crate::ax::bindings::{
    copy_action_names, copy_children, copy_number_attr, copy_string_attr, is_attribute_settable,
    kAXErrorSuccess, perform_action, set_number_attr, set_string_attr, AXError, AXUIElementRef,
};
use crate::focus_guard;
use crate::input::ax_actions::{action_failure_result, AxActionFailure};
use crate::window_change_detector::WindowChangeDetector;
use core_foundation::base::CFRelease;

use super::ToolState;

pub struct SetValueTool {
    state: Arc<ToolState>,
}

impl SetValueTool {
    pub fn new(state: Arc<ToolState>) -> Self {
        Self { state }
    }
}

static DEF: std::sync::OnceLock<ToolDef> = std::sync::OnceLock::new();

fn def() -> &'static ToolDef {
    DEF.get_or_init(|| ToolDef {
        name: "set_value".into(),
        description:
            "Set a value on a UI element. Two modes depending on element role:\n\
             \n\
             - **AXPopUpButton / select dropdown**: finds the child option whose \
             title or value matches `value` (case-insensitive) and AXPresses it \
             directly. Requires a matching AX child in the exact target; a \
             popup with no AX children is refused.\n\
             \n\
             - **All other elements**: writes AXValue directly (sliders, steppers, \
             date pickers, native text fields that expose settable AXValue). \
             Unsettable numeric controls may use advertised stepping actions. \
             The route is selected before dispatch; an uncertain effect never \
             permits another write or a fallback, except that a numeric write whose \
             read-back does not match is set once more to the same absolute value as \
             a string (web number inputs accept only strings).\n\
             \n\
             Optional append:true requires an exact element_token and a readable, \
             writable text target. It reads that retained element's current value \
             and appends value within the same native action, without replacing \
             the snapshot or guessing another element.\n\n\
             For free-form text entry into web inputs, prefer `type_text_chars` \
             which synthesises key events — AXValue writes are ignored by WebKit."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["pid", "value"],
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
                "value": {
                    "type": "string",
                    "description": "New value. Numeric controls require a finite number; text preserves the exact string."
                },
                "append": {
                    "type": "boolean",
                    "default": false,
                    "description": "Append to the exact retained text element's current AXValue. Requires element_token; refuses secure, unreadable, or nontext targets and combined text exceeding 16384 UTF-16 code units. No alternate route or replay."
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
impl Tool for SetValueTool {
    fn def(&self) -> &ToolDef {
        def()
    }

    async fn invoke(&self, args: Value) -> ToolResult {
        use cua_driver_core::tool_args::ArgsExt;
        let pid = match args.require_i32("pid") {
            Ok(v) => v,
            Err(e) => return e,
        };
        let value = match args.require_str("value") {
            Ok(v) => v,
            Err(e) => return e,
        };

        // Surface 6: element_token / element_index precedence. Neither
        // is now schema-required so the resolver can centralize the
        // "missing addressing" error message.
        let element_token_arg = args.opt_str("element_token");
        let append = args.bool_or("append", false);
        if append && element_token_arg.is_none() {
            return action_failure_result(
                AxActionFailure::refused_with_code(
                    "exact_element_token_required",
                    "set_value append requires the original exact element_token",
                )
                .into(),
                "set_value refused",
            );
        }
        let window_id_arg = args.opt_u64("window_id");
        let element_index_arg = args.opt_u64("element_index").map(|v| v as usize);
        let resolved = match self.state.element_cache.resolve_element_args(
            pid,
            element_index_arg,
            element_token_arg.as_deref(),
            args.opt_str("snapshot_id").as_deref(),
            window_id_arg,
            "set_value",
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let (element_index, window_id, element_guard) = match resolved {
            cua_driver_core::element_token::ResolvedElement::None => {
                return ToolResult::error(
                    "set_value requires element_index (+ window_id) or element_token to \
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
                    "set_value requires window_id when element_index is used \
                 (omit only when supplying element_token, which carries it).",
                )
            }
        };

        let element_ptr = element_guard.as_ptr();

        // set_value is an always-background semantic AX mutation. Re-prove
        // that the retained element still belongs to the requested exact
        // window immediately before any cursor or AX work; a cache hit alone
        // is not delivery proof after a window lifecycle or Space change.
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
        // An AXValue read-back is not ground truth for web content. Chromium,
        // WebKit, and Electron can echo the write through accessibility while
        // the renderer never observes it. Reuse type_text's bounded ancestor
        // check so native browser chrome stays trusted but rendered content is
        // always reported as unverified.
        let ax_echo_surface = super::type_text::target_in_web_area(
            pid,
            Some((element_ptr, Some(element_index))),
            Some(window_id),
        );

        // ── Focus-suppression wrap (Swift WindowChangeDetector + FocusGuard) ──
        // AXValue writes on popups / sliders can cause reflex activations
        // in Chromium-based apps; the AXPopUpButton path also AXPresses a
        // child option which can trigger app activation in some setups.
        let prior_front = apps::frontmost_pid();
        let snapshot = WindowChangeDetector::snapshot(prior_front);
        let web_content = ax_echo_surface;

        let result = focus_guard::with_focus_suppressed(
            Some(pid),
            prior_front,
            "set_value.AXValue",
            || async move {
                tokio::task::spawn_blocking(move || {
                    set_value_blocking(element_ptr, element_index, &value, append, web_content)
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
                    "effect": if verified { "confirmed" } else { "unverifiable" },
                });
                if verified {
                    structured["evidence"] = serde_json::json!([{ "kind": "value_readback" }]);
                }
                if ax_echo_surface {
                    structured["observation_required"] = serde_json::json!(true);
                }
                ToolResult::text(msg).with_structured(structured)
            }
            Ok(Err(e)) => action_failure_result(e, "set_value failed"),
            Err(e) => action_failure_result(e.into(), "set_value task failed"),
        }
    }
}

// ── Blocking implementation (runs on spawn_blocking thread) ─────────────────

/// Outcome of a `set_value` write.
///
/// `verified` is `None` for paths that do not perform a value read-back (the
/// AXPopUpButton path drives menu items rather than writing AXValue), and
/// `Some(false)` when a read-back ran but could not confirm the write. A
/// successful `AXUIElementSetAttributeValue` return code is not by itself
/// evidence that the value landed: web content behind an AXWebArea accepts the
/// write and echoes it back through AXValue while the renderer never observes
/// it — the same trap `type_text` already documents.
struct SetValueOutcome {
    detail: String,
    verified: Option<bool>,
    /// `Some(false)` when the element already held the requested value, so the
    /// write was a no-op. Lets callers distinguish "idempotent" from "applied".
    changed: Option<bool>,
}

fn apply_surface_trust(outcome: &mut SetValueOutcome, ax_echo_surface: bool) {
    if ax_echo_surface && outcome.verified == Some(true) {
        outcome.verified = Some(false);
        outcome.changed = None;
        outcome.detail.push_str(
            " AXValue read-back is not trusted for web content; verify the \
             renderer via screenshot or use the browser page tools.",
        );
    }
}

fn apply_verification_label(outcome: &mut SetValueOutcome) {
    if outcome.verified != Some(true) {
        if let Some(rest) = outcome.detail.strip_prefix("✅ Set") {
            outcome.detail = format!("📨 Sent (unverified){rest}");
        }
    }
}

#[derive(Debug, PartialEq)]
enum ValueRoute {
    String,
    Number(f64),
    Steps { initial: f64, target: f64 },
}

/// Choose exactly one actuator from read-only facts, never from a write error.
fn choose_value_route(
    role: &str,
    current_number: Option<f64>,
    settable: bool,
    increment: bool,
    decrement: bool,
    requested: &str,
) -> anyhow::Result<ValueRoute> {
    let is_text = matches!(
        role,
        "AXTextField" | "AXTextArea" | "AXSearchField" | "AXComboBox"
    );
    let is_numeric = !is_text
        && (current_number.is_some() || matches!(role, "AXSlider" | "AXStepper" | "AXIncrementor"));
    if !is_numeric {
        return if settable {
            Ok(ValueRoute::String)
        } else {
            Err(AxActionFailure::refused(
                "AXValue is not writable and no numeric stepping route is available",
            )
            .into())
        };
    }
    let target = requested
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
        .ok_or_else(|| AxActionFailure::refused("numeric AXValue requires a finite number"))?;
    if settable {
        return Ok(ValueRoute::Number(target));
    }
    let initial = current_number
        .filter(|number| number.is_finite())
        .ok_or_else(|| {
            AxActionFailure::refused("numeric stepping requires a readable finite AXValue")
        })?;
    if numbers_match(initial, target)
        || (initial < target && increment)
        || (initial > target && decrement)
    {
        Ok(ValueRoute::Steps { initial, target })
    } else {
        Err(AxActionFailure::refused(
            "numeric target does not advertise the required stepping action",
        )
        .into())
    }
}

fn write_value_with(
    route: &ValueRoute,
    before_write: impl FnOnce() -> anyhow::Result<()>,
    number_write: impl FnOnce(f64) -> AXError,
    string_write: impl FnOnce() -> AXError,
) -> anyhow::Result<()> {
    if matches!(route, ValueRoute::Steps { .. }) {
        return Err(
            AxActionFailure::refused("stepping route cannot issue an AXValue write").into(),
        );
    }
    before_write().map_err(AxActionFailure::before_dispatch)?;
    let error = match route {
        ValueRoute::Number(value) => number_write(*value),
        ValueRoute::String => string_write(),
        ValueRoute::Steps { .. } => unreachable!("stepping refused before any write"),
    };
    if error != kAXErrorSuccess {
        return Err(AxActionFailure::uncertain(format!(
            "AXValue write returned {error} after dispatch; no alternate write or stepping \
             was submitted. Observe fresh state; do not replay."
        ))
        .into());
    }
    Ok(())
}

// Chromium's <input type=number> reports AXValue as a CFNumber but accepts only a string write;
// a numeric write blanks it. Setting the same absolute value again cannot double-apply.
fn needs_string_retry(route: &ValueRoute, after: Option<f64>) -> bool {
    match route {
        ValueRoute::Number(target) => !after.is_some_and(|after| numbers_match(after, *target)),
        _ => false,
    }
}

fn set_value_blocking(
    element_ptr: usize,
    element_index: usize,
    value: &str,
    append: bool,
    web_content: bool,
) -> anyhow::Result<SetValueOutcome> {
    let element = element_ptr as AXUIElementRef;

    let role = unsafe { copy_string_attr(element, "AXRole") }.unwrap_or_default();
    if append {
        let subrole = unsafe { copy_string_attr(element, "AXSubrole") };
        validate_append_target(&role, subrole.as_deref(), unsafe {
            is_attribute_settable(element, "AXValue")
        })?;
        crate::input::ax_actions::ensure_ax_action_enabled(element_ptr, "AXValue append")
            .map_err(AxActionFailure::before_dispatch)?;
    }

    if role == "AXPopUpButton" {
        let element_title = unsafe { copy_string_attr(element, "AXTitle") }.unwrap_or_default();
        // Menu-item selection, not an AXValue write — no read-back to report.
        select_popup_option(element, element_index, value, &element_title).map(|detail| {
            SetValueOutcome {
                detail,
                verified: None,
                changed: None,
            }
        })
    } else {
        let current_number = if append {
            None
        } else {
            unsafe { copy_number_attr(element, "AXValue") }
        };
        let route = if append {
            ValueRoute::String
        } else {
            let actions = unsafe { copy_action_names(element) };
            choose_value_route(
                &role,
                current_number,
                unsafe { is_attribute_settable(element, "AXValue") },
                actions.iter().any(|action| action == "AXIncrement"),
                actions.iter().any(|action| action == "AXDecrement"),
                value,
            )?
        };
        let numeric = !matches!(route, ValueRoute::String);
        let before = if numeric {
            current_number.map(|number| number.to_string())
        } else {
            unsafe { copy_string_attr(element, "AXValue") }
        };
        let appended = append
            .then(|| compose_append_value(before.as_deref(), value))
            .transpose()?;
        let value = appended.as_deref().unwrap_or(value);
        let stepped = match &route {
            ValueRoute::Steps { initial, target } => {
                step_to_value_with(
                    *initial,
                    *target,
                    || {
                        crate::input::ax_actions::ensure_ax_action_enabled(
                            element_ptr,
                            "numeric stepping",
                        )?;
                        crate::input::cancellation::gate().check_target()
                    },
                    |action| unsafe { perform_action(element, action) },
                    || unsafe { copy_number_attr(element, "AXValue") },
                )?;
                true
            }
            _ => {
                crate::input::ax_actions::ensure_ax_action_enabled(element_ptr, "AXValue")
                    .map_err(AxActionFailure::before_dispatch)?;
                let write = || {
                    write_value_with(
                        &route,
                        || Ok(()),
                        |number| unsafe { set_number_attr(element, "AXValue", number) },
                        || unsafe { set_string_attr(element, "AXValue", value) },
                    )
                };
                if append {
                    crate::input::cancellation::gate()
                        .post_ax(element_ptr, write)
                        .map_err(AxActionFailure::before_dispatch)??;
                } else {
                    crate::input::cancellation::gate()
                        .check_target()
                        .map_err(AxActionFailure::before_dispatch)?;
                    write()?;
                    if web_content
                        && needs_string_retry(&route, unsafe { copy_number_attr(element, "AXValue") })
                    {
                        let error = unsafe { set_string_attr(element, "AXValue", value) };
                        if error != kAXErrorSuccess {
                            return Err(AxActionFailure::uncertain(format!(
                                "string AXValue retry returned {error} after an unconfirmed \
                                 numeric write. Observe fresh state; do not replay."
                            ))
                            .into());
                        }
                    }
                }
                false
            }
        };
        let after = if numeric {
            unsafe { copy_number_attr(element, "AXValue") }.map(|number| number.to_string())
        } else {
            unsafe { copy_string_attr(element, "AXValue") }
        };
        let (verified, changed) =
            classify_write(before.as_deref(), after.as_deref(), value, numeric);
        let suffix = match (verified, changed) {
            (Some(true), Some(false)) => " Value already matched.",
            (Some(true), _) => "",
            (Some(false), _) => {
                " Read-back did not confirm the value; observe fresh state; do not replay."
            }
            (None, _) => " Value is not readable through AX; observe fresh state; do not replay.",
        };
        let route_label = if stepped {
            " via AXIncrement/AXDecrement stepping"
        } else {
            ""
        };
        Ok(SetValueOutcome {
            detail: format!("✅ Set AXValue on [{element_index}] {role}{route_label}.{suffix}"),
            verified,
            changed,
        })
    }
}

// Matches packages/contracts COMPUTER_TEXT_MAX_LENGTH, which bounds JavaScript
// strings in UTF-16 code units. Only append's combined value uses this new bound;
// ordinary replacement retains its existing contract and behavior.
const MAX_APPENDED_TEXT_UTF16: usize = 16 * 1024;

fn validate_append_target(role: &str, subrole: Option<&str>, settable: bool) -> anyhow::Result<()> {
    if !matches!(
        role,
        "AXTextField" | "AXTextArea" | "AXSearchField" | "AXComboBox"
    ) || subrole == Some("AXSecureTextField")
    {
        return Err(AxActionFailure::refused_with_code(
            "append_target_unsupported",
            "Appending requires a nonsecure text element; no value was read or written",
        )
        .into());
    }
    if !settable {
        return Err(AxActionFailure::refused_with_code(
            "append_target_unsupported",
            "The exact text element does not expose writable AXValue",
        )
        .into());
    }
    Ok(())
}

fn compose_append_value(current: Option<&str>, suffix: &str) -> anyhow::Result<String> {
    let current = current.ok_or_else(|| {
        AxActionFailure::refused_with_code(
            "append_value_unreadable",
            "The exact element's current AXValue is unreadable; nothing was appended",
        )
    })?;
    if current
        .encode_utf16()
        .chain(suffix.encode_utf16())
        .take(MAX_APPENDED_TEXT_UTF16 + 1)
        .count()
        > MAX_APPENDED_TEXT_UTF16
    {
        return Err(AxActionFailure::refused_with_code(
            "append_value_too_large",
            "The combined text exceeds the 16384 UTF-16 code unit limit; nothing was appended",
        )
        .into());
    }
    Ok(format!("{current}{suffix}"))
}

/// Decide what a post-write AXValue read proves.
///
/// Returns `(verified, changed)`:
/// - `verified = None` when AXValue is not readable at all, so the write can be
///   neither confirmed nor denied.
/// - `verified = Some(true)` when the read-back equals the requested value.
///   Numeric controls are compared numerically so `"25"` matches a slider that
///   reports `"25.0"`.
/// - `changed = Some(false)` when the read-back equals what was there before,
///   i.e. the element's value did not move. Combined with `verified` this
///   separates "already had the requested value" (verified + unchanged) from
///   "the write did not take" (unverified + unchanged).
fn classify_write(
    before: Option<&str>,
    after: Option<&str>,
    requested: &str,
    numeric: bool,
) -> (Option<bool>, Option<bool>) {
    let Some(after) = after else {
        return (None, None);
    };
    let matches = |observed: &str, expected: &str| -> bool {
        if observed == expected {
            return true;
        }
        if !numeric {
            return false;
        }
        match (
            observed.trim().parse::<f64>(),
            expected.trim().parse::<f64>(),
        ) {
            (Ok(a), Ok(b)) => numbers_match(a, b),
            _ => false,
        }
    };
    let verified = matches(after, requested);
    let changed = before.map(|before| !matches(after, before));
    (Some(verified), changed)
}

fn numbers_match(left: f64, right: f64) -> bool {
    left.is_finite()
        && right.is_finite()
        && (left - right).abs() <= 1e-9 * left.abs().max(right.abs()).max(1.0)
}

// ── Read-only-selected AXIncrement / AXDecrement route ──────────────────────

/// Every next step requires a successful AX result and observed progress from
/// the preceding step. An error, missing read-back, no movement, wrong direction
/// or overshoot ends the action. No write fallback or corrective replay follows.
fn step_to_value_with(
    mut current: f64,
    target: f64,
    mut before_step: impl FnMut() -> anyhow::Result<()>,
    mut dispatch: impl FnMut(&str) -> AXError,
    mut readback: impl FnMut() -> Option<f64>,
) -> anyhow::Result<f64> {
    if !current.is_finite() || !target.is_finite() {
        return Err(AxActionFailure::refused("numeric stepping requires finite values").into());
    }
    for attempts in 0..500 {
        if numbers_match(current, target) {
            return Ok(current);
        }
        before_step().map_err(|error| {
            if attempts == 0 {
                AxActionFailure::before_dispatch(error)
            } else {
                AxActionFailure::after_dispatch(error)
            }
        })?;
        let action = if current < target {
            "AXIncrement"
        } else {
            "AXDecrement"
        };
        let error = dispatch(action);
        if error != kAXErrorSuccess {
            return Err(AxActionFailure::uncertain(format!(
                "{action} returned {error} after dispatch; no further step was submitted. \
                 Observe fresh state; do not replay."
            ))
            .into());
        }
        let next = readback()
            .filter(|number| number.is_finite())
            .ok_or_else(|| {
                AxActionFailure::uncertain("numeric step has no finite read-back; do not replay")
            })?;
        if numbers_match(next, target) {
            return Ok(next);
        }
        if numbers_match(next, current)
            || (next - current).signum() != (target - current).signum()
            || (target - next).signum() != (target - current).signum()
        {
            return Err(AxActionFailure::uncertain(format!(
                "numeric step did not prove progress toward the target \
                 (before={current}, after={next}, target={target}); do not replay"
            ))
            .into());
        }
        current = next;
    }
    Err(AxActionFailure::uncertain(
        "numeric stepping reached its bounded limit without proving the requested value; do not replay"
    ).into())
}

// ── AXPopUpButton path ───────────────────────────────────────────────────────

fn select_popup_option(
    element: AXUIElementRef,
    element_index: usize,
    value: &str,
    element_title: &str,
) -> anyhow::Result<String> {
    let children = unsafe { copy_children(element) };
    if children.is_empty() {
        return Err(AxActionFailure::refused(format!(
            "AXPopUpButton [{element_index}] '{element_title}' has no exact AX child; \
             no mutation was submitted. An untargeted browser-document fallback is forbidden."
        ))
        .into());
    }
    let value_lower = value.to_lowercase();
    let mut matched = None;
    let mut available = Vec::with_capacity(children.len());
    for &child in &children {
        let title = unsafe { copy_string_attr(child, "AXTitle") }.unwrap_or_default();
        let child_value = unsafe { copy_string_attr(child, "AXValue") }.unwrap_or_default();
        available.push(title.clone());
        if title.to_lowercase() == value_lower || child_value.to_lowercase() == value_lower {
            matched = Some((child, title));
            break;
        }
    }
    // Keep every retained child alive through the action and release it on all
    // ordinary outcomes, including a preflight refusal or an uncertain AX error.
    let result = if let Some((child, title)) = matched {
        crate::input::ax_actions::perform_ax_action(child as usize, "press").map(|()| {
            format!(
                "📨 Submitted AXPress for '{title}' in AXPopUpButton [{element_index}] \
                 '{element_title}'; observe the selection to confirm it."
            )
        })
    } else {
        Err(AxActionFailure::refused(format!(
            "No exact AX child matching '{value}' in AXPopUpButton [{element_index}] \
             '{element_title}'. Available: {available:?}"
        ))
        .into())
    };
    for child in children {
        unsafe { CFRelease(child as _) };
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{
        apply_surface_trust, apply_verification_label, choose_value_route, classify_write,
        compose_append_value, kAXErrorSuccess, needs_string_retry, step_to_value_with,
        validate_append_target, write_value_with, AxActionFailure, SetValueOutcome, ValueRoute,
        MAX_APPENDED_TEXT_UTF16,
    };
    use std::cell::Cell;

    #[test]
    fn append_accepts_only_writable_nonsecure_text_targets() {
        for role in ["AXTextField", "AXTextArea", "AXSearchField", "AXComboBox"] {
            validate_append_target(role, None, true).unwrap();
            assert!(validate_append_target(role, Some("AXSecureTextField"), true).is_err());
            assert!(validate_append_target(role, None, false).is_err());
        }
        for role in [
            "AXSecureTextField",
            "AXPopUpButton",
            "AXSlider",
            "AXUnknown",
        ] {
            let error = validate_append_target(role, None, true).unwrap_err();
            assert!(!error.downcast_ref::<AxActionFailure>().unwrap().attempted);
        }
    }

    #[test]
    fn append_preserves_exact_text_and_empty_values() {
        assert_eq!(
            compose_append_value(Some("007 "), "🦀\n").unwrap(),
            "007 🦀\n"
        );
        assert_eq!(compose_append_value(Some(""), "new").unwrap(), "new");
        assert_eq!(
            compose_append_value(Some("unchanged"), "").unwrap(),
            "unchanged"
        );
    }

    #[test]
    fn append_never_treats_an_unreadable_value_as_empty() {
        let error = compose_append_value(None, "new").unwrap_err();
        assert!(!error.downcast_ref::<AxActionFailure>().unwrap().attempted);
    }

    #[test]
    fn append_bounds_combined_utf16_text_before_a_write() {
        let prefix = "a".repeat(MAX_APPENDED_TEXT_UTF16 - 2);
        assert_eq!(
            compose_append_value(Some(&prefix), "🦀")
                .unwrap()
                .encode_utf16()
                .count(),
            MAX_APPENDED_TEXT_UTF16
        );
        for (current, suffix) in [
            (prefix.as_str(), "🦀x"),
            (&"a".repeat(MAX_APPENDED_TEXT_UTF16 + 1), ""),
            ("", &"a".repeat(MAX_APPENDED_TEXT_UTF16 + 1)),
        ] {
            let error = compose_append_value(Some(current), suffix).unwrap_err();
            assert!(!error.downcast_ref::<AxActionFailure>().unwrap().attempted);
        }
    }

    #[test]
    fn read_only_facts_select_one_typed_write_route() {
        assert_eq!(
            choose_value_route("AXTextField", None, true, false, false, "007").unwrap(),
            ValueRoute::String
        );
        assert_eq!(
            choose_value_route("AXTextArea", Some(5.0), true, true, true, "007").unwrap(),
            ValueRoute::String
        );
        assert_eq!(
            choose_value_route("AXSlider", Some(5.0), true, true, true, "7").unwrap(),
            ValueRoute::Number(7.0)
        );
        assert_eq!(
            choose_value_route("AXUnknown", Some(5.0), true, false, false, "7").unwrap(),
            ValueRoute::Number(7.0)
        );
        assert_eq!(
            choose_value_route("AXSlider", Some(5.0), false, true, true, "7").unwrap(),
            ValueRoute::Steps {
                initial: 5.0,
                target: 7.0
            }
        );
    }

    #[test]
    fn an_unconfirmed_numeric_write_is_retried_once_as_a_string() {
        assert!(needs_string_retry(&ValueRoute::Number(2.0), None));
        assert!(needs_string_retry(&ValueRoute::Number(2.0), Some(0.0)));
        assert!(!needs_string_retry(&ValueRoute::Number(2.0), Some(2.0)));
        assert!(!needs_string_retry(&ValueRoute::String, None));
    }

    #[test]
    fn unknown_unsettable_or_nonfinite_numeric_routes_refuse_before_dispatch() {
        for result in [
            choose_value_route("AXSlider", None, false, true, true, "7"),
            choose_value_route("AXSlider", Some(5.0), false, false, true, "7"),
            choose_value_route("AXTextField", None, false, true, true, "7"),
            choose_value_route("AXSlider", Some(5.0), true, true, true, "NaN"),
            choose_value_route("AXSlider", Some(5.0), true, true, true, "inf"),
        ] {
            assert!(
                !result
                    .unwrap_err()
                    .downcast_ref::<AxActionFailure>()
                    .unwrap()
                    .attempted
            );
        }
    }

    #[test]
    fn a_failed_numeric_write_never_falls_back_to_string_or_steps() {
        let writes = Cell::new(0);
        let error = write_value_with(
            &ValueRoute::Number(7.0),
            || Ok(()),
            |number| {
                assert_eq!(number, 7.0);
                writes.set(writes.get() + 1);
                -25204
            },
            || panic!("numeric write replayed as a string"),
        )
        .unwrap_err();
        assert_eq!(writes.get(), 1);
        assert!(error.downcast_ref::<AxActionFailure>().unwrap().attempted);
    }

    #[test]
    fn numeric_text_has_only_one_string_write_and_no_numeric_attempt() {
        let writes = Cell::new(0);
        let route = choose_value_route("AXTextField", None, true, false, false, "007").unwrap();
        write_value_with(
            &route,
            || Ok(()),
            |_| panic!("text was rewritten as a number"),
            || {
                writes.set(writes.get() + 1);
                kAXErrorSuccess
            },
        )
        .unwrap();
        assert_eq!(writes.get(), 1);
    }

    #[test]
    fn cancelled_value_write_never_calls_an_actuator() {
        let error = write_value_with(
            &ValueRoute::Number(7.0),
            || anyhow::bail!("cancelled"),
            |_| panic!("refused numeric write"),
            || panic!("refused string write"),
        )
        .unwrap_err();
        assert!(!error.downcast_ref::<AxActionFailure>().unwrap().attempted);
    }

    #[test]
    fn a_failed_numeric_step_stops_after_one_attempt() {
        let steps = Cell::new(0);
        let error = step_to_value_with(
            0.0,
            3.0,
            || Ok(()),
            |_| {
                steps.set(steps.get() + 1);
                -25204
            },
            || panic!("failed step was treated as successful"),
        )
        .unwrap_err();
        assert_eq!(steps.get(), 1);
        assert!(error.downcast_ref::<AxActionFailure>().unwrap().attempted);
    }

    #[test]
    fn uncertain_step_readbacks_stop_before_another_action() {
        for readback in [None, Some(f64::NAN), Some(0.0), Some(-1.0), Some(4.0)] {
            let steps = Cell::new(0);
            let error = step_to_value_with(
                0.0,
                3.0,
                || Ok(()),
                |_| {
                    steps.set(steps.get() + 1);
                    kAXErrorSuccess
                },
                || readback,
            )
            .unwrap_err();
            assert_eq!(steps.get(), 1);
            assert!(error.downcast_ref::<AxActionFailure>().unwrap().attempted);
        }
    }

    #[test]
    fn every_subsequent_step_requires_observed_progress() {
        let current = Cell::new(0.0);
        let steps = Cell::new(0);
        let checks = Cell::new(0);
        let result = step_to_value_with(
            0.0,
            3.0,
            || {
                checks.set(checks.get() + 1);
                Ok(())
            },
            |action| {
                assert_eq!(action, "AXIncrement");
                current.set(current.get() + 1.0);
                steps.set(steps.get() + 1);
                kAXErrorSuccess
            },
            || Some(current.get()),
        )
        .unwrap();
        assert_eq!(result, 3.0);
        assert_eq!(steps.get(), 3);
        assert_eq!(checks.get(), 3);
    }

    #[test]
    fn cancellation_after_a_verified_step_remains_an_uncertain_action() {
        let steps = Cell::new(0);
        let error = step_to_value_with(
            0.0,
            3.0,
            || {
                if steps.get() > 0 {
                    anyhow::bail!("Space changed")
                }
                Ok(())
            },
            |_| {
                steps.set(steps.get() + 1);
                kAXErrorSuccess
            },
            || Some(1.0),
        )
        .unwrap_err();
        assert_eq!(steps.get(), 1);
        assert!(error.downcast_ref::<AxActionFailure>().unwrap().attempted);
    }

    #[test]
    fn stepping_remains_bounded_without_claiming_approximate_success() {
        let steps = Cell::new(0);
        let error = step_to_value_with(
            0.0,
            1000.0,
            || Ok(()),
            |_| {
                steps.set(steps.get() + 1);
                kAXErrorSuccess
            },
            || Some(f64::from(steps.get())),
        )
        .unwrap_err();
        assert_eq!(steps.get(), 500);
        assert!(error.downcast_ref::<AxActionFailure>().unwrap().attempted);
    }

    #[test]
    fn unreadable_value_reports_neither_verified_nor_changed() {
        // AXValue is not exposed: the write can be neither confirmed nor denied,
        // so the tool must not claim success on the return code alone.
        assert_eq!(
            classify_write(Some("old"), None, "new", false),
            (None, None)
        );
    }

    #[test]
    fn matching_read_back_verifies_the_write() {
        assert_eq!(
            classify_write(Some("old"), Some("new"), "new", false),
            (Some(true), Some(true))
        );
    }

    #[test]
    fn echoed_but_wrong_value_fails_verification() {
        // Web content behind an AXWebArea accepts the write and echoes a value
        // the renderer never took. A success return code must not be reported
        // as a verified write.
        assert_eq!(
            classify_write(Some("old"), Some("old"), "new", false),
            (Some(false), Some(false))
        );
    }

    #[test]
    fn idempotent_write_is_verified_but_unchanged() {
        assert_eq!(
            classify_write(Some("same"), Some("same"), "same", false),
            (Some(true), Some(false))
        );
    }

    #[test]
    fn numeric_controls_compare_numerically() {
        // AXSlider reports "25.0" for a requested "25".
        assert_eq!(
            classify_write(Some("10"), Some("25.000000001"), "25", true),
            (Some(true), Some(true))
        );
    }

    #[test]
    fn numeric_text_is_not_normalised_on_a_text_target() {
        assert_eq!(
            classify_write(Some("old"), Some("7"), "007", false),
            (Some(false), Some(true))
        );
    }

    #[test]
    fn missing_before_still_verifies_numeric_after() {
        assert_eq!(
            classify_write(None, Some("25.0"), "25", true),
            (Some(true), None)
        );
    }

    #[test]
    fn web_content_ax_echo_is_never_reported_as_verified() {
        let mut outcome = SetValueOutcome {
            detail: "Set value.".to_owned(),
            verified: Some(true),
            changed: Some(true),
        };
        apply_surface_trust(&mut outcome, true);
        assert_eq!(outcome.verified, Some(false));
        assert_eq!(outcome.changed, None);
        assert!(outcome.detail.contains("not trusted for web content"));
    }

    #[test]
    fn native_read_back_remains_trusted() {
        let mut outcome = SetValueOutcome {
            detail: "Set value.".to_owned(),
            verified: Some(true),
            changed: Some(true),
        };
        apply_surface_trust(&mut outcome, false);
        assert_eq!(outcome.verified, Some(true));
        assert_eq!(outcome.changed, Some(true));
        assert_eq!(outcome.detail, "Set value.");
    }

    #[test]
    fn unverified_result_does_not_keep_a_success_checkmark() {
        let mut outcome = SetValueOutcome {
            detail: "✅ Set AXValue on [4] AXTextField.".to_owned(),
            verified: Some(false),
            changed: Some(false),
        };
        apply_verification_label(&mut outcome);
        assert_eq!(
            outcome.detail,
            "📨 Sent (unverified) AXValue on [4] AXTextField."
        );
    }
}
