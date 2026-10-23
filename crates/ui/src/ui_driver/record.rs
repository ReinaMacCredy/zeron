use super::selector::Selector;
use super::tree::{self, Node, Tree};
use gpui::{
    AsyncApp, DispatchPhase, IntoElement, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, ScrollWheelEvent, Styled as _, Subscription, TouchPhase, Window, px,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const DRAG_PT: f32 = 4.0;
const LINE_PX: f32 = 20.0;
const HOLD_EVERY: Duration = Duration::from_secs(5);
const REDACTED: &str = "<redacted>";
const SECRET_WORDS: [&str; 3] = ["token", "secret", "password"];
const SECRET_TERMS: [&str; 2] = ["key", "api"];

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "do", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(super) enum Record {
    Start {
        #[serde(default)]
        include_driven: bool,
    },
    Stop {
        path: Option<PathBuf>,
    },
}

struct Active {
    window: u64,
    include_driven: bool,
    recorder: Recorder,
    _keys: Subscription,
}

thread_local! {
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
    static DRIVEN: Cell<bool> = const { Cell::new(false) };
}

#[cfg(debug_assertions)]
pub(super) fn driven<R>(f: impl FnOnce() -> R) -> R {
    let before = DRIVEN.replace(true);
    let result = f();
    DRIVEN.set(before);
    result
}

fn recording(window: u64) -> bool {
    ACTIVE.with_borrow(|active| {
        active
            .as_ref()
            .is_some_and(|active| active.window == window)
    })
}

pub(crate) fn tap(window: &Window) -> Option<impl IntoElement + use<>> {
    recording(window.window_handle().window_id().as_u64()).then(|| {
        gpui::canvas(
            |_, _, _| {},
            |_, _, window, _| {
                window.on_mouse_event(|event: &MouseDownEvent, phase, window, _| {
                    let button = match event.button {
                        MouseButton::Left => "left",
                        MouseButton::Right => "right",
                        MouseButton::Middle => "middle",
                        MouseButton::Navigate(_) => return,
                    };
                    if phase == DispatchPhase::Capture {
                        let (x, y) = xy(event.position);
                        let count = event.click_count;
                        let modifiers = modifier_names(&event.modifiers);
                        feed(window, |recorder, tree| {
                            recorder.press(tree(), x, y, button, count, modifiers)
                        });
                    }
                });
                window.on_mouse_event(|event: &MouseUpEvent, phase, window, _| {
                    if phase == DispatchPhase::Capture {
                        let (x, y) = xy(event.position);
                        feed(window, |recorder, _| recorder.release(x, y));
                    }
                });
                window.on_mouse_event(|event: &ScrollWheelEvent, phase, window, _| {
                    if phase == DispatchPhase::Capture {
                        let (x, y) = xy(event.position);
                        let delta = event.delta.pixel_delta(px(LINE_PX));
                        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
                        let trackpad = event.touch_phase == TouchPhase::Started;
                        feed(window, |recorder, tree| {
                            recorder.scroll(tree, x, y, dx, dy, trackpad)
                        });
                    }
                });
            },
        )
        .absolute()
        .size_0()
    })
}

fn xy(position: gpui::Point<gpui::Pixels>) -> (f32, f32) {
    (f32::from(position.x), f32::from(position.y))
}

fn feed(window: &Window, f: impl FnOnce(&mut Recorder, &dyn Fn() -> Option<Tree>)) {
    let id = window.window_handle().window_id().as_u64();
    ACTIVE.with_borrow_mut(|active| {
        let Some(active) = active.as_mut().filter(|active| active.window == id) else {
            return;
        };
        if DRIVEN.get() && !active.include_driven {
            return;
        }
        let tree = || {
            let dump = window.debug_a11y_tree_json()?;
            let mut refs = tree::REFS
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            tree::parse_dump(&dump, id, &mut refs).ok()
        };
        f(&mut active.recorder, &tree);
    });
}

pub(super) async fn run(
    window: Option<u64>,
    record: Record,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    match record {
        Record::Start { include_driven } => start(window, include_driven, cx).await,
        Record::Stop { path } => stop(path, cx),
    }
}

async fn start(
    window: Option<u64>,
    include_driven: bool,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    if include_driven && !cfg!(debug_assertions) {
        return Err(super::observe_only("includeDriven"));
    }
    if ACTIVE.with_borrow(Option::is_some) {
        return Err("a recording is already running; `record {\"do\":\"stop\"}` first".into());
    }
    let handle = cx.update(|cx| super::target_window(window, cx))?;
    tree::read(handle, cx).await?;
    let id = handle.window_id().as_u64();
    cx.update(|cx| {
        if ACTIVE.with_borrow(Option::is_some) {
            return Err(String::from("a recording is already running"));
        }
        let keys = cx.intercept_keystrokes(|event, window, _| {
            let keystroke = event.keystroke.clone();
            feed(window, |recorder, tree| recorder.stroke(tree, &keystroke));
        });
        ACTIVE.set(Some(Active {
            window: id,
            include_driven,
            recorder: Recorder::default(),
            _keys: keys,
        }));
        handle
            .update(cx, |_, window, _| window.refresh())
            .map_err(|error| format!("window is gone: {error}"))
    })?;
    cx.spawn(async move |cx| {
        // The a11y lease lapses after 60 s idle; a tap must always find a current tree.
        while recording(id) {
            cx.update(|cx| tree::hold(id, cx));
            cx.background_executor().timer(HOLD_EVERY).await;
        }
    })
    .detach();
    Ok(json!({ "recording": true, "window": id, "includeDriven": include_driven }))
}

fn stop(path: Option<PathBuf>, cx: &mut AsyncApp) -> Result<Value, String> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    if let Some(path) = &path {
        check_path(path)?;
    }
    if ACTIVE.with_borrow(Option::is_none) {
        return Err("no recording is running; `record {\"do\":\"start\"}` first".into());
    }
    let path = path.unwrap_or_else(|| {
        zeron_proto::ui_driver::run_dir()
            .join("recordings")
            .join(format!(
                "{}-{}.json",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed) + 1
            ))
    });
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("recording")
        .to_owned();
    let (scenario, steps, fallbacks, redacted) = ACTIVE
        .with_borrow(|active| {
            active
                .as_ref()
                .map(|active| active.recorder.scenario(&name))
        })
        .ok_or("no recording is running")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| {
            format!(
                "{}: {error}; still recording, stop again with another path",
                dir.display()
            )
        })?;
    }
    let body = serde_json::to_string_pretty(&scenario).map_err(|error| error.to_string())?;
    std::fs::write(&path, body + "\n").map_err(|error| {
        format!(
            "{}: {error}; still recording, stop again with another path",
            path.display()
        )
    })?;
    if let Some(active) = ACTIVE.take() {
        cx.update(|cx| {
            for handle in cx.windows() {
                if handle.window_id().as_u64() == active.window {
                    let _ = handle.update(cx, |_, window, _| window.refresh());
                }
            }
        });
    }
    Ok(json!({ "path": path, "steps": steps, "fallbacks": fallbacks, "redacted": redacted }))
}

