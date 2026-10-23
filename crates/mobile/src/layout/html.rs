use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use zeron_proto::html_render::{self as shared, HtmlRenderRef};
use zeron_text::WhiteSpace;

use super::display::{ColorRole, DisplayBuilder, WidgetKind};
use super::markdown::{Ctx, PText, Px, prepare_plain};
use super::rows::place_text_lines;
use super::style::{Family, Weight};
use crate::visual::Rgba;
use crate::visual::palette::{Palette, hex};

const HEADER: f32 = 22.0;
const HEADER_GAP: f32 = 6.0;
const ICON: f32 = 13.0;
const EXPAND: f32 = 22.0;

pub(crate) struct HtmlSlot {
    pub id: String,
    pub row_key: u64,
    pub device_id: String,
    pub path: String,
    agent_height: f32,
    state: Mutex<Measured>,
    epoch: AtomicU64,
}

#[derive(Default)]
struct Measured {
    content: Option<f32>,
    // A page sized to its viewport would otherwise resize without end (desktop `capped`).
    capped: bool,
}

impl HtmlSlot {
    pub fn new(id: String, row_key: u64, device_id: &str, page: &HtmlRenderRef) -> Arc<Self> {
        Arc::new(Self {
            id,
            row_key,
            device_id: device_id.to_owned(),
            path: page.path.clone(),
            agent_height: shared::clamp_height(f64::from(page.height)) as f32,
            state: Mutex::new(Measured::default()),
            epoch: AtomicU64::new(0),
        })
    }

    pub fn same_page(&self, device_id: &str, page: &HtmlRenderRef) -> bool {
        self.device_id == device_id
            && self.path == page.path
            && self.agent_height == shared::clamp_height(f64::from(page.height)) as f32
    }

    pub fn snapshot(&self) -> (u64, f32) {
        let state = self.state.lock().unwrap();
        let frame = if state.capped {
            self.agent_height
        } else {
            shared::frame_height(self.agent_height, state.content)
        };
        (self.epoch.load(Ordering::Relaxed), frame)
    }

    pub fn set_content(&self, height: f32) -> bool {
        let height = height.round();
        let mut state = self.state.lock().unwrap();
        if state.content == Some(height) {
            return false;
        }
        state.content = Some(height);
        state.capped |= shared::overflows(self.agent_height, height);
        self.epoch.fetch_add(1, Ordering::Relaxed);
        true
    }

    fn content(&self) -> Option<f32> {
        self.state.lock().unwrap().content
    }
}

pub(crate) struct HtmlRow {
    pub slot: Arc<HtmlSlot>,
    pub title: PText,
    pub name: String,
}

pub(crate) fn html_row(ctx: &mut Ctx, slot: Arc<HtmlSlot>, title: &str) -> HtmlRow {
    let (size, lh) = super::style::TYPE.small;
    let style = ctx.typo.style(Family::Sans, Weight::Medium, false, size);
    let lh = ctx.typo.px(lh);
    HtmlRow {
        title: prepare_plain(ctx, title, style, lh, ColorRole::TextSecondary, WhiteSpace::Normal),
        name: title.to_owned(),
        slot,
    }
}

pub(crate) fn place_html(row: &HtmlRow, frame: f32, px: Px, x: f32, top: f32, cw: f32, out: Option<&mut DisplayBuilder>) -> f32 {
    let header = px.v(HEADER).max(row.title.lh);
    let page_top = top + header + px.v(HEADER_GAP);
    if let Some(out) = out {
        let icon = px.v(ICON);
        out.widget(
            WidgetKind::Icon { name: "globe".into(), color: ColorRole::TextTertiary },
            (x, top + (header - icon) / 2.0, icon, icon),
            None,
        );
        let tx = x + icon + px.v(6.0);
        let expand = px.v(EXPAND);
        place_text_lines(
            &row.title,
            tx,
            top + (header - row.title.lh) / 2.0,
            (cw - (tx - x) - expand - px.v(6.0)).max(1.0),
            1,
            px,
            out,
        );
        let slot = &row.slot;
        out.widget(
            WidgetKind::HtmlExpand {
                id: slot.id.clone(),
                device_id: slot.device_id.clone(),
                path: slot.path.clone(),
                title: row.name.clone(),
            },
            (
                x + cw - expand,
                top + (header - expand) / 2.0,
                expand,
                expand,
            ),
            None,
        );
        out.widget(
            WidgetKind::HtmlRender {
                id: slot.id.clone(),
                device_id: slot.device_id.clone(),
                path: slot.path.clone(),
                scrolls: shared::scrolls_inside(frame, slot.content()),
            },
            (x, page_top, cw, frame),
            None,
        );
    }
    header + px.v(HEADER_GAP) + frame
}

fn css(c: Rgba) -> String {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("rgba({},{},{},{})", ch(c.r), ch(c.g), ch(c.b), (c.a.clamp(0.0, 1.0) * 1000.0).round() / 1000.0)
}

