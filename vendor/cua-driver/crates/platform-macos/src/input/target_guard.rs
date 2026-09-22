//! Read-only, exact-window input admission. Focus-sensitive actions stay bound
//! to the active Space. Exact semantic text may instead stay bound to unchanged
//! WindowServer Space membership without switching the user's active Space.

use crate::windows::{WindowBounds, WindowInfo};
use cua_driver_core::protocol::ToolResult;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Once,
};

static SPACE_EPOCH: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpacePolicy {
    ActiveSpace,
    StableMembership,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
impl ExpectedBounds {
    fn valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.width > 0.0
            && self.height > 0.0
    }
    fn matches(&self, actual: &WindowBounds) -> bool {
        self.x == actual.x
            && self.y == actual.y
            && self.width == actual.width
            && self.height == actual.height
    }
}

#[derive(Clone, Debug)]
pub struct TargetRefusal {
    pub code: &'static str,
    pub reason: String,
    pub pid: i32,
    pub window_id: u32,
}
impl TargetRefusal {
    pub(crate) fn new(
        code: &'static str,
        reason: impl Into<String>,
        pid: i32,
        window_id: u32,
    ) -> Self {
        Self {
            code,
            reason: reason.into(),
            pid,
            window_id,
        }
    }
    pub fn tool_result(&self) -> ToolResult {
        ToolResult::error(format!(
            "Input target refused ({}): {}",
            self.code, self.reason
        ))
        .with_structured(json!({"ready":false,"effect":"refused","code":self.code,
                "reason":self.reason,"pid":self.pid,"window_id":self.window_id}))
    }
}
impl std::fmt::Display for TargetRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.reason)
    }
}
impl std::error::Error for TargetRefusal {}

#[derive(Clone, Debug)]
pub struct InputTarget {
    pub pid: i32,
    pub window_id: u32,
    expected: Option<ExpectedBounds>,
    space_policy: SpacePolicy,
    space_epoch: u64,
    current_space: Option<u64>,
    space_ids: Option<Vec<u64>>,
    modal: Option<super::modal_guard::ModalTarget>,
}
impl InputTarget {
    fn parse(args: &Value, space_policy: SpacePolicy) -> Result<Self, TargetRefusal> {
        let pid = args
            .get("pid")
            .and_then(Value::as_i64)
            .and_then(|pid| i32::try_from(pid).ok())
            .filter(|pid| *pid > 0);
        let window_id = args
            .get("window_id")
            .and_then(Value::as_u64)
            .and_then(|wid| u32::try_from(wid).ok())
            .filter(|wid| *wid > 0);
        let (pid, window_id) = match (pid, window_id) {
            (Some(pid), Some(wid)) => (pid, wid),
            _ => {
                return Err(TargetRefusal::new(
                    "stale_target",
                    "An exact positive pid and window_id are required.",
                    pid.unwrap_or(0),
                    window_id.unwrap_or(0),
                ))
            }
        };
        let expected = args.get("expected_window_bounds").map(|value| {
            serde_json::from_value::<ExpectedBounds>(value.clone()).ok()
                .filter(ExpectedBounds::valid).ok_or_else(|| TargetRefusal::new(
                    "stale_geometry", "expected_window_bounds must contain finite x/y and positive width/height.", pid, window_id))
        }).transpose()?;
        Ok(Self {
            pid,
            window_id,
            expected,
            space_policy,
            space_epoch: SPACE_EPOCH.load(Ordering::Acquire),
            current_space: None,
            space_ids: None,
            modal: None,
        })
    }

    pub fn prepare(args: &Value) -> Result<Self, TargetRefusal> {
        Self::prepare_with_space_policy(args, SpacePolicy::ActiveSpace)
    }

    pub fn prepare_exact_semantic(args: &Value) -> Result<Self, TargetRefusal> {
        Self::prepare_with_space_policy(args, SpacePolicy::StableMembership)
    }

    fn prepare_with_space_policy(
        args: &Value,
        space_policy: SpacePolicy,
    ) -> Result<Self, TargetRefusal> {
        install_space_observer();
        let mut target = Self::parse(args, space_policy)?;
        let window = current_window(target.window_id);
        target.verify_snapshot(window.as_ref(), SPACE_EPOCH.load(Ordering::Acquire))?;
        if let Some(window) = window {
            target.current_space = window.current_space_id;
            target.space_ids = window.space_ids;
        }
        target.modal = super::modal_guard::ModalTarget::capture(target.pid, target.window_id)?;
        Ok(target)
    }

    pub(crate) fn verify_modal(&self, element: Option<usize>) -> Result<(), TargetRefusal> {
        if let Some(modal) = &self.modal {
            modal.verify(self.pid, self.window_id, element)?;
        }
        Ok(())
    }

    pub(crate) fn has_modal(&self) -> bool {
        self.modal.is_some()
    }

    pub fn requires_active_space(&self) -> bool {
        self.space_policy == SpacePolicy::ActiveSpace
    }

