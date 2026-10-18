mod audit;
mod capture;
mod frames;
#[cfg(debug_assertions)]
mod input;
#[cfg(unix)]
mod server;
mod tree;

#[cfg(unix)]
pub(crate) use server::start;

use crate::shell::{Route, Shell};
use gpui::{AnyWindowHandle, App, AsyncApp};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use zeron_proto::ui_driver::{DRIVE_ACTIONS as DRIVE, OBSERVE_ACTIONS as OBSERVE};

static FRAMES: AtomicU64 = AtomicU64::new(0);
static LAST_DRAW_MS: AtomicU64 = AtomicU64::new(0);

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

pub(crate) fn frame_drawn() {
    let elapsed = epoch().elapsed();
    FRAMES.fetch_add(1, Ordering::Relaxed);
    LAST_DRAW_MS.store(elapsed.as_millis() as u64 + 1, Ordering::Relaxed);
    frames::stamp(elapsed.as_micros() as u64);
}

fn frame_count() -> u64 {
    FRAMES.load(Ordering::Relaxed)
}

fn last_draw_ago() -> Option<Duration> {
    match LAST_DRAW_MS.load(Ordering::Relaxed) {
        0 => None,
        at => Some(
            epoch()
                .elapsed()
                .saturating_sub(Duration::from_millis(at - 1)),
        ),
    }
}

async fn next_frame(after: u64, cap: Duration, cx: &AsyncApp) -> Option<u64> {
    let started = Instant::now();
    loop {
        let count = frame_count();
        if count > after {
            return Some(count);
        }
        if started.elapsed() >= cap {
            return None;
        }
        cx.background_executor()
            .timer(Duration::from_millis(8))
            .await;
    }
}

#[derive(Debug, PartialEq, Deserialize)]
struct Request {
    #[serde(default)]
    window: Option<u64>,
    #[serde(flatten)]
    action: Action,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Action {
    State,
    Tree(tree::View),
    Audit,
    Snapshot {
        rect: Option<capture::Rect>,
        path: Option<PathBuf>,
    },
    Wait(Until),
    Frames(frames::Frames),
    Track(frames::Track),
    #[cfg(debug_assertions)]
    Set(input::Set),
    #[cfg(debug_assertions)]
    Click {
        #[serde(flatten)]
        at: input::Target,
        #[serde(default)]
        button: input::Button,
        #[serde(default = "one")]
        count: usize,
        #[serde(default)]
        modifiers: Vec<input::Modifier>,
    },
    #[cfg(debug_assertions)]
    Move {
        #[serde(flatten)]
        at: input::Target,
    },
    #[cfg(debug_assertions)]
    Drag {
        from: input::Target,
        to: input::Target,
        #[serde(default = "drag_steps")]
        steps: u32,
        #[serde(default)]
        hold_ms: u64,
        #[serde(default)]
        modifiers: Vec<input::Modifier>,
    },
    #[cfg(debug_assertions)]
    Scroll {
        #[serde(flatten)]
        at: input::Target,
        #[serde(default)]
        dx: f32,
        #[serde(default)]
        dy: f32,
        #[serde(default)]
        trackpad: bool,
    },
    #[cfg(debug_assertions)]
    Key {
        keys: input::Keys,
    },
    #[cfg(debug_assertions)]
    Type {
        text: String,
        #[serde(flatten)]
        into: input::TypeAt,
    },
    #[cfg(debug_assertions)]
    Press {
        #[serde(flatten)]
        element: tree::Element,
        #[serde(rename = "do")]
        perform: Option<String>,
        value: Option<String>,
    },
    #[cfg(debug_assertions)]
    Action {
        name: String,
        args: Option<Value>,
    },
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "for", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Until {
    Frame {
        #[serde(default = "wait_timeout")]
        timeout_ms: u64,
    },
    Idle {
        #[serde(default = "quiet")]
        quiet_ms: u64,
        #[serde(default = "wait_timeout")]
        timeout_ms: u64,
    },
    Ms {
        ms: u64,
    },
    #[serde(untagged)]
    Seen(tree::Seen),
}

#[cfg(debug_assertions)]
fn one() -> usize {
    1
}

#[cfg(debug_assertions)]
fn drag_steps() -> u32 {
    12
}

fn quiet() -> u64 {
    300
}

fn wait_timeout() -> u64 {
    5000
}

fn valid_actions() -> Vec<&'static str> {
    let mut names = OBSERVE.to_vec();
    if cfg!(debug_assertions) {
        names.extend(DRIVE);
    }
    names
}

fn parse(raw: Value) -> Result<Request, String> {
    let name = raw
        .get("action")
        .and_then(Value::as_str)
        .ok_or("a request needs an \"action\" string")?
        .to_owned();
    if !cfg!(debug_assertions) && DRIVE.contains(&name.as_str()) {
        return Err(format!(
            "`{name}` drives the UI and exists only in debug builds (Zeron Dev bundles); this build is observe-only"
        ));
    }
    if !valid_actions().contains(&name.as_str()) {
        return Err(format!(
            "unknown action `{name}`; valid actions: {}",
            valid_actions().join(", ")
        ));
    }
    if let Some(during) = raw.get("during") {
        check_during(during)?;
    }
    serde_json::from_value(raw).map_err(|error| format!("invalid `{name}` request: {error}"))
}

fn check_during(during: &Value) -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Err(
            "`during` drives the UI and exists only in debug builds (Zeron Dev bundles); this build is observe-only".into(),
        );
    }
    let name = during.get("action").and_then(Value::as_str).unwrap_or("");
    if !DRIVE.contains(&name) {
        return Err(format!(
            "`during` must be a drive request ({}), not {during}",
            DRIVE.join(", ")
        ));
    }
    parse(during.clone()).map(drop)
}

