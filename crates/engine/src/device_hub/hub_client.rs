use std::collections::VecDeque;
use std::time::Duration;

use futures::StreamExt;
use serde::Deserialize;
use zeron_proto::device_hub::{
    AxRect, DeviceAxNode, DeviceFoldState, DevicePlatform, FoldPosture, HubDevice,
    LOCAL_DEVICE_HOST_ID,
};

const LIST_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const BOOT_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Deserialize)]
struct DeviceList {
    #[serde(default)]
    simulators: Vec<HubDevice>,
    #[serde(default)]
    emulators: Vec<HubDevice>,
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

    pub(crate) async fn list(&self) -> Result<(Vec<HubDevice>, Vec<String>), String> {
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
        let ios = list
            .simulators
            .into_iter()
            .filter(|device| device.platform == DevicePlatform::Ios);
        let android = list
            .emulators
            .into_iter()
            .filter(|device| device.platform == DevicePlatform::Android);
        let devices = ios
            .chain(android)
            .map(|device| HubDevice {
                host_id: LOCAL_DEVICE_HOST_ID.into(),
                ..device
            })
            .collect();
        let errors = list
            .errors
            .into_iter()
            .map(|error| error.message)
            .filter(|message| !message.is_empty())
            .collect();
        Ok((devices, errors))
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

    pub(crate) async fn shutdown_simulator(&self, udid: &str) -> Result<(), String> {
        self.post(
            "/vendor/serve-sim/grid/api/shutdown",
            serde_json::json!({ "udid": udid }),
            "Shutting the device down",
        )
        .await
        .map(|_| ())
    }

    pub(crate) async fn shutdown_emulator(&self, serial: &str) -> Result<(), String> {
        self.post(
            "/api/devices/shutdown",
            serde_json::json!({ "platform": DevicePlatform::Android, "id": serial }),
            "Shutting the device down",
        )
        .await
        .map(|_| ())
    }

    pub(crate) fn semu_url(&self, serial: &str) -> String {
        format!(
            "{}/vendor/serve-emu/ws?device={}&frame-meta=1",
            self.origin.replacen("http://", "ws://", 1),
            serial.chars().filter(|c| id_char(*c)).collect::<String>()
        )
    }

    pub(crate) fn mjpeg_url(&self, udid: &str) -> String {
        format!(
            "{}/vendor/serve-sim/helper/{}/stream.mjpeg",
            self.origin,
            path_segment(udid)
        )
    }

    pub(crate) fn avcc_url(&self, udid: &str) -> String {
        format!(
            "{}/vendor/serve-sim/helper/{}/stream.avcc",
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

    pub(crate) async fn ax_tree(&self, udid: &str) -> Result<Vec<DeviceAxNode>, String> {
        let unreadable = |e: reqwest::Error| {
            format!("Reading the accessibility tree failed: {}", e.without_url())
        };
        let body = self
            .http
            .get(format!(
                "{}/vendor/serve-sim/helper/{}/ax",
                self.origin,
                path_segment(udid)
            ))
            .timeout(AX_TIMEOUT)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(unreadable)?
            .bytes_stream();
        let body = read_capped(body, AX_BODY_LIMIT).await.map_err(unreadable)?;
        let payload: serde_json::Value = match body {
            Some(body) => serde_json::from_slice(&body)
                .map_err(|e| format!("Reading the accessibility tree failed: {e}"))?,
            None => return Err(AX_TOO_LARGE.into()),
        };
        match payload {
            serde_json::Value::Array(roots) => {
                let mut budget = AX_NODE_LIMIT;
                Ok(ax_nodes(&roots, &mut budget))
            }
            other => Err(other["error"]
                .as_str()
                .unwrap_or("Unexpected accessibility payload.")
                .to_string()),
        }
    }

    pub(crate) async fn fold(
        &self,
        serial: &str,
        posture: Option<FoldPosture>,
    ) -> Result<DeviceFoldState, String> {
        let url = format!("{}/vendor/serve-emu/api/fold", self.origin);
        let request = match posture {
            Some(posture) => self
                .http
                .post(url)
                .json(&serde_json::json!({ "posture": posture })),
            None => self.http.get(url),
        };
        let response = request
            .query(&[("device", serial)])
            .timeout(FOLD_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("The fold command failed: {}", e.without_url()))?;
        let status = response.status();
        // The UI retries a failed read; a hub without the route would be asked forever.
        if posture.is_none() && status.is_client_error() {
            return Ok(DeviceFoldState::default());
        }
        let body = read_capped(response.bytes_stream(), FOLD_BODY_LIMIT)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let reply: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        if !status.is_success() {
            return Err(reply["error"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| format!("Fold command failed ({}).", status.as_u16())));
        }
        match reply["ok"].as_bool() {
            Some(true) => serde_json::from_value(reply["fold"].clone())
                .map_err(|_| "Unexpected Android fold response.".to_string()),
            _ => Err("Unexpected Android fold response.".into()),
        }
    }

    pub(crate) async fn app_state(&self, udid: &str) -> Result<SseStream, String> {
        self.sse(format!(
            "/vendor/serve-sim/appstate?device={}",
            path_segment(udid)
        ))
        .await
    }

    pub(crate) async fn event_log(&self, udid: &str) -> Result<SseStream, String> {
        self.sse(format!(
            "/vendor/serve-sim/api/event-log/events?device={}&limit={EVENT_LOG_LIMIT}",
            path_segment(udid)
        ))
        .await
    }

    async fn sse(&self, path: String) -> Result<SseStream, String> {
        let body = self
            .http
            .get(format!("{}{path}", self.origin))
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("The device hub feed failed: {}", e.without_url()))?
            .bytes_stream();
        Ok(futures::stream::unfold(
            (body, SseParser::default(), VecDeque::new()),
            |(mut body, mut parser, mut ready)| async move {
                loop {
                    if let Some(value) = ready.pop_front() {
                        return Some((value, (body, parser, ready)));
                    }
                    let chunk = body.next().await?.ok()?;
                    ready.extend(
                        parser
                            .push(&chunk)
                            .into_iter()
                            .filter_map(|data| serde_json::from_str(&data).ok()),
                    );
                }
            },
        )
        .boxed())
    }
}

pub(crate) type SseStream = futures::stream::BoxStream<'static, serde_json::Value>;

const AX_TIMEOUT: Duration = Duration::from_secs(10);
// serve-emu runs `adb emu fold` and polls the posture for up to 2 s; t3code waits 12 s.
const FOLD_TIMEOUT: Duration = Duration::from_secs(12);
const FOLD_BODY_LIMIT: usize = 64 << 10;
pub(crate) const AX_NODE_LIMIT: usize = 500;
pub(crate) const AX_BODY_LIMIT: usize = 4 << 20;
pub(crate) const AX_TOO_LARGE: &str = "The accessibility tree is too large to show.";
pub(crate) const EVENT_LOG_LIMIT: usize = 100;
const SSE_LINE_LIMIT: usize = 1 << 20;
const SSE_EVENT_LIMIT: usize = 1 << 20;

async fn read_capped<B: AsRef<[u8]>, E>(
    chunks: impl futures::Stream<Item = Result<B, E>>,
    limit: usize,
) -> Result<Option<Vec<u8>>, E> {
    let mut chunks = std::pin::pin!(chunks);
    let mut body = Vec::new();
    while let Some(chunk) = chunks.next().await {
        body.extend_from_slice(chunk?.as_ref());
        if body.len() > limit {
            return Ok(None);
        }
    }
    Ok(Some(body))
}

pub(crate) fn ax_nodes(items: &[serde_json::Value], budget: &mut usize) -> Vec<DeviceAxNode> {
    let mut nodes = Vec::new();
    for item in items {
        if *budget == 0 {
            break;
        }
        let Some(frame) = item.get("frame").filter(|frame| frame.is_object()) else {
            continue;
        };
        *budget -= 1;
        let number = |key: &str| frame[key].as_f64().unwrap_or(0.0);
        let text = |key: &str| item[key].as_str().map(str::to_string);
        let children = item["children"].as_array().map_or(&[][..], Vec::as_slice);
        nodes.push(DeviceAxNode {
            frame: AxRect {
                x: number("x"),
                y: number("y"),
                width: number("width"),
                height: number("height"),
            },
            label: text("AXLabel"),
            id: text("AXUniqueId"),
            kind: text("type"),
            children: ax_nodes(children, budget),
        });
    }
    nodes
}

#[derive(Default)]
pub(crate) struct SseParser {
    line: Vec<u8>,
    data: Vec<String>,
    data_len: usize,
    dropping: bool,
}

impl SseParser {
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut events = Vec::new();
        for &byte in chunk {
            if byte != b'\n' {
                if self.line.len() < SSE_LINE_LIMIT {
                    self.line.push(byte);
                } else {
                    self.dropping = true;
                    self.data.clear();
                }
                continue;
            }
            let line = String::from_utf8_lossy(&self.line)
                .trim_end_matches('\r')
                .to_string();
            self.line.clear();
            if line.is_empty() {
                if !self.data.is_empty() && !self.dropping {
                    events.push(self.data.join("\n"));
                }
                self.data.clear();
                self.data_len = 0;
                self.dropping = false;
            } else if self.dropping {
                continue;
            } else if let Some(data) = line.strip_prefix("data:") {
                let data = data.strip_prefix(' ').unwrap_or(data);
                self.data_len += data.len() + 1;
                if self.data_len > SSE_EVENT_LIMIT {
                    self.dropping = true;
                    self.data.clear();
                } else {
                    self.data.push(data.to_string());
                }
            }
        }
        events
    }
}

fn id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')
}

