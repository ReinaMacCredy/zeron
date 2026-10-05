use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use zeron_proto::device_hub::{DeviceInput, DeviceScreenConfig, TouchPhase};

const TOUCH: u8 = 0x03;
const HARDWARE_KEYBOARD: u8 = 0x0E;
const SCREEN_CONFIG: u8 = 0x82;
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

fn message(tag: u8, body: &impl Serialize) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend(serde_json::to_vec(body).unwrap_or_default());
    out
}

#[derive(Serialize)]
struct Touch {
    r#type: TouchPhase,
    x: f32,
    y: f32,
}

pub(crate) fn encode(input: &DeviceInput) -> Vec<u8> {
    match *input {
        DeviceInput::Touch { phase, x, y } => message(
            TOUCH,
            &Touch {
                r#type: phase,
                x: x.clamp(0.0, 1.0),
                y: y.clamp(0.0, 1.0),
            },
        ),
    }
}

pub(crate) fn hardware_keyboard_off() -> Vec<u8> {
    message(HARDWARE_KEYBOARD, &serde_json::json!({ "enabled": false }))
}

pub(crate) fn screen_config(bytes: &[u8]) -> Option<DeviceScreenConfig> {
    let (&tag, body) = bytes.split_first()?;
    (tag == SCREEN_CONFIG)
        .then(|| serde_json::from_slice(body).ok())
        .flatten()
}

pub(crate) struct InputSocket {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    live: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl InputSocket {
    pub(crate) fn connect(
        url: String,
        on_screen: impl Fn(DeviceScreenConfig) + Send + Sync + 'static,
    ) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let live = Arc::new(AtomicBool::new(false));
        let flag = live.clone();
        let task = tokio::spawn(async move {
            loop {
                if let Ok((socket, _)) = tokio_tungstenite::connect_async(url.as_str()).await {
                    let (mut sink, mut stream) = socket.split();
                    // Serve-sim turns the hardware keyboard back on when the
                    // last socket closes; off lets taps raise the on-screen one.
                    if sink
                        .send(Message::Binary(hardware_keyboard_off()))
                        .await
                        .is_ok()
                    {
                        while rx.try_recv().is_ok() {}
                        flag.store(true, Ordering::SeqCst);
                        loop {
                            tokio::select! {
                                outgoing = rx.recv() => match outgoing {
                                    Some(bytes) => {
                                        if sink.send(Message::Binary(bytes)).await.is_err() {
                                            break;
                                        }
                                    }
                                    None => return,
                                },
                                incoming = stream.next() => match incoming {
                                    Some(Ok(Message::Binary(bytes))) => {
                                        if let Some(screen) = screen_config(&bytes) {
                                            on_screen(screen);
                                        }
                                    }
                                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                                    Some(Ok(_)) => {}
                                },
                            }
                        }
                        flag.store(false, Ordering::SeqCst);
                    }
                }
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
        });
        Self { tx, live, task }
    }

    pub(crate) fn send(&self, input: &DeviceInput) -> Result<(), String> {
        if !self.live.load(Ordering::SeqCst) {
            return Err("Input disconnected, reconnecting...".into());
        }
        self.tx
            .send(encode(input))
            .map_err(|_| "Input disconnected".to_string())
    }
}

impl Drop for InputSocket {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_begin_is_tag_three_then_json() {
        let bytes = encode(&DeviceInput::Touch {
            phase: TouchPhase::Begin,
            x: 0.25,
            y: 0.5,
        });
        let mut expected = vec![0x03];
        expected.extend_from_slice(br#"{"type":"begin","x":0.25,"y":0.5}"#);
        assert_eq!(bytes, expected);
    }

    #[test]
    fn hardware_keyboard_off_uses_tag_fourteen() {
        let mut expected = vec![0x0E];
        expected.extend_from_slice(br#"{"enabled":false}"#);
        assert_eq!(hardware_keyboard_off(), expected);
    }

    #[test]
    fn screen_config_reads_only_tag_0x82() {
        let mut bytes = vec![0x82];
        bytes.extend_from_slice(br#"{"width":588,"height":1280,"orientation":"portrait"}"#);
        let screen = screen_config(&bytes).unwrap();
        assert_eq!((screen.width, screen.height), (588, 1280));
        bytes[0] = 0x8F;
        assert!(screen_config(&bytes).is_none());
    }
}
