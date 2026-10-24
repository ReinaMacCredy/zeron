use serde::{Deserialize, Serialize};

pub const CHAT_WALLPAPER_MAX_SIDE: u32 = 2048;
/// Raw bytes per reply: base64 of it stays under the relay's 1 MiB message cap.
pub const CHAT_WALLPAPER_CHUNK_BYTES: usize = 384 * 1024;

const BLUR_MAX: f32 = 48.0;
const BLUR_STEP: f32 = 4.0;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatWallpaperRequest {
    #[serde(default)]
    pub offset: usize,
    /// At offset 0, the image the caller already holds (not sent again);
    /// past it, the image being continued.
    #[serde(default)]
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatBackgroundPlacement {
    #[default]
    Hero,
    Full,
}

/// The desktop's `ChatBackground` as `ui-settings.json` stores it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChatBackgroundSettings {
    pub enabled: bool,
    pub placement: ChatBackgroundPlacement,
    pub opacity: f32,
    pub blur: f32,
    pub scrim: bool,
}

impl Default for ChatBackgroundSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            placement: ChatBackgroundPlacement::Hero,
            opacity: 0.2,
            blur: 24.0,
            scrim: true,
        }
    }
}

impl ChatBackgroundSettings {
    pub fn clamped(mut self) -> Self {
        let default = Self::default();
        self.opacity = if self.opacity.is_finite() {
            self.opacity.clamp(0.0, 1.0)
        } else {
            default.opacity
        };
        let blur = if self.blur.is_finite() {
            self.blur.clamp(0.0, BLUR_MAX)
        } else {
            default.blur
        };
        self.blur = (blur / BLUR_STEP).round() * BLUR_STEP;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatWallpaper {
    pub chat_background: ChatBackgroundSettings,
    pub effect: String,
    pub name: Option<String>,
    pub image: Option<ChatWallpaperImage>,
}

/// One slice of the phone-sized JPEG; empty `data` at offset 0 means the
/// caller's `hash` is still current.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatWallpaperImage {
    pub hash: String,
    pub size: usize,
    pub data: String,
    pub next_offset: usize,
    pub done: bool,
}
