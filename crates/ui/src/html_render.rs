//! Agent-published HTML pages ("HTML renders", the `html_render` thread
//! tool): the theme a page is handed as CSS variables, the bootstrap injected
//! at the start of its `<head>`, and the frame height rules. Ported from t3code
//! `packages/shared/src/htmlRender.ts` so pages written for either app agree.

use crate::theme::Theme;
use gpui::Hsla;

pub const MIN_HEIGHT: f32 = zeron_proto::html_render::MIN_HEIGHT as f32;
pub const MAX_HEIGHT: f32 = zeron_proto::html_render::MAX_HEIGHT as f32;

pub fn clamp_height(height: f32) -> f32 {
    if height.is_finite() {
        height.round().clamp(MIN_HEIGHT, MAX_HEIGHT)
    } else {
        MIN_HEIGHT
    }
}

/// The frame fits the page so it never scrolls inside the thread and takes
/// the reader's scroll. The agent's height caps it only when it is below the
/// page's own height (the agent asked for a scrolling frame).
pub fn frame_height(agent_height: f32, content_height: Option<f32>) -> f32 {
    clamp_height(match content_height {
        Some(content) if !overflows(agent_height, content) => content,
        Some(_) | None => agent_height,
    })
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

fn css_color(color: Hsla) -> String {
    let rgba = gpui::Rgba::from(color);
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "rgba({},{},{},{})",
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b),
        (rgba.a.clamp(0.0, 1.0) * 1000.0).round() / 1000.0
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

// The transcript's sizes at 16px: meta, label, prose, then `heading_metrics` h3/h2/h1.
const TYPE_SCALE: [(&str, f32); 7] = [
    ("--font-size-xs", 11.0),
    ("--font-size-sm", 12.0),
    ("--font-size", crate::markdown::render::MD_TEXT_SIZE),
    ("--font-size-lg", 15.0),
    ("--font-size-xl", 16.0),
    ("--font-size-2xl", 19.0),
    ("--line-height", crate::markdown::render::MD_LINE_HEIGHT),
];

fn css_px(value: f32) -> String {
    format!("{}px", (value * 100.0).round() / 100.0)
}

/// The variables a page styles against, named like t3's so one page works in
/// both apps. `--accent` is the brand color, not a hover surface.
pub fn theme_variables(theme: &Theme, font_size: f32) -> Vec<(&'static str, String)> {
    let dark = theme.appearance.is_dark();
    let (info, info_foreground, chart) = fixed(dark);
    let c = css_color;
    let mut vars = vec![
        ("--background", c(theme.bg)),
        ("--foreground", c(theme.text)),
        ("--faint-foreground", c(theme.text_faint)),
        ("--muted", c(theme.element_hover)),
        ("--muted-foreground", c(theme.text_muted)),
        ("--hover", c(theme.wash(0.08))),
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
    let scale = font_size / 16.0;
    vars.extend(
        TYPE_SCALE
            .into_iter()
            .map(|(name, size)| (name, css_px(size * scale))),
    );
    vars.push(("--font-size-code", css_px(theme.code_font_size)));
    vars.push(("--surface-treatment", surface(theme).into()));
    vars
}

fn surface(theme: &Theme) -> &'static str {
    if theme.is_glass() { "glass" } else { "opaque" }
}

fn appearance(theme: &Theme) -> &'static str {
    if theme.appearance.is_dark() {
        "dark"
    } else {
        "light"
    }
}

/// The theme stylesheet the bootstrap's `<style id="zeron-theme">` holds.
/// Inline cards sit on the thread itself, so their page background is clear;
/// a preview screenshot paints the thread's background.
pub fn theme_css(theme: &Theme, font_size: f32, inline: bool) -> String {
    let mut css = format!(":root{{color-scheme:{};", appearance(theme));
    for (name, value) in theme_variables(theme, font_size) {
        css.push_str(name);
        css.push(':');
        css.push_str(&value);
        css.push(';');
    }
    css.push('}');
    css.push_str(if inline {
        "html{background:transparent;"
    } else {
        "html{background:var(--background);"
    });
    css.push_str("color:var(--foreground);font-family:var(--font-sans);font-size:14px;line-height:1.5;-webkit-font-smoothing:antialiased;-webkit-text-size-adjust:100%;scrollbar-width:none}html::-webkit-scrollbar{display:none}body{margin:0}code,kbd,pre,samp{font-family:var(--font-mono)}");
    css
}

/// Reports the page's content height to the host as it changes (t3's
/// measure: the document's scroll height when it overflows, else its box).
const SIZE_SCRIPT: &str = r#"(function(){var h,z=function(){var r=document.documentElement,v=Math.ceil(r.scrollHeight>r.clientHeight?r.scrollHeight:r.getBoundingClientRect().height);if(v===h||!window.ipc)return;h=v;window.ipc.postMessage(JSON.stringify({height:v}));};if(window.ResizeObserver){var o=new ResizeObserver(z);o.observe(document.documentElement);document.addEventListener("DOMContentLoaded",function(){if(document.body)o.observe(document.body);});}document.addEventListener("DOMContentLoaded",z);window.addEventListener("load",z);})();"#;

