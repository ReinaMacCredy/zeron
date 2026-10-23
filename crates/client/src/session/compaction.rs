use std::time::Duration;

use zeron_proto::HarnessId;
use zeron_proto::compaction::{ResumeCompaction, compact_outcome_notice, resume_compaction_due};
use zeron_proto::stats_view::CompactSection;

use super::{SessionHandle, lock};
use crate::error::Result;
use crate::workspace::SessionRow;

// Outlives the engine's own 120s compaction guard, so the reply carries the
// harness's outcome rather than a local timeout.
const COMPACT_TIMEOUT: Duration = Duration::from_secs(180);

pub(super) fn compact_blocked(compacting: bool, turn_live: bool) -> Option<String> {
    if compacting {
        Some("Compaction in progress".into())
    } else if turn_live {
        Some("Wait for the current turn to finish".into())
    } else {
        None
    }
}

pub(super) fn resume_offer(
    row: Option<&SessionRow>,
    tokens: Option<u64>,
    kept_at: Option<i64>,
) -> Option<ResumeCompaction> {
    let row = row?;
    if kept_at == Some(row.last_activity_ms) {
        return None;
    }
    let harness: HarnessId =
        serde_json::from_value(serde_json::Value::String(row.harness.clone()?)).ok()?;
    let last = chrono::DateTime::from_timestamp_millis(row.last_activity_ms);
    resume_compaction_due(harness, tokens, last, chrono::Utc::now())
}

impl SessionHandle {
    pub async fn compact(&self, focus: Option<String>) -> Result<Option<String>> {
        let composer = self.composer();
        if let Some(reason) = composer.compact_blocked.clone() {
            return Ok(Some(reason));
        }
        let client = self.core.client()?;
        let mut params = serde_json::json!({ "chatId": self.core.chat_id });
        if let Some(focus) = focus {
            params["focus"] = focus.into();
        }
        self.core.update_state(|st| st.compacting = true);
        let result = tokio::time::timeout(
            COMPACT_TIMEOUT,
            client.host_rpc(
                &composer.host.device_id,
                zeron_rpc::methods::COMPACT_CHAT,
                params,
            ),
        )
        .await;
        self.core.update_state(|st| st.compacting = false);
        Ok(match result {
            Ok(Ok(reply)) => compact_outcome_notice(&reply),
            Ok(Err(err)) => Some(format!("Compaction failed: {err}")),
            Err(_) => Some("Compaction failed: timed out".into()),
        })
    }

    pub fn compact_section(&self, now_ms: i64) -> Option<CompactSection> {
        let plan = lock(&self.core.state)
            .session
            .as_ref()
            .and_then(|session| session.compact_plan);
        CompactSection::of(
            plan,
            self.composer().compacting,
            self.session_stats().as_ref(),
            now_ms,
        )
    }

    pub fn keep_full_history(&self) {
        let Ok(client) = self.core.client() else {
            return;
        };
        let last = client
            .workspace
            .snapshot()
            .session(&self.core.chat_id)
            .map(|row| row.last_activity_ms);
        self.core.update_state(|st| st.kept_full_history = last);
    }
}
