//! Fresh exact-target acquisition for background input decisions.
//!
//! Gathers the facts [`cua_driver_core::background_input`] needs about one
//! requested `(pid, CGWindowID)` immediately before a background mutation:
//! WindowServer ownership, fresh `AXWindows` membership (mapped through
//! `_AXUIElementGetWindow`), minimized/hidden state, competing same-pid AX
//! top-level keyboard destinations, and addressed-element ancestry. All reads are
//! bounded and fail closed — an unreadable fact never unlocks a route.

use core_foundation::base::{CFEqual, CFRelease, CFRetain, CFTypeRef};
use cua_driver_core::background_input::{
    BackgroundTargetFacts, ElementAncestry, WindowServerOwnership,
};

use super::bindings::{
    ax_get_window_id, copy_ax_windows, copy_bool_attr, copy_children, copy_element_attr,
    copy_string_attr, focused_element_of_pid, AXUIElementCreateApplication, AXUIElementRef,
    AXUIElementSetMessagingTimeout,
};
use crate::windows::{all_windows, resolve_window_owner, WindowOwner};

/// Bounded `AXParent` ascent used when an element does not expose `AXWindow`.
const MAX_ANCESTRY_DEPTH: usize = 40;

const GATE_TIMEOUT_SECONDS: f32 = 0.15;

/// Resolve the CGWindowID of the top-level AX window that owns `element`.
///
/// Prefers the element's `AXWindow` attribute and falls back to a bounded
/// `AXParent` walk. `None` means ancestry could not be proven — callers must
/// treat that as "not the requested window", never as a wildcard.
///
/// # Safety
///
/// `element` must be a valid `AXUIElementRef` for the duration of the call.
pub unsafe fn element_window_id(element: AXUIElementRef) -> Option<u32> {
    if let Some(window) = copy_element_attr(element, "AXWindow") {
        AXUIElementSetMessagingTimeout(window, GATE_TIMEOUT_SECONDS);
        let window_id = ax_get_window_id(window);
        CFRelease(window as CFTypeRef);
        if window_id.is_some() {
            return window_id;
        }
    }
    // Fallback: ascend AXParent until a window role, then map it.
    let mut current: AXUIElementRef = element;
    let mut owned = false;
    let mut resolved = None;
    for _ in 0..MAX_ANCESTRY_DEPTH {
        match copy_string_attr(current, "AXRole").as_deref() {
            Some("AXWindow") | Some("AXSheet") => {
                resolved = ax_get_window_id(current);
                break;
            }
            Some("AXApplication") | None => break,
            _ => {}
        }
        let parent = copy_element_attr(current, "AXParent");
        if owned {
            CFRelease(current as CFTypeRef);
        }
        match parent {
            Some(parent) => {
                AXUIElementSetMessagingTimeout(parent, GATE_TIMEOUT_SECONDS);
                current = parent;
                owned = true;
            }
            None => return None,
        }
    }
    if owned {
        CFRelease(current as CFTypeRef);
    }
    resolved
}

/// Nodes a dialog-attachment proof may visit in the requested window.
const ATTACHED_SEARCH_BUDGET: usize = 4_000;

