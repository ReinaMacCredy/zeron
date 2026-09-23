use async_trait::async_trait;
use core_foundation::base::{CFRelease, CFTypeRef};
use cua_driver_contract::{ScrollBy, ScrollDirection, ScrollInput};
use cua_driver_core::{
    protocol::ToolResult,
    tool::{Tool, ToolDef},
    tool_args::parse_typed_projection,
};
use serde_json::Value;
use std::sync::Arc;

use crate::apps;
use crate::ax::bindings::{
    copy_action_names, copy_children, copy_element_attr, copy_string_attr, element_screen_center,
    AXUIElementRef,
};
use crate::focus_guard;
use crate::window_change_detector::WindowChangeDetector;

use super::ToolState;

/// Per-notch pixel step for the wheel path. `page` rolls a screenful-ish chunk,
/// `line` a few text lines — tuned to feel like a real wheel notch. Runtime
/// tuning deferred (host is screen-recording); centralized here for one-line edits.
const WHEEL_STEP_LINE_PX: i32 = 120;
const WHEEL_STEP_PAGE_PX: i32 = 600;

/// Resolved pixel-wheel target in screen space, plus optional window-local
/// stamp + window id for backgrounded delivery.
struct WheelTarget {
    screen_x: f64,
    screen_y: f64,
    win_local: Option<(f64, f64)>,
    wid: Option<u32>,
}

fn after_exact_target_gate<T>(
    gate: Result<(), ToolResult>,
    action: impl FnOnce() -> T,
) -> Result<T, ToolResult> {
    gate?;
    Ok(action())
}

pub struct ScrollTool {
    state: Arc<ToolState>,
}

impl ScrollTool {
    pub fn new(state: Arc<ToolState>) -> Self {
        Self { state }
    }
}

fn wheel_failure_result(code: &'static str, foreground: bool) -> ToolResult {
    ToolResult::error(
        "Wheel dispatch failed; its effect is unknown. Observe fresh state; do not replay.",
    )
    .with_structured(serde_json::json!({
        "code": "cua_action_failed", "effect": "unverifiable", "verified": false,
        "diagnostics": { "delivery_path": "pixel", "actuator": "cgevent_wheel",
            "focus_mutation": if foreground { "foreground" } else { "none" },
            "restore_status": if foreground { "unobservable" } else { "not-needed" },
            "error_code": code }
    }))
}

static DEF: std::sync::OnceLock<ToolDef> = std::sync::OnceLock::new();

fn def() -> &'static ToolDef {
    DEF.get_or_init(|| ToolDef {
        name: "scroll".into(),
        description: "Scroll the target pid. Two paths, picked by how you address the scroll:\n\n\
            • **Targeted wheel path** — when you pass a target, either \
            `element_index`/`element_token` (preferred) or window-local `x, y` pixels: \
            the driver synthesizes a real mouse-wheel event (CGEventCreateScrollWheelEvent, \
            at that screen point. The renderer hit-tests the wheel at the \
            cursor, so the scroll lands on whatever element is under the point — exactly \
            like physically rolling the wheel over it. This is the ONLY way to scroll a \
            nested `overflow:auto` region (e.g. a scrollable <div> with no tabindex): such \
            regions never take keyboard focus, so the keystroke path below no-ops on them. \
            Use this for inner/nested scrollers in web views.\n\n\
            • **Keystroke path (focused region)** — when you pass NO target (just pid + \
            direction): synthesizes PageDown/PageUp (by='page') or Down/Up arrows \
            (by='line'); horizontal uses Left/Right arrows. Drives the focused / page \
            scroller only.\n\n\
            Mapping: by='page' → larger step; by='line' → smaller step; amount = number of \
            wheel notches (targeted path) or keystroke repetitions (keystroke path).".into(),
        input_schema: serde_json::json!({
            "type": "object",
            // `pid` conditionally required (validated in code), not pinned in the
            // schema — keeps the contract consistent across platforms.
            "required": ["direction"],
            "properties": {
                "expected_window_bounds": {"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"},"width":{"type":"number"},"height":{"type":"number"}},"required":["x","y","width","height"]},
                "coordinate_space": {"type":"string","enum":["screenshot_pixels","window_points"],"description":"Internal logical window points require expected_window_bounds and exact pid/window_id; default screenshot pixels."},
                "session": { "type": "string", "description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session." },
                "pid": { "type": "integer" },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"],
                    "description": "Scroll direction."
                },
                "by": {
                    "type": "string",
                    "enum": ["line", "page"],
                    "description": "Scroll granularity. Default: line."
                },
                "amount": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Pixel-wheel path: number of wheel notches. Keystroke path: number of keystroke repetitions. Default: 3."
                },
                "delta_x": {
                    "type": "integer",
                    "minimum": -50,
                    "maximum": 50,
                    "description": "Signed horizontal wheel ticks (−50..50). With `delta_y` delivers one two-axis pixel-wheel gesture and overrides `direction`; requires an element or x,y target."
                },
                "delta_y": {
                    "type": "integer",
                    "minimum": -50,
                    "maximum": 50,
                    "description": "Signed vertical wheel ticks (−50..50). See `delta_x`."
                },
                "modifiers": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Modifier keys held during the wheel gesture: cmd, shift, option/alt, ctrl."
                },
                "window_id": { "type": "integer" },
                "element_index": cua_driver_core::tool_schema::element_index_schema(),
                "element_token": cua_driver_core::tool_schema::element_token_schema(),
                "snapshot_id": cua_driver_core::tool_schema::snapshot_id_schema(),
                "x": { "type": "number", "description": "Window-local screenshot X (top-left origin of the PNG from get_window_state). With `y`, routes through the pixel-wheel path at this point — use for a scrollable surface that isn't in the AX tree. Requires window_id to anchor the window→screen conversion." },
                "y": { "type": "number", "description": "Window-local screenshot Y. See `x`." },
                "scope": { "type": "string", "enum": ["window", "desktop"], "default": "window", "description": "Use desktop with x,y and no pid/window_id for native get_desktop_state screenshot coordinates." },
                "delivery_mode": cua_driver_core::tool_schema::delivery_mode_schema()
            },
            "additionalProperties": false
        }),
        read_only: false,
        destructive: false,
        idempotent: false,
        open_world: true,
    })
}

