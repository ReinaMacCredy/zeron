//! Native evidence for the in-chat Computer surfaces with isolated data. No agent runs.
use gpui::{AppContext, AsyncApp, Bounds, WindowBounds, WindowOptions, px, size};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use zeron_ui::*;

async fn pause(cx: &mut AsyncApp, ms: u64) {
    cx.background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

fn capture(
    window: gpui::AnyWindowHandle,
    cx: &mut AsyncApp,
    directory: &std::path::Path,
    name: &str,
) -> anyhow::Result<()> {
    window.update(cx, |_, w, cx| {
        w.draw(cx).clear();
        w.render_to_image()?
            .save(directory.join(format!("{name}.png")))?;
        Ok(())
    })?
}

fn input_pause(cx: &mut gpui::App, paused: bool) {
    let Some(store) = computer::preview::store(cx) else {
        return;
    };
    let mut state = json!({
        "threadId": "computer-fixture",
        "version": 1,
        "computerId": "local",
        "windows": [{ "id": "w1", "title": "Calculator", "appName": "Calculator", "focused": true, "minimized": false, "visible": true }],
        "screenSize": { "width": 1440, "height": 900 },
        "agentActive": paused,
        "controlledByOtherThread": false,
        "availability": { "kind": "available", "backend": "mac" },
        "health": { "status": "connected", "consecutiveFailures": 0, "reconnects": 0, "captureAvailable": true },
        "capabilities": {
            "windows": true, "windowBounds": true, "stacking": false, "capture": true, "input": true,
            "clipboard": false, "focus": true, "raise": true, "ghostCursor": true, "visibleDesktop": true
        }
    });
    if paused {
        state["inputPause"] = json!({ "windowId": "w1", "message": "The screen is locked." });
    }
    let event = zeron_proto::computer::ComputerEvent::ThreadState {
        state: Box::new(serde_json::from_value(state).unwrap()),
    };
    computer::preview::update_store(&store, cx, |s| s.apply_event(&event));
}

fn port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn consent(scope: &str, tool: &str, resolved: bool, answer: Option<&str>) -> Value {
    let question = if scope == "task" {
        format!(
            "Allow Computer for this task? The agent wants to run {tool} and will drive this Mac's apps until the turn ends."
        )
    } else {
        format!("Allow the agent to run {tool} once?")
    };
    let mut part = json!({
        "id": format!("input-{tool}"),
        "kind": "input",
        "requestId": format!("computer-consent-{tool}"),
        "questions": [{
            "id": "consent",
            "header": "Computer",
            "question": question,
            "options": ["Allow", "Decline"],
            "multiSelect": false,
            "kind": { "type": "consent", "scope": scope }
        }],
        "resolved": resolved,
    });
    if let Some(answer) = answer {
        part["answers"] = json!([{ "questionId": "consent", "labels": [answer] }]);
    }
    part
}

fn tool(id: &str, name: &str, input: Value, output: &str, is_error: bool) -> Value {
    json!({
        "id": id,
        "kind": "tool",
        "call": { "kind": "mcp", "server": "zeron", "tool": name, "input": input },
        "isError": is_error,
        "resolved": true,
        "output": output,
    })
}

fn app_consent(app: &str, bundle: &str, tool: &str, params: Value) -> Value {
    json!({
        "id": format!("input-{app}"),
        "kind": "input",
        "requestId": format!("computer-consent-{app}"),
        "questions": [{
            "id": "consent",
            "header": "Computer",
            "question": format!("Allow Computer to use {app}? The agent will use {app} until this response ends."),
            "options": ["Allow", format!("Always allow {app}"), "Deny"],
            "multiSelect": false,
            "kind": {
                "type": "consent",
                "scope": "task",
                "app": { "name": app, "bundleId": bundle },
                "tool": tool,
                "params": params.to_string()
            }
        }],
        "resolved": false,
    })
}

fn foreground(app: &str) -> Value {
    json!({
        "id": "input-foreground",
        "kind": "input",
        "requestId": "computer-foreground-1",
        "questions": [{
            "id": "foreground",
            "header": "Computer",
            "question": format!("Bring {app} to the front?"),
            "options": ["Allow for this turn", "No"],
            "multiSelect": false,
            "kind": { "type": "foreground", "app": { "name": app } }
        }],
        "resolved": false,
    })
}

fn running(id: &str, name: &str, input: Value) -> Value {
    let mut part = tool(id, name, input, "", false);
    part["resolved"] = json!(false);
    part.as_object_mut().unwrap().remove("output");
    part
}

fn text(id: &str, text: &str) -> Value {
    json!({ "id": id, "kind": "text", "text": text })
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("warn").init();
    let output = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&output)?;
    let temp = tempfile::tempdir()?;
    let runtime = tokio::runtime::Runtime::new()?;
    let core = runtime.block_on(async {
        zeron_engine::EngineCore::assemble(
            &temp.path().join("engine"),
            Arc::new(zeron_engine::default_registry()),
            zeron_proto::HarnessId::ClaudeCode,
            None,
        )
    })?;
    core.workspace
        .create_chat("computer-fixture", None, Some(&core.device_id), None, None)?;
    core.workspace
        .rename_chat("computer-fixture", "Add 2 + 2 in Calculator")?;
    let ipc_port = port();
    let _ipc = runtime.block_on(zeron_engine::serve_ipc(ipc_port, core.rpc_service()))?;
    let data = temp.path().join("ui");
    std::fs::create_dir(&data)?;
    let boot = EngineBootConfig {
        data_dir: data.clone(),
        ipc_port,
        edge_url: String::new(),
        edge_token: None,
        org_id: None,
        workos_client_id: None,
        default_harness: zeron_proto::HarnessId::ClaudeCode,
    };
    let handle = runtime.block_on(state::EngineHandle::bootstrap(boot.clone()))?;
    let chats = core.workspace.read_chats()?;
    let device = core.device_id.clone();
    let secret = "hunter2 correct horse";
    let denied = r#"{"error":{"code":"capability_denied","message":"Computer control is off for this turn; no input was sent."}}"#;
    let user = |id: &str, body: &str, at: i64| json!({ "id": id, "role": "user", "parts": [text("t", body)], "createdAt": at, "deviceId": device });
    let assistant = |id: &str, parts: Vec<Value>, at: i64, status: &str| json!({ "id": id, "role": "assistant", "parts": parts, "createdAt": at, "deviceId": device, "status": status });
    let system = |id: &str, note: &str, hint: Option<&str>, at: i64| {
        let mut parts = vec![text("t0", note)];
        if let Some(hint) = hint {
            parts.push(text("t1", hint));
        }
        json!({ "id": id, "role": "system", "parts": parts, "createdAt": at, "deviceId": device })
    };
    let pending = vec![
        user(
            "u1",
            "/computer-use open Calculator and add 2 + 2",
            1788900000000,
        ),
        assistant(
            "a1",
            vec![
                text("t", "I'll open Calculator and do the sum."),
                tool(
                    "p0",
                    "computer_screenshot",
                    json!({}),
                    "Captured 1440×900",
                    false,
                ),
                consent("task", "computer_launch_app", false, None),
            ],
            1788900001000,
            "streaming",
        ),
    ];
    let allowed = vec![
        user(
            "u1",
            "/computer-use open Calculator and add 2 + 2",
            1788900000000,
        ),
        assistant(
            "a1",
            vec![
                text("t", "I'll open Calculator and do the sum."),
                tool(
                    "p0",
                    "computer_screenshot",
                    json!({}),
                    "Captured 1440×900",
                    false,
                ),
                consent("task", "computer_launch_app", true, Some("Allow")),
            ],
            1788900001000,
            "complete",
        ),
        system(
            "s1",
            "Computer control ON for this turn: the agent is driving the desktop and the user can switch it off in Settings.",
            None,
            1788900002000,
        ),
        assistant(
            "a2",
            vec![
                tool(
                    "p1",
                    "computer_launch_app",
                    json!({ "app": "com.apple.calculator" }),
                    "Opened",
                    false,
                ),
                tool(
                    "p2",
                    "computer_click",
                    json!({ "label": "2", "app_name": "Calculator" }),
                    "ok",
                    false,
                ),
                tool(
                    "p3",
                    "computer_type_text",
                    json!({ "text": secret, "label": "Display" }),
                    "ok",
                    false,
                ),
                tool(
                    "p4",
                    "computer_press_key",
                    json!({ "key": "cmd+c" }),
                    "ok",
                    false,
                ),
                tool("p6", "computer_read_clipboard", json!({}), secret, false),
            ],
            1788900003000,
            "streaming",
        ),
    ];
    let clipboard = vec![
        user(
            "u3",
            "/computer-use copy the result and tell me what it says",
            1788900020000,
        ),
        assistant(
            "a4",
            vec![
                text("t", "I'll read the clipboard."),
                consent("call", "computer_read_clipboard", false, None),
            ],
            1788900021000,
            "streaming",
        ),
    ];
    let refusals = vec![
        user("u2", "Open Safari and check the page title", 1788900010000),
        assistant(
            "a3",
            vec![
                text("t", "Let me look at the screen first."),
                tool(
                    "p5",
                    "computer_click",
                    json!({ "x": 812, "y": 344 }),
                    denied,
                    true,
                ),
            ],
            1788900011000,
            "complete",
        ),
        system(
            "s2",
            "Computer control needs setup",
            Some(
                "Allow Accessibility, Screen Recording and Input Monitoring for Zeron in System Settings, then continue.",
            ),
            1788900012000,
        ),
        system(
            "s3",
            "Computer is unavailable for this turn",
            Some(
                "Hermes cannot reload its session, so Zeron cannot restart it with the Computer tools without losing the conversation. Start a new chat to use Computer with it.",
            ),
            1788900013000,
        ),
        system(
            "s4",
            "Computer use stopped: Escape was pressed",
            None,
            1788900014000,
        ),
    ];
    let approval = vec![
        user(
            "u5",
            "/computer-use run the tests in Ghostty",
            1788900030000,
        ),
        assistant(
            "a5",
            vec![
                text("t", "I'll run the test suite in Ghostty."),
                app_consent(
                    "Ghostty",
                    "com.mitchellh.ghostty",
                    "computer_type_text",
                    json!({ "window_id": "cua:4242:17", "text": "cargo test" }),
                ),
            ],
            1788900031000,
            "streaming",
        ),
    ];
    let calculator = |id: &str| {
        let mut part = app_consent(
            "Calculator",
            "com.apple.calculator",
            "computer_click",
            json!({ "label": "7" }),
        );
        part["id"] = json!(format!("input-{id}"));
        part
    };
    let quiet = vec![
        user("u6", "/computer-use add 7 + 5 in Calculator", 1788900040000),
        assistant("a6", vec![calculator("calc")], 1788900041000, "streaming"),
    ];
    let foreground_card = vec![
        user(
            "u7",
            "/computer-use type the commit message in TextEdit",
            1788900050000,
        ),
        assistant(
            "a7",
            vec![foreground("TextEdit")],
            1788900051000,
            "streaming",
        ),
    ];
    let codex_rows = vec![
        user(
            "u8",
            "/computer-use add 7 + 5 in Calculator and paste it into TextEdit",
            1788900060000,
        ),
        assistant(
            "a8",
            vec![
                text("t", "Working in Calculator and TextEdit."),
                tool(
                    "q1",
                    "computer_get_state",
                    json!({ "app_name": "Calculator" }),
                    "ok",
                    false,
                ),
                tool(
                    "q2",
                    "computer_click",
                    json!({ "label": "7", "app_name": "Calculator" }),
                    "ok",
                    false,
                ),
                tool(
                    "q3",
                    "computer_type_text",
                    json!({ "text": "Dear team,   the release   notes for 0.2.105 are attached below", "app_name": "TextEdit" }),
                    "ok",
                    false,
                ),
                tool(
                    "q4",
                    "computer_scroll",
                    json!({ "delta_y": 4, "app_name": "TextEdit" }),
                    "ok",
                    false,
                ),
                tool(
                    "q5",
                    "computer_set_value",
                    json!({ "value": "12", "app_name": "Calculator" }),
                    "ok",
                    false,
                ),
                tool(
                    "q6",
                    "computer_press_key",
                    json!({ "key": "cmd+s", "app_name": "TextEdit" }),
                    "ok",
                    false,
                ),
                tool("q7", "computer_list_apps", json!({}), "ok", false),
                running(
                    "q8",
                    "computer_click",
                    json!({ "label": "Save", "app_name": "TextEdit" }),
                ),
            ],
            1788900061000,
            "streaming",
        ),
    ];
    let failure = Arc::new(std::sync::Mutex::new(None));
    let result = failure.clone();
    gpui_platform::application()
        .with_assets(icons::Assets)
        .run(move |cx| {
            gpui_tokio::init(cx);
            gpui_base::init(cx);
            let settings = settings::UiSettings::default();
            settings::init(settings.clone(), data.clone(), cx);
            let fonts = typography::register_fonts(cx);
            typography::init(
                settings.ui_font_family.clone(),
                settings.ui_font_size,
                settings.terminal_font_family.clone(),
                settings.terminal_font_size,
                settings.code_font_family.clone(),
                settings.code_font_size,
                fonts,
                cx,
            );
            theme_library::init(data.clone(), cx);
            appearance::init(
                appearance::AppearanceMode::Dark,
                settings.theme_selection,
                settings.accent,
                settings.surface,
                cx,
            );
            history::init(
                settings.git_history_columns,
                settings.git_history_column_widths,
                settings.git_history_column_order,
                settings.git_history_author_display,
                cx,
            );
            composer::init(cx, settings.composer_send_behavior);
            terminal::panel::init(cx);
            app_menus::init(cx);
            computer::preview::init(cx);
            let state = cx.new(|_| {
                let mut s = state::AppState::new();
                s.fixture_attachment_engine(handle);
                s.connection = zeron_proto::view::ConnectionStatus::Ready;
                s.workspace_scope = Some(zeron_proto::WorkspaceScope::Development);
                s.local_device_id = Some(device.clone());
                s.devices = vec![serde_json::from_value(json!({"id": device, "name": "This device", "platform": std::env::consts::OS, "lastSeenAt": null})).unwrap()];
                s.chats = chats;
                s.selected_chat = Some("computer-fixture".into());
                s.auto_selected = true;
                s.chats_synced = true;
                s.spaces_synced = true;
                s
            });
            let window = cx
                .open_window(
                    WindowOptions {
                        window_background: theme::Theme::of(cx).window_background_appearance(),
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            gpui::point(px(20.), px(40.)),
                            size(px(1100.), px(900.)),
                        ))),
                        ..Default::default()
                    },
                    |_, cx| cx.new(|cx| shell::Shell::new(state.clone(), boot, cx)),
                )
                .unwrap();
            state.update(cx, |_, cx| cx.notify());
            cx.activate(true);
            cx.spawn(async move |cx| {
                let run: anyhow::Result<()> = async {
                    let reset = |entries: Vec<Value>| zeron_doc::TranscriptFrame::Reset {
                        reset: serde_json::from_value(Value::Array(entries)).unwrap(),
                    };
                    pause(cx, 1000).await;
                    state.update(cx, |s, cx| {
                        s.receive_transcript_frame(reset(pending), cx).unwrap();
                        cx.notify();
                    });
                    pause(cx, 900).await;
                    window.update(cx, |s, _, cx| s.fixture_appshots_transcript_start(cx))?;
                    pause(cx, 400).await;
                    capture(window.into(), cx, &output, "consent-card-pending")?;
                    state.update(cx, |s, cx| {
                        s.receive_transcript_frame(reset(allowed), cx).unwrap();
                        cx.notify();
                    });
                    pause(cx, 900).await;
                    window.update(cx, |s, _, cx| s.fixture_appshots_transcript_start(cx))?;
                    pause(cx, 400).await;
                    capture(window.into(), cx, &output, "consent-allowed-disclosure-and-labels")?;
                    state.update(cx, |s, cx| {
                        s.receive_transcript_frame(reset(clipboard), cx).unwrap();
                        cx.notify();
                    });
                    pause(cx, 900).await;
                    window.update(cx, |s, _, cx| s.fixture_appshots_transcript_start(cx))?;
                    pause(cx, 400).await;
                    capture(window.into(), cx, &output, "clipboard-call-card")?;
                    state.update(cx, |s, cx| {
                        s.receive_transcript_frame(reset(refusals), cx).unwrap();
                        cx.notify();
                    });
                    cx.update(|cx| input_pause(cx, true));
                    pause(cx, 900).await;
                    window.update(cx, |s, _, cx| s.fixture_appshots_transcript_start(cx))?;
                    pause(cx, 400).await;
                    capture(window.into(), cx, &output, "denied-setup-unavailable-escape-pause")?;
                    cx.update(|cx| input_pause(cx, false));
                    for (entries, name) in [
                        (approval, "approval-card-elevated-risk-disclosure"),
                        (quiet, "approval-card-calculator"),
                        (foreground_card, "foreground-card"),
                        (codex_rows, "codex-transcript-rows"),
                    ] {
                        state.update(cx, |s, cx| {
                            s.receive_transcript_frame(reset(entries), cx).unwrap();
                            cx.notify();
                        });
                        pause(cx, 900).await;
                        window.update(cx, |s, _, cx| s.fixture_appshots_transcript_start(cx))?;
                        pause(cx, 400).await;
                        capture(window.into(), cx, &output, name)?;
                    }
                    let composer = window.update(cx, |s, _, _| s.fixture_appshots_composer())?;
                    composer.update(cx, |c, cx| c.fixture_type("/comp", cx));
                    pause(cx, 6000).await;
                    capture(window.into(), cx, &output, "slash-menu-computer-use")?;
                    Ok(())
                }
                .await;
                if let Err(error) = run {
                    eprintln!("Computer fixture failed: {error:#}");
                    *result.lock().unwrap() = Some(format!("{error:#}"));
                }
                let _ = window.update(cx, |_, w, _| w.remove_window());
                pause(cx, 100).await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    runtime.block_on(core.shutdown());
    if let Some(error) = failure.lock().unwrap().take() {
        anyhow::bail!(error);
    }
    Ok(())
}
