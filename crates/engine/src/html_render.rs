//! `html_render`: an agent publishes a self-contained HTML page into its own
//! chat. The page is stored under the profile's uploads (the doc only carries
//! a reference), with local images inlined so it survives the files moving.

use std::path::PathBuf;
use std::sync::OnceLock;

use base64::Engine as _;
use regex::Regex;
use sha2::{Digest, Sha256};
use zeron_proto::html_render::{
    HtmlRenderRef, MAX_HTML_BYTES, MAX_IMAGE_BYTES, MAX_PAGE_BYTES, MAX_TITLE_CHARS, clamp_height,
};

use crate::uploads::Uploads;

#[derive(Debug, PartialEq)]
pub enum PrepareError {
    /// Every message tells the agent what to do next.
    Invalid(String),
}

impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrepareError::Invalid(message) => f.write_str(message),
        }
    }
}

const MIB: f64 = 1024.0 * 1024.0;
const MAX_IMAGE_PATH: usize = 2048;

fn image_mime(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        _ => return None,
    })
}

/// An absolute image path that is a whole quoted string, or an unquoted CSS
/// `url(…)`. URLs, data:, blob: and relative paths never match (t3's rule).
fn local_image_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        let ext = "png|jpe?g|gif|webp|avif|svg|bmp|ico";
        let abs = r"(?:/[^/\s]|[A-Za-z]:[\\/])";
        Regex::new(&format!(
            r#"(?i)"({abs}[^"\r\n]*?\.(?:{ext}))"|'({abs}[^'\r\n]*?\.(?:{ext}))'|`({abs}[^`\r\n]*?\.(?:{ext}))`|url\(\s*({abs}[^\s"'`()]*?\.(?:{ext}))\s*\)"#
        ))
        .expect("valid local image pattern")
    })
}

/// Whether file bytes are an image whatever the file is named, so a renamed
/// file or symlink cannot carry other data (a secret) into a page.
fn is_image_bytes(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(12)];
    head.starts_with(b"\x89PNG")
        || head.starts_with(b"\xff\xd8\xff")
        || head.starts_with(b"GIF8")
        || head.starts_with(b"\0\0\x01\0")
        || (head.starts_with(b"BM") && head.get(6..10) == Some(b"\0\0\0\0"))
        || (head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP"))
        || matches!(
            head.get(4..12),
            Some(b"ftypavif" | b"ftypavis" | b"ftypmif1")
        )
        || has_svg_root(&bytes[..bytes.len().min(4096)])
}

fn has_svg_root(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let mut rest = text.trim_start_matches('\u{feff}').trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("<?") {
            let Some(end) = after.find("?>") else {
                return false;
            };
            rest = after[end + 2..].trim_start();
        } else if let Some(after) = rest.strip_prefix("<!--") {
            let Some(end) = after.find("-->") else {
                return false;
            };
            rest = after[end + 3..].trim_start();
        } else if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case("<!doctype") {
            let Some(end) = rest.find('>') else {
                return false;
            };
            rest = rest[end + 1..].trim_start();
        } else {
            return rest.strip_prefix("<svg").is_some_and(|after| {
                after.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/')
            });
        }
    }
}

/// Inline local images as data URIs. `strict` (publish) refuses a page with
/// unreadable images; a preview keeps going and reports them.
pub fn inline_local_images(
    html: &str,
    strict: bool,
) -> Result<(String, Vec<String>), PrepareError> {
    let mut out = String::with_capacity(html.len());
    let mut missing = Vec::new();
    let mut last = 0;
    let mut cache: std::collections::HashMap<String, Option<String>> = Default::default();
    for captures in local_image_pattern().captures_iter(html) {
        let Some(found) = (1..=4).find_map(|i| captures.get(i)) else {
            continue;
        };
        let reference = found.as_str();
        if reference.len() > MAX_IMAGE_PATH {
            continue;
        }
        let path = if reference.as_bytes().get(1) == Some(&b':') {
            reference.replace("\\\\", "\\")
        } else {
            reference.to_owned()
        };
        let data = match cache.get(&path) {
            Some(data) => data.clone(),
            None => {
                let data = read_image(&path)?;
                cache.insert(path.clone(), data.clone());
                data
            }
        };
        let Some(data) = data else {
            if !missing.contains(&path) {
                missing.push(path);
            }
            continue;
        };
        out.push_str(&html[last..found.start()]);
        out.push_str(&data);
        last = found.end();
        if out.len() + html.len() - last > MAX_PAGE_BYTES {
            return Err(page_too_large(out.len() + html.len() - last));
        }
    }
    out.push_str(&html[last..]);
    if strict && !missing.is_empty() {
        return Err(PrepareError::Invalid(format!(
            "These local images could not be read: {}. Use absolute paths to existing image files, or remove them.",
            missing.join(", ")
        )));
    }
    if out.len() > MAX_PAGE_BYTES {
        return Err(page_too_large(out.len()));
    }
    Ok((out, missing))
}

fn page_too_large(bytes: usize) -> PrepareError {
    PrepareError::Invalid(format!(
        "With its images inlined the page is {:.1} MiB; the limit is {:.1} MiB. Use smaller images.",
        bytes as f64 / MIB,
        MAX_PAGE_BYTES as f64 / MIB
    ))
}

fn read_image(path: &str) -> Result<Option<String>, PrepareError> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(None);
    };
    if !meta.is_file() {
        return Ok(None);
    }
    if meta.len() as usize > MAX_IMAGE_BYTES {
        return Err(PrepareError::Invalid(format!(
            "{path} is {:.1} MiB; each local image must be at most {:.1} MiB.",
            meta.len() as f64 / MIB,
            MAX_IMAGE_BYTES as f64 / MIB
        )));
    }
    let Ok(bytes) = std::fs::read(path) else {
        return Ok(None);
    };
    if !is_image_bytes(&bytes) {
        return Ok(None);
    }
    let mime = image_mime(path).unwrap_or("application/octet-stream");
    Ok(Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )))
}

