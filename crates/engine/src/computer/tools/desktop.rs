use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;

use serde::Serialize;
use serde_json::{Map, Value, json};
use zeron_proto::computer::{
    ComputerActionResult, ComputerDeliveryVerification, ComputerLaunchAppResult, ComputerPoint,
    ComputerScreenshot, ComputerTarget, ComputerUiNode, CuaEffect, McpContent, McpToolResult,
};

use super::args::{
    ScreenshotRequest, ScreenshotTarget, has_target_fields, read_action_name, read_capture_request,
    read_click_gesture, read_clipboard_text, read_drag_duration_ms, read_key_or_chord,
    read_menu_path_arg, read_menu_target_arg, read_modifiers, read_paste_format, read_nested_screenshot_target,
    read_rect, read_required_text, read_screenshot_id_arg, read_screenshot_target,
    read_scroll_delta, read_select_text_selection, read_select_text_target, read_set_value_value,
    read_wait_duration_ms, read_window_id_arg,
};
use super::ax_tree::{TREE_REF_LIMIT, build_tree, render_diff, render_full};
use super::elements::{
    RefLookup, digest_scope_key, element_change_fields, listing_fields,
};
use super::guidance::computer_help_section;
use super::results::{ToolFailure, ToolValue, mcp_json};
use super::tool_input::{
    ToolArgs, ToolInputError, is_safe_integer, read_boolean_arg, read_number_arg,
    read_required_string_arg, read_string_arg, read_string_array_arg, read_verbatim_string_arg,
};
use super::{ComputerToolContext, ComputerTools};
use crate::computer::backend::{
    COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION, ComputerAgentDialect, ComputerCaptureRequest,
    ComputerError, ComputerLaunchOptions, CuaActionError,
};
use crate::computer::desktop_operation_queue::{
    DesktopOperationError, assert_desktop_operation_active, desktop_operation_signal,
};
use crate::computer::element_identity::{ObservedComputerTarget, bind_computer_target_ref};
use crate::computer::manager::{
    ComputerActionObservation, ComputerNotedResult, ComputerScrollOptions, ComputerStateRequest,
    ComputerWaitSettleResult, ComputerSettleMode,
};
use crate::computer::screenshot_frames::{
    screenshot_delta_to_desktop, screenshot_point_to_desktop, screenshot_rect_to_desktop,
};
use crate::computer::ui_tree_targeting::{
    ComputerActionableElementFilter, ComputerTargetErrorCode, actionable_elements_limited,
    is_writable_text_node, normalize_label_spaces, resolve_computer_semantic_target,
};
use crate::computer::visible_use::COMPUTER_FOREGROUND_NOT_REQUESTED_CODE;

pub(crate) const COMPUTER_TEXT_NOT_APPLIED_CODE: &str = "computer_text_not_applied";
const TYPED_TEXT_SETTLE: Duration = Duration::from_millis(250);
use crate::computer::wait_for_control::{
    ComputerControlReadiness, ComputerControlReadinessStatus, ComputerTargetLookup,
    wait_for_control,
};

pub(super) type Handled = Result<ToolValue, ToolFailure>;

const LAUNCH_NULL_WINDOW_GUIDANCE: &str = "No usable window was established. The launch may already have started the app; never launch again automatically. Inspect computer_list_windows once using the returned app identity. If no usable target exists, report the limitation instead of looping. An isolated browser via computer_browser_prepare is an alternative only when compatible with the requested task; do not silently replace a requested personal browser or incognito window.";
const UNCHANGED_FRAME_NOTE: &str = "The screen is byte-for-byte what your previous screenshot showed, with the same coordinates. Continue using this screenshotId. This does not prove nothing changed; wait and look again before repeating an action.";
const UNCHANGED_OBSERVATION_NOTE: &str = "The screen is byte-for-byte what your previous screenshot showed, with the same coordinates. Continue using this screenshotId. This does not prove the action missed; wait and look again before repeating an action.";
const ELEMENT_CHANGE_LIMIT: usize = 40;
const OBSERVED_ELEMENT_CHANGE_TOOLS: &[&str] = &[
    "computer_click",
    "computer_press_key",
    "computer_type_text",
    "computer_paste",
    "computer_set_value",
    "computer_perform_action",
];

pub(super) fn to_map<T: Serialize>(value: &T) -> Map<String, Value> {
    match serde_json::to_value(value) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

pub(super) fn json_of<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

pub(super) fn noted_json(noted: &ComputerNotedResult) -> Map<String, Value> {
    let mut map = to_map(&noted.result);
    if let Some(note) = &noted.note {
        map.insert("note".into(), json!(note));
    }
    map
}

pub(super) fn readiness_json(readiness: &ComputerControlReadiness) -> Map<String, Value> {
    let status = match readiness.status {
        ComputerControlReadinessStatus::Ready => "ready",
        ComputerControlReadinessStatus::Timeout => "timeout",
        ComputerControlReadinessStatus::Unavailable => "unavailable",
        ComputerControlReadinessStatus::Ambiguous => "ambiguous",
        ComputerControlReadinessStatus::Closed => "closed",
    };
    let mut map = Map::new();
    map.insert("status".into(), json!(status));
    map.insert("waitedMs".into(), json!(readiness.waited_ms));
    map
}

fn settle_json(verdict: &ComputerWaitSettleResult) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("settled".into(), json!(verdict.settled));
    map.insert("waitedMs".into(), json!(verdict.waited_ms));
    if let Some(events) = verdict.events_seen {
        map.insert("eventsSeen".into(), json!(events));
    }
    map.insert(
        "mode".into(),
        json!(match verdict.mode {
            ComputerSettleMode::Observer => "observer",
            ComputerSettleMode::Fixed => "fixed",
        }),
    );
    map
}

pub(super) fn semantic_lookup(
    root: &zeron_proto::computer::ComputerUiNode,
    target: &ComputerTarget,
) -> Result<(), ComputerTargetLookup<ComputerError>> {
    match resolve_computer_semantic_target(root, target, false) {
        Ok(_) => Ok(()),
        Err(error) => match error.code {
            ComputerTargetErrorCode::Ambiguous => Err(ComputerTargetLookup::Ambiguous),
            ComputerTargetErrorCode::NotFound => Err(ComputerTargetLookup::NotFound),
            _ => Err(ComputerTargetLookup::Failed(error.into())),
        },
    }
}

pub(super) fn with_launch_guidance(result: &ComputerLaunchAppResult) -> Value {
    let mut map = to_map(result);
    if result.focus_changed_during_launch == Some(true) {
        let mut guidance = String::from(
            "The app changed desktop focus during launch despite the background request. Do not relaunch or continue input from the previous observation. Read fresh state and respect the user's visible-use permission.",
        );
        if result.window.is_none() {
            guidance.push(' ');
            guidance.push_str(LAUNCH_NULL_WINDOW_GUIDANCE);
        }
        map.insert("toolGuidance".into(), json!(guidance));
    } else if result.window.is_none() {
        map.insert("toolGuidance".into(), json!(LAUNCH_NULL_WINDOW_GUIDANCE));
    }
    Value::Object(map)
}