#[async_trait]
impl Tool for ScrollTool {
    fn def(&self) -> &ToolDef {
        def()
    }

    async fn invoke(&self, args: Value) -> ToolResult {
        use cua_driver_core::tool_args::ArgsExt;
        if args.opt_str("scope").as_deref() == Some("desktop")
            && args.get("pid").is_none()
            && args.get("window_id").is_none()
        {
            let input = match parse_typed_projection::<ScrollInput>("scroll", &args) {
                Ok(input) => input,
                Err(result) => return result,
            };
            let (x, y) = (input.x, input.y);
            let direction = input.direction.as_str();
            let by = input.by.unwrap_or(ScrollBy::Line).as_str();
            let amount = input.amount.unwrap_or(3).clamp(1, 50) as usize;
            let step = if input.by == Some(ScrollBy::Page) {
                WHEEL_STEP_PAGE_PX
            } else {
                WHEEL_STEP_LINE_PX
            };
            let (delta_y, delta_x) = match input.direction {
                ScrollDirection::Down => (-step, 0),
                ScrollDirection::Up => (step, 0),
                ScrollDirection::Right => (0, -step),
                ScrollDirection::Left => (0, step),
            };
            let (x, y) = super::desktop_screenshot_point(x, y).await;
            let result = tokio::task::spawn_blocking(move || {
                crate::input::mouse::scroll_wheel_desktop(x, y, delta_y, delta_x, amount)
            })
            .await;
            return match result {
                Ok(Ok(())) => ToolResult::text(format!(
                    "Scrolled desktop {direction} by {by} × {amount} at ({x:.1}, {y:.1})."
                ))
                .with_structured(serde_json::json!({
                    "scope": "desktop",
                    "path": "hid",
                    "effect": "unverifiable"
                })),
                Ok(Err(error)) => ToolResult::error(format!("desktop scroll failed: {error}")),
                Err(error) => ToolResult::error(format!("desktop scroll task failed: {error}")),
            };
        }
        let pid = match args.require_i32("pid") {
            Ok(v) => v,
            Err(e) => return e,
        };
        // delivery_mode: foreground briefly fronts the window before the
        // pixel-wheel dispatch (the explicit last resort for surfaces that drop
        // background CGEvents). Only the pixel-wheel path honors it; the
        // keystroke path is background-by-design and untouched.
        let delivery_mode = super::DeliveryMode::parse(args.opt_str("delivery_mode").as_deref());
        let direction = match args.require_str("direction") {
            Ok(v) => v,
            Err(e) => return e,
        };
        let by = args.str_or("by", "line");
        let amount = args.u64_or("amount", 3) as usize;
        // Synara: optional signed per-axis wheel ticks plus held modifiers.
        // delta_x/delta_y form one two-axis pixel-wheel gesture at the target;
        // either present switches the wheel path off the direction mapping.
        let delta_x_arg = args.opt_i64("delta_x");
        let delta_y_arg = args.opt_i64("delta_y");
        let has_axis_deltas = delta_x_arg.is_some() || delta_y_arg.is_some();
        let modifiers_arg = args.str_array("modifiers");
        let modifier_refs: Vec<&str> = modifiers_arg.iter().map(String::as_str).collect();
        let scroll_flags = crate::input::mouse::parse_modifier_flags(&modifier_refs);
        // Surface 6: element_token / element_index precedence.
        let element_token_arg = args.opt_str("element_token");
        let window_id_arg = args.opt_u64("window_id");
        let element_index_arg = args.opt_u64("element_index").map(|v| v as usize);
        let resolved = match self.state.element_cache.resolve_element_args(
            pid,
            element_index_arg,
            element_token_arg.as_deref(),
            args.opt_str("snapshot_id").as_deref(),
            window_id_arg,
            "scroll",
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let (_, window_id, pre_focus_guard) = resolved.into_parts(window_id_arg);
        let window_id = match super::native_window_id(window_id) {
            Ok(window_id) => window_id,
            Err(error) => return error,
        };
        let pre_focus_ptr: Option<usize> = pre_focus_guard.as_ref().map(|g| g.as_ptr());

        let mut _mutation_lease: Option<super::BackgroundMutationLease> = None;

        // AppKit exposes vertical scroll-bar buttons beneath the text area's
        // AXScrollArea parent. Pressing those controls is a true
        // background-safe scroll: no activation, z-order change, or cursor move.
        let semantic_axis = semantic_scroll_axis(
            &direction,
            amount,
            delta_x_arg,
            delta_y_arg,
            !modifiers_arg.is_empty(),
        );
        if let Some((semantic_direction, semantic_amount)) = semantic_axis {
            if let (Some(element_guard), Some(wid)) = (pre_focus_guard.clone(), window_id) {
                if !delivery_mode.is_foreground() {
                    if let Some(lease) = _mutation_lease.as_ref() {
                        if let Err(refusal_result) = lease
                            .gate_again(
                                wid,
                                pre_focus_ptr,
                                cua_driver_core::background_input::BackgroundAction::AxSemantic,
                            )
                            .await
                        {
                            return refusal_result;
                        }
                    } else {
                        match super::gate_background_window_action(
                            pid,
                            wid,
                            pre_focus_ptr,
                            cua_driver_core::background_input::BackgroundAction::AxSemantic,
                        )
                        .await
                        {
                            Ok(lease) => _mutation_lease = Some(lease),
                            Err(refusal_result) => return refusal_result,
                        }
                    }
                }
                let direction_for_ax = semantic_direction.to_owned();
                let by_for_ax = by.clone();
                let foreground = delivery_mode.is_foreground();
                let ax_result =
                    tokio::task::spawn_blocking(move || -> anyhow::Result<(bool, bool)> {
                        if foreground {
                            let mut delivered = false;
                            let fronted = crate::input::skylight::with_foreground_assist(
                                pid as libc::pid_t,
                                wid,
                                || {
                                    delivered = unsafe {
                                        scroll_native_text_area(
                                            element_guard.as_ptr() as AXUIElementRef,
                                            &direction_for_ax,
                                            &by_for_ax,
                                            semantic_amount,
                                        )?
                                    };
                                    std::thread::sleep(std::time::Duration::from_millis(100));
                                    Ok(())
                                },
                            )?;
                            Ok((delivered, fronted))
                        } else {
                            Ok((
                                unsafe {
                                    scroll_native_text_area(
                                        element_guard.as_ptr() as AXUIElementRef,
                                        &direction_for_ax,
                                        &by_for_ax,
                                        semantic_amount,
                                    )?
                                },
                                false,
                            ))
                        }
                    })
                    .await;
                match ax_result {
                    Ok(Ok((true, fronted))) => {
                        return ToolResult::text(format!(
                        "Submitted native macOS scroll {semantic_direction} by {by} × {semantic_amount} through AX; inspect the returned state to verify movement."
                    ))
                    .with_structured(serde_json::json!({
                        "path": if fronted { "ax_fg" } else { "ax" },
                        "verified": false,
                        "effect": "unverifiable"
                    }));
                    }
                    Ok(Ok((false, _))) => {}
                    Ok(Err(error)) => {
                        return crate::input::ax_actions::action_failure_result(
                            error,
                            "Native AX scroll failed",
                        );
                    }
                    Err(error) => {
                        return crate::input::ax_actions::action_failure_result(
                            error.into(),
                            "Native AX scroll task failed",
                        );
                    }
                }
            }
        }

        // ── Targeted wheel path ─────────────────────────────────────────────
        // A target — element (preferred) OR window-local x,y — routes the scroll
        // through a synthesized mouse-wheel event at that screen point, so the
        // renderer's hit-test delivers it to whatever element is under the
        // cursor. This is the ONLY way to scroll a nested overflow:auto region
        // that never takes keyboard focus (the keystroke path below no-ops on
        // it). No user-facing flag: presence of a target IS the switch.
        let x_arg = args
            .opt_f64("x")
            .or_else(|| args.opt_i64("x").map(|v| v as f64));
        let y_arg = args
            .opt_f64("y")
            .or_else(|| args.opt_i64("y").map(|v| v as f64));

        // Per-notch step + direction→delta mapping (sign convention lives
        // here; the mouse primitive stays sign-agnostic). macOS: +y reveals
        // content ABOVE, -y reveals BELOW; +x reveals LEFT, -x reveals RIGHT.
        let step = if by == "page" {
            WHEEL_STEP_PAGE_PX
        } else {
            WHEEL_STEP_LINE_PX
        };
        // Signed-tick mode: both axes ride one gesture as pixel-unit wheel
        // deltas sized so each axis completes inside `ticks` events. Line
        // units cannot express a fractional per-tick minor axis, and pixel
        // units are what a trackpad posts — nested scrollers honor them.
        let mut ticks = amount;
        let mut pixel_units = false;
        let mut axis_ticks: Option<(i32, i32)> = None;
        let (delta_y, delta_x): (i32, i32) = if has_axis_deltas {
            let ty = delta_y_arg.unwrap_or(0).clamp(-50, 50) as i32;
            let tx = delta_x_arg.unwrap_or(0).clamp(-50, 50) as i32;
            if ty == 0 && tx == 0 {
                return ToolResult::error(
                    "scroll: delta_x/delta_y supplied but both resolve to zero ticks.".to_string(),
                )
                .with_structured(serde_json::json!({ "code": "invalid_arguments" }));
            }
            ticks = ty.abs().max(tx.abs()) as usize;
            pixel_units = true;
            axis_ticks = Some((tx, ty));
            (
                ty * WHEEL_STEP_LINE_PX / ticks as i32,
                tx * WHEEL_STEP_LINE_PX / ticks as i32,
            )
        } else {
            match direction.as_str() {
                "down" => (-step, 0),
                "up" => (step, 0),
                "right" => (0, -step),
                "left" => (0, step),
                _ => (-step, 0),
            }
        };

        // Resolve a screen-space wheel target, if a target was supplied.
        let wheel_target: Option<WheelTarget> = if let Some(element_ptr) = pre_focus_ptr {
            // Revealing an element is itself an AX mutation. Prove that the
            // cached element still belongs to the exact requested window before
            // AXScrollToVisible for every direction, then keep the lease for the
            // stricter pointer revalidation below.
            let semantic_gate = if !delivery_mode.is_foreground() {
                if let Some(wid) = window_id {
                    if let Some(lease) = _mutation_lease.as_ref() {
                        lease
                            .gate_again(
                                wid,
                                Some(element_ptr),
                                cua_driver_core::background_input::BackgroundAction::AxSemantic,
                            )
                            .await
                    } else {
                        match super::gate_background_window_action(
                            pid,
                            wid,
                            Some(element_ptr),
                            cua_driver_core::background_input::BackgroundAction::AxSemantic,
                        )
                        .await
                        {
                            Ok(lease) => {
                                _mutation_lease = Some(lease);
                                Ok(())
                            }
                            Err(refusal) => Err(refusal),
                        }
                    }
                } else {
                    Ok(())
                }
            } else {
                Ok(())
            };
            // Element path: wheel at the element's screen-space center. Both AX
            // coordinates and window bounds are logical top-left points, so no
            // Retina scaling is needed here.
            let wid = window_id;
            let target_guard = pre_focus_guard.clone();
            let target_task = tokio::task::spawn_blocking(move || {
                let _target_guard = target_guard;
                // Web content can be present in AX while its frame is below
                // the outer page viewport. Ask the accessibility hierarchy to
                // reveal the target before taking the screen-space center;
                // otherwise the wheel is posted outside the rendered window
                // and nested overflow regions never receive it.
                after_exact_target_gate(semantic_gate, || unsafe {
                    crate::ax::bindings::perform_action(
                        element_ptr as AXUIElementRef,
                        "AXScrollToVisible",
                    )
                })?;
                std::thread::sleep(std::time::Duration::from_millis(40));
                let center = unsafe { element_screen_center(element_ptr as AXUIElementRef) };
                Ok(center.map(|(cx, cy)| {
                    let win_local = wid
                        .and_then(crate::windows::window_bounds_by_id)
                        .map(|b| (cx - b.x, cy - b.y));
                    WheelTarget {
                        screen_x: cx,
                        screen_y: cy,
                        win_local,
                        wid,
                    }
                }))
            });
            match target_task.await {
                Ok(Ok(target)) => target,
                Ok(Err(refusal)) => return refusal,
                Err(_) => None,
            }
        } else if let (Some(mut cx), Some(mut cy)) = (x_arg, y_arg) {
            // Targeted x,y are window-local screenshot pixels and REQUIRE a
            // window_id to anchor the window→screen conversion (schema contract).
            // Without one, refuse rather than scrolling at screen-absolute coords.
            if window_id.is_none() {
                return ToolResult::error(
                    "window_id is required when scrolling by window-local x,y pixels.".to_string(),
                );
            }
            // Pixel path: x,y are window-local screenshot pixels. Mirror the
            // click pixel path — undo any session downscale, then translate
            // through the shared window frame (which refuses a window with no
            // live frame rather than scrolling at screen-absolute coords).
            if let Some(ratio) = self
                .state
                .resize_registry
                .ratio(pid, window_id)
                .filter(|_| !super::px_frame::is_window_points(&args))
            {
                cx *= ratio;
                cy *= ratio;
            }
            let Some(wid) = window_id else {
                // Unreachable: the None case refused above. Kept explicit so a
                // future edit cannot reintroduce the screen-absolute fallback.
                return ToolResult::error(
                    "window_id is required when scrolling by window-local x,y pixels.".to_string(),
                );
            };
            match super::px_frame::resolve_input_frame(wid, &args).await {
                Ok(frame) => {
                    let (sx, sy, lx, ly) = frame.to_screen(cx, cy);
                    Some(WheelTarget {
                        screen_x: sx,
                        screen_y: sy,
                        win_local: Some((lx, ly)),
                        wid: Some(wid),
                    })
                }
                Err(refusal) => return refusal,
            }
        } else {
            None
        };

        if let Some(target) = wheel_target {
            if !delivery_mode.is_foreground() {
                if let Some(wid) = target.wid {
                    if let (Some((lx, ly)), Some(bounds)) =
                        (target.win_local, crate::windows::window_bounds_by_id(wid))
                    {
                        if lx < 0.0 || ly < 0.0 || lx > bounds.width || ly > bounds.height {
                            return ToolResult::error(format!(
                                "scroll: window-local point ({lx:.1}, {ly:.1}) pt lies outside \
                                 window {wid}'s {:.0}×{:.0} pt frame; background delivery \
                                 refused",
                                bounds.width, bounds.height
                            ));
                        }
                    }
                    // The semantic reveal gate does not authorize pointer
                    // delivery. Revalidate the stricter route immediately
                    // before proceeding to wheel dispatch.
                    if let Some(lease) = _mutation_lease.as_ref() {
                        if let Err(refusal_result) = lease
                            .gate_again(
                                wid,
                                pre_focus_ptr,
                                cua_driver_core::background_input::BackgroundAction::WindowPointer,
                            )
                            .await
                        {
                            return refusal_result;
                        }
                    } else {
                        match super::gate_background_window_action(
                            pid,
                            wid,
                            pre_focus_ptr,
                            cua_driver_core::background_input::BackgroundAction::WindowPointer,
                        )
                        .await
                        {
                            Ok(lease) => _mutation_lease = Some(lease),
                            Err(refusal_result) => return refusal_result,
                        }
                    }
                }
            }
            let cursor_key = super::cursor_tools::resolve_cursor_key(&args);
            // Pin + glide the agent-cursor overlay to the target for visibility
            // (overlay only — does NOT move the hardware cursor). Mirrors click.
            if let Some(wid) = target.wid {
                crate::cursor::overlay::send_command(
                    cursor_key.clone(),
                    cursor_overlay::OverlayCommand::PinAbove(wid as u64),
                );
            }
            crate::cursor::overlay::animate_cursor_to(
                cursor_key.clone(),
                target.screen_x,
                target.screen_y,
            )
            .await;
            self.state.cursor_registry.update_position(
                &cursor_key,
                target.screen_x,
                target.screen_y,
            );

            let prior_front = apps::frontmost_pid();
            let snapshot = WindowChangeDetector::snapshot(prior_front);

            let WheelTarget {
                screen_x,
                screen_y,
                win_local,
                wid,
            } = target;
            let fg = delivery_mode.is_foreground() && wid.is_some();
            let result = focus_guard::with_focus_suppressed(
                Some(pid),
                prior_front,
                "scroll.CGScrollWheel",
                || async move {
                    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                        let do_it = move || -> anyhow::Result<()> {
                            crate::input::mouse::scroll_wheel_at_xy(
                                pid,
                                screen_x,
                                screen_y,
                                win_local,
                                wid,
                                delta_y,
                                delta_x,
                                ticks,
                                scroll_flags,
                                pixel_units,
                            )
                        };
                        // Foreground rung: brief front → wheel → restore prior frontmost.
                        match (fg, wid) {
                            (true, Some(w)) => {
                                crate::input::skylight::with_foreground_assist(
                                    pid as libc::pid_t,
                                    w,
                                    do_it,
                                )?;
                                Ok(())
                            }
                            _ => do_it(),
                        }
                    })
                    .await
                },
            )
            .await;

            let changes = super::finish_window_observation(snapshot, &args).await;
            let mode_label = if fg {
                " (delivery_mode:foreground)"
            } else {
                ""
            };
            return match result {
                Ok(Ok(())) => {
                    let describe = match axis_ticks {
                        Some((tx, ty)) => {
                            format!("scroll delta_x={tx} delta_y={ty} × {ticks} ticks")
                        }
                        None => format!("{direction} scroll by {by} × {amount}"),
                    };
                    ToolResult::text(format!(
                        "✅ Sent {describe} via pixel wheel at \
                         ({screen_x:.0}, {screen_y:.0}){mode_label} (background CGEvent; not \
                         driver-verified — confirm via screenshot).{}",
                        changes.result_suffix()
                    ))
                    .with_structured(serde_json::json!({
                        "path": if fg { "cgevent_fg" } else { "cgevent" }, "verified": false, "effect": "unverifiable",
                        "diagnostics": { "delivery_path": "pixel", "actuator": "cgevent_wheel",
                            "focus_mutation": if fg { "foreground" } else { "none" },
                            "restore_status": if fg { "unobservable" } else { "not-needed" } }
                    }))
                }
                Ok(Err(_)) => wheel_failure_result("pixel_dispatch_failed", fg),
                Err(_) => wheel_failure_result("action_task_failed", fg),
            };
        }

