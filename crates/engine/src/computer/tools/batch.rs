use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Map, Value, json};
use zeron_proto::computer::{
    ComputerAccessibilityStatus, ComputerDeliveryMode, ComputerLaunchWindowStatus, ComputerTarget,
};

use super::args::{
    read_action_name, read_click_gesture, read_clipboard_text, read_delta, read_drag_duration_ms,
    read_key_or_chord, read_menu_path_arg, read_menu_target_arg, read_modifiers,
    read_nested_screenshot_target, read_required_text, read_screenshot_target,
    read_select_text_range, read_select_text_target, read_set_value_value, read_wait_duration_ms,
    read_window_id_arg,
};
use super::desktop::{
    Handled, is_strip_window, json_of, limited_scroll, noted_json, pause, read_pid, readiness_json,
    semantic_lookup, to_map, with_launch_guidance,
};
use super::elements::{RefLookup, digest_scope_key, element_change_fields, listing_fields};
use super::results::{ToolFailure, run_step_error};
use super::tool_input::{
    ToolArgs, ToolInputError, read_boolean_arg, read_record_arg, read_required_string_arg,
    read_string_arg, read_string_array_arg, read_verbatim_string_arg,
};
use super::{
    COMPUTER_RUN_MAX_STEPS, ComputerToolContext, ComputerTools, RUN_CONDITION_FIELDS,
    RUN_STEP_TYPES, run_step_fields,
};
use crate::computer::backend::{
    COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION, ComputerCaptureRequest, ComputerError,
    ComputerLaunchOptions, ComputerTextRange,
};
use crate::computer::cursor_activity::cursor_tool_activity;
use crate::computer::desktop_operation_queue::{
    DesktopOperationError, assert_desktop_operation_active, desktop_operation_signal,
    with_desktop_delivery_mode,
};
use crate::computer::manager::{ComputerScrollOptions, ComputerStateRequest};
use crate::computer::ui_tree_targeting::{
    ComputerActionableElementFilter, ComputerTargetErrorCode, actionable_elements,
    resolve_computer_semantic_target,
};
use crate::computer::wait_for_control::wait_for_control;

type StepFuture<'a> = BoxFuture<'a, Result<Value, ToolFailure>>;

struct PreparedStep<'a> {
    step_type: String,
    if_element: Option<ComputerTarget>,
    unless_element: Option<ComputerTarget>,
    continue_on_error: bool,
    run: StepFuture<'a>,
}

fn aborted() -> bool {
    desktop_operation_signal().is_some_and(|signal| signal.is_aborted())
}

impl ComputerTools {
    fn read_step_element_condition(
        &self,
        step: &ToolArgs,
        name: &str,
        thread_id: &str,
    ) -> Result<Option<ComputerTarget>, ToolFailure> {
        let Some(value) = read_record_arg(step, name)? else {
            return Ok(None);
        };
        let target = self
            .resolve_target(read_screenshot_target(value)?, thread_id)?
            .target;
        if target.label.is_none() {
            return Err(ToolInputError::new(format!(
                "\"{name}\" needs a label, or a ref whose element has one."
            ))
            .into());
        }
        Ok(Some(target))
    }

    async fn element_condition_present(
        &self,
        target: &ComputerTarget,
    ) -> Result<bool, ToolFailure> {
        let state = self
            .manager
            .get_state(ComputerStateRequest {
                include_tree: Some(true),
                window_id: target.window_id.clone(),
                ..ComputerStateRequest::default()
            })
            .await?;
        let unavailable = state.accessibility.as_ref().is_some_and(|report| {
            report.status == ComputerAccessibilityStatus::Unavailable
                || target
                    .window_id
                    .as_ref()
                    .is_some_and(|window_id| report.unavailable_window_ids.contains(window_id))
        });
        let Some(root) = state.root.as_ref().filter(|_| !unavailable) else {
            return Ok(false);
        };
        Ok(
            match resolve_computer_semantic_target(root, target, false) {
                Ok(_) => true,
                Err(error) => error.code == ComputerTargetErrorCode::Ambiguous,
            },
        )
    }