fn point_of(result: &Map<String, Value>) -> Option<ComputerPoint> {
    result
        .get("clampedTo")
        .or_else(|| result.get("point"))
        .and_then(|point| serde_json::from_value(point.clone()).ok())
}

fn window_of(result: &Map<String, Value>) -> Option<String> {
    result
        .get("windowId")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub(super) enum ObservedOutcome {
    Plain(Map<String, Value>),
    Observed {
        result: Map<String, Value>,
        observation: Option<ComputerActionObservation>,
    },
}

fn args_has(args: &ToolArgs, name: &str) -> bool {
    args.contains_key(name)
}

fn app_playbook_topic(app_name: &str) -> Option<&'static str> {
    Some(match app_name {
        "finder" => "finder",
        "calculator" => "calculator",
        "slack" => "slack",
        "notes" | "textedit" | "bbedit" | "coteditor" | "sublime text" | "pages" => "editors",
        "terminal" | "iterm2" | "iterm" | "ghostty" | "warp" | "alacritty" | "kitty"
        | "wezterm" | "hyper" => "terminals",
        "code" | "visual studio code" | "cursor" | "windsurf" | "obsidian" | "notion"
        | "discord" | "figma" | "linear" | "microsoft teams" => "electron",
        _ => return None,
    })
}

pub(super) fn is_strip_window(window: &zeron_proto::computer::ComputerWindow) -> bool {
    let Some(bounds) = window.bounds else {
        return false;
    };
    bounds.width <= 0.0
        || bounds.height <= 0.0
        || (bounds.height <= 40.0 && (!window.visible || bounds.y < 0.0))
}