fn check_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("`path` must be absolute: {}", path.display()));
    }
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        return Err(format!("`path` must end in .json: {}", path.display()));
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() => Err(format!(
            "`path` exists and is not a regular file (a symlink, folder or device): {}",
            path.display()
        )),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{}: {error}", path.display()))
        }
        _ => Ok(()),
    }
}

fn modifier_names(modifiers: &Modifiers) -> Vec<&'static str> {
    [
        (modifiers.platform, "cmd"),
        (modifiers.shift, "shift"),
        (modifiers.alt, "alt"),
        (modifiers.control, "ctrl"),
    ]
    .into_iter()
    .filter_map(|(held, name)| held.then_some(name))
    .collect()
}

#[derive(Debug, Clone, PartialEq)]
enum Target {
    Select { select: String, nth: Option<usize> },
    Point { x: f32, y: f32, why: Option<String> },
}

impl Target {
    fn fields(&self) -> Map<String, Value> {
        let mut fields = Map::new();
        match self {
            Target::Select { select, nth } => {
                fields.insert("select".into(), json!(select));
                if let Some(nth) = nth {
                    fields.insert("nth".into(), json!(nth));
                }
            }
            Target::Point { x, y, .. } => {
                fields.insert("x".into(), json!(round(*x)));
                fields.insert("y".into(), json!(round(*y)));
            }
        }
        fields
    }

