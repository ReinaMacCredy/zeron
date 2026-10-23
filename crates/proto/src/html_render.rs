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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishHtmlRenderParams {
    pub chat_id: String,
    pub run_secret: String,
    pub html: String,
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
    pub width: Option<u32>,
    #[serde(default)]
    pub appearance: Option<PreviewAppearance>,
    #[serde(flatten)]
    pub inspect: PreviewInspect,
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
    /// ReadAttachmentChunk, like a generated image).
    pub path: String,
    pub title: String,
    pub height: u32,
}

pub fn clamp_height(height: f64) -> u32 {
    if height.is_finite() {
        (height.round() as i64).clamp(MIN_HEIGHT as i64, MAX_HEIGHT as i64) as u32
    } else {
        MIN_HEIGHT
    }
}

pub const PAGE_RULES: &str = "Write one self-contained document with inline <style> and <script>. Local images written as absolute file paths (src=\"/abs/shot.png\", CSS url(/abs/bg.webp), or a JS string) are inlined automatically; remote http(s) URLs, such as a CDN chart library, load as-is.";

pub const LAYOUT_GUIDE: &str = "The frame is borderless on the thread's background, as wide as the reply column (736px by default, wider if the reader widens chat), and its left edge lines up with your reply text. Use a fluid width with no horizontal padding on the outermost element, and no outer card, border, or banner title: the page is part of your reply. Give charts fixed pixel heights rather than heights that scale with width. Let content set the page's height. Avoid viewport-based heights such as 100vh or height:100% on html or body; the frame grows to fit the page, so they can make it grow again and again.";

pub const THEME_GUIDE: &str = "Zeron injects its active theme as CSS custom properties on :root, and they follow the user's theme and light/dark mode live: --background (the thread's background), --foreground, --muted, --muted-foreground, --card, --card-foreground, --popover, --popover-foreground, --secondary, --secondary-foreground, --border, --input, --ring, --primary, --primary-foreground (solid buttons), --accent, --accent-foreground (brand accent), --accent-surface, --accent-surface-foreground, --destructive, --destructive-foreground, --destructive-surface, --warning, --warning-foreground, --warning-surface, --success, --success-foreground, --success-surface, --info, --info-foreground, --code-background, --code-foreground, --chart-1 … --chart-6 (categorical series for charts), --radius, --font-sans, --font-mono, --faint-foreground, --hover (hover wash), --border-strong, --selection, --danger (same as --destructive), --surface-treatment (glass or opaque). Type sizes are CSS px at the reader's interface size, matching the thread's own text: --font-size-xs, --font-size-sm, --font-size (body), --font-size-lg, --font-size-xl, --font-size-2xl (headings), --font-size-code, --line-height (body). <html> carries data-surface (glass|opaque) and data-appearance (light|dark). The base stylesheet sets html color/font from these, body margin to 0, and hides the page's scrollbar; your own CSS overrides it.";

/// Run guidance: when to answer with a page instead of only text.
pub const GUIDANCE: &str = "When a visual answer would be clearer than prose, show it before your final reply: a chart, table, dashboard, diagram, simulation, or 3D view as a Luau script with the zeron show_visual tool; an image collage or UI mockup (such as several treatments of a design side by side) as one self-contained HTML page with the zeron html_render tool. It appears live in the thread above your reply, in the user's theme; keep the reply to what it does not already say.";

/// What `html_render` tells the agent once the page is shown.
pub const RENDERED_MESSAGE: &str = "Shown to the reader above your reply. Don't mention or describe the page; reply with only what it doesn't already say.";

pub fn workspace_page(root: &Path, home: Option<&Path>, path: &str) -> Result<String, String> {
    let root = root.canonicalize().map_err(|e| {
        format!(
            "this chat's folder {} could not be read: {e}",
            root.display()
        )
    })?;
    if home.and_then(|home| home.canonicalize().ok()).as_deref() == Some(root.as_path()) {
        return Err("`path` reads from a project folder, and this chat runs in your home folder. Pass `html` instead.".into());
    }
    let file = root
        .join(path)
        .canonicalize()
        .map_err(|_| format!("`{path}` is not a file in this chat's folder"))?;
    if !file.starts_with(&root) {
        return Err(format!(
            "`{path}` is outside this chat's folder, so it was not read"
        ));
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

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"path":"html/c1/ab.html","title":"Chart","height":420})
        );
        assert_eq!(serde_json::from_value::<HtmlRenderRef>(json).unwrap(), r);
    }
}