impl ComputerTools {
    pub(super) fn call_desktop<'a>(
        &'a self,
        name: &'a str,
        args: &'a ToolArgs,
        context: &'a ComputerToolContext,
    ) -> BoxFuture<'a, McpToolResult> {
        let c = context;
        match name {
            "computer_inspect" => self.inspect(args, c).boxed(),
            "computer_help" => self.handle(name, args, c, || self.help(args)).boxed(),
            "computer_run" => self.handle(name, args, c, || self.run_batch(args, c)).boxed(),
            "computer_list_windows" => self.handle(name, args, c, || self.list_windows(args)).boxed(),
            "computer_get_state" => self.handle(name, args, c, || self.get_state(args, c)).boxed(),
            "computer_screenshot" => self.handle(name, args, c, || self.screenshot(args, c)).boxed(),
            "computer_get_screen_size" => {
                self.handle(name, args, c, || async {
                    Ok(json_of(&self.manager.get_screen_size().await?).into())
                })
                .boxed()
            }
            "computer_wait" => self.handle(name, args, c, || self.wait(args, c)).boxed(),
            "computer_read_clipboard" => {
                self.handle(name, args, c, || async {
                    Ok(json_of(&self.manager.read_clipboard(Some(&c.chat_id)).await?).into())
                })
                .boxed()
            }
            "computer_launch_app" => self.handle(name, args, c, || self.launch_app(args, c)).boxed(),
            "computer_list_apps" => {
                self.handle(name, args, c, || async {
                    Ok(json_of(&self.manager.list_apps().await?).into())
                })
                .boxed()
            }
            "computer_verify_state" => self.handle(name, args, c, || self.verify_state(args)).boxed(),
            "computer_zoom" => self.handle(name, args, c, || self.zoom(args)).boxed(),
            "computer_get_accessibility_tree" => {
                self.handle(name, args, c, || async {
                    let window_id = read_window_id_arg(args)?;
                    Ok(json_of(
                        &self
                            .manager
                            .get_accessibility_tree(window_id.as_deref())
                            .await?,
                    )
                    .into())
                })
                .boxed()
            }
            "computer_get_cursor_position" => {
                self.handle(name, args, c, || async {
                    let window_id = read_window_id_arg(args)?;
                    Ok(json_of(&self.manager.get_cursor_position(window_id.as_deref()).await?).into())
                })
                .boxed()
            }
            "computer_set_window_frame" => {
                self.handle(name, args, c, || self.set_window_frame(args, c)).boxed()
            }
            "computer_invoke_menu" => self.handle(name, args, c, || self.invoke_menu(args, c)).boxed(),
            "computer_kill_app" => {
                self.handle(name, args, c, || async {
                    let window_id = required_window(args)?;
                    Ok(json_of(&self.manager.kill_app(Some(&c.chat_id), &window_id).await?).into())
                })
                .boxed()
            }
            "computer_set_window_minimized" => {
                self.handle(name, args, c, || self.set_window_minimized(args, c)).boxed()
            }
            "computer_set_app_visibility" => {
                self.handle(name, args, c, || self.set_app_visibility(args, c)).boxed()
            }
            "computer_activate_window" => {
                self.handle(name, args, c, || self.activate_window(args, c)).boxed()
            }
            "computer_write_clipboard" => {
                self.handle(name, args, c, || async {
                    let text = read_clipboard_text(args)?;
                    Ok(json_of(&self.manager.write_clipboard(Some(&c.chat_id), &text).await?).into())
                })
                .boxed()
            }
            "computer_click" | "computer_move_cursor" | "computer_drag" | "computer_scroll"
            | "computer_type_text" | "computer_press_key" | "computer_paste"
            | "computer_set_value" | "computer_perform_action" | "computer_select_text" => {
                self.handle(name, args, c, || self.observed(name, args, c)).boxed()
            }
            _ => {
                let result = McpToolResult::error(format!("Unknown computer tool \"{name}\"."));
                async move { result }.boxed()
            }
        }
    }

    pub(super) fn native_identity(
        &self,
    ) -> impl Fn(&zeron_proto::computer::ComputerUiNode) -> Option<String> + '_ {
        |node| self.manager.native_element_identity(node)
    }

    pub(super) fn resolve_target(
        &self,
        target: ScreenshotTarget,
        thread_id: &str,
    ) -> Result<ObservedComputerTarget, ToolFailure> {
        let ScreenshotTarget {
            target,
            screenshot_id,
        } = target;
        if let Some(element_ref) = target.element_ref {
            if target.x.is_some() || target.y.is_some() {
                return Err(ToolInputError::new(
                    "A ref target takes no x/y; it names an element from the elements listing.",
                )
                .into());
            }
            let entry = match self.state().elements.lookup_ref(thread_id, element_ref) {
                RefLookup::NoListing => {
                    return Err(ToolInputError::new(
                        "No elements listing exists yet in this thread; ref targets need a computer_get_state first.",
                    )
                    .into());
                }
                RefLookup::Unknown => {
                    return Err(ToolInputError::new(format!(
                        "ref {element_ref} does not match any element this thread has observed. Observe again with computer_get_state."
                    ))
                    .into());
                }
                RefLookup::Found(entry) => entry,
            };
            let quoted = |text: &str| Value::String(text.to_owned()).to_string();
            if let Some(label) = &target.label {
                let claim = normalize_label_spaces(label);
                let claim = claim.strip_suffix('…').unwrap_or(&claim);
                if !normalize_label_spaces(&entry.label).starts_with(claim) {
                    return Err(ToolInputError::new(format!(
                        "ref {element_ref} is the {} {}, not {}. Observe again with computer_get_state.",
                        entry.role,
                        quoted(&entry.label),
                        quoted(label)
                    ))
                    .into());
                }
            }
            if let Some(role) = &target.role
                && *role != entry.role
            {
                return Err(ToolInputError::new(format!(
                    "ref {element_ref} is a {}, not {}. Observe again with computer_get_state.",
                    entry.role,
                    quoted(role)
                ))
                .into());
            }
            if let (Some(window_id), Some(entry_window)) = (&target.window_id, &entry.window_id)
                && window_id != entry_window
            {
                return Err(ToolInputError::new(format!(
                    "ref {element_ref} is in window {}, not {}. Observe again with computer_get_state.",
                    quoted(entry_window),
                    quoted(window_id)
                ))
                .into());
            }
            if let Some(ordinal) = target.ref_ordinal
                && ordinal != entry.ordinal as u64
            {
                return Err(ToolInputError::new(format!(
                    "ref {element_ref} is duplicate {} of its label, not {}. Observe again with computer_get_state.",
                    entry.ordinal + 1,
                    ordinal + 1
                ))
                .into());
            }
            return Ok(bind_computer_target_ref(
                ComputerTarget {
                    label: Some(entry.label.clone()),
                    role: Some(entry.role.clone()),
                    window_id: entry.window_id.clone(),
                    ref_ordinal: Some(entry.ordinal as u64),
                    ..ComputerTarget::default()
                },
                &entry,
            ));
        }
        let (Some(x), Some(y)) = (target.x, target.y) else {
            return Ok(target.into());
        };
        let state = self.state();
        let frame = state.frames.resolve(thread_id, screenshot_id.as_deref())?;
        if let (Some(frame_window), Some(window_id)) = (&frame.window_id, &target.window_id)
            && frame_window != window_id
        {
            return Err(
                ToolInputError::new("Screenshot and action name different windows.").into(),
            );
        }
        let point = screenshot_point_to_desktop(frame, x, y)?;
        let mut resolved = ComputerTarget {
            x: Some(point.x),
            y: Some(point.y),
            ..target
        };
        let mut observed_window_bounds = None;
        if let Some(frame_window) = &frame.window_id {
            resolved.window_id = Some(frame_window.clone());
            observed_window_bounds = Some(frame.region);
        }
        let mut observed: ObservedComputerTarget = resolved.into();
        observed.observed_window_bounds = observed_window_bounds;
        Ok(observed)
    }

    pub(super) fn read_target(
        &self,
        args: &ToolArgs,
        context: &ComputerToolContext,
    ) -> Result<ObservedComputerTarget, ToolFailure> {
        self.resolve_target(read_screenshot_target(args)?, &context.chat_id)
    }

    fn read_nested_target(
        &self,
        args: &ToolArgs,
        name: &str,
        context: &ComputerToolContext,
    ) -> Result<ObservedComputerTarget, ToolFailure> {
        self.resolve_target(read_nested_screenshot_target(args, name)?, &context.chat_id)
    }

    pub(super) fn deliver_screenshot(
        &self,
        thread_id: &str,
        mut payload: Map<String, Value>,
        screenshot: &ComputerScreenshot,
        window_id: Option<&str>,
    ) -> Result<McpToolResult, ToolFailure> {
        assert_desktop_operation_active()?;
        if let (Some(requested), Some(captured)) = (window_id, screenshot.window_id.as_deref())
            && requested != captured
        {
            return Err(ToolInputError::new(
                "Screenshot identity differs from the requested window.",
            )
            .into());
        }
        let window_id = window_id.or(screenshot.window_id.as_deref());
        let mut state = self.state();
        if self.capture_reuse.load(Ordering::SeqCst)
            && let Some(reused) = state.frames.match_latest(thread_id, screenshot, window_id)
        {
            payload.insert("screenshotUnchanged".into(), json!(true));
            payload.insert("screenshotId".into(), json!(reused.id));
            payload.insert("screenshot".into(), frame_json(reused));
            payload.insert("note".into(), json!(UNCHANGED_FRAME_NOTE));
            return Ok(mcp_json(&Value::Object(payload)));
        }
        let frame = state.frames.record(thread_id, screenshot.into(), window_id);
        let mut metadata = to_map(screenshot);
        metadata.remove("bytesBase64");
        if let Some(window_id) = window_id {
            metadata.insert("windowId".into(), json!(window_id));
        }
        if let Some(frame) = frame {
            metadata.insert("screenshotId".into(), json!(frame.id));
        }
        payload.insert("screenshot".into(), Value::Object(metadata));
        Ok(McpToolResult {
            content: vec![
                McpContent::Text {
                    text: Value::Object(payload).to_string(),
                },
                McpContent::Image {
                    data: screenshot.bytes_base64.clone(),
                    mime_type: "image/png".into(),
                },
            ],
            ..McpToolResult::default()
        })
    }

    fn tree_text(
        &self,
        thread_id: &str,
        window_id: &str,
        windows: &[zeron_proto::computer::ComputerWindow],
        ax_tree: Option<&zeron_proto::computer::ComputerAxTree>,
        root: &zeron_proto::computer::ComputerUiNode,
        full: bool,
    ) -> Option<String> {
        let listing = actionable_elements_limited(
            root,
            &ComputerActionableElementFilter {
                window_id: Some(window_id.to_owned()),
                label_contains: None,
            },
            self.native_identity(),
            TREE_REF_LIMIT,
        );
        let mut tool_state = self.state();
        let listing = tool_state.elements.sync_refs(thread_id, listing);
        let window = windows.iter().find(|window| window.id == window_id);
        let snapshot = build_tree(window, window_id, ax_tree, Some(root), &listing)?;
        let key = digest_scope_key(thread_id, Some(window_id), None);
        Some(
            match tool_state.elements.swap_tree(&key, snapshot.clone()) {
                Some(previous) if !full => render_diff(&previous, &snapshot),
                _ => render_full(&snapshot),
            },
        )
    }

    pub(super) fn app_playbook(
        &self,
        thread_id: &str,
        windows: &[zeron_proto::computer::ComputerWindow],
        window_id: &str,
    ) -> Option<&'static str> {
        let app_name = windows
            .iter()
            .find(|window| window.id == window_id)
            .and_then(|window| window.app_name.as_deref())
            .map(str::to_lowercase)?;
        let note = computer_help_section(app_playbook_topic(&app_name)?)?;
        let seen = json!([thread_id, app_name]).to_string();
        let mut tool_state = self.state();
        if tool_state.app_hints_seen.contains(&seen) {
            return None;
        }
        while tool_state.app_hints_seen.len() >= super::THREAD_KEYED_LIMIT {
            tool_state.app_hints_seen.remove(0);
        }
        tool_state.app_hints_seen.push(seen);
        Some(note)
    }

    fn computer_id_payload(&self) -> Map<String, Value> {
        let mut payload = Map::new();
        payload.insert("computerId".into(), json!(self.manager.computer_id()));
        payload
    }

    async fn list_windows(&self, args: &ToolArgs) -> Handled {
        let app = read_string_arg(args, "app", false)?.map(|app| app.to_lowercase());
        let include_strips = read_boolean_arg(args, "include_strips")?.unwrap_or(false);
        let mut result = self.manager.list_windows().await?;
        if !include_strips {
            result.windows.retain(|window| !is_strip_window(window));
        }
        if let Some(app) = app {
            result.windows.retain(|window| {
                window
                    .app_name
                    .as_deref()
                    .is_some_and(|name| name.to_lowercase() == app)
            });
        }
        Ok(json_of(&result).into())
    }

    async fn get_state(&self, args: &ToolArgs, context: &ComputerToolContext) -> Handled {
        let want_text = read_boolean_arg(args, "include_text")?.unwrap_or(false);
        let window_id = read_window_id_arg(args)?;
        let label_contains = match read_verbatim_string_arg(args, "label_contains")? {
            Some(label) => Some(label),
            None => read_verbatim_string_arg(args, "labelContains")?,
        };
        let want_diff = read_boolean_arg(args, "diff")?.unwrap_or(false);
        let include_screenshot = read_boolean_arg(args, "include_screenshot")?.unwrap_or(false);
        let mut state = self
            .manager
            .get_state(ComputerStateRequest {
                include_screenshot: Some(include_screenshot),
                include_text: want_text,
                include_tree: Some(true),
                window_id: window_id.clone(),
            })
            .await?;
        if window_id.is_none() {
            state.windows.retain(|window| !is_strip_window(window));
        }
        let text = state.text.take();
        let root = state.root.take();
        let screenshot = state.screenshot.take();
        let elements = root.as_ref().map(|root| {
            crate::computer::ui_tree_targeting::actionable_elements(
                root,
                &ComputerActionableElementFilter {
                    window_id: window_id.clone(),
                    label_contains: label_contains.clone(),
                },
                self.native_identity(),
            )
        });
        let key = digest_scope_key(
            &context.chat_id,
            window_id.as_deref(),
            label_contains.as_deref(),
        );
        let (before, stable) = {
            let mut tool_state = self.state();
            let before = tool_state.elements.digest(&key);
            let stable = elements
                .map(|elements| tool_state.elements.remember_digest(&context.chat_id, &key, elements));
            (before, stable)
        };
        let ax_tree = state.ax_tree.take();
        let tree = match (window_id.as_deref(), root.as_ref()) {
            (Some(window_id), Some(root)) => self.tree_text(
                &context.chat_id,
                window_id,
                &state.windows,
                ax_tree.as_ref(),
                root,
                read_boolean_arg(args, "full")?.unwrap_or(false),
            ),
            _ => None,
        };
        let app_hint = window_id
            .as_deref()
            .and_then(|window_id| self.app_playbook(&context.chat_id, &state.windows, window_id));
        let mut payload = to_map(&state);
        if want_text && let Some(text) = text {
            payload.insert("text".into(), json!(text));
        }
        if let Some(tree) = tree {
            payload.insert("tree".into(), json!(tree));
        }
        if let Some(stable) = &stable {
            payload.extend(if want_diff {
                element_change_fields(before.as_ref(), stable, None)
            } else {
                listing_fields(stable)
            });
        }
        if let Some(app_hint) = app_hint {
            payload.insert("appHint".into(), json!(app_hint));
        }
        match screenshot {
            None => Ok(Value::Object(payload).into()),
            Some(screenshot) => Ok(self
                .deliver_screenshot(&context.chat_id, payload, &screenshot, None)?
                .into()),
        }
    }

    async fn screenshot(&self, args: &ToolArgs, context: &ComputerToolContext) -> Handled {
        let thread_id = context.chat_id.as_str();
        let screenshot_id = read_screenshot_id_arg(args)?;
        let request = read_capture_request(args, |region| {
            let state = self.state();
            let frame = state.frames.resolve(thread_id, screenshot_id.as_deref())?;
            screenshot_rect_to_desktop(frame, &region)
        })??;
        match request {
            ScreenshotRequest::Focused { max_dimension } => {
                let capture = self
                    .manager
                    .capture_focused_window(max_dimension, false)
                    .await?;
                Ok(self
                    .deliver_screenshot(
                        thread_id,
                        self.computer_id_payload(),
                        &capture.screenshot,
                        capture.window_id.as_deref(),
                    )?
                    .into())
            }
            ScreenshotRequest::Capture(request) => {
                let window_id = match &request {
                    ComputerCaptureRequest::Window { window_id, .. } => Some(window_id.clone()),
                    ComputerCaptureRequest::Region { .. } => None,
                };
                let screenshot = self.manager.capture_screenshot(request).await?;
                Ok(self
                    .deliver_screenshot(
                        thread_id,
                        self.computer_id_payload(),
                        &screenshot,
                        window_id.as_deref(),
                    )?
                    .into())
            }
        }
    }

    async fn wait(&self, args: &ToolArgs, context: &ComputerToolContext) -> Handled {
        let duration_ms = read_wait_duration_ms(args)?;
        let settle = read_boolean_arg(args, "settle")?;
        if args_has(args, "label") && settle == Some(true) {
            return Err(ToolFailure::message(
                "A settle wait cannot combine with label; pick one observation mode.",
            ));
        }
        let manager = &self.manager;
        let chat_id = context.chat_id.as_str();
        let turn_id = context.turn_id.as_deref();
        if args_has(args, "label") {
            let target = self.read_target(args, context)?.target;
            let (Some(window_id), true) = (
                target.window_id.clone(),
                target
                    .label
                    .as_deref()
                    .is_some_and(|label| !label.trim().is_empty()),
            ) else {
                return Err(ToolFailure::message(
                    "A conditional wait requires a nonempty label and window_id.",
                ));
            };
            // Each poll re-mints authority, so only the one held from the start sees a Stop or Escape.
            let authority = manager.thread_authority_signal(chat_id);
            let signal = Some(match desktop_operation_signal() {
                Some(signal) => signal.any(&authority),
                None => authority,
            });
            let readiness = wait_for_control(
                || {
                    let window_id = window_id.clone();
                    let signal = desktop_operation_signal();
                    async move {
                        manager
                            .with_agent_activity(
                                chat_id,
                                || async {
                                    self.assert_turn_active(context)?;
                                    manager
                                        .get_state(ComputerStateRequest {
                                            include_tree: Some(true),
                                            window_id: Some(window_id),
                                            ..ComputerStateRequest::default()
                                        })
                                        .await
                                },
                                signal,
                                turn_id,
                                None,
                            )
                            .await
                    }
                },
                semantic_lookup,
                &target,
                duration_ms,
                signal.as_ref(),
                false,
            )
            .await?;
            let mut result = self.computer_id_payload();
            result.extend(readiness_json(&readiness));
            if read_boolean_arg(args, "include_screenshot")? == Some(false)
                || readiness.status == ComputerControlReadinessStatus::Closed
            {
                return Ok(Value::Object(result).into());
            }
            let screenshot = manager
                .with_agent_activity(
                    chat_id,
                    || async {
                        self.assert_turn_active(context)?;
                        manager
                            .capture_screenshot(ComputerCaptureRequest::Window {
                                window_id: window_id.clone(),
                                max_dimension: Some(COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION),
                            })
                            .await
                    },
                    desktop_operation_signal(),
                    turn_id,
                    None,
                )
                .await?;
            return Ok(self
                .deliver_screenshot(chat_id, result, &screenshot, Some(&window_id))?
                .into());
        }
        if settle == Some(true) {
            let Some(window_id) = self.read_target(args, context)?.target.window_id else {
                return Err(ToolFailure::message("A settle wait requires window_id."));
            };
            let verdict = manager
                .with_agent_activity(
                    chat_id,
                    || async {
                        self.assert_turn_active(context)?;
                        manager.wait_for_settle(&window_id, duration_ms as f64).await
                    },
                    desktop_operation_signal(),
                    turn_id,
                    None,
                )
                .await?;
            let mut result = self.computer_id_payload();
            result.extend(settle_json(&verdict));
            return Ok(Value::Object(result).into());
        }
        if duration_ms > 0 {
            pause(duration_ms).await?;
        }
        let mut result = self.computer_id_payload();
        result.insert("waitedMs".into(), json!(duration_ms));
        Ok(Value::Object(result).into())
    }

    pub(super) async fn refuse_visible_launch(
        &self,
        context: &ComputerToolContext,
        hidden: Option<bool>,
    ) -> Result<(), ToolFailure> {
        if self.dialect != ComputerAgentDialect::Macos
            && hidden == Some(false)
            && !context
                .seams
                .foreground_authorization(None)
                .await
                .user_requested_visible_use
        {
            return Err(CuaActionError::new(
                "The user's task did not ask for this app to be shown; launching it visibly would take their screen. Keep the launch in the background, or ask the user to confirm they want to watch.",
                CuaEffect::NotDispatched,
            )
            .code(COMPUTER_FOREGROUND_NOT_REQUESTED_CODE)
            .into());
        }
        Ok(())
    }

    async fn launch_app(&self, args: &ToolArgs, context: &ComputerToolContext) -> Handled {
        let hidden = read_boolean_arg(args, "hidden")?;
        self.refuse_visible_launch(context, hidden).await?;
        let app = read_required_string_arg(args, "app")?;
        let app_args = read_string_array_arg(args, "arguments")?.unwrap_or_default();
        let wait_ms = if read_boolean_arg(args, "wait_for_window")? == Some(false) {
            0
        } else {
            2_000
        };
        let result = self
            .manager
            .launch_app(
                Some(&context.chat_id),
                &app,
                &app_args,
                wait_ms,
                ComputerLaunchOptions { hidden },
            )
            .await?;
        Ok(with_launch_guidance(&result).into())
    }

    async fn verify_state(&self, args: &ToolArgs) -> Handled {
        let window_id = read_window_id_arg(args)?
            .ok_or_else(|| ToolInputError::new("window_id is required."))?;
        let expect = match args.get("expect") {
            Some(Value::Array(items)) if !items.is_empty() && items.len() <= 8 => items.clone(),
            _ => {
                return Err(ToolInputError::new(
                    "expect must be an array of one to eight predicates.",
                )
                .into());
            }
        };
        if expect.iter().any(|predicate| !predicate.is_object()) {
            return Err(ToolInputError::new("Each expect predicate must be an object.").into());
        }
        Ok(json_of(&self.manager.verify_state(&window_id, &expect).await?).into())
    }

    async fn zoom(&self, args: &ToolArgs) -> Handled {
        let window_id = required_window(args)?;
        let region = read_rect(args)?;
        if region.width <= 0.0 || region.height <= 0.0 {
            return Err(ToolInputError::new(
                "x/y/width/height must be finite, with positive size.",
            )
            .into());
        }
        let zoom = self.manager.zoom_window(&window_id, region).await?;
        let mut metadata = to_map(&zoom);
        metadata.remove("bytesBase64");
        let mime_type = metadata
            .get("mimeType")
            .and_then(Value::as_str)
            .unwrap_or("image/jpeg")
            .to_owned();
        let mut payload = self.computer_id_payload();
        payload.insert("zoom".into(), Value::Object(metadata));
        Ok(McpToolResult {
            content: vec![
                McpContent::Text {
                    text: Value::Object(payload).to_string(),
                },
                McpContent::Image {
                    data: zoom.bytes_base64,
                    mime_type,
                },
            ],
            ..McpToolResult::default()
        }
        .into())
    }

    async fn set_window_frame(&self, args: &ToolArgs, context: &ComputerToolContext) -> Handled {
        let window_id = required_window(args)?;
        let frame = read_rect(args)?;
        if frame.width <= 0.0 || frame.height <= 0.0 {
            return Err(ToolInputError::new(
                "x/y/width/height must be finite, with positive size.",
            )
            .into());
        }
        Ok(json_of(
            &self
                .manager
                .set_window_frame(Some(&context.chat_id), &window_id, frame)
                .await?,
        )
        .into())
    }

    async fn invoke_menu(&self, args: &ToolArgs, context: &ComputerToolContext) -> Handled {
        let target = read_menu_target_arg(args)?;
        let path = read_menu_path_arg(args, "computer_invoke_menu")?;
        let authorization = context.seams.foreground_authorization(Some(args)).await;
        let result = self
            .manager
            .invoke_menu(Some(&context.chat_id), target, &path, Some(authorization))
            .await?;
        Ok(Value::Object(
            self.with_action_element_changes(to_map(&result), context)
                .await?,
        )
        .into())
    }

    async fn set_window_minimized(&self, args: &ToolArgs, context: &ComputerToolContext) -> Handled {
        if read_string_arg(args, "delivery_mode", false)?.as_deref() == Some("foreground") {
            return Err(ToolInputError::new(
                "computer_set_window_minimized never activates; it takes no delivery_mode.",
            )
            .into());
        }
        let window_id = required_window(args)?;
        let minimized = read_boolean_arg(args, "minimized")?.ok_or_else(|| {
            ToolInputError::new("minimized is required and must be a boolean.")
        })?;
        Ok(json_of(
            &self
                .manager
                .set_window_minimized(Some(&context.chat_id), &window_id, minimized)
                .await?,
        )
        .into())
    }

    async fn set_app_visibility(&self, args: &ToolArgs, context: &ComputerToolContext) -> Handled {
        if read_string_arg(args, "delivery_mode", false)?.as_deref() == Some("foreground") {
            return Err(ToolInputError::new(
                "computer_set_app_visibility never activates; it takes no delivery_mode.",
            )
            .into());
        }
        let pid = read_pid(args, "pid is required and must be a positive integer.")?;
        let hidden = read_boolean_arg(args, "hidden")?
            .ok_or_else(|| ToolInputError::new("hidden is required and must be a boolean."))?;
        let noted = self
            .manager
            .set_app_visibility(Some(&context.chat_id), pid, hidden)
            .await?;
        Ok(Value::Object(noted_json(&noted)).into())
    }

    async fn activate_window(&self, args: &ToolArgs, context: &ComputerToolContext) -> Handled {
        let window_id = read_window_id_arg(args)?
            .ok_or_else(|| ToolInputError::new("Missing required argument \"window_id\"."))?;
        let authorization = context.seams.foreground_authorization(Some(args)).await;
        let noted = self
            .manager
            .foreground_with_restore(Some(&context.chat_id), &window_id, None, Some(authorization))
            .await?;
        Ok(Value::Object(noted_json(&noted)).into())
    }

    async fn observed(&self, name: &str, args: &ToolArgs, context: &ComputerToolContext) -> Handled {
        if args_has(args, "wait_for_label") {
            let label = args.get("wait_for_label").and_then(Value::as_str);
            if label.is_none_or(|label| label.trim().is_empty()) {
                return Err(ToolFailure::message("wait_for_label must be a nonempty label."));
            }
            if read_boolean_arg(args, "include_screenshot")? == Some(false) {
                return Err(ToolFailure::message(
                    "wait_for_label requires the action screenshot.",
                ));
            }
        }
        let outcome = self.observed_action(name, args, context).await?;
        let (result, observation) = match outcome {
            ObservedOutcome::Plain(result) => (result, None),
            ObservedOutcome::Observed {
                result,
                observation,
            } => (result, Some(observation)),
        };
        let observed = if OBSERVED_ELEMENT_CHANGE_TOOLS.contains(&name) {
            self.with_action_element_changes(result, context).await?
        } else {
            result
        };
        match observation {
            Some(observation) if !args_has(args, "wait_for_label") => {
                self.with_observation(context, observed, observation)
            }
            _ => self.observe_after_action(args, observed, context).await,
        }
    }

    async fn observed_action(
        &self,
        name: &str,
        args: &ToolArgs,
        context: &ComputerToolContext,
    ) -> Result<ObservedOutcome, ToolFailure> {
        let thread = Some(context.chat_id.as_str());
        let manager = &self.manager;
        let plain = |result: &zeron_proto::computer::ComputerActionResult| {
            ObservedOutcome::Plain(to_map(result))
        };
        Ok(match name {
            "computer_click" => {
                let target = self.read_target(args, context)?;
                let modifiers = read_modifiers(args)?;
                let gesture = read_click_gesture(args)?;
                plain(&manager.click(thread, target, &modifiers, gesture).await?)
            }
            "computer_move_cursor" => {
                plain(&manager.move_cursor(thread, self.read_target(args, context)?).await?)
            }
            "computer_drag" => {
                let from = self.read_nested_target(args, "from", context)?;
                let to = self.read_nested_target(args, "to", context)?;
                let duration = read_drag_duration_ms(args)?;
                plain(&manager.drag(thread, from, to, Some(duration)).await?)
            }
            "computer_scroll" => self.scroll(args, context).await?,
            "computer_type_text" => {
                let target = self.read_target(args, context)?;
                let text = read_required_text(args)?;
                let result = if target.target.label.is_some() || target.target.role.is_some() {
                    manager.type_text_at(thread, &text, target).await?
                } else {
                    manager
                        .type_text(thread, &text, target.target.window_id.as_deref())
                        .await?
                };
                let warning = self.typed_text_readback(&text, &result).await?;
                let mut map = to_map(&result);
                if let Some(warning) = warning {
                    map.insert("warning".into(), json!(warning));
                }
                ObservedOutcome::Plain(map)
            }
            "computer_press_key" => {
                let key = read_key_or_chord(args)?;
                let target = self.read_target(args, context)?;
                let window_id = target.target.window_id.clone();
                let exact = (target.target.label.is_some() || target.target.role.is_some())
                    .then_some(target);
                plain(&match key.chord {
                    None => {
                        manager
                            .press_key(thread, &key.key, window_id.as_deref(), exact)
                            .await?
                    }
                    Some(chord) => {
                        manager
                            .hotkey(thread, &chord, window_id.as_deref(), exact)
                            .await?
                    }
                })
            }
            "computer_paste" => {
                let text = read_clipboard_text(args)?;
                let window_id = read_window_id_arg(args)?;
                let format = read_paste_format(args)?;
                let pasted = manager
                    .paste(thread, &text, window_id.as_deref(), format)
                    .await?;
                let mut result = to_map(&pasted.result);
                result.insert("clipboardRestored".into(), json!(pasted.clipboard_restored));
                ObservedOutcome::Plain(result)
            }
            "computer_set_value" => {
                let target = self.read_target(args, context)?;
                let value = read_set_value_value(args)?;
                plain(&manager.set_value(thread, target, &value).await?)
            }
            "computer_perform_action" => {
                let target = self.read_target(args, context)?;
                let action = read_action_name(args)?;
                plain(&manager.perform_action(thread, target, &action).await?)
            }
            "computer_select_text" => {
                let target = self.resolve_target(read_select_text_target(args)?, &context.chat_id)?;
                let selection = read_select_text_selection(args)?;
                plain(&manager.select_text(thread, target, selection).await?)
            }
            _ => return Err(ToolFailure::message(format!("Unknown computer tool \"{name}\"."))),
        })
    }

    pub(super) fn scroll_request(
        &self,
        args: &ToolArgs,
        thread_id: &str,
    ) -> Result<ScrollRequest, ToolFailure> {
        let raw = read_screenshot_target(args)?;
        let screenshot_id = raw.screenshot_id.clone();
        let (frame_window, frame_region, frame) = {
            let state = self.state();
            let frame = state.frames.resolve(thread_id, screenshot_id.as_deref())?;
            (frame.window_id.clone(), frame.region, frame.clone())
        };
        let resolved = self.resolve_target(raw, thread_id)?;
        let mut target = resolved;
        if !has_target_fields(&target.target)
            && let Some(window) = &frame_window
        {
            target.target.window_id = Some(window.clone());
        }
        let (requested_x, requested_y) = read_scroll_delta(args)?;
        let delta = screenshot_delta_to_desktop(&frame, requested_x, requested_y);
        let limit = |value: f64, span: f64| crate::computer::scroll_calibration::js_sign(value) * value.abs().min(span / 2.0);
        let limited = zeron_proto::computer::ComputerScrollDelta {
            delta_x: limit(delta.delta_x, frame_region.width),
            delta_y: limit(delta.delta_y, frame_region.height),
        };
        Ok(ScrollRequest {
            has_target: has_target_fields(&target.target),
            target,
            delta,
            limited,
            modifiers: read_modifiers(args)?,
            frame_window,
        })
    }

    async fn scroll(
        &self,
        args: &ToolArgs,
        context: &ComputerToolContext,
    ) -> Result<ObservedOutcome, ToolFailure> {
        let thread_id = context.chat_id.as_str();
        let request = self.scroll_request(args, thread_id)?;
        let incoming = request
            .target
            .target
            .window_id
            .clone()
            .or_else(|| request.frame_window.clone());
        {
            let mut state = self.state();
            let streak = state
                .unchanged_scrolls
                .iter()
                .find(|(thread, _)| thread == thread_id)
                .map(|(_, streak)| streak.clone());
            let streak = match streak {
                Some((window, _))
                    if incoming.is_some() && window.is_some() && incoming != window =>
                {
                    state
                        .unchanged_scrolls
                        .retain(|(thread, _)| thread != thread_id);
                    None
                }
                other => other,
            };
            if let Some((window, count)) = streak
                && count >= 3
                && (incoming.is_none() || incoming == window)
            {
                return Err(ToolInputError::new(
                    "Refusing a fourth consecutive scroll with no visible movement on this window. An edge, a non-scrollable target or dropped delivery may explain it. Stop scrolling and call computer_get_state with label_contains to find a labeled control instead.",
                )
                .into());
            }
        }
        let outcome = self
            .manager
            .scroll_calibrated(
                Some(thread_id),
                request.has_target.then_some(request.target.clone()),
                request.limited.delta_x,
                request.limited.delta_y,
                ComputerScrollOptions {
                    observe: read_boolean_arg(args, "include_screenshot")? != Some(false),
                    modifiers: request.modifiers.clone(),
                },
            )
            .await?;
        let traveled_y = outcome
            .result
            .scroll
            .as_ref()
            .and_then(|scroll| scroll.traveled_y);
        let captured = match &outcome.observation {
            Some(ComputerActionObservation::Captured(captured)) => Some(captured.clone()),
            _ => None,
        };
        let will_be_unchanged = !args_has(args, "wait_for_label")
            && captured.as_ref().is_some_and(|captured| {
                self.state()
                    .frames
                    .match_latest(
                        thread_id,
                        &captured.screenshot,
                        captured.window_id.as_deref(),
                    )
                    .is_some()
            });
        let result_window = outcome
            .result
            .window_id
            .clone()
            .or_else(|| captured.as_ref().and_then(|captured| captured.window_id.clone()))
            .or(incoming);
        let no_movement =
            will_be_unchanged || (request.limited.delta_x == 0.0 && traveled_y == Some(0.0));
        {
            let mut state = self.state();
            if no_movement {
                let current = state
                    .unchanged_scrolls
                    .iter()
                    .position(|(thread, _)| thread == thread_id)
                    .map(|index| state.unchanged_scrolls.remove(index).1);
                while state.unchanged_scrolls.len() >= super::THREAD_KEYED_LIMIT {
                    state.unchanged_scrolls.remove(0);
                }
                let count = match current {
                    Some((window, count)) if window == result_window => count + 1,
                    _ => 1,
                };
                state
                    .unchanged_scrolls
                    .push((thread_id.to_owned(), (result_window, count)));
            } else {
                state
                    .unchanged_scrolls
                    .retain(|(thread, _)| thread != thread_id);
            }
        }
        let mut result = to_map(&outcome.result);
        let scroll_observation = if no_movement {
            let mut observation = json!({
                "status": "no-visible-movement",
                "code": "scroll_noop",
                "message": "No content movement was observed. This may be an edge, a non-scrollable target, or dropped delivery. Inspect fresh state and choose a scrollable element; do not blindly repeat the wheel event.",
            });
            if let Some(traveled) = traveled_y {
                observation["measuredDeltaY"] = json!(traveled);
            }
            observation
        } else {
            match traveled_y {
                Some(traveled) => json!({ "status": "movement-observed", "measuredDeltaY": traveled }),
                None => json!({ "status": "unknown", "reason": "movement_not_measurable" }),
            }
        };
        result.insert("scrollObservation".into(), scroll_observation);
        if let Some(scroll) = limited_scroll(&outcome.result, &request) {
            result.insert("scroll".into(), scroll);
        }
        Ok(ObservedOutcome::Observed {
            result,
            observation: outcome.observation,
        })
    }

    async fn focused_field_state(
        &self,
        window_id: &str,
    ) -> Result<zeron_proto::computer::ComputerState, ComputerError> {
        assert_desktop_operation_active()?;
        self.manager
            .get_state(ComputerStateRequest {
                include_screenshot: Some(false),
                include_text: false,
                include_tree: Some(true),
                window_id: Some(window_id.to_owned()),
            })
            .await
    }

    async fn typed_text_readback(
        &self,
        text: &str,
        result: &ComputerActionResult,
    ) -> Result<Option<String>, ComputerError> {
        let confirmed = result
            .delivery
            .as_ref()
            .is_some_and(|delivery| delivery.verified == ComputerDeliveryVerification::Confirmed);
        let typed = text.trim();
        let Some(window_id) = result.window_id.clone().filter(|_| !confirmed && !typed.is_empty())
        else {
            return Ok(None);
        };
        let mut state = self.focused_field_state(&window_id).await?;
        let still_empty = |state: &zeron_proto::computer::ComputerState| {
            state
                .ax_tree
                .as_ref()
                .and_then(|ax| ax.focused.as_ref())
                .is_some_and(|line| {
                    line.value
                        .as_deref()
                        .is_none_or(|value| value.trim().is_empty())
                })
        };
        if still_empty(&state) {
            tokio::time::sleep(TYPED_TEXT_SETTLE).await;
            state = self.focused_field_state(&window_id).await?;
        }
        let Some(focused) = state.ax_tree.and_then(|ax| ax.focused).filter(|line| {
            matches!(
                line.role.as_str(),
                "AXTextField" | "AXTextArea" | "AXSearchField" | "AXComboBox"
            )
        }) else {
            fn has_text_field(node: &ComputerUiNode) -> bool {
                is_writable_text_node(node) || node.children.iter().any(has_text_field)
            }
            let through_keyboard_focus = result.point.is_none();
            return Ok((through_keyboard_focus
                && state
                    .root
                    .as_ref()
                    .is_some_and(|root| root.children.iter().any(has_text_field)))
                .then(|| "No text field in this window has keyboard focus, so the typed text may have gone nowhere. Click the field you mean, then type and read its value.".to_owned()));
        };
        let value = focused.value.unwrap_or_default();
        if value.contains(typed) {
            return Ok(None);
        }
        if value.trim().is_empty() {
            return Err(CuaActionError::new(
                "The text did not reach the focused text field: it is still empty after typing. Nothing landed, so there is nothing to undo. Click the field (or use setValue on its index) and read the state before typing again; do not press Return on the empty field.",
                CuaEffect::DispatchedUnknown,
            )
            .code(COMPUTER_TEXT_NOT_APPLIED_CODE)
            .into());
        }
        Ok(Some(format!(
            "The focused text field does not show the typed text; its value is {}. Read the state before typing again.",
            serde_json::to_string(&value).unwrap_or_default()
        )))
    }

    pub(super) async fn with_action_element_changes(
        &self,
        result: Map<String, Value>,
        context: &ComputerToolContext,
    ) -> Result<Map<String, Value>, ToolFailure> {
        let Some(window_id) = window_of(&result) else {
            return Ok(result);
        };
        let key = digest_scope_key(&context.chat_id, Some(&window_id), None);
        let Some(before) = self.state().elements.digest(&key) else {
            return Ok(result);
        };
        let read = async {
            assert_desktop_operation_active()?;
            let state = self
                .manager
                .get_state(ComputerStateRequest {
                    include_screenshot: Some(false),
                    include_text: false,
                    include_tree: Some(true),
                    window_id: Some(window_id.clone()),
                })
                .await?;
            assert_desktop_operation_active()?;
            Ok::<_, ComputerError>(state.root)
        };
        match read.await {
            Ok(None) => Ok(result),
            Ok(Some(root)) => {
                let elements = crate::computer::ui_tree_targeting::actionable_elements(
                    &root,
                    &ComputerActionableElementFilter {
                        window_id: Some(window_id),
                        label_contains: None,
                    },
                    self.native_identity(),
                );
                let stable = self
                    .state()
                    .elements
                    .remember_digest(&context.chat_id, &key, elements);
                let mut result = result;
                result.extend(element_change_fields(
                    Some(&before),
                    &stable,
                    Some(ELEMENT_CHANGE_LIMIT),
                ));
                Ok(result)
            }
            Err(_) => {
                assert_desktop_operation_active()?;
                Ok(result)
            }
        }
    }

    pub(super) fn with_observation(
        &self,
        context: &ComputerToolContext,
        mut result: Map<String, Value>,
        capture: Option<ComputerActionObservation>,
    ) -> Handled {
        match capture {
            None => Ok(Value::Object(result).into()),
            Some(ComputerActionObservation::TargetWindowClosed) => {
                result.insert("targetWindowClosed".into(), json!(true));
                result.insert("observationEvidence".into(), json!("target-window-closed"));
                result.insert(
                    "note".into(),
                    json!("The window this action targeted no longer exists — the action likely closed it, so no post-action screenshot was taken. Use computer_list_windows or computer_get_state to see the desktop now."),
                );
                Ok(Value::Object(result).into())
            }
            Some(ComputerActionObservation::Captured(captured)) => {
                let reused = self
                    .state()
                    .frames
                    .match_latest(
                        &context.chat_id,
                        &captured.screenshot,
                        captured.window_id.as_deref(),
                    )
                    .cloned();
                if let Some(reused) = reused {
                    result.insert("observationEvidence".into(), json!("frame-unchanged"));
                    result.insert("screenshotUnchanged".into(), json!(true));
                    result.insert("screenshotId".into(), json!(reused.id));
                    result.insert("screenshot".into(), frame_json(&reused));
                    result.insert("note".into(), json!(UNCHANGED_OBSERVATION_NOTE));
                    return Ok(Value::Object(result).into());
                }
                result.insert("observationEvidence".into(), json!("fresh-frame"));
                Ok(self
                    .deliver_screenshot(
                        &context.chat_id,
                        result,
                        &captured.screenshot,
                        captured.window_id.as_deref(),
                    )?
                    .into())
            }
        }
    }

    async fn observe_after_action(
        &self,
        args: &ToolArgs,
        mut result: Map<String, Value>,
        context: &ComputerToolContext,
    ) -> Handled {
        if read_boolean_arg(args, "include_screenshot")? == Some(false) {
            return Ok(Value::Object(result).into());
        }
        let label = read_verbatim_string_arg(args, "wait_for_label")?;
        let window_id = window_of(&result);
        let readiness = match (&label, &window_id) {
            (None, _) => None,
            (Some(_), None) => Some(json!({ "status": "unavailable", "waitedMs": 0 })),
            (Some(label), Some(window_id)) => {
                let target = ComputerTarget {
                    label: Some(label.clone()),
                    window_id: Some(window_id.clone()),
                    ..ComputerTarget::default()
                };
                let signal = desktop_operation_signal();
                let waited = wait_for_control(
                    || {
                        self.manager.get_state(ComputerStateRequest {
                            include_tree: Some(true),
                            window_id: Some(window_id.clone()),
                            ..ComputerStateRequest::default()
                        })
                    },
                    semantic_lookup,
                    &target,
                    2_000,
                    signal.as_ref(),
                    false,
                )
                .await;
                Some(match waited {
                    Ok(readiness) => Value::Object(readiness_json(&readiness)),
                    Err(error) => {
                        assert_desktop_operation_active()?;
                        json!({ "status": "unavailable", "note": error.to_string() })
                    }
                })
            }
        };
        let settle = readiness.is_none();
        if let Some(readiness) = readiness {
            result.insert("readiness".into(), readiness);
        }
        let capture = self
            .manager
            .capture_action_screenshot(window_id.as_deref(), point_of(&result), settle)
            .await;
        self.with_observation(context, result, capture)
    }
}

