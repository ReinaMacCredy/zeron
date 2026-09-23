//! Exact modal admission. A normal dialog is an input target, not an authorization UI.
//! Capture once per action; only actions with a modal need an extra ancestry check.

use crate::ax::bindings::{
    ax_get_window_id, copy_bool_attr, copy_children, copy_element_attr, copy_string_attr,
    kAXErrorCannotComplete, kAXErrorSuccess, AXError, AXUIElementCopyAttributeValue,
    AXUIElementCreateApplication, AXUIElementGetTypeID, AXUIElementRef,
    AXUIElementSetMessagingTimeout,
};
use crate::ax::cache::RetainedElement;
use core_foundation::array::CFArray;
use core_foundation::base::{CFEqual, CFGetTypeID, CFRelease, CFTypeRef, TCFType};
use core_foundation::string::CFString;
use std::time::{Duration, Instant};

use super::target_guard::TargetRefusal;

const MAX_ANCESTORS: usize = 40;
const MODAL_TIMEOUT_SECONDS: f32 = 0.1;

#[derive(Clone)]
pub(crate) struct ModalTarget {
    element: RetainedElement,
    // Only a separately addressable modal window permits raw window input.
    raw_window_id: Option<u32>,
}

impl std::fmt::Debug for ModalTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModalTarget")
            .field("raw_window_id", &self.raw_window_id)
            .finish_non_exhaustive()
    }
}

fn protected_bundle(bundle: Option<&str>) -> bool {
    bundle.is_some_and(|bundle| {
        matches!(
            bundle.to_ascii_lowercase().as_str(),
            "com.apple.securityagent"
                | "com.apple.security.authorization"
                | "com.apple.coreautha.uiagent"
                | "com.apple.localauthentication.uiagent"
                | "com.apple.loginwindow"
        )
    })
}

fn modal_role(role: &str, subrole: &str, modal: Option<bool>, identifier: Option<&str>) -> bool {
    role == "AXSheet"
        || (role == "AXWindow"
            && (modal == Some(true)
                || matches!(subrole, "AXDialog" | "AXSystemDialog")
                || matches!(identifier, Some("open-panel" | "save-panel"))))
}

// A reply that spent the whole messaging timeout is a stalled app; an immediate
// cannot-complete is an app with no AX server, which has no modal to guard.
fn app_read_timed_out(error: AXError, elapsed: Duration) -> bool {
    error == kAXErrorCannotComplete
        && elapsed >= Duration::from_secs_f32(MODAL_TIMEOUT_SECONDS * 0.8)
}

unsafe fn copy_app_attr(app: AXUIElementRef, attr_name: &str) -> Result<Option<CFTypeRef>, ()> {
    let attr = CFString::new(attr_name);
    let mut value: CFTypeRef = std::ptr::null();
    let started = Instant::now();
    let error = AXUIElementCopyAttributeValue(app, attr.as_concrete_TypeRef(), &mut value);
    if error == kAXErrorSuccess && !value.is_null() {
        return Ok(Some(value));
    }
    if !value.is_null() {
        CFRelease(value);
    }
    if app_read_timed_out(error, started.elapsed()) {
        Err(())
    } else {
        Ok(None)
    }
}

unsafe fn copy_app_element_attr(
    app: AXUIElementRef,
    attr_name: &str,
) -> Result<Option<AXUIElementRef>, ()> {
    Ok(copy_app_attr(app, attr_name)?.and_then(|value| {
        if CFGetTypeID(value) == AXUIElementGetTypeID() {
            Some(value as AXUIElementRef)
        } else {
            CFRelease(value);
            None
        }
    }))
}

unsafe fn requested_window_sheet(
    app: AXUIElementRef,
    window_id: u32,
) -> Result<Option<ModalTarget>, ()> {
    let Some(value) = copy_app_attr(app, "AXWindows")? else {
        return Ok(None);
    };
    if CFGetTypeID(value) != CFArray::<CFTypeRef>::type_id() {
        CFRelease(value);
        return Ok(None);
    }
    let windows = CFArray::<CFTypeRef>::wrap_under_create_rule(value as _);
    for window in windows.iter() {
        let window = *window as AXUIElementRef;
        if CFGetTypeID(window as CFTypeRef) != AXUIElementGetTypeID() {
            continue;
        }
        AXUIElementSetMessagingTimeout(window, MODAL_TIMEOUT_SECONDS);
        if ax_get_window_id(window) != Some(window_id) {
            continue;
        }
        let mut sheet = None;
        for child in copy_children(window) {
            AXUIElementSetMessagingTimeout(child, MODAL_TIMEOUT_SECONDS);
            if sheet.is_none() && copy_string_attr(child, "AXRole").as_deref() == Some("AXSheet") {
                sheet = Some(ModalTarget {
                    element: RetainedElement::retain(child as usize),
                    raw_window_id: ax_get_window_id(child).filter(|own| *own != window_id),
                });
            }
            CFRelease(child as CFTypeRef);
        }
        return Ok(sheet);
    }
    Ok(None)
}

