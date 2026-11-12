//! Agent-published HTML pages (`html_render` / `html_preview` thread tools):
//! the limits and agent guidance shared by the MCP server, the engine and the
//! desktop UI. Ported from t3code `packages/shared/src/htmlRender.ts`.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const RENDER_TOOL: &str = "html_render";
pub const PREVIEW_TOOL: &str = "html_preview";
pub const MIN_HEIGHT: u32 = 80;
pub const MAX_HEIGHT: u32 = 2000;
pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_HTML_BYTES: usize = 512_000;
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
/// Below the desktop's 24 MiB attachment read limit, so every stored page
/// can be shown.
pub const MAX_PAGE_BYTES: usize = 20 * 1024 * 1024;
pub const MIN_PREVIEW_WIDTH: u32 = 240;
pub const MAX_PREVIEW_WIDTH: u32 = 1600;
/// The reply column at the default chat width (`TRANSCRIPT_WIDTH_DEFAULT`).
pub const COLUMN_WIDTH: u32 = 736;
pub const MAX_URL_BYTES: usize = 2048;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishHtmlRenderParams {
    pub chat_id: String,
    pub run_secret: String,
    #[serde(default)]
    pub html: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    pub title: String,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewAppearance {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HtmlPreviewParams {
    pub chat_id: String,
    pub run_secret: String,
    #[serde(default)]
    pub html: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub appearance: Option<PreviewAppearance>,
    #[serde(flatten)]
    pub inspect: PreviewInspect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadWorkspacePageParams {
    pub chat_id: String,
    pub run_secret: String,
    pub path: String,
}

pub const MAX_PREVIEW_STEPS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PreviewRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewInspect {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tree: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<PreviewRect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nth: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<serde_json::Value>,
}

impl PreviewInspect {
    pub fn validate(&self) -> Result<(), String> {
        if self.steps.len() > MAX_PREVIEW_STEPS {
            return Err(format!(
                "`steps` takes at most {MAX_PREVIEW_STEPS} steps, not {}",
                self.steps.len()
            ));
        }
        if self.rect.is_some() && self.select.is_some() {
            return Err("give `rect` or `select`, not both".into());
        }
        if let Some(r) = self.rect
            && !([r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite())
                && r.x >= 0.0
                && r.y >= 0.0
                && r.w >= 1.0
                && r.h >= 1.0)
        {
            return Err("`rect` needs finite x, y >= 0 and w, h >= 1 CSS pixels".into());
        }
        Ok(())
    }
}

/// A page the engine asks the desktop app to lay out and screenshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HtmlPreviewRequest {
    pub id: String,
    pub html: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub width: u32,
    pub appearance: PreviewAppearance,
    #[serde(flatten)]
    pub inspect: PreviewInspect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsoleMessage {
    /// log, info, warning or error (uncaught exceptions included).
    pub level: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HtmlPreview {
    pub width: u32,
    /// The height the page needs at this width.
    pub content_height: u32,
    pub captured_height: u32,
    pub console_messages: Vec<ConsoleMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_images: Vec<String>,
    /// Base64 PNG of the top `captured_height` points.
    pub png: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<PreviewRect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<PreviewShot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewShot {
    pub step: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub png: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RespondHtmlPreviewParams {
    pub id: String,
    #[serde(default)]
    pub preview: Option<HtmlPreview>,
    #[serde(default)]
    pub error: Option<String>,
}

/// The tallest capture a preview returns; longer pages report their full
/// `content_height` and are cut here.
pub const MAX_CAPTURE_HEIGHT: u32 = 4000;

pub fn preview_width(width: Option<u32>) -> u32 {
    width
        .unwrap_or(COLUMN_WIDTH)
        .clamp(MIN_PREVIEW_WIDTH, MAX_PREVIEW_WIDTH)
}

/// What a published page's Tool part carries so any client can show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HtmlRenderRef {
    /// The stored page on the device that ran the chat (read back with
    /// ReadAttachmentChunk, like a generated image). Empty for a live `url`.
    pub path: String,
    pub title: String,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderSource<'a> {
    Html(&'a str),
    Url(&'a str),
}

pub fn render_source<'a>(
    html: Option<&'a str>,
    url: Option<&'a str>,
) -> Result<RenderSource<'a>, String> {
    let given = |text: Option<&'a str>| text.filter(|text| !text.trim().is_empty());
    match (given(html), given(url)) {
        (Some(html), None) => Ok(RenderSource::Html(html)),
        (None, Some(url)) => live_url(url).map(|live| RenderSource::Url(live.url)),
        (Some(_), Some(_)) => Err("give `html` or `url`, not both".into()),
        (None, None) => Err(
            "give `html` (a finished page) or `url` (a local dev server you are running)".into(),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderInput<'a> {
    Html(&'a str),
    Path(&'a str),
    Url(&'a str),
}

pub fn render_input<'a>(
    html: Option<&'a str>,
    path: Option<&'a str>,
    url: Option<&'a str>,
) -> Result<RenderInput<'a>, String> {
    let given = |text: Option<&'a str>| text.filter(|text| !text.trim().is_empty());
    match (given(html), given(path), given(url)) {
        (Some(html), None, None) => Ok(RenderInput::Html(html)),
        (None, Some(path), None) => Ok(RenderInput::Path(path.trim())),
        (None, None, Some(url)) => live_url(url).map(|live| RenderInput::Url(live.url)),
        (None, None, None) => Err(
            "give `html` (a finished page), `path` (an .html file you wrote in this chat's folder) or `url` (a local dev server you are running)".into(),
        ),
        _ => Err("give one of `html`, `path` or `url`, not both".into()),
    }
}

/// An engine older than the app refuses `path` and `url` with these words.
pub fn stale_engine_reply(error: &str, sent: RenderInput<'_>) -> Option<String> {
    let what = match sent {
        RenderInput::Html(_) => return None,
        RenderInput::Path(_) => "`path`",
        RenderInput::Url(_) => "`url`",
    };
    let required_html = error.contains("missing field") && error.contains("html");
    let no_path = matches!(sent, RenderInput::Path(_))
        && error.contains("give `html` (a finished page) or `url`");
    (required_html || no_path).then(|| {
        format!(
            "The Zeron engine serving this chat is older than the app and does not take {what} yet. Restart the engine (quit Zeron fully and open it again), then call html_render again; until then pass the page as `html`."
        )
    })
}

pub const PICK_GUIDE: &str = "Options: when the page offers choices to pick from, make each option a <button> with data-zeron-pick=\"A2\" (its code), data-zeron-group=\"A\" (one pick per group; leave it out for options picked freely) and data-zeron-label=\"Floating tab\"; data-zeron-group-label names a group. A click puts the pick in the person's composer; the host marks chosen buttons data-zeron-chosen and sent ones data-zeron-locked, and fills an element with data-zeron-count. You receive the picks with their message.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub group: Option<String>,
    pub code: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickGroup {
    pub code: String,
    pub name: String,
}

/// The agent's words, locked in DECISIONS whatever the app's language.
pub const PICKED_IN: &str = "Đã chọn trong";
pub const NOT_PICKED: &str = "Chưa chọn:";

fn one_line(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn picks_block(card: &str, picks: &[Pick], missing: &[PickGroup]) -> String {
    let mut out = format!("{PICKED_IN} \"{}\":", one_line(card));
    for pick in picks {
        out.push('\n');
        match &pick.group {
            Some(group) => out.push_str(&format!(
                "{group} = {} ({})",
                pick.code,
                one_line(&pick.label)
            )),
            None => out.push_str(&format!("- {}", one_line(&pick.label))),
        }
    }
    if !missing.is_empty() {
        let names: Vec<String> = missing
            .iter()
            .map(|group| format!("{} ({})", group.code, one_line(&group.name)))
            .collect();
        out.push_str(&format!("\n{NOT_PICKED} {}", names.join(", ")));
    }
    out
}

pub fn with_picks(words: &str, cards: &[(String, Vec<Pick>, Vec<PickGroup>)]) -> String {
    let mut out = words.trim_end().to_owned();
    for (card, picks, missing) in cards {
        if picks.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&picks_block(card, picks, missing));
    }
    out
}

/// Only trailing blocks in exactly `with_picks`'s form read back, so typed text stays words.
pub fn extract_picks(text: &str) -> (String, Vec<(String, Vec<Pick>)>) {
    let mut rest = text.trim_end();
    let mut cards = Vec::new();
    loop {
        let (before, block) = match rest.rfind("\n\n") {
            Some(at) => (&rest[..at], &rest[at + 2..]),
            None => ("", rest),
        };
        let Some(card) = picks_of_block(block) else {
            break;
        };
        cards.push(card);
        rest = before;
        if rest.is_empty() {
            break;
        }
    }
    if cards.is_empty() {
        return (text.trim_end().to_owned(), cards);
    }
    cards.reverse();
    (rest.trim_end().to_owned(), cards)
}

fn picks_of_block(block: &str) -> Option<(String, Vec<Pick>)> {
    let mut lines = block.split('\n');
    let card = lines
        .next()?
        .strip_prefix(PICKED_IN)?
        .strip_prefix(" \"")?
        .strip_suffix("\":")?;
    if card.is_empty() {
        return None;
    }
    let mut picks = Vec::new();
    let mut missing = false;
    for line in lines {
        if missing {
            return None;
        }
        if let Some(label) = line.strip_prefix("- ") {
            if label.trim().is_empty() {
                return None;
            }
            picks.push(Pick {
                group: None,
                code: label.to_owned(),
                label: label.to_owned(),
            });
        } else if let Some((group, rest)) = line.split_once(" = ")
            && let Some((code, label)) = rest.split_once(" (")
            && let Some(label) = label.strip_suffix(')')
            && !group.is_empty()
            && !group.contains(char::is_whitespace)
            && !code.is_empty()
            && !code.contains(char::is_whitespace)
        {
            picks.push(Pick {
                group: Some(group.to_owned()),
                code: code.to_owned(),
                label: label.to_owned(),
            });
        } else if line.starts_with(NOT_PICKED) && !picks.is_empty() {
            missing = true;
        } else {
            return None;
        }
    }
    (!picks.is_empty()).then(|| (card.to_owned(), picks))
}

/// A local dev server's page, the only kind of `url` a thread shows live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveUrl<'a> {
    pub url: &'a str,
    /// `host:port` as the URL spells it.
    pub server: &'a str,
    /// `host:port/path` without query or fragment.
    pub label: &'a str,
    /// Without IPv6 brackets.
    pub host: &'a str,
    pub port: u16,
}

pub fn live_url(url: &str) -> Result<LiveUrl<'_>, String> {
    let url = url.trim();
    let refused = || {
        let shown: String = url.chars().take(200).collect();
        format!(
            "`url` takes only a local dev server: http(s)://localhost, 127.0.0.1 or [::1], any port and path, not `{shown}`."
        )
    };
    if url.len() > MAX_URL_BYTES
        || url
            .chars()
            .any(|c| !c.is_ascii_graphic() || "\\\"<>{}|^`".contains(c))
    {
        return Err(refused());
    }
    let (rest, default_port) = match url.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => (rest, 80),
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => (rest, 443),
        _ => return Err(refused()),
    };
    let server = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let (host, port, bracketed) = match server.strip_prefix('[') {
        Some(inner) => {
            let (host, port) = inner.split_once(']').ok_or_else(refused)?;
            (host, port, true)
        }
        None => match server.find(':') {
            Some(at) => (&server[..at], &server[at..], false),
            None => (server, "", false),
        },
    };
    let loopback = if bracketed {
        host == "::1"
    } else {
        host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1"
    };
    let port = match port.strip_prefix(':') {
        _ if !loopback => return Err(refused()),
        None if port.is_empty() => default_port,
        Some(port) if port.bytes().all(|b| b.is_ascii_digit()) => port
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(refused)?,
        _ => return Err(refused()),
    };
    let label = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
    Ok(LiveUrl {
        url,
        server,
        label: label.trim_end_matches('/'),
        host,
        port,
    })
}

pub fn clamp_height(height: f64) -> u32 {
    if height.is_finite() {
        (height.round() as i64).clamp(MIN_HEIGHT as i64, MAX_HEIGHT as i64) as u32
    } else {
        MIN_HEIGHT
    }
}

/// The frame fits the page so it never scrolls inside the thread and takes
/// the reader's scroll. The agent's height caps it only when it is below the
/// page's own height (the agent asked for a scrolling frame).
pub fn frame_height(agent_height: f32, content_height: Option<f32>) -> f32 {
    clamp_height(f64::from(match content_height {
        Some(content) if !overflows(agent_height, content) => content,
        Some(_) | None => agent_height,
    })) as f32
}

/// A page previewed at another width wraps a little taller here; that drift
/// is not a request to scroll.
pub fn overflows(agent_height: f32, content_height: f32) -> bool {
    content_height > agent_height + FIT_DRIFT
}

const FIT_DRIFT: f32 = 64.0;

/// Whether the page is taller than its frame and scrolls inside it.
pub fn scrolls_inside(frame: f32, content_height: Option<f32>) -> bool {
    content_height.is_some_and(|content| content > frame + 1.0)
}

/// Reports the page's content height to the host as it changes (t3's
/// measure: the document's scroll height when it overflows, else its box).
pub const SIZE_SCRIPT: &str = r#"(function(){var h,z=function(){var r=document.documentElement,v=Math.ceil(r.scrollHeight>r.clientHeight?r.scrollHeight:r.getBoundingClientRect().height);if(v===h||!window.ipc)return;h=v;window.ipc.postMessage(JSON.stringify({height:v}));};if(window.ResizeObserver){var o=new ResizeObserver(z);o.observe(document.documentElement);document.addEventListener("DOMContentLoaded",function(){if(document.body)o.observe(document.body);});}document.addEventListener("DOMContentLoaded",z);window.addEventListener("load",z);})();"#;

/// A color as straight RGBA, each channel 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    /// `#rrggbb` or `#rrggbbaa`.
    pub fn hex(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if !matches!(digits.len(), 6 | 8) || !digits.is_ascii() {
            return None;
        }
        let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
        let a = if digits.len() == 8 { byte(6)? } else { 255 };
        Some(Self {
            r: f32::from(byte(0)?) / 255.0,
            g: f32::from(byte(2)?) / 255.0,
            b: f32::from(byte(4)?) / 255.0,
            a: f32::from(a) / 255.0,
        })
    }
}

/// The theme a page is handed, by the roles both apps' themes name.
#[derive(Debug, Clone, PartialEq)]
pub struct PageTheme {
    pub dark: bool,
    pub glass: bool,
    pub bg: Rgba,
    pub text: Rgba,
    pub text_faint: Rgba,
    pub text_muted: Rgba,
    pub element_hover: Rgba,
    pub hover: Rgba,
    pub selection: Rgba,
    pub surface_card: Rgba,
    pub surface_overlay: Rgba,
    pub surface_raised: Rgba,
    pub border: Rgba,
    pub border_strong: Rgba,
    pub input_bg: Rgba,
    pub accent: Rgba,
    pub solid: Rgba,
    pub on_solid: Rgba,
    pub on_accent: Rgba,
    pub accent_wash: Rgba,
    pub accent_strong: Rgba,
    pub danger: Rgba,
    pub danger_strong: Rgba,
    pub danger_muted: Rgba,
    pub warning: Rgba,
    pub warning_muted: Rgba,
    pub success: Rgba,
    pub success_muted: Rgba,
    pub code_wash: Rgba,
    pub code_text: Rgba,
    pub font_sans: String,
    pub font_mono: String,
    pub code_font_size: f32,
    /// The transcript's prose size and line height at a 16px interface.
    pub text_size: f32,
    pub line_height: f32,
}

pub fn css_color(color: Rgba) -> String {
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "rgba({},{},{},{})",
        channel(color.r),
        channel(color.g),
        channel(color.b),
        (color.a.clamp(0.0, 1.0) * 1000.0).round() / 1000.0
    )
}

fn font_stack(family: &str, generic: &str) -> String {
    let family = family.replace(['"', ';', '{', '}', '<', '>'], "");
    if family.trim().is_empty() {
        generic.into()
    } else {
        format!("\"{family}\", {generic}")
    }
}

/// Roles themes cannot change, plus a categorical chart series after the
/// theme's own accent (t3 `FIXED_COLORS`).
fn fixed(dark: bool) -> (&'static str, &'static str, [&'static str; 5]) {
    if dark {
        (
            "#3b82f6",
            "#60a5fa",
            ["#2dd4bf", "#fbbf24", "#c084fc", "#fb7185", "#a3e635"],
        )
    } else {
        (
            "#3b82f6",
            "#1d4ed8",
            ["#0d9488", "#d97706", "#9333ea", "#e11d48", "#65a30d"],
        )
    }
}

fn css_px(value: f32) -> String {
    format!("{}px", (value * 100.0).round() / 100.0)
}

/// The variables a page styles against, named like t3's so one page works in
/// both apps. `--accent` is the brand color, not a hover surface.
pub fn theme_variables(theme: &PageTheme, font_size: f32) -> Vec<(&'static str, String)> {
    let (info, info_foreground, chart) = fixed(theme.dark);
    let c = css_color;
    let mut vars = vec![
        ("--background", c(theme.bg)),
        ("--foreground", c(theme.text)),
        ("--faint-foreground", c(theme.text_faint)),
        ("--muted", c(theme.element_hover)),
        ("--muted-foreground", c(theme.text_muted)),
        ("--hover", c(theme.hover)),
        ("--selection", c(theme.selection)),
        ("--card", c(theme.surface_card)),
        ("--card-foreground", c(theme.text)),
        ("--popover", c(theme.surface_overlay)),
        ("--popover-foreground", c(theme.text)),
        ("--secondary", c(theme.surface_raised)),
        ("--secondary-foreground", c(theme.text)),
        ("--border", c(theme.border)),
        ("--border-strong", c(theme.border_strong)),
        ("--input", c(theme.input_bg)),
        ("--ring", c(theme.accent)),
        ("--primary", c(theme.solid)),
        ("--primary-foreground", c(theme.on_solid)),
        ("--accent", c(theme.accent)),
        ("--accent-foreground", c(theme.on_accent)),
        ("--accent-surface", c(theme.accent_wash)),
        ("--accent-surface-foreground", c(theme.accent_strong)),
        ("--destructive", c(theme.danger)),
        ("--destructive-foreground", c(theme.danger_strong)),
        ("--destructive-surface", c(theme.danger_muted)),
        ("--danger", c(theme.danger)),
        ("--warning", c(theme.warning)),
        ("--warning-foreground", c(theme.warning)),
        ("--warning-surface", c(theme.warning_muted)),
        ("--success", c(theme.success)),
        ("--success-foreground", c(theme.success)),
        ("--success-surface", c(theme.success_muted)),
        ("--info", info.into()),
        ("--info-foreground", info_foreground.into()),
        ("--code-background", c(theme.code_wash)),
        ("--code-foreground", c(theme.code_text)),
        ("--chart-1", c(theme.accent)),
    ];
    const CHART: [&str; 5] = [
        "--chart-2",
        "--chart-3",
        "--chart-4",
        "--chart-5",
        "--chart-6",
    ];
    vars.extend(
        CHART
            .into_iter()
            .zip(chart)
            .map(|(name, color)| (name, color.into())),
    );
    vars.push(("--radius", "0.625rem".into()));
    vars.push((
        "--font-sans",
        font_stack(
            &theme.font_sans,
            "-apple-system, BlinkMacSystemFont, system-ui, sans-serif",
        ),
    ));
    vars.push((
        "--font-mono",
        font_stack(&theme.font_mono, "\"SF Mono\", Menlo, monospace"),
    ));
    // The transcript's sizes at 16px: meta, label, prose, then its h3/h2/h1.
    let type_scale: [(&'static str, f32); 7] = [
        ("--font-size-xs", 11.0),
        ("--font-size-sm", 12.0),
        ("--font-size", theme.text_size),
        ("--font-size-lg", 15.0),
        ("--font-size-xl", 16.0),
        ("--font-size-2xl", 19.0),
        ("--line-height", theme.line_height),
    ];
    let scale = font_size / 16.0;
    vars.extend(
        type_scale
            .into_iter()
            .map(|(name, size)| (name, css_px(size * scale))),
    );
    vars.push(("--font-size-code", css_px(theme.code_font_size)));
    vars.push(("--surface-treatment", surface(theme).into()));
    vars
}

fn surface(theme: &PageTheme) -> &'static str {
    if theme.glass { "glass" } else { "opaque" }
}

fn appearance(theme: &PageTheme) -> &'static str {
    if theme.dark { "dark" } else { "light" }
}

/// The theme stylesheet the bootstrap's `<style id="zeron-theme">` holds.
/// Inline cards sit on the thread itself, so their page background is clear;
/// a preview screenshot paints the thread's background.
pub fn theme_css(theme: &PageTheme, font_size: f32, inline: bool) -> String {
    let mut css = format!(":root{{color-scheme:{};", appearance(theme));
    for (name, value) in theme_variables(theme, font_size) {
        css.push_str(name);
        css.push(':');
        css.push_str(&value);
        css.push(';');
    }
    css.push('}');
    css.push_str(if inline {
        "html{background:transparent;overscroll-behavior:none;"
    } else {
        "html{background:var(--background);"
    });
    css.push_str("color:var(--foreground);font-family:var(--font-sans);font-size:14px;line-height:1.5;-webkit-font-smoothing:antialiased;-webkit-text-size-adjust:100%;scrollbar-width:none}html::-webkit-scrollbar{display:none}body{margin:0}code,kbd,pre,samp{font-family:var(--font-mono)}");
    css
}

/// The host hands the wheel back to the transcript once the page is scrolled to that edge.
pub const EDGE_SCRIPT: &str = r#"(function(){var l=-1,e=function(){var s=document.scrollingElement||document.documentElement,y=s.scrollTop,x=s.scrollLeft,v=(y<=0?1:0)|(y+s.clientHeight>=s.scrollHeight-1?2:0)|(x<=0?4:0)|(x+s.clientWidth>=s.scrollWidth-1?8:0);if(v===l||!window.ipc)return;l=v;window.ipc.postMessage(JSON.stringify({edges:v}));};window.addEventListener("scroll",e,{passive:true});window.addEventListener("resize",e);window.addEventListener("load",e);document.addEventListener("DOMContentLoaded",e);if(window.ResizeObserver)new ResizeObserver(e).observe(document.documentElement);})();"#;

/// Snapshots are taken only while the page reports a change, at most every 250ms.
pub const ACTIVITY_SCRIPT: &str = r#"(function(){var t=0;function a(){var n=Date.now();if(n-t<250||!window.ipc)return;t=n;window.ipc.postMessage(JSON.stringify({active:1}));}function w(f){return typeof f==="function"?function(){a();return f.apply(this,arguments);}:f;}var r=window.requestAnimationFrame;window.requestAnimationFrame=function(f){a();return r.call(window,f);};var st=window.setTimeout,si=window.setInterval;window.setTimeout=function(f){var g=[].slice.call(arguments);g[0]=w(f);return st.apply(window,g);};window.setInterval=function(f){var g=[].slice.call(arguments);g[0]=w(f);return si.apply(window,g);};new MutationObserver(a).observe(document.documentElement,{subtree:true,childList:true,attributes:true,characterData:true});["transitionrun","transitionend","animationstart","animationiteration","animationend","input","change","scroll","load"].forEach(function(e){window.addEventListener(e,a,true);});})();"#;

pub fn attributes_script(theme: &PageTheme) -> String {
    format!(
        "(function(){{var d=document.documentElement;d.setAttribute(\"data-surface\",\"{}\");d.setAttribute(\"data-appearance\",\"{}\");}})();",
        surface(theme),
        appearance(theme)
    )
}

/// JavaScript that swaps a mounted page's theme without reloading it.
pub fn retheme_script(theme: &PageTheme, font_size: f32, inline: bool) -> String {
    let css = serde_json::to_string(&theme_css(theme, font_size, inline))
        .unwrap_or_else(|_| "\"\"".into());
    format!(
        "(function(){{var s=document.getElementById('zeron-theme');if(s)s.textContent={css};}})();{}",
        attributes_script(theme)
    )
}

/// The page with its theme bootstrap; inline cards report their height, hand
/// the wheel back at their edges and say when they change, then run `extra`.
pub fn themed_page(
    html: &str,
    theme: &PageTheme,
    font_size: f32,
    inline: bool,
    extra: &str,
) -> String {
    let mut script = attributes_script(theme);
    if inline {
        script.push_str(SIZE_SCRIPT);
        script.push_str(EDGE_SCRIPT);
        script.push_str(ACTIVITY_SCRIPT);
    }
    script.push_str(extra);
    inject_bootstrap(html, &theme_css(theme, font_size, inline), &script)
}

/// Inserts the bootstrap (`css` in `<style id="zeron-theme">`, then
/// `script`) at the start of the document head, so a page's own styles and
/// scripts come after it and win.
pub fn inject_bootstrap(html: &str, css: &str, script: &str) -> String {
    let scan = blank_non_markup(html);
    let scan_text = String::from_utf8_lossy(&scan);
    let markup = bootstrap_markup(&scan_text, css, script);
    let find_open = |name: &[u8]| (0..scan.len()).find(|&p| tag_at(&scan, p, name));
    if let Some(head) = find_open(b"head") {
        let at = tag_end(&scan, head);
        return format!("{}{markup}{}", &html[..at], &html[at..]);
    }
    if let Some(open) = find_open(b"html") {
        let at = tag_end(&scan, open);
        return format!("{}<head>{markup}</head>{}", &html[..at], &html[at..]);
    }
    let trimmed = html.trim_start();
    if trimmed
        .get(..9)
        .is_some_and(|start| start.eq_ignore_ascii_case("<!doctype"))
    {
        let start = html.len() - trimmed.len();
        let at = tag_end(&scan, start);
        return format!("{}<head>{markup}</head>{}", &html[..at], &html[at..]);
    }
    format!("<!doctype html><head>{markup}</head>{html}")
}

fn bootstrap_markup(scan: &str, css: &str, script: &str) -> String {
    let mut markup = String::new();
    let head = &scan[..scan.floor_char_boundary(4096)];
    if !(contains_ci(head, "<meta") && contains_ci(head, "charset")) {
        markup.push_str("<meta charset=\"utf-8\">");
    }
    if !contains_ci(scan, "name=\"viewport\"") && !contains_ci(scan, "name=viewport") {
        markup.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    }
    markup.push_str("<style id=\"zeron-theme\">");
    markup.push_str(&css.replace("</", "<\\/"));
    markup.push_str("</style><script>");
    markup.push_str(script);
    markup.push_str("</script>");
    markup
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    find_ci(haystack.as_bytes(), needle.as_bytes(), 0).is_some()
}

fn find_ci(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len())
        .find(|&at| haystack[at..at + needle.len()].eq_ignore_ascii_case(needle))
}

/// The tag starting at `at` (`<name`) is a real open tag of `name`: the name
/// is followed by whitespace, `>`, `/` or the end.
fn tag_at(scan: &[u8], at: usize, name: &[u8]) -> bool {
    let end = at + 1 + name.len();
    scan.len() >= end
        && scan[at] == b'<'
        && scan[at + 1..end].eq_ignore_ascii_case(name)
        && scan
            .get(end)
            .is_none_or(|c| c.is_ascii_whitespace() || *c == b'>' || *c == b'/')
}

/// `</name` at `at`, followed by whitespace, `>`, `/` or the end.
fn close_tag_at(scan: &[u8], at: usize, name: &[u8]) -> bool {
    scan.get(at..at + 2) == Some(b"</".as_slice())
        && scan
            .get(at + 2..at + 2 + name.len())
            .is_some_and(|found| found.eq_ignore_ascii_case(name))
        && scan
            .get(at + 2 + name.len())
            .is_none_or(|c| c.is_ascii_whitespace() || *c == b'>' || *c == b'/')
}

fn tag_end(scan: &[u8], at: usize) -> usize {
    scan[at..]
        .iter()
        .position(|&c| c == b'>')
        .map_or(scan.len(), |p| at + p + 1)
}

const RAW_TEXT: [&[u8]; 9] = [
    b"script",
    b"style",
    b"textarea",
    b"title",
    b"xmp",
    b"iframe",
    b"noembed",
    b"noframes",
    b"noscript",
];

/// Comments, raw text, and template contents blanked to spaces of the same
/// byte length, so offsets still line up and inert tags cannot receive the
/// bootstrap.
fn blank_non_markup(html: &str) -> Vec<u8> {
    let mut scan = html.as_bytes().to_vec();
    let len = scan.len();
    let blank = |scan: &mut Vec<u8>, from: usize, to: usize| {
        for byte in &mut scan[from..to.min(len)] {
            *byte = b' ';
        }
    };
    let mut at = 0;
    while at < len {
        if scan[at] != b'<' {
            at += 1;
            continue;
        }
        if scan[at..].starts_with(b"<!--") {
            let end = find_ci(&scan, b"-->", at + 4).map_or(len, |p| p + 3);
            blank(&mut scan, at, end);
            at = end;
            continue;
        }
        if tag_at(&scan, at, b"plaintext") {
            blank(&mut scan, at, len);
            break;
        }
        if let Some(name) = RAW_TEXT.iter().find(|name| tag_at(&scan, at, name)) {
            let mut close = Vec::with_capacity(name.len() + 2);
            close.extend_from_slice(b"</");
            close.extend_from_slice(name);
            let mut search = at + 1 + name.len();
            let end = loop {
                match find_ci(&scan, &close, search) {
                    Some(p) if close_tag_at(&scan, p, name) => {
                        break tag_end(&scan, p);
                    }
                    Some(p) => search = p + 1,
                    None => break len,
                }
            };
            blank(&mut scan, at, end);
            at = end;
            continue;
        }
        if tag_at(&scan, at, b"template") {
            let mut depth = 0usize;
            let mut cursor = at;
            let end = loop {
                let Some(next) = scan[cursor..]
                    .iter()
                    .position(|&c| c == b'<')
                    .map(|offset| cursor + offset)
                else {
                    break len;
                };
                if tag_at(&scan, next, b"template") {
                    depth += 1;
                    cursor = tag_end(&scan, next);
                } else if close_tag_at(&scan, next, b"template") {
                    depth -= 1;
                    cursor = tag_end(&scan, next);
                    if depth == 0 {
                        break cursor;
                    }
                } else {
                    cursor = next + 1;
                }
            };
            blank(&mut scan, at, end);
            at = end;
            continue;
        }
        at += 1;
    }
    scan
}

pub const PAGE_RULES: &str = "Write one self-contained document with inline <style> and <script>. Local images written as absolute file paths (src=\"/abs/shot.png\", CSS url(/abs/bg.webp), or a JS string) are inlined automatically; remote http(s) URLs, such as a CDN chart library, load as-is.";

pub const LAYOUT_GUIDE: &str = "Layout: the frame is borderless on the thread's background, as wide as the reply column (736px by default) and aligned with your text, so use a fluid width with no outer padding, card, border or banner title. Give charts fixed pixel heights. Let content set the height: no 100vh or height:100% on html or body, which makes the frame grow again and again.";

pub const THEME_GUIDE: &str = "Theme: CSS variables on :root that follow the person's theme and light or dark mode live: --background (the thread's), --foreground, --muted, --muted-foreground, --card, --card-foreground, --popover, --popover-foreground, --secondary, --secondary-foreground, --border, --input, --ring, --primary, --primary-foreground, --accent, --accent-foreground, --accent-surface, --accent-surface-foreground, --destructive, --destructive-foreground, --destructive-surface, --warning, --warning-foreground, --warning-surface, --success, --success-foreground, --success-surface, --info, --info-foreground, --code-background, --code-foreground, --chart-1 … --chart-6 (series), --radius, --font-sans, --font-mono, --faint-foreground, --hover, --border-strong, --selection, --danger, --surface-treatment, and type sizes in px: --font-size-xs, --font-size-sm, --font-size, --font-size-lg, --font-size-xl, --font-size-2xl, --font-size-code, --line-height. <html> carries data-surface (glass|opaque) and data-appearance (light|dark). The base stylesheet sets color and font, body margin 0, and hides the scrollbar.";

/// Run guidance: when to answer with a page instead of only text.
pub const GUIDANCE: &str = "When a visual says more than prose, show it before your final reply: a chart, table, dashboard, diagram, simulation or 3D view as a Luau script with the zeron `show_visual` tool; an image collage or UI mockup as one self-contained HTML page with `html_render`. It appears live above your reply in the person's theme; the reply adds only what it doesn't say.";

/// What `html_render` tells the agent once the page is shown.
pub const RENDERED_MESSAGE: &str = "Shown to the reader above your reply. Don't mention or describe the page; reply with only what it doesn't already say.";

pub const CHAT_FOLDER: &str = "this chat's folder";
pub const CURRENT_FOLDER: &str = "the current folder";

pub fn workspace_page(
    root: &Path,
    home: Option<&Path>,
    folder: &str,
    path: &str,
) -> Result<String, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("{folder} {} could not be read: {e}", root.display()))?;
    if home.and_then(|home| home.canonicalize().ok()).as_deref() == Some(root.as_path()) {
        return Err(format!(
            "`path` reads from a project folder, and {folder} is your home folder. Pass `html` instead."
        ));
    }
    let file = root
        .join(path)
        .canonicalize()
        .map_err(|_| format!("`{path}` is not a file in {folder}"))?;
    if !file.starts_with(&root) {
        return Err(format!("`{path}` is outside {folder}, so it was not read"));
    }
    let html_file = file
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"));
    if !html_file || !file.is_file() {
        return Err(format!("`{path}` is not an .html or .htm file"));
    }
    let bytes = std::fs::metadata(&file).map_err(|e| e.to_string())?.len();
    if bytes > MAX_HTML_BYTES as u64 {
        return Err(format!(
            "`{path}` is {bytes} bytes; the limit is {MAX_HTML_BYTES}"
        ));
    }
    std::fs::read_to_string(&file).map_err(|e| format!("`{path}` could not be read: {e}"))
}

