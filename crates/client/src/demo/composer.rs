use zeron_doc::{DocError, SessionDoc};
use zeron_proto::SessionStats;

pub(super) fn seed_meta(chat_id: &str, doc: &SessionDoc, now: i64) -> Result<(), DocError> {
    if chat_id != "chat-home" {
        return Ok(());
    }
    doc.update_context_usage(Some(184_312), Some(200_000))?;
    doc.record_session_stats(&SessionStats {
        turns: 14,
        steps: 212,
        llm_ms: 1_284_000,
        tool_ms: 402_000,
        ttft_ms: 26_600,
        ttft_count: 14,
        output_tokens: 61_400,
        usage_llm_ms: 1_284_000,
        input_tokens: 38_200,
        cache_read_tokens: 2_914_000,
        cache_write_tokens: 186_300,
        cache_ttl_secs: Some(3600),
        cache_touched_at: Some(now - 8 * 60_000),
        cost_micros: 4_870_000,
        cost_estimated: true,
        ..Default::default()
    })
}
