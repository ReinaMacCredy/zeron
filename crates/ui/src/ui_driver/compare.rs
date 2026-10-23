use super::selector::Element;
use super::tree::{Bounds, Node, Tree};
use gpui::AsyncApp;
use serde::Deserialize;
use serde_json::{Value, json};

const TEXT_ROLES: &[&str] = &["Label", "Paragraph", "StaticText", "Heading"];
const LABEL_CAP: usize = 80;
const PIXEL_TOLERANCE: u8 = 24;

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Compare {
    html: String,
    rect: Option<Bounds>,
    #[serde(rename = "ref")]
    id: Option<String>,
    select: Option<String>,
    nth: Option<usize>,
    width: Option<f32>,
    #[serde(default = "threshold")]
    threshold: f32,
    #[serde(default)]
    json: bool,
}

fn threshold() -> f32 {
    4.0
}

enum Region {
    Rect(Bounds),
    Element(Element),
}

impl Compare {
    fn region(&self) -> Result<Region, String> {
        match (self.rect, &self.id, &self.select, self.nth) {
            (Some(rect), None, None, None) => Ok(Region::Rect(rect)),
            (None, Some(id), None, None) => Ok(Region::Element(Element::Ref { id: id.clone() })),
            (None, None, Some(select), nth) => Ok(Region::Element(Element::Select {
                select: select.clone(),
                nth,
            })),
            _ => {
                Err("give one of rect, ref, or select (with an optional nth) as the region".into())
            }
        }
    }
}

fn locate(region: &Region, tree: &Tree) -> Result<Bounds, String> {
    let area = match region {
        Region::Rect(rect) => *rect,
        Region::Element(element) => {
            let node = element.find(tree)?;
            node.bounds
                .ok_or_else(|| format!("`#{}` has no bounds to compare", node.id))?
        }
    };
    let (w, h) = tree.size;
    let x = area.x.max(0.0);
    let y = area.y.max(0.0);
    let clipped = Bounds {
        x,
        y,
        w: (area.x + area.w).min(w) - x,
        h: (area.y + area.h).min(h) - y,
    };
    if clipped.w < 1.0 || clipped.h < 1.0 {
        return Err(format!(
            "the region {area:?} lies outside the {w}x{h} window"
        ));
    }
    Ok(clipped)
}

fn normalize(text: &str) -> String {
    let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let words = words
        .trim_end_matches('…')
        .trim_end_matches("...")
        .trim_end();
    words.to_lowercase()
}

fn class(role: &str) -> &str {
    if TEXT_ROLES.contains(&role) {
        "text"
    } else {
        role
    }
}

struct Item<'a> {
    node: &'a Node,
    at: Bounds,
    key: (String, String),
}

fn covers(a: &Bounds, b: &Bounds) -> bool {
    (a.x - b.x).abs() <= 1.0
        && (a.y - b.y).abs() <= 1.0
        && (a.w - b.w).abs() <= 1.0
        && (a.h - b.h).abs() <= 1.0
}

fn items<'a>(nodes: &'a [Node], area: &Bounds) -> Vec<Item<'a>> {
    let inside = |b: &Bounds| {
        b.x >= area.x - 1.0
            && b.y >= area.y - 1.0
            && b.x + b.w <= area.x + area.w + 1.0
            && b.y + b.h <= area.y + area.h + 1.0
    };
    nodes
        .iter()
        .filter_map(|node| {
            let label = normalize(node.label.as_deref()?);
            let b = node.bounds?;
            if label.is_empty() || b.is_empty() || !inside(&b) || covers(&b, area) {
                return None;
            }
            Some(Item {
                node,
                at: Bounds {
                    x: b.x - area.x,
                    y: b.y - area.y,
                    w: b.w,
                    h: b.h,
                },
                key: (class(&node.role).to_owned(), label),
            })
        })
        .collect()
}

fn center(b: &Bounds) -> (f32, f32) {
    (b.x + b.w / 2.0, b.y + b.h / 2.0)
}

struct Matched<'a> {
    native: &'a Item<'a>,
    mock: &'a Item<'a>,
    delta: [f32; 4],
}

impl Matched<'_> {
    fn moved(&self, threshold: f32) -> bool {
        self.delta.iter().any(|d| d.abs() > threshold)
    }
}

struct Report<'a> {
    matched: Vec<Matched<'a>>,
    missing: Vec<&'a Item<'a>>,
    extra: Vec<&'a Item<'a>>,
    unchecked_text: usize,
}

