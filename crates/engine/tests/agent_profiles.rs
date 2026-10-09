use std::sync::Arc;

use serde_json::json;
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_proto::AgentProfilesSnapshot;
use zeron_rpc::methods;

fn ids(snapshot: &AgentProfilesSnapshot) -> Vec<&str> {
    snapshot.profiles.iter().map(|p| p.id.as_str()).collect()
}

#[tokio::test]
async fn agent_profiles_crud_over_rpc_survives_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let assemble = || {
        EngineCore::assemble(
            &data,
            Arc::new(HarnessRegistry::new()),
            zeron_proto::HarnessId::Mock,
            None,
        )
        .expect("engine assembles")
    };

    let core = assemble();
    let client = zeron_rpc::memory_client(core.rpc_service());
    let empty: AgentProfilesSnapshot = client
        .call_as(methods::LIST_AGENT_PROFILES, json!({}))
        .await
        .unwrap();
    assert!(empty.profiles.is_empty());

    for (name, harness) in [("Reviewer", "codex"), ("UI work", "claude-code")] {
        client
            .call(
                methods::UPSERT_AGENT_PROFILE,
                json!({"profile": {
                    "name": name,
                    "harness": harness,
                    "model": "gpt-5.6",
                    "reasoning": "xhigh",
                    "permissionMode": "ask",
                    "notes": "Review a diff."
                }}),
            )
            .await
            .unwrap();
    }
    let reordered: AgentProfilesSnapshot = client
        .call_as(
            methods::REORDER_AGENT_PROFILES,
            json!({"profileIds": ["ui-work", "reviewer"]}),
        )
        .await
        .unwrap();
    assert_eq!(ids(&reordered), ["ui-work", "reviewer"]);

    let edited: AgentProfilesSnapshot = client
        .call_as(
            methods::UPSERT_AGENT_PROFILE,
            json!({"profileId": "reviewer", "profile": {"name": "Reviewer", "harness": "codex"}}),
        )
        .await
        .unwrap();
    assert_eq!(edited.profiles[1].model, None);
    assert!(
        client
            .call(methods::DELETE_AGENT_PROFILE, json!({"profileId": "nope"}))
            .await
            .is_err()
    );
    drop(client);
    drop(core);

    let core = assemble();
    let client = zeron_rpc::memory_client(core.rpc_service());
    let reloaded: AgentProfilesSnapshot = client
        .call_as(methods::LIST_AGENT_PROFILES, json!({}))
        .await
        .unwrap();
    assert_eq!(reloaded, edited);
    let after_delete: AgentProfilesSnapshot = client
        .call_as(
            methods::DELETE_AGENT_PROFILE,
            json!({"profileId": "ui-work"}),
        )
        .await
        .unwrap();
    assert_eq!(ids(&after_delete), ["reviewer"]);
}

#[tokio::test]
async fn deleting_a_profile_detaches_it_from_this_hosts_chats() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let core = EngineCore::assemble(
        &data,
        Arc::new(HarnessRegistry::new()),
        zeron_proto::HarnessId::Mock,
        None,
    )
    .expect("engine assembles");
    let client = zeron_rpc::memory_client(core.rpc_service());
    for name in ["Planner", "Executor"] {
        client
            .call(
                methods::UPSERT_AGENT_PROFILE,
                json!({"profile": {"name": name, "harness": "mock", "instructions": name}}),
            )
            .await
            .unwrap();
    }
    client
        .call(
            methods::MUTATE,
            json!({"op": "createSpace", "spaceId": "s1", "deviceId": core.workspace.device_id(),
                   "path": tmp.path().to_string_lossy()}),
        )
        .await
        .unwrap();
    for (chat, profile, instructions) in
        [("a", "planner", "Planner"), ("b", "executor", "Executor")]
    {
        client
            .call(
                methods::MUTATE,
                json!({"op": "createChat", "chatId": chat, "spaceId": "s1", "config": {
                    "harness": "mock", "model": null, "reasoning": null, "modelOptions": {},
                    "sandbox": "workspace-write", "instructions": instructions, "profileId": profile
                }}),
            )
            .await
            .unwrap();
    }

    client
        .call(
            methods::DELETE_AGENT_PROFILE,
            json!({"profileId": "planner"}),
        )
        .await
        .unwrap();

    let config = |chat: &str| core.workspace.chat(chat).unwrap().unwrap().config.unwrap();
    assert_eq!(config("a").profile_id, None);
    assert_eq!(config("a").instructions, "");
    assert_eq!(config("b").profile_id.as_deref(), Some("executor"));
    assert_eq!(config("b").instructions, "Executor");
    core.shutdown().await;
}