/// Whether `element`, which reports another window of `pid`, is reachable from
/// the AX subtree of window `window_id`. Chromium shows a page's JS alert this
/// way: the alert is its own top-level window, but its dialog group is also a
/// child inside the browser window's root view, so the browser window's state
/// lists the alert's buttons. The proof mirrors that walk: it finds the
/// element's top container below its own window and searches the requested
/// window's subtree, outside web content, for that same container.
///
/// # Safety
///
/// `element` must be a valid `AXUIElementRef` for the duration of the call.
pub unsafe fn element_in_attached_window(
    pid: i32,
    element: AXUIElementRef,
    window_id: u32,
) -> bool {
    let mut container = element;
    CFRetain(container as CFTypeRef);
    for _ in 0..MAX_ANCESTRY_DEPTH {
        let Some(parent) = copy_element_attr(container, "AXParent") else {
            CFRelease(container as CFTypeRef);
            return false;
        };
        AXUIElementSetMessagingTimeout(parent, GATE_TIMEOUT_SECONDS);
        let role = copy_string_attr(parent, "AXRole");
        if matches!(role.as_deref(), Some("AXWindow") | Some("AXSheet")) {
            let own = ax_get_window_id(parent);
            CFRelease(parent as CFTypeRef);
            if own.is_none() || own == Some(window_id) {
                CFRelease(container as CFTypeRef);
                return false;
            }
            break;
        }
        CFRelease(container as CFTypeRef);
        if matches!(role.as_deref(), Some("AXApplication") | None) {
            CFRelease(parent as CFTypeRef);
            return false;
        }
        container = parent;
    }

    let app = AXUIElementCreateApplication(pid);
    if app.is_null() {
        CFRelease(container as CFTypeRef);
        return false;
    }
    AXUIElementSetMessagingTimeout(app, GATE_TIMEOUT_SECONDS);
    let mut queue = std::collections::VecDeque::new();
    for window in copy_ax_windows(app) {
        if queue.is_empty() && ax_get_window_id(window) == Some(window_id) {
            queue.push_back(window);
        } else {
            CFRelease(window as CFTypeRef);
        }
    }
    CFRelease(app as CFTypeRef);

    let mut found = false;
    let mut visited = 0;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        if !found && visited <= ATTACHED_SEARCH_BUDGET {
            AXUIElementSetMessagingTimeout(node, GATE_TIMEOUT_SECONDS);
            if CFEqual(node as CFTypeRef, container as CFTypeRef) != 0 {
                found = true;
            } else if copy_string_attr(node, "AXRole").as_deref() != Some("AXWebArea") {
                queue.extend(copy_children(node));
            }
        }
        CFRelease(node as CFTypeRef);
    }
    CFRelease(container as CFTypeRef);
    found
}

/// The process's focused AX element, but only when it provably belongs to the
/// requested window. Returns a retained element the caller must release.
///
/// This is the only focused-element reader background window-scoped keyboard
/// paths may use: a PID-global focused element can belong to a sibling window,
/// and sibling state must never address or confirm the requested target.
///
/// # Safety
///
/// Caller must `CFRelease` the returned element.
pub unsafe fn focused_element_in_window(pid: i32, window_id: u32) -> Option<AXUIElementRef> {
    let element = focused_element_of_pid(pid)?;
    if element_window_id(element) == Some(window_id) {
        Some(element)
    } else {
        CFRelease(element as CFTypeRef);
        None
    }
}

/// Observed application keyboard window. Missing AX metadata stays unknown.
/// The caller must cross-check the id against the same PID's WindowServer rows.
pub fn keyboard_focused_window_id(pid: i32) -> Option<u32> {
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return None;
        }
        super::bindings::AXUIElementSetMessagingTimeout(app, 0.05);
        let window = copy_element_attr(app, "AXFocusedWindow");
        CFRelease(app as CFTypeRef);
        window.and_then(|window| {
            let id = ax_get_window_id(window);
            CFRelease(window as CFTypeRef);
            id
        })
    }
}

/// Read-only proof of the application's keyboard destination. The application's
/// focused window must identify this exact window, even while inactive. No
/// activation or focus write is performed.
pub fn keyboard_focus_matches_window(pid: i32, window_id: u32) -> bool {
    keyboard_focus_matches_target(pid, window_id, None)
}

fn keyboard_focus_facts_match(
    focused_window_matches: bool,
    app_focused_element_matches: Option<bool>,
    addressed_element_focus_matches: Option<bool>,
) -> bool {
    focused_window_matches
        && match app_focused_element_matches {
            Some(matches) => matches,
            None => addressed_element_focus_matches.unwrap_or(true),
        }
}

