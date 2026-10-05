use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use zeron_proto::device_hub::{
    DeviceButton, DeviceInput, DeviceOrientation, DeviceScreenConfig, KeyPhase, StreamLinkStatus,
    TouchPhase,
};

const TOUCH: u8 = 0x03;
const BUTTON: u8 = 0x04;
const KEY: u8 = 0x06;
const ORIENTATION: u8 = 0x07;
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

#[derive(Serialize)]
struct Button {
    button: DeviceButton,
}

#[derive(Serialize)]
struct Key {
    r#type: KeyPhase,
    usage: u16,
}

#[derive(Serialize)]
struct Orientation {
    orientation: DeviceOrientation,
}

pub(crate) fn encode(input: &DeviceInput) -> Option<Vec<u8>> {
    Some(match *input {
        DeviceInput::Button {
            button: DeviceButton::Back,
        }
        | DeviceInput::Keycode { .. }
        | DeviceInput::Text { .. } => return None,
        DeviceInput::Touch { phase, x, y } => message(
            TOUCH,
            &Touch {
                r#type: phase,
                x: x.clamp(0.0, 1.0),
                y: y.clamp(0.0, 1.0),
            },
        ),
        DeviceInput::Button { button } => message(BUTTON, &Button { button }),
        DeviceInput::Key { phase, usage } => message(
            KEY,
            &Key {
                r#type: phase,
                usage,
            },
        ),
        DeviceInput::Orientation { orientation } => {
            message(ORIENTATION, &Orientation { orientation })
        }
    })
}

pub(crate) const RESET_VIDEO: &str = r#"{"type":"reset-video","ack":false}"#;

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Control<'a> {
    Touch {
        action: &'static str,
        x: f32,
        y: f32,
    },
    Home,
    Back,
    Recents,
    Power,
    Key {
        keycode: u16,
        #[serde(rename = "metaState", skip_serializing_if = "is_zero")]
        meta_state: u32,
    },
    Text {
        text: &'a str,
    },
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

