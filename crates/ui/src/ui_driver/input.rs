use super::{frame_count, frames, next_frame, target_window, tree};
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
#[serde(try_from = "RawTarget")]
pub(super) enum Target {
    Point { x: f32, y: f32 },
    Pair([f32; 2]),
    Element(tree::Element),
}

#[derive(Deserialize)]
#[serde(
    untagged,
    expecting = "a target: \"ref\", \"select\" (with optional \"nth\"), or numeric \"x\" and \"y\" in window points"
)]
enum RawTarget {
    Pair([f32; 2]),
    Fields(TargetFields),
}

#[derive(Deserialize)]
struct TargetFields {
    #[serde(rename = "ref")]
    id: Option<String>,
    select: Option<String>,
    nth: Option<usize>,
    x: Option<f32>,
    y: Option<f32>,
}

const ONE_TARGET: &str = "give one of ref, select, or x and y";

impl TargetFields {
    fn target(self) -> Result<Option<Target>, String> {
        match self {
            TargetFields {
                id: None,
                select: None,
                nth: None,
                x: None,
                y: None,
            } => Ok(None),
            TargetFields {
                id: None,
                select: None,
                nth: None,
                x: Some(x),
                y: Some(y),
            } => Ok(Some(Target::Point { x, y })),
            TargetFields {
                id: Some(id),
                select: None,
                nth: None,
                x: None,
                y: None,
            } => Ok(Some(Target::Element(tree::Element::Ref { id }))),
            TargetFields {
                id: None,
                select: Some(select),
                nth,
                x: None,
                y: None,
            } => Ok(Some(Target::Element(tree::Element::Select { select, nth }))),
            _ => Err(ONE_TARGET.into()),
        }
    }
}

impl TryFrom<RawTarget> for Target {
    type Error = String;

    fn try_from(raw: RawTarget) -> Result<Self, String> {
        match raw {
            RawTarget::Pair(pair) => Ok(Target::Pair(pair)),
            RawTarget::Fields(fields) => fields.target()?.ok_or_else(|| ONE_TARGET.into()),
        }
    }
}