/// An explicitly addressed field must itself remain the keyboard recipient.
pub fn keyboard_focus_matches_target(
    pid: i32,
    window_id: u32,
    expected_element: Option<usize>,
) -> bool {
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return false;
        }
        super::bindings::AXUIElementSetMessagingTimeout(app, 0.15);
        let window = copy_element_attr(app, "AXFocusedWindow");
        let focused = copy_element_attr(app, "AXFocusedUIElement");
        let focused_window_matches =
            window.is_some_and(|window| ax_get_window_id(window) == Some(window_id));
        let app_focused_element_matches = focused.map(|element| {
            element_window_id(element) == Some(window_id)
                && expected_element.is_none_or(|expected| {
                    CFEqual(element as CFTypeRef, expected as CFTypeRef) != 0
                })
        });
        // Chromium retains AXFocusedWindow while inactive but can omit the
        // app-level AXFocusedUIElement. The exact addressed field can prove its
        // own focus in that window; AXFocused alone is insufficient because a
        // background sibling's DOM field may also report it.
        let addressed_element_focus_matches = if focused.is_none() {
            expected_element.map(|element| {
                element_window_id(element as AXUIElementRef) == Some(window_id)
                    && copy_bool_attr(element as AXUIElementRef, "AXFocused") == Some(true)
            })
        } else {
            None
        };
        let matches = keyboard_focus_facts_match(
            focused_window_matches,
            app_focused_element_matches,
            addressed_element_focus_matches,
        );
        if let Some(window) = window {
            CFRelease(window as CFTypeRef);
        }
        if let Some(element) = focused {
            CFRelease(element as CFTypeRef);
        }
        CFRelease(app as CFTypeRef);
        matches
    }
}

/// One fresh `AXWindows` row. Unreadable attributes remain unknown; absence
/// alone never proves that a sibling cannot receive keyboard input.
struct AxWindowRecord {
    window_id: u32,
    minimized: Option<bool>,
    subrole: Option<String>,
    main: Option<bool>,
    focused: Option<bool>,
}

/// Map the application's fresh `AXWindows` through `_AXUIElementGetWindow`.
/// Windows whose id the SPI cannot resolve are omitted: an unmappable window
/// can never satisfy an exact-target requirement.
unsafe fn ax_window_records(app: AXUIElementRef) -> Vec<AxWindowRecord> {
    let focused_window = copy_element_attr(app, "AXFocusedWindow");
    let focused_window_id = focused_window.and_then(|window| {
        AXUIElementSetMessagingTimeout(window, GATE_TIMEOUT_SECONDS);
        let id = ax_get_window_id(window);
        CFRelease(window as CFTypeRef);
        id
    });
    copy_ax_windows(app)
        .into_iter()
        .filter_map(|window| {
            AXUIElementSetMessagingTimeout(window, GATE_TIMEOUT_SECONDS);
            let record = ax_get_window_id(window).map(|window_id| AxWindowRecord {
                window_id,
                minimized: copy_bool_attr(window, "AXMinimized"),
                subrole: copy_string_attr(window, "AXSubrole"),
                main: copy_bool_attr(window, "AXMain"),
                focused: if focused_window_id == Some(window_id) {
                    Some(true)
                } else {
                    copy_bool_attr(window, "AXFocused")
                        .or_else(|| focused_window_id.map(|id| id == window_id))
                },
            });
            CFRelease(window as CFTypeRef);
            record
        })
        .collect()
}

