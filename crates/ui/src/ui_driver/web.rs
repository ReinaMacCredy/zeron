use super::tree::{Bounds as Rect, Node, Tree};
use gpui::{AnyWindowHandle, AsyncApp, BackgroundExecutor, Bounds, Pixels};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::{WKContentWorld, WKWebView};
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const WALK_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_RESULT: usize = 1024 * 1024;

pub(crate) struct Page {
    pub(crate) view: Retained<WKWebView>,
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) visible: Bounds<Pixels>,
}

pub(crate) trait Hosted {
    fn page(&self) -> Option<Page>;
    fn pressing(&self) {}
}

thread_local! {
    static PAGES: RefCell<Vec<(u64, Weak<dyn Hosted>)>> = const { RefCell::new(Vec::new()) };
}

static NEXT_VIEW: AtomicU64 = AtomicU64::new(0);

pub(crate) fn register(host: Weak<dyn Hosted>) {
    let id = NEXT_VIEW.fetch_add(1, Ordering::Relaxed) + 1;
    PAGES.with(|pages| {
        let mut pages = pages.borrow_mut();
        pages.retain(|(_, host)| host.strong_count() > 0);
        pages.push((id, host));
    });
}

fn live() -> Vec<(u64, Rc<dyn Hosted>, Page)> {
    PAGES.with(|pages| {
        pages
            .borrow()
            .iter()
            .filter_map(|(id, host)| {
                let host = host.upgrade()?;
                let page = host.page()?;
                Some((*id, host, page))
            })
            .collect()
    })
}

fn in_window(window: &gpui::Window) -> Vec<(u64, Rc<dyn Hosted>, Page)> {
    let Some(number) = crate::window_capture::window_number(window) else {
        return Vec::new();
    };
    live()
        .into_iter()
        .filter(|(_, _, page)| {
            page.view
                .window()
                .is_some_and(|w| w.windowNumber() == number as isize)
        })
        .collect()
}

pub(crate) fn script(request: &Value) -> String {
    format!(
        "return JSON.stringify({});",
        include_str!("web.js").replace("__REQUEST__", &request.to_string())
    )
}

pub(crate) fn evaluate(
    view: &WKWebView,
    body: &str,
    page_world: bool,
) -> futures::channel::oneshot::Receiver<Result<Value, String>> {
    let (tx, rx) = futures::channel::oneshot::channel();
    let Some(mtm) = MainThreadMarker::new() else {
        let _ = tx.send(Err("pages are read on the main thread".into()));
        return rx;
    };
    let tx = RefCell::new(Some(tx));
    let completion = block2::RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
        let result = if let Some(error) = unsafe { error.as_ref() } {
            Err(error.localizedDescription().to_string())
        } else {
            unsafe { value.as_ref() }
                .and_then(|value| value.downcast_ref::<NSString>())
                .ok_or_else(|| "The page did not answer with JSON text.".to_string())
                .and_then(|text| {
                    let text = text.to_string();
                    if text.len() > MAX_RESULT {
                        return Err("The page's answer exceeds 1 MiB.".into());
                    }
                    serde_json::from_str(&text).map_err(|e| e.to_string())
                })
        };
        if let Some(tx) = tx.borrow_mut().take() {
            let _ = tx.send(result);
        }
    });
    unsafe {
        let world = if page_world {
            WKContentWorld::pageWorld(mtm)
        } else {
            WKContentWorld::defaultClientWorld(mtm)
        };
        view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(body),
            None,
            None,
            &world,
            Some(&completion),
        );
    }
    rx
}

async fn ask(
    view: &WKWebView,
    request: Value,
    executor: &BackgroundExecutor,
) -> Result<Value, String> {
    let answer = evaluate(view, &script(&request), false);
    let timeout = executor.timer(WALK_TIMEOUT);
    futures::pin_mut!(timeout);
    match futures::future::select(answer, timeout).await {
        futures::future::Either::Left((answer, _)) => {
            answer.map_err(|_| "the page dropped the request".to_string())?
        }
        futures::future::Either::Right(_) => Err(format!(
            "the page did not answer within {} ms",
            WALK_TIMEOUT.as_millis()
        )),
    }
}

#[derive(Debug, Default, Deserialize)]
struct Walk {
    #[serde(default)]
    title: String,
    #[serde(default)]
    nodes: Vec<WalkNode>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Debug, Deserialize)]