// Snapshots are taken only while the page reports a change, at most every 250ms.
const ACTIVITY_SCRIPT: &str = r#"(function(){var t=0;function a(){var n=Date.now();if(n-t<250||!window.ipc)return;t=n;window.ipc.postMessage(JSON.stringify({active:1}));}function w(f){return typeof f==="function"?function(){a();return f.apply(this,arguments);}:f;}var r=window.requestAnimationFrame;window.requestAnimationFrame=function(f){a();return r.call(window,f);};var st=window.setTimeout,si=window.setInterval;window.setTimeout=function(f){var g=[].slice.call(arguments);g[0]=w(f);return st.apply(window,g);};window.setInterval=function(f){var g=[].slice.call(arguments);g[0]=w(f);return si.apply(window,g);};new MutationObserver(a).observe(document.documentElement,{subtree:true,childList:true,attributes:true,characterData:true});["transitionrun","transitionend","animationstart","animationiteration","animationend","input","change","scroll","load"].forEach(function(e){window.addEventListener(e,a,true);});})();"#;

// Snapshots omit WebKit's selection highlight, so the page draws its own.
const SELECTION_SCRIPT: &str = r#"(function(){var l=null;function d(){var s=getSelection();if(l){l.remove();l=null;}if(!s||s.isCollapsed||!s.rangeCount)return;var c=getComputedStyle(document.documentElement).getPropertyValue("--selection").trim()||"rgba(0,122,255,.3)";l=document.createElement("div");l.setAttribute("aria-hidden","true");l.style.cssText="position:absolute;left:0;top:0;width:0;height:0;pointer-events:none;z-index:2147483647";for(var i=0;i<s.rangeCount;i++){var rs=s.getRangeAt(i).getClientRects();for(var j=0;j<rs.length;j++){var r=rs[j];if(!r.width||!r.height)continue;var b=document.createElement("div");b.style.cssText="position:absolute;border-radius:2px;background:"+c+";left:"+(r.left+scrollX)+"px;top:"+(r.top+scrollY)+"px;width:"+r.width+"px;height:"+r.height+"px";l.appendChild(b);}}document.documentElement.appendChild(l);}document.addEventListener("selectionchange",d);window.addEventListener("resize",d);})();"#;

fn attributes_script(theme: &Theme) -> String {
    format!(
        "(function(){{var d=document.documentElement;d.setAttribute(\"data-surface\",\"{}\");d.setAttribute(\"data-appearance\",\"{}\");}})();",
        surface(theme),
        appearance(theme)
    )
}

/// JavaScript that swaps a mounted page's theme without reloading it.
pub fn retheme_script(theme: &Theme, font_size: f32, inline: bool) -> String {
    let css = serde_json::to_string(&theme_css(theme, font_size, inline))
        .unwrap_or_else(|_| "\"\"".into());
    format!(
        "(function(){{var s=document.getElementById('zeron-theme');if(s)s.textContent={css};}})();{}",
        attributes_script(theme)
    )
}

