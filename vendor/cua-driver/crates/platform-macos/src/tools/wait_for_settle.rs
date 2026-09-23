//! Wait for one application's accessibility surface to go quiet.
//!
//! `wait_for_settle` is the read-only settle observation Synara runs after a
//! mutation: an `AXObserver` bound to the exact pid's application element —
//! or, when `window_id` scopes it, to that exact AX window — reports UI churn,
//! and the call resolves `{settled: true}` as soon as no notification has
//! arrived for `quiet_ms`, bounded by `timeout_ms`.
//!
//! It performs no input, holds no mutation lease, and its observer lives only
//! inside this one call: teardown removes the run-loop source and the
//! registered notifications and releases every CF object, so a retired driver
//! generation keeps nothing. The wait also bails early once the input gate is
//! stopped, but `cancel_input` never waits on this tool — it registers no
//! input operation for the gate to drain.

use async_trait::async_trait;
use core_foundation::{
    base::{CFRelease, CFTypeRef, TCFType},
    runloop::{
        kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopAddSource, CFRunLoopRemoveSource,
        CFRunLoopSourceRef,
    },
    string::{CFString, CFStringRef},
};
use cua_driver_core::{
    protocol::ToolResult,
    tool::{Tool, ToolDef},
    tool_args::ArgsExt,
};
use serde_json::{json, Value};
use std::os::raw::c_void;
use std::time::{Duration, Instant};

use crate::ax::bindings::{
    ax_get_window_id, copy_ax_windows, kAXErrorSuccess, AXObserverAddNotification,
    AXObserverCreate, AXObserverGetRunLoopSource, AXObserverRef, AXObserverRemoveNotification,
    AXUIElementCreateApplication, AXUIElementRef, AXUIElementSetMessagingTimeout,
};
use crate::windows::WindowOwner;

pub struct WaitForSettleTool;

static DEF: std::sync::OnceLock<ToolDef> = std::sync::OnceLock::new();

fn def() -> &'static ToolDef {
    DEF.get_or_init(|| ToolDef {
        name: "wait_for_settle".into(),
        description: "Watch one application's accessibility surface until it stops changing: \
             resolves {settled:true} once no AX notification has arrived for quiet_ms, or \
             {settled:false} with the observed event count at timeout_ms. Read-only — sends \
             no input and takes no mutation lease. window_id scopes the watch to that exact \
             window's AX subtree."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["pid"],
            "properties": {
                "session": { "type": "string", "description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session." },
                "pid": { "type": "integer", "minimum": 1 },
                "window_id": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Exact CGWindowID scoping the watch to that window's accessibility subtree. Omitted, the whole application element is observed."
                },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Hard bound on the wait in milliseconds; defaults to 5000, capped at 30000."
                },
                "quiet_ms": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "How long the surface must stay silent to count as settled, in milliseconds; defaults to 1000, capped at 5000."
                }
            },
            "additionalProperties": false
        }),
        read_only: true,
        destructive: false,
        idempotent: true,
        open_world: false,
    })
}

/// Notifications the settle watch subscribes to, by their AX string names.
/// Names the target does not post are declined by `AXObserverAddNotification`
/// and skipped rather than treated as fatal.
const SETTLE_NOTIFICATIONS: &[&str] = &[
    "AXUIElementDestroyed",
    "AXCreated",
    "AXValueChanged",
    "AXLayoutChanged",
    "AXFocusedUIElementChanged",
    "AXElementBusyChanged",
];

/// Run-loop slices never exceed this: each wake re-checks the deadline, the
/// quiet window, and whether the driver generation has been retired.
const SETTLE_POLL_SLICE: Duration = Duration::from_millis(50);

/// Counter shared with the observer callback. The callback only ever runs on
/// this call's own run loop — the same thread — so the raw refcon pointer is
/// sound for the call's duration.
#[derive(Default)]
struct SettleWatch {
    events_seen: u64,
    last_event: Option<Instant>,
}

unsafe extern "C" fn settle_notification(
    _observer: AXObserverRef,
    _element: AXUIElementRef,
    _notification: CFStringRef,
    refcon: *mut c_void,
) {
    if let Some(watch) = (refcon as *mut SettleWatch).as_mut() {
        watch.events_seen += 1;
        watch.last_event = Some(Instant::now());
    }
}