    fn prepare_run_step<'a>(
        &'a self,
        step_type: &str,
        step: &'a ToolArgs,
        context: &'a ComputerToolContext,
    ) -> Result<StepFuture<'a>, ToolFailure> {
        let thread = context.chat_id.as_str();
        let who = Some(thread);
        let manager = &self.manager;
        let action =
            |value: zeron_proto::computer::ComputerActionResult| Value::Object(to_map(&value));
        Ok(match step_type {
            "click" => {
                let target = self.read_target(step, context)?;
                let modifiers = read_modifiers(step)?;
                let gesture = read_click_gesture(step)?;
                async move {
                    Ok(action(
                        manager.click(who, target, &modifiers, gesture).await?,
                    ))
                }
                .boxed()
            }
            "move_cursor" => {
                let target = self.read_target(step, context)?;
                async move { Ok(action(manager.move_cursor(who, target).await?)) }.boxed()
            }
            "drag" => {
                let from =
                    self.resolve_target(read_nested_screenshot_target(step, "from")?, thread)?;
                let to = self.resolve_target(read_nested_screenshot_target(step, "to")?, thread)?;
                let duration = read_drag_duration_ms(step)?;
                async move { Ok(action(manager.drag(who, from, to, Some(duration)).await?)) }
                    .boxed()
            }
            "scroll" => {
                let request = self.scroll_request(step, thread)?;
                async move {
                    let outcome = manager
                        .scroll_calibrated(
                            who,
                            request.has_target.then_some(request.target.clone()),
                            request.limited.delta_x,
                            request.limited.delta_y,
                            ComputerScrollOptions {
                                observe: false,
                                modifiers: request.modifiers.clone(),
                            },
                        )
                        .await?;
                    let mut result = to_map(&outcome.result);
                    if let Some(scroll) = limited_scroll(&outcome.result, &request) {
                        result.insert("scroll".into(), scroll);
                    }
                    Ok(Value::Object(result))
                }
                .boxed()
            }
            "type_text" => {
                let text = read_required_text(step)?;
                let target = self.read_target(step, context)?;
                async move {
                    Ok(action(
                        if target.target.label.is_some() || target.target.role.is_some() {
                            manager.type_text_at(who, &text, target).await?
                        } else {
                            let window = target.target.window_id.clone();
                            manager.type_text(who, &text, window.as_deref()).await?
                        },
                    ))
                }
                .boxed()
            }
            "press_key" => {
                let key = read_key_or_chord(step)?;
                let target = self.read_target(step, context)?;
                async move {
                    let window = target.target.window_id.clone();
                    let exact = (target.target.label.is_some() || target.target.role.is_some())
                        .then_some(target);
                    Ok(action(match key.chord {
                        None => {
                            manager
                                .press_key(who, &key.key, window.as_deref(), exact)
                                .await?
                        }
                        Some(chord) => {
                            manager
                                .hotkey(who, &chord, window.as_deref(), exact)
                                .await?
                        }
                    }))
                }
                .boxed()
            }
            "set_value" => {
                let target = self.read_target(step, context)?;
                let value = read_set_value_value(step)?;
                async move { Ok(action(manager.set_value(who, target, &value).await?)) }.boxed()
            }
            "perform_action" => {
                let target = self.read_target(step, context)?;
                let name = read_action_name(step)?;
                async move { Ok(action(manager.perform_action(who, target, &name).await?)) }.boxed()
            }
            "select_text" => {
                let target = self.resolve_target(read_select_text_target(step)?, thread)?;
                let range: ComputerTextRange = read_select_text_range(step)?;
                async move { Ok(action(manager.select_text(who, target, range).await?)) }.boxed()
            }
            "wait" => {
                let duration = read_wait_duration_ms(step)?;
                let absent = read_boolean_arg(step, "absent")? == Some(true);
                let raw = read_screenshot_target(step)?;
                let semantic = raw.target.element_ref.is_some()
                    || raw.target.label.is_some()
                    || raw.target.role.is_some()
                    || raw.target.ref_ordinal.is_some();
                if semantic {
                    let target = self.resolve_target(raw, thread)?.target;
                    if target
                        .label
                        .as_deref()
                        .is_none_or(|label| label.trim().is_empty())
                    {
                        return Err(ToolInputError::new(
                            "A \"wait\" step with an element target requires a nonempty label or a ref.",
                        )
                        .into());
                    }
                    async move {
                        let signal = desktop_operation_signal();
                        let readiness = wait_for_control(
                            || {
                                manager.get_state(ComputerStateRequest {
                                    include_tree: Some(true),
                                    window_id: target.window_id.clone(),
                                    ..ComputerStateRequest::default()
                                })
                            },
                            semantic_lookup,
                            &target,
                            duration,
                            signal.as_ref(),
                            absent,
                        )
                        .await?;
                        Ok(Value::Object(readiness_json(&readiness)))
                    }
                    .boxed()
                } else {
                    if absent {
                        return Err(ToolInputError::new(
                            "A \"wait\" step with \"absent\" requires an element target.",
                        )
                        .into());
                    }
                    async move {
                        if duration > 0 {
                            pause(duration).await?;
                        }
                        Ok(json!({ "waitedMs": duration }))
                    }
                    .boxed()
                }
            }
            "activate_window" => {
                let window_id = read_window_id_arg(step)?.ok_or_else(|| {
                    ToolInputError::new("Step \"activate_window\" requires \"window_id\".")
                })?;
                async move {
                    let authorization = context.seams.foreground_authorization(Some(step)).await;
                    let noted = with_desktop_delivery_mode(
                        ComputerDeliveryMode::Foreground,
                        manager.foreground_with_restore(who, &window_id, None, Some(authorization)),
                    )
                    .await?;
                    Ok(Value::Object(noted_json(&noted)))
                }
                .boxed()
            }
            "launch_app" => {
                let app = read_required_string_arg(step, "app")?;
                let app_args = read_string_array_arg(step, "arguments")?.unwrap_or_default();
                let wait_ms = if read_boolean_arg(step, "wait_for_window")? == Some(false) {
                    0
                } else {
                    2_000
                };
                let hidden = read_boolean_arg(step, "hidden")?;
                async move {
                    self.refuse_visible_launch(context, hidden).await?;
                    let result = manager
                        .launch_app(
                            who,
                            &app,
                            &app_args,
                            wait_ms,
                            ComputerLaunchOptions { hidden },
                        )
                        .await?;
                    Ok(with_launch_guidance(&result))
                }
                .boxed()
            }
            "write_clipboard" => {
                let text = read_clipboard_text(step)?;
                async move { Ok(action(manager.write_clipboard(who, &text).await?)) }.boxed()
            }
            "paste" => {
                let text = read_clipboard_text(step)?;
                let window_id = read_window_id_arg(step)?;
                async move {
                    let pasted = manager.paste(who, &text, window_id.as_deref()).await?;
                    let mut result = to_map(&pasted.result);
                    result.insert("clipboardRestored".into(), json!(pasted.clipboard_restored));
                    Ok(Value::Object(result))
                }
                .boxed()
            }
            "set_window_frame" => {
                let window_id = read_window_id_arg(step)?.ok_or_else(|| {
                    ToolInputError::new("Step \"set_window_frame\" requires \"window_id\".")
                })?;
                let frame = zeron_proto::computer::ComputerRect {
                    x: read_delta(step, "x")?,
                    y: read_delta(step, "y")?,
                    width: read_delta(step, "width")?,
                    height: read_delta(step, "height")?,
                };
                if frame.width <= 0.0 || frame.height <= 0.0 {
                    return Err(ToolInputError::new(
                        "Step \"set_window_frame\" needs finite geometry and positive width/height.",
                    )
                    .into());
                }
                async move {
                    Ok(action(
                        manager.set_window_frame(who, &window_id, frame).await?,
                    ))
                }
                .boxed()
            }
            "invoke_menu" => {
                let target = read_menu_target_arg(step)?;
                let path = read_menu_path_arg(step, "Step \"invoke_menu\"")?;
                async move {
                    let authorization = context.seams.foreground_authorization(Some(step)).await;
                    Ok(action(
                        manager
                            .invoke_menu(who, target, &path, Some(authorization))
                            .await?,
                    ))
                }
                .boxed()
            }
            "kill_app" => {
                let window_id = read_window_id_arg(step)?.ok_or_else(|| {
                    ToolInputError::new("Step \"kill_app\" requires \"window_id\".")
                })?;
                async move { Ok(action(manager.kill_app(who, &window_id).await?)) }.boxed()
            }
            "set_window_minimized" => {
                let window_id = read_window_id_arg(step)?.ok_or_else(|| {
                    ToolInputError::new("Step \"set_window_minimized\" requires \"window_id\".")
                })?;
                let minimized = read_boolean_arg(step, "minimized")?.ok_or_else(|| {
                    ToolInputError::new(
                        "Step \"set_window_minimized\" requires a boolean \"minimized\".",
                    )
                })?;
                async move {
                    Ok(action(
                        manager
                            .set_window_minimized(who, &window_id, minimized)
                            .await?,
                    ))
                }
                .boxed()
            }
            "set_app_visibility" => {
                let pid = read_pid(
                    step,
                    "Step \"set_app_visibility\" requires a positive integer \"pid\".",
                )?;
                let hidden = read_boolean_arg(step, "hidden")?.ok_or_else(|| {
                    ToolInputError::new(
                        "Step \"set_app_visibility\" requires a boolean \"hidden\".",
                    )
                })?;
                async move {
                    let noted = manager.set_app_visibility(who, pid, hidden).await?;
                    Ok(Value::Object(noted_json(&noted)))
                }
                .boxed()
            }
            "get_state" => {
                let window_id = read_window_id_arg(step)?;
                let label_contains = match read_verbatim_string_arg(step, "label_contains")? {
                    Some(label) => Some(label),
                    None => read_verbatim_string_arg(step, "labelContains")?,
                };
                async move {
                    let state = manager
                        .get_state(ComputerStateRequest {
                            include_tree: Some(true),
                            window_id: window_id.clone(),
                            ..ComputerStateRequest::default()
                        })
                        .await?;
                    let elements = state.root.as_ref().map(|root| {
                        actionable_elements(
                            root,
                            &ComputerActionableElementFilter {
                                window_id: window_id.clone(),
                                label_contains: label_contains.clone(),
                            },
                            self.native_identity(),
                        )
                    });
                    let stable = elements.map(|elements| {
                        self.state().elements.remember_digest(
                            thread,
                            &digest_scope_key(
                                thread,
                                window_id.as_deref(),
                                label_contains.as_deref(),
                            ),
                            elements,
                        )
                    });
                    let mut result = Map::new();
                    if let Some(window_id) = &window_id {
                        result.insert("windowId".into(), json!(window_id));
                    }
                    match &stable {
                        Some(stable) => result.extend(listing_fields(stable)),
                        None => {
                            result.insert("elements".into(), json!([]));
                        }
                    }
                    if let Some(accessibility) = &state.accessibility {
                        result.insert("accessibility".into(), json_of(accessibility));
                    }
                    Ok(Value::Object(result))
                }
                .boxed()
            }
            "verify_state" => {
                let window_id = read_window_id_arg(step)?.ok_or_else(|| {
                    ToolInputError::new("Step \"verify_state\" requires \"window_id\".")
                })?;
                let expect = match step.get("expect") {
                    Some(Value::Array(items))
                        if !items.is_empty()
                            && items.len() <= 8
                            && items.iter().all(Value::is_object) =>
                    {
                        items.clone()
                    }
                    _ => {
                        return Err(ToolInputError::new(
                            "Step \"verify_state\" needs \"expect\" as an array of one to eight predicates.",
                        )
                        .into());
                    }
                };
                async move { Ok(json_of(&manager.verify_state(&window_id, &expect).await?)) }
                    .boxed()
            }
            other => {
                return Err(ToolInputError::new(format!(
                    "Unknown run step type {}.",
                    Value::String(other.to_owned())
                ))
                .into());
            }
        })
    }

    fn prepare_batch<'a>(
        &'a self,
        raw_steps: &'a [Value],
        context: &'a ComputerToolContext,
    ) -> Result<Vec<PreparedStep<'a>>, ToolFailure> {
        let thread = context.chat_id.as_str();
        raw_steps
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let step = entry.as_object().ok_or_else(|| {
                    ToolInputError::new(format!(
                        "Step {index} must be an object with a \"type\" field."
                    ))
                })?;
                let step_type = read_string_arg(step, "type", false)?;
                let fields = step_type.as_deref().and_then(run_step_fields);
                let (Some(step_type), Some(fields)) = (step_type, fields) else {
                    return Err(ToolInputError::new(format!(
                        "Step {index}: \"type\" must be one of {}.",
                        RUN_STEP_TYPES.join(", ")
                    ))
                    .into());
                };
                let unknown: Vec<String> = step
                    .keys()
                    .filter(|key| {
                        *key != "type"
                            && !fields.contains(&key.as_str())
                            && !RUN_CONDITION_FIELDS.contains(&key.as_str())
                    })
                    .map(|key| Value::String(key.clone()).to_string())
                    .collect();
                if !unknown.is_empty() {
                    return Err(ToolInputError::new(format!(
                        "Step {index} ({step_type}): unknown field {}.",
                        unknown.join(", ")
                    ))
                    .into());
                }
                Ok(PreparedStep {
                    if_element: self.read_step_element_condition(step, "if_element", thread)?,
                    unless_element: self.read_step_element_condition(
                        step,
                        "unless_element",
                        thread,
                    )?,
                    continue_on_error: read_boolean_arg(step, "continue_on_error")? == Some(true),
                    run: self.prepare_run_step(&step_type, step, context)?,
                    step_type,
                })
            })
            .collect()
    }

    pub(super) async fn run_batch(
        &self,
        args: &ToolArgs,
        context: &ComputerToolContext,
    ) -> Handled {
        let thread = context.chat_id.as_str();
        let raw_steps = match args.get("steps") {
            Some(Value::Array(steps)) if !steps.is_empty() => steps,
            _ => {
                return Err(ToolInputError::new(
                    "\"steps\" must be a nonempty array of step objects.",
                )
                .into());
            }
        };
        if raw_steps.len() > COMPUTER_RUN_MAX_STEPS {
            return Err(ToolInputError::new(format!(
                "\"steps\" accepts at most {COMPUTER_RUN_MAX_STEPS} steps; got {}. Split the sequence into multiple computer_run calls.",
                raw_steps.len()
            ))
            .into());
        }
        let prepared = self.prepare_batch(raw_steps, context)?;
        let mut steps: Vec<Value> = Vec::new();
        let mut stopped = false;
        let mut stopped_reason = None;
        let mut last_window_id: Option<String> = None;
        let mut failed = false;
        let cursor = self.manager.cursor_activity().clone();
        for (index, prepared_step) in prepared.into_iter().enumerate() {
            assert_desktop_operation_active()?;
            self.assert_turn_active(context)?;
            let skipped = if let Some(target) = &prepared_step.if_element
                && !self.element_condition_present(target).await?
            {
                Some("if_element_absent")
            } else if let Some(target) = &prepared_step.unless_element
                && self.element_condition_present(target).await?
            {
                Some("unless_element_present")
            } else {
                None
            };
            if let Some(reason) = skipped {
                steps.push(json!({
                    "step": index,
                    "type": prepared_step.step_type,
                    "ok": true,
                    "skipped": true,
                    "skippedReason": reason,
                }));
                continue;
            }
            let activity = cursor_tool_activity(&format!("computer_{}", prepared_step.step_type));
            match cursor.during(thread, activity, prepared_step.run).await {
                Ok(value) => {
                    if let Some(window_id) = value.get("windowId").and_then(Value::as_str) {
                        last_window_id = Some(window_id.to_owned());
                    }
                    let result = match value {
                        Value::Object(mut map) => {
                            map.remove("computerId");
                            Value::Object(map)
                        }
                        other => other,
                    };
                    let unusable = prepared_step.step_type == "launch_app"
                        && result.get("windowStatus")
                            == Some(&json_of(&ComputerLaunchWindowStatus::NoUsableWindow));
                    steps.push(json!({
                        "step": index,
                        "type": prepared_step.step_type,
                        "ok": true,
                        "result": result,
                    }));
                    if unusable {
                        stopped = true;
                        stopped_reason = Some("no_usable_window");
                        break;
                    }
                }
                Err(error) => {
                    if aborted() {
                        return Err(DesktopOperationError::Aborted.into());
                    }
                    self.assert_turn_active(context)?;
                    failed = true;
                    if last_window_id.is_none() {
                        last_window_id = self.step_window(&raw_steps[index], thread);
                    }
                    let entry = run_step_error(&error);
                    let uncertain =
                        entry.get("effect").and_then(Value::as_str) == Some("dispatched-unknown");
                    steps.push(json!({
                        "step": index,
                        "type": prepared_step.step_type,
                        "ok": false,
                        "error": entry,
                    }));
                    if !prepared_step.continue_on_error || uncertain {
                        stopped = true;
                        if prepared_step.continue_on_error {
                            stopped_reason = Some("uncertain_effect");
                        }
                        break;
                    }
                }
            }
        }
        let state_fields = self
            .closing_state(thread, last_window_id.as_deref(), failed)
            .await?;
        let skipped_count = steps
            .iter()
            .filter(|entry| entry.get("skipped") == Some(&json!(true)))
            .count();
        let completed = steps
            .iter()
            .filter(|entry| {
                entry.get("ok") == Some(&json!(true)) && entry.get("skipped") != Some(&json!(true))
            })
            .count();
        let mut payload = Map::new();
        payload.insert("computerId".into(), json!(self.manager.computer_id()));
        payload.insert("steps".into(), Value::Array(steps));
        payload.insert("completed".into(), json!(completed));
        if skipped_count > 0 {
            payload.insert("skipped".into(), json!(skipped_count));
        }
        payload.insert("stopped".into(), json!(stopped));
        if let Some(reason) = stopped_reason {
            payload.insert("stoppedReason".into(), json!(reason));
        }
        payload.extend(state_fields);
        if read_boolean_arg(args, "include_screenshot")? != Some(true) {
            return Ok(Value::Object(payload).into());
        }
        let captured = match &last_window_id {
            None => self
                .manager
                .capture_focused_window(Some(COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION), false)
                .await
                .map(|capture| capture.screenshot),
            Some(window_id) => {
                self.manager
                    .capture_screenshot(ComputerCaptureRequest::Window {
                        window_id: window_id.clone(),
                        max_dimension: Some(COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION),
                    })
                    .await
            }
        };
        match captured {
            Ok(screenshot) => Ok(self
                .deliver_screenshot(thread, payload, &screenshot, last_window_id.as_deref())?
                .into()),
            Err(error) => {
                if aborted() {
                    return Err(DesktopOperationError::Aborted.into());
                }
                payload.insert("screenshotError".into(), json!(error.to_string()));
                Ok(Value::Object(payload).into())
            }
        }
    }

    fn step_window(&self, step: &Value, thread: &str) -> Option<String> {
        if let Some(window_id) = step.get("window_id").and_then(Value::as_str) {
            return Some(window_id.to_owned());
        }
        let element_ref = step.get("ref").and_then(Value::as_u64)?;
        match self.state().elements.lookup_ref(thread, element_ref) {
            RefLookup::Found(entry) => entry.window_id,
            _ => None,
        }
    }

    async fn closing_state(
        &self,
        thread: &str,
        last_window_id: Option<&str>,
        failed: bool,
    ) -> Result<Map<String, Value>, ToolFailure> {
        let read = async {
            assert_desktop_operation_active()?;
            let mut state = self
                .manager
                .get_state(ComputerStateRequest {
                    include_screenshot: Some(false),
                    include_text: false,
                    include_tree: Some(true),
                    window_id: last_window_id.map(str::to_owned),
                })
                .await?;
            assert_desktop_operation_active()?;
            Ok::<_, ComputerError>({
                let root = state.root.take();
                state.text = None;
                state.screenshot = None;
                (state, root)
            })
        };
        match read.await {
            Ok((mut state, root)) => {
                match last_window_id {
                    Some(window_id) => state.windows.retain(|window| window.id == window_id),
                    None => state.windows.retain(|window| !is_strip_window(window)),
                }
                let elements = root.as_ref().map(|root| {
                    actionable_elements(
                        root,
                        &ComputerActionableElementFilter {
                            window_id: last_window_id.map(str::to_owned),
                            label_contains: None,
                        },
                        self.native_identity(),
                    )
                });
                let key = digest_scope_key(thread, last_window_id, None);
                let (before, stable) = {
                    let mut tool_state = self.state();
                    let before = tool_state.elements.digest(&key);
                    let stable = elements.map(|elements| {
                        tool_state.elements.remember_digest(thread, &key, elements)
                    });
                    (before, stable)
                };
                let mut fields = Map::new();
                if last_window_id.is_some()
                    && let (Some(before), Some(stable)) = (&before, &stable)
                {
                    fields.extend(element_change_fields(Some(before), stable, Some(40)));
                }
                let mut state_json = to_map(&state);
                if failed && last_window_id.is_none() {
                    state_json.remove("windows");
                }
                if let Some(stable) = &stable {
                    state_json.extend(listing_fields(stable));
                }
                fields.insert("state".into(), Value::Object(state_json));
                Ok(fields)
            }
            Err(error) => {
                if aborted() {
                    return Err(DesktopOperationError::Aborted.into());
                }
                let mut fields = Map::new();
                fields.insert("stateError".into(), json!(error.to_string()));
                Ok(fields)
            }
        }
    }
}
