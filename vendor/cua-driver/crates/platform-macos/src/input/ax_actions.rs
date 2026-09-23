//! AX action dispatch — the preferred click/interaction path for indexed elements.

use crate::ax::bindings::*;
use core_foundation::base::{CFEqual, CFRelease, CFRetain, CFTypeRef};

const MAX_SELECTION_ANCESTORS: usize = 8;

fn is_selectable_container_role(role: &str) -> bool {
    matches!(role, "AXRow" | "AXCell" | "AXListItem" | "AXImage")
}

/// An AX error can be returned after the target processed a mutation. Only
/// `NotAttempted` permits choosing another ancestor or a different actuator.
#[derive(Debug, PartialEq, Eq)]
pub enum SelectionOutcome {
    NotAttempted,
    AttemptedConfirmed {
        role: String,
    },
    AttemptedUncertain {
        role: String,
        error: AXError,
        selected: Option<bool>,
    },
}

/// Preserve whether a mutation crossed the AX boundary when reporting errors.
/// A failed preflight is a refusal; a failed AX call may already have acted.
#[derive(Debug)]
pub struct AxActionFailure {
    pub attempted: bool,
    code: Option<&'static str>,
    ax_error: Option<AXError>,
    actuator: &'static str,
    message: String,
}

impl AxActionFailure {
    pub fn refused(message: impl Into<String>) -> Self {
        Self {
            attempted: false,
            code: None,
            ax_error: None,
            actuator: "ax_action",
            message: message.into(),
        }
    }

    pub fn refused_with_code(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: Some(code),
            ..Self::refused(message)
        }
    }

    pub fn before_dispatch(error: anyhow::Error) -> Self {
        Self {
            attempted: false,
            code: error
                .downcast_ref::<super::target_guard::TargetRefusal>()
                .map(|failure| failure.code),
            ax_error: None,
            actuator: "ax_action",
            message: error.to_string(),
        }
    }

    pub fn uncertain(message: impl Into<String>) -> Self {
        Self {
            attempted: true,
            code: None,
            ax_error: None,
            actuator: "ax_action",
            message: message.into(),
        }
    }

    fn from_ax_error(action: &str, error: AXError) -> Self {
        Self {
            attempted: true,
            code: Some("ax_action_uncertain"),
            ax_error: Some(error),
            actuator: if action == "AXPress" { "ax_press" } else { "ax_action" },
            message: format!("AX actuator returned {error} after dispatch; effect unknown. Observe fresh state; do not replay."),
        }
    }

    pub fn after_dispatch(error: anyhow::Error) -> Self {
        Self {
            attempted: true,
            code: error
                .downcast_ref::<Self>()
                .and_then(|failure| failure.code)
                .or_else(|| {
                    error
                        .downcast_ref::<super::target_guard::TargetRefusal>()
                        .map(|failure| failure.code)
                }),
            ax_error: error
                .downcast_ref::<Self>()
                .and_then(|failure| failure.ax_error),
            actuator: error
                .downcast_ref::<Self>()
                .map(|failure| failure.actuator)
                .unwrap_or("ax_action"),
            message: format!(
                "{error}; prior input may have acted; observe fresh state and do not replay"
            ),
        }
    }
}

impl std::fmt::Display for AxActionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}

impl std::error::Error for AxActionFailure {}

pub fn action_failure_result(
    error: anyhow::Error,
    label: &str,
) -> cua_driver_core::protocol::ToolResult {
    let failure = error.downcast_ref::<AxActionFailure>();
    let refused = failure.is_some_and(|failure| !failure.attempted);
    let code = failure
        .and_then(|failure| failure.code)
        .unwrap_or(if refused {
            "ax_action_refused"
        } else {
            "ax_action_uncertain"
        });
    cua_driver_core::protocol::ToolResult::error(format!("{label}: {error}")).with_structured(
        serde_json::json!({
            "verified": false,
            "effect": if refused { "refused" } else { "unverifiable" },
            "code": code,
            "diagnostics": {
                "delivery_path": "ax",
                "actuator": failure.map(|failure| failure.actuator).unwrap_or("ax_action"),
                "error_code": if failure.and_then(|failure| failure.ax_error).is_some() {
                    "ax_dispatch_failed"
                } else if refused { "ax_action_refused" } else { "ax_action_uncertain" },
                "ax_error": failure.and_then(|failure| failure.ax_error)
            }
        }),
    )
}

