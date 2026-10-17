use serde_json::{Map, Value, json};
use zeron_proto::computer::{
    COMPUTER_DRAG_MAX_DURATION_MS, COMPUTER_MODIFIERS_MAX_ITEMS, COMPUTER_SELECT_TEXT_RANGE_MAX,
    COMPUTER_TEXT_MAX_LENGTH, COMPUTER_WAIT_MAX_MS, McpToolDefinition,
};

use super::guidance::computer_help_topics;
use super::{
    COMPUTER_INSPECTION_TOOL_NAMES, COMPUTER_RUN_MAX_STEPS, ComputerToolEntry, RUN_STEP_TYPES,
};
use crate::computer::backend::{ComputerAgentDialect, DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION};

pub(super) const DEFAULT_DRAG_DURATION_MS: f64 = 250.0;

const SCREENSHOT_FRAME_NOTE: &str = "Screenshots include screenshotId and pixel width/height; pass x/y as pixel coordinates in that image from its top-left corner. The server maps them onto the desktop.";
pub(super) const SHARED_CLIPBOARD_NOTE: &str = "The desktop has a single clipboard shared with the human user, not a private one for the agent.";
const POINTER_COORDINATE_HINT: &str =
    "x/y are pixels in the received screenshot, never desktop coordinates.";
const SEMANTIC_TARGETING_NOTE: &str =
    "Prefer label and role from computer_get_state over estimated x/y.";
const ACTION_SCREENSHOT_HINT: &str =
    "Returns a screenshot of the affected window unless include_screenshot:false.";
const WINDOW_FOCUS_NOTE: &str = "focused: agent target; keyboardFocused: app keyboard window; active: native activation. Absent fields mean unknown.";
const KEYBOARD_TARGET_HINT: &str = "Pass window_id or use the last aimed window; hover does not aim keys. Use a field ref or label to disambiguate text targets.";
const DELIVERY_HINT: &str = "delivery.verified and delivery.effect report evidence, not retry permission — never replay an uncertain action.";

pub(super) fn read_only_annotations(title: &str) -> Value {
    json!({
        "title": title,
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false,
    })
}

pub(super) fn write_annotations(title: &str) -> Value {
    json!({
        "title": title,
        "readOnlyHint": false,
        "destructiveHint": true,
        "idempotentHint": false,
        "openWorldHint": false,
    })
}

fn gated_read_annotations(title: &str) -> Value {
    json!({
        "title": title,
        "readOnlyHint": false,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false,
    })
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn merged(base: Map<String, Value>, extra: Map<String, Value>) -> Map<String, Value> {
    let mut out = base;
    out.extend(extra);
    out
}

fn entry(name: &str, description: String, input_schema: Value, annotations: Value) -> ComputerToolEntry {
    ComputerToolEntry {
        definition: McpToolDefinition {
            name: name.to_owned(),
            description,
            input_schema,
            annotations: Some(annotations),
        },
        discovery_only: false,
    }
}

fn hidden(mut entry: ComputerToolEntry) -> ComputerToolEntry {
    entry.discovery_only = true;
    entry
}

fn action_entry(
    name: &str,
    title: &str,
    description: String,
    input_schema: Value,
    annotations: Option<Value>,
) -> ComputerToolEntry {
    let mut schema = object(input_schema);
    let mut properties = schema
        .remove("properties")
        .map(object)
        .unwrap_or_default();
    let menu = name == "computer_invoke_menu";
    properties.insert(
        "delivery_mode".into(),
        json!({
            "type": "string",
            "enum": if menu { json!(["foreground"]) } else { json!(["background", "foreground"]) },
            "description": if menu {
                "This menu route is always foreground and requires the user's explicit visible-use request."
            } else {
                "Background by default. Foreground requires the user's own task to ask to see the screen. Never replay uncertain input."
            },
        }),
    );
    schema.insert("properties".into(), Value::Object(properties));
    entry(
        name,
        description,
        Value::Object(schema),
        annotations.unwrap_or_else(|| write_annotations(title)),
    )
}

fn include_action_screenshot_property() -> Map<String, Value> {
    object(json!({
        "include_screenshot": {
            "type": "boolean",
            "description": "Post-action screenshot, default true. For a short sequence, use false then verify with fresh state or a final screenshot.",
        },
    }))
}

fn with_action_screenshot_schema(schema: Value) -> Value {
    let mut schema = object(schema);
    let properties = schema
        .remove("properties")
        .map(object)
        .unwrap_or_default();
    let mut properties = merged(properties, include_action_screenshot_property());
    properties.insert(
        "wait_for_label".into(),
        json!({
            "type": "string",
            "description": "Wait up to 2 seconds for this label in the affected window before capturing.",
        }),
    );
    schema.insert("properties".into(), Value::Object(properties));
    Value::Object(schema)
}

fn observed_action_entry(
    name: &str,
    title: &str,
    description: String,
    input_schema: Value,
    annotations: Option<Value>,
) -> ComputerToolEntry {
    action_entry(
        name,
        title,
        format!("{description} {ACTION_SCREENSHOT_HINT}"),
        with_action_screenshot_schema(input_schema),
        annotations,
    )
}

fn keyboard_target_property() -> Map<String, Value> {
    object(json!({
        "window_id": {
            "type": "string",
            "description": "Exact target window from computer_list_windows; does not activate it. Screenshot is scoped to it.",
        },
    }))
}

fn text_target_property() -> Map<String, Value> {
    merged(
        keyboard_target_property(),
        object(json!({
            "label": {
                "type": "string",
                "description": "Writable label from computer_get_state, scoped by window_id without activation.",
            },
            "role": { "type": "string", "description": "Role to disambiguate the field label." },
            "ref": {
                "type": "integer",
                "minimum": 0,
                "description": "Text control ref from computer_get_state.",
            },
            "ref_ordinal": {
                "type": "integer",
                "minimum": 0,
                "description": "With label: zero-based duplicate index when no ref is given.",
            },
        })),
    )
}

fn modifiers_property() -> Map<String, Value> {
    object(json!({
        "modifiers": {
            "type": "array",
            "items": { "type": "string", "enum": ["ctrl", "alt", "shift", "meta"] },
            "maxItems": COMPUTER_MODIFIERS_MAX_ITEMS,
            "description": "Held through the gesture, then released; \"meta\" is Command on macOS.",
        },
    }))
}

fn screenshot_id_property() -> Map<String, Value> {
    object(json!({
        "screenshot_id": {
            "type": "string",
            "description": "Frame for x/y; defaults to the latest screenshot. An earlier one must still be valid.",
        },
    }))
}

fn target_properties() -> Map<String, Value> {
    let mut properties = object(json!({
        "x": { "type": "number", "description": "Pixel x in the screenshot." },
        "y": { "type": "number", "description": "Pixel y in the screenshot." },
    }));
    properties.extend(screenshot_id_property());
    properties.extend(object(json!({
        "label": {
            "type": "string",
            "description": "Exact label from computer_get_state; matched verbatim against fresh state, including surrounding spaces.",
        },
        "role": { "type": "string", "description": "Optional role used to disambiguate the label." },
        "ref": {
            "type": "integer",
            "minimum": 0,
            "description": "Ref from computer_get_state; distinguishes duplicates and stays bound to that element, never another.",
        },
        "ref_ordinal": {
            "type": "integer",
            "minimum": 0,
            "description": "With label: zero-based index among same-labelled controls, when no ref is given.",
        },
        "window_id": {
            "type": "string",
            "description": "Exact window for label or x/y targeting; outside coordinates are refused. For computer_scroll, window_id alone targets the window.",
        },
    })));
    properties
}

fn object_schema(properties: Map<String, Value>, required: &[&str]) -> Value {
    let mut schema = object(json!({ "type": "object", "additionalProperties": false }));
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), json!(required));
    }
    Value::Object(schema)
}