        if has_axis_deltas || scroll_flags != core_graphics::event::CGEventFlags::CGEventFlagNull {
            return ToolResult::error(
                "scroll: delta_x/delta_y/modifiers require an element or window-local x,y \
                 target (wheel path); the focused keystroke path cannot carry them."
                    .to_string(),
            )
            .with_structured(serde_json::json!({ "code": "invalid_arguments" }));
        }

        let key = match (by.as_str(), direction.as_str()) {
            ("page", "down") | (_, "down") if by == "page" => "pagedown",
            ("page", "up") | (_, "up") if by == "page" => "pageup",
            ("line", "down") | (_, "down") => "down",
            ("line", "up") | (_, "up") => "up",
            (_, "left") => "left",
            (_, "right") => "right",
            _ => "down",
        };
        let key = key.to_owned();

        if !delivery_mode.is_foreground() {
            if let Some(wid) = window_id {
                if let Some(lease) = _mutation_lease.as_ref() {
                    if let Err(refusal_result) = lease
                        .gate_again(
                            wid,
                            pre_focus_ptr,
                            cua_driver_core::background_input::BackgroundAction::GenericKey,
                        )
                        .await
                    {
                        return refusal_result;
                    }
                } else {
                    match super::gate_background_window_action(
                        pid,
                        wid,
                        pre_focus_ptr,
                        cua_driver_core::background_input::BackgroundAction::GenericKey,
                    )
                    .await
                    {
                        Ok(lease) => _mutation_lease = Some(lease),
                        Err(refusal_result) => return refusal_result,
                    }
                }
            }
        }