struct WalkNode {
    n: u64,
    parent: i64,
    role: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    placeholder: String,
    #[serde(default)]
    description: String,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    #[serde(default)]
    focused: bool,
    toggled: Option<String>,
    selected: Option<bool>,
    expanded: Option<bool>,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    actions: Vec<String>,
}

fn text(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn nodes(view: u64, walk: &Walk, origin: (f32, f32), clip: Rect, focus: bool) -> Vec<Node> {
    let mut out = vec![Node {
        id: format!("w{view}-0"),
        role: "WebView".into(),
        label: text(&walk.title),
        description: walk
            .truncated
            .then(|| format!("only the first {} page nodes", walk.nodes.len())),
        bounds: Some(clip),
        web: true,
        ..Node::default()
    }];
    let mut placed: Vec<Option<usize>> = Vec::with_capacity(walk.nodes.len());
    for raw in &walk.nodes {
        let up = usize::try_from(raw.parent)
            .ok()
            .and_then(|p| placed.get(p).copied().flatten())
            .unwrap_or(0);
        let bounds = Rect {
            x: origin.0 + raw.x,
            y: origin.1 + raw.y,
            w: raw.w,
            h: raw.h,
        };
        if !overlaps(&bounds, &clip) {
            placed.push(Some(up));
            continue;
        }
        placed.push(Some(out.len()));
        out.push(Node {
            id: format!("w{view}-{}", raw.n),
            role: super::selector::role_name(&raw.role)
                .unwrap_or_else(|| "GenericContainer".into()),
            label: text(&raw.name),
            value: text(&raw.value),
            description: text(&raw.description),
            placeholder: text(&raw.placeholder),
            bounds: Some(bounds),
            actions: raw.actions.clone(),
            focused: focus && raw.focused,
            selected: raw.selected,
            expanded: raw.expanded,
            toggled: raw.toggled.clone(),
            disabled: raw.disabled,
            depth: out[up].depth + 1,
            parent: Some(up),
            web: true,
            ..Node::default()
        });
    }
    out
}

fn host_of(tree: &Tree, page: &Rect) -> usize {
    let holds = |b: &Rect| {
        b.x <= page.x + 1.0
            && b.y <= page.y + 1.0
            && b.x + b.w >= page.x + page.w - 1.0
            && b.y + b.h >= page.y + page.h - 1.0
    };
    tree.nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| !node.web && node.bounds.is_some_and(|b| !b.is_empty() && holds(&b)))
        .max_by(|(_, a), (_, b)| {
            let area = |n: &Node| n.bounds.map_or(0.0, |b| b.w * b.h);
            a.depth.cmp(&b.depth).then(area(b).total_cmp(&area(a)))
        })
        .map_or(0, |(i, _)| i)
}

fn insert(tree: &mut Tree, host: usize, nodes: Vec<Node>) {
    if tree.nodes.is_empty() {
        tree.nodes = nodes;
        return;
    }
    let base = tree.nodes[host].depth + 1;
    let end = (host + 1..tree.nodes.len())
        .find(|&i| tree.nodes[i].depth < base)
        .unwrap_or(tree.nodes.len());
    let count = nodes.len();
    for node in &mut tree.nodes[end..] {
        if let Some(parent) = node.parent.as_mut().filter(|p| **p >= end) {
            *parent += count;
        }
    }
    let placed = nodes.into_iter().map(|mut node| {
        node.depth += base;
        node.parent = Some(node.parent.map_or(host, |p| end + p));
        node
    });
    tree.nodes.splice(end..end, placed);
}

pub(super) async fn splice(tree: &mut Tree, handle: AnyWindowHandle, cx: &mut AsyncApp) {
    let Ok(pages) = cx.update(|cx| handle.update(cx, |_, window, _| in_window(window))) else {
        return;
    };
    let window = Rect {
        x: 0.0,
        y: 0.0,
        w: tree.size.0,
        h: tree.size.1,
    };
    for (id, _, page) in pages {
        let origin = (
            f32::from(page.bounds.origin.x),
            f32::from(page.bounds.origin.y),
        );
        let full = Rect {
            x: origin.0,
            y: origin.1,
            w: f32::from(page.bounds.size.width),
            h: f32::from(page.bounds.size.height),
        };
        let clip = intersect(&full, &window);
        if clip.is_empty() {
            continue;
        }
        let focus = has_focus(&page.view);
        let nodes = match ask(
            &page.view,
            json!({ "op": "walk" }),
            cx.background_executor(),
        )
        .await
        {
            Ok(value) => match serde_json::from_value::<Walk>(value) {
                Ok(walk) => nodes(id, &walk, origin, clip, focus),
                Err(error) => unread(id, clip, &error.to_string()),
            },
            Err(error) => unread(id, clip, &error),
        };
        if nodes.iter().skip(1).any(|node| node.focused) {
            for node in &mut tree.nodes {
                node.focused = false;
            }
        }
        let host = host_of(tree, &clip);
        insert(tree, host, nodes);
    }
}

