use std::sync::Arc;

use serde_json::Value;
use zeron_doc::{DocError, SessionDoc};
use zeron_proto::invocation::Skill;
use zeron_proto::prediction::ComposerPrediction;
use zeron_proto::{Chat, HarnessId, SessionStats, SlashCommand, SlashCommandKind};

use crate::client::ClientInner;
use crate::error::ClientError;

pub(super) fn prediction(chat_id: &str) -> Option<ComposerPrediction> {
    (chat_id == "chat-home").then(|| ComposerPrediction {
        entry_id: "demo-prediction".into(),
        text: "Run the iOS layout tests on the simulator".into(),
    })
}

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

pub(super) fn continue_in_new_thread(
    client: &Arc<ClientInner>,
    params: &Value,
) -> crate::error::Result<Value> {
    let source_id = params["sourceChatId"].as_str().unwrap_or_default();
    let source = client
        .workspace
        .chat(source_id)
        .ok_or_else(|| ClientError::NotFound(source_id.to_owned()))?;
    let mut config = source.config.clone();
    if let (Some(config), Ok(harness), Some(model)) = (
        config.as_mut(),
        serde_json::from_value::<HarnessId>(params["harness"].clone()),
        params["model"].as_str(),
    ) {
        if config.harness != harness {
            config.reasoning = None;
            config.model_options.clear();
        }
        config.harness = harness;
        config.model = Some(model.to_owned());
    }
    let space = params["targetSpaceId"].as_str().and_then(|id| {
        client
            .workspace
            .snapshot()
            .projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| (p.id.clone(), p.path.clone()))
    });
    let seed = source.title.clone().unwrap_or_else(|| "chat".into());
    let chat = Chat {
        id: params["chatId"].as_str().unwrap_or_default().to_owned(),
        title: Some(format!("{seed} (cont.)")),
        config,
        last_message_preview: None,
        last_message_at: None,
        created_at: chrono::Utc::now(),
        harness_session_id: None,
        last_seen_at: None,
        sidechat_source_chat_id: None,
        created_by_chat_id: None,
        forked_from_chat_id: None,
        delegation: None,
        settle: None,
        unsettled_at: None,
        ..match space {
            Some((space_id, path)) => Chat {
                space_id: Some(space_id),
                cwd: Some(path),
                branch: None,
                checkout_id: None,
                ..source
            },
            None => source,
        }
    };
    client.registry_write(|doc| doc.upsert_chat(&chat))?;
    Ok(serde_json::to_value(chat).unwrap_or_default())
}

pub(super) fn create_sidechat(
    client: &Arc<ClientInner>,
    params: &Value,
) -> crate::error::Result<Value> {
    let source_id = params["sourceChatId"].as_str().unwrap_or_default();
    let source = client
        .workspace
        .chat(source_id)
        .ok_or_else(|| ClientError::NotFound(source_id.to_owned()))?;
    if source.sidechat_source_chat_id.is_some() {
        return Err(ClientError::HostError(
            "nested sidechats are not supported".into(),
        ));
    }
    let seed = source.title.clone().unwrap_or_else(|| "chat".into());
    let chat = Chat {
        id: params["chatId"].as_str().unwrap_or_default().to_owned(),
        title: Some(format!("Sidechat: {seed}")),
        sidechat_source_chat_id: Some(source_id.to_owned()),
        last_message_preview: None,
        last_message_at: None,
        created_at: chrono::Utc::now(),
        harness_session_id: None,
        last_seen_at: None,
        created_by_chat_id: None,
        forked_from_chat_id: None,
        delegation: None,
        settle: None,
        unsettled_at: None,
        ..source
    };
    client.registry_write(|doc| doc.upsert_chat(&chat))?;
    Ok(serde_json::to_value(chat).unwrap_or_default())
}
