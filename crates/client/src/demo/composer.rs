use zeron_doc::{DocError, SessionDoc};
use zeron_proto::invocation::Skill;
use zeron_proto::{SessionStats, SlashCommand, SlashCommandKind};

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

pub(super) fn invocation_catalog(method: &str) -> Option<serde_json::Value> {
    let command = |name: &str, description: &str, input_hint: Option<&str>| SlashCommand {
        name: name.into(),
        description: description.into(),
        input_hint: input_hint.map(Into::into),
        kind: SlashCommandKind::BuiltIn,
    };
    let skill = |name: &str, description: &str| Skill {
        name: name.into(),
        path: format!("/Users/demo/.claude/skills/{name}/SKILL.md"),
        description: description.into(),
        enabled: true,
        command: None,
    };
    match method {
        zeron_rpc::methods::LIST_COMMANDS => serde_json::to_value(vec![
            command(
                "clear",
                "Clear conversation history and free up context",
                None,
            ),
            command("review", "Review a pull request", Some("pr")),
            command(
                "init",
                "Initialize a CLAUDE.md file with codebase documentation",
                None,
            ),
            command(
                "cost",
                "Show the total cost and duration of the current session",
                None,
            ),
        ])
        .ok(),
        zeron_rpc::methods::LIST_SKILLS => serde_json::to_value(Some(vec![
            skill(
                "frontend-design",
                "Create distinctive, production-grade frontend interfaces",
            ),
            skill("release-notes", "Draft release notes from merged changes"),
            skill("pdf", "Read, create and edit PDF files"),
        ]))
        .ok(),
        _ => None,
    }
}