/// The Source view shows at most this much text; inlined image data is
/// summarized first, since a page can carry megabytes of it.
pub const MAX_SOURCE_CHARS: usize = 200_000;

/// The page as the Source view shows it: inlined image data summarized,
/// long pages cut with a pointer to Save .html for the whole file.
pub fn source_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len().min(MAX_SOURCE_CHARS));
    let mut rest = html;
    while let Some(at) = rest.find(";base64,") {
        let start = at + ";base64,".len();
        let run = rest[start..]
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
            .count();
        out.push_str(&rest[..start]);
        if run > 256 {
            out.push_str(&format!("…{} KB…", run * 3 / 4 / 1024));
        } else {
            out.push_str(&rest[start..start + run]);
        }
        rest = &rest[start + run..];
    }
    out.push_str(rest);
    if out.chars().count() > MAX_SOURCE_CHARS {
        let cut: String = out.chars().take(MAX_SOURCE_CHARS).collect();
        return format!("{cut}\n\n… Save .html for the whole page.");
    }
    out
}

/// A readable download name: the title without characters file systems reject.
pub fn page_file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let name: String = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(120)
        .collect();
    let name = name.trim();
    format!("{}.html", if name.is_empty() { "Page" } else { name })
}

/// "Open in" cards a settled reply shows at most, one per `.html` file it names.
pub const MAX_OPEN_CARDS: usize = 3;
const TITLE_SCAN_BYTES: usize = 64 * 1024;

