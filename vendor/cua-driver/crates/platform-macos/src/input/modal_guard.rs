//! Exact modal admission. A normal dialog is an input target, not an authorization UI.
//! Capture once per action; only actions with a modal need an extra ancestry check.

use crate::ax::bindings::{
    ax_get_window_id, copy_bool_attr, copy_element_attr, copy_string_attr,
    AXUIElementCreateApplication, AXUIElementRef, AXUIElementSetMessagingTimeout,
};
use crate::ax::cache::RetainedElement;
use core_foundation::base::{CFEqual, CFRelease, CFTypeRef};

use super::target_guard::TargetRefusal;

const MAX_ANCESTORS: usize = 40;

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

fn modal_role(role: &str, subrole: &str, modal: Option<bool>) -> bool {
    role == "AXSheet"
        || (role == "AXWindow"
            && (modal == Some(true) || matches!(subrole, "AXDialog" | "AXSystemDialog")))
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
        unsafe {
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return Ok(None);
            }
            AXUIElementSetMessagingTimeout(app, 0.1);
            let mut current = copy_element_attr(app, "AXFocusedUIElement")
                .or_else(|| copy_element_attr(app, "AXFocusedWindow"));
            CFRelease(app as CFTypeRef);
            for _ in 0..MAX_ANCESTORS {
                let Some(element) = current.take() else { break };
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
                let is_modal = if role == "AXWindow" {
                    copy_bool_attr(element, "AXModal")
                } else {
                    None
                };
                if modal_role(&role, &subrole, is_modal) {
                    let own_window_id = ax_get_window_id(element);
                    let parent_window_id =
                        copy_element_attr(element, "AXParent").and_then(|parent| {
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
            Ok(None)
        }
    }

    pub(crate) fn verify(
        &self,
        pid: i32,
        window_id: u32,
        element: Option<usize>,
    ) -> Result<(), TargetRefusal> {
        let descendant = element.is_some_and(|element| unsafe {
            is_descendant(
                element as AXUIElementRef,
                self.element.as_ptr() as AXUIElementRef,
            )
        });
        if modal_target_matches(element.is_some(), descendant, self.raw_window_id, window_id) {
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

unsafe fn is_descendant(element: AXUIElementRef, modal: AXUIElementRef) -> bool {
    let mut current = Some(RetainedElement::retain(element as usize));
    for _ in 0..MAX_ANCESTORS {
        let Some(retained) = current.take() else {
            break;
        };
        let element = retained.as_ptr() as AXUIElementRef;
        if CFEqual(element as CFTypeRef, modal as CFTypeRef) != 0 {
            return true;
        }
        current = copy_element_attr(element, "AXParent").map(|parent| {
            let retained = RetainedElement::retain(parent as usize);
            CFRelease(parent as CFTypeRef);
            retained
        });
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(modal_role("AXSheet", "", None));
        assert!(modal_role("AXWindow", "AXDialog", None));
        assert!(modal_role("AXWindow", "", Some(true)));
        assert!(!modal_role("AXWindow", "AXStandardWindow", Some(false)));
    }

    #[test]
    fn sheet_controls_are_allowed_but_parent_and_siblings_are_not() {
        assert!(modal_target_matches(true, true, None, 7));
        assert!(!modal_target_matches(true, false, None, 7));
        assert!(!modal_target_matches(false, false, None, 7));
        assert!(!modal_target_matches(true, false, Some(7), 7));
    }

    #[test]
    fn raw_input_requires_the_modals_own_distinct_window() {
        assert!(modal_target_matches(false, false, Some(8), 8));
        assert!(!modal_target_matches(false, false, Some(8), 7));
        assert!(!modal_target_matches(false, false, None, 8));
    }
}