fn unread(view: u64, clip: Rect, why: &str) -> Vec<Node> {
    vec![Node {
        id: format!("w{view}-0"),
        role: "WebView".into(),
        description: Some(format!("page not read: {why}")),
        bounds: Some(clip),
        web: true,
        ..Node::default()
    }]
}

fn intersect(a: &Rect, b: &Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Rect {
        x,
        y,
        w: ((a.x + a.w).min(b.x + b.w) - x).max(0.0),
        h: ((a.y + a.h).min(b.y + b.h) - y).max(0.0),
    }
}

fn has_focus(view: &WKWebView) -> bool {
    view.window()
        .and_then(|w| w.firstResponder())
        .and_then(|r| r.downcast::<objc2_app_kit::NSView>().ok())
        .is_some_and(|v| v.isDescendantOf(view))
}

pub(super) fn parse_ref(id: &str) -> Option<(u64, u64)> {
    let (view, n) = id.strip_prefix('w')?.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !(digits(view) && digits(n)) {
        return None;
    }
    Some((view.parse().ok()?, n.parse().ok()?))
}

const PREVIEW_VIEW: u64 = 1;
const PREVIEW_POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Step {
    Click {
        #[serde(flatten)]
        element: super::selector::Element,
    },
    Type {
        text: String,
        #[serde(flatten)]
        element: Option<super::selector::Element>,
    },
    Key {
        keys: super::selector::Keys,
    },
    Wait(Until),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "for", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Until {
    Ms {
        ms: u64,
    },
    #[serde(untagged)]
    Seen(super::selector::Seen),
}

// A windowless preview page takes no AppKit input, so its steps act through the DOM.
pub(crate) struct Preview<'a> {
    pub(crate) view: &'a WKWebView,
    pub(crate) size: (f32, f32),
    pub(crate) executor: &'a BackgroundExecutor,
}

impl Preview<'_> {
    async fn read(&self) -> Result<Tree, String> {
        let walk: Walk =
            serde_json::from_value(ask(self.view, json!({ "op": "walk" }), self.executor).await?)
                .map_err(|e| e.to_string())?;
        let clip = Rect {
            x: 0.0,
            y: 0.0,
            w: self.size.0,
            h: self.size.1,
        };
        Ok(Tree {
            title: walk.title.clone(),
            size: self.size,
            nodes: nodes(PREVIEW_VIEW, &walk, (0.0, 0.0), clip, true),
            ..Tree::default()
        })
    }

    pub(crate) async fn tree(&self) -> Result<String, String> {
        let tree = self.read().await?;
        let text = super::tree::render(&tree, &super::tree::View::default())?;
        Ok(text
            .as_str()
            .unwrap_or_default()
            .replacen("window 0 ", "page ", 1))
    }

    pub(crate) async fn bounds(
        &self,
        select: &str,
        nth: Option<usize>,
    ) -> Result<[f64; 4], String> {
        let tree = self.read().await?;
        let element = super::selector::Element::Select {
            select: select.into(),
            nth,
        };
        let b = element
            .find(&tree)?
            .bounds
            .ok_or("that element has no bounds")?;
        Ok([b.x, b.y, b.w, b.h].map(f64::from))
    }

    async fn node(&self, element: &super::selector::Element) -> Result<u64, String> {
        let tree = self.read().await?;
        let node = element.find(&tree)?;
        if node.disabled {
            return Err(format!(
                "`#{}` is disabled: {}",
                node.id,
                super::tree::line(node)
            ));
        }
        parse_ref(&node.id)
            .map(|(_, n)| n)
            .filter(|&n| n > 0)
            .ok_or_else(|| format!("`#{}` is the page itself; pick an element in it", node.id))
    }

    pub(crate) async fn step(&self, step: &Value) -> Result<(), String> {
        let step: Step =
            serde_json::from_value(step.clone()).map_err(|e| format!("invalid step: {e}"))?;
        let request = match step {
            Step::Click { element } => {
                json!({ "op": "act", "n": self.node(&element).await?, "do": "click" })
            }
            Step::Type { text, element } => {
                let n = match element {
                    Some(element) => Some(self.node(&element).await?),
                    None => None,
                };
                json!({ "op": "type", "n": n, "text": text })
            }
            Step::Key { keys } => json!({ "op": "key", "keys": keys.strokes() }),
            Step::Wait(Until::Ms { ms }) => {
                self.executor
                    .timer(Duration::from_millis(
                        ms.min(zeron_proto::ui_driver::MAX_WAIT_MS),
                    ))
                    .await;
                return Ok(());
            }
            Step::Wait(Until::Seen(seen)) => {
                let timeout = Duration::from_millis(
                    seen.timeout_ms().min(zeron_proto::ui_driver::MAX_WAIT_MS),
                );
                let started = std::time::Instant::now();
                loop {
                    let found = super::selector::check(&seen, &self.read().await?);
                    match found {
                        Ok(_) => return Ok(()),
                        Err(error) if started.elapsed() >= timeout => return Err(error),
                        Err(_) => self.executor.timer(PREVIEW_POLL).await,
                    }
                }
            }
        };
        ask(self.view, request, self.executor).await.map(drop)
    }
}