fn select_container_with(
    role: String,
    selected: Option<bool>,
    settable: bool,
    before_write: impl FnOnce() -> anyhow::Result<()>,
    write: impl FnOnce() -> AXError,
    readback: impl FnOnce() -> Option<bool>,
) -> anyhow::Result<SelectionOutcome> {
    if !is_selectable_container_role(&role) || selected.is_none() || !settable {
        return Ok(SelectionOutcome::NotAttempted);
    }
    before_write().map_err(AxActionFailure::before_dispatch)?;
    let error = write();
    let selected = readback();
    if error == kAXErrorSuccess && selected == Some(true) {
        Ok(SelectionOutcome::AttemptedConfirmed { role })
    } else {
        Ok(SelectionOutcome::AttemptedUncertain {
            role,
            error,
            selected,
        })
    }
}

fn select_ancestor_with<Element>(
    mut current: Element,
    mut attempt: impl FnMut(&Element) -> anyhow::Result<SelectionOutcome>,
    mut parent: impl FnMut(&Element) -> Option<Element>,
) -> anyhow::Result<SelectionOutcome> {
    for _ in 0..MAX_SELECTION_ANCESTORS {
        match attempt(&current)? {
            SelectionOutcome::NotAttempted => {}
            terminal => return Ok(terminal),
        }
        let Some(next) = parent(&current) else {
            break;
        };
        current = next;
    }
    Ok(SelectionOutcome::NotAttempted)
}

struct SelectionElement {
    ptr: AXUIElementRef,
    owned: bool,
}

impl Drop for SelectionElement {
    fn drop(&mut self) {
        if self.owned {
            unsafe { CFRelease(self.ptr as CFTypeRef) };
        }
    }
}

/// Select the nearest writable collection item before attempting AXPress.
/// Read-only capability checks may advance to an ancestor. Once AXSelected is
/// written, even an AX error or an absent/false read-back is terminal: it is
/// never permission to mutate another ancestor or submit a pointer click.
pub fn select_nearest_container(element_ptr: usize) -> anyhow::Result<SelectionOutcome> {
    select_ancestor_with(
        SelectionElement {
            ptr: element_ptr as AXUIElementRef,
            owned: false,
        },
        |element| {
            let role = unsafe { copy_string_attr(element.ptr, "AXRole") }.unwrap_or_default();
            let selected = unsafe { copy_bool_attr(element.ptr, "AXSelected") };
            let settable = is_selectable_container_role(&role)
                && selected.is_some()
                && unsafe { is_attribute_settable(element.ptr, "AXSelected") };
            select_container_with(
                role,
                selected,
                settable,
                || {
                    ensure_ax_action_enabled(element.ptr as usize, "AXSelected")?;
                    crate::input::cancellation::gate().ensure_active()
                },
                || unsafe { set_bool_attr_true(element.ptr, "AXSelected") },
                || unsafe { copy_bool_attr(element.ptr, "AXSelected") },
            )
        },
        |element| {
            unsafe { copy_element_attr(element.ptr, "AXParent") }
                .map(|ptr| SelectionElement { ptr, owned: true })
        },
    )
}

/// Read the selection state of the nearest collection-like element without
/// mutating it. This is used to verify a coordinate fallback when AppKit
/// exposes `AXSelected` but refuses to set it directly (notably Finder icon
/// views).
pub fn nearest_container_selection_state(element_ptr: usize) -> Option<(String, bool)> {
    let mut current = element_ptr as AXUIElementRef;
    let mut owns_current = false;

    for _ in 0..MAX_SELECTION_ANCESTORS {
        let role = unsafe { copy_string_attr(current, "AXRole") }.unwrap_or_default();
        if is_selectable_container_role(&role) {
            if let Some(selected) = unsafe { copy_bool_attr(current, "AXSelected") } {
                if owns_current {
                    unsafe { CFRelease(current as CFTypeRef) };
                }
                return Some((role, selected));
            }
        }

        let parent = unsafe { copy_element_attr(current, "AXParent") };
        if owns_current {
            unsafe { CFRelease(current as CFTypeRef) };
        }
        let Some(parent) = parent else {
            return None;
        };
        current = parent;
        owns_current = true;
    }

    if owns_current {
        unsafe { CFRelease(current as CFTypeRef) };
    }
    None
}

/// A retained selection context for proving the settled result of a modified
/// pointer click.
///
/// Reading only the target's `AXSelected` bit is insufficient for multi-select:
/// AppKit can expose a transient target transition while it is still resolving
/// the synthetic gesture, and a modifier-less outcome can replace the prior
/// selection with the target. Keep the target and every selected sibling alive
/// across delivery so callers can require both the intended target transition
/// and preservation of the pre-existing selection.
pub struct SelectionReadback {
    role: String,
    target: AXUIElementRef,
    peer_model_observed: bool,
    previously_selected_peers: Vec<AXUIElementRef>,
}

