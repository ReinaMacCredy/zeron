mod audit;
mod capture;
mod frames;
#[cfg(debug_assertions)]
mod input;
mod record;
mod selector;
#[cfg(unix)]
mod server;
mod tree;
#[cfg(target_os = "macos")]
pub(crate) mod web;

#[cfg(unix)]
pub(crate) use server::start;

pub(crate) use record::tap as record_tap;

use crate::shell::{Route, Shell};
use gpui::{AnyWindowHandle, App, AsyncApp};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use zeron_proto::ui_driver::{DRIVE_ACTIONS as DRIVE, MAX_WAIT_MS, OBSERVE_ACTIONS as OBSERVE};

static FRAMES: AtomicU64 = AtomicU64::new(0);

type Cancel = Arc<AtomicBool>;

#[cfg(debug_assertions)]
use input::{Drive, run as drive};

#[cfg(not(debug_assertions))]
#[derive(Debug, PartialEq)]
enum Drive {}

#[cfg(not(debug_assertions))]
impl<'de> Deserialize<'de> for Drive {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(observe_only("during")))
    }
}

#[cfg(not(debug_assertions))]
async fn drive(_: Option<u64>, drive: Drive, _: Cancel, _: &mut AsyncApp) -> Result<Value, String> {
    match drive {}
}

fn observe_only(name: &str) -> String {
    format!(
        "`{name}` drives the UI and exists only in debug builds (Zeron Dev bundles); this build is observe-only"
    )
}

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

pub(crate) fn frame_drawn() {
    let elapsed = epoch().elapsed();
    FRAMES.fetch_add(1, Ordering::Relaxed);
    frames::stamp(elapsed.as_micros() as u64);
}

fn frame_count() -> u64 {
    FRAMES.load(Ordering::Relaxed)
}

