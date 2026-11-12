use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use zeron_proto::prediction::ComposerPrediction;
use zeron_rpc::methods;

use super::SessionCore;

#[derive(serde::Deserialize)]
struct Frame {
    prediction: Option<ComposerPrediction>,
}

pub(super) fn start(core: &Arc<SessionCore>) -> Option<CancellationToken> {
    let client = core.client().ok()?;
    if let Some(demo) = client.demo() {
        let prediction = demo.prediction(&core.chat_id);
        core.update_state(|st| st.prediction = prediction);
        return None;
    }
    Some(super::host_watch::spawn(
        core,
        methods::WATCH_COMPOSER_PREDICTION,
        serde_json::json!({ "chatId": core.chat_id }),
        |core, item| {
            if let Ok(frame) = serde_json::from_value::<Frame>(item) {
                core.update_state(|st| st.prediction = frame.prediction);
            }
        },
    ))
}
