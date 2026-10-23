//! Agent-published HTML pages (`html_render` / `html_preview` thread tools):
//! the limits and agent guidance shared by the MCP server, the engine and the
//! desktop UI. Ported from t3code `packages/shared/src/htmlRender.ts`.

use serde::{Deserialize, Serialize};

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
    pub html: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub appearance: Option<PreviewAppearance>,
}

/// A page the engine asks the desktop app to lay out and screenshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HtmlPreviewRequest {
    pub id: String,
    pub html: String,
    pub width: u32,
    pub appearance: PreviewAppearance,
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

pub const LAYOUT_GUIDE: &str = "The frame is borderless on the thread's background, as wide as the reply column (736px by default, wider if the reader widens chat), and its left edge lines up with your reply text. Use a fluid width with no horizontal padding on the outermost element, and no outer card, border, or banner title: the page is part of your reply. Give charts fixed pixel heights rather than heights that scale with width. Let content set the page's height. Avoid viewport-based heights such as 100vh or height:100% on html or body; the frame grows to fit the page, so they can make it grow again and again.";

pub const THEME_GUIDE: &str = "Zeron injects its active theme as CSS custom properties on :root, and they follow the user's theme and light/dark mode live: --background (the thread's background), --foreground, --muted, --muted-foreground, --card, --card-foreground, --popover, --popover-foreground, --secondary, --secondary-foreground, --border, --input, --ring, --primary, --primary-foreground (solid buttons), --accent, --accent-foreground (brand accent), --accent-surface, --accent-surface-foreground, --destructive, --destructive-foreground, --destructive-surface, --warning, --warning-foreground, --warning-surface, --success, --success-foreground, --success-surface, --info, --info-foreground, --code-background, --code-foreground, --chart-1 … --chart-6 (categorical series for charts), --radius, --font-sans, --font-mono, --faint-foreground, --hover (hover wash), --border-strong, --selection, --danger (same as --destructive), --surface-treatment (glass or opaque). Type sizes are CSS px at the reader's interface size, matching the thread's own text: --font-size-xs, --font-size-sm, --font-size (body), --font-size-lg, --font-size-xl, --font-size-2xl (headings), --font-size-code, --line-height (body). <html> carries data-surface (glass|opaque) and data-appearance (light|dark). The base stylesheet sets html color/font from these, body margin to 0, and hides the page's scrollbar; your own CSS overrides it.";

/// Run guidance: when to answer with a page instead of only text.
pub const GUIDANCE: &str = "When a visual answer would be clearer than prose, show it before your final reply: a chart, table, dashboard, diagram, simulation, or 3D view as a Luau script with the zeron show_visual tool; an image collage or UI mockup (such as several treatments of a design side by side) as one self-contained HTML page with the zeron html_render tool. It appears live in the thread above your reply, in the user's theme; keep the reply to what it does not already say.";

/// What `html_render` tells the agent once the page is shown.
pub const RENDERED_MESSAGE: &str = "Shown to the reader above your reply. Don't mention or describe the page; reply with only what it doesn't already say.";

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

#[cfg(test)]
mod tests {
    use super::*;

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
