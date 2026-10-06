use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use zeron_proto::device_hub::{
    DeviceButton, DeviceFacing, DeviceInput, DeviceOrientation, DeviceScreenConfig, HingeCommand,
    HingePose, KeyPhase, StreamLinkStatus, TouchPhase,
};

const TOUCH: u8 = 0x03;
const BUTTON: u8 = 0x04;
const KEY: u8 = 0x06;
const ORIENTATION: u8 = 0x07;
const HARDWARE_KEYBOARD: u8 = 0x0E;
const HINGE: u8 = 0x10;
const SCREEN_CONFIG: u8 = 0x82;
const CONTROL_REPLY: u8 = 0x90;
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

#[derive(Serialize)]
#[serde(tag = "control", content = "value", rename_all = "lowercase")]
enum HingeControl {
    Pose(HingePose),
    Angle(f64),
    Physical(DeviceFacing),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HingeRequest {
    request_id: u64,
    command: HingeControl,
}

pub(crate) fn hinge_request(request_id: u64, command: HingeCommand) -> Vec<u8> {
    let command = match command {
        HingeCommand::Pose(pose) => HingeControl::Pose(pose),
        HingeCommand::Angle(angle) => HingeControl::Angle(angle),
        HingeCommand::Physical(facing) => HingeControl::Physical(facing),
        // Serve-sim refuses `orientation` over 0x10; its 0x07 gets no 0x90, only a 0x82.
        HingeCommand::Orientation(orientation) => {
            return message(ORIENTATION, &Orientation { orientation });
        }
    };
    message(
        HINGE,
        &HingeRequest {
            request_id,
            command,
        },
    )
}

const HINGE_TIMEOUT: Duration = Duration::from_secs(5);
const FIRST_HINGE_TIMEOUT: Duration = Duration::from_secs(15);
const HINGE_TIMED_OUT: &str = "Device control timed out. Its position is unknown.";
const DISCONNECTED: &str = "Device is disconnected.";
const REPLACED: &str = "Replaced by a newer hinge command.";

fn valid_hinge(command: HingeCommand) -> Result<(), String> {
    match command {
        HingeCommand::Angle(angle) if !(0.0..=180.0).contains(&angle) => {
            Err("Hinge angle must be a number from 0 to 180 degrees.".into())
        }
        _ => Ok(()),
    }
}

type HingeRequestOut = (u64, HingeCommand);

fn fail(waiting: Vec<HingeReply>, error: &str) {
    for reply in waiting {
        let _ = reply.send(Err(error.to_string()));
    }
}

#[derive(Default)]
struct HingeQueue {
    last_id: u64,
    warm: bool,
    active: Option<(u64, HingeCommand, Vec<HingeReply>)>,
    queued: VecDeque<(HingeCommand, Vec<HingeReply>)>,
}

impl HingeQueue {
    fn enqueue(&mut self, command: HingeCommand, reply: HingeReply) -> Option<HingeRequestOut> {
        match (command, self.queued.back_mut()) {
            (HingeCommand::Angle(_), Some((queued @ HingeCommand::Angle(_), waiting))) => {
                *queued = command;
                waiting.push(reply);
            }
            (HingeCommand::Pose(_), Some((HingeCommand::Angle(_), _))) => {
                if let Some((_, replaced)) = self.queued.pop_back() {
                    fail(replaced, REPLACED);
                }
                self.queued.push_back((command, vec![reply]));
            }
            _ => self.queued.push_back((command, vec![reply])),
        }
        self.drain()
    }

    fn drain(&mut self) -> Option<HingeRequestOut> {
        if self.active.is_some() {
            return None;
        }
        let (command, waiting) = self.queued.pop_front()?;
        self.last_id += 1;
        self.active = Some((self.last_id, command, waiting));
        Some((self.last_id, command))
    }