impl SelectionReadback {
    pub fn role(&self) -> &str {
        &self.role
    }

    pub fn observe(&self) -> Option<(bool, bool)> {
        let target_selected = unsafe { copy_bool_attr(self.target, "AXSelected") }?;
        let peers_preserved = self.peer_model_observed
            && self
                .previously_selected_peers
                .iter()
                .all(|&peer| unsafe { copy_bool_attr(peer, "AXSelected") } == Some(true));
        Some((target_selected, peers_preserved))
    }
}

impl Drop for SelectionReadback {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.target as CFTypeRef);
            for peer in self.previously_selected_peers.drain(..) {
                CFRelease(peer as CFTypeRef);
            }
        }
    }
}

/// Capture the nearest selectable container and its currently-selected peers.
/// Returns `None` when the platform does not expose a readable selection model;
/// callers must then leave the action unverifiable instead of inventing proof.
pub fn capture_nearest_container_selection(element_ptr: usize) -> Option<SelectionReadback> {
    let mut current = element_ptr as AXUIElementRef;
    let mut owns_current = false;

    for _ in 0..MAX_SELECTION_ANCESTORS {
        let role = unsafe { copy_string_attr(current, "AXRole") }.unwrap_or_default();
        if is_selectable_container_role(&role)
            && unsafe { copy_bool_attr(current, "AXSelected") }.is_some()
        {
            if !owns_current {
                unsafe { CFRetain(current as CFTypeRef) };
            }
            let target = current;
            let parent = unsafe { copy_element_attr(target, "AXParent") };
            let mut peers = Vec::new();
            let mut peer_model_observed = false;
            if let Some(parent) = parent {
                for child in unsafe { copy_children(parent) } {
                    let is_target =
                        unsafe { CFEqual(child as CFTypeRef, target as CFTypeRef) != 0 };
                    peer_model_observed |= is_target;
                    if !is_target && unsafe { copy_bool_attr(child, "AXSelected") } == Some(true) {
                        peers.push(child);
                    } else {
                        unsafe { CFRelease(child as CFTypeRef) };
                    }
                }
                unsafe { CFRelease(parent as CFTypeRef) };
            }
            return Some(SelectionReadback {
                role,
                target,
                peer_model_observed,
                previously_selected_peers: peers,
            });
        }

        let parent = unsafe { copy_element_attr(current, "AXParent") };
        if owns_current {
            unsafe { CFRelease(current as CFTypeRef) };
        }
        let Some(parent) = parent else {
            return None;
        };
        current = parent;
        owns_current = true;
    }

    if owns_current {
        unsafe { CFRelease(current as CFTypeRef) };
    }
    None
}

fn ensure_ax_enabled(enabled: Option<bool>, action: &str) -> anyhow::Result<()> {
    if enabled == Some(false) {
        anyhow::bail!(
            "refusing {action}: the target reports AXEnabled=false. \
             An explicitly approved delivery_mode:\"foreground\" action or bring_to_front \
             may enable it; observe current state before choosing a new action"
        );
    }
    Ok(())
}

/// Refuse AX actions that macOS reports as disabled.
///
/// This must be checked immediately before dispatch rather than trusting the
/// cached snapshot value: foreground delivery can make a menu item live after
/// it was resolved, while backgrounding can disable it in the other direction.
pub fn ensure_ax_action_enabled(element_ptr: usize, action: &str) -> anyhow::Result<()> {
    crate::input::cancellation::gate().check_ax_target(element_ptr)?;
    let enabled = unsafe { copy_bool_attr(element_ptr as AXUIElementRef, "AXEnabled") };
    ensure_ax_enabled(enabled, action)
}

/// Perform an AX action on a cached element.
pub fn perform_ax_action(element_ptr: usize, action: &str) -> anyhow::Result<()> {
    let ax_action = map_action(action);
    dispatch_ax_action_with(
        ax_action,
        || {
            ensure_ax_action_enabled(element_ptr, ax_action)?;
            crate::input::cancellation::gate().ensure_active()
        },
        || unsafe { perform_action(element_ptr as AXUIElementRef, ax_action) },
    )
}

fn dispatch_ax_action_with(
    action: &str,
    before_dispatch: impl FnOnce() -> anyhow::Result<()>,
    dispatch: impl FnOnce() -> AXError,
) -> anyhow::Result<()> {
    before_dispatch().map_err(AxActionFailure::before_dispatch)?;
    let err = dispatch();
    if err == kAXErrorSuccess {
        Ok(())
    } else {
        Err(AxActionFailure::from_ax_error(action, err).into())
    }
}

