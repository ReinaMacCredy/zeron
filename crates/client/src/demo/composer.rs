use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use serde_json::Value;
use zeron_doc::{DocError, MessagePart, MessageRole, SessionDoc, SubagentStatus};
use zeron_proto::invocation::Skill;
use zeron_proto::prediction::ComposerPrediction;
use zeron_proto::{
    AgentProfile, AgentProfileDraft, AgentProfilesSnapshot, BackgroundTask, BackgroundTaskKind,
    Chat, ChatConfig, CompactPlan, HarnessId, ProviderInstance, ProviderInstancesSnapshot,
    ReasoningLevel, Session, SessionStats, SessionStatus, SlashCommand, SlashCommandKind, ToolCall,
};
use zeron_rpc::methods;

use super::transcripts::{PHONE, entry, text, tool};
use crate::client::ClientInner;
use crate::error::ClientError;
use crate::lock;
use crate::session::SessionCore;

const DEMO_SHELL: &str = "shell-mobile-tests";

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
    for entry in subagent_turn(now) {
        doc.push_message(&entry)?;
    }
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

fn spawn(id: &str, task: &str, model: &str, status: SubagentStatus) -> MessagePart {
    let mut part = tool(
        id,
        ToolCall::Unknown {
            name: format!("Agent: {task}"),
            input: Some(serde_json::json!({ "description": task, "model": model })),
        },
        false,
        None,
    );
    if let MessagePart::Tool {
        subagent_ref,
        subagent_status,
        ..
    } = &mut part
    {
        *subagent_ref = Some(format!("chat-home--sub--{id}"));
        *subagent_status = Some(status);
    }
    part
}

fn subagent_turn(now: i64) -> Vec<zeron_doc::SessionMessageEntry> {
    let host = super::fixtures::MAC;
    vec![
        entry(
            "m-agents-ask",
            MessageRole::User,
            PHONE,
            now - 260_000,
            vec![text(
                "t0",
                "Audit the composer pickers and the sheet layout in parallel.",
            )],
        ),
        entry(
            "m-agents-1",
            MessageRole::Assistant,
            host,
            now - 252_000,
            vec![spawn(
                "spawn-layout",
                "Check the sheet layout",
                "gpt-5.5",
                SubagentStatus::Done,
            )],
        ),
        entry(
            "m-agents-2",
            MessageRole::Assistant,
            host,
            now - 72_000,
            vec![
                spawn(
                    "spawn-pickers",
                    "Audit the composer pickers",
                    "sonnet-5",
                    SubagentStatus::Running,
                ),
                spawn(
                    "spawn-drafts",
                    "Trace the draft restore path",
                    "sonnet-5",
                    SubagentStatus::Running,
                ),
            ],
        ),
    ]
}

pub(super) fn background_session(chat_id: &str) -> Option<Session> {
    (chat_id == "chat-home").then(|| Session {
        last_completed_turn: None,
        chat_id: chat_id.into(),
        device_id: super::fixtures::MAC.into(),
        status: SessionStatus::Working,
        held_by_subagents: true,
        compacting: false,
        classifying: None,
        background_tasks: vec![BackgroundTask {
            id: DEMO_SHELL.into(),
            kind: BackgroundTaskKind::Shell,
            label: "cargo test -p zeron-mobile".into(),
            started_at: Some(Utc::now() - Duration::seconds(41)),
            output_path: Some("/tmp/zeron-demo/shell-mobile-tests.log".into()),
        }],
        started_at: None,
        updated_at: Utc::now(),
        running_subagents: 2,
        compact_plan: Some(CompactPlan::Idle {
            at: (Utc::now() + Duration::minutes(38)).timestamp_millis(),
        }),
    })
}

pub(super) fn shell_output(task_id: &str) -> Vec<String> {
    if task_id != DEMO_SHELL {
        return Vec::new();
    }
    [
        "   Compiling zeron-client v0.1.0",
        "   Compiling zeron-mobile v0.1.0",
        "    Finished `test` profile [unoptimized + debuginfo] target(s) in 38.2s",
        "     Running unittests src/lib.rs (target/debug/deps/zeron_mobile-5f1c)",
        "running 214 tests",
        "test client_ffi::composer::tests::roster_labels ... ok",
        "test client_ffi::composer::tests::stop_routes_by_kind ... ok",
    ]
    .map(str::to_owned)
    .to_vec()
}