fn last_draw_ago() -> Option<Duration> {
    match frames::LAST_US.load(Ordering::Relaxed) {
        0 => None,
        at => Some(epoch().elapsed().saturating_sub(Duration::from_micros(at))),
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

#[derive(Debug, PartialEq)]
struct Request {
    window: Option<u64>,
    action: Action,
}

#[derive(Debug, PartialEq)]
enum Action {
    Observe(Observe),
    Drive(Drive),
}

#[derive(Debug, PartialEq, Deserialize)]
struct Windowed<A> {
    #[serde(default)]
    window: Option<u64>,
    #[serde(flatten)]
    action: A,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Observe {
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
    Record(record::Record),
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
    Seen(selector::Seen),
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
    let drives = DRIVE.contains(&name.as_str());
    if drives && !cfg!(debug_assertions) {
        return Err(observe_only(&name));
    }
    if !valid_actions().contains(&name.as_str()) {
        return Err(format!(
            "unknown action `{name}`; valid actions: {}",
            valid_actions().join(", ")
        ));
    }
    if let Some(inner) = raw
        .get("during")
        .and_then(|during| during.get("action"))
        .and_then(Value::as_str)
        .filter(|inner| !DRIVE.contains(inner))
    {
        return Err(format!(
            "`during` must be a drive request ({}), not `{inner}`",
            DRIVE.join(", ")
        ));
    }
    let invalid = |error: serde_json::Error| format!("invalid `{name}` request: {error}");
    if drives {
        let Windowed { window, action } = serde_json::from_value(raw).map_err(invalid)?;
        return Ok(Request {
            window,
            action: Action::Drive(action),
        });
    }
    let Windowed { window, action } = serde_json::from_value(raw).map_err(invalid)?;
    if let Observe::Wait(until) = &action {
        until.check()?;
    }
    Ok(Request {
        window,
        action: Action::Observe(action),
    })
}

impl Until {
    fn check(&self) -> Result<(), String> {
        if let Until::Seen(
            selector::Seen::Text { text, .. }
            | selector::Seen::Gone {
                absent: selector::Absent::Text { text },
                ..
            },
        ) = self
            && text.is_empty()
        {
            return Err("`text` must not be empty: every node contains the empty string".into());
        }
        let limits = match self {
            Until::Frame { timeout_ms } => vec![("timeoutMs", *timeout_ms)],
            Until::Idle {
                quiet_ms,
                timeout_ms,
            } => vec![("quietMs", *quiet_ms), ("timeoutMs", *timeout_ms)],
            Until::Ms { ms } => vec![("ms", *ms)],
            Until::Seen(seen) => vec![("timeoutMs", seen.timeout_ms())],
        };
        match limits.into_iter().find(|&(_, ms)| ms > MAX_WAIT_MS) {
            Some((name, ms)) => Err(format!(
                "`{name}` must be at most {MAX_WAIT_MS} ms, not {ms}"
            )),
            None => Ok(()),
        }
    }
}

async fn handle(raw: Value, cancel: Cancel, cx: &mut AsyncApp) -> Value {
    let id = raw.get("id").cloned().unwrap_or(Value::Null);
    match run(raw, cancel, cx).await {
        Ok(ok) => json!({ "id": id, "ok": ok }),
        Err(error) => json!({ "id": id, "error": error }),
    }
}

async fn run(raw: Value, cancel: Cancel, cx: &mut AsyncApp) -> Result<Value, String> {
    let Request { window, action } = parse(raw)?;
    let observe = match action {
        Action::Drive(action) => return drive(window, action, cancel, cx).await,
        Action::Observe(observe) => observe,
    };
    match observe {
        Observe::State => cx.update(state),
        Observe::Tree(view) => {
            let handle = cx.update(|cx| target_window(window, cx))?;
            tree::render(&tree::read(handle, cx).await?, &view)
        }
        Observe::Audit => {
            let handle = cx.update(|cx| target_window(window, cx))?;
            let tree = tree::read_native(handle, cx).await?;
            let focus_held = cx
                .update(|cx| handle.update(cx, |_, window, cx| window.focused(cx).is_some()))
                .map_err(|error| format!("window is gone: {error}"))?;
            Ok(audit::audit(&tree, focus_held))
        }
        Observe::Snapshot { rect, path } => {
            let handle = cx.update(|cx| target_window(window, cx))?;
            // The window server holds the frame before the last one drawn until another presents.
            let drawn = fresh_frame(handle, Duration::from_millis(500), cx).await?;
            let mut reply = cx.update(|cx| capture::snapshot(handle, rect, path, cx))?;
            if drawn.is_none() {
                reply["stale"] = json!(true);
            }
            Ok(reply)
        }
        Observe::Wait(until) => wait(window, until, cx).await,
        Observe::Frames(request) => frames::frames(window, request, cancel, cx).await,
        Observe::Track(request) => frames::track(window, request, cancel, cx).await,
        Observe::Record(record) => record::run(window, record, cx).await,
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
        Route::TaskCenter => ("taskCenter", None),
        Route::Supervisor => ("supervisor", None),
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
            selector::wait(handle, seen, cx).await
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
                action: Action::Observe(Observe::State)
            }
        );
    }

    #[test]
    fn wait_defaults_and_unknown_condition() {
        let request = parse(json!({ "id": "a", "action": "wait", "for": "idle" })).unwrap();
        assert_eq!(
            request.action,
            Action::Observe(Observe::Wait(Until::Idle {
                quiet_ms: 300,
                timeout_ms: 5000
            }))
        );
        let error = parse(json!({ "action": "wait", "for": "forever" })).unwrap_err();
        assert!(error.starts_with("invalid `wait` request"), "{error}");
    }

    #[test]
    fn every_wait_is_capped_at_the_ceiling() {
        let at = MAX_WAIT_MS;
        for (request, field) in [
            (json!({ "for": "frame", "timeoutMs": at + 1 }), "timeoutMs"),
            (json!({ "for": "idle", "quietMs": at + 1 }), "quietMs"),
            (json!({ "for": "idle", "timeoutMs": 70_000 }), "timeoutMs"),
            (json!({ "for": "ms", "ms": at + 1 }), "ms"),
            (
                json!({ "for": "select", "select": "Button", "timeoutMs": at + 1 }),
                "timeoutMs",
            ),
            (
                json!({ "for": "gone", "ref": "e1", "timeoutMs": at + 1 }),
                "timeoutMs",
            ),
            (
                json!({ "for": "text", "text": "x", "timeoutMs": at + 1 }),
                "timeoutMs",
            ),
        ] {
            let mut over = request.clone();
            over["action"] = json!("wait");
            let error = parse(over).unwrap_err();
            assert!(
                error.starts_with(&format!("`{field}` must be at most {MAX_WAIT_MS} ms")),
                "{request}: {error}"
            );
            let mut limit = request;
            limit["action"] = json!("wait");
            limit[field] = json!(at);
            assert!(parse(limit.clone()).is_ok(), "{limit}");
        }
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

    fn variants<T: std::fmt::Debug + for<'de> Deserialize<'de>>() -> Vec<String> {
        let error = serde_json::from_value::<T>(json!({ "action": "zzz" }))
            .unwrap_err()
            .to_string();
        let (_, variants) = error.split_once("expected one of ").expect(&error);
        let mut variants: Vec<String> = variants
            .split(", ")
            .map(|variant| {
                variant
                    .trim_matches(|c: char| c == '`' || c.is_whitespace())
                    .to_owned()
            })
            .collect();
        variants.sort_unstable();
        variants
    }

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut names: Vec<String> = names.iter().map(|&name| name.to_owned()).collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn every_listed_action_is_a_variant() {
        for &name in OBSERVE.iter().chain(DRIVE) {
            match parse(json!({ "action": name })) {
                Err(error) => assert!(
                    !error.contains("unknown variant") && !error.contains("unknown action"),
                    "{name}: {error}"
                ),
                Ok(request) => assert!(
                    cfg!(debug_assertions) || matches!(request.action, Action::Observe(_)),
                    "{name} parsed as a drive request in a release build"
                ),
            }
        }
        assert_eq!(variants::<Observe>(), sorted(OBSERVE));
        #[cfg(debug_assertions)]
        assert_eq!(variants::<Drive>(), sorted(DRIVE));
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn a_release_build_refuses_every_drive_request_and_during() {
        for &name in DRIVE {
            let error = parse(json!({ "action": name, "x": 1, "y": 2 })).unwrap_err();
            assert!(
                error.starts_with(&format!(
                    "`{name}` drives the UI and exists only in debug builds"
                )),
                "{name}: {error}"
            );
        }
        for measure in ["frames", "track"] {
            let error = parse(json!({
                "action": measure, "ref": "e1", "during": { "action": "click", "x": 1, "y": 2 },
            }))
            .unwrap_err();
            assert!(
                error.contains("`during` drives the UI and exists only in debug builds"),
                "{error}"
            );
        }
    }

    #[test]
    fn parses_tree_audit_and_element_waits() {
        let tree = parse(json!({ "action": "tree", "query": "send", "depth": 2 })).unwrap();
        assert_eq!(
            tree.action,
            Action::Observe(Observe::Tree(tree::View {
                query: Some("send".into()),
                depth: Some(2),
                ..tree::View::default()
            }))
        );
        assert_eq!(
            parse(json!({ "action": "audit" })).unwrap().action,
            Action::Observe(Observe::Audit)
        );
        let select = parse(json!({
            "action": "wait", "for": "select", "select": "Button \"Send\"", "nth": 1,
        }))
        .unwrap();
        assert_eq!(
            select.action,
            Action::Observe(Observe::Wait(Until::Seen(selector::Seen::Select {
                element: selector::Element::Select {
                    select: "Button \"Send\"".into(),
                    nth: Some(1)
                },
                timeout_ms: 5000,
            })))
        );
        let gone = parse(json!({ "action": "wait", "for": "gone", "ref": "e4", "timeoutMs": 9 }));
        assert_eq!(
            gone.unwrap().action,
            Action::Observe(Observe::Wait(Until::Seen(selector::Seen::Gone {
                absent: selector::Absent::Element(selector::Element::Ref { id: "e4".into() }),
                timeout_ms: 9,
            })))
        );
        let gone = parse(json!({ "action": "wait", "for": "gone", "text": "Working" }));
        assert_eq!(
            gone.unwrap().action,
            Action::Observe(Observe::Wait(Until::Seen(selector::Seen::Gone {
                absent: selector::Absent::Text {
                    text: "Working".into()
                },
                timeout_ms: 5000,
            })))
        );
        let text = parse(json!({ "action": "wait", "for": "text", "text": "Done" })).unwrap();
        assert_eq!(
            text.action,
            Action::Observe(Observe::Wait(Until::Seen(selector::Seen::Text {
                text: "Done".into(),
                timeout_ms: 5000,
            })))
        );
        assert!(parse(json!({ "action": "wait", "for": "gone" })).is_err());
        for until in ["gone", "text"] {
            let error = parse(json!({ "action": "wait", "for": until, "text": "" })).unwrap_err();
            assert!(error.starts_with("`text` must not be empty"), "{error}");
        }
    }

    #[cfg(debug_assertions)]
    fn driven(raw: Value) -> Drive {
        match parse(raw).unwrap().action {
            Action::Drive(drive) => drive,
            other => panic!("not a drive request: {other:?}"),
        }
    }

    fn observed(raw: Value) -> Observe {
        match parse(raw).unwrap().action {
            Action::Observe(observe) => observe,
            other => panic!("not an observe request: {other:?}"),
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    fn parses_element_targets_and_press() {
        assert_eq!(
            driven(json!({ "action": "click", "select": "Tab \"Git\"", "nth": 0 })),
            Drive::Click {
                at: input::Target::Element(selector::Element::Select {
                    select: "Tab \"Git\"".into(),
                    nth: Some(0)
                }),
                button: input::Button::Left,
                count: 1,
                modifiers: vec![],
            }
        );
        let drag = driven(json!({ "action": "drag", "from": { "ref": "e3" }, "to": [5, 6] }));
        let Drive::Drag { from, to, .. } = drag else {
            panic!("not a drag");
        };
        assert_eq!(
            from,
            input::Target::Element(selector::Element::Ref { id: "e3".into() })
        );
        assert_eq!(to, input::Target::Pair([5.0, 6.0]));
        assert_eq!(
            driven(json!({ "action": "type", "text": "hi" })),
            Drive::Type {
                text: "hi".into(),
                into: input::TypeAt::Focused
            }
        );
        assert_eq!(
            driven(json!({ "action": "type", "text": "hi", "ref": "e7" })),
            Drive::Type {
                text: "hi".into(),
                into: input::TypeAt::Target(input::Target::Element(selector::Element::Ref {
                    id: "e7".into()
                }))
            }
        );
        let press = driven(json!({
            "action": "press", "ref": "e7", "do": "setValue", "value": "hello",
        }));
        assert_eq!(
            press,
            Drive::Press {
                element: selector::Element::Ref { id: "e7".into() },
                perform: Some("setValue".into()),
                value: Some("hello".into()),
            }
        );
        let error = parse(json!({ "action": "press", "do": "click" })).unwrap_err();
        assert!(error.starts_with("invalid `press` request"), "{error}");
    }

    #[cfg(debug_assertions)]
    #[test]
    fn malformed_or_doubled_targets_fail() {
        for bad in [
            json!({ "action": "type", "text": "hi", "ref": 5 }),
            json!({ "action": "type", "text": "hi", "x": 3 }),
            json!({ "action": "type", "text": "hi", "nth": 1 }),
            json!({ "action": "click", "x": 1, "y": 2, "ref": "e3" }),
            json!({ "action": "move", "x": 1, "y": 2, "select": "Button" }),
            json!({ "action": "type", "text": "hi", "x": 1, "y": 2, "ref": "e3" }),
            json!({ "action": "drag", "from": { "x": 1, "y": 2, "ref": "e3" }, "to": [3, 4] }),
            json!({ "action": "click", "ref": "e3", "select": "Button" }),
            json!({ "action": "click" }),
        ] {
            let error = parse(bad.clone()).unwrap_err();
            assert!(error.starts_with("invalid `"), "{bad}: {error}");
        }
        let error = parse(json!({ "action": "click", "x": 1, "y": 2, "ref": "e3" })).unwrap_err();
        assert!(
            error.contains("give one of ref, select, or x and y"),
            "{error}"
        );
        assert_eq!(
            driven(json!({ "id": 9, "action": "type", "text": "hi", "x": 1, "y": 2 })),
            Drive::Type {
                text: "hi".into(),
                into: input::TypeAt::Target(input::Target::Point { x: 1.0, y: 2.0 })
            }
        );
    }

    #[test]
    fn parses_frames_and_track() {
        let Observe::Frames(frames) = observed(json!({ "action": "frames" })) else {
            panic!("not frames");
        };
        assert!(format!("{frames:?}").contains("ms: 1000"), "{frames:?}");
        let track = observed(json!({ "action": "track", "select": "Button \"Go\"", "nth": 1 }));
        let Observe::Track(track) = track else {
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
        assert!(error.contains("must be a drive request"), "{error}");
        if cfg!(debug_assertions) {
            let bad = parse(json!({ "action": "frames", "during": { "action": "click" } }));
            let error = bad.unwrap_err();
            assert!(error.starts_with("invalid `frames` request"), "{error}");
            let ok = parse(json!({
                "action": "track", "ref": "e3", "during": { "action": "key", "keys": "cmd-k" },
            }));
            assert!(ok.is_ok(), "{ok:?}");
            let windowed = observed(json!({
                "action": "frames", "window": 7, "during": { "action": "key", "keys": "escape", "window": 5 },
            }));
            assert!(
                format!("{windowed:?}").contains("window: Some(5)"),
                "{windowed:?}"
            );
        }
        let null = parse(json!({ "action": "frames", "during": null }));
        assert!(null.is_ok(), "{null:?}");
    }

    #[cfg(debug_assertions)]
    #[test]
    fn parses_set() {
        let set =
            driven(json!({ "action": "set", "theme": "dark", "size": { "w": 1200, "h": 800 } }));
        let Drive::Set(set) = set else {
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
        let request = driven(json!({
            "id": 4,
            "action": "click",
            "x": 12,
            "y": 40.5,
            "button": "right",
            "count": 2,
            "modifiers": ["cmd", "shift"],
        }));
        assert_eq!(
            request,
            Drive::Click {
                at: input::Target::Point { x: 12.0, y: 40.5 },
                button: input::Button::Right,
                count: 2,
                modifiers: vec![input::Modifier::Cmd, input::Modifier::Shift],
            }
        );
        assert_eq!(
            driven(json!({ "action": "drag", "from": [1, 2], "to": [3, 4], "holdMs": 50 })),
            Drive::Drag {
                from: input::Target::Pair([1.0, 2.0]),
                to: input::Target::Pair([3.0, 4.0]),
                steps: 12,
                hold_ms: 50,
                modifiers: vec![],
            }
        );
        let keys = driven(json!({ "action": "key", "keys": ["cmd-k", "down enter"] }));
        let Drive::Key { keys } = keys else {
            panic!("not a key action");
        };
        assert_eq!(keys.strokes(), ["cmd-k", "down", "enter"]);
        let error = parse(json!({ "action": "click", "y": 3 })).unwrap_err();
        assert!(error.starts_with("invalid `click` request"), "{error}");
    }
}
