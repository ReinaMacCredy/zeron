mod args;
mod batch;
mod desktop;
mod elements;
pub mod guidance;
mod help;
mod pipeline;
pub mod progress_guard;
pub mod results;
mod schemas;
pub mod tool_input;
pub mod tool_permission;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use zeron_proto::computer::{McpToolDefinition, McpToolResult};

pub use self::results::{ToolFailure, ToolValue};
pub use self::tool_input::{ToolArgs, ToolInputError};
use self::elements::ElementState;
use self::guidance::ToolGuidanceCadence;
use self::progress_guard::ComputerProgressGuard;
use super::backend::{ComputerAgentDialect, ComputerError};
use super::call_context::{ZERON_CUA_CAPTURE_REUSE, cua_capture_reuse_enabled};
use super::manager::ComputerManager;
use super::screenshot_frames::ScreenshotFrameRegistry;
use super::setup_signal::ComputerSetupSignal;
use super::turn_timing::ComputerTurnTimings;
use super::visible_use::ComputerForegroundAuthorization;

pub const COMPUTER_CONTROL_FIRST_MUTATION_DISCLOSURE: &str = "Computer control ON for this turn: the agent is driving the desktop and the user can switch it off in Settings.";

pub const COMPUTER_TOOL_REFRESH_GUIDANCE: &str = "Computer routing reminder: observe with computer_get_state and exact window_id before acting; prefer element refs; computer_invoke_menu activates the app, so only with visible-use consent. Use coordinates only for controls absent from elements. computer_type_text with window_id alone inserts the whole string into the focused field; never spell text through computer_press_key. Background text is focus-neutral only when Cua proves one writable Accessibility target. Foreground delivery requires the user's visible-use authorization; never replay uncertain delivery; off-Space pixels are not live.";

pub const COMPUTER_APPROVAL_REQUIRED_TOOLS: &[&str] = &[
    "computer_read_clipboard",
    "computer_launch_app",
    "computer_click",
    "computer_move_cursor",
    "computer_drag",
    "computer_scroll",
    "computer_type_text",
    "computer_press_key",
    "computer_write_clipboard",
    "computer_set_value",
    "computer_perform_action",
    "computer_select_text",
    "computer_paste",
    "computer_run",
    "computer_activate_window",
    "computer_set_window_frame",
    "computer_invoke_menu",
    "computer_kill_app",
    "computer_set_window_minimized",
    "computer_set_app_visibility",
];

pub fn computer_tool_requires_approval(name: &str) -> bool {
    COMPUTER_APPROVAL_REQUIRED_TOOLS.contains(&name)
}

pub fn is_mutating_tool_call(name: &str) -> bool {
    computer_tool_requires_approval(name) && name != "computer_read_clipboard"
}

pub const COMPUTER_RUN_MAX_STEPS: usize = 25;

pub const COMPUTER_INSPECTION_TOOL_NAMES: &[&str] = &[
    "computer_read_clipboard",
    "computer_zoom",
    "computer_get_accessibility_tree",
    "computer_get_cursor_position",
    "computer_spaces",
];

pub fn is_inspection_tool_name(name: &str) -> bool {
    COMPUTER_INSPECTION_TOOL_NAMES.contains(&name)
}

const RUN_TARGET_FIELDS: &[&str] = &[
    "x",
    "y",
    "screenshot_id",
    "screenshotId",
    "label",
    "role",
    "ref",
    "ref_ordinal",
    "refOrdinal",
    "window_id",
    "windowId",
];

pub const RUN_STEP_TYPES: &[&str] = &[
    "click",
    "move_cursor",
    "drag",
    "scroll",
    "type_text",
    "press_key",
    "set_value",
    "perform_action",
    "select_text",
    "wait",
    "activate_window",
    "launch_app",
    "write_clipboard",
    "paste",
    "set_window_frame",
    "invoke_menu",
    "kill_app",
    "set_window_minimized",
    "set_app_visibility",
    "get_state",
    "verify_state",
];

