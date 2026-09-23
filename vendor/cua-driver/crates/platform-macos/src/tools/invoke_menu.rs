use async_trait::async_trait;
use core_foundation::base::{CFRelease, CFTypeRef};
use cua_driver_contract::InvokeMenuInput;
use cua_driver_core::{
    action_record::{
        ActionEffect, ActionEvidence, ActionExecutionRecord, ActionTransport, ActualDelivery,
        EvidenceKind, RequestedDelivery,
    },
    protocol::ToolResult,
    tool::{Tool, ToolDef},
};
use serde_json::Value;
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc,
    },
    time::Duration,
};

use crate::ax::bindings::{
    ax_get_window_id, copy_action_names, copy_ax_windows, copy_bool_attr, copy_children,
    copy_element_attr, copy_string_attr, kAXErrorSuccess, perform_action, set_bool_attr_true,
    AXUIElementCreateApplication, AXUIElementRef, AXUIElementSetMessagingTimeout,
};

pub struct InvokeMenuTool;

const AX_MESSAGING_TIMEOUT_SECONDS: f32 = 2.0;
const MAIN_QUEUE_TIMEOUT: Duration = Duration::from_secs(5);

unsafe fn set_messaging_timeout(element: AXUIElementRef) {
    let _ = AXUIElementSetMessagingTimeout(element, AX_MESSAGING_TIMEOUT_SECONDS);
}

static DEF: std::sync::OnceLock<ToolDef> = std::sync::OnceLock::new();

fn def() -> &'static ToolDef {
    DEF.get_or_init(|| {
        let contract =
            cua_driver_contract::tool_contract("invoke_menu").expect("invoke_menu contract");
        ToolDef {
            name: contract.name,
            description: contract.description,
            input_schema: contract.input_schema,
            read_only: contract.annotations.read_only,
            destructive: contract.annotations.destructive,
            idempotent: contract.annotations.idempotent,
            open_world: contract.annotations.open_world,
        }
    })
}

fn normalized_path(path: Vec<String>) -> Result<Vec<String>, String> {
    if path.is_empty() || path.len() > 16 {
        return Err("invoke_menu: path must contain between 1 and 16 segments".into());
    }
    path.into_iter()
        .enumerate()
        .map(|(index, segment)| {
            let segment = segment.trim();
            if segment.is_empty() {
                Err(format!("invoke_menu: path segment {index} is empty"))
            } else {
                Ok(segment.to_owned())
            }
        })
        .collect()
}

/// Return the semantic menu children of an AX node. AppKit inserts an
/// untitled `AXMenu` container between a menu-bar/submenu item and its items;
/// callers express paths in visible labels, so that container is transparent.
unsafe fn semantic_children(parent: AXUIElementRef) -> Vec<AXUIElementRef> {
    let mut out = Vec::new();
    for child in copy_children(parent) {
        if copy_string_attr(child, "AXRole").as_deref() == Some("AXMenu") {
            out.extend(copy_children(child));
            CFRelease(child as CFTypeRef);
        } else {
            out.push(child);
        }
    }
    out
}

unsafe fn resolve_exact_prefix(
    menu_bar: AXUIElementRef,
    prefix: &[String],
) -> Result<AXUIElementRef, String> {
    let mut current = menu_bar;
    let mut owns_current = false;

    for (depth, segment) in prefix.iter().enumerate() {
        let children = semantic_children(current);
        if owns_current {
            CFRelease(current as CFTypeRef);
        }

        let mut matches = Vec::new();
        for child in children {
            let title = copy_string_attr(child, "AXTitle").unwrap_or_default();
            if title.trim() == segment {
                matches.push(child);
            } else {
                CFRelease(child as CFTypeRef);
            }
        }

        if matches.len() != 1 {
            let match_count = matches.len();
            for candidate in matches {
                CFRelease(candidate as CFTypeRef);
            }
            return Err(if match_count == 0 {
                format!("invoke_menu: path segment {depth} was not found")
            } else {
                format!("invoke_menu: path segment {depth} is ambiguous")
            });
        }
        current = matches.pop().expect("one exact match");
        owns_current = true;
    }

    if owns_current {
        Ok(current)
    } else {
        Err("invoke_menu: path is empty".into())
    }
}

fn choose_action(actions: &[String], final_segment: bool) -> Option<&'static str> {
    let supports = |name: &str| actions.iter().any(|action| action == name);
    let order: &[&str] = if final_segment {
        &["AXPress", "AXPick", "AXConfirm"]
    } else {
        &["AXPress", "AXPick", "AXShowMenu", "AXOpen"]
    };
    order.iter().copied().find(|action| supports(action))
}