fn perform_action_note(dialect: ComputerAgentDialect) -> &'static str {
    match dialect {
        ComputerAgentDialect::Macos => "Cua performs named AX actions through a freshly resolved element token: press (or the legacy AXPress spelling) activates the control, open performs AXOpen, show_menu/menu perform AXShowMenu, and pick, confirm and cancel perform their namesakes. Any other action the element lists under Secondary Actions is performed by those printed words or by its raw AX name. Every name past press dispatches only when the element advertises that AX action in a fresh snapshot; otherwise the call refuses and nothing is submitted.",
        ComputerAgentDialect::Linux => "This desktop supports \"activate\" and \"click\".",
    }
}

fn perform_action_argument_note(dialect: ComputerAgentDialect) -> &'static str {
    match dialect {
        ComputerAgentDialect::Macos => "\"press\" (AXPress), \"open\" (AXOpen), \"show_menu\"/\"menu\" (AXShowMenu), \"pick\", \"confirm\", \"cancel\", or any Secondary Action the element lists (its printed words or raw AX name); use the exact window and its fresh accessibility snapshot, and expect a refusal when the element does not advertise the action.",
        ComputerAgentDialect::Linux => "Use \"activate\" or \"click\".",
    }
}

fn select_text_note(dialect: ComputerAgentDialect) -> &'static str {
    match dialect {
        ComputerAgentDialect::Macos => "Cua writes AXSelectedTextRange on a freshly resolved element token and verifies the selection by native read-back. A target without a settable selection attribute refuses before dispatch — nothing falls back to triple-click or select-all, and an uncertain result is never replayed. Label and role come from computer_get_state; pass window_id alone when the window holds exactly one writable text control.",
        ComputerAgentDialect::Linux => "This desktop exposes no native range-selection write, so the call refuses rather than approximating the selection with triple-click, select-all, or a pointer drag.",
    }
}

fn hotkey_form_note(dialect: ComputerAgentDialect) -> &'static str {
    match dialect {
        ComputerAgentDialect::Macos => "A chord is one or more modifiers plus exactly one other key, pressed together and released together — \"meta+s\" to save, \"meta+shift+z\" to redo. More than one non-modifier key is refused; to press two shortcuts, call this twice.",
        ComputerAgentDialect::Linux => "A chord presses every key in the order given, holds them, then releases in reverse — \"ctrl+s\" to save, \"ctrl+shift+z\" to redo. It is not a sequence of separate keystrokes: to press two shortcuts, call this twice.",
    }
}

