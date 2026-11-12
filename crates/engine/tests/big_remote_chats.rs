//! A reader reaching a very long chat on another device, through real engines
//! over a relay that behaves as the edge's does: it delivers each message as
//! one frame, and past 32 MiB in one message it closes the host's socket
//! (measured on workerd). A transcript used to be one message, which ended the
//! link: `WatchDocMessages: stream ended before the transcript arrived`.

#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::handshake::server::{
    Request as WsRequest, Response as WsResponse,
};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use zeron_doc::{
    MessagePart, MessageRole, MessageStatus, SessionMessageEntry, TranscriptFrame,
    TranscriptWindowInfo, apply_transcript_frame,
};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_proto::HarnessId;
use zeron_rpc::{
    DeviceFrameHeader, LinkCache, LinkCacheConfig, StaticToken, decode_device_frame,
    encode_device_frame,
};

/// What workerd does with a message from a socket, measured: past this it
/// closes that socket.
const ROOM_MESSAGE_CAP: usize = 32 << 20;

#[derive(Default)]
struct Room {
    host: Option<mpsc::UnboundedSender<Vec<u8>>>,
    clients: HashMap<String, mpsc::UnboundedSender<Vec<u8>>>,
}

#[derive(Default)]
struct Counters {
    largest_host_message: AtomicUsize,
    host_sockets: AtomicUsize,
    client_sockets: AtomicUsize,
}

async fn device_room(counters: Arc<Counters>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind relay");
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let room = Arc::new(Mutex::new(Room::default()));
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let room = room.clone();
            let counters = counters.clone();
            tokio::spawn(async move {
                let mut uri = String::new();
                let config = WebSocketConfig {
                    max_message_size: Some(1 << 30),
                    max_frame_size: Some(1 << 30),
                    ..Default::default()
                };
                let Ok(ws) = tokio_tungstenite::accept_hdr_async_with_config(
                    stream,
                    |req: &WsRequest, res: WsResponse| {
                        uri = req.uri().to_string();
                        Ok(res)
                    },
                    Some(config),
                )
                .await
                else {
                    return;
                };
                let query = uri.split_once('?').map(|(_, q)| q).unwrap_or("").to_owned();
                let is_host = query.contains("role=host");
                let conn = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("connId="))
                    .unwrap_or("anon")
                    .to_owned();
                let (mut sink, mut reader) = ws.split();
                let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
                if is_host {
                    counters.host_sockets.fetch_add(1, Ordering::SeqCst);
                    room.lock().unwrap().host = Some(tx);
                } else {
                    counters.client_sockets.fetch_add(1, Ordering::SeqCst);
                    room.lock().unwrap().clients.insert(conn.clone(), tx);
                }
                tokio::spawn(async move {
                    while let Some(bytes) = rx.recv().await {
                        if sink.send(WsMessage::Binary(bytes)).await.is_err() {
                            break;
                        }
                    }
                });
                while let Some(Ok(message)) = reader.next().await {
                    let WsMessage::Binary(bytes) = message else {
                        continue;
                    };
                    if is_host {
                        counters
                            .largest_host_message
                            .fetch_max(bytes.len(), Ordering::SeqCst);
                    }
                    if bytes.len() > ROOM_MESSAGE_CAP {
                        break;
                    }
                    let Ok((header, payload)) = decode_device_frame(&bytes) else {
                        break;
                    };
                    let room = room.lock().unwrap();
                    if is_host {
                        let Some(to) = header.to else { continue };
                        if let Some(client) = room.clients.get(&to) {
                            let plain = DeviceFrameHeader::new(header.s, header.k);
                            let _ = client.send(encode_device_frame(&plain, &payload).unwrap());
                        }
                    } else if let Some(host) = &room.host {
                        let mut routed = DeviceFrameHeader::new(header.s, header.k);
                        routed.from = Some(conn.clone());
                        let _ = host.send(encode_device_frame(&routed, &payload).unwrap());
                    }
                }
                if is_host {
                    room.lock().unwrap().host = None;
                } else {
                    room.lock().unwrap().clients.remove(&conn);
                }
            });
        }
    });
    url
}

fn assemble(dir: &std::path::Path, device: &str) -> EngineCore {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("device-id"), device).unwrap();
    EngineCore::assemble(dir, Arc::new(HarnessRegistry::new()), HarnessId::Mock, None)
        .expect("engine")
}

const CHAT: &str = "whale";
const MESSAGES: usize = 20_000;
/// 20,000 of these is 40 MiB of transcript: past the room's cap and any frame's.
const MESSAGE_BYTES: usize = 2 * 1024;

fn config() -> zeron_proto::ChatConfig {
    serde_json::from_value(
        json!({"harness": "mock", "model": null, "reasoning": null, "sandbox": "workspace-write"}),
    )
    .unwrap()
}

