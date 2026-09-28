// tungstenite's `accept_hdr_async` callback fixes the Err type as a full `Response`.
#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::handshake::server::{
    Request as WsRequest, Response as WsResponse,
};

use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_proto::HarnessId;
use zeron_proto::computer::{ComputerHostMethod, ComputerHostStreamItem, ComputerSettings};
use zeron_rpc::{
    DeviceFrameHeader, LinkCache, LinkCacheConfig, RpcClient, RpcSubscription, StaticToken,
    decode_device_frame, encode_device_frame, methods,
};

#[derive(Default)]
struct RelayState {
    host: Option<mpsc::UnboundedSender<Vec<u8>>>,
    clients: HashMap<String, mpsc::UnboundedSender<Vec<u8>>>,
}

async fn fake_device_room() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind relay");
    let url = format!(
        "http://127.0.0.1:{}",
        listener.local_addr().expect("addr").port()
    );
    let state = Arc::new(Mutex::new(RelayState::default()));
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let state = state.clone();
            tokio::spawn(async move {
                let mut uri = String::new();
                let Ok(ws) = tokio_tungstenite::accept_hdr_async(
                    stream,
                    |req: &WsRequest, res: WsResponse| {
                        uri = req.uri().to_string();
                        Ok(res)
                    },
                )
                .await
                else {
                    return;
                };
                let query = uri.split_once('?').map(|(_, q)| q).unwrap_or("");
                let is_host = query.contains("role=host");
                let conn_id = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("connId="))
                    .unwrap_or("anon")
                    .to_string();
                let (mut sink, mut ws_stream) = ws.split();
                let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
                {
                    let mut st = state.lock().expect("lock");
                    if is_host {
                        st.host = Some(tx);
                    } else {
                        st.clients.insert(conn_id.clone(), tx);
                    }
                }
                let writer = tokio::spawn(async move {
                    while let Some(bytes) = rx.recv().await {
                        if sink.send(WsMessage::Binary(bytes)).await.is_err() {
                            break;
                        }
                    }
                });
                while let Some(Ok(message)) = ws_stream.next().await {
                    let WsMessage::Binary(bytes) = message else {
                        continue;
                    };
                    let Ok((header, payload)) = decode_device_frame(&bytes) else {
                        break;
                    };
                    let st = state.lock().expect("lock");
                    if is_host {
                        let Some(to) = header.to else { continue };
                        if let Some(client) = st.clients.get(&to) {
                            let stripped = DeviceFrameHeader::new(header.s, header.k);
                            let _ = client
                                .send(encode_device_frame(&stripped, &payload).expect("encode"));
                        }
                    } else if let Some(host) = &st.host {
                        let mut routed = DeviceFrameHeader::new(header.s, header.k);
                        routed.from = Some(conn_id.clone());
                        let _ = host.send(encode_device_frame(&routed, &payload).expect("encode"));
                    }
                }
                writer.abort();
            });
        }
    });
    (url, task)
}

fn assemble(dir: &std::path::Path, device_id: &str) -> EngineCore {
    std::fs::create_dir_all(dir).expect("create data dir");
    std::fs::write(dir.join("device-id"), device_id).expect("write device id");
    EngineCore::assemble(dir, Arc::new(HarnessRegistry::new()), HarnessId::Mock, None)
        .expect("engine assembles")
}

async fn watch_host(client: &RpcClient) -> Result<RpcSubscription, zeron_rpc::RpcError> {
    client
        .subscribe_checked(
            methods::WATCH_COMPUTER_HOST_REQUESTS,
            json!({ "bundleId": "sh.zeron.test" }),
        )
        .await
}