        // ── Focus-suppression wrap (Swift WindowChangeDetector + FocusGuard) ──
        // Scroll keystrokes (PageDown / arrow) into search-box autocomplete
        // can spawn floating helper windows; rare but real. Wrap for parity
        // with the other action tools.
        //
        // The AX focus_element() pre-write also runs inside the closure so
        // any reflex activations it triggers are caught by both the wildcard
        // snapshot suppressor and the targeted FocusGuard lease.
        let prior_front = apps::frontmost_pid();
        let snapshot = WindowChangeDetector::snapshot(prior_front);

        let result = focus_guard::with_focus_suppressed(
            Some(pid),
            prior_front,
            "scroll.CGEvent",
            || async move {
                // Pre-focus the element under suppression so its
                // side-effects are captured by the snapshot + lease.
                if let Some(guard) = pre_focus_guard {
                    let _ = tokio::task::spawn_blocking(move || {
                        crate::input::ax_actions::focus_element(guard.as_ptr())
                    })
                    .await;
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                }

                tokio::task::spawn_blocking(move || {
                    for _ in 0..amount {
                        crate::input::keyboard::press_key(pid, &key, &[])?;
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Ok::<(), anyhow::Error>(())
                })
                .await
            },
        )
        .await;

        let changes = super::finish_window_observation(snapshot, &args).await;

        match result {
            Ok(Ok(())) => ToolResult::text(format!(
                "✅ Sent {direction} scroll by {by} × {amount} via keystroke \
                 (background; not driver-verified — confirm via screenshot).{}",
                changes.result_suffix()
            ))
            .with_structured(serde_json::json!({ "path": "key_events", "verified": false })),
            Ok(Err(e)) => ToolResult::error(format!("Scroll failed: {e}")),
            Err(e) => ToolResult::error(format!("Task error: {e}")),
        }
    }
}

