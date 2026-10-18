use super::tree;
use super::{frame_count, next_frame, target_window};
use gpui::accesskit::{Action as AxAction, ActionData, NodeId};
use gpui::{
    AnyWindowHandle, AppContext as _, AsyncApp, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput, Point, ScrollDelta, ScrollWheelEvent,
    TouchPhase, point, px,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

const STEP_CAP: Duration = Duration::from_millis(100);
const SETTLE_CAP: Duration = Duration::from_millis(500);
const TRACKPAD_STEPS: u32 = 6;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(
    untagged,
    expecting = "a target: \"ref\", \"select\" (with optional \"nth\"), or numeric \"x\" and \"y\" in window points"
)]
pub(super) enum Target {
    Point { x: f32, y: f32 },
    Pair([f32; 2]),
    Element(tree::Element),
}

impl Target {
    async fn point(
        &self,
        window: AnyWindowHandle,
        cx: &mut AsyncApp,
    ) -> Result<Point<Pixels>, String> {
        match self {
            Target::Point { x, y } | Target::Pair([x, y]) => Ok(point(px(*x), px(*y))),
            Target::Element(element) => {
                let (node, viewport) = tree::find(window, element, cx).await?;
                center(&node, viewport)
            }
        }
    }
}

fn center(node: &tree::Node, (width, height): (f32, f32)) -> Result<Point<Pixels>, String> {
    let Some(b) = node.bounds.filter(|b| !b.is_empty()) else {
        return Err(format!(
            "{} has no on-screen bounds to point at",
            tree::line(node)
        ));
    };
    let (x, y) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    if x < 0.0 || y < 0.0 || x >= width || y >= height {
        return Err(format!(
            "{} is outside the {width}x{height} window; scroll it into view first",
            tree::line(node)
        ));
    }
    Ok(point(px(x), px(y)))
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub(super) enum TypeAt {
    Target(Target),
    Focused {},
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Button {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Modifier {
    Cmd,
    Shift,
    Alt,
    Ctrl,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub(super) enum Keys {
    One(String),
    Many(Vec<String>),
}

impl Keys {
    pub(super) fn strokes(&self) -> Vec<&str> {
        match self {
            Keys::One(keys) => keys.split_whitespace().collect(),
            Keys::Many(keys) => keys.iter().flat_map(|k| k.split_whitespace()).collect(),
        }
    }
}

fn modifiers(list: &[Modifier]) -> Modifiers {
    let mut modifiers = Modifiers::default();
    for modifier in list {
        match modifier {
            Modifier::Cmd => modifiers.platform = true,
            Modifier::Shift => modifiers.shift = true,
            Modifier::Alt => modifiers.alt = true,
            Modifier::Ctrl => modifiers.control = true,
        }
    }
    modifiers
}

struct Driver {
    window: AnyWindowHandle,
    mark: u64,
}

impl Driver {
    fn open(window: Option<u64>, cx: &mut AsyncApp) -> Result<Self, String> {
        let window = cx.update(|cx| target_window(window, cx))?;
        Ok(Self {
            window,
            mark: frame_count(),
        })
    }

    fn apply<R>(
        &mut self,
        cx: &mut AsyncApp,
        f: impl FnOnce(&mut gpui::Window, &mut gpui::App) -> R,
    ) -> Result<R, String> {
        let result = cx
            .update_window(self.window, |_, window, cx| f(window, cx))
            .map_err(|error| format!("window is gone: {error}"));
        // Key dispatch may draw inline before its handlers run; only a later frame shows their effect.
        self.mark = frame_count();
        result
    }

    async fn send(
        &mut self,
        cx: &mut AsyncApp,
        events: impl IntoIterator<Item = PlatformInput>,
    ) -> Result<(), String> {
        self.apply(cx, |window, cx| {
            for event in events {
                window.dispatch_event(event, cx);
            }
        })?;
        self.step(cx).await;
        Ok(())
    }

    async fn step(&self, cx: &AsyncApp) {
        next_frame(self.mark, STEP_CAP, cx).await;
    }

    async fn settle(self, cx: &AsyncApp) -> Result<Value, String> {
        next_frame(self.mark, SETTLE_CAP.saturating_sub(STEP_CAP), cx).await;
        Ok(json!({ "frame": frame_count() }))
    }
}

fn mouse_move(
    position: Point<Pixels>,
    pressed_button: Option<MouseButton>,
    modifiers: Modifiers,
) -> PlatformInput {
    PlatformInput::MouseMove(MouseMoveEvent {
        position,
        pressed_button,
        modifiers,
    })
}

fn mouse_press(
    position: Point<Pixels>,
    button: MouseButton,
    modifiers: Modifiers,
    click_count: usize,
) -> [PlatformInput; 2] {
    [
        PlatformInput::MouseDown(MouseDownEvent {
            button,
            position,
            modifiers,
            click_count,
            first_mouse: false,
        }),
        PlatformInput::MouseUp(MouseUpEvent {
            button,
            position,
            modifiers,
            click_count,
        }),
    ]
}

pub(super) async fn click(
    window: Option<u64>,
    at: Target,
    button: Button,
    count: usize,
    held: &[Modifier],
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let mut driver = Driver::open(window, cx)?;
    let (position, modifiers) = (at.point(driver.window, cx).await?, modifiers(held));
    let button = match button {
        Button::Left => MouseButton::Left,
        Button::Right => MouseButton::Right,
        Button::Middle => MouseButton::Middle,
    };
    driver
        .send(cx, [mouse_move(position, None, modifiers)])
        .await?;
    for click_count in 1..=count.max(1) {
        driver
            .send(cx, mouse_press(position, button, modifiers, click_count))
            .await?;
    }
    driver.settle(cx).await
}

pub(super) async fn hover(
    window: Option<u64>,
    at: Target,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let mut driver = Driver::open(window, cx)?;
    let position = at.point(driver.window, cx).await?;
    driver
        .send(cx, [mouse_move(position, None, Modifiers::default())])
        .await?;
    driver.settle(cx).await
}

pub(super) async fn drag(
    window: Option<u64>,
    from: Target,
    to: Target,
    steps: u32,
    hold_ms: u64,
    held: &[Modifier],
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let mut driver = Driver::open(window, cx)?;
    let modifiers = modifiers(held);
    let (from, to) = (
        from.point(driver.window, cx).await?,
        to.point(driver.window, cx).await?,
    );
    let at = |t: f32| from + (to - from) * t;
    let [down, _] = mouse_press(at(0.0), MouseButton::Left, modifiers, 1);
    driver
        .send(cx, [mouse_move(at(0.0), None, modifiers), down])
        .await?;
    let steps = steps.max(1);
    for step in 1..=steps {
        let position = at(step as f32 / steps as f32);
        driver
            .send(
                cx,
                [mouse_move(position, Some(MouseButton::Left), modifiers)],
            )
            .await?;
    }
    if hold_ms > 0 {
        cx.background_executor()
            .timer(Duration::from_millis(hold_ms))
            .await;
    }
    let [_, up] = mouse_press(at(1.0), MouseButton::Left, modifiers, 1);
    driver.send(cx, [up]).await?;
    driver.settle(cx).await
}

pub(super) async fn scroll(
    window: Option<u64>,
    at: Target,
    dx: f32,
    dy: f32,
    trackpad: bool,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let mut driver = Driver::open(window, cx)?;
    let position = at.point(driver.window, cx).await?;
    let wheel = |dx: f32, dy: f32, touch_phase| {
        PlatformInput::ScrollWheel(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
            modifiers: Modifiers::default(),
            touch_phase,
        })
    };
    if trackpad {
        driver
            .send(cx, [wheel(0.0, 0.0, TouchPhase::Started)])
            .await?;
        let n = TRACKPAD_STEPS as f32;
        for _ in 0..TRACKPAD_STEPS {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            driver
                .send(cx, [wheel(dx / n, dy / n, TouchPhase::Moved)])
                .await?;
        }
        driver
            .send(cx, [wheel(0.0, 0.0, TouchPhase::Ended)])
            .await?;
    } else {
        driver.send(cx, [wheel(dx, dy, TouchPhase::Moved)]).await?;
    }
    driver.settle(cx).await
}

pub(super) async fn keys(
    window: Option<u64>,
    keys: Keys,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let strokes = keys
        .strokes()
        .into_iter()
        .map(|stroke| {
            Keystroke::parse(stroke).map_err(|error| format!("bad keystroke `{stroke}`: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if strokes.is_empty() {
        return Err("`keys` is empty".into());
    }
    let mut driver = Driver::open(window, cx)?;
    let mut handled = Vec::new();
    for stroke in strokes {
        handled.push(driver.apply(cx, |window, cx| window.dispatch_keystroke(stroke, cx))?);
        driver.step(cx).await;
    }
    let mut reply = driver.settle(cx).await?;
    reply["handled"] = json!(handled);
    Ok(reply)
}

fn typed(ch: char) -> Keystroke {
    let key = match ch {
        ' ' => "space".to_owned(),
        '\n' => "enter".to_owned(),
        '\t' => "tab".to_owned(),
        _ => ch.to_lowercase().collect(),
    };
    Keystroke {
        modifiers: Modifiers::default(),
        key,
        key_char: Some(ch.to_string()),
    }
}

pub(super) async fn type_text(
    window: Option<u64>,
    text: &str,
    into: TypeAt,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let mut driver = Driver::open(window, cx)?;
    let position = match into {
        TypeAt::Focused {} => None,
        TypeAt::Target(Target::Element(element)) => {
            let (node, viewport) = tree::find(driver.window, &element, cx).await?;
            if node.supports("focus") {
                let id = NodeId(node.accesskit_id);
                driver.apply(cx, |window, cx| {
                    window.perform_a11y_action(id, AxAction::Focus, None, cx)
                })?;
                driver.step(cx).await;
                None
            } else {
                Some(center(&node, viewport)?)
            }
        }
        TypeAt::Target(at) => Some(at.point(driver.window, cx).await?),
    };
    if let Some(position) = position {
        driver
            .send(cx, [mouse_move(position, None, Modifiers::default())])
            .await?;
        driver
            .send(
                cx,
                mouse_press(position, MouseButton::Left, Modifiers::default(), 1),
            )
            .await?;
    }
    for ch in text.chars() {
        driver.apply(cx, |window, cx| window.dispatch_keystroke(typed(ch), cx))?;
        driver.step(cx).await;
    }
    driver.settle(cx).await
}

pub(super) async fn action(
    window: Option<u64>,
    name: &str,
    args: Option<Value>,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let mut driver = Driver::open(window, cx)?;
    driver.apply(cx, |window, cx| {
        let action = cx
            .build_action(name, args)
            .map_err(|error| format!("cannot build action `{name}`: {error}"))?;
        window.dispatch_action(action, cx);
        Ok::<_, String>(())
    })??;
    driver.settle(cx).await
}

fn ax_action(name: &str) -> Result<AxAction, String> {
    let all = || (0..=u8::MAX).map_while(AxAction::n);
    all()
        .find(|action| format!("{action:?}").eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            let names: Vec<String> = all()
                .map(|action| tree::lower_camel(&format!("{action:?}")))
                .collect();
            format!("unknown a11y action `{name}`; valid: {}", names.join(", "))
        })
}

pub(super) async fn press(
    window: Option<u64>,
    element: tree::Element,
    perform: Option<String>,
    value: Option<String>,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let action = ax_action(perform.as_deref().unwrap_or("click"))?;
    let mut driver = Driver::open(window, cx)?;
    let (node, viewport) = tree::find(driver.window, &element, cx).await?;
    let name = tree::lower_camel(&format!("{action:?}"));
    if !node.supports(&name) {
        if action == AxAction::Click {
            // gpui answers an unlistened Click with a synthetic mouse click at the node's center.
            center(&node, viewport)?;
        } else {
            return Err(format!(
                "{} does not support `{name}`; it lists [{}]",
                tree::line(&node),
                node.actions.join(",")
            ));
        }
    }
    let data = match (action, value) {
        (AxAction::SetValue, Some(value)) => Some(ActionData::Value(value.into())),
        (AxAction::SetValue, None) => return Err("`setValue` needs a string `value`".into()),
        (_, Some(_)) => return Err(format!("`{name}` takes no `value`")),
        (_, None) => None,
    };
    let id = NodeId(node.accesskit_id);
    driver.apply(cx, |window, cx| {
        window.perform_a11y_action(id, action, data, cx)
    })?;
    let mut reply = driver.settle(cx).await?;
    reply["ref"] = json!(node.id);
    Ok(reply)
}