pub fn is_html_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".html") || lower.ends_with(".htm")
}

/// An `.html` file a reply names: a link to it, or a path in inline code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlTarget {
    pub dest: String,
    pub linked: bool,
}

pub fn html_link_targets<'a>(texts: impl Iterator<Item = &'a str>) -> Vec<HtmlTarget> {
    use pulldown_cmark::{Event, Parser, Tag};
    let mut out: Vec<HtmlTarget> = Vec::new();
    for text in texts {
        for event in Parser::new(text) {
            let (dest, linked) = match &event {
                Event::Start(Tag::Link { dest_url, .. }) => (dest_url.trim(), true),
                Event::Code(code) if !code.contains(char::is_whitespace) => (code.as_ref(), false),
                _ => continue,
            };
            if dest.contains("://") && !dest.starts_with("file://") {
                continue;
            }
            let mut path = dest.split('#').next().unwrap_or(dest);
            while let Some((head, tail)) = path.rsplit_once(':')
                && !tail.is_empty()
                && tail.bytes().all(|b| b.is_ascii_digit())
            {
                path = head;
            }
            if !is_html_path(path) {
                continue;
            }
            match out.iter_mut().find(|seen| seen.dest == dest) {
                Some(seen) => seen.linked |= linked,
                None => out.push(HtmlTarget {
                    dest: dest.to_string(),
                    linked,
                }),
            }
        }
    }
    out
}

