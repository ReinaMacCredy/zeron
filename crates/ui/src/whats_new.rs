use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use gpui::{Image, ImageFormat};
use zeron_update::version_newer;

pub use zeron_docs::changelog::{Entry, Feature, entries, entry_for};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const IMAGES: &[(&str, &[u8])] = &[
    ("inbox.jpg", include_bytes!("../assets/whats_new/inbox.jpg")),
    (
        "kanban.jpg",
        include_bytes!("../assets/whats_new/kanban.jpg"),
    ),
    ("usage.jpg", include_bytes!("../assets/whats_new/usage.jpg")),
    (
        "mission-control.jpg",
        include_bytes!("../assets/whats_new/mission-control.jpg"),
    ),
    (
        "supervisor.jpg",
        include_bytes!("../assets/whats_new/supervisor.jpg"),
    ),
    (
        "add-provider.jpg",
        include_bytes!("../assets/whats_new/add-provider.jpg"),
    ),
    (
        "mission-tree.jpg",
        include_bytes!("../assets/whats_new/mission-tree.jpg"),
    ),
    (
        "task-card-context.jpg",
        include_bytes!("../assets/whats_new/task-card-context.jpg"),
    ),
];

pub struct Picture {
    pub image: Arc<Image>,
    pub aspect: f32,
}

#[derive(Debug, PartialEq)]
pub enum Resolution<'a> {
    Show(&'a Entry),
    Advance,
    Noop,
}

pub fn resolve<'a>(entries: &'a [Entry], current: &str, last_seen: Option<&str>) -> Resolution<'a> {
    let Some(last_seen) = last_seen else {
        return Resolution::Advance;
    };
    if !version_newer(current, last_seen) {
        return Resolution::Noop;
    }
    entry_for(entries, current).map_or(Resolution::Advance, Resolution::Show)
}

pub fn picture(name: &str) -> Option<&'static Picture> {
    static PICTURES: OnceLock<Vec<(&'static str, Picture)>> = OnceLock::new();
    PICTURES
        .get_or_init(|| {
            IMAGES
                .iter()
                .filter_map(|(name, bytes)| Some((*name, decode(bytes)?)))
                .collect()
        })
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, picture)| picture)
}

fn decode(bytes: &[u8]) -> Option<Picture> {
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let format = match reader.format()? {
        image::ImageFormat::Jpeg => ImageFormat::Jpeg,
        image::ImageFormat::WebP => ImageFormat::Webp,
        image::ImageFormat::Png => ImageFormat::Png,
        _ => return None,
    };
    let (width, height) = reader.into_dimensions().ok()?;
    (width > 0 && height > 0).then(|| Picture {
        image: Arc::new(Image::from_bytes(format, bytes.to_vec())),
        aspect: width as f32 / height as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(version: &str) -> Entry {
        Entry {
            version: version.into(),
            date: "Oct 17".into(),
            hero: None,
            features: Vec::new(),
        }
    }

    #[test]
    fn fresh_install_records_the_current_version_silently() {
        let entries = [entry("0.2.113")];
        assert_eq!(resolve(&entries, "0.2.113", None), Resolution::Advance);
    }

    #[test]
    fn same_or_older_version_does_nothing() {
        let entries = [entry("0.2.113")];
        assert_eq!(
            resolve(&entries, "0.2.113", Some("0.2.113")),
            Resolution::Noop
        );
        assert_eq!(
            resolve(&entries, "0.2.112", Some("0.2.113")),
            Resolution::Noop
        );
    }

    #[test]
    fn upgrade_with_an_entry_shows_it() {
        let entries = [entry("0.2.112"), entry("0.2.113")];
        assert_eq!(
            resolve(&entries, "0.2.113", Some("0.2.99")),
            Resolution::Show(&entries[1])
        );
        assert_eq!(
            resolve(&entries, "0.2.113", Some("0.0.0")),
            Resolution::Show(&entries[1])
        );
    }

    #[test]
    fn upgrade_without_an_entry_advances_silently() {
        let entries = [entry("0.2.112")];
        assert_eq!(
            resolve(&entries, "0.2.113", Some("0.2.112")),
            Resolution::Advance
        );
    }

    #[test]
    fn bundled_changelog_parses_newest_first_with_every_image() {
        let entries = entries();
        assert!(!entries.is_empty());
        assert!(
            entries
                .windows(2)
                .all(|pair| version_newer(&pair[0].version, &pair[1].version))
        );
        for entry in entries {
            assert!(
                !entry.features.is_empty(),
                "{} has no features",
                entry.version
            );
            let images = entry
                .hero
                .iter()
                .chain(entry.features.iter().filter_map(|f| f.image.as_ref()));
            for name in images {
                assert!(picture(name).is_some(), "{name} is not a bundled image");
            }
        }
    }
}
