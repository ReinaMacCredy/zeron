use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::json;
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_proto::{
    AgentAccountsSnapshot, AgentProfilesSnapshot, HarnessId, ProviderInstancesSnapshot,
};
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

#[tokio::test]
async fn provider_instances_live_in_the_device_data_dir_and_survive_a_restart() {
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
    let mut watch = client
        .subscribe(methods::WATCH_PROVIDER_INSTANCES, json!({}))
        .await
        .unwrap();
    let mut next = async || -> ProviderInstancesSnapshot {
        let value = tokio::time::timeout(std::time::Duration::from_secs(10), watch.recv())
            .await
            .unwrap()
            .unwrap();
        serde_json::from_value(value).unwrap()
    };
    assert!(next().await.instances.is_empty());
    let saved: ProviderInstancesSnapshot = client
        .call_as(
            methods::UPSERT_PROVIDER_INSTANCE,
            json!({"instance": {"id": "codex_work", "driver": "codex", "label": "Work",
                   "config": {"kind": "native", "homeDir": {"kind": "managed"}}}}),
        )
        .await
        .unwrap();
    assert_eq!(saved.instances.len(), 1);
    assert_eq!(next().await, saved);
    assert!(
        client
            .call(
                methods::UPSERT_PROVIDER_INSTANCE,
                json!({"instance": {"id": "codex", "driver": "codex", "label": "Codex"}}),
            )
            .await
            .is_err()
    );
    assert!(data.join("provider-instances.json").is_file());
    drop(client);
    drop(core);

    let core = assemble();
    let client = zeron_rpc::memory_client(core.rpc_service());
    let reloaded: ProviderInstancesSnapshot = client
        .call_as(methods::LIST_PROVIDER_INSTANCES, json!({}))
        .await
        .unwrap();
    assert_eq!(reloaded, saved);

    let work_home = data.join("provider-homes").join("codex_work");
    let work_slots = data.join("agent-accounts/instances/codex_work");
    let claims = URL_SAFE_NO_PAD.encode(
        json!({"email": "work@example.com",
               "https://api.openai.com/auth": {"chatgpt_account_id": "acct-work"}})
        .to_string(),
    );
    std::fs::create_dir_all(&work_home).unwrap();
    std::fs::write(
        work_home.join("auth.json"),
        json!({"tokens": {"id_token": format!("e30.{claims}.x"), "access_token": "at"}})
            .to_string(),
    )
    .unwrap();
    let listed: AgentAccountsSnapshot = client
        .call_as(
            methods::LIST_AGENT_ACCOUNTS,
            json!({"instanceId": "codex_work"}),
        )
        .await
        .unwrap();
    assert_eq!(listed.accounts.len(), 1, "{listed:?}");
    assert_eq!(listed.accounts[0].harness, HarnessId::Codex);
    assert_eq!(
        listed.accounts[0].email.as_deref(),
        Some("work@example.com")
    );
    assert!(listed.accounts[0].active);
    assert!(work_slots.is_dir());
    let unknown = client
        .call(
            methods::LIST_AGENT_ACCOUNTS,
            json!({"instanceId": "codex_gone"}),
        )
        .await
        .unwrap_err();
    assert!(unknown.to_string().contains("'codex_gone'"), "{unknown}");
    let mismatch = client
        .call(
            methods::START_AGENT_LOGIN,
            json!({"harness": "claude-code", "instanceId": "codex_work"}),
        )
        .await
        .unwrap_err();
    assert!(mismatch.to_string().contains("runs on Codex"), "{mismatch}");

    let custom_home = tmp.path().join("custom-codex");
    std::fs::create_dir_all(&custom_home).unwrap();
    std::fs::write(custom_home.join("auth.json"), b"{}").unwrap();
    client
        .call(
            methods::UPSERT_PROVIDER_INSTANCE,
            json!({"instance": {"id": "codex_custom", "driver": "codex", "label": "Custom",
                   "config": {"kind": "native",
                              "homeDir": {"kind": "custom", "path": custom_home}}}}),
        )
        .await
        .unwrap();
    client
        .call(
            methods::LIST_AGENT_ACCOUNTS,
            json!({"instanceId": "codex_custom"}),
        )
        .await
        .unwrap();

    client
        .call(
            methods::DELETE_PROVIDER_INSTANCE,
            json!({"instanceId": "codex_work"}),
        )
        .await
        .unwrap();
    assert!(!work_home.exists());
    assert!(!work_slots.exists());
    let emptied: ProviderInstancesSnapshot = client
        .call_as(
            methods::DELETE_PROVIDER_INSTANCE,
            json!({"instanceId": "codex_custom"}),
        )
        .await
        .unwrap();
    assert!(emptied.instances.is_empty());
    assert!(custom_home.join("auth.json").is_file());
    assert!(!data.join("agent-accounts/instances/codex_custom").exists());
}