fn pair<'a>(native: &'a [Item<'a>], mock: &'a [Item<'a>]) -> Report<'a> {
    let mut candidates = Vec::new();
    for (n, a) in native.iter().enumerate() {
        for (m, b) in mock.iter().enumerate() {
            if a.key == b.key {
                let ((ax, ay), (bx, by)) = (center(&a.at), center(&b.at));
                candidates.push(((ax - bx).hypot(ay - by), n, m));
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let (mut native_used, mut mock_used) = (vec![false; native.len()], vec![false; mock.len()]);
    let mut matched = Vec::new();
    for (_, n, m) in candidates {
        if native_used[n] || mock_used[m] {
            continue;
        }
        native_used[n] = true;
        mock_used[m] = true;
        let (a, b) = (&native[n].at, &mock[m].at);
        matched.push(Matched {
            native: &native[n],
            mock: &mock[m],
            delta: [a.x - b.x, a.y - b.y, a.w - b.w, a.h - b.h],
        });
    }
    let order = |b: &Bounds| (b.y, b.x);
    matched.sort_by(|a, b| {
        order(&a.native.at)
            .partial_cmp(&order(&b.native.at))
            .unwrap()
    });
    let left = |items: &'a [Item<'a>], used: &[bool]| {
        let mut left: Vec<&Item> = items
            .iter()
            .zip(used)
            .filter(|(_, used)| !**used)
            .map(|(item, _)| item)
            .collect();
        left.sort_by(|a, b| order(&a.at).partial_cmp(&order(&b.at)).unwrap());
        left
    };
    let mut missing = left(mock, &mock_used);
    let before = missing.len();
    if !native.iter().any(|item| item.key.0 == "text") {
        missing.retain(|item| item.key.0 != "text");
    }
    Report {
        unchecked_text: before - missing.len(),
        missing,
        extra: left(native, &native_used),
        matched,
    }
}

fn round(v: f32) -> f64 {
    (f64::from(v) * 10.0).round() / 10.0
}

fn rounded(b: &Bounds) -> Value {
    json!({ "x": round(b.x), "y": round(b.y), "w": round(b.w), "h": round(b.h) })
}

fn matched_json(m: &Matched) -> Value {
    let [dx, dy, dw, dh] = m.delta.map(round);
    let mut item = json!({
        "role": m.native.node.role,
        "label": m.native.node.label,
        "native": rounded(&m.native.at),
        "mock": rounded(&m.mock.at),
        "dx": dx, "dy": dy, "dw": dw, "dh": dh,
    });
    if m.mock.node.role != m.native.node.role {
        item["mockRole"] = json!(m.mock.node.role);
    }
    item
}

fn side_json(item: &Item, side: &str, web: bool) -> Value {
    let mut out =
        json!({ "role": item.node.role, "label": item.node.label, side: rounded(&item.at) });
    if web {
        out["web"] = json!(true);
    }
    out
}

fn report_json(report: &Report, threshold: f32) -> Value {
    let (moved, matched): (Vec<_>, Vec<_>) =
        report.matched.iter().partition(|m| m.moved(threshold));
    let mut reply = json!({
        "matched": matched.into_iter().map(matched_json).collect::<Vec<_>>(),
        "moved": moved.into_iter().map(matched_json).collect::<Vec<_>>(),
        "missing": report.missing.iter().map(|i| side_json(i, "mock", true)).collect::<Vec<_>>(),
        "extra": report.extra.iter().map(|i| side_json(i, "native", i.node.web)).collect::<Vec<_>>(),
    });
    if report.unchecked_text > 0 {
        reply["uncheckedText"] = json!(report.unchecked_text);
    }
    reply
}

fn label(node: &Value) -> String {
    let text = node["label"].as_str().unwrap_or_default();
    let clipped: String = text.chars().take(LABEL_CAP).collect();
    if clipped.len() < text.len() {
        format!("{:?}", clipped + "…")
    } else {
        format!("{clipped:?}")
    }
}

fn num(v: &Value) -> String {
    let v = v.as_f64().unwrap_or_default();
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn at(b: &Value) -> String {
    format!(
        "@{},{} {}x{}",
        num(&b["x"]),
        num(&b["y"]),
        num(&b["w"]),
        num(&b["h"])
    )
}

fn signed(v: &Value) -> String {
    match v.as_f64() {
        Some(d) if d > 0.0 => format!("+{}", num(v)),
        _ => num(v),
    }
}

fn role(node: &Value) -> &str {
    node["role"].as_str().unwrap_or_default()
}

fn text(reply: &Value, threshold: f32) -> String {
    let mut lines = Vec::new();
    for m in reply["moved"].as_array().into_iter().flatten() {
        lines.push(format!(
            "moved   {} {} dx={} dy={} dw={} dh={} native {} mock {}",
            role(m),
            label(m),
            signed(&m["dx"]),
            signed(&m["dy"]),
            signed(&m["dw"]),
            signed(&m["dh"]),
            at(&m["native"]),
            at(&m["mock"])
        ));
    }
    for m in reply["missing"].as_array().into_iter().flatten() {
        lines.push(format!(
            "missing {} {} mock {} web",
            role(m),
            label(m),
            at(&m["mock"])
        ));
    }
    for m in reply["extra"].as_array().into_iter().flatten() {
        let web = if m["web"] == true { " web" } else { "" };
        lines.push(format!(
            "extra   {} {} native {}{web}",
            role(m),
            label(m),
            at(&m["native"])
        ));
    }
    let count = |key: &str| reply[key].as_array().map_or(0, Vec::len);
    let region = &reply["region"];
    let mut summary = format!(
        "compare {}x{} at @{},{}: {} matched, {} moved, {} missing, {} extra (threshold {threshold} pt)",
        num(&region["w"]),
        num(&region["h"]),
        num(&region["x"]),
        num(&region["y"]),
        count("matched"),
        count("moved"),
        count("missing"),
        count("extra"),
    );
    if let Some(n) = reply["uncheckedText"].as_u64() {
        summary += &format!(
            "; {n} mock text nodes not compared: the native region exposes no text, the pixels cover it"
        );
    }
    let pixel = &reply["pixel"];
    if let Some(ratio) = pixel["ratio"].as_f64() {
        summary += &format!(
            "; {:.2}% of pixels differ, highlighted in {}",
            ratio * 100.0,
            pixel["path"].as_str().unwrap_or_default()
        );
    }
    if reply["stale"] == true {
        summary += "; the native pixels may be a frame old";
    }
    lines.push(summary);
    lines.join("\n")
}

fn differs(a: &[u8], b: &[u8]) -> bool {
    a.iter()
        .zip(b)
        .take(3)
        .any(|(a, b)| a.abs_diff(*b) > PIXEL_TOLERANCE)
}

fn pixel_diff(native: &image::RgbaImage, mock: &image::RgbaImage) -> (f64, image::RgbaImage) {
    let mut out = native.clone();
    let mut differing = 0u64;
    for (x, y, px) in out.enumerate_pixels_mut() {
        let same = mock
            .get_pixel_checked(x, y)
            .is_some_and(|m| !differs(&px.0, &m.0));
        if same {
            continue;
        }
        differing += 1;
        let [r, g, b, _] = px.0;
        let mix = |c: u8, to: u8| ((u16::from(c) * 2 + u16::from(to) * 3) / 5) as u8;
        px.0 = [mix(r, 255), mix(g, 0), mix(b, 64), 255];
    }
    let total = u64::from(native.width()) * u64::from(native.height());
    (differing as f64 / total.max(1) as f64, out)
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn run(_: Option<u64>, _: Compare, _: &mut AsyncApp) -> Result<Value, String> {
    Err("`compare` is only implemented on macOS".into())
}

#[cfg(target_os = "macos")]
pub(super) async fn run(
    window: Option<u64>,
    request: Compare,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    use std::time::Duration;
    let region = request.region()?;
    if request.threshold < 0.0 {
        return Err("`threshold` must be a number of points, 0 or more".into());
    }
    let handle = cx.update(|cx| super::target_window(window, cx))?;
    let first = locate(&region, &super::tree::read(handle, cx).await?)?;
    let size = (
        request.width.unwrap_or(first.w).round().max(1.0) as u32,
        first.h.round().max(1.0) as u32,
    );
    let (mock_tree, mock_image) = mock(&request.html, size, cx).await?;
    let drawn = super::fresh_frame(handle, Duration::from_millis(500), cx).await?;
    let tree = super::tree::read(handle, cx).await?;
    let area = locate(&region, &tree)?;
    let native_image = cx.update(|cx| super::capture::region(handle, Some(area), cx))?;
    let native_items = items(&tree.nodes, &area);
    let mock_area = Bounds {
        x: 0.0,
        y: 0.0,
        w: size.0 as f32,
        h: size.1 as f32,
    };
    let mock_items = items(mock_tree.nodes.get(1..).unwrap_or_default(), &mock_area);
    let report = pair(&native_items, &mock_items);
    let mut reply = report_json(&report, request.threshold);
    let (ratio, highlight) = pixel_diff(&native_image, &mock_image);
    let path = shot_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    }
    highlight
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    reply["region"] = rounded(&area);
    reply["pixel"] = json!({ "ratio": (ratio * 10_000.0).round() / 10_000.0, "path": path });
    if drawn.is_none() {
        reply["stale"] = json!(true);
    }
    if request.json {
        return Ok(reply);
    }
    Ok(Value::String(text(&reply, request.threshold)))
}

#[cfg(target_os = "macos")]
fn shot_path() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    zeron_proto::ui_driver::run_dir()
        .join("shots")
        .join(format!(
            "{}-compare-{}.png",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed) + 1
        ))
}

#[cfg(target_os = "macos")]
async fn mock(
    html: &str,
    (w, h): (u32, u32),
    cx: &mut AsyncApp,
) -> Result<(Tree, image::RgbaImage), String> {
    use crate::browser::preview_host;
    const LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
    let page = cx.update(|cx| {
        let theme = crate::theme::Theme::of(cx);
        let font_size = crate::typography::font_size(cx).pixels();
        preview_host::load(
            &crate::html_render::inject_bootstrap(html, theme, font_size, false),
            w,
            h,
        )
    })?;
    let executor = cx.background_executor().clone();
    let work = async {
        page.ready().await?;
        preview_host::settle(&page.view).await?;
        let tree = super::web::Preview {
            view: &page.view,
            size: (w as f32, h as f32),
            executor: &executor,
        }
        .read()
        .await?;
        let tiff = preview_host::capture(&page.view, None).await?;
        let image = executor
            .spawn(async move {
                let image = image::load_from_memory(&tiff).map_err(|e| e.to_string())?;
                let image = if image.width() != w || image.height() != h {
                    image.resize_exact(w, h, image::imageops::FilterType::Triangle)
                } else {
                    image
                };
                Ok::<_, String>(image.to_rgba8())
            })
            .await?;
        Ok((tree, image))
    };
    let timeout = executor.timer(LOAD_TIMEOUT);
    futures::pin_mut!(work, timeout);
    match futures::future::select(work, timeout).await {
        futures::future::Either::Left((result, _)) => result,
        futures::future::Either::Right(_) => Err(format!(
            "the mockup did not load within {} s; check its scripts",
            LOAD_TIMEOUT.as_secs()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_driver::tree::tests::node;

    fn at(mut node: Node, x: f32, y: f32, w: f32, h: f32) -> Node {
        node.bounds = Some(Bounds { x, y, w, h });
        node
    }

    fn request(json: Value) -> Result<Compare, serde_json::Error> {
        serde_json::from_value(json)
    }

    fn labels_of(reply: &Value, key: &str) -> Vec<String> {
        reply[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                format!(
                    "{} {}",
                    item["role"].as_str().unwrap(),
                    item["label"].as_str().unwrap()
                )
            })
            .collect()
    }

    #[test]
    fn a_region_is_one_of_rect_ref_or_select() {
        let one = |extra: Value| {
            let mut args = json!({ "html": "<p>x</p>" });
            args.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            request(args).unwrap().region().map(|_| ())
        };
        assert!(one(json!({ "rect": { "x": 0, "y": 0, "w": 10, "h": 10 } })).is_ok());
        assert!(one(json!({ "ref": "e4" })).is_ok());
        assert!(one(json!({ "select": "Group \"General\"", "nth": 1 })).is_ok());
        for bad in [
            json!({}),
            json!({ "ref": "e4", "select": "Button" }),
            json!({ "rect": { "x": 0, "y": 0, "w": 10, "h": 10 }, "ref": "e4" }),
            json!({ "ref": "e4", "nth": 1 }),
        ] {
            let error = one(bad.clone()).unwrap_err();
            assert!(
                error.starts_with("give one of rect, ref, or select"),
                "{bad}: {error}"
            );
        }
        assert!(
            request(json!({ "select": "Button" })).is_err(),
            "html is required"
        );
    }

    #[test]
    fn nodes_match_by_role_and_label_relative_to_each_region() {
        let area = Bounds {
            x: 500.0,
            y: 200.0,
            w: 400.0,
            h: 300.0,
        };
        let native = [
            at(
                node("e1", "Group", Some("Settings")),
                500.0,
                200.0,
                400.0,
                300.0,
            ),
            at(
                node("e2", "Label", Some("General")),
                520.0,
                210.0,
                80.0,
                20.0,
            ),
            at(node("e3", "Button", Some("Save")), 520.0, 260.0, 60.0, 24.0),
            at(node("e4", "Switch", Some("On")), 520.0, 300.0, 30.0, 18.0),
            at(node("e5", "Switch", Some("On")), 520.0, 340.0, 30.0, 18.0),
            at(node("e6", "Button", Some("Sync")), 520.0, 380.0, 60.0, 24.0),
            at(
                node("e7", "TextInput", Some("Do anything…")),
                520.0,
                420.0,
                200.0,
                24.0,
            ),
            at(
                node("e8", "Button", Some("Elsewhere")),
                10.0,
                10.0,
                60.0,
                24.0,
            ),
        ];
        let mock = [
            at(
                node("w1-1", "Heading", Some("general")),
                21.0,
                11.0,
                80.0,
                20.0,
            ),
            at(node("w1-2", "Button", Some("Save")), 60.0, 60.0, 60.0, 24.0),
            at(node("w1-3", "Switch", Some("On")), 20.0, 141.0, 30.0, 18.0),
            at(node("w1-4", "Switch", Some("On")), 20.0, 99.0, 30.0, 18.0),
            at(
                node("w1-5", "Button", Some("Sync now")),
                20.0,
                180.0,
                60.0,
                24.0,
            ),
            at(
                node("w1-6", "TextInput", Some("do  anything")),
                20.0,
                220.0,
                200.0,
                24.0,
            ),
        ];
        let mock_area = Bounds {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        };
        let (native, mock) = (items(&native, &area), items(&mock, &mock_area));
        let reply = report_json(&pair(&native, &mock), 4.0);
        let labels = |key: &str| labels_of(&reply, key);
        assert_eq!(
            labels("matched"),
            [
                "Label General",
                "Switch On",
                "Switch On",
                "TextInput Do anything…"
            ]
        );
        assert_eq!(labels("moved"), ["Button Save"]);
        assert_eq!(labels("missing"), ["Button Sync now"]);
        assert_eq!(labels("extra"), ["Button Sync"]);
        let general = &reply["matched"][0];
        assert_eq!(
            (&general["dx"], &general["dy"]),
            (&json!(-1.0), &json!(-1.0))
        );
        assert_eq!(general["mockRole"], "Heading");
        let switches: Vec<&Value> = reply["matched"].as_array().unwrap()[1..3]
            .iter()
            .map(|m| &m["dy"])
            .collect();
        assert_eq!(
            switches,
            [&json!(1.0), &json!(-1.0)],
            "duplicates pair with the nearest one"
        );
        assert_eq!(reply["moved"][0]["dx"], json!(-40.0));
        assert_eq!(
            reply["missing"][0]["web"], true,
            "mock text is page-controlled"
        );
        let line = text(&reply, 4.0);
        assert!(
            line.contains("moved   Button \"Save\" dx=-40 dy=0 dw=0 dh=0 native @20,60 60x24 mock @60,60 60x24"),
            "{line}"
        );
        assert!(
            line.contains("missing Button \"Sync now\" mock @20,180 60x24 web"),
            "{line}"
        );
        assert!(
            line.contains("extra   Button \"Sync\" native @20,180 60x24\n"),
            "{line}"
        );
        assert!(reply.get("uncheckedText").is_none());
        assert_eq!(
            rounded(&Bounds {
                x: 16.0,
                y: 11.0,
                w: 281.2,
                h: 16.0
            })["w"],
            json!(281.2)
        );

        let without_text: Vec<Item> = native
            .into_iter()
            .filter(|item| item.key.0 != "text")
            .collect();
        let reply = report_json(&pair(&without_text, &mock), 4.0);
        assert_eq!(
            labels_of(&reply, "missing"),
            ["Button Sync now"],
            "mock text is not missing when the native region exposes no text"
        );
        assert_eq!(reply["uncheckedText"], 1);
        assert!(
            text(&reply, 4.0).contains("; 1 mock text nodes not compared"),
            "{}",
            text(&reply, 4.0)
        );
    }

    #[test]
    fn pixels_differ_only_past_the_tolerance() {
        let native = image::RgbaImage::from_fn(2, 2, |_, _| image::Rgba([100, 100, 100, 255]));
        let mut mock = native.clone();
        mock.put_pixel(0, 0, image::Rgba([100 + PIXEL_TOLERANCE, 100, 100, 255]));
        mock.put_pixel(1, 1, image::Rgba([200, 100, 100, 255]));
        let (ratio, highlight) = pixel_diff(&native, &mock);
        assert_eq!(ratio, 0.25);
        assert_eq!(highlight.get_pixel(0, 0), native.get_pixel(0, 0));
        assert_ne!(highlight.get_pixel(1, 1), native.get_pixel(1, 1));
        let small = image::RgbaImage::from_fn(1, 2, |_, _| image::Rgba([100, 100, 100, 255]));
        assert_eq!(
            pixel_diff(&native, &small).0,
            0.5,
            "pixels the mockup lacks differ"
        );
    }
}