/// `CFRelease` on drop for one retained AX element or observer.
struct ReleasedCf(CFTypeRef);
impl Drop for ReleasedCf {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

/// `CFRelease` on drop for every element in a retained list.
struct ReleasedElements(Vec<AXUIElementRef>);
impl Drop for ReleasedElements {
    fn drop(&mut self) {
        for element in self.0.drain(..) {
            unsafe { CFRelease(element as CFTypeRef) }
        }
    }
}

#[derive(Debug)]
struct SettleOutcome {
    settled: bool,
    cancelled: bool,
    events_seen: u64,
    waited_ms: u64,
    subscribed: Vec<&'static str>,
    declined: Vec<(&'static str, i32)>,
}

/// Resolve the element notifications are registered on: the exact AX window
/// matching `window_id` (ownership-proven through WindowServer first, exactly
/// like the window mutation tools), else the application element itself. The
/// returned element is borrowed — the guards own it and outlive the borrow.
fn watch_element(
    app: AXUIElementRef,
    pid: i32,
    window_id: Option<u32>,
) -> Result<(AXUIElementRef, Option<ReleasedElements>), String> {
    let Some(window_id) = window_id else {
        return Ok((app, None));
    };
    match crate::windows::resolve_window_owner(pid, window_id) {
        WindowOwner::SamePid => {}
        WindowOwner::ForeignPid {
            owner_pid,
            owner_app_name,
        } => {
            return Err(format!(
                "window_id {window_id} belongs to pid {owner_pid} ({owner_app_name}), not pid {pid}"
            ));
        }
        WindowOwner::Unknown => {
            return Err(format!(
                "window_id {window_id} is closed, stale, or unknown to WindowServer"
            ));
        }
    }
    unsafe {
        let windows = ReleasedElements(copy_ax_windows(app));
        let window = windows
            .0
            .iter()
            .copied()
            .find(|element| ax_get_window_id(*element) == Some(window_id));
        match window {
            Some(element) => Ok((element, Some(windows))),
            None => Err(format!(
                "window_id {window_id} belongs to pid {pid} in WindowServer but has no matching AXWindow"
            )),
        }
    }
}

fn watch_for_settle(
    pid: i32,
    window_id: Option<u32>,
    timeout: Duration,
    quiet: Duration,
) -> Result<SettleOutcome, String> {
    // A read must not start a fresh wait on a generation cleanup has already
    // retired; the same flag ends an in-flight wait early below.
    if crate::input::cancellation::gate().is_stopped() {
        return Err("the driver input generation is retired".to_owned());
    }

    unsafe {
        let app = ReleasedCf({
            let element = AXUIElementCreateApplication(pid);
            if element.is_null() {
                return Err(format!(
                    "could not create an accessibility element for pid {pid}"
                ));
            }
            element as CFTypeRef
        });
        AXUIElementSetMessagingTimeout(app.0 as AXUIElementRef, 2.0);
        let (element, _scope_guard) = watch_element(app.0 as AXUIElementRef, pid, window_id)?;

        let mut raw_observer: AXObserverRef = std::ptr::null_mut();
        let error = AXObserverCreate(pid, settle_notification, &mut raw_observer);
        if error != kAXErrorSuccess || raw_observer.is_null() {
            return Err(format!(
                "AXObserverCreate failed for pid {pid} (AXError {error})"
            ));
        }
        let observer = ReleasedCf(raw_observer as CFTypeRef);
        let source: CFRunLoopSourceRef = AXObserverGetRunLoopSource(observer.0 as AXObserverRef);
        if source.is_null() {
            return Err("AXObserver produced no run-loop source".to_owned());
        }
        // The blocking worker this call runs on owns a lazily-created run loop;
        // the source is added for the wait and removed before returning, so a
        // pooled thread keeps nothing between calls.
        let run_loop = CFRunLoop::get_current();
        CFRunLoopAddSource(
            run_loop.as_concrete_TypeRef(),
            source,
            kCFRunLoopDefaultMode,
        );

        let mut watch = SettleWatch::default();
        let watch_ptr = &mut watch as *mut SettleWatch as *mut c_void;
        let mut subscribed: Vec<&'static str> = Vec::new();
        let mut declined: Vec<(&'static str, i32)> = Vec::new();
        for name in SETTLE_NOTIFICATIONS {
            let notification = CFString::new(name);
            let error = AXObserverAddNotification(
                observer.0 as AXObserverRef,
                element,
                notification.as_concrete_TypeRef(),
                watch_ptr,
            );
            if error == kAXErrorSuccess {
                subscribed.push(*name);
            } else {
                declined.push((*name, error));
            }
        }

        let outcome = if subscribed.is_empty() {
            Err(format!(
                "pid {pid} declined every AX notification ({declined:?}); there is nothing to watch"
            ))
        } else {
            // The quiet window measures from the last observed notification —
            // or from arming time when nothing has arrived yet. Deadline and
            // the retired-generation check bound every loop turn.
            let armed = Instant::now();
            let deadline = armed + timeout;
            let mut settled = false;
            let mut cancelled = false;
            loop {
                let now = Instant::now();
                let quiet_elapsed = now.duration_since(watch.last_event.unwrap_or(armed));
                if quiet_elapsed >= quiet {
                    settled = true;
                    break;
                }
                if now >= deadline {
                    break;
                }
                if crate::input::cancellation::gate().is_stopped() {
                    cancelled = true;
                    break;
                }
                let slice = (quiet - quiet_elapsed)
                    .min(deadline - now)
                    .min(SETTLE_POLL_SLICE);
                CFRunLoop::run_in_mode(kCFRunLoopDefaultMode, slice, true);
            }
            Ok(SettleOutcome {
                settled,
                cancelled,
                events_seen: watch.events_seen,
                waited_ms: armed.elapsed().as_millis() as u64,
                subscribed: subscribed.clone(),
                declined: declined.clone(),
            })
        };

        // Teardown on every path: subscriptions come off the element, the
        // source leaves the run loop, and the guards release the CF objects.
        for name in &subscribed {
            let notification = CFString::new(name);
            AXObserverRemoveNotification(
                observer.0 as AXObserverRef,
                element,
                notification.as_concrete_TypeRef(),
            );
        }
        CFRunLoopRemoveSource(
            run_loop.as_concrete_TypeRef(),
            source,
            kCFRunLoopDefaultMode,
        );
        outcome
    }
}

/// The validated call shape, separated from `invoke` so the clamps and the
/// per-field refusals can be tested without standing up an `AXObserver`.
struct SettleArgs {
    pid: i32,
    window_id: Option<u32>,
    timeout_ms: u64,
    quiet_ms: u64,
}

fn parse_settle_args(args: &Value) -> Result<SettleArgs, ToolResult> {
    let pid = match args.require_i32("pid") {
        Ok(v) if v > 0 => v,
        Ok(_) => {
            return Err(ToolResult::error(
                "wait_for_settle: pid must be a positive integer",
            ))
        }
        Err(e) => return Err(e),
    };
    let window_id = match args.get("window_id") {
        None => None,
        Some(value) => match value
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0)
        {
            Some(v) => Some(v),
            None => {
                return Err(ToolResult::error(
                    "wait_for_settle: window_id must be a positive integer",
                ))
            }
        },
    };
    Ok(SettleArgs {
        pid,
        window_id,
        timeout_ms: args.opt_u64("timeout_ms").unwrap_or(5_000).min(30_000),
        quiet_ms: args.opt_u64("quiet_ms").unwrap_or(1_000).min(5_000),
    })
}

#[async_trait]
impl Tool for WaitForSettleTool {
    fn def(&self) -> &ToolDef {
        def()
    }