    fn receive(&mut self, id: u64, result: Result<(), String>) -> Option<HingeRequestOut> {
        if self.active.as_ref().map(|(active, ..)| *active) != Some(id) {
            return None;
        }
        let (_, _, waiting) = self.active.take()?;
        self.warm = true;
        for reply in waiting {
            let _ = reply.send(result.clone());
        }
        match result {
            Ok(()) => self.drain(),
            Err(error) => {
                self.clear(&error);
                None
            }
        }
    }

    fn screen_changed(&mut self, orientation: DeviceOrientation) -> Option<HingeRequestOut> {
        match self.active {
            Some((id, HingeCommand::Orientation(wanted), _)) if wanted == orientation => {
                self.receive(id, Ok(()))
            }
            _ => None,
        }
    }

    fn timed_out(&mut self, id: u64) -> Option<HingeRequestOut> {
        if self.active.as_ref().map(|(active, ..)| *active) != Some(id) {
            return None;
        }
        let (_, _, waiting) = self.active.take()?;
        fail(waiting, HINGE_TIMED_OUT);
        self.drain()
    }

    fn clear(&mut self, error: &str) {
        if let Some((_, _, waiting)) = self.active.take() {
            fail(waiting, error);
        }
        for (_, waiting) in self.queued.drain(..) {
            fail(waiting, error);
        }
    }
}

type SharedQueue = Arc<Mutex<HingeQueue>>;

fn lock(queue: &SharedQueue) -> std::sync::MutexGuard<'_, HingeQueue> {
    queue.lock().unwrap_or_else(PoisonError::into_inner)
}