fn modal_target_matches(
    addressed_element: bool,
    descendant: bool,
    raw_window_id: Option<u32>,
    requested_window_id: u32,
) -> bool {
    if addressed_element {
        descendant
    } else {
        raw_window_id == Some(requested_window_id)
    }
}

impl ModalTarget {
    pub(crate) fn capture(pid: i32, window_id: u32) -> Result<Option<Self>, TargetRefusal> {
        let protected = || {
            TargetRefusal::new(
                "auth_sheet_focused",
                "Authentication or protected security input requires the user; nothing was sent.",
                pid,
                window_id,
            )
        };
        if protected_bundle(crate::apps::bundle_id_for_pid(pid).as_deref()) {
            return Err(protected());
        }
        let unresponsive = || {
            TargetRefusal::new(
                "modal_state_unknown",
                "The application did not answer whether a dialog owns its input; nothing was sent. Observe again once it responds.",
                pid,
                window_id,
            )
        };
        unsafe {
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return Ok(None);
            }
            let owned_app = RetainedElement::retain(app as usize);
            CFRelease(app as CFTypeRef);
            let app_ref = owned_app.as_ptr() as AXUIElementRef;
            AXUIElementSetMessagingTimeout(app_ref, MODAL_TIMEOUT_SECONDS);
            let mut current = match copy_app_element_attr(app_ref, "AXFocusedUIElement") {
                Ok(Some(element)) => Some(element),
                Ok(None) => copy_app_element_attr(app_ref, "AXFocusedWindow")
                    .map_err(|()| unresponsive())?,
                Err(()) => return Err(unresponsive()),
            };
            for _ in 0..MAX_ANCESTORS {
                let Some(element) = current.take() else { break };
                AXUIElementSetMessagingTimeout(element, MODAL_TIMEOUT_SECONDS);
                let retained = RetainedElement::retain(element as usize);
                CFRelease(element as CFTypeRef);
                let role = copy_string_attr(element, "AXRole").unwrap_or_default();
                let subrole = if matches!(role.as_str(), "AXTextField" | "AXWindow") {
                    copy_string_attr(element, "AXSubrole").unwrap_or_default()
                } else {
                    String::new()
                };
                if role == "AXSecureTextField" || subrole == "AXSecureTextField" {
                    return Err(protected());
                }
                let (is_modal, identifier) = if role == "AXWindow" {
                    (
                        copy_bool_attr(element, "AXModal"),
                        copy_string_attr(element, "AXIdentifier"),
                    )
                } else {
                    (None, None)
                };
                if modal_role(&role, &subrole, is_modal, identifier.as_deref()) {
                    let own_window_id = ax_get_window_id(element);
                    let parent_window_id =
                        copy_element_attr(element, "AXParent").and_then(|parent| {
                            AXUIElementSetMessagingTimeout(parent, MODAL_TIMEOUT_SECONDS);
                            let id = crate::ax::exact_target::element_window_id(parent);
                            CFRelease(parent as CFTypeRef);
                            id
                        });
                    // Sheets commonly share their parent's window identity. In
                    // that case only exact descendants may be changed; accepting
                    // the parent id would admit clicks behind the sheet.
                    let raw_window_id = own_window_id.filter(|own| {
                        role == "AXWindow" || parent_window_id.is_some_and(|parent| parent != *own)
                    });
                    return Ok(Some(Self {
                        element: retained,
                        raw_window_id,
                    }));
                }
                if role == "AXWindow" || role == "AXApplication" {
                    break;
                }
                current = copy_element_attr(element, "AXParent");
            }
            if let Some(element) = current {
                CFRelease(element as CFTypeRef);
            }
            requested_window_sheet(app_ref, window_id).map_err(|()| unresponsive())
        }
    }

    pub(crate) fn verify(
        &self,
        pid: i32,
        window_id: u32,
        addressed_element: bool,
        facts: Option<&ElementFacts>,
    ) -> Result<(), TargetRefusal> {
        let descendant = facts.is_some_and(|facts| facts.has_ancestor(&self.element));
        if modal_target_matches(addressed_element, descendant, self.raw_window_id, window_id) {
            Ok(())
        } else {
            Err(TargetRefusal::new(
                "modal_target_mismatch",
                "A modal dialog owns this application's input. Observe and address a control inside that exact dialog; input behind it was not sent.",
                pid,
                window_id,
            ))
        }
    }
}

// Gathered before the input gate locks: AX IPC under that lock stalls every app's input.
pub(crate) struct ElementFacts {
    ancestors: Vec<RetainedElement>,
    window_id: Option<u32>,
}

