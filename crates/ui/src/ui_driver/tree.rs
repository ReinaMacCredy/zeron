use gpui::{AnyWindowHandle, App, AsyncApp};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const LEASE: Duration = Duration::from_secs(60);
const REAP_EVERY: Duration = Duration::from_secs(5);
const TEXT_CAP: usize = 80;
pub(super) const GENERIC: &[&str] = &["GenericContainer", "Unknown", "Group", "Section", "Pane"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub(super) struct Bounds {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
}

impl Bounds {
    pub(super) fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Node {
    #[serde(rename = "ref")]
    pub(super) id: String,
    #[serde(skip)]
    pub(super) accesskit_id: u64,
    pub(super) role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) bounds: Option<Bounds>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) actions: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(super) focused: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) selected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) expanded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) toggled: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(super) disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) view: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) element_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) source: Option<String>,
    pub(super) depth: usize,
    #[serde(skip)]
    pub(super) parent: Option<usize>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(super) web: bool,
}

impl Node {
    pub(super) fn named(&self) -> bool {
        [&self.label, &self.value, &self.description]
            .into_iter()
            .any(|text| text.as_deref().is_some_and(|text| !text.trim().is_empty()))
    }

    pub(super) fn supports(&self, action: &str) -> bool {
        self.actions
            .iter()
            .any(|supported| supported.eq_ignore_ascii_case(action))
    }