async fn handle(raw: Value, cx: &mut AsyncApp) -> Value {
    let id = raw.get("id").cloned().unwrap_or(Value::Null);
    match run(raw, cx).await {
        Ok(ok) => json!({ "id": id, "ok": ok }),
        Err(error) => json!({ "id": id, "error": error }),
    }
}

async fn run(raw: Value, cx: &mut AsyncApp) -> Result<Value, String> {
    let Request { window, action } = parse(raw)?;
    match action {
        Action::State => cx.update(state),
        Action::Tree(view) => {
            let handle = cx.update(|cx| target_window(window, cx))?;
            tree::render(&tree::read(handle, cx).await?, &view)
        }
        Action::Audit => {
            let handle = cx.update(|cx| target_window(window, cx))?;
            let tree = tree::read(handle, cx).await?;
            let focus_held = cx
                .update(|cx| handle.update(cx, |_, window, cx| window.focused(cx).is_some()))
                .map_err(|error| format!("window is gone: {error}"))?;
            Ok(audit::audit(&tree, focus_held))
        }
        Action::Snapshot { rect, path } => {
            let handle = cx.update(|cx| target_window(window, cx))?;
            // The window server holds the frame before the last one drawn until another presents.
            fresh_frame(handle, Duration::from_millis(500), cx).await?;
            cx.update(|cx| capture::snapshot(handle, rect, path, cx))
        }
        Action::Wait(until) => wait(window, until, cx).await,
        Action::Frames(request) => frames::frames(window, request, cx).await,
        Action::Track(request) => frames::track(window, request, cx).await,
        #[cfg(debug_assertions)]
        Action::Set(set) => input::set(window, set, cx).await,
        #[cfg(debug_assertions)]
        Action::Click {
            at,
            button,
            count,
            modifiers,
        } => input::click(window, at, button, count, &modifiers, cx).await,
        #[cfg(debug_assertions)]
        Action::Move { at } => input::hover(window, at, cx).await,
        #[cfg(debug_assertions)]
        Action::Drag {
            from,
            to,
            steps,
            hold_ms,
            modifiers,
        } => input::drag(window, from, to, steps, hold_ms, &modifiers, cx).await,
        #[cfg(debug_assertions)]
        Action::Scroll {
            at,
            dx,
            dy,
            trackpad,
        } => input::scroll(window, at, dx, dy, trackpad, cx).await,
        #[cfg(debug_assertions)]
        Action::Key { keys } => input::keys(window, keys, cx).await,
        #[cfg(debug_assertions)]
        Action::Type { text, into } => input::type_text(window, &text, into, cx).await,
        #[cfg(debug_assertions)]
        Action::Press {
            element,
            perform,
            value,
        } => input::press(window, element, perform, value, cx).await,
        #[cfg(debug_assertions)]
        Action::Action { name, args } => input::action(window, &name, args, cx).await,
    }
}