pub(super) struct ScrollRequest {
    pub target: ObservedComputerTarget,
    pub has_target: bool,
    pub delta: zeron_proto::computer::ComputerScrollDelta,
    pub limited: zeron_proto::computer::ComputerScrollDelta,
    pub modifiers: Vec<zeron_proto::computer::ComputerInputModifier>,
    pub frame_window: Option<String>,
}

pub(super) fn limited_scroll(
    result: &zeron_proto::computer::ComputerActionResult,
    request: &ScrollRequest,
) -> Option<Value> {
    let scroll = result.scroll.as_ref()?;
    if request.limited.delta_x == request.delta.delta_x
        && request.limited.delta_y == request.delta.delta_y
    {
        return None;
    }
    let mut scroll = to_map(scroll);
    scroll.insert("requested".into(), json_of(&request.delta));
    scroll.insert("limitedTo".into(), json_of(&request.limited));
    Some(Value::Object(scroll))
}

fn frame_json(frame: &crate::computer::screenshot_frames::ScreenshotFrame) -> Value {
    let mut value = json!({
        "screenshotId": frame.id,
        "region": frame.region,
        "width": frame.width,
        "height": frame.height,
        "scale": frame.scale,
    });
    if let Some(window_id) = &frame.window_id {
        value["windowId"] = json!(window_id);
    }
    value
}

pub(super) fn required_window(args: &ToolArgs) -> Result<String, ToolInputError> {
    read_window_id_arg(args)?.ok_or_else(|| ToolInputError::new("window_id is required."))
}

pub(super) fn read_pid(args: &ToolArgs, message: &str) -> Result<u32, ToolInputError> {
    match read_number_arg(args, "pid")? {
        Some(pid) if is_safe_integer(pid) && pid > 0.0 && pid <= f64::from(u32::MAX) => {
            Ok(pid as u32)
        }
        _ => Err(ToolInputError::new(message)),
    }
}

pub(super) async fn pause(duration_ms: u64) -> Result<(), DesktopOperationError> {
    let sleep = tokio::time::sleep(Duration::from_millis(duration_ms));
    match desktop_operation_signal() {
        Some(signal) => tokio::select! {
            () = sleep => Ok(()),
            () = signal.aborted() => Err(DesktopOperationError::Aborted),
        },
        None => {
            sleep.await;
            Ok(())
        }
    }
}
