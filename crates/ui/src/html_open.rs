use std::path::Path;
use std::sync::Arc;

pub(crate) const MAX_CARDS: usize = 3;
const TITLE_SCAN_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HtmlOpenApp {
    #[default]
    SystemDefault,
    Zeron,
    App {
        path: String,
    },
}

pub(crate) fn is_html_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".html") || lower.ends_with(".htm")
}

pub(crate) fn html_link_targets<'a>(texts: impl Iterator<Item = &'a str>) -> Vec<String> {
    use pulldown_cmark::{Event, Parser, Tag};
    let mut out: Vec<String> = Vec::new();
    for text in texts {
        for event in Parser::new(text) {
            let Event::Start(Tag::Link { dest_url, .. }) = event else {
                continue;
            };
            let dest = dest_url.trim();
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
            if is_html_path(path) && !out.iter().any(|seen| seen == dest) {
                out.push(dest.to_string());
            }
        }
    }
    out
}

pub(crate) fn read_title(path: &Path) -> Result<Option<String>, ()> {
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

fn title_from_html(html: &str) -> Option<String> {
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

#[derive(Clone)]
pub(crate) struct AppChoice {
    pub path: String,
    pub name: String,
    pub icon: Option<Arc<gpui::Image>>,
    pub is_default: bool,
}

#[cfg(target_os = "macos")]
pub(crate) fn default_browser() -> Option<String> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::URLWithString(&NSString::from_str("https://example.com"))?;
        NSWorkspace::sharedWorkspace()
            .URLForApplicationToOpenURL(&url)
            .and_then(|app| app.path())
            .map(|path| path.to_string())
    })
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn default_browser() -> Option<String> {
    None
}

/// Browsers are the `.html` handlers that also open `https://` but not
/// `ssh://`; iTerm and kitty claim both web schemes and `public.html`.
/// Zeron's own bundles are left out: the menu's first row already is Zeron.
#[cfg(target_os = "macos")]
pub(crate) fn apps_for(file: &Path) -> Vec<AppChoice> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSBundle, NSFileManager, NSString, NSURL};
    use std::collections::HashSet;
    objc2::rc::autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let bundle_id = |path: &NSString| {
            NSBundle::bundleWithPath(path)
                .and_then(|bundle| bundle.bundleIdentifier())
                .map(|id| id.to_string())
                .unwrap_or_else(|| path.to_string())
        };
        let handlers = |address: &str| -> HashSet<String> {
            NSURL::URLWithString(&NSString::from_str(address))
                .map(|url| {
                    workspace
                        .URLsForApplicationsToOpenURL(&url)
                        .iter()
                        .filter_map(|app| app.path())
                        .map(|path| bundle_id(&path))
                        .collect()
                })
                .unwrap_or_default()
        };
        let web = handlers("https://example.com");
        let shells = handlers("ssh://example.com");
        let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
        let default = default_browser();
        let manager = NSFileManager::defaultManager();
        let mut seen = HashSet::new();
        let mut apps = Vec::new();
        for app in workspace.URLsForApplicationsToOpenURL(&url).iter() {
            let Some(path) = app.path() else {
                continue;
            };
            let bundle = bundle_id(&path);
            if bundle.starts_with("sh.zeron")
                || !web.contains(&bundle)
                || shells.contains(&bundle)
                || !seen.insert(bundle)
            {
                continue;
            }
            let name = manager.displayNameAtPath(&path).to_string();
            let name = name.strip_suffix(".app").unwrap_or(&name).to_string();
            let path = path.to_string();
            let icon = crate::appshots::app_icon_png(&path).and_then(|bytes| {
                crate::attachments::queue_thumbnail_image(&gpui::Image::from_bytes(
                    gpui::ImageFormat::Png,
                    bytes,
                ))
            });
            apps.push(AppChoice {
                is_default: default.as_deref() == Some(path.as_str()),
                path,
                name,
                icon,
            });
        }
        apps.sort_by(|a, b| {
            b.is_default
                .cmp(&a.is_default)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        apps
    })
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn apps_for(_file: &Path) -> Vec<AppChoice> {
    Vec::new()
}

pub(crate) fn open_with(file: &Path, app: Option<&str>) {
    let mut command = std::process::Command::new("open");
    if let Some(app) = app {
        command.arg("-a").arg(app);
    }
    if let Err(error) = command.arg(file).spawn() {
        tracing::warn!(%error, "could not open HTML file");
    }
}

pub(crate) fn reveal(file: &Path) {
    if let Err(error) = std::process::Command::new("open")
        .arg("-R")
        .arg(file)
        .spawn()
    {
        tracing::warn!(%error, "could not reveal HTML file");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_html_link_destinations_only() {
        let text = "Made [demo.html](demo.html), [page](out/Index.HTM#L3), \
                    [again](demo.html), [notes](notes.md), [web](https://x.dev/a.html), \
                    [file](file:///tmp/w/a.html), [line](b.html:12:3), `c.html`";
        assert_eq!(
            html_link_targets([text].into_iter()),
            vec![
                "demo.html",
                "out/Index.HTM#L3",
                "file:///tmp/w/a.html",
                "b.html:12:3"
            ]
        );
    }

    #[test]
    fn title_is_trimmed_and_collapsed() {
        assert_eq!(
            title_from_html("<html><head><TITLE lang=vi>\n  Demo   đơn giản </TITLE>"),
            Some("Demo đơn giản".into())
        );
        assert_eq!(title_from_html("<title></title>"), None);
        assert_eq!(title_from_html("<h1>No title</h1>"), None);
    }

    #[test]
    fn read_title_reports_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.html");
        assert_eq!(read_title(&path), Err(()));
        std::fs::write(&path, "<title>A</title>").unwrap();
        assert_eq!(read_title(&path), Ok(Some("A".into())));
        assert_eq!(read_title(dir.path()), Err(()));
    }

    #[test]
    fn remembered_app_round_trips_through_json() {
        for app in [
            HtmlOpenApp::SystemDefault,
            HtmlOpenApp::Zeron,
            HtmlOpenApp::App {
                path: "/Applications/Safari.app".into(),
            },
        ] {
            let json = serde_json::to_string(&app).unwrap();
            assert_eq!(serde_json::from_str::<HtmlOpenApp>(&json).unwrap(), app);
        }
    }
}