fn map_action(action: &str) -> &'static str {
    match action.to_lowercase().as_str() {
        "press" | "click" | "axpress" => "AXPress",
        "show_menu" | "right_click" | "rightclick" | "axshowmenu" => "AXShowMenu",
        "pick" | "axpick" => "AXPick",
        "confirm" | "axconfirm" => "AXConfirm",
        "cancel" | "axcancel" => "AXCancel",
        "open" | "axopen" => "AXOpen",
        _ => "AXPress",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn action_error_preserves_numeric_ax_failure_without_content_in_diagnostics() {
        let result = action_failure_result(
            AxActionFailure::from_ax_error("AXPress", -25204).into(),
            "AX action failed",
        );
        let details = result.structured_content.unwrap();
        assert_eq!(details["diagnostics"]["ax_error"], -25204);
        assert_eq!(details["diagnostics"]["actuator"], "ax_press");
        assert_eq!(details["diagnostics"]["error_code"], "ax_dispatch_failed");
        assert_eq!(details["effect"], "unverifiable");
        let result = action_failure_result(
            AxActionFailure::uncertain("private field text or title").into(),
            "AX action failed",
        );
        assert!(!result.structured_content.unwrap()["diagnostics"]
            .to_string()
            .contains("private"));
    }

    #[test]
    fn disabled_elements_are_refused_before_dispatch() {
        let error = ensure_ax_enabled(Some(false), "AXPick").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("AXEnabled=false"));
        assert!(message.contains("delivery_mode:\"foreground\""));
        assert!(message.contains("bring_to_front"));
    }

    #[test]
    fn enabled_or_unreported_state_is_allowed() {
        assert!(ensure_ax_enabled(Some(true), "AXPress").is_ok());
        assert!(ensure_ax_enabled(None, "AXPress").is_ok());
    }

    #[test]
    fn selection_fallback_is_limited_to_collection_item_roles() {
        for role in ["AXRow", "AXCell", "AXListItem", "AXImage"] {
            assert!(is_selectable_container_role(role), "{role}");
        }
        for role in ["AXButton", "AXTextField", "AXWindow", "AXOutline"] {
            assert!(!is_selectable_container_role(role), "{role}");
        }
    }

    #[test]
    fn an_attempted_selection_never_visits_another_ancestor() {
        for (error, selected) in [
            (kAXErrorSuccess, Some(true)),
            (kAXErrorSuccess, Some(false)),
            (kAXErrorSuccess, None),
            (kAXErrorFailure, Some(true)),
            (kAXErrorFailure, Some(false)),
            (-25204, None), // kAXErrorCannotComplete is not proof of no effect.
        ] {
            let writes = Cell::new(0);
            let parent_reads = Cell::new(0);
            let outcome = select_ancestor_with(
                0,
                |_| {
                    select_container_with(
                        "AXRow".to_owned(),
                        Some(false),
                        true,
                        || Ok(()),
                        || {
                            writes.set(writes.get() + 1);
                            error
                        },
                        || selected,
                    )
                },
                |index| {
                    parent_reads.set(parent_reads.get() + 1);
                    Some(index + 1)
                },
            )
            .unwrap();
            assert_eq!(writes.get(), 1);
            assert_eq!(parent_reads.get(), 0);
            if error == kAXErrorSuccess && selected == Some(true) {
                assert!(matches!(
                    outcome,
                    SelectionOutcome::AttemptedConfirmed { .. }
                ));
            } else {
                assert_eq!(
                    outcome,
                    SelectionOutcome::AttemptedUncertain {
                        role: "AXRow".to_owned(),
                        error,
                        selected,
                    }
                );
            }
        }
    }

    #[test]
    fn read_only_non_candidates_allow_an_ancestor_before_one_write() {
        let writes = Cell::new(0);
        let parent_reads = Cell::new(0);
        let outcome = select_ancestor_with(
            0,
            |index| {
                select_container_with(
                    "AXRow".to_owned(),
                    Some(false),
                    *index == 1,
                    || Ok(()),
                    || {
                        writes.set(writes.get() + 1);
                        kAXErrorSuccess
                    },
                    || Some(true),
                )
            },
            |index| {
                parent_reads.set(parent_reads.get() + 1);
                Some(index + 1)
            },
        )
        .unwrap();
        assert!(matches!(
            outcome,
            SelectionOutcome::AttemptedConfirmed { .. }
        ));
        assert_eq!(writes.get(), 1);
        assert_eq!(parent_reads.get(), 1);
    }

    #[test]
    fn selection_preflight_refusal_submits_nothing_and_stops_traversal() {
        let error = select_ancestor_with(
            0,
            |_| {
                select_container_with(
                    "AXRow".to_owned(),
                    Some(false),
                    true,
                    || anyhow::bail!("native input admission closed"),
                    || panic!("refused selection was submitted"),
                    || panic!("refused selection was read back"),
                )
            },
            |_| -> Option<usize> { panic!("refused selection tried another ancestor") },
        )
        .unwrap_err();
        assert!(!error.downcast_ref::<AxActionFailure>().unwrap().attempted);
    }

    #[test]
    fn selection_without_a_writable_candidate_is_not_attempted_and_bounded() {
        let examined = Cell::new(0);
        let outcome = select_ancestor_with(
            0,
            |_| {
                examined.set(examined.get() + 1);
                Ok(SelectionOutcome::NotAttempted)
            },
            |index| Some(index + 1),
        )
        .unwrap();
        assert_eq!(examined.get(), MAX_SELECTION_ANCESTORS);
        assert_eq!(outcome, SelectionOutcome::NotAttempted);
    }

    #[test]
    fn ax_dispatch_errors_are_uncertain_after_exactly_one_attempt() {
        for code in [kAXErrorFailure, kAXErrorInvalidUIElement, -25204] {
            let dispatches = Cell::new(0);
            let error = dispatch_ax_action_with(
                "AXPress",
                || Ok(()),
                || {
                    dispatches.set(dispatches.get() + 1);
                    code
                },
            )
            .unwrap_err();
            assert_eq!(dispatches.get(), 1);
            assert!(error.downcast_ref::<AxActionFailure>().unwrap().attempted);
            assert!(error.to_string().contains("do not replay"));
        }
    }

    #[test]
    fn ax_dispatch_preflight_refusal_never_calls_the_actuator() {
        let error = dispatch_ax_action_with(
            "AXPress",
            || anyhow::bail!("target is disabled"),
            || panic!("refused AXPress was submitted"),
        )
        .unwrap_err();
        assert!(!error.downcast_ref::<AxActionFailure>().unwrap().attempted);
    }

    #[test]
    fn mapped_ax_actions_retain_the_requested_semantics() {
        for action in [
            "AXPress",
            "AXShowMenu",
            "AXPick",
            "AXConfirm",
            "AXCancel",
            "AXOpen",
        ] {
            assert_eq!(map_action(action), action);
        }
    }
}

