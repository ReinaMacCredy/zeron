use gpui::{
    AppContext, AsyncApp, Bounds, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, PlatformInput, Point, WindowBounds, WindowOptions, px, size,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use zeron_proto::computer::{ComputerEvent, ThreadComputerState};
use zeron_ui::computer::preview::{self, PreviewFailure, TapEvent};
use zeron_ui::*;

const CHAT: &str = "computer-fixture";

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

fn pointer(
    window: gpui::AnyWindowHandle,
    cx: &mut AsyncApp,
    events: Vec<PlatformInput>,
) -> anyhow::Result<()> {
    window.update(cx, |_, w, cx| {
        for event in events {
            w.dispatch_event(event, cx);
        }
    })
}

fn press(position: Point<Pixels>) -> Vec<PlatformInput> {
    vec![
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }),
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
        }),
    ]
}

fn hover(position: Point<Pixels>, button: Option<MouseButton>) -> PlatformInput {
    PlatformInput::MouseMove(MouseMoveEvent {
        position,
        pressed_button: button,
        modifiers: Modifiers::default(),
    })
}

fn port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn synthetic_window(width: u32, height: u32, highlight: u32) -> Vec<u8> {
    let pixels = image::RgbImage::from_fn(width, height, |x, y| {
        let title = y < height / 14;
        let sidebar = x < width / 5;
        let (cols, rows) = (4, 5);
        let pad_x = width / 5 + 24;
        let pad_y = height / 14 + 90;
        let cell_w = (width - pad_x - 24) / cols;
        let cell_h = (height - pad_y - 24) / rows;
        if title {
            return image::Rgb([58, 58, 62]);
        }
        if sidebar {
            return image::Rgb([36, 36, 40]);
        }
        if y > height / 14 + 20 && y < pad_y - 20 && x > pad_x {
            return image::Rgb([22, 22, 24]);
        }
        if x >= pad_x && y >= pad_y && x < pad_x + cell_w * cols && y < pad_y + cell_h * rows {
            let (cx, cy) = ((x - pad_x) / cell_w, (y - pad_y) / cell_h);
            let inset = (x - pad_x) % cell_w > 6 && (y - pad_y) % cell_h > 6;
            if !inset {
                return image::Rgb([28, 28, 30]);
            }
            if cy * cols + cx == highlight {
                return image::Rgb([255, 159, 10]);
            }
            return if cx == cols - 1 {
                image::Rgb([200, 120, 20])
            } else {
                image::Rgb([80, 80, 86])
            };
        }
        image::Rgb([44, 44, 48])
    });
    let mut jpeg = Vec::new();
    pixels
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    jpeg
}

fn thread_state(agent_active: bool, input_pause: bool) -> ThreadComputerState {
    let mut state = json!({
        "threadId": CHAT,
        "version": 1,
        "computerId": "local",
        "windows": [{ "id": "w1", "title": "Calculator", "appName": "Calculator", "focused": true, "minimized": false, "visible": true }],
        "screenSize": { "width": 1440, "height": 900 },
        "agentActive": agent_active,
        "controlOwnerThreadId": if agent_active { Some(CHAT) } else { None },
        "controlledByOtherThread": false,
        "availability": { "kind": "available", "backend": "mac" },
        "health": { "status": "connected", "consecutiveFailures": 0, "reconnects": 0, "captureAvailable": true },
        "capabilities": {
            "windows": true, "windowBounds": true, "stacking": false, "capture": true, "input": true,
            "clipboard": false, "focus": true, "raise": true, "ghostCursor": true, "visibleDesktop": true
        }
    });
    if input_pause {
        state["inputPause"] = json!({ "windowId": "w1", "message": "The screen is locked." });
    }
    serde_json::from_value(state).unwrap()
}

fn text(id: &str, text: &str) -> Value {
    json!({ "id": id, "kind": "text", "text": text })
}