pub const RUN_CONDITION_FIELDS: &[&str] = &["if_element", "unless_element", "continue_on_error"];

pub fn run_step_fields(step_type: &str) -> Option<Vec<&'static str>> {
    let with_target = |extra: &[&'static str]| {
        let mut fields = RUN_TARGET_FIELDS.to_vec();
        fields.extend_from_slice(extra);
        fields
    };
    let semantic = |extra: &[&'static str]| {
        let mut fields = extra.to_vec();
        fields.extend_from_slice(&[
            "label",
            "role",
            "ref",
            "ref_ordinal",
            "refOrdinal",
            "window_id",
            "windowId",
        ]);
        fields
    };
    Some(match step_type {
        "click" => with_target(&["modifiers", "count", "button"]),
        "move_cursor" => with_target(&[]),
        "drag" => vec!["from", "to", "duration_ms"],
        "scroll" => with_target(&["delta_x", "delta_y", "modifiers"]),
        "type_text" => semantic(&["text"]),
        "press_key" => vec![
            "key",
            "window_id",
            "windowId",
            "label",
            "role",
            "ref",
            "ref_ordinal",
            "refOrdinal",
        ],
        "set_value" => with_target(&["value"]),
        "perform_action" => with_target(&["action"]),
        "select_text" => {
            let mut fields = semantic(&[]);
            fields.extend_from_slice(&["start", "length"]);
            fields
        }
        "wait" => {
            let mut fields = semantic(&["duration_ms"]);
            fields.push("absent");
            fields
        }
        "activate_window" => vec!["window_id", "windowId"],
        "launch_app" => vec!["app", "arguments", "wait_for_window", "hidden"],
        "write_clipboard" => vec!["text"],
        "paste" => vec!["text", "window_id", "windowId"],
        "set_window_frame" => vec!["window_id", "windowId", "x", "y", "width", "height"],
        "invoke_menu" => vec!["window_id", "windowId", "app", "pid", "path"],
        "kill_app" => vec!["window_id", "windowId"],
        "set_window_minimized" => vec!["window_id", "windowId", "minimized"],
        "set_app_visibility" => vec!["pid", "hidden"],
        "get_state" => vec!["window_id", "windowId", "label_contains", "labelContains"],
        "verify_state" => vec!["window_id", "windowId", "expect"],
        _ => return None,
    })
}

pub fn batch_step_type_for(tool_name: &str) -> Option<&'static str> {
    let step = tool_name.strip_prefix("computer_").unwrap_or(tool_name);
    RUN_STEP_TYPES.iter().find(|name| **name == step).copied()
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerToolEntry {
    pub definition: McpToolDefinition,
    pub discovery_only: bool,
}

impl ComputerToolEntry {
    pub fn name(&self) -> &str {
        &self.definition.name
    }

    fn title(&self) -> Option<&str> {
        self.definition
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.get("title"))
            .and_then(Value::as_str)
    }
}

#[async_trait]
pub trait ComputerToolSeams: Send + Sync {
    fn assert_turn_active(&self) -> Result<(), ComputerError>;

    async fn foreground_authorization(&self) -> ComputerForegroundAuthorization;

    async fn setup_required(&self, _tool: &str, _signal: &ComputerSetupSignal) {}
}

#[derive(Clone)]
pub struct ComputerToolContext {
    pub chat_id: String,
    pub chat_label: Option<String>,
    pub turn_id: Option<String>,
    pub session_key: String,
    pub request_id: Value,
    pub signal: CancellationToken,
    pub seams: Arc<dyn ComputerToolSeams>,
    pub mutation_dispatched: Arc<AtomicBool>,
}

impl ComputerToolContext {
    pub fn mutation_dispatched(&self) -> bool {
        self.mutation_dispatched.load(Ordering::SeqCst)
    }
}