#[derive(Debug, PartialEq)]
enum MenuFailure {
    Refused(String),
    Dispatched { action: &'static str, ax_error: i32 },
}

impl From<String> for MenuFailure {
    fn from(message: String) -> Self {
        Self::Refused(message)
    }
}

fn hop_failure(depth: usize, final_segment: bool, action: &'static str, error: i32) -> MenuFailure {
    if final_segment {
        MenuFailure::Dispatched {
            action,
            ax_error: error,
        }
    } else {
        MenuFailure::Refused(format!(
            "invoke_menu: native action for path segment {depth} failed with AX error {error}"
        ))
    }
}

unsafe fn cancel_open_menu(app: AXUIElementRef, path: &[String]) {
    let Some(menu_bar) = copy_element_attr(app, "AXMenuBar") else {
        return;
    };
    set_messaging_timeout(menu_bar);
    let top = resolve_exact_prefix(menu_bar, &path[..1]);
    CFRelease(menu_bar as CFTypeRef);
    if let Ok(top) = top {
        set_messaging_timeout(top);
        let _ = perform_action(top, "AXCancel");
        CFRelease(top as CFTypeRef);
    }
}

unsafe fn invoke_path(pid: i32, path: &[String]) -> Result<(), MenuFailure> {
    let app = AXUIElementCreateApplication(pid);
    if app.is_null() {
        return Err("invoke_menu: target application is unavailable"
            .to_owned()
            .into());
    }
    set_messaging_timeout(app);

    let mut opened_menu = false;
    let result = (|| {
        for depth in 0..path.len() {
            let final_segment = depth + 1 == path.len();
            // Resolve from the live app root for every hop. Opening a menu can
            // replace its AX objects and reorder unrelated snapshot indices.
            let menu_bar = copy_element_attr(app, "AXMenuBar")
                .ok_or_else(|| "invoke_menu: target exposes no AXMenuBar".to_owned())?;
            set_messaging_timeout(menu_bar);
            let target = resolve_exact_prefix(menu_bar, &path[..=depth]);
            CFRelease(menu_bar as CFTypeRef);
            let target = target?;
            set_messaging_timeout(target);

            if copy_bool_attr(target, "AXEnabled") == Some(false) {
                CFRelease(target as CFTypeRef);
                return Err(format!("invoke_menu: path segment {depth} is disabled").into());
            }
            let actions = copy_action_names(target);
            let Some(action) = choose_action(&actions, final_segment) else {
                CFRelease(target as CFTypeRef);
                return Err(format!(
                    "invoke_menu: path segment {depth} has no usable native menu action"
                )
                .into());
            };
            let error = perform_action(target, action);
            CFRelease(target as CFTypeRef);
            if !final_segment {
                opened_menu = true;
            }
            if error != kAXErrorSuccess {
                return Err(hop_failure(depth, final_segment, action, error));
            }
            if !final_segment {
                std::thread::sleep(Duration::from_millis(80));
            }
        }
        Ok(())
    })();

    if opened_menu && matches!(result, Err(MenuFailure::Refused(_))) {
        cancel_open_menu(app, path);
    }
    CFRelease(app as CFTypeRef);
    result
}

fn failure_result(failure: MenuFailure) -> ToolResult {
    match failure {
        MenuFailure::Refused(message) => refusal(message),
        MenuFailure::Dispatched { action, ax_error } => {
            crate::input::ax_actions::action_failure_result(
                crate::input::ax_actions::AxActionFailure::from_ax_error(action, ax_error).into(),
                "invoke_menu",
            )
        }
    }
}

/// Make one exact application window key before resolving focus-sensitive
/// native menu state.
///
/// `SLPSSetFrontProcessWithOptions(..., kCPSNoWindows)` makes the application
/// active without broadly raising its windows. That is the right default for
/// input delivery, but it can leave the requested window non-key. macOS then
/// exposes contextual Window-menu commands (including Move & Resize) as
/// disabled even though the application itself is frontmost. Raise and mark
/// only the requested AX window, then require an exact focused-window readback
/// before menu resolution proceeds.
fn focus_ax_window(pid: i32, window_id: u32) -> Result<(), String> {
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return Err("invoke_menu: target application is unavailable".into());
        }
        set_messaging_timeout(app);