impl Target {
    async fn point(&self, driver: &mut Driver, cx: &mut AsyncApp) -> Result<Point<Pixels>, String> {
        match self {
            Target::Point { x, y } | Target::Pair([x, y]) => Ok(point(px(*x), px(*y))),
            Target::Element(element) => {
                let (node, viewport) = driver.find(element, cx).await?;
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
#[serde(try_from = "TargetFields")]
pub(super) enum TypeAt {
    Target(Target),
    Focused,
}

impl TryFrom<TargetFields> for TypeAt {
    type Error = String;

    fn try_from(fields: TargetFields) -> Result<Self, String> {
        Ok(fields.target()?.map_or(TypeAt::Focused, TypeAt::Target))
    }
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
    unsettled: bool,
}

impl Driver {
    fn open(window: Option<u64>, cx: &mut AsyncApp) -> Result<Self, String> {
        let window = cx.update(|cx| target_window(window, cx))?;
        Ok(Self {
            window,
            mark: frame_count(),
            unsettled: false,
        })
    }

    async fn find(
        &mut self,
        element: &tree::Element,
        cx: &mut AsyncApp,
    ) -> Result<(tree::Node, (f32, f32)), String> {
        let (node, viewport, unsettled) = frames::settled(self.window, element, cx).await?;
        self.unsettled |= unsettled;
        Ok((node, viewport))
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
        let drawn = next_frame(self.mark, SETTLE_CAP.saturating_sub(STEP_CAP), cx).await;
        let mut reply = json!({ "frame": frame_count() });
        if self.unsettled {
            reply["unsettled"] = json!(true);
        }
        if drawn.is_none() {
            reply["stale"] = json!(true);
        }
        Ok(reply)
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
    let (position, modifiers) = (at.point(&mut driver, cx).await?, modifiers(held));
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
    let position = at.point(&mut driver, cx).await?;
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
        from.point(&mut driver, cx).await?,
        to.point(&mut driver, cx).await?,
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
    let position = at.point(&mut driver, cx).await?;
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
        TypeAt::Focused => None,
        TypeAt::Target(Target::Element(element)) => {
            let (node, viewport) = driver.find(&element, cx).await?;
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
        TypeAt::Target(at) => Some(at.point(&mut driver, cx).await?),
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

fn performable(
    node: &tree::Node,
    action: AxAction,
    viewport: (f32, f32),
) -> Result<String, String> {
    let name = tree::lower_camel(&format!("{action:?}"));
    if !node.supports(&name) {
        if action == AxAction::Click {
            // gpui answers an unlistened Click with a synthetic mouse click at the node's center.
            center(node, viewport)?;
        } else {
            return Err(format!(
                "{} does not support `{name}`; it lists [{}]",
                tree::line(node),
                node.actions.join(",")
            ));
        }
    }
    Ok(name)
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
    let (node, viewport) = driver.find(&element, cx).await?;
    let name = performable(&node, action, viewport)?;
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

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(
    untagged,
    expecting = "a window size: {\"w\", \"h\"} or [w, h] in points"
)]
pub(super) enum Size {
    Object { w: f32, h: f32 },
    Pair([f32; 2]),
}

impl Size {
    fn points(self) -> (f32, f32) {
        match self {
            Size::Object { w, h } | Size::Pair([w, h]) => (w, h),
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Set {
    theme: Option<crate::appearance::AppearanceMode>,
    size: Option<Size>,
    motion_scale: Option<f32>,
}

pub(super) async fn set(window: Option<u64>, set: Set, cx: &mut AsyncApp) -> Result<Value, String> {
    if set.theme.is_none() && set.size.is_none() && set.motion_scale.is_none() {
        return Err("`set` needs `theme`, `size` or `motionScale`".into());
    }
    if let Some(scale) = set.motion_scale.filter(|s| !s.is_finite() || *s <= 0.0) {
        return Err(format!(
            "`motionScale` must be a positive number, not {scale}"
        ));
    }
    let wanted = set.size.map(Size::points);
    if let Some((w, h)) =
        wanted.filter(|(w, h)| !(w.is_finite() && h.is_finite() && *w >= 1.0 && *h >= 1.0))
    {
        return Err(format!("`size` must be positive points, not {w}x{h}"));
    }
    let mut driver = Driver::open(window, cx)?;
    driver.apply(cx, |window, cx| {
        if let Some(mode) = set.theme {
            crate::appearance::set_mode(mode, cx);
        }
        if let Some(scale) = set.motion_scale {
            crate::motion::set_speed_scale(scale);
        }
        if let Some((w, h)) = wanted {
            window.resize(gpui::size(px(w), px(h)));
        }
        window.refresh();
    })?;
    let handle = driver.window;
    let viewport = |cx: &mut AsyncApp| {
        cx.update_window(handle, |_, window, _| {
            let size = window.viewport_size();
            (f32::from(size.width), f32::from(size.height))
        })
        .map_err(|error| format!("window is gone: {error}"))
    };
    let mut reply = driver.settle(cx).await?;
    if let Some(wanted) = wanted {
        for _ in 0..5 {
            if viewport(cx)? == wanted {
                break;
            }
            let mark = frame_count();
            next_frame(mark, STEP_CAP, cx).await;
            reply["frame"] = json!(frame_count());
        }
    }
    let (w, h) = viewport(cx)?;
    reply["size"] = json!({ "w": w, "h": h });
    reply["motionScale"] = json!(crate::motion::speed_scale());
    reply["theme"] = cx.update(super::theme_json);
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree::tests::node;

    #[test]
    fn center_points_inside_the_window_only() {
        let button = node("e1", "Button", Some("Go"));
        assert_eq!(
            center(&button, (800.0, 600.0)),
            Ok(point(px(60.0), px(35.0)))
        );
        let outside = center(&button, (50.0, 600.0)).unwrap_err();
        assert!(outside.contains("outside the 50x600 window"), "{outside}");
        let mut empty = button.clone();
        empty.bounds = Some(tree::Bounds::default());
        assert!(
            center(&empty, (800.0, 600.0))
                .unwrap_err()
                .contains("no on-screen bounds")
        );
        empty.bounds = None;
        assert!(center(&empty, (800.0, 600.0)).is_err());
    }

    #[test]
    fn ax_action_names_are_case_insensitive() {
        assert_eq!(ax_action("click"), Ok(AxAction::Click));
        assert_eq!(ax_action("SETVALUE"), Ok(AxAction::SetValue));
        let error = ax_action("explode").unwrap_err();
        assert!(
            error.starts_with("unknown a11y action `explode`"),
            "{error}"
        );
        assert!(
            error.contains("click") && error.contains("setValue"),
            "{error}"
        );
    }

    #[test]
    fn press_refuses_the_offscreen_click_fallback() {
        let mut plain = node("e1", "GenericContainer", Some("Row"));
        plain.bounds.as_mut().unwrap().y = 900.0;
        let error = performable(&plain, AxAction::Click, (800.0, 600.0)).unwrap_err();
        assert!(error.contains("outside the 800x600 window"), "{error}");
        plain.bounds.as_mut().unwrap().y = 20.0;
        assert_eq!(
            performable(&plain, AxAction::Click, (800.0, 600.0)),
            Ok("click".into())
        );
        let mut listened = plain.clone();
        listened.actions = vec!["click".into()];
        listened.bounds.as_mut().unwrap().y = 900.0;
        assert_eq!(
            performable(&listened, AxAction::Click, (800.0, 600.0)),
            Ok("click".into())
        );
        let error = performable(&plain, AxAction::Expand, (800.0, 600.0)).unwrap_err();
        assert!(error.contains("does not support `expand`"), "{error}");
    }
}