/// Count independently AX-mapped, non-minimized sibling top-level windows.
///
/// WindowServer may expose several layer-0 compositor surfaces for one native
/// Electron, Tauri, or WebKit window. A raw same-pid CGWindow row is therefore
/// not enough to prove another process-scoped keyboard destination. Requiring a
/// fresh `AXWindows` mapping preserves the fail-closed two-window guard while
/// ignoring render surfaces that cannot independently become the AX key window.
/// A mapped sibling is excluded only for a known non-keyboard subrole, or
/// explicit off-screen evidence with AX main/focus both proven false. Size is
/// irrelevant; missing visibility or AX focus metadata keeps the refusal.
fn count_competing_keyboard_destinations(
    pid: i32,
    target_window_id: u32,
    window_server_rows: impl IntoIterator<Item = (i32, u32, Option<bool>)>,
    ax_records: &[AxWindowRecord],
) -> usize {
    window_server_rows
        .into_iter()
        .filter(|(owner_pid, window_id, on_screen)| {
            *owner_pid == pid
                && *window_id != target_window_id
                && ax_records.iter().any(|record| {
                    let non_keyboard_subrole = record.subrole.as_deref().is_some_and(|subrole| {
                        !subrole.trim().is_empty()
                            && !matches!(
                                subrole,
                                "AXUnknown"
                                    | "AXStandardWindow"
                                    | "AXDialog"
                                    | "AXSystemDialog"
                                    | "AXFloatingWindow"
                            )
                    });
                    let off_screen_non_key = *on_screen == Some(false)
                        && record.main == Some(false)
                        && record.focused == Some(false);
                    // A panel with an unusual subrole can still hold the key
                    // window: its subrole excludes it only with AX main and
                    // focus both proven false, like the off-screen rule.
                    let non_keyboard_subrole = non_keyboard_subrole
                        && record.main == Some(false)
                        && record.focused == Some(false);
                    record.window_id == *window_id
                        && record.minimized != Some(true)
                        && !non_keyboard_subrole
                        && !off_screen_non_key
                })
        })
        .count()
}

