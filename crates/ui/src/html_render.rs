//! Agent-published HTML pages ("HTML renders", the `html_render` thread
//! tool): the theme a page is handed as CSS variables, the bootstrap injected
//! at the start of its `<head>`, and the frame height rules. Ported from t3code
//! `packages/shared/src/htmlRender.ts` so pages written for either app agree.

use crate::theme::Theme;
use gpui::Hsla;
use zeron_proto::html_render::{self as shared, PageTheme, Rgba};

pub const MIN_HEIGHT: f32 = shared::MIN_HEIGHT as f32;
pub const MAX_HEIGHT: f32 = shared::MAX_HEIGHT as f32;

pub use zeron_proto::html_render::{frame_height, overflows, scrolls_inside};

pub fn clamp_height(height: f32) -> f32 {
    if height.is_finite() {
        height.round().clamp(MIN_HEIGHT, MAX_HEIGHT)
    } else {
        MIN_HEIGHT
    }
}

fn rgba(color: Hsla) -> Rgba {
    let rgba = gpui::Rgba::from(color);
    Rgba {
        r: rgba.r,
        g: rgba.g,
        b: rgba.b,
        a: rgba.a,
    }
}

/// The theme's roles as the shared page theme holds them.
pub fn page_theme(theme: &Theme) -> PageTheme {
    let c = rgba;
    PageTheme {
        dark: theme.appearance.is_dark(),
        glass: theme.is_glass(),
        bg: c(theme.bg),
        text: c(theme.text),
        text_faint: c(theme.text_faint),
        text_muted: c(theme.text_muted),
        element_hover: c(theme.element_hover),
        hover: c(theme.wash(0.08)),
        selection: c(theme.selection),
        surface_card: c(theme.surface_card),
        surface_overlay: c(theme.surface_overlay),
        surface_raised: c(theme.surface_raised),
        border: c(theme.border),
        border_strong: c(theme.border_strong),
        input_bg: c(theme.input_bg),
        accent: c(theme.accent),
        solid: c(theme.solid),
        on_solid: c(theme.on_solid),
        on_accent: c(theme.on_accent),
        accent_wash: c(theme.accent_wash),
        accent_strong: c(theme.accent_strong),
        danger: c(theme.danger),
        danger_strong: c(theme.danger_strong),
        danger_muted: c(theme.danger_muted),
        warning: c(theme.warning),
        warning_muted: c(theme.warning_muted),
        success: c(theme.success),
        success_muted: c(theme.success_muted),
        code_wash: c(theme.code_wash),
        code_text: c(theme.code_text),
        font_sans: theme.font_sans.to_string(),
        font_mono: theme.font_mono.to_string(),
        code_font_size: theme.code_font_size,
        text_size: crate::markdown::render::MD_TEXT_SIZE,
        line_height: crate::markdown::render::MD_LINE_HEIGHT,
    }
}

/// The variables a page styles against, named like t3's so one page works in
/// both apps. `--accent` is the brand color, not a hover surface.
pub fn theme_variables(theme: &Theme, font_size: f32) -> Vec<(&'static str, String)> {
    shared::theme_variables(&page_theme(theme), font_size)
}

/// The theme stylesheet the bootstrap's `<style id="zeron-theme">` holds.
pub fn theme_css(theme: &Theme, font_size: f32, inline: bool) -> String {
    shared::theme_css(&page_theme(theme), font_size, inline)
}

#[cfg(target_os = "macos")]
pub fn live_script() -> String {
    format!("{}{}", shared::EDGE_SCRIPT, shared::ACTIVITY_SCRIPT)
}

/// JavaScript that swaps a mounted page's theme without reloading it.
pub fn retheme_script(theme: &Theme, font_size: f32, inline: bool) -> String {
    shared::retheme_script(&page_theme(theme), font_size, inline)
}

/// Inserts the theme bootstrap at the start of the document head, so a
/// page's own styles and scripts come after it and win.
pub fn inject_bootstrap(html: &str, theme: &Theme, font_size: f32, inline: bool) -> String {
    shared::themed_page(html, &page_theme(theme), font_size, inline, "")
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
        assert!(inline.contains("{active:1}") && inline.contains("{edges:v}"));
        assert!(inline.contains("html{background:transparent"));
        assert!(!preview.contains("window.ipc"));
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
}
