use std::time::Duration;

use serde::Deserialize;
use zeron_proto::device_hub::{DevicePlatform, HubDevice, LOCAL_DEVICE_HOST_ID};

const LIST_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const BOOT_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Deserialize)]
struct DeviceList {
    #[serde(default)]
    simulators: Vec<HubDevice>,
    #[serde(default)]
    errors: Vec<ListError>,
}

#[derive(Deserialize)]
struct ListError {
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct ActionResult {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    serial: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

pub(crate) struct HubClient<'a> {
    pub http: &'a reqwest::Client,
    pub origin: &'a str,
}

impl HubClient<'_> {
    async fn post(
        &self,
        path: &str,
        body: serde_json::Value,
        operation: &str,
    ) -> Result<ActionResult, String> {
        let result: ActionResult = self
            .http
            .post(format!("{}{path}", self.origin))
            .json(&body)
            .timeout(BOOT_TIMEOUT)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("{operation} failed: {}", e.without_url()))?
            .json()
            .await
            .map_err(|e| format!("{operation} failed: {}", e.without_url()))?;
        if result.ok {
            Ok(result)
        } else {
            Err(format!(
                "{operation} failed: {}",
                result.error.as_deref().unwrap_or("the device hub refused")
            ))
        }
    }

    pub(crate) async fn list(&self) -> Result<(Vec<HubDevice>, Option<String>), String> {
        let list: DeviceList = self
            .http
            .get(format!("{}/api/devices", self.origin))
            .timeout(LIST_TIMEOUT)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("Listing devices failed: {}", e.without_url()))?
            .json()
            .await
            .map_err(|e| format!("Listing devices failed: {}", e.without_url()))?;
        let devices = list
            .simulators
            .into_iter()
            .filter(|device| device.platform == DevicePlatform::Ios)
            .map(|device| HubDevice {
                host_id: LOCAL_DEVICE_HOST_ID.into(),
                ..device
            })
            .collect();
        let errors: Vec<String> = list
            .errors
            .into_iter()
            .map(|error| error.message)
            .filter(|message| !message.is_empty())
            .collect();
        Ok((devices, (!errors.is_empty()).then(|| errors.join("\n"))))
    }

    pub(crate) async fn boot(&self, device: &HubDevice) -> Result<String, String> {
        let result = self
            .post(
                "/api/devices/boot",
                serde_json::json!({ "platform": device.platform, "id": device.id, "name": device.name }),
                "Booting the device",
            )
            .await?;
        Ok(result
            .serial
            .or(result.id)
            .unwrap_or_else(|| device.id.clone()))
    }

    pub(crate) async fn attach_stream(&self, udid: &str) -> Result<(), String> {
        self.post(
            "/vendor/serve-sim/grid/api/start",
            serde_json::json!({ "udid": udid }),
            "Attaching the device stream",
        )
        .await
        .map(|_| ())
    }

    pub(crate) fn mjpeg_url(&self, udid: &str) -> String {
        format!(
            "{}/vendor/serve-sim/helper/{}/stream.mjpeg",
            self.origin,
            path_segment(udid)
        )
    }

    pub(crate) fn input_url(&self, udid: &str) -> String {
        format!(
            "{}/vendor/serve-sim/helper/ws?device={}",
            self.origin.replacen("http://", "ws://", 1),
            path_segment(udid)
        )
    }
}

pub(crate) fn valid_device_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn path_segment(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_ids_are_alphanumerics_and_dashes_only() {
        assert!(valid_device_id("9C1D2E3F-0000-4ABC-8DEF-0123456789AB"));
        for id in [
            "",
            ".",
            "..",
            "../x",
            "a/b",
            "a.b",
            "a_b",
            "a?device=b",
            "a b",
        ] {
            assert!(!valid_device_id(id), "{id:?}");
        }
        assert_eq!(path_segment("../a.b_c/-1"), "abc-1");
    }
}