/// Gather fresh background-input facts for one `(pid, window_id)` target.
///
/// `element_ptr` is an optional retained `AXUIElementRef` (as `usize`) for an
/// explicitly addressed element; the caller must keep it retained for the
/// duration of this call. Blocking: performs one CGWindowList enumeration and
/// bounded AX reads. Call from a blocking context immediately before deciding.
pub fn gather_background_facts(
    pid: i32,
    window_id: u32,
    element_ptr: Option<usize>,
) -> BackgroundTargetFacts {
    let window_server = match resolve_window_owner(pid, window_id) {
        WindowOwner::SamePid => WindowServerOwnership::SamePid,
        WindowOwner::Unknown => WindowServerOwnership::NotFound,
        WindowOwner::ForeignPid { owner_pid, .. } => {
            WindowServerOwnership::ForeignPid { owner_pid }
        }
    };

    // SAFETY: the application element is created and released here; window
    // elements are released inside ax_window_records; the caller guarantees
    // element_ptr stays retained.
    let (records, app_hidden, element) = unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            (
                Vec::new(),
                None,
                element_ptr.map(|_| ElementAncestry::Unproven),
            )
        } else {
            AXUIElementSetMessagingTimeout(app, super::tree::AX_MESSAGING_TIMEOUT_SECONDS);
            // Electron/Chromium apps may need per-process-lifetime enablement
            // before their AX windows and subtrees are materialized.
            super::enablement::ensure_chromium_ax_enabled(pid, app);
            AXUIElementSetMessagingTimeout(app, GATE_TIMEOUT_SECONDS);
            let records = ax_window_records(app);
            let app_hidden = copy_bool_attr(app, "AXHidden");
            let element = element_ptr.map(|ptr| {
                // The addressed element is the cached object a later AXPress
                // uses, so it keeps the walk's bound instead of the gate's.
                AXUIElementSetMessagingTimeout(
                    ptr as AXUIElementRef,
                    super::tree::AX_MESSAGING_TIMEOUT_SECONDS,
                );
                let probed = element_window_id(ptr as AXUIElementRef);
                match probed {
                    Some(id) if id == window_id => ElementAncestry::ProvenDescendant,
                    Some(_)
                        if element_in_attached_window(pid, ptr as AXUIElementRef, window_id) =>
                    {
                        ElementAncestry::AttachedModal
                    }
                    Some(_) => ElementAncestry::OutsideTargetWindow,
                    None => ElementAncestry::Unproven,
                }
            });
            CFRelease(app as CFTypeRef);
            (records, app_hidden, element)
        }
    };

    let target = records.iter().find(|record| record.window_id == window_id);
    let competing_keyboard_destinations = count_competing_keyboard_destinations(
        pid,
        window_id,
        all_windows()
            .iter()
            .map(|window| (window.pid, window.window_id, window.on_screen_evidence)),
        &records,
    );

    BackgroundTargetFacts {
        window_server,
        ax_window_present: target.is_some(),
        target_minimized: target.and_then(|record| record.minimized),
        app_hidden,
        competing_keyboard_destinations,
        // Only focused-key admission and the explicit capability query prove
        // the addressed field's keyboard focus. Sibling focus facts above do
        // not establish that the requested field is the keyboard recipient.
        keyboard_focus_proven: false,
        element: element.unwrap_or(ElementAncestry::NotAddressed),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        count_competing_keyboard_destinations, keyboard_focus_facts_match, AxWindowRecord,
    };

    #[test]
    fn inactive_app_focus_requires_exact_window_and_addressed_field() {
        // An inactive Chromium app can omit the app-level focused element.
        assert!(keyboard_focus_facts_match(true, None, Some(true)));
        assert!(!keyboard_focus_facts_match(true, None, Some(false)));
        // A window-only key is bound by the application's exact focused window.
        assert!(keyboard_focus_facts_match(true, None, None));
        // A sibling's focused field never overrides the app's focused window.
        assert!(!keyboard_focus_facts_match(false, None, Some(true)));
        assert!(!keyboard_focus_facts_match(false, Some(true), Some(true)));
        // Contradictory app-level focus always refuses, even if a field claims focus.
        assert!(!keyboard_focus_facts_match(true, Some(false), Some(true)));
        assert!(keyboard_focus_facts_match(true, Some(true), None));
        assert!(!keyboard_focus_facts_match(false, None, None));
    }

    fn ax_window(window_id: u32, minimized: Option<bool>) -> AxWindowRecord {
        AxWindowRecord {
            window_id,
            minimized,
            subrole: Some("AXStandardWindow".to_string()),
            main: Some(false),
            focused: Some(false),
        }
    }

    #[test]
    fn compositor_surfaces_do_not_create_keyboard_ambiguity() {
        let rows = [
            (42, 10, Some(true)),
            (42, 11, Some(true)),
            (42, 12, Some(true)),
            (42, 13, Some(true)),
            (42, 14, Some(true)),
            (42, 15, Some(true)),
        ];
        let records = [ax_window(10, Some(false))];

        assert_eq!(
            count_competing_keyboard_destinations(42, 10, rows, &records),
            0
        );
    }

    #[test]
    fn independently_mapped_sibling_remains_ambiguous() {
        let rows = [(42, 10, Some(true)), (42, 11, Some(true))];
        let records = [ax_window(10, Some(false)), ax_window(11, Some(false))];

        assert_eq!(
            count_competing_keyboard_destinations(42, 10, rows, &records),
            1
        );
    }

    #[test]
    fn minimized_mapped_sibling_is_not_a_keyboard_destination() {
        let rows = [(42, 10, Some(true)), (42, 11, Some(true))];
        let records = [ax_window(10, Some(false)), ax_window(11, Some(true))];

        assert_eq!(
            count_competing_keyboard_destinations(42, 10, rows, &records),
            0
        );
    }

    #[test]
    fn unmapped_window_server_sibling_is_not_a_keyboard_destination() {
        let rows = [
            (42, 10, Some(true)),
            (42, 99, Some(true)),
            (7, 11, Some(true)),
        ];
        let records = [ax_window(10, Some(false)), ax_window(11, Some(false))];

        assert_eq!(
            count_competing_keyboard_destinations(42, 10, rows, &records),
            0
        );
    }

    #[test]
    fn focused_or_unproven_non_standard_subrole_sibling_remains_ambiguous() {
        let rows = [(42, 10, Some(true)), (42, 11, Some(true))];
        for (main, focused) in [
            (Some(false), Some(true)),
            (Some(true), Some(false)),
            (None, Some(false)),
            (Some(false), None),
        ] {
            let mut sibling = ax_window(11, Some(false));
            sibling.subrole = Some("AXSystemFloatingWindow".to_string());
            sibling.main = main;
            sibling.focused = focused;
            let records = [ax_window(10, Some(false)), sibling];
            assert_eq!(
                count_competing_keyboard_destinations(42, 10, rows, &records),
                1
            );
        }
    }

    #[test]
    fn non_standard_subrole_sibling_is_not_a_keyboard_destination() {
        let rows = [(42, 10, Some(true)), (42, 11, Some(true))];
        let mut sibling = ax_window(11, Some(false));
        sibling.subrole = Some("AXSystemFloatingWindow".to_string());
        let records = [ax_window(10, Some(false)), sibling];
        assert_eq!(
            count_competing_keyboard_destinations(42, 10, rows, &records),
            0
        );
    }

    #[test]
    fn off_screen_non_focused_sibling_is_not_a_keyboard_destination() {
        let rows = [(42, 10, Some(true)), (42, 11, Some(false))];
        let records = [ax_window(10, Some(false)), ax_window(11, Some(false))];
        assert_eq!(
            count_competing_keyboard_destinations(42, 10, rows, &records),
            0
        );
    }

    #[test]
    fn off_screen_but_ax_focused_sibling_remains_ambiguous() {
        let rows = [(42, 10, Some(true)), (42, 11, Some(false))];
        let mut sibling = ax_window(11, Some(false));
        sibling.focused = Some(true);
        let records = [ax_window(10, Some(false)), sibling];
        assert_eq!(
            count_competing_keyboard_destinations(42, 10, rows, &records),
            1
        );
    }

    #[test]
    fn off_screen_but_ax_main_sibling_remains_ambiguous() {
        let rows = [(42, 10, Some(true)), (42, 11, Some(false))];
        let mut sibling = ax_window(11, Some(false));
        sibling.main = Some(true);
        let records = [ax_window(10, Some(false)), sibling];
        assert_eq!(
            count_competing_keyboard_destinations(42, 10, rows, &records),
            1
        );
    }

    #[test]
    fn unknown_subrole_on_screen_sibling_remains_ambiguous() {
        for subrole in [None, Some(""), Some("AXUnknown")] {
            let rows = [(42, 10, Some(true)), (42, 11, Some(true))];
            let mut sibling = ax_window(11, Some(false));
            sibling.subrole = subrole.map(str::to_string);
            let records = [ax_window(10, Some(false)), sibling];
            assert_eq!(
                count_competing_keyboard_destinations(42, 10, rows, &records),
                1
            );
        }
    }

    #[test]
    fn keyboard_subroles_on_screen_remain_ambiguous() {
        for subrole in [
            "AXStandardWindow",
            "AXDialog",
            "AXSystemDialog",
            "AXFloatingWindow",
        ] {
            let rows = [(42, 10, Some(true)), (42, 11, Some(true))];
            let mut sibling = ax_window(11, Some(false));
            sibling.subrole = Some(subrole.to_string());
            let records = [ax_window(10, Some(false)), sibling];
            assert_eq!(
                count_competing_keyboard_destinations(42, 10, rows, &records),
                1
            );
        }
    }

    #[test]
    fn unknown_visibility_or_ax_focus_keeps_sibling_ambiguous() {
        for (on_screen, main, focused) in [
            (None, Some(false), Some(false)),
            (Some(false), None, Some(false)),
            (Some(false), Some(false), None),
        ] {
            let rows = [(42, 10, Some(true)), (42, 11, on_screen)];
            let mut sibling = ax_window(11, Some(false));
            sibling.main = main;
            sibling.focused = focused;
            let records = [ax_window(10, Some(false)), sibling];
            assert_eq!(
                count_competing_keyboard_destinations(42, 10, rows, &records),
                1
            );
        }
    }
}