fn send_hinge(
    queue: &SharedQueue,
    held: &mut HingeQueue,
    tx: &mpsc::UnboundedSender<Message>,
    request: Option<HingeRequestOut>,
) {
    let Some((id, command)) = request else {
        return;
    };
    if tx
        .send(Message::Binary(hinge_request(id, command)))
        .is_err()
    {
        held.clear(DISCONNECTED);
        return;
    }
    // A live iPhone Duo answered its first hinge command after boot in up to 6.2 s.
    let budget = match held.warm {
        true => HINGE_TIMEOUT,
        false => FIRST_HINGE_TIMEOUT,
    };
    let (queue, tx) = (Arc::downgrade(queue), tx.clone());
    tokio::spawn(async move {
        tokio::time::sleep(budget).await;
        if let Some(queue) = queue.upgrade() {
            let mut held = lock(&queue);
            let next = held.timed_out(id);
            send_hinge(&queue, &mut held, &tx, next);
        }
    });
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControlReply {
    request_id: u64,
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

fn control_reply(bytes: &[u8]) -> Option<(u64, Result<(), String>)> {
    let (&tag, body) = bytes.split_first()?;
    let reply: ControlReply = (tag == CONTROL_REPLY)
        .then(|| serde_json::from_slice(body).ok())
        .flatten()?;
    let result = match reply.ok {
        true => Ok(()),
        false => Err(reply
            .error
            .unwrap_or_else(|| "Device control failed. Its position is unknown.".into())),
    };
    Some((reply.request_id, result))
}

pub(crate) const UNSUPPORTED_INPUT: &str = "This device does not accept that input.";

type HingeReply = oneshot::Sender<Result<(), String>>;

pub(crate) struct InputSocket {
    tx: mpsc::UnboundedSender<Message>,
    live: Arc<AtomicBool>,
    encode: fn(&DeviceInput) -> Option<Message>,
    hinges: Option<SharedQueue>,
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
        let hinges: SharedQueue = Arc::default();
        let (waiting, hinge_tx) = (hinges.clone(), tx.clone());
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
                                                let orientation = screen.orientation;
                                                on_screen(screen);
                                                let mut held = lock(&waiting);
                                                let next = held.screen_changed(orientation);
                                                send_hinge(&waiting, &mut held, &hinge_tx, next);
                                            } else if let Some((id, result)) = control_reply(&bytes) {
                                                let mut held = lock(&waiting);
                                                let next = held.receive(id, result);
                                                send_hinge(&waiting, &mut held, &hinge_tx, next);
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
                            let mut held = lock(&waiting);
                            held.warm = false;
                            held.clear(DISCONNECTED);
                            drop(held);
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
            hinges: Some(hinges),
            task: Some(task),
        }
    }

    pub(crate) fn semu(tx: mpsc::UnboundedSender<Message>, live: Arc<AtomicBool>) -> Self {
        Self {
            tx,
            live,
            encode: |input| encode_semu(input).map(Message::Text),
            hinges: None,
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

    pub(crate) fn hinge(
        &self,
        command: HingeCommand,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        let hinges = self.hinges.as_ref().ok_or(UNSUPPORTED_INPUT)?;
        valid_hinge(command)?;
        let (reply, receiver) = oneshot::channel();
        let mut held = lock(hinges);
        // Checked under the queue lock: the reader drops `live` before it clears the queue.
        if !self.live.load(Ordering::SeqCst) {
            return Err("Input disconnected, reconnecting...".into());
        }
        let request = held.enqueue(command, reply);
        send_hinge(hinges, &mut held, &self.tx, request);
        Ok(receiver)
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
    fn hinge_commands_go_out_as_tag_0x10_or_0x07_and_replies_come_back_as_tag_0x90() {
        let tagged = |tag: u8, json: &str| [&[tag], json.as_bytes()].concat();
        assert_eq!(
            hinge_request(7, HingeCommand::Pose(HingePose::Closed)),
            tagged(
                0x10,
                r#"{"requestId":7,"command":{"control":"pose","value":"closed"}}"#
            )
        );
        for (pose, wire) in [
            (HingePose::Book, "book"),
            (HingePose::Open, "open"),
            (HingePose::Laptop, "laptop"),
            (HingePose::Tent, "tent"),
        ] {
            let bytes = hinge_request(1, HingeCommand::Pose(pose));
            let body: serde_json::Value = serde_json::from_slice(&bytes[1..]).unwrap();
            assert_eq!(body["command"]["value"], wire);
        }
        let cases = [
            (
                HingeCommand::Angle(45.5),
                tagged(
                    0x10,
                    r#"{"requestId":2,"command":{"control":"angle","value":45.5}}"#,
                ),
            ),
            (
                HingeCommand::Angle(45.0),
                tagged(
                    0x10,
                    r#"{"requestId":2,"command":{"control":"angle","value":45.0}}"#,
                ),
            ),
            (
                HingeCommand::Physical(DeviceFacing::FaceDown),
                tagged(
                    0x10,
                    r#"{"requestId":2,"command":{"control":"physical","value":"facedown"}}"#,
                ),
            ),
            (
                HingeCommand::Physical(DeviceFacing::FaceUp),
                tagged(
                    0x10,
                    r#"{"requestId":2,"command":{"control":"physical","value":"faceup"}}"#,
                ),
            ),
            (
                HingeCommand::Orientation(DeviceOrientation::LandscapeLeft),
                tagged(0x07, r#"{"orientation":"landscape_left"}"#),
            ),
        ];
        for (command, wire) in cases {
            assert_eq!(hinge_request(2, command), wire, "{command:?}");
        }
        assert_eq!(
            control_reply(&tagged(0x90, r#"{"requestId":1,"ok":true}"#)),
            Some((1, Ok(())))
        );
        assert_eq!(
            control_reply(&tagged(
                0x90,
                r#"{"requestId":6,"ok":false,"error":"Invalid hinge control request."}"#
            )),
            Some((6, Err("Invalid hinge control request.".into())))
        );
        assert_eq!(
            control_reply(&tagged(0x90, r#"{"requestId":2,"ok":false}"#)),
            Some((
                2,
                Err("Device control failed. Its position is unknown.".into())
            ))
        );
        assert_eq!(
            control_reply(&tagged(0x82, r#"{"requestId":1,"ok":true}"#)),
            None
        );
        let (tx, _rx) = mpsc::unbounded_channel();
        let semu = InputSocket::semu(tx, Arc::new(AtomicBool::new(true)));
        assert_eq!(
            semu.hinge(HingeCommand::Pose(HingePose::Open))
                .err()
                .as_deref(),
            Some(UNSUPPORTED_INPUT)
        );
        let (tx, _rx) = mpsc::unbounded_channel();
        let hinges: SharedQueue = Arc::default();
        let mut closed = InputSocket::semu(tx, Arc::new(AtomicBool::new(false)));
        closed.hinges = Some(hinges.clone());
        assert!(closed.hinge(HingeCommand::Pose(HingePose::Open)).is_err());
        let held = lock(&hinges);
        assert!(
            held.active.is_none() && held.queued.is_empty(),
            "a request on a closed socket leaves no waiter behind"
        );
    }

    #[test]
    fn a_hinge_angle_outside_0_to_180_degrees_never_leaves() {
        for angle in [0.0, 45.5, 180.0] {
            assert_eq!(valid_hinge(HingeCommand::Angle(angle)), Ok(()), "{angle}");
        }
        for angle in [
            -0.5,
            180.5,
            200.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert_eq!(
                valid_hinge(HingeCommand::Angle(angle)),
                Err("Hinge angle must be a number from 0 to 180 degrees.".into()),
                "{angle}"
            );
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut socket = InputSocket::semu(tx, Arc::new(AtomicBool::new(true)));
        socket.hinges = Some(Arc::default());
        assert!(socket.hinge(HingeCommand::Angle(200.0)).is_err());
        assert!(rx.try_recv().is_err(), "nothing was sent");
    }

    fn waiter() -> (HingeReply, oneshot::Receiver<Result<(), String>>) {
        oneshot::channel()
    }

    fn angle(value: f64) -> HingeCommand {
        HingeCommand::Angle(value)
    }

    #[tokio::test]
    async fn a_failed_send_fails_its_request_including_a_disconnect_while_draining_queued_motion() {
        let shared: SharedQueue = Arc::default();
        let (dead, gone) = mpsc::unbounded_channel();
        drop(gone);
        let (live, mut sent) = mpsc::unbounded_channel();
        let mut held = lock(&shared);
        let (reply, mut forty) = waiter();
        let request = held.enqueue(angle(40.0), reply);
        assert_eq!(request, Some((1, angle(40.0))));
        send_hinge(&shared, &mut held, &dead, request);
        assert_eq!(forty.try_recv(), Ok(Err(DISCONNECTED.into())));
        let (reply, mut book) = waiter();
        let request = held.enqueue(HingeCommand::Pose(HingePose::Book), reply);
        send_hinge(&shared, &mut held, &live, request);
        assert_eq!(
            sent.try_recv().unwrap(),
            Message::Binary(hinge_request(2, HingeCommand::Pose(HingePose::Book)))
        );
        let (reply, mut sixty) = waiter();
        assert_eq!(held.enqueue(angle(60.0), reply), None);
        let request = held.receive(2, Ok(()));
        assert_eq!(request, Some((3, angle(60.0))));
        send_hinge(&shared, &mut held, &dead, request);
        assert_eq!(book.try_recv(), Ok(Ok(())));
        assert_eq!(sixty.try_recv(), Ok(Err(DISCONNECTED.into())));
        assert!(held.active.is_none() && held.queued.is_empty());
    }

    #[test]
    fn hinge_edits_coalesce_behind_replies_and_a_preset_replaces_queued_edits() {
        let mut queue = HingeQueue::default();
        let (reply, mut forty) = waiter();
        assert_eq!(queue.enqueue(angle(40.0), reply), Some((1, angle(40.0))));
        let (reply, mut fifty) = waiter();
        assert_eq!(queue.enqueue(angle(50.0), reply), None);
        let (reply, mut sixty) = waiter();
        assert_eq!(queue.enqueue(angle(60.0), reply), None);
        assert_eq!(queue.receive(1, Ok(())), Some((2, angle(60.0))));
        assert_eq!(forty.try_recv(), Ok(Ok(())));
        assert!(
            fifty.try_recv().is_err(),
            "coalesced into 60, still waiting"
        );
        let (reply, mut hundred) = waiter();
        assert_eq!(queue.enqueue(angle(100.0), reply), None);
        let (reply, mut tent) = waiter();
        let preset = HingeCommand::Pose(HingePose::Tent);
        assert_eq!(queue.enqueue(preset, reply), None);
        assert_eq!(hundred.try_recv(), Ok(Err(REPLACED.into())));
        assert_eq!(queue.receive(2, Ok(())), Some((3, preset)));
        assert_eq!(fifty.try_recv(), Ok(Ok(())));
        assert_eq!(sixty.try_recv(), Ok(Ok(())));
        assert_eq!(queue.receive(3, Ok(())), None);
        assert_eq!(tent.try_recv(), Ok(Ok(())));
        assert!(queue.active.is_none() && queue.queued.is_empty());
    }

    #[test]
    fn only_angles_coalesce_a_pose_replaces_a_queued_angle_and_other_commands_queue_in_order() {
        let mut queue = HingeQueue::default();
        let down = HingeCommand::Physical(DeviceFacing::FaceDown);
        let up = HingeCommand::Physical(DeviceFacing::FaceUp);
        let tent = HingeCommand::Pose(HingePose::Tent);
        let book = HingeCommand::Pose(HingePose::Book);
        let turn = HingeCommand::Orientation(DeviceOrientation::LandscapeLeft);
        let mut waiting = Vec::new();
        let (reply, receiver) = waiter();
        assert_eq!(queue.enqueue(angle(40.0), reply), Some((1, angle(40.0))));
        waiting.push(receiver);
        for command in [down, angle(50.0)] {
            let (reply, receiver) = waiter();
            assert_eq!(queue.enqueue(command, reply), None);
            waiting.push(receiver);
        }
        let (reply, mut sixty) = waiter();
        assert_eq!(queue.enqueue(angle(60.0), reply), None);
        let (reply, receiver) = waiter();
        assert_eq!(queue.enqueue(tent, reply), None);
        let mut fifty = waiting.pop().unwrap();
        assert_eq!(fifty.try_recv(), Ok(Err(REPLACED.into())));
        assert_eq!(sixty.try_recv(), Ok(Err(REPLACED.into())));
        waiting.push(receiver);
        for command in [turn, book, angle(70.0), up] {
            let (reply, receiver) = waiter();
            assert_eq!(queue.enqueue(command, reply), None);
            waiting.push(receiver);
        }
        assert_eq!(queue.receive(1, Ok(())), Some((2, down)));
        assert_eq!(queue.receive(2, Ok(())), Some((3, tent)));
        assert_eq!(queue.receive(3, Ok(())), Some((4, turn)));
        assert_eq!(
            queue.screen_changed(DeviceOrientation::LandscapeLeft),
            Some((5, book))
        );
        assert_eq!(queue.receive(5, Ok(())), Some((6, angle(70.0))));
        assert_eq!(queue.receive(6, Ok(())), Some((7, up)));
        assert_eq!(queue.receive(7, Ok(())), None);
        for mut receiver in waiting {
            assert_eq!(receiver.try_recv(), Ok(Ok(())));
        }
    }

    #[test]
    fn a_timeout_sends_the_next_command_and_failure_or_disconnect_drops_the_queue() {
        let mut queue = HingeQueue::default();
        let (reply, mut ninety) = waiter();
        assert_eq!(queue.enqueue(angle(90.0), reply), Some((1, angle(90.0))));
        let (reply, mut hundred) = waiter();
        assert_eq!(queue.enqueue(angle(100.0), reply), None);
        assert_eq!(queue.timed_out(1), Some((2, angle(100.0))));
        assert_eq!(ninety.try_recv(), Ok(Err(HINGE_TIMED_OUT.into())));
        assert!(hundred.try_recv().is_err(), "sent after the timeout");
        assert_eq!(queue.receive(1, Ok(())), None, "a late reply");
        assert_eq!(queue.timed_out(1), None);
        assert!(hundred.try_recv().is_err(), "still pending");
        let (reply, mut open) = waiter();
        assert_eq!(
            queue.enqueue(HingeCommand::Pose(HingePose::Open), reply),
            None
        );
        let (reply, mut one_thirty) = waiter();
        assert_eq!(queue.enqueue(angle(130.0), reply), None);
        assert_eq!(queue.receive(2, Err("native refused".into())), None);
        for receiver in [&mut hundred, &mut open, &mut one_thirty] {
            assert_eq!(receiver.try_recv(), Ok(Err("native refused".into())));
        }
        let (reply, mut book) = waiter();
        let request = queue.enqueue(HingeCommand::Pose(HingePose::Book), reply);
        assert_eq!(request, Some((3, HingeCommand::Pose(HingePose::Book))));
        let (reply, mut closed) = waiter();
        assert_eq!(
            queue.enqueue(HingeCommand::Pose(HingePose::Closed), reply),
            None
        );
        queue.clear(DISCONNECTED);
        assert_eq!(queue.receive(3, Ok(())), None);
        assert_eq!(book.try_recv(), Ok(Err(DISCONNECTED.into())));
        assert_eq!(closed.try_recv(), Ok(Err(DISCONNECTED.into())));
        assert!(valid_hinge(angle(f64::INFINITY)).is_err());
    }

    #[test]
    fn an_orientation_is_acknowledged_by_the_first_screen_config_that_shows_it() {
        let mut queue = HingeQueue::default();
        assert_eq!(queue.screen_changed(DeviceOrientation::Portrait), None);
        let turn = HingeCommand::Orientation(DeviceOrientation::LandscapeLeft);
        let (reply, mut turned) = waiter();
        assert_eq!(queue.enqueue(turn, reply), Some((1, turn)));
        let (reply, mut folded) = waiter();
        assert_eq!(queue.enqueue(angle(45.0), reply), None);
        assert_eq!(
            queue.screen_changed(DeviceOrientation::Portrait),
            None,
            "the hub's first config after 0x07 can still show the old orientation"
        );
        assert!(turned.try_recv().is_err());
        assert_eq!(
            queue.screen_changed(DeviceOrientation::LandscapeLeft),
            Some((2, angle(45.0)))
        );
        assert_eq!(turned.try_recv(), Ok(Ok(())));
        assert_eq!(
            queue.screen_changed(DeviceOrientation::LandscapeLeft),
            None,
            "an angle waits for its 0x90"
        );
        assert!(folded.try_recv().is_err());
        assert_eq!(queue.receive(2, Ok(())), None);
        assert_eq!(folded.try_recv(), Ok(Ok(())));
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_request_waits_longer_and_a_timeout_sends_the_queued_request() {
        let shared: SharedQueue = Arc::default();
        let (tx, mut sent) = mpsc::unbounded_channel();
        let send = |command, reply| {
            let mut held = lock(&shared);
            let request = held.enqueue(command, reply);
            send_hinge(&shared, &mut held, &tx, request);
        };
        let (reply, mut cold) = waiter();
        send(angle(45.0), reply);
        assert_eq!(
            sent.try_recv(),
            Ok(Message::Binary(hinge_request(1, angle(45.0))))
        );
        tokio::time::sleep(FIRST_HINGE_TIMEOUT - Duration::from_millis(1)).await;
        assert!(cold.try_recv().is_err(), "not yet");
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert_eq!(cold.try_recv(), Ok(Err(HINGE_TIMED_OUT.into())));
        let (reply, mut first) = waiter();
        send(angle(50.0), reply);
        tokio::time::sleep(HINGE_TIMEOUT + Duration::from_millis(1)).await;
        assert!(first.try_recv().is_err(), "no reply yet, so still cold");
        assert_eq!(lock(&shared).receive(2, Ok(())), None);
        assert_eq!(first.try_recv(), Ok(Ok(())));
        let (reply, mut second) = waiter();
        send(angle(60.0), reply);
        let down = HingeCommand::Physical(DeviceFacing::FaceDown);
        let (reply, mut third) = waiter();
        send(down, reply);
        tokio::time::sleep(HINGE_TIMEOUT - Duration::from_millis(1)).await;
        assert!(second.try_recv().is_err(), "not yet");
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert_eq!(second.try_recv(), Ok(Err(HINGE_TIMED_OUT.into())));
        assert!(third.try_recv().is_err());
        let mut out = Vec::new();
        while let Ok(message) = sent.try_recv() {
            out.push(message);
        }
        assert_eq!(
            out,
            [
                Message::Binary(hinge_request(2, angle(50.0))),
                Message::Binary(hinge_request(3, angle(60.0))),
                Message::Binary(hinge_request(4, down)),
            ]
        );
    }

    #[tokio::test]
    async fn hinge_replies_resolve_their_own_request_and_a_closed_socket_fails_the_rest() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/", listener.local_addr().unwrap());
        let (seen_tx, mut seen) = mpsc::unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let mut hinges = 0;
            while let Some(Ok(Message::Binary(bytes))) = socket.next().await {
                let _ = seen_tx.send(bytes.to_vec());
                if bytes[0] == 0x07 {
                    let config = br#"{"width":2006,"height":2852,"orientation":"landscape_left"}"#;
                    let out = [&[0x82], &config[..]].concat();
                    socket.send(Message::Binary(out)).await.unwrap();
                    continue;
                }
                if bytes[0] != 0x10 {
                    continue;
                }
                hinges += 1;
                let id =
                    serde_json::from_slice::<serde_json::Value>(&bytes[1..]).unwrap()["requestId"]
                        .as_u64()
                        .unwrap();
                let reply = match hinges {
                    1 => format!(r#"{{"requestId":{id},"ok":true}}"#),
                    2 => format!(
                        r#"{{"requestId":{id},"ok":false,"error":"Simulator could not change the device pose."}}"#
                    ),
                    _ => return,
                };
                let mut out = vec![0x90];
                out.extend(reply.into_bytes());
                socket.send(Message::Binary(out.into())).await.unwrap();
            }
        });
        let socket = InputSocket::connect(url, |_| {}, |_| {});
        tokio::time::timeout(Duration::from_secs(5), async {
            while !socket.live.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let closed = HingeCommand::Pose(HingePose::Closed);
        let tent = HingeCommand::Pose(HingePose::Tent);
        let turn = HingeCommand::Orientation(DeviceOrientation::LandscapeLeft);
        assert_eq!(socket.hinge(closed).unwrap().await, Ok(Ok(())));
        assert_eq!(
            socket.hinge(tent).unwrap().await,
            Ok(Err("Simulator could not change the device pose.".into()))
        );
        assert_eq!(socket.hinge(turn).unwrap().await, Ok(Ok(())));
        let dropped = socket.hinge(HingeCommand::Pose(HingePose::Open)).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), dropped)
                .await
                .unwrap(),
            Ok(Err(DISCONNECTED.into())),
            "a closed socket fails the waiting request"
        );
        assert_eq!(seen.recv().await.unwrap(), hardware_keyboard_off());
        assert_eq!(seen.recv().await.unwrap(), hinge_request(1, closed));
        assert_eq!(seen.recv().await.unwrap(), hinge_request(2, tent));
        assert_eq!(seen.recv().await.unwrap(), hinge_request(3, turn));
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
