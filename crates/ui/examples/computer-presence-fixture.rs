#[cfg(not(target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("This fixture requires macOS")
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    macos::main()
}

#[cfg(target_os = "macos")]
mod macos {
    use core_foundation::base::{CFType, CFTypeRef, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_graphics::geometry::CGRect;
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer,
        kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowNumber,
        kCGWindowOwnerPID,
    };
    use gpui::{
        AppContext, AsyncApp, Bounds, Render, WindowBounds, WindowOptions, div, prelude::*, px,
        rgb, size,
    };
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use zeron_proto::computer::{ComputerEvent, ThreadComputerState};
    use zeron_ui::computer::native::status_item;
    use zeron_ui::computer::{preview, status_menu};
    use zeron_ui::state;

    const CHAT: &str = "presence-fixture";

    struct Target;

    impl Render for Target {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div().size_full().bg(rgb(0x2b2d31))
        }
    }

    fn own_windows() -> Vec<(u32, i64, CGRect)> {
        let pid = std::process::id() as i64;
        let Some(list) = copy_window_info(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        ) else {
            return Vec::new();
        };
        let number = |dict: &CFDictionary, key| {
            let value = dict.find(key as *const std::ffi::c_void)?;
            unsafe { CFType::wrap_under_get_rule(*value as CFTypeRef) }
                .downcast::<CFNumber>()?
                .to_i64()
        };
        list.iter()
            .filter_map(|item| {
                let dict = unsafe { CFType::wrap_under_get_rule(*item as CFTypeRef) }
                    .downcast::<CFDictionary>()?;
                if number(&dict, unsafe { kCGWindowOwnerPID })? != pid {
                    return None;
                }
                let bounds = dict.find(unsafe { kCGWindowBounds } as *const std::ffi::c_void)?;
                let bounds = unsafe { CFType::wrap_under_get_rule(*bounds as CFTypeRef) }
                    .downcast::<CFDictionary>()
                    .and_then(|bounds| CGRect::from_dict_representation(&bounds))?;
                Some((
                    number(&dict, unsafe { kCGWindowNumber })? as u32,
                    number(&dict, unsafe { kCGWindowLayer })?,
                    bounds,
                ))
            })
            .collect()
    }

    fn own_window() -> Option<(u32, CGRect)> {
        own_windows()
            .into_iter()
            .find(|(_, layer, bounds)| *layer == 0 && bounds.size.height > 100.0)
            .map(|(id, _, bounds)| (id, bounds))
    }

    fn overlays(step: &str) {
        let panels: Vec<_> = own_windows()
            .into_iter()
            .filter(|(_, layer, _)| *layer != 0)
            .map(|(_, layer, bounds)| (layer, bounds.size.width, bounds.size.height))
            .collect();
        println!("{step}: overlay panels on screen {panels:?}");
    }

    fn thread_state(active: bool) -> serde_json::Value {
        json!({
            "threadId": CHAT, "version": 1, "computerId": "local",
            "windows": [], "screenSize": { "width": 1440, "height": 900 },
            "agentActive": active, "controlledByOtherThread": false,
            "availability": { "kind": "available", "backend": "mac" },
            "health": { "status": "connected", "consecutiveFailures": 0, "reconnects": 0, "captureAvailable": true },
            "capabilities": { "windows": true, "windowBounds": true, "stacking": false, "capture": true, "input": true,
                "clipboard": false, "focus": true, "raise": true, "ghostCursor": true, "visibleDesktop": true }
        })
    }

    fn event(cx: &mut AsyncApp, event: ComputerEvent) {
        cx.update(|cx| {
            let store = preview::store(cx).expect("preview store");
            preview::update_store(&store, cx, |store| store.apply_event(&event));
        });
    }

    fn thread(cx: &mut AsyncApp, value: serde_json::Value) {
        let state: ThreadComputerState = serde_json::from_value(value).expect("thread state");
        event(
            cx,
            ComputerEvent::ThreadState {
                state: Box::new(state),
            },
        );
    }

    async fn pause(cx: &mut AsyncApp, ms: u64) {
        cx.background_executor()
            .timer(Duration::from_millis(ms))
            .await;
    }

    fn capture(directory: &Path, name: &str, bounds: CGRect) -> anyhow::Result<()> {
        shoot(
            directory,
            name,
            (
                bounds.origin.x - 40.0,
                bounds.origin.y - 60.0,
                bounds.size.width + 80.0,
                bounds.size.height + 100.0,
            ),
        )
    }

    fn menu_bar(directory: &Path, name: &str, height: f64) -> anyhow::Result<()> {
        let displays = core_graphics::display::CGDisplay::active_displays()
            .map_err(|error| anyhow::anyhow!("displays: {error}"))?;
        for (index, id) in displays.into_iter().enumerate() {
            let bounds = core_graphics::display::CGDisplay::new(id).bounds();
            shoot(
                directory,
                &format!("{name}-{index}"),
                (bounds.origin.x, bounds.origin.y, bounds.size.width, height),
            )?;
        }
        Ok(())
    }

    fn shoot(
        directory: &Path,
        name: &str,
        (x, y, width, height): (f64, f64, f64, f64),
    ) -> anyhow::Result<()> {
        let region = format!("{x},{y},{width},{height}");
        let path = directory.join(format!("{name}.png"));
        let status = std::process::Command::new("screencapture")
            .args(["-x", "-R", &region])
            .arg(&path)
            .status()?;
        anyhow::ensure!(status.success(), "screencapture {name} failed");
        println!("captured {}", path.display());
        Ok(())
    }

    pub fn main() -> anyhow::Result<()> {
        tracing_subscriber::fmt().with_env_filter("warn").init();
        let output = PathBuf::from(std::env::args().nth(1).expect("output directory"));
        std::fs::create_dir_all(&output)?;
        gpui_platform::application().run(move |cx| {
        gpui_tokio::init(cx);
        gpui_base::init(cx);
        preview::init(cx);
        let app_state = cx.new(|_| {
            let mut s = state::AppState::new();
            s.chats = vec![serde_json::from_value(json!({
                "id": CHAT, "deviceId": "local", "title": "Agent Orchestration for the release train",
                "archived": false, "createdAt": "2026-09-28T00:00:00Z"
            }))
            .unwrap()];
            s
        });
        status_menu::drive(
            app_state,
            |chat_id, cx| {
                println!("menu action: open chat {chat_id}");
                cx.activate(true);
            },
            status_item::apply,
            cx,
        );
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        gpui::point(px(240.), px(260.)),
                        size(px(640.), px(400.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| Target),
            )
            .unwrap();
        cx.activate(true);
        cx.spawn(async move |cx| {
            let run: anyhow::Result<()> = async {
                pause(cx, 800).await;
                let (wid, bounds) = own_window().ok_or_else(|| anyhow::anyhow!("own window not found"))?;
                let target = format!("cua:{}:{wid}", std::process::id());
                println!("target {target} bounds {bounds:?}");
                let started = Instant::now();
                for _ in 0..200 {
                    own_window();
                }
                println!("on-screen window list: {:?} per read", started.elapsed() / 200);

                thread(cx, thread_state(true));
                event(cx, ComputerEvent::Action {
                    window_id: Some(target.clone()),
                    delivery: None,
                    action: "computer_type_text".into(),
                    ok: true,
                    message: Some("typed hunter2".into()),
                    thread_id: Some(CHAT.into()),
                    restored_window_id: None,
                    restore_status: None,
                    masked: false,
                });
                let mut typing = thread_state(true);
                typing["activity"] = json!("Typing");
                thread(cx, typing.clone());
                pause(cx, 700).await;
                overlays("driving");
                capture(&output, "target-driving", bounds)?;
                menu_bar(&output, "menubar-driving", 26.0)?;
                status_item::open_menu();
                pause(cx, 600).await;
                menu_bar(&output, "menu-open", 260.0)?;
                status_item::close_menu();
                pause(cx, 300).await;
                status_item::perform("Open chat");
                pause(cx, 300).await;

                let mut paused = typing.clone();
                paused["inputPause"] = json!({ "windowId": target, "pid": std::process::id(), "message": "Screen locked" });
                thread(cx, paused);
                pause(cx, 700).await;
                capture(&output, "target-paused", bounds)?;
                menu_bar(&output, "menubar-paused", 26.0)?;

                thread(cx, typing.clone());
                pause(cx, 500).await;
                window.update(cx, |_, window, _| window.minimize_window())?;
                pause(cx, 1200).await;
                overlays("minimized");
                window.update(cx, |_, window, _| window.activate_window())?;
                pause(cx, 1200).await;
                overlays("restored");
                capture(&output, "target-restored", bounds)?;

                let mut revoked = typing;
                revoked["inputStopped"] = json!(true);
                thread(cx, revoked);
                pause(cx, 700).await;
                overlays("revoked");
                capture(&output, "target-revoked", bounds)?;
                menu_bar(&output, "menubar-revoked", 26.0)?;
                Ok(())
            }
            .await;
            if let Err(error) = run {
                eprintln!("fixture failed: {error:#}");
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
        Ok(())
    }
}
