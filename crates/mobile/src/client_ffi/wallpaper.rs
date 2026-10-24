use std::cmp::Reverse;

use zeron_client as zc;
use zeron_proto::chat_wallpaper::ChatBackgroundPlacement;

use super::{CoreClient, CoreResult, DeviceView, on_runtime};

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SyncedWallpaper {
    pub enabled: bool,
    pub placement: String,
    pub opacity: f32,
    pub blur: f32,
    pub scrim: bool,
    pub effect: String,
    pub name: Option<String>,
    pub image_hash: Option<String>,
    /// Only when it differs from the `known_hash` asked with.
    pub image: Option<Vec<u8>>,
}

fn wallpaper_hosts(devices: &[zc::DeviceView]) -> Vec<DeviceView> {
    let mut hosts: Vec<&zc::DeviceView> = devices
        .iter()
        .filter(|d| {
            d.is_execution_host && !d.is_self && !zeron_proto::is_cloud_device(&d.id, &d.platform)
        })
        .collect();
    hosts.sort_by_key(|d| (!d.online, Reverse(d.last_seen_ms)));
    hosts.into_iter().map(Into::into).collect()
}

#[uniffi::export]
impl CoreClient {
    pub fn wallpaper_hosts(&self) -> Vec<DeviceView> {
        wallpaper_hosts(&self.client.workspace().devices)
    }

    pub async fn chat_wallpaper(
        &self,
        device_id: String,
        known_hash: Option<String>,
    ) -> CoreResult<SyncedWallpaper> {
        let client = self.client.clone();
        let (wallpaper, image) = on_runtime(async move {
            client
                .chat_wallpaper(&device_id, known_hash.as_deref())
                .await
        })
        .await?;
        let chat = wallpaper.chat_background;
        Ok(SyncedWallpaper {
            enabled: chat.enabled,
            placement: match chat.placement {
                ChatBackgroundPlacement::Hero => "hero",
                ChatBackgroundPlacement::Full => "full",
            }
            .into(),
            opacity: chat.opacity,
            blur: chat.blur,
            scrim: chat.scrim,
            effect: wallpaper.effect,
            name: wallpaper.name,
            image_hash: wallpaper.image.map(|image| image.hash),
            image,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, platform: &str, online: bool, last_seen_ms: i64) -> zc::DeviceView {
        zc::DeviceView {
            id: id.into(),
            name: id.into(),
            platform: platform.into(),
            online,
            last_seen_ms: Some(last_seen_ms),
            version: None,
            capabilities: Vec::new(),
            is_execution_host: platform != "ios",
            is_self: platform == "ios",
            session_count: 0,
        }
    }

    #[test]
    fn follows_the_most_recently_online_desktop_and_never_cloud_or_this_phone() {
        let devices = [
            device("old-mac", "macos", false, 300),
            device("phone", "ios", true, 900),
            device("cloud-1", "cloud", true, 800),
            device("studio", "macos", true, 100),
            device("laptop", "macos", true, 200),
        ];
        let ids: Vec<String> = wallpaper_hosts(&devices)
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, ["laptop", "studio", "old-mac"]);
    }
}