fn seed(core: &EngineCore) {
    let handle = core.doc_host.open(CHAT).expect("open chat");
    let text = "x".repeat(MESSAGE_BYTES);
    for i in 0..MESSAGES {
        handle
            .doc()
            .push_message(&SessionMessageEntry {
                id: format!("m-{i}"),
                role: if i % 2 == 0 {
                    MessageRole::User
                } else {
                    MessageRole::Assistant
                },
                parts: vec![MessagePart::Text {
                    id: "t".into(),
                    text: text.clone(),
                }],
                created_at: i as i64,
                device_id: "device-b".into(),
                status: Some(MessageStatus::Complete),
                continuation_of: None,
                from_chat_id: None,
                fork_import: false,
                answers_request_id: None,
                duration_ms: None,
            })
            .expect("seed message");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agents_reach_a_huge_chat_on_another_device() {
    let counters = Arc::new(Counters::default());
    let relay = device_room(counters.clone()).await;
    let dirs = tempfile::tempdir().unwrap();

    let host = assemble(&dirs.path().join("b"), "device-b");
    host.workspace
        .create_chat(CHAT, None, Some("device-b"), Some(config()), None)
        .unwrap();
    host.workspace.rename_chat(CHAT, "Whale").unwrap();
    seed(&host);
    let _relay_host = host.start_host_relay(&relay);

    // The reader knows the chat from the registry, not the doc: it holds none
    // of the transcript.
    let reader = assemble(&dirs.path().join("a"), "device-a");
    reader
        .workspace
        .create_chat(CHAT, None, Some("device-b"), Some(config()), None)
        .unwrap();
    reader.workspace.rename_chat(CHAT, "Whale").unwrap();
    let mut links = LinkCacheConfig::new(relay, Arc::new(StaticToken("user".into())));
    links.probe_timeout = Duration::from_secs(20);
    reader.set_links(LinkCache::new(links));

    let client = zeron_rpc::memory_client(reader.rpc_service());

    // A window of one asks for one message, however long the chat. The first
    // read opens the doc and builds the host's mirror; the second is what a
    // repeat costs.
    let read_one = || read(&client, Some(json!({"limit": 1})));
    let started = Instant::now();
    let (entries, info) = loop {
        match read_one().await {
            Ok(read) => break read,
            Err(error) => {
                assert!(
                    started.elapsed() < Duration::from_secs(60),
                    "relay never came up: {error}"
                );
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    };
    let cold = started.elapsed();
    let info = info.expect("the host applied the window");
    assert_eq!(info.total, MESSAGES);
    assert_eq!(info.start, MESSAGES - 1);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, format!("m-{}", MESSAGES - 1));
    let started = Instant::now();
    read_one().await.expect("a second read");
    let warm = started.elapsed();
    eprintln!("a window of 1 of {MESSAGES} messages: cold {cold:?}, warm {warm:?}");
    assert!(warm < Duration::from_secs(2), "took {warm:?}");
    let largest = counters.largest_host_message.load(Ordering::SeqCst);
    assert!(largest < 64 << 10, "{largest} bytes for one message");

    // And a full read of all of it: over 32 MiB, whole, on the same sockets.
    let hosts_before = counters.host_sockets.load(Ordering::SeqCst);
    let clients_before = counters.client_sockets.load(Ordering::SeqCst);
    let started = Instant::now();
    let (everything, _) = read(&client, None).await.expect("a full read");
    eprintln!(
        "full read of {} messages: {:?}",
        everything.len(),
        started.elapsed()
    );
    assert_eq!(everything.len(), MESSAGES);
    let largest = counters.largest_host_message.load(Ordering::SeqCst);
    assert!(
        largest <= 1 << 20,
        "a {largest}-byte message reached the relay"
    );
    assert_eq!(counters.host_sockets.load(Ordering::SeqCst), hosts_before);
    assert_eq!(
        counters.client_sockets.load(Ordering::SeqCst),
        clients_before
    );
}

async fn read(
    client: &zeron_rpc::RpcClient,
    window: Option<serde_json::Value>,
) -> Result<(Vec<SessionMessageEntry>, Option<TranscriptWindowInfo>), String> {
    let mut params = json!({"chatId": CHAT, "targetDeviceId": "device-b"});
    if let Some(window) = window {
        params["window"] = window;
    }
    let mut rx = client
        .subscribe_scoped(zeron_rpc::methods::WATCH_DOC_MESSAGES, params)
        .await
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    loop {
        let mut item = tokio::time::timeout(Duration::from_secs(60), rx.recv())
            .await
            .map_err(|_| "no transcript within 60s".to_owned())?
            .ok_or("stream ended before the transcript arrived")?;
        let info = item
            .get_mut("window")
            .map(serde_json::Value::take)
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| e.to_string())?;
        let is_reset = item.get("reset").is_some();
        let frame: TranscriptFrame = serde_json::from_value(item).map_err(|e| e.to_string())?;
        apply_transcript_frame(&mut entries, frame).map_err(|e| e.to_string())?;
        if is_reset {
            return Ok((entries, info));
        }
    }
}