    fn fallback(&self) -> Option<&str> {
        match self {
            Target::Point { why: Some(why), .. } => Some(why),
            _ => None,
        }
    }
}

fn round(value: f32) -> f64 {
    (f64::from(value) * 10.0).round() / 10.0
}

#[derive(Debug, Clone, PartialEq)]
enum Step {
    Wait {
        select: String,
        nth: Option<usize>,
    },
    Click {
        at: Target,
        from: (f32, f32),
        button: &'static str,
        count: usize,
        modifiers: Vec<&'static str>,
        open: bool,
    },
    Drag {
        from: Target,
        to: (f32, f32),
        modifiers: Vec<&'static str>,
    },
    Scroll {
        at: Target,
        dx: f32,
        dy: f32,
        trackpad: bool,
    },
    Keys(Vec<String>),
    Text {
        text: String,
        redacted: bool,
    },
}

#[derive(Default)]
pub(super) struct Recorder {
    steps: Vec<Step>,
    signature: Option<u64>,
}

impl Recorder {
    fn press(
        &mut self,
        tree: Option<Tree>,
        x: f32,
        y: f32,
        button: &'static str,
        count: usize,
        modifiers: Vec<&'static str>,
    ) {
        if count > 1
            && let Some(Step::Click {
                button: last,
                count: clicks,
                open,
                ..
            }) = self.steps.last_mut()
            && *last == button
            && !*open
        {
            *clicks = count;
            *open = true;
            return;
        }
        let at = self.locate(tree.as_ref(), x, y);
        self.steps.push(Step::Click {
            at,
            from: (x, y),
            button,
            count: 1,
            modifiers,
            open: true,
        });
    }

    fn release(&mut self, x: f32, y: f32) {
        let Some(Step::Click {
            at,
            from,
            modifiers,
            open: open @ true,
            ..
        }) = self.steps.last_mut()
        else {
            return;
        };
        *open = false;
        if (x - from.0).hypot(y - from.1) > DRAG_PT {
            let (from, modifiers) = (at.clone(), std::mem::take(modifiers));
            *self.steps.last_mut().expect("matched above") = Step::Drag {
                from,
                to: (x, y),
                modifiers,
            };
        }
    }

    fn scroll(
        &mut self,
        tree: &dyn Fn() -> Option<Tree>,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        started: bool,
    ) {
        if let Some(Step::Scroll {
            dx: sum_x,
            dy: sum_y,
            trackpad,
            ..
        }) = self.steps.last_mut()
        {
            *sum_x += dx;
            *sum_y += dy;
            *trackpad |= started;
            return;
        }
        let at = self.locate(tree().as_ref(), x, y);
        self.steps.push(Step::Scroll {
            at,
            dx,
            dy,
            trackpad: started,
        });
    }

    fn stroke(&mut self, tree: &dyn Fn() -> Option<Tree>, keystroke: &Keystroke) {
        if let Some(text) = typed(keystroke) {
            let secret = tree().is_none_or(|tree| tree.focus().is_some_and(secret_field));
            match self.steps.last_mut() {
                Some(Step::Text {
                    text: typed,
                    redacted,
                }) if *redacted == secret => {
                    if !secret {
                        typed.push_str(text);
                    }
                }
                _ => self.steps.push(Step::Text {
                    text: if secret {
                        String::new()
                    } else {
                        text.to_owned()
                    },
                    redacted: secret,
                }),
            }
            return;
        }
        let stroke = keystroke.unparse();
        match self.steps.last_mut() {
            Some(Step::Keys(keys)) => keys.push(stroke),
            _ => self.steps.push(Step::Keys(vec![stroke])),
        }
    }