fn theme_css(dark: bool, text_scale: f32) -> String {
    let p = Palette::new(dark);
    let pick = |light: u32, night: u32| css(hex(if dark { night } else { light }));
    let background = pick(0xF3F3F5, 0x060606);
    let card = pick(0xFFFFFF, 0x111113);
    let chip = pick(0xE7E7EB, 0x1C1C20);
    let code = pick(0xFAFAFB, 0x0B0B0D);
    let (info, info_foreground) = if dark { ("#3b82f6", "#60a5fa") } else { ("#3b82f6", "#1d4ed8") };
    let mut vars: Vec<(&str, String)> = vec![
        ("--background", background.clone()),
        ("--foreground", css(p.text)),
        ("--faint-foreground", css(p.faint)),
        ("--muted", chip.clone()),
        ("--muted-foreground", css(p.muted)),
        ("--hover", css(p.wash)),
        ("--selection", css(p.accent.alpha(0.25))),
        ("--card", card.clone()),
        ("--card-foreground", css(p.text)),
        ("--popover", card.clone()),
        ("--popover-foreground", css(p.text)),
        ("--secondary", chip.clone()),
        ("--secondary-foreground", css(p.text)),
        ("--border", css(p.border)),
        ("--border-strong", css(p.border_strong)),
        ("--input", chip),
        ("--ring", css(p.accent)),
        ("--primary", css(p.text)),
        ("--primary-foreground", background),
        ("--accent", css(p.accent)),
        ("--accent-foreground", css(p.on_accent)),
        ("--accent-surface", css(p.accent.alpha(0.12))),
        ("--accent-surface-foreground", css(p.accent)),
        ("--destructive", css(p.danger)),
        ("--destructive-foreground", css(p.danger)),
        ("--destructive-surface", css(p.danger.alpha(0.12))),
        ("--danger", css(p.danger)),
        ("--warning", css(p.warning)),
        ("--warning-foreground", css(p.warning)),
        ("--warning-surface", css(p.warning.alpha(0.12))),
        ("--success", css(p.success)),
        ("--success-foreground", css(p.success)),
        ("--success-surface", css(p.success.alpha(0.12))),
        ("--info", info.into()),
        ("--info-foreground", info_foreground.into()),
        ("--code-background", code),
        ("--code-foreground", css(p.text)),
    ];
    const CHART: [&str; 6] = ["--chart-1", "--chart-2", "--chart-3", "--chart-4", "--chart-5", "--chart-6"];
    vars.extend(CHART.into_iter().zip(p.chart).map(|(name, c)| (name, css(c))));
    vars.push(("--radius", "0.625rem".into()));
    vars.push(("--font-sans", "-apple-system, system-ui, sans-serif".into()));
    vars.push(("--font-mono", "ui-monospace, \"SF Mono\", Menlo, monospace".into()));
    let px = |v: f32| format!("{}px", (v * text_scale * 100.0).round() / 100.0);
    let (body, line) = super::style::TYPE.body;
    let (small, _) = super::style::TYPE.small;
    vars.extend([
        ("--font-size-xs", px(12.0)),
        ("--font-size-sm", px(small)),
        ("--font-size", px(body)),
        ("--font-size-lg", px(super::style::TYPE.h[2].0)),
        ("--font-size-xl", px(super::style::TYPE.h[1].0)),
        ("--font-size-2xl", px(super::style::TYPE.h[0].0)),
        ("--font-size-code", px(super::style::TYPE.code.0)),
        ("--line-height", px(line)),
        ("--surface-treatment", "opaque".into()),
    ]);
    let mut out = format!(":root{{color-scheme:{};", if dark { "dark" } else { "light" });
    for (name, value) in vars {
        out.push_str(name);
        out.push(':');
        out.push_str(&value);
        out.push(';');
    }
    out.push_str("}html{background:transparent;overscroll-behavior:none;color:var(--foreground);font-family:var(--font-sans);font-size:14px;line-height:1.5;-webkit-font-smoothing:antialiased;-webkit-text-size-adjust:100%;scrollbar-width:none}html::-webkit-scrollbar{display:none}body{margin:0}code,kbd,pre,samp{font-family:var(--font-mono)}");
    out
}

#[uniffi::export]
pub fn html_render_document(html: String, dark: bool, text_scale: f32) -> String {
    let mut script = format!(
        "(function(){{var d=document.documentElement;d.setAttribute(\"data-surface\",\"opaque\");d.setAttribute(\"data-appearance\",\"{}\");}})();",
        if dark { "dark" } else { "light" }
    );
    script.push_str("window.ipc={postMessage:function(m){window.webkit.messageHandlers.zeron.postMessage(m);}};");
    script.push_str(shared::SIZE_SCRIPT);
    shared::inject_bootstrap(&html, &theme_css(dark, text_scale), &script)
}

#[uniffi::export]
pub fn html_render_source(html: String) -> String {
    shared::source_text(&html)
}

#[uniffi::export]
pub fn html_render_file_name(title: String) -> String {
    shared::page_file_name(&title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_document_carries_every_theme_name_and_the_height_bridge() {
        let page = html_render_document("<p>x</p>".into(), true, 1.0);
        let head = page.find("<head>").unwrap();
        assert!(page[head..].contains("zeron-theme"));
        assert!(page.ends_with("<p>x</p>"));
        assert!(page.contains("messageHandlers.zeron.postMessage") && page.contains("{height:v}"));
        for name in [
            "--background:", "--foreground:", "--muted-foreground:", "--card:", "--border:",
            "--primary:", "--accent:", "--destructive:", "--success:", "--chart-6:", "--font-sans:",
            "--font-mono:", "--font-size:", "--font-size-2xl:", "--line-height:", "--hover:",
            "--surface-treatment:",
        ] {
            assert!(page.contains(name), "{name} missing");
            assert!(shared::THEME_GUIDE.contains(name.trim_end_matches(':')));
        }
        assert!(page.contains("color-scheme:dark;"));
        assert!(html_render_document("<p>x</p>".into(), false, 1.0).contains("color-scheme:light;"));
    }
}