pub(crate) fn encode_semu(input: &DeviceInput) -> Option<String> {
    let control = match input {
        DeviceInput::Touch { phase, x, y } => Control::Touch {
            action: match phase {
                TouchPhase::Begin => "down",
                TouchPhase::Move => "move",
                TouchPhase::End => "up",
            },
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
        },
        DeviceInput::Button { button } => match button {
            DeviceButton::Home => Control::Home,
            DeviceButton::Back => Control::Back,
            DeviceButton::AppSwitcher => Control::Recents,
            DeviceButton::Lock => Control::Power,
        },
        DeviceInput::Keycode {
            keycode,
            meta_state,
        } => Control::Key {
            keycode: *keycode,
            meta_state: *meta_state,
        },
        DeviceInput::Text { text } if !text.is_empty() => Control::Text { text },
        _ => return None,
    };
    serde_json::to_string(&control).ok()
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

pub(crate) const UNSUPPORTED_INPUT: &str = "This device does not accept that input.";

pub(crate) struct InputSocket {
    tx: mpsc::UnboundedSender<Message>,
    live: Arc<AtomicBool>,
    encode: fn(&DeviceInput) -> Option<Message>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl InputSocket {
    pub(crate) fn connect(
        url: String,
        on_screen: impl Fn(DeviceScreenConfig) + Send + Sync + 'static,
        on_status: impl Fn(StreamLinkStatus) + Send + Sync + 'static,
    ) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let live = Arc::new(AtomicBool::new(false));
        let flag = live.clone();
        let task = tokio::spawn(async move {
            loop {
                on_status(StreamLinkStatus::Connecting);
                let detail = match tokio_tungstenite::connect_async(url.as_str()).await {
                    Err(_) => "could not connect",
                    Ok((socket, _)) => {
                        let (mut sink, mut stream) = socket.split();
                        // Serve-sim turns the hardware keyboard back on when the
                        // last socket closes; off lets taps raise the on-screen one.
                        if sink
                            .send(Message::Binary(hardware_keyboard_off()))
                            .await
                            .is_err()
                        {
                            "socket closed"
                        } else {
                            while rx.try_recv().is_ok() {}
                            flag.store(true, Ordering::SeqCst);
                            on_status(StreamLinkStatus::Live);
                            let detail = loop {
                                tokio::select! {
                                    outgoing = rx.recv() => match outgoing {
                                        Some(message) => {
                                            if sink.send(message).await.is_err() {
                                                break "socket closed";
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
                                        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                                            break "socket closed";
                                        }
                                        Some(Ok(_)) => {}
                                    },
                                }
                            };
                            flag.store(false, Ordering::SeqCst);
                            detail
                        }
                    }
                };
                on_status(StreamLinkStatus::Failed {
                    detail: detail.into(),
                });
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
        });
        Self {
            tx,
            live,
            encode: |input| encode(input).map(Message::Binary),
            task: Some(task),
        }
    }

    pub(crate) fn semu(tx: mpsc::UnboundedSender<Message>, live: Arc<AtomicBool>) -> Self {
        Self {
            tx,
            live,
            encode: |input| encode_semu(input).map(Message::Text),
            task: None,
        }
    }

    pub(crate) fn send(&self, input: &DeviceInput) -> Result<(), String> {
        let message = (self.encode)(input).ok_or(UNSUPPORTED_INPUT)?;
        if !self.live.load(Ordering::SeqCst) {
            return Err("Input disconnected, reconnecting...".into());
        }
        self.tx
            .send(message)
            .map_err(|_| "Input disconnected".to_string())
    }
}

impl Drop for InputSocket {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
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
        })
        .unwrap();
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
    fn buttons_keys_and_orientation_use_the_serve_sim_tags() {
        let tagged = |tag: u8, json: &[u8]| {
            let mut bytes = vec![tag];
            bytes.extend_from_slice(json);
            bytes
        };
        assert_eq!(
            encode(&DeviceInput::Button {
                button: DeviceButton::Home
            }),
            Some(tagged(0x04, br#"{"button":"home"}"#))
        );
        assert_eq!(
            encode(&DeviceInput::Key {
                phase: KeyPhase::Up,
                usage: 0xE1
            }),
            Some(tagged(0x06, br#"{"type":"up","usage":225}"#))
        );
        assert_eq!(
            encode(&DeviceInput::Orientation {
                orientation: DeviceOrientation::PortraitUpsideDown
            }),
            Some(tagged(0x07, br#"{"orientation":"portrait_upside_down"}"#))
        );
        for android_only in [
            DeviceInput::Button {
                button: DeviceButton::Back,
            },
            DeviceInput::Keycode {
                keycode: 66,
                meta_state: 0,
            },
            DeviceInput::Text { text: "a".into() },
        ] {
            assert_eq!(encode(&android_only), None, "{android_only:?}");
        }
    }

    #[test]
    fn android_control_messages_are_serve_emu_json() {
        let touch = |phase, x, y| DeviceInput::Touch { phase, x, y };
        let button = |button| DeviceInput::Button { button };
        let cases = [
            (
                touch(TouchPhase::Begin, 0.25, 0.5),
                r#"{"type":"touch","action":"down","x":0.25,"y":0.5}"#,
            ),
            (
                touch(TouchPhase::Move, 0.1, 1.5),
                r#"{"type":"touch","action":"move","x":0.1,"y":1.0}"#,
            ),
            (
                touch(TouchPhase::End, -0.2, 0.75),
                r#"{"type":"touch","action":"up","x":0.0,"y":0.75}"#,
            ),
            (button(DeviceButton::Home), r#"{"type":"home"}"#),
            (button(DeviceButton::Back), r#"{"type":"back"}"#),
            (button(DeviceButton::AppSwitcher), r#"{"type":"recents"}"#),
            (button(DeviceButton::Lock), r#"{"type":"power"}"#),
            (
                DeviceInput::Keycode {
                    keycode: 66,
                    meta_state: 0,
                },
                r#"{"type":"key","keycode":66}"#,
            ),
            (
                DeviceInput::Keycode {
                    keycode: 29,
                    meta_state: 0x3000,
                },
                r#"{"type":"key","keycode":29,"metaState":12288}"#,
            ),
            (
                DeviceInput::Text {
                    text: "é\"".into()
                },
                r#"{"type":"text","text":"é\""}"#,
            ),
        ];
        for (input, wire) in cases {
            assert_eq!(encode_semu(&input).as_deref(), Some(wire), "{input:?}");
        }
        for ios_only in [
            DeviceInput::Key {
                phase: KeyPhase::Down,
                usage: 4,
            },
            DeviceInput::Orientation {
                orientation: DeviceOrientation::LandscapeLeft,
            },
            DeviceInput::Text {
                text: String::new(),
            },
        ] {
            assert_eq!(encode_semu(&ios_only), None, "{ios_only:?}");
        }
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(RESET_VIDEO).unwrap(),
            serde_json::json!({"type": "reset-video", "ack": false})
        );
        let (tx, mut rx) = mpsc::unbounded_channel();
        let live = Arc::new(AtomicBool::new(true));
        let socket = InputSocket::semu(tx, live.clone());
        socket.send(&button(DeviceButton::Back)).unwrap();
        assert_eq!(
            rx.try_recv().unwrap(),
            Message::Text(r#"{"type":"back"}"#.into())
        );
        assert_eq!(
            socket.send(&DeviceInput::Orientation {
                orientation: DeviceOrientation::Portrait
            }),
            Err(UNSUPPORTED_INPUT.into())
        );
        live.store(false, Ordering::SeqCst);
        assert!(socket.send(&button(DeviceButton::Home)).is_err());
    }

    #[tokio::test]
    async fn a_dropped_socket_reports_failed_and_reconnects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "ws://{}/vendor/serve-sim/helper/ws?device=SIM",
            listener.local_addr().unwrap()
        );
        let (first_message, mut first_seen) = mpsc::unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            let mut connection = 0;
            loop {
                let (tcp, _) = listener.accept().await.unwrap();
                let Ok(mut socket) = tokio_tungstenite::accept_async(tcp).await else {
                    continue;
                };
                connection += 1;
                if let Some(Ok(Message::Binary(bytes))) = socket.next().await {
                    let _ = first_message.send(bytes.to_vec());
                }
                if connection == 1 {
                    drop(socket);
                } else {
                    tokio::spawn(async move { while socket.next().await.is_some() {} });
                }
            }
        });
        let (status_tx, mut statuses) = mpsc::unbounded_channel();
        let socket = InputSocket::connect(
            url,
            |_| {},
            move |status| {
                let _ = status_tx.send(status);
            },
        );
        let mut seen = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(status) = statuses.recv().await {
                let live = status == StreamLinkStatus::Live;
                seen.push(status);
                if live && seen.len() > 2 {
                    break;
                }
            }
        })
        .await
        .expect("reconnected");
        assert_eq!(
            seen,
            vec![
                StreamLinkStatus::Connecting,
                StreamLinkStatus::Live,
                StreamLinkStatus::Failed {
                    detail: "socket closed".into()
                },
                StreamLinkStatus::Connecting,
                StreamLinkStatus::Live,
            ]
        );
        assert_eq!(first_seen.recv().await.unwrap(), hardware_keyboard_off());
        assert_eq!(first_seen.recv().await.unwrap(), hardware_keyboard_off());
        socket
            .send(&DeviceInput::Button {
                button: DeviceButton::Home,
            })
            .unwrap();
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