    async fn invoke(&self, args: Value) -> ToolResult {
        let parsed = match parse_settle_args(&args) {
            Ok(parsed) => parsed,
            Err(result) => return result,
        };
        let pid = parsed.pid;
        let window_id = parsed.window_id;
        let timeout_ms = parsed.timeout_ms;
        let quiet_ms = parsed.quiet_ms;
        let outcome = match tokio::task::spawn_blocking(move || {
            watch_for_settle(
                pid,
                window_id,
                Duration::from_millis(timeout_ms),
                Duration::from_millis(quiet_ms),
            )
        })
        .await
        {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(error)) => return ToolResult::error(format!("wait_for_settle: {error}")),
            Err(error) => {
                return ToolResult::error(format!("wait_for_settle: blocking task failed: {error}"))
            }
        };
        let mut structured = json!({
            "settled": outcome.settled,
            "events_seen": outcome.events_seen,
            "waited_ms": outcome.waited_ms,
            "timeout_ms": timeout_ms,
            "quiet_ms": quiet_ms,
            "pid": pid,
            "scope": if window_id.is_some() { "window" } else { "application" },
            "subscribed_notifications": outcome.subscribed,
        });
        if let Some(window_id) = window_id {
            structured["window_id"] = json!(window_id);
        }
        if outcome.cancelled {
            structured["cancelled"] = json!(true);
        }
        if !outcome.declined.is_empty() {
            structured["declined_notifications"] = json!(outcome
                .declined
                .iter()
                .map(|(name, error)| json!({ "name": name, "ax_error": error }))
                .collect::<Vec<_>>());
        }
        ToolResult::text(if outcome.cancelled {
            "Settle wait ended early: the driver input generation was retired.".to_owned()
        } else if outcome.settled {
            format!(
                "UI settled after {} ms ({} notification{} observed).",
                outcome.waited_ms,
                outcome.events_seen,
                if outcome.events_seen == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "UI did not settle within {} ms ({} notification{} observed).",
                timeout_ms,
                outcome.events_seen,
                if outcome.events_seen == 1 { "" } else { "s" }
            )
        })
        .with_structured(structured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(args: Value) -> Result<SettleArgs, ToolResult> {
        parse_settle_args(&args)
    }

    #[test]
    fn pid_only_defaults_the_bounds() {
        let parsed = parse(json!({ "pid": 1234 })).unwrap();
        assert_eq!(parsed.pid, 1234);
        assert_eq!(parsed.window_id, None);
        assert_eq!(parsed.timeout_ms, 5_000);
        assert_eq!(parsed.quiet_ms, 1_000);
    }

    #[test]
    fn bounds_clamp_to_the_driver_caps() {
        let parsed = parse(json!({
            "pid": 7,
            "window_id": 42,
            "timeout_ms": 999_999,
            "quiet_ms": 999_999,
        }))
        .unwrap();
        assert_eq!(parsed.window_id, Some(42));
        assert_eq!(parsed.timeout_ms, 30_000);
        assert_eq!(parsed.quiet_ms, 5_000);
    }

    #[test]
    fn missing_or_nonpositive_pid_is_refused() {
        assert!(parse(json!({})).is_err());
        assert!(parse(json!({ "pid": 0 })).is_err());
        assert!(parse(json!({ "pid": -3 })).is_err());
        assert!(parse(json!({ "pid": "abc" })).is_err());
    }

    #[test]
    fn nonpositive_window_id_is_refused_but_absent_is_application_scope() {
        assert!(parse(json!({ "pid": 1, "window_id": 0 })).is_err());
        assert!(parse(json!({ "pid": 1, "window_id": -2 })).is_err());
        assert!(parse(json!({ "pid": 1, "window_id": "20" })).is_err());
        assert_eq!(parse(json!({ "pid": 1 })).unwrap().window_id, None);
    }

    #[test]
    fn the_watch_covers_every_settle_notification_name() {
        // The six names the contract advertises; additions must be deliberate.
        assert_eq!(
            SETTLE_NOTIFICATIONS,
            &[
                "AXUIElementDestroyed",
                "AXCreated",
                "AXValueChanged",
                "AXLayoutChanged",
                "AXFocusedUIElementChanged",
                "AXElementBusyChanged",
            ]
        );
    }

    #[test]
    fn the_tool_is_registered_read_only() {
        let def = WaitForSettleTool.def();
        assert!(def.read_only);
        assert!(!def.destructive);
        assert!(def.idempotent);
        assert_eq!(def.name, "wait_for_settle");
    }
}