    fn locate(&mut self, tree: Option<&Tree>, x: f32, y: f32) -> Target {
        let Some(tree) = tree else {
            return point(x, y, "no accessibility tree at the time of the event");
        };
        let signature = signature(tree);
        let changed = self
            .signature
            .replace(signature)
            .is_some_and(|before| before != signature);
        let target = target_at(tree, x, y);
        if changed && let Target::Select { select, nth } = &target {
            self.steps.push(Step::Wait {
                select: select.clone(),
                nth: *nth,
            });
        }
        target
    }

    fn scenario(&self, name: &str) -> (Value, usize, usize, usize) {
        let mut fallbacks = 0;
        let mut redacted = 0;
        let steps: Vec<Value> = self
            .steps
            .iter()
            .map(|step| {
                let (action, mut fields, fallback) = encode(step);
                fields.insert("ui".into(), json!(action));
                if let Some(why) = fallback {
                    fallbacks += 1;
                    fields.insert("name".into(), json!(format!("x,y fallback: {why}")));
                }
                if matches!(step, Step::Text { redacted: true, .. }) {
                    redacted += 1;
                }
                Value::Object(fields)
            })
            .collect();
        let count = steps.len();
        (
            json!({ "name": name, "steps": steps }),
            count,
            fallbacks,
            redacted,
        )
    }
}

fn encode(step: &Step) -> (&'static str, Map<String, Value>, Option<&str>) {
    match step {
        Step::Wait { select, nth } => {
            let mut fields = Target::Select {
                select: select.clone(),
                nth: *nth,
            }
            .fields();
            fields.insert("for".into(), json!("select"));
            ("wait", fields, None)
        }
        Step::Click {
            at,
            button,
            count,
            modifiers,
            ..
        } => {
            let mut fields = at.fields();
            if *button != "left" {
                fields.insert("button".into(), json!(button));
            }
            if *count > 1 {
                fields.insert("count".into(), json!(count));
            }
            if !modifiers.is_empty() {
                fields.insert("modifiers".into(), json!(modifiers));
            }
            ("click", fields, at.fallback())
        }
        Step::Drag {
            from,
            to,
            modifiers,
        } => {
            let mut fields = Map::new();
            fields.insert("from".into(), Value::Object(from.fields()));
            fields.insert("to".into(), json!({ "x": round(to.0), "y": round(to.1) }));
            if !modifiers.is_empty() {
                fields.insert("modifiers".into(), json!(modifiers));
            }
            ("drag", fields, from.fallback())
        }
        Step::Scroll {
            at,
            dx,
            dy,
            trackpad,
        } => {
            let mut fields = at.fields();
            fields.insert("dx".into(), json!(round(*dx)));
            fields.insert("dy".into(), json!(round(*dy)));
            if *trackpad {
                fields.insert("trackpad".into(), json!(true));
            }
            ("scroll", fields, at.fallback())
        }
        Step::Keys(keys) => {
            let mut fields = Map::new();
            fields.insert("keys".into(), json!(keys));
            ("key", fields, None)
        }
        Step::Text { text, redacted } => {
            let mut fields = Map::new();
            fields.insert(
                "text".into(),
                json!(if *redacted { REDACTED } else { text.as_str() }),
            );
            ("type", fields, None)
        }
    }
}

fn typed(keystroke: &Keystroke) -> Option<&str> {
    let held = &keystroke.modifiers;
    if held.platform || held.control || held.alt || held.function {
        return None;
    }
    keystroke
        .key_char
        .as_deref()
        .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

fn secret_field(node: &Node) -> bool {
    node.role.to_lowercase().contains("password")
        || [&node.label, &node.placeholder, &node.description]
            .into_iter()
            .flatten()
            .any(|text| {
                let text = text.to_lowercase();
                SECRET_WORDS.iter().any(|word| text.contains(word))
                    || text
                        .split(|c: char| !c.is_alphanumeric())
                        .any(|term| SECRET_TERMS.contains(&term))
            })
}

fn point(x: f32, y: f32, why: &str) -> Target {
    Target::Point {
        x,
        y,
        why: Some(why.to_owned()),
    }
}

fn signature(tree: &Tree) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for node in &tree.nodes {
        (&node.role, &node.label).hash(&mut hasher);
    }
    hasher.finish()
}