fn tool(id: &str, name: &str, input: Value) -> Value {
    json!({
        "id": id,
        "kind": "tool",
        "call": { "kind": "mcp", "server": "zeron", "tool": name, "input": input },
        "isError": false,
        "resolved": true,
        "output": "ok",
    })
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("warn").init();
    let output = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    let p45_output = std::env::args().nth(2).map(PathBuf::from);
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
        .create_chat(CHAT, None, Some(&core.device_id), None, None)?;
    core.workspace
        .rename_chat(CHAT, "Add 2 + 2 in Calculator")?;
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
    let settings_client = handle.clone();
    let chats = core.workspace.read_chats()?;
    let device = core.device_id.clone();
    let transcript = vec![
        json!({ "id": "u1", "role": "user", "parts": [text("t", "/computer-use open Calculator and add 2 + 2")], "createdAt": 1788900000000i64, "deviceId": device }),
        json!({
            "id": "a1",
            "role": "assistant",
            "parts": [
                text("t", "I'll open Calculator and do the sum."),
                tool("p1", "computer_launch_app", json!({ "app": "com.apple.calculator" })),
                tool("p2", "computer_click", json!({ "label": "2", "app_name": "Calculator" })),
            ],
            "createdAt": 1788900001000i64,
            "deviceId": device,
            "status": "streaming",
        }),
    ];
    let mut long_transcript = transcript.clone();
    long_transcript.push(json!({
        "id": "a2",
        "role": "assistant",
        "parts": [text("t2", "Calculator is open and the keypad is ready. I clicked the 2 key, then the plus key, then the 2 key again, and the display now reads 4. The preview on the right follows the window I am driving, so you can watch each key press land without switching apps. While it stays docked the conversation keeps a gutter free for it, so none of this text runs underneath the card; pop it out when you want the full width back.")],
        "createdAt": 1788900002000i64,
        "deviceId": device,
        "status": "streaming",
    }));
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
                s.selected_chat = Some(CHAT.into());
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
                            size(px(1320.), px(860.)),
                        ))),
                        ..Default::default()
                    },
                    |_, cx| cx.new(|cx| shell::Shell::new(state.clone(), boot, cx)),
                )
                .unwrap();
            state.update(cx, |_, cx| cx.notify());
            cx.activate(true);
            let store = preview::store(cx).expect("preview store");
            let taps = preview::tap_sender(cx).expect("tap sender");
            cx.spawn(async move |cx| {
                let run: anyhow::Result<()> = async {
                    let event = |cx: &mut AsyncApp, event: ComputerEvent| {
                        cx.update(|cx| preview::update_store(&store, cx, |s| s.apply_event(&event)))
                    };
                    let frame = |seq: u64, highlight: u32| TapEvent::Frame {
                        thread_id: CHAT.into(),
                        seq,
                        jpeg: synthetic_window(960, 600, highlight),
                    };
                    let reset = |entries: Vec<Value>| zeron_doc::TranscriptFrame::Reset {
                        reset: serde_json::from_value(Value::Array(entries)).unwrap(),
                    };
                    let rearm = |cx: &mut AsyncApp| {
                        event(cx, ComputerEvent::ThreadState { state: Box::new(thread_state(false, false)) });
                        event(cx, ComputerEvent::ThreadState { state: Box::new(thread_state(true, false)) });
                    };
                    pause(cx, 1000).await;
                    state.update(cx, |s, cx| {
                        s.receive_transcript_frame(reset(transcript.clone()), cx).unwrap();
                        cx.notify();
                    });
                    cx.update(|cx| preview::update_store(&store, cx, |s| {
                        s.set_events_live(true);
                        false
                    }));
                    pause(cx, 600).await;
                    window.update(cx, |s, _, cx| s.fixture_appshots_transcript_start(cx))?;
                    rearm(cx);
                    event(cx, ComputerEvent::Action {
                        window_id: Some("w1".into()),
                        delivery: serde_json::from_value(json!({ "path": "background_ax", "verified": "confirmed" })).ok(),
                        action: "computer_click".into(),
                        ok: true,
                        message: None,
                        thread_id: Some(CHAT.into()),
                        restored_window_id: None,
                        restore_status: None,
                        masked: false,
                    });
                    for seq in 1..=15u64 {
                        taps.unbounded_send(frame(seq, (seq % 20) as u32))?;
                        pause(cx, 66).await;
                    }
                    pause(cx, 600).await;
                    capture(window.into(), cx, &output, "rail-thumbnail")?;
                    if !cx.update(|cx| store.read(cx).thumbnail_shown(CHAT)) {
                        anyhow::bail!("the default placement shows no rail thumbnail");
                    }
                    let thumbnail = window
                        .update(cx, |s, _, _| s.fixture_computer_thumbnail())?
                        .expect("thumbnail entity");
                    let Some(thumb) = cx.update(|cx| thumbnail.read(cx).fixture_bounds()) else {
                        anyhow::bail!("the rail thumbnail never painted");
                    };
                    pointer(window.into(), cx, vec![hover(thumb.center(), None)])?;
                    pause(cx, 100).await;
                    pointer(window.into(), cx, press(thumb.center()))?;
                    pointer(window.into(), cx, vec![hover(gpui::point(px(300.), px(500.)), None)])?;
                    pause(cx, 800).await;
                    capture(window.into(), cx, &output, "rail-popped-out")?;
                    let popped = cx.update(|cx| store.read(cx).view(CHAT).floating);
                    eprintln!("rail thumbnail {thumb:?} popped out at {popped:?}");
                    if popped.is_none() {
                        anyhow::bail!("clicking the thumbnail did not pop the card out");
                    }
                    cx.update(|cx| preview::update_store(&store, cx, |s| s.set_floating(CHAT, None)));

                    let card = zeron_proto::computer::ComputerSettings {
                        computer_preview_placement: zeron_proto::computer::ComputerPreviewPlacement::Card,
                        ..Default::default()
                    };
                    settings_client
                        .client()
                        .call(zeron_rpc::methods::SET_COMPUTER_SETTINGS, serde_json::to_value(&card)?)
                        .await?;
                    rearm(cx);
                    pause(cx, 800).await;
                    for seq in 20..=30u64 {
                        taps.unbounded_send(frame(seq, (seq % 20) as u32))?;
                        pause(cx, 66).await;
                    }
                    pause(cx, 600).await;
                    capture(window.into(), cx, &output, "compact-live")?;

                    window.update(cx, |_, w, cx| {
                        let width = w.viewport_size().width;
                        let hover = MouseMoveEvent {
                            position: gpui::point(width - px(16. + 200.), px(38. + 12. + 100.)),
                            pressed_button: None,
                            modifiers: Modifiers::default(),
                        };
                        w.dispatch_event(PlatformInput::MouseMove(hover), cx);
                    })?;
                    pause(cx, 400).await;
                    capture(window.into(), cx, &output, "compact-hover-hide")?;
                    window.update(cx, |_, w, cx| {
                        w.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent {
                            position: gpui::point(px(400.), px(400.)),
                            pressed_button: None,
                            modifiers: Modifiers::default(),
                        }), cx);
                    })?;

                    let large = zeron_proto::computer::ComputerSettings {
                        computer_preview_size: zeron_proto::computer::ComputerPreviewSize::Large,
                        ..card.clone()
                    };
                    settings_client
                        .client()
                        .call(zeron_rpc::methods::SET_COMPUTER_SETTINGS, serde_json::to_value(&large)?)
                        .await?;
                    rearm(cx);
                    pause(cx, 800).await;
                    for seq in 100..=110u64 {
                        taps.unbounded_send(frame(seq, 7))?;
                        pause(cx, 66).await;
                    }
                    pause(cx, 600).await;
                    capture(window.into(), cx, &output, "large-live")?;

                    settings_client
                        .client()
                        .call(
                            zeron_rpc::methods::SET_COMPUTER_SETTINGS,
                            serde_json::to_value(&card)?,
                        )
                        .await?;
                    rearm(cx);
                    pause(cx, 800).await;
                    taps.unbounded_send(frame(200, 9))?;
                    pause(cx, 400).await;
                    taps.unbounded_send(TapEvent::Failed {
                        thread_id: CHAT.into(),
                        code: "target_unavailable".into(),
                        message: "The selected window is no longer available.".into(),
                    })?;
                    pause(cx, 600).await;
                    capture(window.into(), cx, &output, "window-gone-last-frame")?;

                    rearm(cx);
                    pause(cx, 400).await;
                    cx.update(|cx| preview::update_store(&store, cx, |s| {
                        s.accept_tap_failure(CHAT, PreviewFailure {
                            code: "target_unavailable".into(),
                            message: "The selected window is no longer available.".into(),
                        })
                    }));
                    pause(cx, 600).await;
                    capture(window.into(), cx, &output, "window-gone-before-first-frame")?;

                    rearm(cx);
                    pause(cx, 400).await;
                    use base64::Engine as _;
                    let still = json!({ "frame": {
                        "sequence": 1,
                        "timestampMs": 1.0,
                        "keyframe": true,
                        "data": base64::engine::general_purpose::STANDARD.encode(synthetic_window(640, 400, 13)),
                    }});
                    let (seq, bytes) = preview::parse_still(&still).expect("still");
                    let decoded = cx
                        .background_executor()
                        .spawn(async move { preview::decode_frame(seq, &bytes) })
                        .await
                        .expect("decoded still");
                    cx.update(|cx| preview::update_store(&store, cx, |s| s.accept_still(CHAT, Some(decoded))));
                    event(cx, ComputerEvent::ThreadState { state: Box::new(thread_state(true, true)) });
                    pause(cx, 800).await;
                    capture(window.into(), cx, &output, "still-fallback-input-paused")?;

                    if let Some(p45) = &p45_output {
                        std::fs::create_dir_all(p45)?;
                        state.update(cx, |s, cx| {
                            s.receive_transcript_frame(reset(long_transcript.clone()), cx).unwrap();
                            cx.notify();
                        });
                        rearm(cx);
                        pause(cx, 400).await;
                        for seq in 300..=310u64 {
                            taps.unbounded_send(frame(seq, 5))?;
                            pause(cx, 66).await;
                        }
                        let away = gpui::point(px(300.), px(500.));
                        pointer(window.into(), cx, vec![hover(away, None)])?;
                        pause(cx, 800).await;
                        capture(window.into(), cx, p45, "docked-with-inset")?;

                        let width = window.update(cx, |_, w, _| w.viewport_size().width)?;
                        let card_right = width - px(16.);
                        let card_top = px(theme::Theme::TITLEBAR_HEIGHT + 12.);
                        let on_card = gpui::point(card_right - px(200.), card_top + px(120.));
                        pointer(window.into(), cx, vec![hover(on_card, None)])?;
                        pause(cx, 300).await;
                        capture(window.into(), cx, p45, "docked-hover-controls")?;
                        let pop_out = gpui::point(card_right - px(59.), card_top + px(27.));
                        pointer(window.into(), cx, vec![hover(pop_out, None)])?;
                        pause(cx, 100).await;
                        pointer(window.into(), cx, press(pop_out))?;
                        pause(cx, 300).await;
                        pointer(window.into(), cx, vec![hover(away, None)])?;
                        pause(cx, 800).await;
                        capture(window.into(), cx, p45, "floating-popped-out")?;
                        let floating = cx.update(|cx| store.read(cx).view(CHAT).floating);
                        let Some(origin) = floating else {
                            anyhow::bail!("the pop-out button did not detach the card");
                        };

                        let grab = gpui::point(px(origin.x + 300.), px(origin.y + 200.));
                        let drop = gpui::point(px(origin.x - 380.), px(origin.y + 260.));
                        pointer(window.into(), cx, vec![hover(grab, None)])?;
                        pause(cx, 100).await;
                        pointer(window.into(), cx, vec![PlatformInput::MouseDown(MouseDownEvent {
                            button: MouseButton::Left,
                            position: grab,
                            modifiers: Modifiers::default(),
                            click_count: 1,
                            first_mouse: false,
                        })])?;
                        pause(cx, 50).await;
                        for step in 1..=10 {
                            let t = step as f32 / 10.;
                            let at = grab + (drop - grab) * t;
                            pointer(window.into(), cx, vec![hover(at, Some(MouseButton::Left))])?;
                            pause(cx, 30).await;
                        }
                        pointer(window.into(), cx, vec![PlatformInput::MouseUp(MouseUpEvent {
                            button: MouseButton::Left,
                            position: drop,
                            modifiers: Modifiers::default(),
                            click_count: 1,
                        })])?;
                        pointer(window.into(), cx, vec![hover(away, None)])?;
                        pause(cx, 600).await;
                        capture(window.into(), cx, p45, "floating-dragged")?;
                        let moved = cx.update(|cx| store.read(cx).view(CHAT).floating);
                        eprintln!("p4-5 floating origin {origin:?} -> {moved:?}");

                        let dragged = moved.unwrap_or(origin);
                        let card = gpui::point(px(dragged.x + 300.), px(dragged.y + 150.));
                        pointer(window.into(), cx, vec![hover(card, None)])?;
                        pause(cx, 300).await;
                        let dock = gpui::point(px(dragged.x + 600. - 59.), px(dragged.y + 27.));
                        pointer(window.into(), cx, vec![hover(dock, None)])?;
                        pause(cx, 100).await;
                        pointer(window.into(), cx, press(dock))?;
                        pointer(window.into(), cx, vec![hover(away, None)])?;
                        pause(cx, 800).await;
                        capture(window.into(), cx, p45, "docked-again")?;
                        let docked = cx.update(|cx| store.read(cx).view(CHAT).floating);
                        eprintln!("p4-5 after dock: {docked:?}");
                    }

                    settings_client
                        .client()
                        .call(
                            zeron_rpc::methods::SET_COMPUTER_SETTINGS,
                            serde_json::to_value(zeron_proto::computer::ComputerSettings::default())?,
                        )
                        .await?;
                    window.update(cx, |s, _, cx| {
                        s.fixture_computer_settings(cx);
                    })?;
                    pause(cx, 1200).await;
                    capture(window.into(), cx, &output, "settings-placement")?;
                    Ok(())
                }
                .await;
                if let Err(error) = run {
                    eprintln!("Computer preview fixture failed: {error:#}");
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