impl ElementFacts {
    pub(crate) unsafe fn gather(element: usize) -> Self {
        let element = element as AXUIElementRef;
        AXUIElementSetMessagingTimeout(element, crate::ax::tree::AX_MESSAGING_TIMEOUT_SECONDS);
        let window_id = crate::ax::exact_target::element_window_id(element);
        let mut ancestors = Vec::new();
        let mut current = Some(RetainedElement::retain(element as usize));
        while let Some(retained) = current.take() {
            if ancestors.len() == MAX_ANCESTORS {
                break;
            }
            current =
                copy_element_attr(retained.as_ptr() as AXUIElementRef, "AXParent").map(|parent| {
                    AXUIElementSetMessagingTimeout(parent, MODAL_TIMEOUT_SECONDS);
                    let parent_retained = RetainedElement::retain(parent as usize);
                    CFRelease(parent as CFTypeRef);
                    parent_retained
                });
            ancestors.push(retained);
        }
        Self {
            ancestors,
            window_id,
        }
    }

    pub(crate) fn window_id(&self) -> Option<u32> {
        self.window_id
    }

    fn has_ancestor(&self, modal: &RetainedElement) -> bool {
        self.ancestors.iter().any(|ancestor| unsafe {
            CFEqual(ancestor.as_ptr() as CFTypeRef, modal.as_ptr() as CFTypeRef) != 0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ax::bindings::{
        kAXErrorAPIDisabled, kAXErrorAttributeUnsupported, kAXErrorFailure, kAXErrorNoValue,
    };

    #[test]
    fn ordinary_dialog_is_not_a_protected_authentication_surface() {
        assert!(!protected_bundle(Some(
            "com.blackmagic-design.DaVinciResolve"
        )));
        assert!(!protected_bundle(Some("net.imput.helium")));
        assert!(protected_bundle(Some("com.apple.SecurityAgent")));
        assert!(protected_bundle(Some(
            "com.apple.LocalAuthentication.UIAgent"
        )));
        assert!(modal_role("AXSheet", "", None, None));
        assert!(modal_role("AXWindow", "AXDialog", None, None));
        assert!(modal_role("AXWindow", "", Some(true), None));
        assert!(!modal_role(
            "AXWindow",
            "AXStandardWindow",
            Some(false),
            None
        ));
    }

    #[test]
    fn tahoe_open_and_save_panels_are_modal_by_identifier() {
        assert!(modal_role(
            "AXWindow",
            "AXStandardWindow",
            Some(false),
            Some("open-panel")
        ));
        assert!(modal_role(
            "AXWindow",
            "AXStandardWindow",
            None,
            Some("save-panel")
        ));
        assert!(!modal_role(
            "AXWindow",
            "AXStandardWindow",
            Some(false),
            Some("document-window")
        ));
    }

    #[test]
    fn only_an_app_read_that_spent_its_timeout_refuses() {
        let timeout = Duration::from_secs_f32(MODAL_TIMEOUT_SECONDS);
        assert!(app_read_timed_out(kAXErrorCannotComplete, timeout));
        assert!(!app_read_timed_out(
            kAXErrorCannotComplete,
            Duration::from_millis(1)
        ));
        for error in [
            kAXErrorNoValue,
            kAXErrorAttributeUnsupported,
            kAXErrorFailure,
            kAXErrorAPIDisabled,
        ] {
            assert!(!app_read_timed_out(error, timeout));
        }
    }

    #[test]
    fn sheet_controls_are_allowed_but_parent_and_siblings_are_not() {
        assert!(modal_target_matches(true, true, None, 7));
        assert!(!modal_target_matches(true, false, None, 7));
        assert!(!modal_target_matches(false, false, None, 7));
        assert!(!modal_target_matches(true, false, Some(7), 7));
    }

    fn app_element(pid: i32) -> RetainedElement {
        unsafe {
            let app = AXUIElementCreateApplication(pid);
            let retained = RetainedElement::retain(app as usize);
            CFRelease(app as CFTypeRef);
            retained
        }
    }

    #[test]
    fn modal_verification_reads_only_ancestry_gathered_before_the_gate_lock() {
        let modal = ModalTarget {
            element: app_element(424_242),
            raw_window_id: None,
        };
        let inside = ElementFacts {
            ancestors: vec![app_element(424_241), app_element(424_242)],
            window_id: Some(7),
        };
        let outside = ElementFacts {
            ancestors: vec![app_element(424_241)],
            window_id: Some(7),
        };
        assert!(modal.verify(424_242, 7, true, Some(&inside)).is_ok());
        assert!(modal.verify(424_242, 7, true, Some(&outside)).is_err());
        assert!(modal.verify(424_242, 7, true, None).is_err());
        assert!(modal.verify(424_242, 7, false, None).is_err());
    }

    #[test]
    fn raw_input_requires_the_modals_own_distinct_window() {
        assert!(modal_target_matches(false, false, Some(8), 8));
        assert!(!modal_target_matches(false, false, Some(8), 7));
        assert!(!modal_target_matches(false, false, None, 8));
    }
}
