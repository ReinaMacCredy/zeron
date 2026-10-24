use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use base64::Engine as _;
use sha2::{Digest, Sha256};
use zeron_proto::chat_wallpaper::{
    CHAT_WALLPAPER_CHUNK_BYTES, CHAT_WALLPAPER_MAX_SIDE, ChatBackgroundSettings, ChatWallpaper,
    ChatWallpaperImage, ChatWallpaperRequest,
};

struct Prepared {
    source: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
    hash: String,
    jpeg: Arc<Vec<u8>>,
}

static PREPARED: Mutex<Option<Arc<Prepared>>> = Mutex::new(None);

pub(crate) fn read(
    data_dir: &Path,
    request: &ChatWallpaperRequest,
) -> Result<ChatWallpaper, String> {
    let ui = std::fs::read(data_dir.join("ui-settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .unwrap_or_default();
    let chat_background = ui
        .get("chatBackground")
        .cloned()
        .and_then(|value| serde_json::from_value::<ChatBackgroundSettings>(value).ok())
        .unwrap_or_default()
        .clamped();
    let effect = ui
        .get("newThreadBackgroundEffect")
        .and_then(|v| v.as_str())
        .unwrap_or("none")
        .to_owned();
    let background = ui.get("newThreadComposerBackground");
    let name = background
        .and_then(|b| b.get("name"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let image = match background
        .and_then(|b| b.get("path"))
        .and_then(|v| v.as_str())
    {
        Some(path) => {
            let prepared = prepare(Path::new(path))?;
            Some(chunk(&prepared, request)?)
        }
        None => None,
    };
    Ok(ChatWallpaper {
        chat_background,
        effect,
        name: image.as_ref().and(name),
        image,
    })
}

fn chunk(
    prepared: &Prepared,
    request: &ChatWallpaperRequest,
) -> Result<ChatWallpaperImage, String> {
    let size = prepared.jpeg.len();
    let known = request.hash.as_deref() == Some(prepared.hash.as_str());
    if request.offset == 0 && known {
        return Ok(ChatWallpaperImage {
            hash: prepared.hash.clone(),
            size,
            data: String::new(),
            next_offset: size,
            done: true,
        });
    }
    if request.offset > 0 && !known {
        return Err("The wallpaper changed while it was being sent; start again".into());
    }
    if request.offset > size {
        return Err("Invalid wallpaper offset".into());
    }
    let end = (request.offset + CHAT_WALLPAPER_CHUNK_BYTES).min(size);
    Ok(ChatWallpaperImage {
        hash: prepared.hash.clone(),
        size,
        data: base64::engine::general_purpose::STANDARD.encode(&prepared.jpeg[request.offset..end]),
        next_offset: end,
        done: end == size,
    })
}

fn prepare(source: &Path) -> Result<Arc<Prepared>, String> {
    let meta =
        std::fs::metadata(source).map_err(|_| "The wallpaper image is missing".to_string())?;
    let modified = meta.modified().ok();
    let mut cached = PREPARED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(hit) = cached
        .as_ref()
        .filter(|p| p.source == source && p.len == meta.len() && p.modified == modified)
    {
        return Ok(hit.clone());
    }
    let bytes =
        std::fs::read(source).map_err(|_| "The wallpaper image is unreadable".to_string())?;
    let hash = crate::repos::hex(&Sha256::digest(&bytes));
    let image = image::load_from_memory(&bytes)
        .map_err(|_| "The wallpaper image is unsupported or damaged".to_string())?;
    let image = if image.width().max(image.height()) > CHAT_WALLPAPER_MAX_SIDE {
        image.resize(
            CHAT_WALLPAPER_MAX_SIDE,
            CHAT_WALLPAPER_MAX_SIDE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode_image(&image.to_rgb8())
        .map_err(|e| e.to_string())?;
    let prepared = Arc::new(Prepared {
        source: source.to_path_buf(),
        len: meta.len(),
        modified,
        hash,
        jpeg: Arc::new(jpeg),
    });
    *cached = Some(prepared.clone());
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::chat_wallpaper::ChatBackgroundPlacement;

    fn fetch(data_dir: &Path, hash: Option<String>) -> (ChatWallpaper, Vec<u8>) {
        let mut request = ChatWallpaperRequest { offset: 0, hash };
        let mut bytes = Vec::new();
        loop {
            let reply = read(data_dir, &request).unwrap();
            let Some(image) = reply.image.clone() else {
                return (reply, bytes);
            };
            bytes.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(&image.data)
                    .unwrap(),
            );
            if image.done {
                return (reply, bytes);
            }
            request = ChatWallpaperRequest {
                offset: image.next_offset,
                hash: Some(image.hash),
            };
        }
    }

    #[test]
    fn serves_the_desktop_artwork_as_a_phone_sized_jpeg_with_its_settings() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("art.png");
        let mut art = image::RgbImage::new(3000, 1500);
        for (x, y, px) in art.enumerate_pixels_mut() {
            *px = image::Rgb([
                (x * 7 % 251) as u8,
                (y * 13 % 241) as u8,
                ((x ^ y) % 239) as u8,
            ]);
        }
        art.save(&source).unwrap();
        std::fs::write(
            dir.path().join("ui-settings.json"),
            serde_json::json!({
                "newThreadComposerBackground": { "path": source, "name": "art.png" },
                "newThreadBackgroundEffect": "halftone",
                "chatBackground": { "enabled": true, "placement": "full", "opacity": 0.55, "blur": 9.0 },
            })
            .to_string(),
        )
        .unwrap();

        let (reply, jpeg) = fetch(dir.path(), None);
        assert_eq!(reply.effect, "halftone");
        assert_eq!(reply.name.as_deref(), Some("art.png"));
        assert_eq!(
            reply.chat_background,
            ChatBackgroundSettings {
                enabled: true,
                placement: ChatBackgroundPlacement::Full,
                opacity: 0.55,
                blur: 8.0,
                scrim: true,
            }
        );
        let image = reply.image.unwrap();
        assert!(
            image.size > CHAT_WALLPAPER_CHUNK_BYTES,
            "spans several chunks"
        );
        assert_eq!(jpeg.len(), image.size);
        let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2048, 1024));

        let (again, bytes) = fetch(dir.path(), Some(image.hash.clone()));
        assert!(bytes.is_empty(), "a known image is not sent again");
        assert_eq!(again.image.unwrap().hash, image.hash);

        let stale = read(
            dir.path(),
            &ChatWallpaperRequest {
                offset: CHAT_WALLPAPER_CHUNK_BYTES,
                hash: Some("old".into()),
            },
        );
        assert!(stale.is_err());
    }

    #[test]
    fn no_artwork_still_reports_the_settings_with_desktop_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let (reply, _) = fetch(dir.path(), None);
        assert!(reply.image.is_none() && reply.name.is_none());
        assert_eq!(reply.effect, "none");
        assert_eq!(reply.chat_background, ChatBackgroundSettings::default());
    }
}