    fn hidden(&self) -> bool {
        if self.parent.is_none() || self.focused {
            return false;
        }
        let generic =
            GENERIC.contains(&self.role.as_str()) && !self.named() && self.actions.is_empty();
        let empty = self.bounds.is_none_or(|bounds| bounds.is_empty()) && self.label.is_none();
        generic || empty
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Tree {
    pub(super) window: u64,
    pub(super) title: String,
    pub(super) size: (f32, f32),
    pub(super) tab_stops: usize,
    pub(super) frame: u64,
    pub(super) nodes: Vec<Node>,
}

impl Tree {
    pub(super) fn focus(&self) -> Option<&Node> {
        self.nodes.iter().find(|node| node.focused)
    }

    fn index(&self, id: &str) -> Option<usize> {
        let id = id.strip_prefix('#').unwrap_or(id);
        self.nodes.iter().position(|node| node.id == id)
    }
}

#[derive(Default)]
pub(super) struct Refs {
    ids: HashMap<u64, usize>,
}

impl Refs {
    fn of(&mut self, accesskit_id: u64) -> String {
        let next = self.ids.len() + 1;
        format!("e{}", self.ids.entry(accesskit_id).or_insert(next))
    }
}

pub(super) static REFS: LazyLock<Mutex<Refs>> = LazyLock::new(Mutex::default);

#[derive(Deserialize)]
struct Dump {
    root: Option<String>,
    gpui_focus: Option<String>,
    frame: Option<DumpFrame>,
    #[serde(default)]
    nodes: HashMap<String, DumpNode>,
}

#[derive(Deserialize)]
struct DumpFrame {
    frame_number: u64,
    window_title: Option<String>,
    viewport_size: DumpSize,
    tab_stop_count: usize,
}

#[derive(Deserialize)]
struct DumpSize {
    width: f32,
    height: f32,
}

#[derive(Deserialize)]
struct DumpNode {
    accesskit_id: String,
    #[serde(default)]
    children: Vec<String>,
    bounds: Option<Bounds>,
    element_id: Option<String>,
    view: Option<String>,
    source_location: Option<String>,
    aria: Aria,
}

#[derive(Deserialize)]
struct Aria {
    role: String,
    #[serde(default)]
    on_action: Vec<String>,
    label: Option<String>,
    description: Option<String>,
    value: Option<String>,
    placeholder: Option<String>,
    selected: Option<bool>,
    expanded: Option<bool>,
    toggled: Option<String>,
    #[serde(default)]
    disabled: bool,
}

#[derive(Deserialize)]
struct FrameOnly {
    frame: Option<FrameNumber>,
}

#[derive(Deserialize)]
struct FrameNumber {
    frame_number: u64,
}

fn frame_number(dump: &str) -> Option<u64> {
    serde_json::from_str::<FrameOnly>(dump)
        .ok()?
        .frame
        .map(|frame| frame.frame_number)
}

pub(super) fn lower_camel(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|first| first.to_ascii_lowercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

pub(super) fn parse_dump(dump: &str, window: u64, refs: &mut Refs) -> Result<Tree, String> {
    let mut dump: Dump =
        serde_json::from_str(dump).map_err(|error| format!("unreadable a11y dump: {error}"))?;
    let frame = dump.frame.as_ref();
    let mut tree = Tree {
        window,
        title: frame
            .and_then(|frame| frame.window_title.clone())
            .unwrap_or_default(),
        size: frame.map_or((0.0, 0.0), |frame| {
            (frame.viewport_size.width, frame.viewport_size.height)
        }),
        tab_stops: frame.map_or(0, |frame| frame.tab_stop_count),
        frame: frame.map_or(0, |frame| frame.frame_number),
        nodes: Vec::new(),
    };
    let Some(root) = dump.root.take() else {
        return Ok(tree);
    };
    let mut seen = HashSet::new();
    let mut stack = vec![(root, 0, None)];
    while let Some((key, depth, parent)) = stack.pop() {
        if !seen.insert(key.clone()) {
            continue;
        }
        let Some(raw) = dump.nodes.remove(&key) else {
            continue;
        };
        let accesskit_id = raw
            .accesskit_id
            .parse()
            .map_err(|_| format!("bad accesskit_id `{}`", raw.accesskit_id))?;
        let index = tree.nodes.len();
        for child in raw.children.into_iter().rev() {
            stack.push((child, depth + 1, Some(index)));
        }
        let aria = raw.aria;
        tree.nodes.push(Node {
            id: refs.of(accesskit_id),
            accesskit_id,
            role: aria.role,
            label: aria.label,
            value: aria.value,
            description: aria.description,
            placeholder: aria.placeholder,
            bounds: raw.bounds,
            actions: aria
                .on_action
                .iter()
                .map(|name| lower_camel(name))
                .collect(),
            focused: dump.gpui_focus.as_deref() == Some(key.as_str()),
            selected: aria.selected,
            expanded: aria.expanded,
            toggled: aria.toggled.map(|toggled| toggled.to_lowercase()),
            disabled: aria.disabled,
            view: raw.view,
            element_id: raw.element_id,
            source: raw.source_location,
            depth,
            parent,
            web: false,
        });
    }
    Ok(tree)
}

fn clip(text: &str) -> String {
    match text.char_indices().nth(TEXT_CAP) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

pub(super) fn line(node: &Node) -> String {
    let mut out = format!("{} {}", node.id, node.role);
    if let Some(label) = &node.label {
        out += &format!(" {:?}", clip(label));
    }
    match (&node.value, &node.placeholder) {
        (Some(value), _) => out += &format!(" value={:?}", clip(value)),
        (None, Some(placeholder)) => out += &format!(" placeholder={:?}", clip(placeholder)),
        (None, None) => {}
    }
    if let (None, Some(description)) = (&node.label, &node.description) {
        out += &format!(" desc={:?}", clip(description));
    }
    if let Some(b) = node.bounds {
        out += &format!(
            " @{},{} {}x{}",
            b.x.round(),
            b.y.round(),
            b.w.round(),
            b.h.round()
        );
    }
    if node.focused {
        out += " focused";
    }
    if node.selected == Some(true) {
        out += " selected";
    }
    if let Some(expanded) = node.expanded {
        out += &format!(" expanded={expanded}");
    }
    if let Some(toggled) = &node.toggled {
        out += &format!(" toggled={toggled}");
    }
    if node.disabled {
        out += " disabled";
    }
    if !node.actions.is_empty() {
        out += &format!(" [{}]", node.actions.join(","));
    }
    if node.web {
        out += " web";
    }
    out
}

#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct View {
    pub(super) query: Option<String>,
    pub(super) root: Option<String>,
    pub(super) depth: Option<usize>,
    #[serde(default)]
    pub(super) full: bool,
    #[serde(default)]
    pub(super) json: bool,
}

pub(super) fn contains(field: &Option<String>, needle: &str) -> bool {
    field
        .as_deref()
        .is_some_and(|text| text.to_lowercase().contains(needle))
}

fn rows(tree: &Tree, view: &View) -> Result<Vec<(usize, usize)>, String> {
    if tree.nodes.is_empty() {
        return Ok(Vec::new());
    }
    let start = match &view.root {
        Some(root) => tree
            .index(root)
            .ok_or_else(|| format!("`root` {root} is not in the current tree"))?,
        None => 0,
    };
    let base = tree.nodes[start].depth;
    let end = (start + 1..tree.nodes.len())
        .find(|&i| tree.nodes[i].depth <= base)
        .unwrap_or(tree.nodes.len());
    let span = start..end;
    let shown: Vec<bool> = span
        .clone()
        .map(|i| view.full || i == start || !tree.nodes[i].hidden())
        .collect();
    let mut level = vec![0usize; span.len()];
    for i in start + 1..end {
        let parent = tree.nodes[i].parent.map_or(start, |p| p.max(start)) - start;
        level[i - start] = level[parent] + usize::from(shown[parent]);
    }
    let printed: Vec<bool> = match view.query.as_deref().map(str::to_lowercase) {
        None => shown.clone(),
        Some(query) => {
            let hit: Vec<bool> = span
                .clone()
                .map(|i| {
                    let node = &tree.nodes[i];
                    node.role.to_lowercase().contains(&query)
                        || [
                            &node.label,
                            &node.value,
                            &node.description,
                            &node.placeholder,
                        ]
                        .into_iter()
                        .any(|field| contains(field, &query))
                })
                .collect();
            let mut below = hit.clone();
            for i in (start + 1..end).rev() {
                if let Some(parent) = tree.nodes[i].parent.filter(|&p| p >= start) {
                    below[parent - start] |= below[i - start];
                }
            }
            (0..span.len())
                .map(|i| hit[i] || (shown[i] && below[i]))
                .collect()
        }
    };
    Ok(span
        .filter(|&i| printed[i - start] && view.depth.is_none_or(|d| level[i - start] <= d))
        .map(|i| (i, level[i - start]))
        .collect())
}

pub(super) fn render(tree: &Tree, view: &View) -> Result<Value, String> {
    let rows = rows(tree, view)?;
    let focus = tree.focus().map(|node| node.id.clone());
    if view.json {
        let nodes: Vec<Value> = rows
            .iter()
            .map(|&(i, level)| {
                let mut node = json!(tree.nodes[i]);
                node["depth"] = json!(level);
                node
            })
            .collect();
        return Ok(json!({
            "window": tree.window,
            "title": tree.title,
            "size": { "w": tree.size.0, "h": tree.size.1 },
            "focus": focus,
            "total": tree.nodes.len(),
            "nodes": nodes,
        }));
    }
    let mut out = format!(
        "window {} {:?} {}x{} nodes={}/{} focus={}",
        tree.window,
        tree.title,
        tree.size.0.round(),
        tree.size.1.round(),
        rows.len(),
        tree.nodes.len(),
        focus.as_deref().unwrap_or("none")
    );
    for (i, level) in rows {
        out.push('\n');
        out.push_str(&"  ".repeat(level));
        out.push_str(&line(&tree.nodes[i]));
    }
    Ok(Value::String(out))
}

struct Lease {
    used: Instant,
    forced_after: Option<u64>,
}

fn touch(window: u64, forced_after: Option<Option<u64>>, cx: &mut App) -> Option<u64> {
    let mut leases = LEASES.lock().unwrap_or_else(|poison| poison.into_inner());
    let lease = leases.entry(window).or_insert(Lease {
        used: Instant::now(),
        forced_after: None,
    });
    lease.used = Instant::now();
    if let Some(prior) = forced_after {
        lease.forced_after = prior;
    }
    let prior = lease.forced_after;
    drop(leases);
    if !REAPING.swap(true, Ordering::Relaxed) {
        cx.spawn(async move |cx| reap(cx).await).detach();
    }
    prior
}

pub(super) fn hold(window: u64, cx: &mut App) {
    touch(window, None, cx);
}

static LEASES: LazyLock<Mutex<HashMap<u64, Lease>>> = LazyLock::new(Mutex::default);
static REAPING: AtomicBool = AtomicBool::new(false);

async fn reap(cx: &mut AsyncApp) {
    loop {
        cx.background_executor().timer(REAP_EVERY).await;
        let (expired, done) = {
            let mut leases = LEASES.lock().unwrap_or_else(|poison| poison.into_inner());
            let expired: Vec<u64> = leases
                .iter()
                .filter(|(_, lease)| lease.used.elapsed() >= LEASE)
                .map(|(&window, _)| window)
                .collect();
            leases.retain(|window, _| !expired.contains(window));
            let done = leases.is_empty();
            if done {
                REAPING.store(false, Ordering::Relaxed);
            }
            (expired, done)
        };
        cx.update(|cx| {
            for handle in cx.windows() {
                if expired.contains(&handle.window_id().as_u64()) {
                    let _ = handle.update(cx, |_, window, _| window.set_a11y_forced(false));
                }
            }
        });
        if done {
            return;
        }
    }
}

pub(super) async fn read(handle: AnyWindowHandle, cx: &mut AsyncApp) -> Result<Tree, String> {
    let tree = read_native(handle, cx).await?;
    #[cfg(target_os = "macos")]
    let tree = {
        let mut tree = tree;
        super::web::splice(&mut tree, handle, cx).await;
        tree
    };
    Ok(tree)
}

pub(super) async fn read_native(
    handle: AnyWindowHandle,
    cx: &mut AsyncApp,
) -> Result<Tree, String> {
    let id = handle.window_id().as_u64();
    let gone = |error: anyhow::Error| format!("window is gone: {error}");
    let prior = cx
        .update(|cx| {
            let forced = handle.update(cx, |_, window, _| {
                if window.is_a11y_forced() {
                    return None;
                }
                let prior = window
                    .debug_a11y_tree_json()
                    .and_then(|dump| frame_number(&dump));
                window.set_a11y_forced(true);
                Some(prior)
            })?;
            anyhow::Ok(touch(id, forced, cx))
        })
        .map_err(gone)?;
    // The forced flag is sampled at the end of a draw, so the first tree comes one frame after the next.
    for attempt in 0..=6 {
        if attempt > 0 {
            super::fresh_frame(handle, Duration::from_millis(250), cx).await?;
        }
        let (dump, title) = cx
            .update(|cx| {
                handle.update(cx, |_, window, _| {
                    (window.debug_a11y_tree_json(), window.window_title())
                })
            })
            .map_err(gone)?;
        let Some(dump) = dump else {
            continue;
        };
        let mut refs = REFS.lock().unwrap_or_else(|poison| poison.into_inner());
        let mut tree = parse_dump(&dump, id, &mut refs)?;
        if prior.is_some_and(|prior| tree.frame <= prior) {
            continue;
        }
        if tree.title.is_empty() {
            tree.title = title;
        }
        return Ok(tree);
    }
    Err(
        "the window drew no frame after the accessibility tree was enabled (minimized or occluded window?); retry once it is visible"
            .into(),
    )
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(in crate::ui_driver) fn node(id: &str, role: &str, label: Option<&str>) -> Node {
        Node {
            id: id.to_owned(),
            role: role.to_owned(),
            label: label.map(str::to_owned),
            bounds: Some(Bounds {
                x: 10.0,
                y: 20.0,
                w: 100.0,
                h: 30.0,
            }),
            ..Node::default()
        }
    }

    pub(in crate::ui_driver) fn tree_of(specs: &[(usize, Node)]) -> Tree {
        let mut nodes = Vec::new();
        let mut path: Vec<usize> = Vec::new();
        for (depth, node) in specs {
            path.truncate(*depth);
            let mut node = node.clone();
            node.depth = *depth;
            node.parent = path.last().copied();
            path.push(nodes.len());
            nodes.push(node);
        }
        Tree {
            window: 1,
            title: "Zeron Dev".into(),
            size: (800.0, 600.0),
            tab_stops: 3,
            frame: 1,
            nodes,
        }
    }

    #[test]
    fn default_view_hoists_generic_and_empty_nodes() {
        let mut empty = node("e5", "Button", None);
        empty.bounds = Some(Bounds::default());
        let mut clickable = node("e6", "GenericContainer", None);
        clickable.actions = vec!["click".into()];
        let tree = tree_of(&[
            (0, node("e1", "Window", Some("Zeron"))),
            (1, node("e2", "GenericContainer", None)),
            (2, node("e3", "Button", Some("New chat"))),
            (2, node("e4", "GenericContainer", Some("Sidebar"))),
            (3, empty),
            (3, clickable),
        ]);
        let Value::String(text) = render(&tree, &View::default()).unwrap() else {
            panic!("text view is a string");
        };
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "window 1 \"Zeron Dev\" 800x600 nodes=4/6 focus=none",
                "e1 Window \"Zeron\" @10,20 100x30",
                "  e3 Button \"New chat\" @10,20 100x30",
                "  e4 GenericContainer \"Sidebar\" @10,20 100x30",
                "    e6 GenericContainer @10,20 100x30 [click]",
            ]
        );
        let full = render(
            &tree,
            &View {
                full: true,
                ..View::default()
            },
        )
        .unwrap();
        assert_eq!(full.as_str().unwrap().lines().count(), 7);
        let shallow = render(
            &tree,
            &View {
                depth: Some(1),
                ..View::default()
            },
        )
        .unwrap();
        assert_eq!(shallow.as_str().unwrap().lines().count(), 4);
    }

    #[test]
    fn query_keeps_the_ancestor_chain() {
        let mut input = node("e5", "TextInput", None);
        input.placeholder = Some("Ask anything".into());
        let tree = tree_of(&[
            (0, node("e1", "Window", Some("Zeron"))),
            (1, node("e2", "Group", Some("Sidebar"))),
            (2, node("e3", "Button", Some("New chat"))),
            (1, node("e4", "Group", Some("Composer"))),
            (2, input),
            (2, node("e6", "Button", Some("Send"))),
        ]);
        let view = |query: &str, root: Option<&str>| View {
            query: Some(query.into()),
            root: root.map(str::to_owned),
            ..View::default()
        };
        let text = render(&tree, &view("ASK", None)).unwrap();
        let lines: Vec<&str> = text.as_str().unwrap().lines().skip(1).collect();
        assert_eq!(
            lines,
            [
                "e1 Window \"Zeron\" @10,20 100x30",
                "  e4 Group \"Composer\" @10,20 100x30",
                "    e5 TextInput placeholder=\"Ask anything\" @10,20 100x30",
            ]
        );
        let rooted = render(&tree, &view("button", Some("e4"))).unwrap();
        let lines: Vec<&str> = rooted.as_str().unwrap().lines().skip(1).collect();
        assert_eq!(
            lines,
            [
                "e4 Group \"Composer\" @10,20 100x30",
                "  e6 Button \"Send\" @10,20 100x30"
            ]
        );
        assert!(render(&tree, &view("x", Some("e99"))).is_err());
        let json = render(
            &tree,
            &View {
                json: true,
                ..view("send", None)
            },
        )
        .unwrap();
        assert_eq!(json["nodes"][2]["ref"], "e6");
        assert_eq!(json["nodes"][2]["depth"], 2);
    }

    #[test]
    fn parses_a_gpui_dump_in_document_order() {
        let dump = json!({
            "root": "a",
            "gpui_focus": "c",
            "frame": {
                "frame_number": 7,
                "window_title": "Zeron Dev",
                "viewport_size": { "width": 1502.0, "height": 903.0 },
                "tab_stop_count": 4,
                "scale_factor": 2.0,
            },
            "nodes": {
                "a": { "accesskit_id": "900", "children": ["c", "b"], "aria": { "role": "Window" } },
                "b": {
                    "accesskit_id": "901",
                    "bounds": { "x": 1.0, "y": 2.0, "w": 3.0, "h": 4.0 },
                    "source_location": "crates/ui/src/shell.rs:10:5",
                    "aria": { "role": "Button", "label": "Go", "on_action": ["Click", "ScrollIntoView"], "toggled": "True", "disabled": true }
                },
                "c": { "accesskit_id": "902", "aria": { "role": "TextInput", "value": "hi" } },
            },
        });
        let mut refs = Refs::default();
        let tree = parse_dump(&dump.to_string(), 5, &mut refs).unwrap();
        let ids: Vec<&str> = tree.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, ["e1", "e2", "e3"]);
        assert_eq!(tree.nodes[1].role, "TextInput");
        assert!(tree.nodes[1].focused);
        assert_eq!(tree.nodes[2].parent, Some(0));
        assert_eq!(tree.nodes[2].actions, ["click", "scrollIntoView"]);
        assert_eq!(
            line(&tree.nodes[2]),
            "e3 Button \"Go\" @1,2 3x4 toggled=true disabled [click,scrollIntoView]"
        );
        assert_eq!(json!(tree.nodes[2])["disabled"], true);
        assert!(json!(tree.nodes[1]).get("disabled").is_none());
        assert_eq!((tree.frame, tree.tab_stops), (7, 4));
        let again = parse_dump(&dump.to_string(), 5, &mut refs).unwrap();
        assert_eq!(again.nodes[2].id, "e3");
        assert_eq!(frame_number(&dump.to_string()), Some(7));
    }
}