async fn next_item(stream: &mut RpcSubscription) -> Option<ComputerHostStreamItem> {
    let value = tokio::time::timeout(Duration::from_secs(10), stream.recv())
        .await
        .expect("host item in time")?;
    Some(serde_json::from_value(value).expect("host stream item"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_loopback_host_registers_answers_and_frees_the_slot_on_drop() {
    let dir = tempfile::tempdir().expect("tempdir");
    let core = assemble(dir.path(), "device-mac");
    let client = zeron_rpc::memory_client(core.rpc_service());

    let mut first = watch_host(&client).await.expect("first host registers");
    let Some(ComputerHostStreamItem::Registered { host_token }) = next_item(&mut first).await
    else {
        panic!("the first item carries the host token");
    };

    let mut second = watch_host(&client).await.expect("refusal is a stream item");
    assert_eq!(
        next_item(&mut second).await,
        Some(ComputerHostStreamItem::Refused {
            retry_after_ms: 2_000
        })
    );
    assert_eq!(next_item(&mut second).await, None);

    let Some(ComputerHostStreamItem::Request(probe)) = next_item(&mut first).await else {
        panic!("the service rebuild probes the new host");
    };
    assert_eq!(probe.call.method, ComputerHostMethod::Probe);
    assert_eq!(probe.host_token, host_token);
    let forged = client
        .call(
            methods::RESPOND_COMPUTER_HOST_REQUEST,
            json!({
                "hostToken": "forged",
                "correlationId": probe.call.correlation_id,
                "result": { "ok": true },
            }),
        )
        .await;
    assert!(forged.is_err(), "a reply without the live token is refused");
    client
        .call(
            methods::RESPOND_COMPUTER_HOST_REQUEST,
            json!({
                "hostToken": host_token,
                "correlationId": probe.call.correlation_id,
                "result": { "ok": true },
            }),
        )
        .await
        .expect("host answers its request");
    client
        .call(
            methods::RESPOND_COMPUTER_HOST_REQUEST,
            json!({
                "hostToken": host_token,
                "correlationId": "unknown",
                "result": { "ok": true },
            }),
        )
        .await
        .expect("a late reply for an unknown id is a no-op");

    let status = async {
        loop {
            match client
                .call_as::<zeron_proto::computer::ComputerHostStatus>(
                    methods::GET_COMPUTER_STATUS,
                    json!({}),
                )
                .await
            {
                Ok(status) => break status,
                Err(error) if error.to_string().contains("still starting") => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => panic!("status read failed: {error}"),
            }
        }
    };
    let answer_probe = async {
        let Some(ComputerHostStreamItem::Request(probe)) = next_item(&mut first).await else {
            panic!("the status read probes the host");
        };
        assert_eq!(probe.call.method, ComputerHostMethod::Probe);
        client
            .call(
                methods::RESPOND_COMPUTER_HOST_REQUEST,
                json!({
                    "hostToken": host_token,
                    "correlationId": probe.call.correlation_id,
                    "result": { "ok": true },
                }),
            )
            .await
            .expect("host answers the status probe");
    };
    let (status, ()) = tokio::join!(status, answer_probe);
    assert_eq!(status.host_bundle_id.as_deref(), Some("sh.zeron.test"));

    drop(first);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let mut next = watch_host(&client).await.expect("watch");
        match next_item(&mut next).await {
            Some(ComputerHostStreamItem::Registered { .. }) => break,
            Some(ComputerHostStreamItem::Refused { .. }) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the dropped host never freed the slot"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            other => panic!("unexpected host item {other:?}"),
        }
    }
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computer_settings_round_trip_over_rpc_and_default_off() {
    let dir = tempfile::tempdir().expect("tempdir");
    let core = assemble(dir.path(), "device-mac");
    let client = zeron_rpc::memory_client(core.rpc_service());
    let settings: ComputerSettings = client
        .call_as(methods::GET_COMPUTER_SETTINGS, json!({}))
        .await
        .expect("get settings");
    assert!(!settings.computer_control_enabled);
    client
        .call(
            methods::SET_COMPUTER_SETTINGS,
            json!({ "computerControlEnabled": true, "computerPreviewSize": "large" }),
        )
        .await
        .expect("set settings");
    let settings: ComputerSettings = client
        .call_as(methods::GET_COMPUTER_SETTINGS, json!({}))
        .await
        .expect("get settings");
    assert!(settings.computer_control_enabled);
    let stored: ComputerSettings = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("computer-settings.json")).expect("persisted"),
    )
    .expect("settings file");
    assert_eq!(stored, settings);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_clients_are_refused_the_host_and_computer_writes() {
    let (relay_url, _relay) = fake_device_room().await;
    let dirs = tempfile::tempdir().expect("tempdir");
    let core_b = assemble(&dirs.path().join("b"), "device-b");
    let _host = core_b.start_host_relay(&relay_url);
    let local_b = zeron_rpc::memory_client(core_b.rpc_service());
    local_b
        .call(
            methods::MUTATE,
            json!({ "op": "createChat", "chatId": "chat-b", "deviceId": "device-b" }),
        )
        .await
        .expect("createChat on B");

    let core_a = assemble(&dirs.path().join("a"), "device-a");
    let mut link_config =
        LinkCacheConfig::new(relay_url.clone(), Arc::new(StaticToken("test-user".into())));
    link_config.probe_timeout = Duration::from_secs(5);
    core_a.set_links(LinkCache::new(link_config));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let (relay, settings) = loop {
        let attempt = async {
            let relay = core_a.dial_device("device-b").await.ok()?;
            let settings = relay
                .call_as::<ComputerSettings>(methods::GET_COMPUTER_SETTINGS, json!({}))
                .await
                .ok()?;
            Some((relay, settings))
        };
        if let Some(ready) = attempt.await {
            break ready;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "relay never came up"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert!(!settings.computer_control_enabled);

    let refusal = |method: &str| format!("{method} is only served to connections on this device");
    let error = match watch_host(&relay).await {
        Ok(_) => panic!("a relay client must never register as the desktop host"),
        Err(error) => error.to_string(),
    };
    assert_eq!(error, refusal(methods::WATCH_COMPUTER_HOST_REQUESTS));
    let refused = [
        (
            methods::RESPOND_COMPUTER_HOST_REQUEST,
            json!({ "hostToken": "t", "correlationId": "c", "result": { "ok": true } }),
        ),
        (
            methods::CANCEL_COMPUTER_ACTION,
            json!({ "chatId": "chat-b", "runSecret": "s", "requestId": 1 }),
        ),
        (
            methods::SET_COMPUTER_SETTINGS,
            json!({ "computerControlEnabled": true }),
        ),
        (
            methods::GET_COMPUTER_CATALOG,
            json!({ "chatId": "chat-b", "runSecret": "s" }),
        ),
        (
            methods::COMPUTER_ACTION,
            json!({
                "chatId": "chat-b",
                "runSecret": "s",
                "tool": "computer_click",
                "arguments": {},
                "requestId": 1,
            }),
        ),
        (methods::COMPUTER_EMERGENCY_STOP, json!({})),
    ];
    for (method, params) in refused {
        let error = relay
            .call(method, params)
            .await
            .expect_err("relay-refused method");
        assert_eq!(error.to_string(), refusal(method));
    }
    let events = tokio::time::timeout(
        Duration::from_secs(5),
        relay.subscribe_checked(methods::WATCH_COMPUTER_EVENTS, json!({})),
    )
    .await
    .expect("the event watch is answered at once");
    match events {
        Ok(_) => panic!("a relay client must never watch the Computer events"),
        Err(error) => assert_eq!(error.to_string(), refusal(methods::WATCH_COMPUTER_EVENTS)),
    }
    let settings: ComputerSettings = local_b
        .call_as(methods::GET_COMPUTER_SETTINGS, json!({}))
        .await
        .expect("local read");
    assert!(
        !settings.computer_control_enabled,
        "the relay write never landed"
    );

    relay
        .call(
            methods::QUEUE_COMMAND,
            json!({
                "chatId": "chat-b",
                "command": {
                    "kind": "respondInput",
                    "requestId": "consent-1",
                    "answers": [],
                },
            }),
        )
        .await
        .expect("a consent answer still arrives over the relay");
    relay
        .call(methods::GET_COMPUTER_AUDIT_HISTORY, json!({}))
        .await
        .expect("audit history stays relay-readable");
    let status: zeron_proto::computer::ComputerHostStatus = relay
        .call_as(methods::GET_COMPUTER_STATUS, json!({}))
        .await
        .expect("status stays relay-readable");
    assert_eq!(status.host_bundle_id, None);
    assert_eq!(status.driver_native_revision, None);
    let frame = relay
        .call(
            methods::GET_COMPUTER_STILL_FRAME,
            json!({ "chatId": "chat-b" }),
        )
        .await
        .expect("still frames stay relay-readable");
    assert_eq!(frame, json!({ "frame": null }));
    core_a.shutdown().await;
    core_b.shutdown().await;
}

async fn stop_until_the_event_arrives(
    core: &EngineCore,
    client: &RpcClient,
    events: &mut mpsc::Receiver<serde_json::Value>,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no thread-state event"
        );
        let stopped = client
            .call(methods::COMPUTER_EMERGENCY_STOP, json!({}))
            .await
            .expect("emergency stop");
        assert_eq!(stopped, json!({ "stoppedChatIds": [] }));
        core.computer.stop_chat("probe-chat", 0);
        while let Ok(Some(value)) =
            tokio::time::timeout(Duration::from_millis(200), events.recv()).await
        {
            if value["type"] == "computer.thread-state"
                && value["state"]["threadId"] == "probe-chat"
            {
                return;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computer_events_keep_flowing_across_a_host_rebuild() {
    let dir = tempfile::tempdir().expect("tempdir");
    let core = assemble(dir.path(), "device-mac");
    let client = zeron_rpc::memory_client(core.rpc_service());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while core.computer.service().is_none() {
        assert!(tokio::time::Instant::now() < deadline, "no boot");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let mut events = client
        .subscribe(methods::WATCH_COMPUTER_EVENTS, json!({}))
        .await
        .expect("event stream");
    stop_until_the_event_arrives(&core, &client, &mut events).await;

    let mut host = watch_host(&client).await.expect("host registers");
    let Some(ComputerHostStreamItem::Registered { host_token }) = next_item(&mut host).await else {
        panic!("the first item carries the host token");
    };
    let answering = {
        let client = zeron_rpc::memory_client(core.rpc_service());
        tokio::spawn(async move {
            while let Some(ComputerHostStreamItem::Request(request)) = next_item(&mut host).await {
                let _ = client
                    .call(
                        methods::RESPOND_COMPUTER_HOST_REQUEST,
                        json!({
                            "hostToken": host_token,
                            "correlationId": request.call.correlation_id,
                            "result": { "ok": true },
                        }),
                    )
                    .await;
            }
        })
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !core
        .computer
        .service()
        .is_some_and(|service| service.supported)
    {
        assert!(tokio::time::Instant::now() < deadline, "no rebuild");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    stop_until_the_event_arrives(&core, &client, &mut events).await;
    core.shutdown().await;
    answering.abort();
}
