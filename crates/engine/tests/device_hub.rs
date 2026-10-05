use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use zeron_engine::device_hub::DEVICE_SUPPORT_OFF;
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_proto::HarnessId;
use zeron_proto::device_hub::{DeviceHostStatus, DeviceHubState};
use zeron_rpc::{RpcError, RpcReply, RpcService, methods};

fn device_dir(data: &std::path::Path) -> Vec<std::ffi::OsString> {
    std::fs::read_dir(data.join("device"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn list_hub_devices_is_refused_while_disabled_and_installs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let adapters = dir.path().join("adapters");
    // SAFETY: the only test in this binary; set before the engine reads it.
    unsafe { std::env::set_var("ZERON_ADAPTERS_DIR", &adapters) };
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let core = EngineCore::assemble(
        &data,
        Arc::new(HarnessRegistry::new()),
        HarnessId::Mock,
        None,
    )
    .unwrap();
    let rpc = core.rpc_service();

    let Ok(RpcReply::Stream(mut watch)) = rpc
        .handle(methods::WATCH_DEVICE_HUB, serde_json::json!({}))
        .await
    else {
        panic!("WatchDeviceHub streams");
    };
    let first: DeviceHubState = serde_json::from_value(
        tokio::time::timeout(Duration::from_secs(5), watch.next())
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first.host_status, DeviceHostStatus::Disabled);
    assert!(!first.settings.enabled);

    for (method, params) in [
        (methods::LIST_HUB_DEVICES, serde_json::json!({})),
        (
            methods::OPEN_HUB_DEVICE,
            serde_json::json!({"chatId": "c", "deviceId": "SIM", "platform": "ios"}),
        ),
        (
            methods::WATCH_HUB_DEVICE_FRAMES,
            serde_json::json!({"deviceId": "SIM"}),
        ),
    ] {
        match rpc.handle(method, params).await {
            Err(RpcError::Failed(message)) => assert_eq!(message, DEVICE_SUPPORT_OFF, "{method}"),
            Err(other) => panic!("{method}: {other}"),
            Ok(_) => panic!("{method} must be refused while disabled"),
        }
    }
    let input = rpc
        .handle(
            methods::SEND_HUB_DEVICE_INPUT,
            serde_json::json!({"deviceId": "SIM", "input": {"touch": {"phase": "begin", "x": 0.5, "y": 0.5}}}),
        )
        .await;
    assert!(input.is_err());

    assert!(!adapters.exists(), "no install was attempted");
    assert_eq!(device_dir(&data), ["bin"], "no hub was started");
    assert!(!data.join("device-hub-settings.json").exists());
    assert!(
        tokio::time::timeout(Duration::from_millis(200), watch.next())
            .await
            .is_err(),
        "refusals publish nothing"
    );

    let Ok(RpcReply::Value(inspected)) = rpc
        .handle(
            methods::LIST_HUB_DEVICES,
            serde_json::json!({"inspectOnly": true}),
        )
        .await
    else {
        panic!("inspectOnly answers while disabled");
    };
    let inspected: DeviceHubState = serde_json::from_value(inspected).unwrap();
    let tools = inspected.hosts[0].tools.clone().expect("versions are read");
    assert_eq!(tools.hub.required, "0.12.0");
    assert!(tools.hub.installed.is_empty() && tools.hub.running.is_none());
    assert_eq!(inspected.host_status, DeviceHostStatus::Disabled);
    assert!(!adapters.exists(), "inspectOnly installs nothing");
    assert_eq!(device_dir(&data), ["bin"], "inspectOnly starts nothing");
    core.shutdown().await;
}