pub(super) fn stop_background_task(
    core: &SessionCore,
    params: &Value,
) -> crate::error::Result<Value> {
    let task_id = params["taskId"].as_str().unwrap_or_default();
    if !core.write(|doc| doc.update_subagent_chip(task_id, None, Some("stopped"), None))? {
        core.drop_background_task(task_id);
    }
    Ok(serde_json::json!({}))
}

const REVIEWER: &str = "ios-reviewer";
const FAILING_INSTANCE: &str = "codex_work";

fn reviewer_tools() -> Vec<String> {
    vec!["WebFetch".into(), "NotebookEdit".into()]
}

pub(super) fn profiles() -> Vec<AgentProfile> {
    vec![
        AgentProfile {
            id: REVIEWER.into(),
            name: "iOS reviewer".into(),
            harness: HarnessId::ClaudeCode,
            model: Some("claude-sonnet-5".into()),
            reasoning: Some(ReasoningLevel::High),
            permission_mode: None,
            model_options: Default::default(),
            notes: "UIKit layout and accessibility review".into(),
            instructions: String::new(),
            disabled_tools: reviewer_tools(),
            by_agent: false,
        },
        AgentProfile {
            id: "planner".into(),
            name: "Planner".into(),
            harness: HarnessId::Codex,
            model: None,
            reasoning: Some(ReasoningLevel::Max),
            permission_mode: Some(zeron_proto::PermissionMode::Ask),
            model_options: Default::default(),
            notes: String::new(),
            instructions: "Plan first; do not edit files.".into(),
            disabled_tools: Vec::new(),
            by_agent: false,
        },
    ]
}

/// chat-home's effort has moved off the reviewer profile's, so its menu shows Edited.
pub(super) fn with_profile(chat_id: &str, mut config: ChatConfig) -> ChatConfig {
    if chat_id == "chat-home" {
        config.profile_id = Some(REVIEWER.into());
        config.disabled_tools = reviewer_tools();
    }
    config
}

fn instances() -> Vec<ProviderInstance> {
    let instance = |id: &str, driver, label: &str, accent: &str| ProviderInstance {
        id: id.into(),
        driver,
        label: label.into(),
        accent: Some(accent.into()),
        config: Default::default(),
    };
    vec![
        instance(
            "claude_personal",
            HarnessId::ClaudeCode,
            "Claude · Personal",
            "#46a758",
        ),
        instance(
            FAILING_INSTANCE,
            HarnessId::Codex,
            "Codex · Work",
            "#3e9cf0",
        ),
    ]
}

pub(super) fn model_menu_rpc(
    profiles: &Mutex<Vec<AgentProfile>>,
    method: &str,
    params: &Value,
) -> Option<crate::error::Result<Value>> {
    let snapshot = |profiles: Vec<AgentProfile>| {
        Ok(serde_json::to_value(AgentProfilesSnapshot { profiles }).unwrap_or_default())
    };
    match method {
        methods::LIST_PROVIDER_INSTANCES => {
            Some(Ok(serde_json::to_value(ProviderInstancesSnapshot {
                instances: instances(),
            })
            .unwrap_or_default()))
        }
        methods::LIST_AGENT_PROFILES => Some(snapshot(lock(profiles).clone())),
        methods::UPSERT_AGENT_PROFILE => {
            let draft = serde_json::from_value::<AgentProfileDraft>(params["profile"].clone());
            let id = params["profileId"].as_str().unwrap_or_default();
            let mut profiles = lock(profiles);
            let (Ok(draft), Some(profile)) = (draft, profiles.iter_mut().find(|p| p.id == id))
            else {
                return Some(Err(ClientError::HostError("unknown profile".into())));
            };
            profile.harness = draft.harness;
            profile.model = draft.model;
            profile.reasoning = draft.reasoning;
            profile.permission_mode = draft.permission_mode;
            profile.model_options = draft.model_options;
            profile.disabled_tools = draft.disabled_tools;
            Some(snapshot(profiles.clone()))
        }
        methods::LIST_MODELS => {
            let instance = params["instanceId"].as_str()?;
            if instance == FAILING_INSTANCE {
                return Some(Err(ClientError::HostError(
                    "model discovery timeout: timed out after 20s".into(),
                )));
            }
            let harness = params["harness"].as_str().unwrap_or_default();
            Some(Ok(serde_json::to_value(crate::catalog::fallback_models(
                harness,
            ))
            .unwrap_or_default()))
        }
        _ => None,
    }
}