fn semantic_scroll_axis<'a>(
    direction: &'a str,
    amount: usize,
    dx: Option<i64>,
    dy: Option<i64>,
    has_modifiers: bool,
) -> Option<(&'a str, usize)> {
    if has_modifiers {
        return None;
    }
    if dx.is_none() && dy.is_none() {
        return Some((direction, amount));
    }
    match (
        dx.unwrap_or(0).clamp(-50, 50),
        dy.unwrap_or(0).clamp(-50, 50),
    ) {
        (0, y) if y != 0 => Some((if y < 0 { "down" } else { "up" }, y.unsigned_abs() as usize)),
        (x, 0) if x != 0 => Some((
            if x < 0 { "right" } else { "left" },
            x.unsigned_abs() as usize,
        )),
        _ => None,
    }
}

/// Read capabilities first; one submitted AX action never falls back to wheels.
/// Ancestor ascent stops at the nearest scrollbar, preserving nested targets.
unsafe fn scroll_advertised_action(
    element: AXUIElementRef,
    direction: &str,
    by: &str,
    amount: usize,
) -> anyhow::Result<bool> {
    let (attribute, action, page_action) = match direction {
        "down" => ("AXVerticalScrollBar", "AXIncrement", "AXScrollDownByPage"),
        "up" => ("AXVerticalScrollBar", "AXDecrement", "AXScrollUpByPage"),
        "right" => (
            "AXHorizontalScrollBar",
            "AXIncrement",
            "AXScrollRightByPage",
        ),
        "left" => ("AXHorizontalScrollBar", "AXDecrement", "AXScrollLeftByPage"),
        _ => return Ok(false),
    };
    let window_id = crate::ax::exact_target::element_window_id(element);
    if window_id.is_none() {
        return Ok(false);
    }
    let mut current = crate::ax::cache::RetainedElement::retain(element as usize);
    for _ in 0..12 {
        let ptr = current.as_ptr() as AXUIElementRef;
        let role = copy_string_attr(ptr, "AXRole");
        if matches!(role.as_deref(), Some("AXWindow" | "AXApplication")) {
            break;
        }
        let candidate = if by == "page"
            && copy_action_names(ptr)
                .iter()
                .any(|name| name == page_action)
        {
            Some((current.clone(), page_action))
        } else if by != "page" {
            if let Some(bar) = copy_element_attr(ptr, attribute) {
                let retained = crate::ax::cache::RetainedElement::retain(bar as usize);
                CFRelease(bar as CFTypeRef);
                if copy_action_names(retained.as_ptr() as AXUIElementRef)
                    .iter()
                    .any(|name| name == action)
                {
                    Some((retained, action))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        if let Some((target, action)) = candidate {
            if crate::ax::exact_target::element_window_id(target.as_ptr() as AXUIElementRef)
                != window_id
            {
                return Ok(false);
            }
            for _ in 0..amount.max(1) {
                crate::input::ax_actions::perform_ax_action(target.as_ptr(), action)?;
                crate::input::cancellation::gate().pause(std::time::Duration::from_millis(30))?;
            }
            return Ok(true);
        }
        // A nested scroll area without a semantic actuator must use a wheel
        // at its own point; never replace it with an outer area's scrollbar.
        if role.as_deref() == Some("AXScrollArea") {
            return Ok(false);
        }
        let Some(parent) = copy_element_attr(ptr, "AXParent") else {
            break;
        };
        current = crate::ax::cache::RetainedElement::retain(parent as usize);
        CFRelease(parent as CFTypeRef);
    }
    Ok(false)
}

unsafe fn scroll_native_text_area(
    element: AXUIElementRef,
    direction: &str,
    by: &str,
    amount: usize,
) -> anyhow::Result<bool> {
    if scroll_advertised_action(element, direction, by, amount)? {
        return Ok(true);
    }
    if !matches!(direction, "up" | "down")
        || copy_string_attr(element, "AXRole").as_deref() != Some("AXTextArea")
    {
        return Ok(false);
    }
    let Some(scroll_area) = copy_element_attr(element, "AXParent") else {
        return Ok(false);
    };
    if copy_string_attr(scroll_area, "AXRole").as_deref() != Some("AXScrollArea") {
        CFRelease(scroll_area as CFTypeRef);
        return Ok(false);
    }
    let mut buttons = Vec::new();
    collect_ax_buttons(scroll_area, 0, &mut buttons);
    CFRelease(scroll_area as CFTypeRef);
    if buttons.is_empty() {
        return Ok(false);
    }

    let reverse = direction == "up";
    let base = if by == "page" && buttons.len() >= 4 {
        2
    } else {
        0
    };
    let index = base + usize::from(reverse);
    let mut delivered = false;
    if let Some(target) = buttons.get(index).copied() {
        if copy_action_names(target)
            .iter()
            .any(|action| action == "AXPress")
        {
            for _ in 0..amount.max(1) {
                let result =
                    crate::input::ax_actions::perform_ax_action(target as usize, "AXPress");
                if let Err(error) = result {
                    for button in buttons {
                        CFRelease(button as CFTypeRef);
                    }
                    return Err(error);
                }
                delivered = true;
                if let Err(error) =
                    crate::input::cancellation::gate().pause(std::time::Duration::from_millis(30))
                {
                    for button in buttons {
                        CFRelease(button as CFTypeRef);
                    }
                    return Err(error);
                }
            }
        }
    }
    for button in buttons {
        CFRelease(button as CFTypeRef);
    }
    Ok(delivered)
}

unsafe fn collect_ax_buttons(
    element: AXUIElementRef,
    depth: usize,
    buttons: &mut Vec<AXUIElementRef>,
) {
    if depth >= 4 || buttons.len() >= 4 {
        return;
    }
    for child in copy_children(element) {
        if buttons.len() >= 4 {
            CFRelease(child as CFTypeRef);
        } else if copy_string_attr(child, "AXRole").as_deref() == Some("AXButton") {
            buttons.push(child);
        } else {
            collect_ax_buttons(child, depth + 1, buttons);
            CFRelease(child as CFTypeRef);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cua_driver_core::background_input::{
        decide_background_input, BackgroundAction, BackgroundInputDecision, BackgroundTargetFacts,
        ElementAncestry, ExactWindowTarget, WindowServerOwnership,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn signed_axes_keep_semantic_scroll_available_without_changing_direction() {
        assert_eq!(
            semantic_scroll_axis("up", 3, Some(0), Some(-4), false),
            Some(("down", 4))
        );
        assert_eq!(
            semantic_scroll_axis("up", 3, None, Some(2), false),
            Some(("up", 2))
        );
        assert_eq!(
            semantic_scroll_axis("up", 3, Some(-2), Some(0), false),
            Some(("right", 2))
        );
        assert_eq!(semantic_scroll_axis("up", 3, Some(1), Some(2), false), None);
        assert_eq!(semantic_scroll_axis("up", 3, None, Some(2), true), None);
    }

    #[test]
    fn exact_target_refusal_prevents_ax_reveal() {
        let action_ran = AtomicBool::new(false);
        let target = ExactWindowTarget {
            pid: 42,
            window_id: 7,
        };
        let facts = BackgroundTargetFacts {
            window_server: WindowServerOwnership::SamePid,
            ax_window_present: true,
            target_minimized: Some(false),
            app_hidden: Some(false),
            competing_keyboard_destinations: 0,
            keyboard_focus_proven: false,
            element: ElementAncestry::OutsideTargetWindow,
        };
        let refusal = match decide_background_input(target, &facts, BackgroundAction::AxSemantic) {
            BackgroundInputDecision::Refuse(refusal) => Err(
                super::super::background_refusal_result(target.pid, target.window_id, &refusal),
            ),
            BackgroundInputDecision::Execute { .. } => panic!("exact-target facts must refuse"),
        };

        let result = after_exact_target_gate(refusal, || {
            action_ran.store(true, Ordering::SeqCst);
        });

        assert!(result.is_err());
        assert!(!action_ran.load(Ordering::SeqCst));
    }
}