#[cfg(debug_assertions)]
pub(super) use drive::*;

#[cfg(debug_assertions)]
mod drive {
    use super::*;
    use gpui::{Keystroke, Point};
    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_foundation::{NSPoint, NSProcessInfo};

    fn find(id: u64) -> Result<(Rc<dyn Hosted>, Page), String> {
        live()
            .into_iter()
            .find(|(view, _, _)| *view == id)
            .map(|(_, host, page)| (host, page))
            .ok_or_else(|| format!("page w{id} is not painted now; read tree again"))
    }

    fn contains(b: &Bounds<Pixels>, at: Point<Pixels>) -> bool {
        b.size.width > gpui::px(0.) && b.size.height > gpui::px(0.) && b.contains(&at)
    }

    pub(in crate::ui_driver) fn page_at(window: &gpui::Window, at: Point<Pixels>) -> Option<u64> {
        in_window(window)
            .into_iter()
            .find(|(_, _, page)| contains(&page.visible, at))
            .map(|(id, _, _)| id)
    }

    pub(in crate::ui_driver) fn focused(window: &gpui::Window) -> Option<u64> {
        in_window(window)
            .into_iter()
            .find(|(_, _, page)| has_focus(&page.view))
            .map(|(id, _, _)| id)
    }

    fn base_point(view: &WKWebView, at: Point<Pixels>) -> Result<NSPoint, String> {
        let parent = unsafe { view.superview() }.ok_or("the page has no parent view")?;
        let (x, y) = (f64::from(f32::from(at.x)), f64::from(f32::from(at.y)));
        let y = if parent.isFlipped() {
            y
        } else {
            parent.bounds().size.height - y
        };
        Ok(parent.convertPoint_toView(NSPoint::new(x, y), None))
    }

    fn uptime() -> f64 {
        NSProcessInfo::processInfo().systemUptime()
    }