/// Set AXFocused=true on an element (for pre-focusing before key press).
pub fn focus_element(element_ptr: usize) -> anyhow::Result<()> {
    crate::input::cancellation::gate()
        .check_ax_target(element_ptr)
        .map_err(AxActionFailure::before_dispatch)?;
    let err = unsafe { set_bool_attr_true(element_ptr as AXUIElementRef, "AXFocused") };
    if err == kAXErrorSuccess {
        Ok(())
    } else {
        // Focus errors are often benign (element doesn't support focus).
        tracing::warn!("AXSetAttribute(AXFocused) returned {err}");
        Ok(())
    }
}

/// Report whether `element_ptr` is the application's currently focused element.
///
/// This is a read-only confirmation for the foreground typing rung: an
/// `AXFocused` write can be accepted by the element and then immediately
/// clobbered when AppKit installs the window's remembered first responder, so
/// "the write returned success" is not evidence that focus stuck. Identity is
/// compared with `CFEqual` because the app hands back a fresh `AXUIElementRef`
/// for the same underlying element.
///
/// A `false` return is deliberately conservative: an app whose
/// `AXFocusedUIElement` is unreadable reports not-focused, which at worst costs
/// one extra re-apply.
pub fn is_element_focused(pid: i32, element_ptr: usize) -> bool {
    unsafe {
        let Some(focused) = crate::ax::bindings::focused_element_of_pid(pid) else {
            return false;
        };
        let same = CFEqual(focused as CFTypeRef, element_ptr as CFTypeRef) != 0;
        CFRelease(focused as CFTypeRef);
        same
    }
}

/// Set the AXValue of an element (for dropdowns, text fields, etc.).
pub fn set_ax_value(element_ptr: usize, value: &str) -> anyhow::Result<()> {
    crate::input::cancellation::gate()
        .check_ax_target(element_ptr)
        .map_err(AxActionFailure::before_dispatch)?;
    let err = unsafe { set_string_attr(element_ptr as AXUIElementRef, "AXValue", value) };
    if err == kAXErrorSuccess {
        Ok(())
    } else {
        Err(AxActionFailure::uncertain(format!(
            "AXUIElementSetAttributeValue(AXValue) returned {err} after dispatch; do not replay"
        ))
        .into())
    }
}
