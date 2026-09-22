use async_trait::async_trait;
use cua_driver_core::{
    protocol::ToolResult,
    tool::{Tool, ToolDef},
};
use serde_json::Value;

pub struct ListWindowsTool;

static DEF: std::sync::OnceLock<ToolDef> = std::sync::OnceLock::new();

fn def() -> &'static ToolDef {
    DEF.get_or_init(|| ToolDef {
        name: "list_windows".into(),
        description: "List all layer-0 top-level windows currently known to WindowServer. \
            Includes off-screen windows (minimized, on another Space, hidden-launched). \
            Use this to find a window_id before calling get_window_state.\n\n\
            Per-record fields: window_id, pid, app_name, title, bounds \
            (x/y/width/height, top-left origin), z_index (integer or null; higher values are \
            closer to the front; null means stacking order is unavailable and callers must not \
            infer one), is_on_screen, space_ids, current_space_id (the active Space on that \
            window's display), and on_current_space. When include_keyboard_focus is true, optional \
            keyboard_focused reports the app's observed keyboard window; missing means unknown, not \
            false. The top-level current_space_id is \
            WindowServer's main/global active Space and can differ from a record's \
            current_space_id when displays use independent Spaces. To select a frontmost candidate, take the \
            maximum integer z_index; if every value is null, use an explicit fallback instead of \
            relying on array order.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "pid": {
                    "type": "integer",
                    "description": "Optional pid filter. When set, only this pid's windows are returned."
                },
                "on_screen_only": {
                    "type": "boolean",
                    "description": "When true, drop windows not on the current Space. Default false."
                },
                "include_keyboard_focus": {
                    "type": "boolean",
                    "default": false,
                    "description": "Observe the app's keyboard-focused window through Accessibility. Default false; enable for an explicit desktop observation, not geometry revalidation."
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

#[async_trait]
impl Tool for ListWindowsTool {
    fn def(&self) -> &ToolDef {
        def()
    }

    async fn invoke(&self, args: Value) -> ToolResult {
        use cua_driver_core::tool_args::ArgsExt;
        let pid_filter: Option<i32> = args.opt_i64("pid").map(|v| v as i32);
        let on_screen_only = args.bool_or("on_screen_only", false);
        let include_keyboard_focus = args.bool_or("include_keyboard_focus", false);

        let enumeration = if on_screen_only {
            crate::windows::visible_windows_with_space_snapshot()
        } else {
            crate::windows::all_windows_with_space_snapshot()
        };
        let current_space_id = enumeration.current_space_id;
        let mut windows = enumeration.windows;

        if let Some(pid) = pid_filter {
            windows.retain(|w| w.pid == pid);
        }

        let windows_json = if include_keyboard_focus {
            match tokio::task::spawn_blocking(move || {
                window_records_with_keyboard_focus(
                    &windows,
                    true,
                    crate::ax::exact_target::keyboard_focused_window_id,
                )
            })
            .await
            {
                Ok(records) => records,
                Err(error) => {
                    return ToolResult::error(format!("Window focus observation failed: {error}"))
                }
            }
        } else {
            window_records_with_keyboard_focus(
                &windows,
                false,
                crate::ax::exact_target::keyboard_focused_window_id,
            )
        };

        ToolResult::text(format!("Found {} window(s).", windows_json.len())).with_structured(
            serde_json::json!({
                "windows": windows_json,
                "current_space_id": current_space_id
            }),
        )
    }
}

fn window_records_with_keyboard_focus(
    windows: &[crate::windows::WindowInfo],
    include_keyboard_focus: bool,
    mut focused_window: impl FnMut(i32) -> Option<u32>,
) -> Vec<Value> {
    if !include_keyboard_focus {
        return windows.iter().map(window_record_json).collect();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    let mut by_pid = std::collections::HashMap::<i32, Option<u32>>::new();
    windows
        .iter()
        .map(|window| {
            let focused = *by_pid.entry(window.pid).or_insert_with(|| {
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                focused_window(window.pid).filter(|id| {
                    windows
                        .iter()
                        .any(|candidate| candidate.pid == window.pid && candidate.window_id == *id)
                })
            });
            let mut record = window_record_json(window);
            if let Some(focused) = focused {
                record["keyboard_focused"] = Value::Bool(window.window_id == focused);
            }
            record
        })
        .collect()
}

pub(super) fn window_record_json(w: &crate::windows::WindowInfo) -> Value {
    serde_json::json!({
        "window_id": w.window_id,
        "pid": w.pid,
        "app_name": w.app_name,
        "title": w.title,
        "bounds": {
            "x": w.bounds.x,
            "y": w.bounds.y,
            "width": w.bounds.width,
            "height": w.bounds.height
        },
        "layer": w.layer,
        "z_index": w.z_index,
        "is_on_screen": w.is_on_screen,
        "current_space_id": w.current_space_id,
        "on_current_space": w.on_current_space,
        "space_ids": w.space_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> crate::windows::WindowInfo {
        crate::windows::WindowInfo {
            window_id: 42,
            pid: 123,
            app_name: "Example".into(),
            title: "Document".into(),
            bounds: crate::windows::WindowBounds {
                x: 1.0,
                y: 2.0,
                width: 300.0,
                height: 200.0,
            },
            layer: 0,
            z_index: 7,
            is_on_screen: true,
            on_screen_evidence: Some(true),
            current_space_id: Some(1),
            on_current_space: Some(true),
            space_ids: Some(vec![1]),
        }
    }

    #[test]
    fn keyboard_focus_is_opt_in_and_disabled_observations_do_not_read_ax() {
        use cua_driver_core::tool_args::ArgsExt;
        assert_eq!(
            def().input_schema["properties"]["include_keyboard_focus"]["default"],
            false
        );
        let args = serde_json::json!({});
        let records = window_records_with_keyboard_focus(
            &[window()],
            args.bool_or("include_keyboard_focus", false),
            |_| panic!("Default window enumeration must not query Accessibility"),
        );
        assert_eq!(records, vec![window_record_json(&window())]);
        let records = window_records_with_keyboard_focus(&[window()], false, |_| {
            panic!("Disabled keyboard-focus observation must not query Accessibility")
        });
        assert_eq!(records, vec![window_record_json(&window())]);
    }

    #[test]
    fn keyboard_focus_is_read_once_per_pid_and_unknown_stays_absent() {
        let mut sibling = window();
        sibling.window_id = 43;
        let mut unknown = window();
        unknown.pid = 456;
        unknown.window_id = 44;
        let mut reads = std::collections::HashMap::new();
        let records =
            window_records_with_keyboard_focus(&[window(), sibling, unknown], true, |pid| {
                *reads.entry(pid).or_insert(0) += 1;
                if pid == 123 {
                    Some(42)
                } else {
                    None
                }
            });
        assert_eq!(reads.get(&123), Some(&1));
        assert_eq!(reads.get(&456), Some(&1));
        assert_eq!(records[0]["keyboard_focused"], true);
        assert_eq!(records[1]["keyboard_focused"], false);
        assert!(records[2].get("keyboard_focused").is_none());
        let records = window_records_with_keyboard_focus(&[window()], true, |_| Some(99));
        assert!(records[0].get("keyboard_focused").is_none());
    }

    #[test]
    fn window_record_includes_observed_z_index() {
        let window = window();
        assert_eq!(window_record_json(&window)["z_index"], serde_json::json!(7));
        assert_eq!(
            window_record_json(&window)["current_space_id"],
            serde_json::json!(1)
        );
        assert_eq!(
            window_record_json(&window)["on_current_space"],
            serde_json::json!(true)
        );
    }
}