        let mut target = None;
        for window in copy_ax_windows(app) {
            if target.is_none() && ax_get_window_id(window) == Some(window_id) {
                target = Some(window);
            } else {
                CFRelease(window as CFTypeRef);
            }
        }
        CFRelease(app as CFTypeRef);

        let Some(target) = target else {
            return Err("invoke_menu: target accessibility window is unavailable".into());
        };
        set_messaging_timeout(target);

        // SkyLight has requested native key status for this exact window. AX
        // raise/main/focus completes the corresponding visible and semantic
        // state; these writes remain best-effort for applications that expose
        // only a subset of the attributes.
        let _ = perform_action(target, "AXRaise");
        let _ = set_bool_attr_true(target, "AXMain");
        let _ = set_bool_attr_true(target, "AXFocused");
        CFRelease(target as CFTypeRef);
    }
    Ok(())
}

struct AxWindowFocusRequest {
    pid: i32,
    window_id: u32,
    tx: SyncSender<Result<(), String>>,
    cancelled: Arc<AtomicBool>,
}

#[link(name = "System", kind = "framework")]
extern "C" {
    static _dispatch_main_q: u8;
    fn dispatch_async_f(
        queue: *const c_void,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
}

unsafe extern "C" fn focus_ax_window_on_main(context: *mut c_void) {
    let request = unsafe { Box::from_raw(context.cast::<AxWindowFocusRequest>()) };
    if request.cancelled.load(Ordering::Acquire) {
        return;
    }
    let result = focus_ax_window(request.pid, request.window_id);
    let _ = request.tx.send(result);
}

fn focus_ax_window_with_thread_affinity(pid: i32, window_id: u32) -> Result<(), String> {
    let is_main_thread = objc2_foundation::MainThreadMarker::new().is_some();
    if pid != std::process::id() as i32 || is_main_thread {
        return focus_ax_window(pid, window_id);
    }

    // AX actions against another process execute in that process. An embedded
    // driver targeting its own window is different: AppKit services AXRaise
    // in this process, and window ordering is main-thread-only. Queue just the
    // self-process AX mutation onto AppKit's main queue, then return to the
    // blocking worker for readiness polling and menu traversal.
    let (tx, rx) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let request = Box::new(AxWindowFocusRequest {
        pid,
        window_id,
        tx,
        cancelled: Arc::clone(&cancelled),
    });
    unsafe {
        let main_queue = &raw const _dispatch_main_q as *const c_void;
        dispatch_async_f(
            main_queue,
            Box::into_raw(request).cast::<c_void>(),
            focus_ax_window_on_main,
        );
    }
    match rx.recv_timeout(MAIN_QUEUE_TIMEOUT) {
        Ok(result) => result,
        Err(error) => {
            // If AppKit never serviced the request, prevent a stale focus
            // change from firing after this tool call has already failed.
            cancelled.store(true, Ordering::Release);
            Err(format!(
                "invoke_menu: failed waiting for the embedded host window on the AppKit main queue: {error}"
            ))
        }
    }
}

fn focus_exact_window(pid: i32, window_id: u32) -> Result<(), String> {
    let native_key_requested = crate::input::skylight::make_exact_window_key(pid, window_id);
    if !native_key_requested {
        // No raising fallback: the Cocoa `activate_pid` path would bring the
        // whole target app forward (visible z-order churn) for what is meant
        // to be a background-capable action. When the target app is already
        // frontmost no activation is needed at all; otherwise use the
        // yabai-style focus-without-raise recipe, which makes the exact window
        // key without raising it, and refuse honestly when even that is
        // unavailable instead of silently raising the app.
        let frontmost = crate::apps::frontmost_pid();
        if frontmost != Some(pid) && !crate::input::skylight::activate_without_raise(pid, window_id)
        {
            return Err(
                "invoke_menu: this macOS build exposes no non-raising activation route for the \
                 target window; refusing instead of raising it"
                    .into(),
            );
        }
    }
    if !native_key_requested {
        std::thread::sleep(Duration::from_millis(120));
    }

    focus_ax_window_with_thread_affinity(pid, window_id)?;

    // AXFocusedWindow can lead AppKit's native `isKeyWindow` state while the
    // WindowServer activation is still settling. Menu validation observes the
    // latter. Require the exact app/window pair to remain stable briefly so a
    // loaded desktop cannot expose a transiently disabled contextual item.
    let deadline = std::time::Instant::now() + Duration::from_millis(800);
    let mut stable_since = None;
    loop {
        let now = std::time::Instant::now();
        if exact_window_is_ready(
            crate::apps::frontmost_pid(),
            pid,
            crate::ax::bindings::focused_window_id_of_pid(pid),
            window_id,
        ) {
            let since = stable_since.get_or_insert(now);
            if now.duration_since(*since) >= Duration::from_millis(120) {
                return Ok(());
            }
        } else {
            stable_since = None;
        }
        if now >= deadline {
            return Err(format!(
                "invoke_menu: target window {window_id} did not become stably key and frontmost"
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn exact_window_is_ready(
    frontmost_pid: Option<i32>,
    target_pid: i32,
    focused_window_id: Option<u32>,
    target_window_id: u32,
) -> bool {
    frontmost_pid == Some(target_pid) && focused_window_id == Some(target_window_id)
}

/// Where a menu invocation resolves its path from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuScope {
    /// One exact window: validate its focus-sensitive native menu state first.
    ExactWindow(u32),
    /// The application-level `AXMenuBar` of the application element. Used
    /// when the caller targets no window — including applications that have
    /// no windows at all. Nothing is focused, raised, or activated on this
    /// path.
    Application,
}

/// Decide the menu scope from the requested window anchor and the window ids
/// the target process actually owns.
///
/// The caller may omit `window_id` to invoke from the application-level menu
/// bar; this is the only possible route for a windowless app. When a window
/// IS requested, it must belong to `pid` — the exact-window semantics are
/// kept, just relocated into a testable helper. Out-of-range ids are refused
/// rather than truncated.
fn resolve_menu_scope(
    window_id: Option<u64>,
    owned_window_ids: &[u32],
) -> Result<MenuScope, String> {
    match window_id {
        None => Ok(MenuScope::Application),
        Some(window_id) => {
            let window_id = u32::try_from(window_id)
                .map_err(|_| "invoke_menu: window_id is out of range".to_owned())?;
            if owned_window_ids.contains(&window_id) {
                Ok(MenuScope::ExactWindow(window_id))
            } else {
                Err("invoke_menu: window_id does not belong to pid".to_owned())
            }
        }
    }
}

fn refusal(message: String) -> ToolResult {
    ToolResult::error(message.clone()).with_structured(serde_json::json!({
        "status": "refused",
        "refusal": { "code": "menu_path_unavailable", "message": message }
    }))
}

#[async_trait]
impl Tool for InvokeMenuTool {
    fn def(&self) -> &ToolDef {
        def()
    }

    async fn invoke(&self, args: Value) -> ToolResult {
        let input: InvokeMenuInput =
            match cua_driver_core::tool_args::parse_typed_input("invoke_menu", args) {
                Ok(input) => input,
                Err(result) => return result,
            };
        let path = match normalized_path(input.path) {
            Ok(path) => path,
            Err(error) => return refusal(error),
        };
        let pid = match i32::try_from(input.pid) {
            Ok(pid) => pid,
            Err(_) => return refusal("invoke_menu: pid is out of range".into()),
        };
        // The owned-window snapshot is captured once, before the blocking
        // task: an exact request is validated against it, while an omitted
        // `window_id` selects the application-level menu bar and never
        // touches (or needs) a window.
        let owned_window_ids: Vec<u32> = crate::windows::all_windows()
            .iter()
            .filter(|window| window.pid == pid)
            .map(|window| window.window_id)
            .collect();
        let scope = match resolve_menu_scope(input.window_id, &owned_window_ids) {
            Ok(scope) => scope,
            Err(error) => return refusal(error),
        };

        let outcome = tokio::task::spawn_blocking(move || {
            let result = match scope {
                MenuScope::ExactWindow(window_id) => {
                    let prior_frontmost = crate::apps::frontmost_pid();
                    let prior_frontmost_window =
                        prior_frontmost.and_then(crate::ax::bindings::focused_window_id_of_pid);
                    let needs_activation = prior_frontmost != Some(pid);

                    let result = focus_exact_window(pid, window_id)
                        .map_err(MenuFailure::from)
                        .and_then(|()| unsafe { invoke_path(pid, &path) });

                    // Restore the exact prior key window when one was
                    // observable, including across applications. Falling back
                    // to app activation preserves the previous behavior for
                    // apps without an AX window.
                    if let Some(prior_pid) = prior_frontmost {
                        let already_restored =
                            prior_pid == pid && prior_frontmost_window == Some(window_id);
                        if !already_restored {
                            let restored_exact =
                                prior_frontmost_window.is_some_and(|prior_window_id| {
                                    focus_exact_window(prior_pid, prior_window_id).is_ok()
                                });
                            if needs_activation && !restored_exact {
                                let _ = crate::apps::activate_pid(prior_pid);
                            }
                        }
                    }
                    result
                }
                // Application-level path: resolve the live AXMenuBar from the
                // application element for every hop and invoke without any
                // window focus, raise, or activation. This is the only
                // route for an app that has no windows; the earlier refusal
                // ("window_id does not belong to pid") is not reachable here.
                MenuScope::Application => unsafe { invoke_path(pid, &path) },
            };
            result
        })
        .await;

        match outcome {
            Ok(Ok(())) => ToolResult::text(
                "Resolved the live native menu path and dispatched its final accessibility action; verify the command's semantic effect from fresh state.",
            )
            .with_action_record(
                ActionExecutionRecord::builder(
                    ActionEffect::Unverifiable,
                    ActionTransport::MacosAxAction,
                    RequestedDelivery::Foreground,
                )
                .actual_delivery(ActualDelivery::Foreground)
                .evidence(ActionEvidence {
                    kind: EvidenceKind::NativeApiResult,
                    detail: "Every menu hop resolved uniquely and AX accepted the final action".into(),
                })
                .build()
                .expect("invoke_menu record is valid"),
            ),
            Ok(Err(failure)) => failure_result(failure),
            Err(error) => refusal(format!("invoke_menu: blocking task failed: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_normalization_rejects_empty_segments() {
        assert_eq!(
            normalized_path(vec![" Window ".into(), " Left ".into()]).unwrap(),
            vec!["Window", "Left"]
        );
        assert!(normalized_path(vec!["Window".into(), "  ".into()]).is_err());
    }

    #[test]
    fn action_selection_is_explicit_and_ordered() {
        let actions = vec!["AXShowMenu".into(), "AXPress".into()];
        assert_eq!(choose_action(&actions, false), Some("AXPress"));
        assert_eq!(choose_action(&actions, true), Some("AXPress"));
        assert_eq!(choose_action(&["AXShowMenu".into()], true), None);
    }

    #[test]
    fn a_final_menu_action_that_errors_after_dispatch_is_unverifiable_not_refused() {
        let result = failure_result(hop_failure(1, true, "AXPress", -25204));
        let structured = result.structured_content.expect("structured");
        assert_eq!(structured["effect"], "unverifiable");
        assert_eq!(structured["code"], "ax_action_uncertain");
        assert!(structured.get("status").is_none());
    }

    #[test]
    fn an_intermediate_hop_error_stays_a_refusal() {
        let result = failure_result(hop_failure(0, false, "AXPress", -25204));
        let structured = result.structured_content.expect("structured");
        assert_eq!(structured["status"], "refused");
        assert!(structured.get("effect").is_none());
    }

    #[test]
    fn menu_focus_requires_the_exact_frontmost_app_and_window() {
        assert!(exact_window_is_ready(Some(7), 7, Some(42), 42));
        assert!(!exact_window_is_ready(Some(8), 7, Some(42), 42));
        assert!(!exact_window_is_ready(Some(7), 7, Some(41), 42));
        assert!(!exact_window_is_ready(Some(7), 7, None, 42));
    }

    #[test]
    fn menu_scope_keeps_exact_window_semantics_when_a_window_is_targeted() {
        assert_eq!(
            resolve_menu_scope(Some(42), &[7, 42]),
            Ok(MenuScope::ExactWindow(42))
        );
        assert!(resolve_menu_scope(Some(42), &[7]).is_err());
        assert!(resolve_menu_scope(Some(42), &[]).is_err());
        assert!(resolve_menu_scope(Some(u64::MAX), &[]).is_err());
    }

    #[test]
    fn menu_scope_allows_a_windowless_app_from_the_application_menu_bar() {
        // The windowless fixture: the app owns no windows at all. Omitting
        // the window anchor is the only menu route, and it resolves from the
        // application-level AXMenuBar.
        assert_eq!(resolve_menu_scope(None, &[]), Ok(MenuScope::Application));
        // An omitted anchor is still valid when windows exist: the caller
        // asked for the application-level path, which touches no window.
        assert_eq!(
            resolve_menu_scope(None, &[7, 42]),
            Ok(MenuScope::Application)
        );
    }
}