    fn mouse(
        kind: NSEventType,
        at: NSPoint,
        window: isize,
        count: isize,
    ) -> Result<Retained<NSEvent>, String> {
        let pressure = if kind == NSEventType::LeftMouseDown {
            1.0
        } else {
            0.0
        };
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            at,
            NSEventModifierFlags::empty(),
            uptime(),
            window,
            None,
            0,
            count,
            pressure,
        )
        .ok_or_else(|| "AppKit refused the mouse event".to_string())
    }

    pub(in crate::ui_driver) fn hover(id: u64, at: Point<Pixels>) -> Result<(), String> {
        let (_, page) = find(id)?;
        let window = page.view.window().ok_or("the page has no window")?;
        let point = base_point(&page.view, at)?;
        page.view.mouseMoved(&*mouse(
            NSEventType::MouseMoved,
            point,
            window.windowNumber(),
            0,
        )?);
        Ok(())
    }

    pub(in crate::ui_driver) fn click(
        id: u64,
        at: Point<Pixels>,
        count: usize,
    ) -> Result<(), String> {
        let (host, page) = find(id)?;
        if !contains(&page.visible, at) {
            return Err(format!(
                "({}, {}) is not on the live part of page w{id}: app chrome covers it there; scroll it into view first",
                f32::from(at.x).round(),
                f32::from(at.y).round()
            ));
        }
        let window = page.view.window().ok_or("the page has no window")?;
        let number = window.windowNumber();
        let point = base_point(&page.view, at)?;
        host.pressing();
        page.view
            .mouseMoved(&*mouse(NSEventType::MouseMoved, point, number, 0)?);
        window.makeFirstResponder(Some(&page.view));
        for count in 1..=count.max(1) as isize {
            page.view
                .mouseDown(&*mouse(NSEventType::LeftMouseDown, point, number, count)?);
            page.view
                .mouseUp(&*mouse(NSEventType::LeftMouseUp, point, number, count)?);
        }
        host.pressing();
        Ok(())
    }

    fn key_code(key: &str) -> Option<(u16, String)> {
        let named = |code: u16, ch: u32| Some((code, char::from_u32(ch)?.to_string()));
        match key {
            "enter" => named(36, 0x0d),
            "tab" => named(48, 0x09),
            "space" => Some((49, " ".into())),
            "backspace" => named(51, 0x7f),
            "escape" => named(53, 0x1b),
            "delete" => named(117, 0xf728),
            "left" => named(123, 0xf702),
            "right" => named(124, 0xf703),
            "down" => named(125, 0xf701),
            "up" => named(126, 0xf700),
            "home" => named(115, 0xf729),
            "end" => named(119, 0xf72b),
            "pageup" => named(116, 0xf72c),
            "pagedown" => named(121, 0xf72d),
            _ => None,
        }
    }

    fn ansi(ch: char) -> u16 {
        const KEYS: &str = "asdfhgzxcv\0bqweryt123465=97-80]ou[ip\0lj'k;\\,/nm.\0\0`";
        KEYS.chars()
            .position(|key| key == ch.to_ascii_lowercase())
            .map_or(0, |code| code as u16)
    }

    fn flags(stroke: &Keystroke) -> NSEventModifierFlags {
        let mut flags = NSEventModifierFlags::empty();
        let m = &stroke.modifiers;
        for (on, flag) in [
            (m.platform, NSEventModifierFlags::Command),
            (m.shift, NSEventModifierFlags::Shift),
            (m.alt, NSEventModifierFlags::Option),
            (m.control, NSEventModifierFlags::Control),
        ] {
            if on {
                flags |= flag;
            }
        }
        flags
    }

    fn key_event(
        kind: NSEventType,
        stroke: &Keystroke,
        window: isize,
    ) -> Result<Retained<NSEvent>, String> {
        let (code, plain, typed) = match key_code(&stroke.key) {
            Some((code, named)) => (code, named.clone(), named),
            None => {
                let ch = stroke.key.chars().next().ok_or("an empty key")?;
                let typed = stroke
                    .key_char
                    .clone()
                    .filter(|_| !stroke.modifiers.platform && !stroke.modifiers.control)
                    .unwrap_or_else(|| stroke.key.clone());
                (ansi(ch), stroke.key.clone(), typed)
            }
        };
        NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            kind,
            NSPoint::ZERO,
            flags(stroke),
            uptime(),
            window,
            None,
            &NSString::from_str(&typed),
            &NSString::from_str(&plain),
            false,
            code,
        )
        .ok_or_else(|| "AppKit refused the key event".to_string())
    }

    pub(in crate::ui_driver) fn keys(id: u64, strokes: &[Keystroke]) -> Result<(), String> {
        let (_, page) = find(id)?;
        let window = page.view.window().ok_or("the page has no window")?;
        if !has_focus(&page.view) {
            window.makeFirstResponder(Some(&page.view));
        }
        let number = window.windowNumber();
        for stroke in strokes {
            let down = key_event(NSEventType::KeyDown, stroke, number)?;
            if !(stroke.modifiers.platform && page.view.performKeyEquivalent(&down)) {
                page.view.keyDown(&down);
            }
            page.view
                .keyUp(&*key_event(NSEventType::KeyUp, stroke, number)?);
        }
        Ok(())
    }

    pub(in crate::ui_driver) async fn script_on(
        id: u64,
        request: Value,
        cx: &AsyncApp,
    ) -> Result<Value, String> {
        let (_, page) = cx.update(|_| find(id))?;
        ask(&page.view, request, cx.background_executor()).await
    }

    pub(in crate::ui_driver) async fn scroll(
        id: u64,
        at: Point<Pixels>,
        dx: f32,
        dy: f32,
        cx: &AsyncApp,
    ) -> Result<Value, String> {
        let (_, page) = cx.update(|_| find(id))?;
        let request = json!({
            "op": "scroll",
            "x": f32::from(at.x - page.bounds.origin.x),
            "y": f32::from(at.y - page.bounds.origin.y),
            "dx": -dx,
            "dy": -dy,
        });
        ask(&page.view, request, cx.background_executor()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_driver::tree::tests::{node, tree_of};

    fn walk(json: Value) -> Walk {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn page_nodes_map_to_window_points_and_drop_what_is_outside() {
        let page = walk(json!({
            "title": "Options",
            "nodes": [
                { "n": 3, "parent": -1, "role": "Button", "name": "Option B", "x": 10, "y": 20, "w": 80, "h": 24, "actions": ["click"] },
                { "n": 4, "parent": -1, "role": "List", "x": 0, "y": 900, "w": 100, "h": 50 },
                { "n": 5, "parent": 1, "role": "ListItem", "name": "kept", "x": 0, "y": 100, "w": 100, "h": 20 },
                { "n": 6, "parent": -1, "role": "<img onerror>", "name": "x", "x": 0, "y": 0, "w": 5, "h": 5 }
            ]
        }));
        let clip = Rect {
            x: 100.0,
            y: 200.0,
            w: 400.0,
            h: 300.0,
        };
        let nodes = nodes(2, &page, (100.0, 200.0), clip, false);
        let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["w2-0", "w2-3", "w2-5", "w2-6"]);
        assert_eq!(
            super::super::tree::line(&nodes[1]),
            "w2-3 Button \"Option B\" @110,220 80x24 [click] web"
        );
        assert_eq!(
            nodes[2].parent,
            Some(0),
            "a dropped parent lifts its child to the root"
        );
        assert_eq!(
            nodes[3].role, "GenericContainer",
            "a page cannot print its own role text"
        );
        assert!(nodes.iter().all(|n| n.web));
    }

    #[test]
    fn page_nodes_nest_after_the_host_subtree_and_keep_later_parents() {
        let mut tree = tree_of(&[
            (0, node("e1", "Window", None)),
            (1, node("e2", "Group", Some("card"))),
            (2, node("e3", "Button", Some("Copy"))),
            (1, node("e4", "Button", Some("Send"))),
            (2, node("e5", "Label", Some("hint"))),
        ]);
        let web = vec![
            Node {
                id: "w1-0".into(),
                role: "WebView".into(),
                web: true,
                ..Node::default()
            },
            Node {
                id: "w1-3".into(),
                role: "Button".into(),
                depth: 1,
                parent: Some(0),
                web: true,
                ..Node::default()
            },
        ];
        insert(&mut tree, 1, web);
        let ids: Vec<&str> = tree.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["e1", "e2", "e3", "w1-0", "w1-3", "e4", "e5"]);
        assert_eq!(tree.nodes[3].parent, Some(1));
        assert_eq!(tree.nodes[3].depth, 2);
        assert_eq!(tree.nodes[4].parent, Some(3));
        assert_eq!(tree.nodes[4].depth, 3);
        assert_eq!(
            tree.nodes[6].parent,
            Some(5),
            "a later native parent shifts with the insert"
        );
    }

    #[test]
    fn preview_steps_read_in_driver_syntax() {
        let step = |v: Value| serde_json::from_value::<Step>(v).map(|s| format!("{s:?}"));
        assert!(
            step(json!({ "action": "click", "select": "Button \"Option B\"" }))
                .unwrap()
                .starts_with("Click")
        );
        assert!(
            step(json!({ "action": "type", "text": "Ada" }))
                .unwrap()
                .contains("element: None")
        );
        assert!(
            step(json!({ "action": "type", "text": "Ada", "ref": "w1-4" }))
                .unwrap()
                .contains("w1-4")
        );
        assert!(
            step(json!({ "action": "key", "keys": "enter" }))
                .unwrap()
                .starts_with("Key")
        );
        assert!(
            step(json!({ "action": "wait", "for": "ms", "ms": 50 }))
                .unwrap()
                .contains("Ms")
        );
        assert!(
            step(json!({ "action": "wait", "for": "text", "text": "Picked" }))
                .unwrap()
                .contains("Text")
        );
        assert!(step(json!({ "action": "drag" })).is_err());
    }

    #[test]
    fn web_refs_name_a_view_and_a_node() {
        assert_eq!(parse_ref("w3-12"), Some((3, 12)));
        assert_eq!(parse_ref("w3"), None);
        assert_eq!(parse_ref("e3"), None);
        assert_eq!(parse_ref("w-1"), None);
    }
}