fn key_argument_note(dialect: ComputerAgentDialect) -> &'static str {
    match dialect {
        ComputerAgentDialect::Macos => "One key name — \"enter\", \"escape\", \"tab\", \"backspace\", \"delete\" (forward delete), \"home\", \"end\", \"pageup\", \"pagedown\", an arrow (\"arrowdown\" or \"down\"), \"f1\"-\"f12\", a modifier (\"command\", \"shift\", \"option\", \"ctrl\", \"fn\", \"capslock\"), or a single printable character — or one chord joined with \"+\", modifiers first: \"meta\" (Command), \"ctrl\", \"alt\" (Option), \"shift\" or \"fn\", then exactly one other key, as in \"meta+s\" or \"meta+shift+z\". xdotool spellings such as \"page_up\" and \"caps_lock\" are accepted. \"insert\" is refused — macOS has no Insert key; \"kp_*\" keypad keys, \"f13\"-\"f20\", \"menu\" and \"help\" need the extended keymap.",
        ComputerAgentDialect::Linux => "One key name — \"enter\", \"escape\", \"tab\", \"backspace\", \"delete\", \"home\", \"end\", \"pageup\", \"pagedown\", an arrow, \"f1\"-\"f12\", a modifier, or a single printable character — or one chord joined with \"+\", modifiers first: \"ctrl\", \"alt\", \"shift\" or \"meta\" (Super), then the key they apply to, as in \"ctrl+s\" or \"ctrl+shift+z\".",
    }
}

fn launch_app_note(dialect: ComputerAgentDialect) -> &'static str {
    match dialect {
        ComputerAgentDialect::Macos => "Names an application the way macOS does. A normal launch requests no foreground activation and keeps its windows available for background input. hidden:true explicitly hides the app and may create no usable window. Reuse the returned process/window; never kill or relaunch it merely because readiness is delayed.",
        ComputerAgentDialect::Linux => "Names an executable on PATH or a desktop application id.",
    }
}

fn launch_app_argument_note(dialect: ComputerAgentDialect) -> &'static str {
    match dialect {
        ComputerAgentDialect::Macos => "The application: its name as shown in the Applications folder (\"Safari\", \"Visual Studio Code\"), its bundle identifier (\"com.apple.Safari\"). The result reports what the name resolved to.",
        ComputerAgentDialect::Linux => "The application: an executable name on PATH (\"firefox\"), a desktop application id (\"org.mozilla.firefox\"), or an absolute path to an executable. The result reports what the name resolved to.",
    }
}

fn window_list_completeness_note(dialect: ComputerAgentDialect) -> &'static str {
    match dialect {
        ComputerAgentDialect::Macos => " If the result carries a setupRequired note about a screen-capture grant, this list is also incomplete: without that grant macOS withholds window titles, and an untitled off-screen window cannot be addressed and is left out — so minimized and other-Space windows disappear from it. What it does report is accurate.",
        ComputerAgentDialect::Linux => "",
    }
}

fn drag_limit_note(dialect: ComputerAgentDialect) -> &'static str {
    match dialect {
        ComputerAgentDialect::Macos => "On macOS an exact-target drag uses the driver's window-local background delivery. Surfaces that drop background events report an unverifiable result: observe the result and never replay the drag or switch to foreground automatically. The duration is limited to 10 seconds and both endpoints must stay inside the exact target window. Verify the drop from the returned screenshot.",
        ComputerAgentDialect::Linux => "This desktop injects the drag at screen coordinates, so it works for anything the pointer can sweep — selecting text, moving a slider — but cross-application drag-and-drop and dragging a window by its titlebar are handled by the compositor and may not follow. Check the result with computer_screenshot rather than assuming the drop landed.",
    }
}