pub(crate) fn valid_device_id(id: &str) -> bool {
    id.starts_with(|c: char| c.is_ascii_alphanumeric()) && id.chars().all(id_char)
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
    fn device_ids_are_udids_serials_or_avd_names_and_never_paths_or_flags() {
        for id in [
            "9C1D2E3F-0000-4ABC-8DEF-0123456789AB",
            "emulator-5554",
            "Medium_Phone_API_36.1",
            "192.168.1.20:5555",
        ] {
            assert!(valid_device_id(id), "{id:?}");
        }
        for id in [
            "",
            ".",
            "..",
            "../x",
            ".hidden",
            "_x",
            "a/b",
            "a?device=b",
            "a&b",
            "a b",
            "-",
            "-x",
            "--set",
        ] {
            assert!(!valid_device_id(id), "{id:?}");
        }
        assert_eq!(path_segment("../a.b_c/-1"), "abc-1");
        let client = HubClient {
            http: &reqwest::Client::new(),
            origin: "http://127.0.0.1:4000",
        };
        assert_eq!(
            client.semu_url("emulator-5554"),
            "ws://127.0.0.1:4000/vendor/serve-emu/ws?device=emulator-5554&frame-meta=1"
        );
        assert_eq!(
            client.semu_url("a&b=c/../d"),
            "ws://127.0.0.1:4000/vendor/serve-emu/ws?device=abc..d&frame-meta=1"
        );
    }

    #[test]
    fn sse_events_survive_chunk_splits_crlf_and_keepalives() {
        let stream = b": \n\ndata: {\"bundleId\":\"com.apple.Maps\",\"pid\":7}\r\n\r\n:\n\ndata: {\"a\":\ndata: 1}\n\n";
        let mut parser = SseParser::default();
        let mut events = Vec::new();
        for chunk in stream.chunks(5) {
            events.extend(parser.push(chunk));
        }
        assert_eq!(
            events,
            vec![
                "{\"bundleId\":\"com.apple.Maps\",\"pid\":7}".to_string(),
                "{\"a\":\n1}".to_string()
            ]
        );
    }

    #[test]
    fn sse_drops_overlong_lines_and_oversized_events_and_recovers() {
        let mut parser = SseParser::default();
        let long_line = format!("data: {}\n\n", "x".repeat(SSE_LINE_LIMIT + 10));
        assert!(parser.push(long_line.as_bytes()).is_empty());
        assert!(parser.line.is_empty());
        let line = format!("data: {}\n", "y".repeat(SSE_LINE_LIMIT / 2));
        assert!(parser.push(line.repeat(3).as_bytes()).is_empty());
        assert!(parser.data.is_empty(), "an oversized event is not buffered");
        assert_eq!(parser.push(b"\ndata: {\"ok\":1}\n\n"), vec!["{\"ok\":1}"]);
    }

    #[tokio::test]
    async fn bodies_past_the_cap_are_refused_before_parsing() {
        let chunks = |n: usize| {
            futures::stream::iter(
                (0..n).map(|_| Ok::<_, std::convert::Infallible>(vec![b'a'; 1024])),
            )
        };
        assert_eq!(
            read_capped(chunks(4), 4096).await,
            Ok(Some(vec![b'a'; 4096]))
        );
        assert_eq!(read_capped(chunks(5), 4096).await, Ok(None));
    }

    #[test]
    fn ax_nodes_keep_the_tree_and_stop_at_five_hundred() {
        let leaf = |i: usize| {
            serde_json::json!({
                "frame": {"x": i, "y": 2, "width": 10, "height": 20},
                "AXLabel": format!("Icon {i}"), "AXUniqueId": format!("id-{i}"), "type": "Button"
            })
        };
        let root = serde_json::json!([{
            "frame": {"x": 0, "y": 0, "width": 402, "height": 874},
            "AXLabel": "SpringBoard",
            "children": (0..700).map(leaf).chain([serde_json::json!({"AXLabel": "no frame"})]).collect::<Vec<_>>()
        }]);
        let mut budget = AX_NODE_LIMIT;
        let nodes = ax_nodes(root.as_array().unwrap(), &mut budget);
        fn count(nodes: &[DeviceAxNode]) -> usize {
            nodes.iter().map(|node| 1 + count(&node.children)).sum()
        }
        assert_eq!(count(&nodes), AX_NODE_LIMIT);
        assert_eq!(nodes[0].frame.width, 402.0);
        assert_eq!(nodes[0].label.as_deref(), Some("SpringBoard"));
        let first = &nodes[0].children[0];
        assert_eq!(
            (first.frame.x, first.id.as_deref(), first.kind.as_deref()),
            (0.0, Some("id-0"), Some("Button"))
        );
        assert_eq!(nodes[0].children.len(), AX_NODE_LIMIT - 1);
    }
}