/// Checks the agent's arguments before anything touches disk.
pub fn validate(html: &str, title: &str) -> Result<String, PrepareError> {
    if html.trim().is_empty() {
        return Err(PrepareError::Invalid(
            "html is empty; pass a complete HTML document.".into(),
        ));
    }
    if html.len() > MAX_HTML_BYTES {
        return Err(PrepareError::Invalid(format!(
            "html is {} bytes; the limit is {MAX_HTML_BYTES}. Move large data out of the page or inline less.",
            html.len()
        )));
    }
    let title: String = title.trim().chars().take(MAX_TITLE_CHARS).collect();
    if title.is_empty() {
        return Err(PrepareError::Invalid(
            "title is empty; give the page a short name.".into(),
        ));
    }
    Ok(title)
}

/// The key the engine matches a published page to its tool call by: the
/// agent's own html, before images were inlined.
pub fn html_key(html: &str) -> String {
    format!("{:x}", Sha256::digest(html.as_bytes()))
}

fn chat_dir(uploads: &Uploads, chat_id: &str) -> PathBuf {
    let safe: String = chat_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    uploads.dir().join("html").join(safe)
}

/// Stores a prepared page for `chat_id` and returns its reference.
pub fn publish(
    uploads: &Uploads,
    chat_id: &str,
    html: &str,
    title: &str,
    height: f64,
) -> Result<HtmlRenderRef, PrepareError> {
    use std::io::Write;
    let title = validate(html, title)?;
    let (page, _) = inline_local_images(html, true)?;
    let store = || PrepareError::Invalid("The page could not be saved. Try again.".into());
    let dir = chat_dir(uploads, chat_id);
    std::fs::create_dir_all(&dir).map_err(|_| store())?;
    let path = dir.join(format!("{}.html", html_key(&page)));
    let mut temporary = tempfile::NamedTempFile::new_in(&dir).map_err(|_| store())?;
    temporary.write_all(page.as_bytes()).map_err(|_| store())?;
    temporary.as_file().sync_all().map_err(|_| store())?;
    temporary.persist(&path).map_err(|_| store())?;
    Ok(HtmlRenderRef {
        path: path.to_string_lossy().into_owned(),
        title,
        height: clamp_height(height),
    })
}

/// A deleted chat takes its pages with it.
pub fn remove_chat_pages(uploads: &Uploads, chat_id: &str) {
    let dir = chat_dir(uploads, chat_id);
    if dir.exists()
        && let Err(error) = std::fs::remove_dir_all(&dir)
    {
        tracing::warn!(chat = %chat_id, %error, "html render pages not removed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfixture";

    #[test]
    fn local_images_inline_in_every_spelling_and_urls_stay() {
        let dir = tempfile::tempdir().unwrap();
        let shot = dir.path().join("shot.png");
        std::fs::write(&shot, PNG).unwrap();
        let p = shot.to_string_lossy();
        let html = format!(
            r#"<img src="{p}"><div style="background:url({p})"></div><script>let s='{p}';</script><img src="https://cdn.test/a.png"><img src="rel/b.png">"#
        );
        let (out, missing) = inline_local_images(&html, true).unwrap();
        assert!(missing.is_empty());
        assert_eq!(out.matches("data:image/png;base64,").count(), 3);
        assert!(out.contains("https://cdn.test/a.png"));
        assert!(out.contains("rel/b.png"));
        assert!(!out.contains(&*p));
    }

    #[test]
    fn a_renamed_secret_never_rides_in_as_an_image() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("id_rsa.png");
        std::fs::write(&secret, b"-----BEGIN OPENSSH PRIVATE KEY-----").unwrap();
        let html = format!(r#"<img src="{}">"#, secret.display());
        assert!(
            matches!(inline_local_images(&html, true), Err(PrepareError::Invalid(m)) if m.contains("could not be read"))
        );
        let (out, missing) = inline_local_images(&html, false).unwrap();
        assert_eq!(out, html);
        assert_eq!(missing, vec![secret.to_string_lossy().into_owned()]);
    }

    #[test]
    fn svg_is_recognized_by_its_root_element() {
        assert!(is_image_bytes(
            b"<?xml version=\"1.0\"?><!-- x --><svg xmlns=\"http://www.w3.org/2000/svg\"/>"
        ));
        assert!(!is_image_bytes(b"<html><svg></svg></html>"));
    }

    #[test]
    fn publish_stores_the_page_per_chat_and_delete_removes_it() {
        let root = tempfile::tempdir().unwrap();
        let uploads = Uploads::from_root(root.path());
        let page = publish(&uploads, "c/1", "<p>hi</p>", "  Chart  ", 5000.0).unwrap();
        assert_eq!(page.title, "Chart");
        assert_eq!(page.height, 2000);
        let path = PathBuf::from(&page.path);
        assert!(path.starts_with(root.path().join("html").join("c_1")));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "<p>hi</p>");
        remove_chat_pages(&uploads, "c/1");
        assert!(!path.exists());
    }

    #[test]
    fn arguments_are_checked_with_actionable_messages() {
        assert!(validate("", "t").is_err());
        assert!(validate("<p>x</p>", "  ").is_err());
        assert!(validate(&"x".repeat(MAX_HTML_BYTES + 1), "t").is_err());
        assert_eq!(
            validate("<p>x</p>", &"t".repeat(300)).unwrap().len(),
            MAX_TITLE_CHARS
        );
    }
}