/// A page file's `<title>`, from its first 64 KB; `Err` when it is not a
/// readable file.
pub fn read_title(path: &Path) -> Result<Option<String>, ()> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| ())?;
    if !file.metadata().map_err(|_| ())?.is_file() {
        return Err(());
    }
    let mut bytes = Vec::new();
    file.take(TITLE_SCAN_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    Ok(title_from_html(&String::from_utf8_lossy(&bytes)))
}

pub fn title_from_html(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let body = open + lower[open..].find('>')? + 1;
    let close = body + lower[body..].find("</title")?;
    let title = html[body..close]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!title.is_empty()).then_some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_theme() -> PageTheme {
        let g = |v: u8, a: f32| Rgba {
            r: f32::from(v) / 255.0,
            g: f32::from(v) / 255.0,
            b: f32::from(v) / 255.0,
            a,
        };
        PageTheme {
            dark: true,
            glass: false,
            bg: g(16, 1.0),
            text: g(240, 1.0),
            text_faint: g(120, 1.0),
            text_muted: g(170, 1.0),
            element_hover: g(40, 1.0),
            hover: g(255, 0.08),
            selection: Rgba::hex("#ec83b759").unwrap(),
            surface_card: g(24, 1.0),
            surface_overlay: g(30, 1.0),
            surface_raised: g(28, 1.0),
            border: g(255, 0.1),
            border_strong: g(255, 0.2),
            input_bg: g(20, 1.0),
            accent: Rgba::hex("#ec83b7").unwrap(),
            solid: g(240, 1.0),
            on_solid: g(16, 1.0),
            on_accent: g(0, 1.0),
            accent_wash: Rgba::hex("#ec83b729").unwrap(),
            accent_strong: Rgba::hex("#f4a8cf").unwrap(),
            danger: Rgba::hex("#ff5f57").unwrap(),
            danger_strong: Rgba::hex("#ff8a84").unwrap(),
            danger_muted: Rgba::hex("#ff5f571f").unwrap(),
            warning: Rgba::hex("#febc2e").unwrap(),
            warning_muted: Rgba::hex("#febc2e1f").unwrap(),
            success: Rgba::hex("#28c840").unwrap(),
            success_muted: Rgba::hex("#28c8401f").unwrap(),
            code_wash: g(255, 0.06),
            code_text: g(230, 1.0),
            font_sans: "Inter".into(),
            font_mono: "".into(),
            code_font_size: 12.5,
            text_size: 14.0,
            line_height: 22.0,
        }
    }

    // gpui's `theme_css` (crates/ui/src/html_render.rs before U23), worked by hand for `fixed_theme`.
    const GPUI_CSS: &str = ":root{color-scheme:dark;--background:rgba(16,16,16,1);--foreground:rgba(240,240,240,1);--faint-foreground:rgba(120,120,120,1);--muted:rgba(40,40,40,1);--muted-foreground:rgba(170,170,170,1);--hover:rgba(255,255,255,0.08);--selection:rgba(236,131,183,0.349);--card:rgba(24,24,24,1);--card-foreground:rgba(240,240,240,1);--popover:rgba(30,30,30,1);--popover-foreground:rgba(240,240,240,1);--secondary:rgba(28,28,28,1);--secondary-foreground:rgba(240,240,240,1);--border:rgba(255,255,255,0.1);--border-strong:rgba(255,255,255,0.2);--input:rgba(20,20,20,1);--ring:rgba(236,131,183,1);--primary:rgba(240,240,240,1);--primary-foreground:rgba(16,16,16,1);--accent:rgba(236,131,183,1);--accent-foreground:rgba(0,0,0,1);--accent-surface:rgba(236,131,183,0.161);--accent-surface-foreground:rgba(244,168,207,1);--destructive:rgba(255,95,87,1);--destructive-foreground:rgba(255,138,132,1);--destructive-surface:rgba(255,95,87,0.122);--danger:rgba(255,95,87,1);--warning:rgba(254,188,46,1);--warning-foreground:rgba(254,188,46,1);--warning-surface:rgba(254,188,46,0.122);--success:rgba(40,200,64,1);--success-foreground:rgba(40,200,64,1);--success-surface:rgba(40,200,64,0.122);--info:#3b82f6;--info-foreground:#60a5fa;--code-background:rgba(255,255,255,0.06);--code-foreground:rgba(230,230,230,1);--chart-1:rgba(236,131,183,1);--chart-2:#2dd4bf;--chart-3:#fbbf24;--chart-4:#c084fc;--chart-5:#fb7185;--chart-6:#a3e635;--radius:0.625rem;--font-sans:\"Inter\", -apple-system, BlinkMacSystemFont, system-ui, sans-serif;--font-mono:\"SF Mono\", Menlo, monospace;--font-size-xs:11px;--font-size-sm:12px;--font-size:14px;--font-size-lg:15px;--font-size-xl:16px;--font-size-2xl:19px;--line-height:22px;--font-size-code:12.5px;--surface-treatment:opaque;}html{background:transparent;overscroll-behavior:none;color:var(--foreground);font-family:var(--font-sans);font-size:14px;line-height:1.5;-webkit-font-smoothing:antialiased;-webkit-text-size-adjust:100%;scrollbar-width:none}html::-webkit-scrollbar{display:none}body{margin:0}code,kbd,pre,samp{font-family:var(--font-mono)}";

    #[test]
    fn the_theme_css_is_gpuis_for_the_same_colors() {
        assert_eq!(theme_css(&fixed_theme(), 16.0, true), GPUI_CSS);
        let preview = theme_css(&fixed_theme(), 16.0, false);
        assert!(preview.contains("}html{background:var(--background);color:"));
        let large = theme_css(&fixed_theme(), 20.0, true);
        assert!(large.contains("--font-size:17.5px;") && large.contains("--font-size-xs:13.75px;"));
        assert!(large.contains("--font-size-code:12.5px;"));
        let mut light = fixed_theme();
        light.dark = false;
        light.glass = true;
        let css = theme_css(&light, 16.0, true);
        assert!(css.starts_with(":root{color-scheme:light;"));
        assert!(
            css.contains("--info-foreground:#1d4ed8;--code") && css.contains("--chart-2:#0d9488;")
        );
        assert!(css.contains("--surface-treatment:glass;"));
    }

    #[test]
    fn the_bootstrap_is_gpuis_for_the_same_page() {
        let theme = fixed_theme();
        let attributes = "(function(){var d=document.documentElement;d.setAttribute(\"data-surface\",\"opaque\");d.setAttribute(\"data-appearance\",\"dark\");})();";
        assert_eq!(attributes_script(&theme), attributes);
        let page = "<html><head><title>x</title></head><body>hi</body></html>";
        let inline = themed_page(page, &theme, 16.0, true, "");
        assert_eq!(
            inline,
            format!(
                "<html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><style id=\"zeron-theme\">{GPUI_CSS}</style><script>{attributes}{SIZE_SCRIPT}{EDGE_SCRIPT}{ACTIVITY_SCRIPT}</script><title>x</title></head><body>hi</body></html>"
            )
        );
        let extra = themed_page(page, &theme, 16.0, true, "X();");
        assert!(extra.contains(&format!("{ACTIVITY_SCRIPT}X();</script>")));
        let preview = themed_page(page, &theme, 16.0, false, "");
        assert!(preview.contains(&format!("<script>{attributes}</script>")));
        let retheme = retheme_script(&theme, 16.0, true);
        assert!(retheme.starts_with("(function(){var s=document.getElementById('zeron-theme');if(s)s.textContent=\":root{color-scheme:dark;"));
        assert!(retheme.ends_with(attributes));
    }

    #[test]
    fn colors_read_from_hex_with_or_without_alpha() {
        assert_eq!(
            css_color(Rgba::hex("#c0347a").unwrap()),
            "rgba(192,52,122,1)"
        );
        assert_eq!(
            css_color(Rgba::hex("#c0347a80").unwrap()),
            "rgba(192,52,122,0.502)"
        );
        for bad in ["c0347a", "#c034", "#c0347g", "#ếế"] {
            assert_eq!(Rgba::hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_render_takes_exactly_one_of_html_path_or_url() {
        assert_eq!(
            render_input(Some("<p>x</p>"), None, None),
            Ok(RenderInput::Html("<p>x</p>"))
        );
        assert_eq!(
            render_input(None, Some(" out/tray.html "), Some("")),
            Ok(RenderInput::Path("out/tray.html"))
        );
        assert_eq!(
            render_input(None, None, Some("http://localhost:5391/")),
            Ok(RenderInput::Url("http://localhost:5391/"))
        );
        for (html, path, url) in [
            (Some("<p>x</p>"), Some("a.html"), None),
            (None, Some("a.html"), Some("http://localhost:5391/")),
            (Some("<p>x</p>"), None, Some("http://localhost:5391/")),
        ] {
            assert!(
                render_input(html, path, url)
                    .unwrap_err()
                    .contains("not both")
            );
        }
        let none = render_input(None, Some(" "), None).unwrap_err();
        assert!(none.contains("give `html`") && none.contains("`path`"));
        assert!(render_input(None, None, Some("https://example.com/")).is_err());
    }

    #[test]
    fn a_stale_engines_refusal_names_the_skew() {
        let old = "PublishHtmlRender: bad params: missing field `html`";
        let url = RenderInput::Url("http://localhost:5391/");
        let path = RenderInput::Path("a.html");
        let said = stale_engine_reply(old, url).unwrap();
        assert!(said.contains("older than the app") && said.contains("`url`"));
        assert!(said.contains("Restart the engine") && !said.contains("missing field"));
        assert!(stale_engine_reply(old, path).unwrap().contains("`path`"));
        let before_path = "PublishHtmlRender: bad params: give `html` (a finished page) or `url` (a local dev server you are running)";
        assert!(stale_engine_reply(before_path, path).is_some());
        assert_eq!(stale_engine_reply(before_path, url), None);
        assert_eq!(stale_engine_reply(old, RenderInput::Html("<p>x</p>")), None);
        assert_eq!(
            stale_engine_reply(
                "PublishHtmlRender: `a.html` is not a file in this chat's folder",
                path
            ),
            None
        );
    }

    fn pick(group: Option<&str>, code: &str, label: &str) -> Pick {
        Pick {
            group: group.map(str::to_owned),
            code: code.into(),
            label: label.into(),
        }
    }

    #[test]
    fn picks_are_sent_one_line_a_group_with_what_was_left() {
        let picks = [
            pick(Some("A"), "A2", "Nổi sát đỉnh"),
            pick(Some("B"), "B2", "Không viền"),
        ];
        let missing = [PickGroup {
            code: "C".into(),
            name: "Rail".into(),
        }];
        let sent = with_picks(
            "+ header của C2",
            &[("Khung app".into(), picks.to_vec(), missing.to_vec())],
        );
        assert_eq!(
            sent,
            "+ header của C2\n\nĐã chọn trong \"Khung app\":\nA = A2 (Nổi sát đỉnh)\nB = B2 (Không viền)\nChưa chọn: C (Rail)"
        );
        let free = with_picks(
            "",
            &[(
                "Ảnh".into(),
                vec![pick(None, "p1", "Sea"), pick(None, "p3", "Hill")],
                vec![],
            )],
        );
        assert_eq!(free, "Đã chọn trong \"Ảnh\":\n- Sea\n- Hill");
        let (words, cards) = extract_picks(&sent);
        assert_eq!(words, "+ header của C2");
        assert_eq!(cards, vec![("Khung app".into(), picks.to_vec())]);
        let (words, cards) = extract_picks(&free);
        assert_eq!(words, "");
        assert_eq!(
            cards[0].1,
            vec![pick(None, "Sea", "Sea"), pick(None, "Hill", "Hill")]
        );
        let both = with_picks(
            "two",
            &[
                ("A".into(), vec![pick(Some("A"), "A1", "x")], vec![]),
                ("B".into(), vec![pick(None, "f", "f")], vec![]),
            ],
        );
        let (words, cards) = extract_picks(&both);
        assert_eq!((words.as_str(), cards.len()), ("two", 2));
        let spaced = with_picks(
            "",
            &[(
                "Two\nlines".into(),
                vec![pick(Some("A"), "A1", "a\n  b")],
                vec![],
            )],
        );
        assert_eq!(spaced, "Đã chọn trong \"Two lines\":\nA = A1 (a b)");
        for typed in [
            "Đã chọn trong \"x\":\nnot a pick",
            "Đã chọn trong \"x\":\n- y\n\nand then more words",
            "Đã chọn trong \"x\":\n- y\nand a line after",
            "Đã chọn trong \"x\":\nChưa chọn: C (Rail)",
            "Đã chọn trong \"x\":\nChưa chọn: C (Rail)\n- y",
            "Đã chọn trong \"x\":\nA B = A1 (x)",
        ] {
            assert_eq!(
                extract_picks(typed),
                (typed.to_owned(), vec![]),
                "typed, not sent: {typed}"
            );
        }
    }

    #[test]
    fn a_replys_html_files_are_its_links_and_code_paths() {
        let text = "Made [demo.html](demo.html), [web](https://x.dev/a.html), `c.html`, `open e.html`, [line](b.html:12:3)";
        let targets = html_link_targets([text].into_iter());
        assert_eq!(
            targets
                .iter()
                .map(|t| (t.dest.as_str(), t.linked))
                .collect::<Vec<_>>(),
            vec![
                ("demo.html", true),
                ("c.html", false),
                ("b.html:12:3", true)
            ]
        );
        assert_eq!(
            title_from_html("<TITLE lang=vi>\n Demo  đơn giản </TITLE>"),
            Some("Demo đơn giản".into())
        );
    }

    #[test]
    fn the_source_view_summarizes_inlined_images_and_cuts_long_pages() {
        let image = "A".repeat(4096);
        let html = format!(
            r#"<img src="data:image/png;base64,{image}"><img src="data:image/gif;base64,R0lG">"#
        );
        let shown = source_text(&html);
        assert!(shown.contains("data:image/png;base64,…3 KB…"));
        assert!(shown.contains("base64,R0lG"));
        assert!(!shown.contains(&image));
        let long = "x".repeat(MAX_SOURCE_CHARS + 10);
        assert!(source_text(&long).ends_with("Save .html for the whole page."));
    }

    #[test]
    fn download_names_drop_characters_file_systems_reject() {
        assert_eq!(page_file_name("Q3: tokens / turn?"), "Q3 tokens turn.html");
        assert_eq!(page_file_name("  "), "Page.html");
        assert_eq!(page_file_name(&"a".repeat(300)).len(), 125);
    }

    #[test]
    fn heights_clamp_to_the_frame_range() {
        assert_eq!(clamp_height(10.0), MIN_HEIGHT);
        assert_eq!(clamp_height(640.4), 640);
        assert_eq!(clamp_height(9_999.0), MAX_HEIGHT);
        assert_eq!(clamp_height(f64::NAN), MIN_HEIGHT);
    }

    #[test]
    fn a_ref_round_trips_camel_free_json() {
        let r = HtmlRenderRef {
            path: "html/c1/ab.html".into(),
            title: "Chart".into(),
            height: 420,
            url: None,
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"path":"html/c1/ab.html","title":"Chart","height":420})
        );
        assert_eq!(serde_json::from_value::<HtmlRenderRef>(json).unwrap(), r);
    }

    #[test]
    fn templates_close_and_never_swallow_the_rest_of_the_page() {
        let page = "<body><template><p>a</p><template>b</template></template><head>c";
        let scan = String::from_utf8(blank_non_markup(page)).unwrap();
        assert!(scan.starts_with("<body>  "));
        assert!(scan.ends_with("<head>c"), "{scan}");
        let many = "<template>x</template>".repeat(20_000) + "<p>end</p>";
        let started = std::time::Instant::now();
        let scan = String::from_utf8(blank_non_markup(&many)).unwrap();
        assert!(scan.ends_with("<p>end</p>"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn raw_text_ends_at_its_own_closing_tag() {
        let page = "<script>var a = '</scripts>';</script><p>x</p><style>p{}</STYLE ><p>y</p>";
        let scan = String::from_utf8(blank_non_markup(page)).unwrap();
        assert!(scan.contains("<p>x</p>"));
        assert!(scan.ends_with("<p>y</p>"));
        assert!(!scan.contains("var a"));
    }

    #[test]
    fn live_urls_are_loopback_dev_servers_only() {
        for url in [
            "http://localhost:5391/?capture=zeron-tray",
            "https://LOCALHOST:8443",
            "http://127.0.0.1:3000/app/#x",
            "http://[::1]:5173/",
            "http://localhost",
        ] {
            assert!(live_url(url).is_ok(), "{url}");
        }
        let live = live_url(" http://localhost:5391/?capture=zeron-tray ").unwrap();
        assert_eq!(live.url, "http://localhost:5391/?capture=zeron-tray");
        assert_eq!(
            (live.server, live.label),
            ("localhost:5391", "localhost:5391")
        );
        assert_eq!((live.host, live.port), ("localhost", 5391));
        let v6 = live_url("http://[::1]:5173/app/?q").unwrap();
        assert_eq!(
            (v6.host, v6.port, v6.label),
            ("::1", 5173, "[::1]:5173/app")
        );
        assert_eq!(live_url("https://127.0.0.1/").unwrap().port, 443);
        for url in [
            "https://example.com/",
            "http://localhost.example.com:5391/",
            "http://192.168.1.4:5391/",
            "http://user@localhost:5391/",
            "http://localhost:5391@evil.test/",
            "http://localhost\\@evil.test/",
            "http://[127.0.0.1]:80/",
            "http://localhost:0/",
            "http://localhost:99999/",
            "http://localhost:/",
            "file:///etc/passwd",
            "ftp://localhost/",
            "localhost:5391",
            "javascript:alert(1)",
            "http://local host/",
            "http://localhost:+5173/",
            "http://localhost:5173/caf\u{e9}",
            "http://localhost:5173/?q={\"a\":1}",
        ] {
            assert!(live_url(url).is_err(), "{url}");
        }
        assert!(live_url(&format!("http://localhost/{}", "a".repeat(MAX_URL_BYTES))).is_err());
    }

    #[test]
    fn a_render_shows_exactly_one_of_html_or_url() {
        assert_eq!(
            render_source(Some("<p>x</p>"), None),
            Ok(RenderSource::Html("<p>x</p>"))
        );
        assert_eq!(
            render_source(None, Some("http://localhost:5391/")),
            Ok(RenderSource::Url("http://localhost:5391/"))
        );
        assert!(render_source(Some("<p>x</p>"), Some("http://localhost:5391/")).is_err());
        assert!(render_source(None, None).is_err());
        assert_eq!(
            render_source(Some("<p>x</p>"), Some("")),
            Ok(RenderSource::Html("<p>x</p>"))
        );
        assert_eq!(
            render_source(Some(" "), Some("http://localhost:5391/")),
            Ok(RenderSource::Url("http://localhost:5391/"))
        );
        assert!(render_source(None, Some("https://example.com/")).is_err());
    }

    #[test]
    fn a_live_ref_keeps_its_url_and_old_refs_read_without_one() {
        let live = HtmlRenderRef {
            path: String::new(),
            title: "Tray".into(),
            height: 600,
            url: Some("http://localhost:5391/".into()),
        };
        let json = serde_json::to_value(&live).unwrap();
        assert_eq!(json["url"], "http://localhost:5391/");
        assert_eq!(serde_json::from_value::<HtmlRenderRef>(json).unwrap(), live);
        let old = serde_json::json!({"path":"p.html","title":"T","height":80});
        assert_eq!(
            serde_json::from_value::<HtmlRenderRef>(old).unwrap().url,
            None
        );
    }

    #[test]
    fn frames_fit_the_page_unless_the_agent_asked_for_less() {
        assert_eq!(frame_height(600.0, None), 600.0);
        assert_eq!(frame_height(600.0, Some(420.0)), 420.0);
        assert_eq!(frame_height(600.0, Some(900.0)), 600.0);
        assert_eq!(frame_height(600.0, Some(640.0)), 640.0);
        assert_eq!(frame_height(1200.0, Some(900.0)), 900.0);
        assert_eq!(frame_height(2400.0, Some(5000.0)), MAX_HEIGHT as f32);
        assert_eq!(frame_height(600.0, Some(10.0)), MIN_HEIGHT as f32);
        assert!(scrolls_inside(600.0, Some(900.0)));
        assert!(!scrolls_inside(420.0, Some(420.0)));
    }
}