pub(super) fn desktop_entries(dialect: ComputerAgentDialect) -> Vec<ComputerToolEntry> {
    let macos = dialect == ComputerAgentDialect::Macos;
    let overview_scope = if macos {
        "the primary display, or the exact window when window_id is supplied"
    } else {
        "the desktop workspace across all monitors"
    };
    let capture_target_note = if macos {
        "Capture an exact window by \"window_id\" from computer_list_windows. Rectangular region capture is unavailable on this backend. With no arguments it captures the selected or focused window."
    } else {
        "With no arguments it captures the window that currently has focus. Otherwise capture a single window by \"window_id\" from computer_list_windows, or a rectangle given as \"x\", \"y\", \"width\" and \"height\" in pixels of the screenshot you are zooming into (the most recent one, or the one named by screenshot_id); never pass both forms. Region capture is clipped to the desktop workspace."
    };
    let pointer = target_properties();
    let text_target = text_target_property();
    let target_schema = object_schema(pointer.clone(), &[]);
    let mut click_properties = pointer.clone();
    click_properties.extend(modifiers_property());
    click_properties.extend(object(json!({
        "count": {
            "type": "integer",
            "minimum": 1,
            "maximum": 3,
            "description": "1 plain; 2 double-click (open item/select word); 3 triple-click (select line/paragraph). A native multi-click gesture, never separate clicks; unsupported gestures are refused.",
        },
        "button": {
            "type": "string",
            "enum": ["left", "right", "middle"],
            "description": "\"left\" supports every count; \"right\" opens the context menu at count 1. Other pairs are refused.",
        },
    })));
    let mut screenshot_properties = object(json!({
        "window_id": {
            "type": "string",
            "description": if macos {
                "Exact window id from computer_list_windows. Omit to capture the selected or focused window."
            } else {
                "Window id from computer_list_windows. Mutually exclusive with x/y/width/height. Omit both forms to capture the focused window."
            },
        },
    }));
    if !macos {
        screenshot_properties.extend(object(json!({
            "x": { "type": "number", "description": "Region left edge, in pixels of the screenshot being zoomed into (the most recent one, or the one named by screenshot_id)." },
            "y": { "type": "number", "description": "Region top edge, in pixels of the same screenshot." },
            "width": { "type": "number", "description": "Region width in pixels of the same screenshot." },
            "height": { "type": "number", "description": "Region height in pixels of the same screenshot." },
        })));
        screenshot_properties.extend(screenshot_id_property());
    }
    screenshot_properties.insert(
        "max_dimension".into(),
        json!({
            "type": "integer",
            "minimum": 1,
            "maximum": DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION,
            "description": format!(
                "Longest screenshot side in pixels before downscaling. Defaults to and is capped at {DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION}, which is the largest image that reaches you unaltered — ask for more and the picture you see would no longer be the picture your coordinates are mapped against. {}",
                if macos {
                    "Use computer_get_state with window_id and include_text for accessible text that is too small to read."
                } else {
                    "To read finer detail, capture a smaller region rather than a bigger image."
                }
            ),
        }),
    );
    let mut wait_properties = object(json!({
        "duration_ms": {
            "type": "integer",
            "minimum": 0,
            "maximum": COMPUTER_WAIT_MAX_MS,
            "description": format!("How long to wait, in milliseconds. Clamped to {COMPUTER_WAIT_MAX_MS}."),
        },
        "label": { "type": "string", "description": "The next control's label to wait for. Requires window_id." },
        "role": { "type": "string", "description": "Optional role to distinguish controls with the same label." },
        "window_id": { "type": "string", "description": "Window to observe without raising or activating it." },
        "settle": {
            "type": "boolean",
            "description": "Wait for the window's accessibility surface to go quiet instead of for a named control. Requires window_id; cannot combine with label.",
        },
    }));
    wait_properties.extend(include_action_screenshot_property());
    let mut scroll_properties = pointer.clone();
    scroll_properties.extend(modifiers_property());
    scroll_properties.extend(object(json!({
        "delta_x": {
            "type": "number",
            "description": "Horizontal scroll distance in screenshot pixels; defaults to 0. Positive scrolls toward the right of the content.",
        },
        "delta_y": {
            "type": "number",
            "description": "Vertical scroll distance in screenshot pixels; defaults to 0. Positive scrolls toward the end of the content, the way a wheel notch pulled downward does.",
        },
    })));
    let mut type_properties = object(json!({
        "text": { "type": "string", "description": "The exact text to insert at the caret." },
    }));
    type_properties.extend(text_target.clone());
    let mut key_properties = object(json!({
        "key": { "type": "string", "description": key_argument_note(dialect) },
    }));
    key_properties.extend(text_target.clone());
    let mut paste_properties = object(json!({
        "text": { "type": "string", "description": "The exact text to paste at the caret." },
        "format": { "type": "string", "enum": ["text", "md", "html"] },
    }));
    paste_properties.extend(keyboard_target_property());
    let mut set_value_properties = pointer.clone();
    set_value_properties.insert(
        "value".into(),
        json!({ "type": "string", "description": "The control's complete new value." }),
    );
    let mut perform_action =
        json!({ "type": "string", "description": perform_action_argument_note(dialect) });
    if let ComputerAgentDialect::Linux = dialect {
        perform_action["enum"] = json!(["activate", "click"]);
    }
    let mut perform_properties = pointer.clone();
    perform_properties.insert("action".into(), perform_action);
    let mut select_properties = text_target.clone();
    select_properties.extend(object(json!({
        "start": {
            "type": "integer",
            "minimum": 0,
            "maximum": COMPUTER_SELECT_TEXT_RANGE_MAX,
            "description": "Zero-based offset where the selection begins, counted in UTF-16 code units of the element's complete value as computer_get_state's text shows it (untrimmed; the elements list clamps values). Most characters count 1; an emoji or other character outside the Basic Multilingual Plane counts 2, and a range that splits one is refused. A start past the end is refused rather than clamped.",
        },
        "length": {
            "type": "integer",
            "minimum": 0,
            "maximum": COMPUTER_SELECT_TEXT_RANGE_MAX,
            "description": "Number of UTF-16 code units to select; 0 collapses the selection to a caret at start. A range running past the element's end is refused rather than clamped. The result's value is the text the range covered; compare it with the text you meant to select before typing over it.",
        },
        "text": {
            "type": "string",
            "description": "Instead of start/length, the exact text to find in the element's current value. It must occur exactly once after prefix/suffix narrowing, or the call refuses.",
        },
        "prefix": { "type": "string", "description": "Text that must immediately precede the match." },
        "suffix": { "type": "string", "description": "Text that must immediately follow the match." },
        "selection": {
            "type": "string",
            "enum": ["text", "cursor_before", "cursor_after"],
            "description": "Select the matched text (default) or place a caret before or after it.",
        },
    })));
    let rect_properties = |window: &str| {
        object(json!({
            "window_id": { "type": "string", "description": window },
            "x": { "type": "number" },
            "y": { "type": "number" },
            "width": { "type": "number" },
            "height": { "type": "number" },
        }))
    };
    let mut help_topics: Vec<&str> = computer_help_topics().collect();
    help_topics.push("all");

    vec![
        entry(
            "computer_list_windows",
            format!(
                "List windows topmost-first with bounds, stackingIndex and occludedBy. app filters by exact appName, ignoring case. Strips (hidden or off-screen, at most 40pt tall, or zero-area) are omitted unless include_strips. Pass window_id to scope input; selection does not activate it. {WINDOW_FOCUS_NOTE} Use computer_activate_window within task consent when needed; never replay uncertain input.{}",
                window_list_completeness_note(dialect)
            ),
            object_schema(
                object(json!({
                    "app": { "type": "string" },
                    "include_strips": { "type": "boolean" },
                })),
                &[],
            ),
            read_only_annotations("List computer windows"),
        ),
        entry(
            "computer_get_state",
            format!(
                "Read labeled controls and values before acting; prefer semantic targets. {WINDOW_FOCUS_NOTE} Returns elements (no image) with stable refs that distinguish duplicates; use ref on later actions. A one-window listing hoists its id to elementWindowId. window_id adds tree: the window's hierarchy and values numbered by ref; repeat reads return a diff unless full:true. include_screenshot adds {overview_scope} (or that window). {SCREENSHOT_FRAME_NOTE} Narrow elementsTruncated/elementsOmitted results with window_id or label_contains."
            ),
            object_schema(
                object(json!({
                    "include_screenshot": {
                        "type": "boolean",
                        "description": format!("Add a downscaled image of {overview_scope} for x/y targeting or visual context. Default false."),
                    },
                    "include_text": {
                        "type": "boolean",
                        "description": "Add full accessibility text only when elements are insufficient. Default false; can be large.",
                    },
                    "window_id": {
                        "type": "string",
                        "description": if macos {
                            "Select this exact window for accessibility inspection and any requested screenshot. Without window_id, Cua returns window metadata but no application accessibility tree."
                        } else {
                            "Restrict the elements list to controls in this window (from computer_list_windows). The windows, screen size and screenshot are unaffected."
                        },
                    },
                    "label_contains": {
                        "type": "string",
                        "description": "Filter elements by case-insensitive label substring, including truncated results.",
                    },
                    "full": { "type": "boolean" },
                    "diff": {
                        "type": "boolean",
                        "description": "Return elementChanges since this scope's last read (same window_id and label_contains); the first lists all as added.",
                    },
                })),
                &[],
            ),
            read_only_annotations("Get computer state"),
        ),
        entry(
            "computer_screenshot",
            format!(
                "Zoom into one part of the desktop when detail is too small to read in a screenshot you have. {capture_target_note} {SCREENSHOT_FRAME_NOTE} A window that is off-screen or unmeasurable is refused rather than answered with pixels it cannot place; bring it forward with computer_activate_window first if the user asked to see it."
            ),
            object_schema(screenshot_properties, &[]),
            read_only_annotations("Capture computer screenshot"),
        ),
        entry(
            "computer_get_screen_size",
            "Read the logical screen dimensions of the desktop workspace. Informational only: pointer tools take pixel coordinates in a screenshot, not screen coordinates.".into(),
            object_schema(Map::new(), &[]),
            read_only_annotations("Get screen size"),
        ),
        entry(
            "computer_wait",
            format!(
                "Wait for delayed content without sending input or changing focus. Prefer label plus window_id when you know the next control: duration_ms is then a maximum, and the tool returns as soon as that unique control appears, with a screenshot by default. Target it by label afterward so a layout change cannot leave stale coordinates. An unavailable accessibility tree returns immediately; use the screenshot and do not repeat semantic waits until the environment changes. Or pass settle:true plus window_id to return as soon as the window's accessibility surface stops changing — duration_ms is again the maximum — or after a fixed pause reported as mode:\"fixed\" when it cannot be watched. Without label or settle this is a fixed pause with no screenshot. Never repeat the preceding action merely because a page is still loading. Waiting is capped at {COMPUTER_WAIT_MAX_MS} ms; one accessibility read may finish after the deadline."
            ),
            object_schema(wait_properties, &["duration_ms"]),
            read_only_annotations("Wait"),
        ),
        hidden(entry(
            "computer_read_clipboard",
            format!(
                "Read the desktop clipboard as text, returned as \"value\". {SHARED_CLIPBOARD_NOTE} It returns whatever was copied last by anyone, so it may hold something the user copied for their own purposes. An empty clipboard returns an empty string; a clipboard holding an image, other non-text content, or more than {COMPUTER_TEXT_MAX_LENGTH} characters of text is an error."
            ),
            object_schema(Map::new(), &[]),
            gated_read_annotations("Read computer clipboard"),
        )),
        action_entry(
            "computer_launch_app",
            "Launch computer app",
            format!(
                "Launch an application. {} Checks briefly for one unambiguous usable window and returns its id without a screenshot. windowStatus distinguishes readiness from launch delivery. no_usable_window must not trigger automatic relaunches.",
                launch_app_note(dialect)
            ),
            object_schema(
                object(json!({
                    "app": { "type": "string", "description": launch_app_argument_note(dialect) },
                    "wait_for_window": {
                        "type": "boolean",
                        "description": "Wait up to 2 seconds for one matching window. Defaults to true.",
                    },
                    "arguments": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Arguments passed to the application, such as a file path to open. Omit for a plain launch.",
                    },
                    "hidden": {
                        "type": "boolean",
                        "description": "Explicitly hide the launched app. Defaults to false on macOS: its windows remain available for background input without requesting activation. true may prevent the app from creating a usable window. This option never authorizes foreground input.",
                    },
                })),
                &["app"],
            ),
            None,
        ),
        entry(
            "computer_list_apps",
            "List running applications with pid, name, bundle id and active state. Use it to find the app that owns a window, or to confirm an app is running before launching it again.".into(),
            object_schema(Map::new(), &[]),
            read_only_annotations("List computer apps"),
        ),
        entry(
            "computer_verify_state",
            "Assert what the exact window looks like right now without changing anything: element exists / enabled / selected / value_equals matched by role or label_contains, or window bounds within a pixel tolerance. Pass one to eight predicates, combined with AND. Returns a tri-state status — satisfied, unsatisfied, or unknown — plus the per-predicate evidence; unknown means the check could not be proven either way, not that it failed. Use it to prove an action's effect before continuing, or to check a control's state without touching it.".into(),
            object_schema(
                object(json!({
                    "window_id": { "type": "string", "description": "The exact window to inspect." },
                    "expect": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 8,
                        "items": { "type": "object" },
                        "description": "Predicates such as {\"element\":{\"selector\":{\"role\":\"AXButton\",\"label_contains\":\"Save\"},\"enabled\":true,\"exists\":true,\"value_equals\":null,\"selected\":null}} or {\"window\":{\"bounds\":{\"x\":0,\"y\":0,\"width\":800,\"height\":600,\"tolerance_px\":4}}}.",
                    },
                })),
                &["window_id", "expect"],
            ),
            read_only_annotations("Verify computer state"),
        ),
        hidden(entry(
            "computer_zoom",
            "Capture a magnified JPEG of a rect inside the exact window — for reading small text or dense UI that the window screenshot downscales away. x/y/width/height are window-local points: (0,0) is the window's top-left and the window's width/height come from computer_list_windows or get_state.".into(),
            object_schema(
                rect_properties("The exact window to magnify."),
                &["window_id", "x", "y", "width", "height"],
            ),
            read_only_annotations("Zoom into a window region"),
        )),
        hidden(entry(
            "computer_get_accessibility_tree",
            "Return the driver's lightweight desktop inventory — running apps and their on-screen windows with pid, title and window id — the fast discovery read that works before any OS grant is given. Pass window_id to scope the answer to the app that owns that exact window. It carries no control elements: computer_get_state stays the heavier per-window elements digest.".into(),
            object_schema(
                object(json!({
                    "window_id": {
                        "type": "string",
                        "description": "Exact window from computer_list_windows; scopes the snapshot to the app that owns it.",
                    },
                })),
                &[],
            ),
            read_only_annotations("List desktop apps and windows"),
        )),
        hidden(entry(
            "computer_get_cursor_position",
            "Read the human cursor's current position in desktop points — top-left origin, the same coordinate space computer_list_windows reports bounds in. Pure read: it never moves the pointer. Pass window_id to also learn whether the point lies inside that window's bounds.".into(),
            object_schema(
                object(json!({
                    "window_id": {
                        "type": "string",
                        "description": "Exact window from computer_list_windows; adds whether the cursor is inside its bounds.",
                    },
                })),
                &[],
            ),
            read_only_annotations("Read cursor position"),
        )),
        entry(
            "computer_inspect",
            "Inspect clipboard, a magnified window region, desktop inventory or cursor position. Read computer_help with the selected tool for its argument schema. Clipboard reads require task approval.".into(),
            object_schema(
                object(json!({
                    "tool": { "type": "string", "enum": COMPUTER_INSPECTION_TOOL_NAMES },
                    "arguments": { "type": "object", "description": "Arguments from that tool's schema; default {}." },
                })),
                &["tool"],
            ),
            gated_read_annotations("Inspect the computer"),
        ),
        entry(
            "computer_help",
            "Read Computer guidance: no arguments lists chapters; topic reads one. Pass tool instead for its exact schema and computer_run or computer_inspect route. Lookup does not register a hidden tool with your provider.".into(),
            object_schema(
                object(json!({
                    "topic": {
                        "type": "string",
                        "enum": help_topics,
                        "description": "Which chapter to read. Omit for the index of chapters; \"all\" reads every chapter.",
                    },
                    "tool": {
                        "type": "string",
                        "description": "Exact desktop tool name, such as \"computer_invoke_menu\". Use instead of topic.",
                    },
                })),
                &[],
            ),
            read_only_annotations("Computer playbook"),
        ),
        hidden(action_entry(
            "computer_set_window_frame",
            "Set window frame",
            format!(
                "Move and resize the exact window to x/y/width/height in desktop coordinates — the same space computer_list_windows reports bounds in. The new frame is read back and reported verified only when it matches; an unconfirmed result means the window may not have moved, so observe before relying on it. {DELIVERY_HINT}"
            ),
            object_schema(
                rect_properties("The exact window to move or resize."),
                &["window_id", "x", "y", "width", "height"],
            ),
            None,
        )),
        hidden(action_entry(
            "computer_invoke_menu",
            "Invoke menu item",
            "Invoke an exact menu-bar path, e.g. [\"File\", \"Save\"]. This native route activates the app: it requires the user's explicit visible-use request, like computer_activate_window, even in computer_run. Name window_id or app/pid (for an app with no windows). One to six levels; missing or disabled items refuse. Observe afterwards; dispatch does not prove the command succeeded.".into(),
            object_schema(
                object(json!({
                    "window_id": { "type": "string", "description": "A window owned by the app whose menu to invoke." },
                    "app": {
                        "type": "string",
                        "description": "App name or bundle id, from computer_list_apps. Invokes the app's own menu bar without a window; pass either app or pid, not both.",
                    },
                    "pid": {
                        "type": "number",
                        "description": "Running app's process id, from computer_list_apps. Invokes the app's own menu bar without a window; pass either app or pid, not both.",
                    },
                    "path": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "maxItems": 6,
                        "description": "Menu titles from the menu bar down, e.g. [\"File\", \"Export As…\"].",
                    },
                })),
                &["path"],
            ),
            None,
        )),
        hidden(action_entry(
            "computer_kill_app",
            "Force-quit app",
            format!(
                "Force-terminate the app that owns the exact window — the escalation after a cooperative close (computer_invoke_menu [\"File\",\"Quit\"], or \"cmd+q\" via computer_press_key) has already failed. Unsaved state is lost and every window of that app closes; the kill is refused when the window no longer exists. {DELIVERY_HINT}"
            ),
            object_schema(
                object(json!({
                    "window_id": { "type": "string", "description": "A window owned by the app to force-terminate." },
                })),
                &["window_id"],
            ),
            None,
        )),
        hidden(entry(
            "computer_set_window_minimized",
            format!(
                "Minimize or restore the exact window in place — no activation, no focus change, no Space switch. A minimized window stays open and keeps answering the semantic tools (set_value, clicks by label, get_window_state) but takes no coordinate input and is not on screen. The driver reads the minimized state back; confirmed means the readback matched, anything less means observe before relying on it. {DELIVERY_HINT}"
            ),
            object_schema(
                object(json!({
                    "window_id": { "type": "string", "description": "The exact window to minimize or restore." },
                    "minimized": { "type": "boolean", "description": "true minimizes the window into the dock; false restores it." },
                })),
                &["window_id", "minimized"],
            ),
            write_annotations("Minimize or restore window"),
        )),
        hidden(entry(
            "computer_set_app_visibility",
            format!(
                "Hide or unhide a running application by pid — the explicit visibility control for when the user asks to get an app out of the way or bring it back. Every window stays open but leaves the screen, without activating, focusing, or switching Spaces, and the semantic tools keep working. The driver reads the hidden state back; confirmed means the readback matched, anything less means observe before relying on it. {DELIVERY_HINT}"
            ),
            object_schema(
                object(json!({
                    "pid": { "type": "number", "description": "The running application's process id, from computer_list_apps." },
                    "hidden": { "type": "boolean", "description": "true hides the app's windows; false brings them back on screen." },
                })),
                &["pid", "hidden"],
            ),
            write_annotations("Hide or unhide app"),
        )),
        observed_action_entry(
            "computer_click",
            "Click",
            format!(
                "Click a coordinate or a uniquely labelled visible control — once, or as a double-/triple-click via count, with the secondary button via button:\"right\". Ambiguous and off-screen targets are refused. {SEMANTIC_TARGETING_NOTE} {POINTER_COORDINATE_HINT}"
            ),
            object_schema(click_properties, &[]),
            None,
        ),
        hidden(observed_action_entry(
            "computer_move_cursor",
            "Move cursor",
            format!(
                "Move the dedicated computer-use cursor to a coordinate or uniquely labelled visible control. It posts no click and presses nothing: it moves the agent's own visible cursor so the user can see where you are working. On macOS with Cua this only draws an overlay: it does not deliver hover events or open hover menus, and no synthetic move can — macOS discards posted pointer moves unless the user's own cursor is already inside the target window, so a real background hover is not available on this backend. It does not aim the keyboard, so a move followed by computer_type_text without a window_id is refused rather than typed into whatever the cursor happens to be over. The real system pointer never moves. {POINTER_COORDINATE_HINT}"
            ),
            target_schema.clone(),
            Some(json!({
                "title": "Move cursor",
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false,
            })),
        )),
        hidden(observed_action_entry(
            "computer_drag",
            "Drag",
            format!(
                "Drag between two coordinates or uniquely labelled visible controls, holding the primary button down the whole way — a selection swept across text, a file moved, a slider pulled, a window handle resized. {} {POINTER_COORDINATE_HINT}",
                drag_limit_note(dialect)
            ),
            object_schema(
                object(json!({
                    "from": target_schema,
                    "to": target_schema,
                    "duration_ms": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": COMPUTER_DRAG_MAX_DURATION_MS,
                        "description": format!(
                            "How long the pointer takes to travel, in milliseconds. Defaults to {}; clamped to {COMPUTER_DRAG_MAX_DURATION_MS}. A longer glide helps an application that needs to see the drag in progress, such as a drag-and-drop target that must highlight before the drop.",
                            DEFAULT_DRAG_DURATION_MS as u64
                        ),
                    },
                })),
                &["from", "to"],
            ),
            None,
        )),
        observed_action_entry(
            "computer_scroll",
            "Scroll",
            format!(
                "Scroll with delta_x and delta_y in screenshot pixels; omitted axes default to 0, e.g. {{delta_y:300}}. amount/direction are not supported. Requires a screenshot even without coordinates. Capped at half its width/height for overlap; scroll.limitedTo reports reductions. scroll.traveledY measures vertical movement; 0 may mean an edge or dropped input. Inspect the returned image before another scroll; use computer_get_state to find controls. {POINTER_COORDINATE_HINT}"
            ),
            object_schema(scroll_properties, &[]),
            None,
        ),
        observed_action_entry(
            "computer_type_text",
            "Type text",
            format!(
                "Insert text through an exact writable ref or label in window_id; this focus-neutral route can run in different windows concurrently. Use computer_set_value to replace the whole field. window_id alone types into the app's focused field. Semantic writes do not send keydown/keyup: verify autocomplete or submission separately. Background physical keys may refuse on multi-window apps. {KEYBOARD_TARGET_HINT} {DELIVERY_HINT}"
            ),
            object_schema(type_properties, &["text"]),
            None,
        ),
        observed_action_entry(
            "computer_press_key",
            "Press key",
            format!(
                "Press a key or chord such as \"cmd+s\". Pass the observed field ref for exact background Enter/Return. Other shortcuts can still refuse with same_pid_keyboard_ambiguity; use an advertised action instead of retrying. {} {KEYBOARD_TARGET_HINT} {DELIVERY_HINT}",
                hotkey_form_note(dialect)
            ),
            object_schema(key_properties, &["key"]),
            None,
        ),
        hidden(action_entry(
            "computer_write_clipboard",
            "Write computer clipboard",
            format!(
                "Replace the desktop clipboard with text, then paste it with the target application's own paste command. {SHARED_CLIPBOARD_NOTE} Writing discards whatever the user had copied, so prefer computer_type_text for short input and use this for text too long or too awkward to type."
            ),
            object_schema(object(json!({ "text": { "type": "string" } })), &["text"]),
            None,
        )),
        observed_action_entry(
            "computer_paste",
            "Paste text",
            format!(
                "Paste text through the clipboard, the fast path for long or awkward text. It saves the user's clipboard, pastes, restores it and reports clipboardRestored. format md or html pastes rich text with a plain-text fallback. A clipboard holding an image or other non-text content cannot be saved and is replaced. {SHARED_CLIPBOARD_NOTE} {KEYBOARD_TARGET_HINT} {DELIVERY_HINT}"
            ),
            object_schema(paste_properties, &["text"]),
            None,
        ),
        action_entry(
            "computer_activate_window",
            "Activate window",
            "Bring a window into view and aim the agent keyboard at it — only when the user's own task text asked to see the screen; otherwise refused with foreground_not_requested (naming an app is not asking to see it). Ordinary background targeting does not activate a window. A desktop that cannot raise the window refuses. It returns no screenshot; observe with computer_screenshot or computer_get_state when needed.".into(),
            object_schema(
                object(json!({
                    "window_id": { "type": "string", "description": "Window id from computer_list_windows." },
                })),
                &["window_id"],
            ),
            None,
        ),
        observed_action_entry(
            "computer_set_value",
            "Set computer value",
            "Set the value of a uniquely labelled accessible control after a fresh snapshot, through its freshly resolved element token. The label comes from computer_get_state's elements list; this writes atomically instead of typing keystrokes, so prefer it over click-then-type for any field that appears there. It replaces the control's whole value rather than inserting at the caret.".into(),
            object_schema(set_value_properties, &["value"]),
            None,
        ),
        hidden(observed_action_entry(
            "computer_perform_action",
            "Perform computer action",
            format!(
                "Perform a named semantic action on a uniquely labelled accessible control, through the accessibility layer rather than by clicking. {}",
                perform_action_note(dialect)
            ),
            object_schema(perform_properties, &["action"]),
            None,
        )),
        hidden(observed_action_entry(
            "computer_select_text",
            "Select text",
            format!(
                "Select an exact character range inside a text element, through the accessibility layer rather than by key chord or pointer drag. Pass start and length, or text with optional prefix, suffix and selection. {}",
                select_text_note(dialect)
            ),
            object_schema(select_properties, &[]),
            None,
        )),
        action_entry(
            "computer_run",
            "Run computer actions",
            "Batch up to 25 known desktop steps in one call, such as {\"steps\":[{\"type\":\"press_key\",\"key\":\"tab\",\"window_id\":\"...\"}]}. Each step uses type plus the matching computer_ tool fields; computer_help({tool:\"computer_\" + type}) lists exact fields. All steps are validated before dispatch and keep targeting, consent and refusal checks. Read state before acting; use get_state or if_element/unless_element for changes between steps. Stops on failure unless continue_on_error:true. No per-step screenshots or browser steps; an optional final screenshot covers the affected window.".into(),
            object_schema(
                object(json!({
                    "steps": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": COMPUTER_RUN_MAX_STEPS,
                        "items": {
                            "type": "object",
                            "required": ["type"],
                            "properties": { "type": { "type": "string", "enum": RUN_STEP_TYPES } },
                            "additionalProperties": true,
                        },
                        "description": "Ordered steps; read computer_help with the matching tool for allowed fields.",
                    },
                    "include_screenshot": {
                        "type": "boolean",
                        "description": "Attach a final screenshot of the affected window. Defaults to false.",
                    },
                })),
                &["steps"],
            ),
            None,
        ),
    ]
}