fn target_at(tree: &Tree, x: f32, y: f32) -> Target {
    let inside = |node: &Node| {
        node.bounds.is_some_and(|b| {
            !b.is_empty() && x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h
        })
    };
    let Some(deepest) = (0..tree.nodes.len())
        .filter(|&i| inside(&tree.nodes[i]))
        .max_by_key(|&i| (tree.nodes[i].depth, i))
    else {
        return point(x, y, "no node at the point");
    };
    let chain: Vec<usize> =
        std::iter::successors(Some(deepest), |&i| tree.nodes[i].parent).collect();
    let pick = match chain
        .iter()
        .copied()
        .find(|&i| !tree.nodes[i].actions.is_empty())
    {
        Some(i) if tree.nodes[i].named() => Some(i),
        Some(i) => {
            return point(
                x,
                y,
                &format!("the {} at the point has no label", tree.nodes[i].role),
            );
        }
        None => chain.iter().copied().find(|&i| {
            let node = &tree.nodes[i];
            node.named() && node.role != "Window" && !tree::GENERIC.contains(&node.role.as_str())
        }),
    };
    pick.and_then(|i| selector_for(tree, i))
        .map(|(select, nth)| Target::Select { select, nth })
        .unwrap_or_else(|| point(x, y, "no labeled node at the point"))
}