    pub fn verify(&self) -> Result<(), TargetRefusal> {
        self.verify_snapshot(
            current_window(self.window_id).as_ref(),
            SPACE_EPOCH.load(Ordering::Acquire),
        )
    }

    fn verify_snapshot(
        &self,
        window: Option<&WindowInfo>,
        epoch: u64,
    ) -> Result<(), TargetRefusal> {
        let refuse = |code, reason| TargetRefusal::new(code, reason, self.pid, self.window_id);
        if self.requires_active_space() && epoch != self.space_epoch {
            return Err(refuse("target_not_on_active_space", "The active Space changed during this action; observe the target before continuing."));
        }
        let Some(window) =
            window.filter(|window| window.pid == self.pid && window.window_id == self.window_id)
        else {
            return Err(refuse(
                "stale_target",
                "The exact window closed or changed owner.",
            ));
        };
        if window.bounds.width <= 0.0 || window.bounds.height <= 0.0 {
            return Err(refuse(
                "input_target_unavailable",
                "The target window has no usable geometry.",
            ));
        }
        match self.space_policy {
            SpacePolicy::ActiveSpace => {
                if window.on_current_space == Some(false) {
                    return Err(refuse(
                        "target_not_on_active_space",
                        "The target window is on another Space.",
                    ));
                }
                if !window.is_on_screen {
                    return Err(refuse(
                        "input_target_unavailable",
                        "The target window is hidden, minimized or unavailable for input.",
                    ));
                }
                if let Some(space) = self.current_space {
                    if window.current_space_id != Some(space) {
                        return Err(refuse(
                            "target_not_on_active_space",
                            "The target display's active Space changed during this action.",
                        ));
                    }
                }
            }
            SpacePolicy::StableMembership => {
                match window
                    .space_ids
                    .as_ref()
                    .filter(|space_ids| !space_ids.is_empty())
                {
                    Some(actual_space_ids) => {
                        if self
                            .space_ids
                            .as_ref()
                            .is_some_and(|expected| expected != actual_space_ids)
                        {
                            return Err(refuse(
                                "stale_space_membership",
                                "The exact window changed Space membership during this action.",
                            ));
                        }
                    }
                    // A window of a hidden application is detached from every
                    // managed Space — WindowServer reports no membership at
                    // all. That state is itself stable: while the app stays
                    // hidden the window cannot migrate Spaces, and the
                    // element-ancestry proof still binds the action to this
                    // exact window. Accept it as stably-detached membership;
                    // any other empty report stays fail-closed.
                    None if crate::apps::app_is_hidden(window.pid) => {}
                    None => {
                        return Err(refuse(
                            "space_membership_unresolved",
                            "WindowServer did not report stable Space membership for the exact window.",
                        ));
                    }
                }
            }
        }
        if self
            .expected
            .as_ref()
            .is_some_and(|expected| !expected.matches(&window.bounds))
        {
            return Err(refuse(
                "stale_geometry",
                "The target bounds differ from the observed coordinate frame.",
            ));
        }
        Ok(())
    }
}

fn current_window(window_id: u32) -> Option<WindowInfo> {
    // kCGWindowListOptionIncludingWindow obtains only this window's metadata;
    // no capture, AX mutation, activation or full-desktop enumeration is needed.
    crate::windows::input_window_info_by_id(window_id)
}

pub fn readiness(args: &Value) -> ToolResult {
    match InputTarget::prepare(args) {
        Ok(target) => ToolResult::text("The exact window is ready for input on the current Space.")
            .with_structured(json!({"ready":true,"pid":target.pid,"window_id":target.window_id})),
        Err(refusal) => refusal.tool_result(),
    }
}

/// Whether macOS is currently suppressing synthesized key events because a
/// password field holds secure input. Authoritative HIToolbox state, read at
/// dispatch time so keyboard tools refuse before any target admission.
pub fn secure_input_active() -> bool {
    unsafe { IsSecureEventInputEnabled() != 0 }
}

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

