//! Native evidence for Settings › Computer with isolated data. No agent runs.
use gpui::{AppContext, AsyncApp, Bounds, WindowBounds, WindowOptions, px, size};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};
use zeron_ui::appshots::{AppshotCapabilities, AppshotPlatform, CapabilityState, CaptureTarget};
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

fn port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn grants(accessibility: bool, screen: bool, input: bool) -> AppshotCapabilities {
    let state = |granted: bool| {
        if granted {
            CapabilityState::Ready
        } else {
            CapabilityState::PermissionRequired
        }
    };
    AppshotCapabilities {
        platform: AppshotPlatform::MacOs,
        global_shortcut: CapabilityState::Ready,
        window_capture: state(screen),
        application_text: state(accessibility),
        input_monitoring: state(input),
        target: CaptureTarget::ActiveWindow,
    }
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
    let device = core.device_id.clone();
    let engine_settings = core.computer.clone();
    let ready_status: zeron_proto::computer::ComputerHostStatus = serde_json::from_value(json!({
        "hostBundleId": "sh.zeron.app.dev",
        "driverNativeRevision": 39,
        "status": {
            "computerId": "desktop",
            "availability": { "kind": "available", "backend": "cua" },
            "health": { "status": "connected", "consecutiveFailures": 0, "reconnects": 1, "captureAvailable": true },
            "capabilities": { "windows": true, "windowBounds": true, "stacking": true, "capture": true, "input": true, "clipboard": true, "focus": true, "raise": true, "ghostCursor": true, "visibleDesktop": true },
        },
    }))?;
    let entry = |id: usize, tool: &str, effect: &str, minute: usize| json!({ "id": format!("{id:064x}"), "ts": format!("2026-09-23T09:{minute:02}:00.000Z"), "tool": tool, "effect": effect });
    let history: Vec<zeron_proto::computer::ComputerGetAuditHistoryResult> =
        serde_json::from_value(json!([
            {
                "entries": [
                    entry(1, "computer_launch_app", "verified", 41),
                    entry(2, "computer_click", "verified", 40),
                    entry(3, "computer_type_text", "dispatched-unknown", 39),
                    entry(4, "computer_press_key", "verified", 38),
                    entry(5, "computer_read_clipboard", "verified", 37),
                    entry(6, "computer_click", "refused", 36),
                    entry(7, "computer_scroll", "not-dispatched", 35),
                ],
                "nextCursor": "older",
                "truncated": true,
                "status": "available",
            }
        ]))?;
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
            let state = cx.new(|_| {
                let mut s = state::AppState::new();
                s.fixture_attachment_engine(handle);
                s.connection = zeron_proto::view::ConnectionStatus::Ready;
                s.workspace_scope = Some(zeron_proto::WorkspaceScope::Development);
                s.local_device_id = Some(device.clone());
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
                    pause(cx, 800).await;
                    let page = window.update(cx, |s, _, cx| s.fixture_computer_settings(cx))?;
                    page.update(cx, |page, _| page.fixture_grants(grants(true, true, true)));
                    pause(cx, 1500).await;
                    capture(window.into(), cx, &output, "page-toggle-off")?;
                    page.update(cx, |page, cx| page.fixture_toggle(cx));
                    pause(cx, 1200).await;
                    anyhow::ensure!(
                        engine_settings.settings().computer_control_enabled,
                        "the toggle never reached the engine"
                    );
                    capture(window.into(), cx, &output, "page-toggle-on")?;
                    page.update(cx, |page, cx| page.fixture_view(false, 330.0, cx));
                    pause(cx, 500).await;
                    capture(window.into(), cx, &output, "getting-started")?;
                    cx.update(|cx| {
                        settings::update(settings::SavePolicy::Immediate, cx, |s| {
                            s.computer_getting_started_acknowledged = true
                        });
                    });
                    page.update(cx, |page, cx| {
                        page.fixture_grants(grants(false, true, false));
                        page.fixture_status(ready_status.clone());
                        page.fixture_set_up(cx);
                        page.fixture_status(ready_status.clone());
                        page.fixture_view(true, 0.0, cx);
                    });
                    pause(cx, 900).await;
                    capture(window.into(), cx, &output, "advanced-set-up-guide")?;
                    page.update(cx, |page, cx| page.fixture_view(true, 420.0, cx));
                    pause(cx, 500).await;
                    capture(window.into(), cx, &output, "advanced-set-up-guide-scrolled")?;
                    page.update(cx, |page, cx| {
                        page.fixture_grants(grants(true, true, true));
                        cx.notify();
                    });
                    pause(cx, 900).await;
                    page.update(cx, |page, cx| {
                        page.fixture_status(ready_status.clone());
                        page.fixture_view(true, 420.0, cx);
                    });
                    pause(cx, 600).await;
                    capture(window.into(), cx, &output, "advanced-ready")?;
                    page.update(cx, |page, cx| {
                        page.fixture_history(history.clone());
                        page.fixture_view(false, 150.0, cx);
                    });
                    pause(cx, 700).await;
                    capture(window.into(), cx, &output, "history")?;
                    Ok(())
                }
                .await;
                if let Err(error) = run {
                    eprintln!("Computer settings fixture failed: {error:#}");
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