fn selector_for(tree: &Tree, index: usize) -> Option<(String, Option<usize>)> {
    let node = &tree.nodes[index];
    let typed = node.role.ends_with("Input");
    let mut best: Option<(String, Option<usize>, usize)> = None;
    for text in [
        node.label.as_ref(),
        node.value.as_ref().filter(|_| !typed),
        node.placeholder.as_ref(),
        node.description.as_ref(),
    ]
    .into_iter()
    .flatten()
    .filter(|text| !text.trim().is_empty())
    {
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        let raw = format!("{} \"{escaped}\"", node.role);
        let Ok(selector) = Selector::parse(&raw) else {
            continue;
        };
        let hits = selector.hits(&tree.nodes);
        let Some(nth) = hits.iter().position(|&hit| hit == index) else {
            continue;
        };
        if best.as_ref().is_none_or(|(.., count)| hits.len() < *count) {
            best = Some((raw, (hits.len() > 1).then_some(nth), hits.len()));
        }
        if hits.len() == 1 {
            break;
        }
    }
    best.map(|(raw, nth, _)| (raw, nth))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_driver::tree::Bounds;
    use crate::ui_driver::tree::tests::{node, tree_of};

    fn at(x: f32, y: f32, w: f32, h: f32, mut node: Node) -> Node {
        node.bounds = Some(Bounds { x, y, w, h });
        node
    }

    fn clickable(mut node: Node) -> Node {
        node.actions = vec!["click".into(), "focus".into()];
        node
    }

    fn settings() -> Tree {
        let mut icon = at(10.0, 860.0, 16.0, 16.0, node("e3", "Image", None));
        icon.actions = vec![];
        tree_of(&[
            (
                0,
                at(
                    0.0,
                    0.0,
                    800.0,
                    900.0,
                    node("e1", "Window", Some("Zeron Dev 63")),
                ),
            ),
            (
                1,
                at(
                    0.0,
                    850.0,
                    40.0,
                    40.0,
                    clickable(node("e2", "Button", Some("Settings"))),
                ),
            ),
            (2, icon),
            (
                1,
                at(
                    300.0,
                    100.0,
                    40.0,
                    20.0,
                    clickable(node("e4", "Switch", Some("Compact mode"))),
                ),
            ),
            (
                1,
                at(
                    300.0,
                    140.0,
                    40.0,
                    20.0,
                    clickable(node("e5", "Switch", Some("Compact model picker"))),
                ),
            ),
            (
                1,
                at(
                    400.0,
                    10.0,
                    30.0,
                    30.0,
                    clickable(node("e6", "Button", None)),
                ),
            ),
            (
                1,
                at(
                    500.0,
                    300.0,
                    60.0,
                    20.0,
                    clickable(node("e7", "Button", Some("Open"))),
                ),
            ),
            (
                1,
                at(
                    500.0,
                    330.0,
                    60.0,
                    20.0,
                    clickable(node("e8", "Button", Some("Open"))),
                ),
            ),
        ])
    }

    fn keystroke(raw: &str, key_char: Option<&str>) -> Keystroke {
        let mut keystroke = Keystroke::parse(raw).unwrap();
        keystroke.key_char = key_char.map(str::to_owned);
        keystroke
    }

    #[test]
    fn real_input_maps_to_selector_steps_with_waits_nth_and_flagged_fallbacks() {
        let home = settings();
        let mut opened = settings();
        opened.nodes[2].label = Some("Settings open".into());
        let mut recorder = Recorder::default();
        recorder.press(Some(home.clone()), 18.0, 868.0, "left", 1, vec![]);
        recorder.release(18.0, 868.0);
        recorder.press(Some(opened.clone()), 310.0, 110.0, "left", 1, vec![]);
        recorder.release(310.0, 110.0);
        recorder.press(Some(opened.clone()), 310.0, 110.0, "left", 1, vec![]);
        recorder.release(310.0, 110.0);
        recorder.press(Some(opened.clone()), 520.0, 340.0, "right", 1, vec!["cmd"]);
        recorder.release(520.0, 340.0);
        recorder.press(Some(opened.clone()), 410.0, 20.0, "left", 1, vec![]);
        recorder.release(410.0, 20.0);
        recorder.press(Some(opened.clone()), 700.0, 700.0, "left", 1, vec![]);
        recorder.release(700.0, 700.0);
        recorder.press(None, 5.0, 5.0, "left", 1, vec![]);
        recorder.release(5.0, 5.0);
        for (raw, key_char) in [
            ("h", Some("h")),
            ("shift-i", Some("I")),
            ("space", Some(" ")),
            ("enter", Some("\n")),
            ("cmd-k", Some("k")),
            ("escape", None),
            ("x", Some("x")),
        ] {
            recorder.stroke(&|| Some(opened.clone()), &keystroke(raw, key_char));
        }
        let wheel = || Some(opened.clone());
        recorder.scroll(&wheel, 310.0, 150.0, 0.0, -40.0, true);
        recorder.scroll(&wheel, 310.0, 160.0, 0.0, -60.5, false);
        recorder.press(Some(opened.clone()), 310.0, 110.0, "left", 1, vec![]);
        recorder.release(380.0, 110.0);
        let (scenario, steps, fallbacks, redacted) = recorder.scenario("compact");
        assert_eq!(
            scenario,
            json!({ "name": "compact", "steps": [
                { "ui": "click", "select": "Button \"Settings\"" },
                { "ui": "wait", "for": "select", "select": "Switch \"Compact mode\"" },
                { "ui": "click", "select": "Switch \"Compact mode\"" },
                { "ui": "click", "select": "Switch \"Compact mode\"" },
                { "ui": "click", "select": "Button \"Open\"", "nth": 1, "button": "right", "modifiers": ["cmd"] },
                { "ui": "click", "x": 410.0, "y": 20.0, "name": "x,y fallback: the Button at the point has no label" },
                { "ui": "click", "x": 700.0, "y": 700.0, "name": "x,y fallback: no labeled node at the point" },
                { "ui": "click", "x": 5.0, "y": 5.0, "name": "x,y fallback: no accessibility tree at the time of the event" },
                { "ui": "type", "text": "hI " },
                { "ui": "key", "keys": ["enter", "cmd-k", "escape"] },
                { "ui": "type", "text": "x" },
                { "ui": "scroll", "select": "Switch \"Compact model picker\"", "dx": 0.0, "dy": -100.5, "trackpad": true },
                { "ui": "drag", "from": { "select": "Switch \"Compact mode\"" }, "to": { "x": 380.0, "y": 110.0 } },
            ] })
        );
        assert_eq!((steps, fallbacks, redacted), (13, 3, 0));
    }

    #[test]
    fn key_and_api_count_only_as_whole_words() {
        let labelled = |label: &str| node("e1", "TextInput", Some(label));
        for label in [
            "API key",
            "api_key",
            "Signing Key",
            "https://api.openai.com/v1",
        ] {
            assert!(secret_field(&labelled(label)), "{label}");
        }
        for label in ["Hotkey", "Keychain item", "Keyboard layout", "Rapid notes"] {
            assert!(!secret_field(&labelled(label)), "{label}");
        }
    }

    #[test]
    fn typing_into_a_key_token_or_password_field_records_redacted_text() {
        let focused_on = |mut field: Node| {
            field.focused = true;
            let tree = tree_of(&[(0, node("e1", "Window", Some("Zeron Dev 67"))), (1, field)]);
            move || Some(tree.clone())
        };
        let mut api = node("e2", "MultilineTextInput", None);
        api.placeholder = Some("OpenAI API Key".into());
        let password = node("e3", "PasswordInput", None);
        let draft = node("e4", "MultilineTextInput", Some("Draft"));
        let mut notes = node("e5", "TextInput", Some("Notes"));
        notes.description = Some("Personal access Token".into());
        let mut recorder = Recorder::default();
        let type_into = |recorder: &mut Recorder, tree: &dyn Fn() -> Option<Tree>, text: &str| {
            for c in text.chars() {
                let raw = c.to_string();
                recorder.stroke(tree, &keystroke(&raw, Some(&raw)));
            }
        };
        type_into(&mut recorder, &focused_on(api), "sk-1");
        type_into(&mut recorder, &focused_on(password), "pw");
        type_into(&mut recorder, &focused_on(draft.clone()), "hi");
        type_into(&mut recorder, &|| None, "x");
        type_into(&mut recorder, &focused_on(draft), "y");
        recorder.stroke(&|| None, &keystroke("tab", None));
        type_into(&mut recorder, &focused_on(notes), "t");
        let (scenario, steps, _, redacted) = recorder.scenario("secrets");
        assert_eq!(
            scenario["steps"],
            json!([
                { "ui": "type", "text": "<redacted>" },
                { "ui": "type", "text": "hi" },
                { "ui": "type", "text": "<redacted>" },
                { "ui": "type", "text": "y" },
                { "ui": "key", "keys": ["tab"] },
                { "ui": "type", "text": "<redacted>" },
            ])
        );
        assert_eq!((steps, redacted), (6, 3));
    }

    #[test]
    fn a_double_click_folds_into_one_click_and_selectors_escape_quotes() {
        let mut quoted = settings();
        quoted.nodes[1].label = Some("Say \"hi\" \\ now".into());
        let mut recorder = Recorder::default();
        recorder.press(Some(quoted.clone()), 18.0, 868.0, "left", 1, vec![]);
        recorder.release(18.0, 868.0);
        recorder.press(Some(quoted.clone()), 18.0, 868.0, "left", 2, vec![]);
        recorder.release(18.0, 868.0);
        let (scenario, ..) = recorder.scenario("double");
        let select = scenario["steps"][0]["select"].as_str().unwrap().to_owned();
        assert_eq!(scenario["steps"][0]["count"], 2);
        assert_eq!(scenario["steps"].as_array().unwrap().len(), 1);
        let parsed = Selector::parse(&select).unwrap();
        assert_eq!(parsed.hits(&quoted.nodes), vec![1]);
    }

    #[test]
    fn record_requests_parse_and_stop_paths_are_absolute_json_files() {
        assert_eq!(
            serde_json::from_value::<Record>(json!({ "do": "start", "includeDriven": true }))
                .unwrap(),
            Record::Start {
                include_driven: true
            }
        );
        assert_eq!(
            serde_json::from_value::<Record>(json!({ "do": "stop" })).unwrap(),
            Record::Stop { path: None }
        );
        assert!(serde_json::from_value::<Record>(json!({ "do": "pause" })).is_err());
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(check_path(&dir.path().join("a.json")), Ok(()));
        assert!(
            check_path(Path::new("rel.json"))
                .unwrap_err()
                .contains("absolute")
        );
        assert!(
            check_path(&dir.path().join("a.png"))
                .unwrap_err()
                .contains(".json")
        );
        assert!(check_path(dir.path()).is_err());
    }
}
