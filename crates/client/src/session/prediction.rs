use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use zeron_proto::prediction::ComposerPrediction;
use zeron_rpc::{RpcError, methods};

use super::SessionCore;

const RETRY: Duration = Duration::from_secs(2);

#[derive(serde::Deserialize)]
struct Frame {
    prediction: Option<ComposerPrediction>,
}

/// The engine predicts only while someone watches the chat, so the watch
/// lives exactly as long as the view is on screen.
pub(super) fn start(core: &Arc<SessionCore>) -> Option<CancellationToken> {
    let client = core.client().ok()?;
    if let Some(demo) = client.demo() {
        let prediction = demo.prediction(&core.chat_id);
        core.update_state(|st| st.prediction = prediction);
        return None;
    }
    let cancel = core.tasks.child_token();
    let token = cancel.clone();
    let weak = Arc::downgrade(core);
    crate::runtime::shared().spawn(async move {
        loop {
            let Some(core) = weak.upgrade() else { return };
            let Ok(client) = core.client() else { return };
            let chat_id = core.chat_id.clone();
            drop(core);
            let Some(device_id) = client.workspace.chat(&chat_id).map(|c| c.device_id) else {
                return;
            };
            let Some(live) = client.live() else { return };
            let subscription = tokio::select! {
                _ = cancel.cancelled() => return,
                result = live.relay.subscribe(
                    &device_id,
                    methods::WATCH_COMPOSER_PREDICTION,
                    serde_json::json!({ "chatId": chat_id }),
                ) => result,
            };
            drop(client);
            match subscription {
                Ok(mut stream) => loop {
                    let item = tokio::select! {
                        _ = cancel.cancelled() => return,
                        item = stream.recv() => item,
                    };
                    let Some(item) = item else { break };
                    let Ok(frame) = serde_json::from_value::<Frame>(item) else {
                        continue;
                    };
                    let Some(core) = weak.upgrade() else { return };
                    core.update_state(|st| st.prediction = frame.prediction);
                },
                Err(RpcError::UnknownMethod(_)) => return,
                Err(_) => {}
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(RETRY) => {}
            }
        }
    });
    Some(token)
}