fn install_space_observer() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        use block2::RcBlock;
        use objc2_app_kit::{NSWorkspace, NSWorkspaceActiveSpaceDidChangeNotification};
        use objc2_foundation::{NSNotification, NSOperationQueue};
        use std::ptr::NonNull;
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };
        let center = unsafe { workspace.notificationCenter() };
        let queue = unsafe { NSOperationQueue::new() };
        unsafe { queue.setMaxConcurrentOperationCount(1) };
        let block = RcBlock::new(|_: NonNull<NSNotification>| {
            SPACE_EPOCH.fetch_add(1, Ordering::AcqRel);
            super::cancellation::gate().target_space_changed();
        });
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceActiveSpaceDidChangeNotification),
                None,
                Some(&queue),
                &block,
            )
        };
        // Process-lifetime observer, matching the existing activation observer.
        std::mem::forget(token);
        std::mem::forget(queue);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn window() -> WindowInfo {
        WindowInfo {
            window_id: 7,
            pid: 42,
            app_name: "fixture".into(),
            title: "fixture".into(),
            bounds: WindowBounds {
                x: -500.0,
                y: 30.0,
                width: 400.0,
                height: 300.0,
            },
            layer: 0,
            z_index: 0,
            is_on_screen: true,
            on_screen_evidence: Some(true),
            current_space_id: Some(9),
            on_current_space: Some(true),
            space_ids: Some(vec![9]),
        }
    }
    fn target() -> InputTarget {
        let mut target = InputTarget::parse(&json!({"pid":42,"window_id":7,"expected_window_bounds":{"x":-500,"y":30,"width":400,"height":300}}), SpacePolicy::ActiveSpace).unwrap();
        target.space_epoch = 4;
        target.current_space = Some(9);
        target.space_ids = Some(vec![9]);
        target
    }
    fn semantic_target() -> InputTarget {
        let mut target = InputTarget::parse(
            &json!({"pid":42,"window_id":7,"expected_window_bounds":{"x":-500,"y":30,"width":400,"height":300}}),
            SpacePolicy::StableMembership,
        )
        .unwrap();
        target.space_epoch = 4;
        target.current_space = Some(9);
        target.space_ids = Some(vec![9]);
        target
    }
    #[test]
    fn exact_identity_and_frame_are_required_before_input() {
        let target = target();
        let mut actual = window();
        assert!(target.verify_snapshot(Some(&actual), 4).is_ok());
        actual.pid = 43;
        assert_eq!(
            target.verify_snapshot(Some(&actual), 4).unwrap_err().code,
            "stale_target"
        );
        actual = window();
        actual.bounds.x += 1.0;
        assert_eq!(
            target.verify_snapshot(Some(&actual), 4).unwrap_err().code,
            "stale_geometry"
        );
        assert_eq!(
            target.verify_snapshot(None, 4).unwrap_err().code,
            "stale_target"
        );
    }
    #[test]
    fn space_epoch_invalidates_even_if_the_window_returns_to_the_same_space() {
        let target = target();
        let actual = window();
        assert_eq!(
            target.verify_snapshot(Some(&actual), 5).unwrap_err().code,
            "target_not_on_active_space"
        );
        let mut next = target.clone();
        next.space_epoch = 5;
        assert!(next.verify_snapshot(Some(&actual), 5).is_ok());
    }
    #[test]
    fn fresh_space_visibility_and_display_membership_are_checked() {
        let target = target();
        let mut actual = window();
        actual.on_current_space = Some(false);
        assert_eq!(
            target.verify_snapshot(Some(&actual), 4).unwrap_err().code,
            "target_not_on_active_space"
        );
        actual = window();
        actual.is_on_screen = false;
        assert_eq!(
            target.verify_snapshot(Some(&actual), 4).unwrap_err().code,
            "input_target_unavailable"
        );
        actual = window();
        actual.current_space_id = Some(10);
        assert_eq!(
            target.verify_snapshot(Some(&actual), 4).unwrap_err().code,
            "target_not_on_active_space"
        );
    }
    #[test]
    fn exact_semantic_target_survives_active_space_and_visibility_changes() {
        let target = semantic_target();
        let mut actual = window();
        actual.on_current_space = Some(false);
        actual.is_on_screen = false;
        actual.current_space_id = Some(10);
        assert!(target.verify_snapshot(Some(&actual), 5).is_ok());
    }
    #[test]
    fn exact_semantic_target_requires_stable_known_space_membership() {
        let target = semantic_target();
        let mut actual = window();
        actual.space_ids = Some(vec![10]);
        assert_eq!(
            target.verify_snapshot(Some(&actual), 5).unwrap_err().code,
            "stale_space_membership"
        );
        actual.space_ids = None;
        assert_eq!(
            target.verify_snapshot(Some(&actual), 5).unwrap_err().code,
            "space_membership_unresolved"
        );
        actual.space_ids = Some(Vec::new());
        assert_eq!(
            target.verify_snapshot(Some(&actual), 5).unwrap_err().code,
            "space_membership_unresolved"
        );
    }
    #[test]
    fn malformed_bounds_and_ambiguous_identity_fail_closed() {
        for args in [
            json!({"pid":42}),
            json!({"pid":-1,"window_id":7}),
            json!({"pid":42,"window_id":0}),
        ] {
            assert_eq!(
                InputTarget::parse(&args, SpacePolicy::ActiveSpace)
                    .unwrap_err()
                    .code,
                "stale_target"
            );
        }
        let args = json!({"pid":42,"window_id":7,"expected_window_bounds":{"x":0,"y":0,"width":0,"height":50}});
        assert_eq!(
            InputTarget::parse(&args, SpacePolicy::ActiveSpace)
                .unwrap_err()
                .code,
            "stale_geometry"
        );
        let refused = target().verify_snapshot(None, 4).unwrap_err().tool_result();
        let value = serde_json::to_value(refused).unwrap();
        assert_eq!(value["structuredContent"]["effect"], "refused");
        assert_eq!(value["structuredContent"]["ready"], false);
    }
}