#[async_trait]
pub trait ComputerToolFamily: Send + Sync {
    fn entries(&self) -> Vec<ComputerToolEntry>;

    async fn call(
        &self,
        tools: &ComputerTools,
        name: &str,
        args: ToolArgs,
        context: &ComputerToolContext,
    ) -> McpToolResult;
}

struct ToolState {
    frames: ScreenshotFrameRegistry,
    elements: ElementState,
    unchanged_scrolls: Vec<(String, (Option<String>, u32))>,
    disclosed_first_mutations: Vec<String>,
    app_hints_seen: Vec<String>,
    guidance_cadence: ToolGuidanceCadence,
    progress_guard: ComputerProgressGuard,
}

const DISCLOSED_FIRST_MUTATIONS_MAX: usize = 512;
const THREAD_KEYED_LIMIT: usize = 256;

pub struct ComputerTools {
    manager: Arc<ComputerManager>,
    dialect: ComputerAgentDialect,
    desktop: Vec<ComputerToolEntry>,
    families: Vec<Arc<dyn ComputerToolFamily>>,
    state: Mutex<ToolState>,
    turn_timings: ComputerTurnTimings,
    capture_reuse: AtomicBool,
}

impl ComputerTools {
    pub fn new(
        manager: Arc<ComputerManager>,
        families: Vec<Arc<dyn ComputerToolFamily>>,
    ) -> Arc<Self> {
        let dialect = manager.agent_dialect();
        let started = std::time::Instant::now();
        Arc::new(Self {
            desktop: schemas::desktop_entries(dialect),
            dialect,
            families,
            state: Mutex::new(ToolState {
                frames: ScreenshotFrameRegistry::default(),
                elements: ElementState::default(),
                unchanged_scrolls: Vec::new(),
                disclosed_first_mutations: Vec::new(),
                app_hints_seen: Vec::new(),
                guidance_cadence: ToolGuidanceCadence::new(10, THREAD_KEYED_LIMIT)
                    .expect("positive cadence"),
                progress_guard: ComputerProgressGuard::default(),
            }),
            turn_timings: ComputerTurnTimings::with_log(Arc::new(move || {
                started.elapsed().as_secs_f64() * 1_000.0
            })),
            capture_reuse: AtomicBool::new(cua_capture_reuse_enabled(
                std::env::var(ZERON_CUA_CAPTURE_REUSE).ok().as_deref(),
            )),
            manager,
        })
    }

    pub fn set_capture_reuse(&self, enabled: bool) {
        self.capture_reuse.store(enabled, Ordering::SeqCst);
    }

    pub fn manager(&self) -> &Arc<ComputerManager> {
        &self.manager
    }

    pub fn dialect(&self) -> ComputerAgentDialect {
        self.dialect
    }

    fn state(&self) -> MutexGuard<'_, ToolState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn entries(&self) -> Vec<ComputerToolEntry> {
        let mut entries = self.desktop.clone();
        for family in &self.families {
            entries.extend(family.entries());
        }
        entries
    }

    pub fn entry(&self, name: &str) -> Option<ComputerToolEntry> {
        self.entries().into_iter().find(|entry| entry.name() == name)
    }

    pub fn catalog(&self) -> Vec<McpToolDefinition> {
        self.entries()
            .into_iter()
            .filter(|entry| !entry.discovery_only)
            .map(|entry| entry.definition)
            .collect()
    }

    pub fn call<'a>(
        &'a self,
        name: &'a str,
        args: ToolArgs,
        context: &'a ComputerToolContext,
    ) -> BoxFuture<'a, McpToolResult> {
        async move {
            if self.desktop.iter().any(|entry| entry.name() == name) {
                return self.call_desktop(name, &args, context).await;
            }
            for family in &self.families {
                if family.entries().iter().any(|entry| entry.name() == name) {
                    return family.call(self, name, args, context).await;
                }
            }
            McpToolResult::error(format!("Unknown computer tool \"{name}\"."))
        }
        .boxed()
    }
}