fn bootstrap_markup(scan: &str, theme: &Theme, font_size: f32, inline: bool) -> String {
    let mut markup = String::new();
    let head = &scan[..scan.floor_char_boundary(4096)];
    if !(contains_ci(head, "<meta") && contains_ci(head, "charset")) {
        markup.push_str("<meta charset=\"utf-8\">");
    }
    if !contains_ci(scan, "name=\"viewport\"") && !contains_ci(scan, "name=viewport") {
        markup.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    }
    markup.push_str("<style id=\"zeron-theme\">");
    markup.push_str(&theme_css(theme, font_size, inline).replace("</", "<\\/"));
    markup.push_str("</style><script>");
    markup.push_str(&attributes_script(theme));
    if inline {
        markup.push_str(SIZE_SCRIPT);
        markup.push_str(ACTIVITY_SCRIPT);
        markup.push_str(SELECTION_SCRIPT);
    }
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

/// Inserts the theme bootstrap at the start of the document head, so a
/// page's own styles and scripts come after it and win.
pub fn inject_bootstrap(html: &str, theme: &Theme, font_size: f32, inline: bool) -> String {
    let scan = blank_non_markup(html);
    let scan_text = String::from_utf8_lossy(&scan);
    let markup = bootstrap_markup(&scan_text, theme, font_size, inline);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        Theme::dark()
    }

    #[test]
    fn bootstrap_lands_at_the_start_of_the_real_head() {
        let page = "<!doctype html><html><head><title>x</title><style>body{}</style></head><body>hi</body></html>";
        let out = inject_bootstrap(page, &theme(), 16.0, true);
        let head = out.find("<head>").unwrap() + "<head>".len();
        assert!(out[head..].starts_with("<meta charset=\"utf-8\">"));
        assert!(out.find("zeron-theme").unwrap() < out.find("<title>").unwrap());
        assert!(out.ends_with("<body>hi</body></html>"));
    }

    #[test]
    fn inert_heads_never_receive_the_bootstrap() {
        let page = "<!-- <head> --><script>var s='<head>'</script><template><head></head></template><p>x</p>";
        let out = inject_bootstrap(page, &theme(), 16.0, false);
        assert!(out.starts_with("<!doctype html><head><meta"));
        assert!(out.ends_with(page));
    }

    #[test]
    fn a_page_without_head_gets_one_after_html_or_doctype() {
        let out = inject_bootstrap("<html lang=en><body>x</body></html>", &theme(), 16.0, false);
        assert!(out.starts_with("<html lang=en><head><meta charset"));
        let out = inject_bootstrap("  <!DOCTYPE html><body>x</body>", &theme(), 16.0, false);
        assert!(out.starts_with("  <!DOCTYPE html><head>"));
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
    fn existing_charset_and_viewport_are_kept() {
        let page = "<head><meta charset=utf-8><meta name=\"viewport\" content=\"x\"></head>";
        let out = inject_bootstrap(page, &theme(), 16.0, false);
        assert_eq!(out.matches("charset").count(), 1);
        assert_eq!(out.matches("viewport").count(), 1);
    }

    #[test]
    fn multibyte_text_at_the_scan_cutoffs_never_panics() {
        let page = format!("<p>{}ở</p>{}", "a".repeat(4092), "b".repeat(100));
        assert!(!page.is_char_boundary(4096));
        assert!(inject_bootstrap(&page, &theme(), 16.0, true).ends_with(&page));
        let page = "<!doctypở html><p>x</p>";
        assert!(inject_bootstrap(page, &theme(), 16.0, false).ends_with(page));
    }

    #[test]
    fn only_inline_cards_report_their_height_and_clear_their_background() {
        let inline = inject_bootstrap("<p>x</p>", &theme(), 16.0, true);
        let preview = inject_bootstrap("<p>x</p>", &theme(), 16.0, false);
        assert!(inline.contains("window.ipc.postMessage"));
        assert!(inline.contains("{active:1}") && inline.contains("selectionchange"));
        assert!(inline.contains("html{background:transparent"));
        assert!(!preview.contains("window.ipc"));
        assert!(!preview.contains("selectionchange"));
        assert!(preview.contains("html{background:var(--background)"));
    }

    #[test]
    fn theme_variables_cover_the_documented_names() {
        let css = theme_css(&theme(), 16.0, true);
        for name in [
            "--background:",
            "--foreground:",
            "--muted-foreground:",
            "--card:",
            "--border:",
            "--primary:",
            "--accent:",
            "--destructive:",
            "--success:",
            "--chart-6:",
            "--font-sans:",
            "--font-mono:",
            "--font-size-xs:",
            "--font-size-sm:",
            "--font-size:",
            "--font-size-lg:",
            "--font-size-xl:",
            "--font-size-2xl:",
            "--font-size-code:",
            "--line-height:",
            "--hover:",
            "--faint-foreground:",
            "--border-strong:",
            "--selection:",
            "--danger:",
            "--surface-treatment:",
        ] {
            assert!(css.contains(name), "{name} missing");
            assert!(zeron_proto::html_render::THEME_GUIDE.contains(name.trim_end_matches(':')));
        }
        assert!(css.starts_with(":root{color-scheme:dark;"));
        assert!(!css.contains("</"));
        for attribute in ["data-surface", "data-appearance"] {
            assert!(zeron_proto::html_render::THEME_GUIDE.contains(attribute));
            assert!(inject_bootstrap("<p>x</p>", &theme(), 16.0, false).contains(attribute));
        }
    }

    #[test]
    fn type_sizes_follow_the_interface_size_and_code_keeps_its_own() {
        let default = theme_css(&theme(), 16.0, true);
        assert!(default.contains("--font-size:14px;"));
        assert!(default.contains("--font-size-2xl:19px;"));
        assert!(default.contains("--line-height:22px;"));
        assert!(default.contains("--font-size-code:12.5px;"));
        let large = theme_css(&theme(), 20.0, true);
        assert!(large.contains("--font-size:17.5px;"));
        assert!(large.contains("--font-size-xs:13.75px;"));
        assert!(large.contains("--font-size-code:12.5px;"));
        assert!(large.contains("font-size:14px;line-height:1.5"));
    }

    #[test]
    fn frames_fit_the_page_unless_the_agent_asked_for_less() {
        assert_eq!(frame_height(600.0, None), 600.0);
        assert_eq!(frame_height(600.0, Some(420.0)), 420.0);
        assert_eq!(frame_height(600.0, Some(900.0)), 600.0);
        assert_eq!(frame_height(600.0, Some(640.0)), 640.0);
        assert_eq!(frame_height(1200.0, Some(900.0)), 900.0);
        assert_eq!(frame_height(2400.0, Some(5000.0)), MAX_HEIGHT);
        assert_eq!(frame_height(600.0, Some(10.0)), MIN_HEIGHT);
        assert_eq!(clamp_height(f32::NAN), MIN_HEIGHT);
        assert!(scrolls_inside(600.0, Some(900.0)));
        assert!(!scrolls_inside(420.0, Some(420.0)));
    }
}
