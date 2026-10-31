use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use gpui::{Image, ImageFormat};

pub use zeron_docs::changelog::{
    Entry, Feature, Improvement, Resolution, entries, entry_for, resolve,
};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Picture {
    pub image: Arc<Image>,
    pub aspect: f32,
}

pub fn picture(name: &str) -> Option<&'static Picture> {
    static PICTURES: OnceLock<Vec<(&'static str, Picture)>> = OnceLock::new();
    PICTURES
        .get_or_init(|| {
            zeron_docs::images::IMAGES
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
    use zeron_update::version_newer;

    use super::*;

    #[test]
    fn bundled_changelog_parses_newest_first_with_every_image() {
        let entries = entries();
        assert!(!entries.is_empty());
        assert!(
            entry_for(entries, CURRENT_VERSION).is_some(),
            "a version bump needs its whats_new.toml entry"
        );
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