fn target_window(id: Option<u64>, cx: &App) -> Result<AnyWindowHandle, String> {
    if let Some(id) = id {
        return cx
            .windows()
            .into_iter()
            .find(|handle| handle.window_id().as_u64() == id)
            .ok_or_else(|| format!("no window {id}; `state` lists the open windows"));
    }
    let windows = cx.window_stack().unwrap_or_else(|| cx.windows());
    windows
        .iter()
        .find(|handle| handle.downcast::<Shell>().is_some())
        .or(windows.first())
        .copied()
        .ok_or_else(|| "no open window".to_owned())
}

fn build() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn frames_json() -> Value {
    json!({
        "count": frame_count(),
        "lastDrawAgoMs": last_draw_ago().map(|ago| ago.as_millis() as u64),
    })
}

fn state(cx: &mut App) -> Result<Value, String> {
    let mut windows = Vec::new();
    let mut route = Value::Null;
    for handle in cx.window_stack().unwrap_or_else(|| cx.windows()) {
        let shell = handle.downcast::<Shell>();
        let _ = handle.update(cx, |_, window, _| {
            let size = window.viewport_size();
            windows.push(json!({
                "id": handle.window_id().as_u64(),
                "title": window.window_title(),
                "kind": if shell.is_some() { "shell" } else { "other" },
                "size": { "w": f32::from(size.width), "h": f32::from(size.height) },
                "active": window.is_window_active(),
                "a11yForced": window.is_a11y_forced(),
            }));
        });
        if route.is_null()
            && let Some(shell) = shell
            && let Ok(current) = shell.read_with(cx, |shell, cx| shell.driver_route(cx))
        {
            route = route_json(current);
        }
    }
    Ok(json!({
        "pid": std::process::id(),
        "build": build(),
        "input": cfg!(debug_assertions),
        "windows": windows,
        "route": route,
        "frames": frames_json(),
        "motionScale": crate::motion::speed_scale(),
        "theme": theme_json(cx),
    }))
}

fn theme_json(cx: &mut App) -> Value {
    let painted = cx.try_global::<crate::theme::Theme>().map(|theme| {
        if theme.appearance.is_dark() {
            "dark"
        } else {
            "light"
        }
    });
    json!({ "mode": crate::appearance::mode(cx), "appearance": painted })
}

fn route_json((route, selected_chat): (Route, Option<String>)) -> Value {
    let (kind, section) = match route {
        Route::Chat if selected_chat.is_none() => ("newChat", None),
        Route::Chat => ("chat", None),
        Route::PullRequests => ("pullRequests", None),
        Route::Usage => ("usage", None),
        Route::Inbox => ("inbox", None),
        Route::Settings(section) => ("settings", Some(format!("{section:?}"))),
        Route::Terminal => ("terminal", None),
        Route::Tasks => ("tasks", None),
        Route::Docs => ("docs", None),
    };
    json!({ "kind": kind, "section": section, "selectedChat": selected_chat })
}

async fn fresh_frame(
    handle: AnyWindowHandle,
    cap: Duration,
    cx: &mut AsyncApp,
) -> Result<Option<u64>, String> {
    let mark = frame_count();
    cx.update(|cx| handle.update(cx, |_, window, _| window.refresh()))
        .map_err(|error| format!("window is gone: {error}"))?;
    Ok(next_frame(mark, cap, cx).await)
}

