use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::SessionCore;

const RETRY: Duration = Duration::from_secs(2);

pub(super) fn spawn(
    core: &Arc<SessionCore>,
    method: &'static str,
    params: Value,
    apply: impl Fn(&SessionCore, Value) + Send + 'static,
) -> CancellationToken {
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
                result = live.relay.subscribe(&device_id, method, params.clone()) => result,
            };
            drop(client);
            match subscription {
                Ok(mut stream) => loop {
                    let item = tokio::select! {
                        _ = cancel.cancelled() => return,
                        item = stream.recv() => item,
                    };
                    let Some(item) = item else { break };
                    let Some(core) = weak.upgrade() else { return };
                    apply(&core, item);
                },
                Err(crate::ClientError::Unsupported(_)) => return,
                Err(_) => {}
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(RETRY) => {}
            }
        }
    });
    token
}