async fn wait(window: Option<u64>, until: Until, cx: &mut AsyncApp) -> Result<Value, String> {
    match until {
        Until::Frame { timeout_ms } => {
            let handle = cx.update(|cx| target_window(window, cx))?;
            fresh_frame(handle, Duration::from_millis(timeout_ms), cx)
                .await?
                .map(|frame| json!({ "frame": frame }))
                .ok_or_else(|| format!("no frame drawn within {timeout_ms} ms"))
        }
        Until::Idle {
            quiet_ms,
            timeout_ms,
        } => {
            let quiet = Duration::from_millis(quiet_ms);
            let (started, mark) = (Instant::now(), frame_count());
            loop {
                let ago = last_draw_ago();
                if ago.is_none_or(|ago| ago >= quiet) {
                    return Ok(json!({ "frame": frame_count(), "quietMs": quiet_ms }));
                }
                let elapsed = started.elapsed();
                if elapsed >= Duration::from_millis(timeout_ms) {
                    let drawn = frame_count() - mark;
                    return Err(format!(
                        "not idle after {timeout_ms} ms: {drawn} frames drawn ({:.1} fps), last {} ms ago",
                        drawn as f64 / elapsed.as_secs_f64(),
                        ago.unwrap_or_default().as_millis()
                    ));
                }
                let left = quiet.saturating_sub(ago.unwrap_or_default());
                cx.background_executor()
                    .timer(left.clamp(Duration::from_millis(5), Duration::from_millis(50)))
                    .await;
            }
        }
        Until::Seen(seen) => {
            let handle = cx.update(|cx| target_window(window, cx))?;
            tree::wait(handle, seen, cx).await
        }
        Until::Ms { ms } => {
            cx.background_executor()
                .timer(Duration::from_millis(ms))
                .await;
            Ok(json!({ "frame": frame_count() }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_observe_action_with_a_window() {
        let request = parse(json!({ "id": 1, "action": "state", "window": 7 })).unwrap();
        assert_eq!(
            request,
            Request {
                window: Some(7),
                action: Action::State
            }
        );
    }

    #[test]
    fn wait_defaults_and_unknown_condition() {
        let request = parse(json!({ "id": "a", "action": "wait", "for": "idle" })).unwrap();
        assert_eq!(
            request.action,
            Action::Wait(Until::Idle {
                quiet_ms: 300,
                timeout_ms: 5000
            })
        );
        let error = parse(json!({ "action": "wait", "for": "forever" })).unwrap_err();
        assert!(error.starts_with("invalid `wait` request"), "{error}");
    }

    #[test]
    fn unknown_action_lists_this_builds_actions() {
        let error = parse(json!({ "id": 2, "action": "explode" })).unwrap_err();
        assert!(error.starts_with("unknown action `explode`"), "{error}");
        for name in valid_actions() {
            assert!(error.contains(name), "{error} misses {name}");
        }
        assert!(parse(json!({ "id": 3 })).is_err());
    }

    #[test]
    fn every_listed_action_is_a_variant() {
        for name in valid_actions() {
            if let Err(error) = parse(json!({ "action": name })) {
                assert!(!error.contains("unknown variant"), "{name}: {error}");
            }
        }
    }

    #[test]
    fn parses_tree_audit_and_element_waits() {
        let tree = parse(json!({ "action": "tree", "query": "send", "depth": 2 })).unwrap();
        assert_eq!(
            tree.action,
            Action::Tree(tree::View {
                query: Some("send".into()),
                depth: Some(2),
                ..tree::View::default()
            })
        );
        assert_eq!(
            parse(json!({ "action": "audit" })).unwrap().action,
            Action::Audit
        );
        let select = parse(json!({
            "action": "wait", "for": "select", "select": "Button \"Send\"", "nth": 1,
        }))
        .unwrap();
        assert_eq!(
            select.action,
            Action::Wait(Until::Seen(tree::Seen::Select {
                element: tree::Element::Select {
                    select: "Button \"Send\"".into(),
                    nth: Some(1)
                },
                timeout_ms: 5000,
            }))
        );
        let gone = parse(json!({ "action": "wait", "for": "gone", "ref": "e4", "timeoutMs": 9 }));
        assert_eq!(
            gone.unwrap().action,
            Action::Wait(Until::Seen(tree::Seen::Gone {
                element: tree::Element::Ref { id: "e4".into() },
                timeout_ms: 9,
            }))
        );
        let text = parse(json!({ "action": "wait", "for": "text", "text": "Done" })).unwrap();
        assert_eq!(
            text.action,
            Action::Wait(Until::Seen(tree::Seen::Text {
                text: "Done".into(),
                timeout_ms: 5000,
            }))
        );
        assert!(parse(json!({ "action": "wait", "for": "gone" })).is_err());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn parses_element_targets_and_press() {
        let click = parse(json!({ "action": "click", "select": "Tab \"Git\"", "nth": 0 }));
        assert_eq!(
            click.unwrap().action,
            Action::Click {
                at: input::Target::Element(tree::Element::Select {
                    select: "Tab \"Git\"".into(),
                    nth: Some(0)
                }),
                button: input::Button::Left,
                count: 1,
                modifiers: vec![],
            }
        );
        let drag = parse(json!({ "action": "drag", "from": { "ref": "e3" }, "to": [5, 6] }));
        let Action::Drag { from, to, .. } = drag.unwrap().action else {
            panic!("not a drag");
        };
        assert_eq!(
            from,
            input::Target::Element(tree::Element::Ref { id: "e3".into() })
        );
        assert_eq!(to, input::Target::Pair([5.0, 6.0]));
        let typed = parse(json!({ "action": "type", "text": "hi" })).unwrap();
        assert_eq!(
            typed.action,
            Action::Type {
                text: "hi".into(),
                into: input::TypeAt::Focused {}
            }
        );
        let typed = parse(json!({ "action": "type", "text": "hi", "ref": "e7" })).unwrap();
        assert_eq!(
            typed.action,
            Action::Type {
                text: "hi".into(),
                into: input::TypeAt::Target(input::Target::Element(tree::Element::Ref {
                    id: "e7".into()
                }))
            }
        );
        let press = parse(json!({
            "action": "press", "ref": "e7", "do": "setValue", "value": "hello",
        }))
        .unwrap();
        assert_eq!(
            press.action,
            Action::Press {
                element: tree::Element::Ref { id: "e7".into() },
                perform: Some("setValue".into()),
                value: Some("hello".into()),
            }
        );
        let error = parse(json!({ "action": "press", "do": "click" })).unwrap_err();
        assert!(error.starts_with("invalid `press` request"), "{error}");
    }

    #[test]
    fn parses_frames_and_track() {
        let frames = parse(json!({ "action": "frames" })).unwrap();
        let Action::Frames(frames) = frames.action else {
            panic!("not frames");
        };
        assert!(format!("{frames:?}").contains("ms: 1000"), "{frames:?}");
        let track = parse(json!({ "action": "track", "select": "Button \"Go\"", "nth": 1 }));
        let Action::Track(track) = track.unwrap().action else {
            panic!("not track");
        };
        let shown = format!("{track:?}");
        assert!(
            shown.contains("ms: 800") && shown.contains("jump_px: 40.0"),
            "{shown}"
        );
        assert!(parse(json!({ "action": "track", "ms": 50 })).is_err());
    }

    #[test]
    fn during_takes_only_a_valid_drive_request() {
        let observe = parse(json!({ "action": "frames", "during": { "action": "state" } }));
        let error = observe.unwrap_err();
        if cfg!(debug_assertions) {
            assert!(error.contains("must be a drive request"), "{error}");
            let bad = parse(json!({ "action": "frames", "during": { "action": "click" } }));
            assert!(bad.unwrap_err().starts_with("invalid `click` request"));
            let ok = parse(json!({
                "action": "track", "ref": "e3", "during": { "action": "key", "keys": "cmd-k" },
            }));
            assert!(ok.is_ok(), "{ok:?}");
        } else {
            assert!(error.contains("only in debug builds"), "{error}");
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    fn parses_set() {
        let set =
            parse(json!({ "action": "set", "theme": "dark", "size": { "w": 1200, "h": 800 } }));
        let Action::Set(set) = set.unwrap().action else {
            panic!("not set");
        };
        let shown = format!("{set:?}");
        assert!(
            shown.contains("Dark") && shown.contains("1200.0"),
            "{shown}"
        );
        assert!(parse(json!({ "action": "set", "size": [900, 700], "motionScale": 5 })).is_ok());
        assert!(parse(json!({ "action": "set", "theme": "sepia" })).is_err());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn parses_a_drive_actions_args() {
        let request = parse(json!({
            "id": 4,
            "action": "click",
            "x": 12,
            "y": 40.5,
            "button": "right",
            "count": 2,
            "modifiers": ["cmd", "shift"],
        }))
        .unwrap();
        assert_eq!(
            request.action,
            Action::Click {
                at: input::Target::Point { x: 12.0, y: 40.5 },
                button: input::Button::Right,
                count: 2,
                modifiers: vec![input::Modifier::Cmd, input::Modifier::Shift],
            }
        );
        let drag =
            parse(json!({ "action": "drag", "from": [1, 2], "to": [3, 4], "holdMs": 50 })).unwrap();
        assert_eq!(
            drag.action,
            Action::Drag {
                from: input::Target::Pair([1.0, 2.0]),
                to: input::Target::Pair([3.0, 4.0]),
                steps: 12,
                hold_ms: 50,
                modifiers: vec![],
            }
        );
        let keys = parse(json!({ "action": "key", "keys": ["cmd-k", "down enter"] })).unwrap();
        let Action::Key { keys } = keys.action else {
            panic!("not a key action");
        };
        assert_eq!(keys.strokes(), ["cmd-k", "down", "enter"]);
        let error = parse(json!({ "action": "click", "y": 3 })).unwrap_err();
        assert!(error.starts_with("invalid `click` request"), "{error}");
    }
}
