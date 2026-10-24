use zeron_proto::app_commands::COMPACT_COMMAND;
use zeron_proto::command_catalog::{
    CatalogFailure, InvocationCandidate, catalog_error_message, compaction_unsupported,
    merge_invocation_results, with_workspace_commands, with_zeron_commands,
};
use zeron_proto::invocation::Skill;
use zeron_proto::workspace::capabilities;
use zeron_proto::{CompactionKind, HarnessCompactionResult, HarnessId, SlashCommand};
use zeron_rpc::methods;

use crate::Client;
use crate::error::ClientError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationCatalog {
    pub entries: Vec<InvocationCandidate>,
    pub supported: bool,
    pub references: bool,
    pub warning: Option<String>,
    pub compact_unsupported: Option<String>,
}

fn failure(err: &ClientError) -> CatalogFailure {
    match err {
        ClientError::Unsupported(_) => CatalogFailure::OutdatedEngine,
        ClientError::HostUnavailable(_) | ClientError::Network(_) | ClientError::Closed => {
            CatalogFailure::Unreachable
        }
        _ => CatalogFailure::Failed,
    }
}

impl Client {
    /// Skills are fetched for `/` too, to drop commands that are skill aliases.
    /// A failure comes back as the warning, uncached by the caller.
    pub async fn invocation_catalog(
        &self,
        device_id: &str,
        harness: &str,
        chat_id: Option<&str>,
        space_id: Option<&str>,
        skill: bool,
    ) -> InvocationCatalog {
        let harness_id: Option<HarnessId> =
            serde_json::from_value(serde_json::Value::String(harness.into())).ok();
        let mut params = serde_json::json!({ "harness": harness, "targetDeviceId": device_id });
        if let Some(chat_id) = chat_id {
            params["chatId"] = chat_id.into();
        } else if let Some(space_id) = space_id {
            params["spaceId"] = space_id.into();
        }
        let commands = async {
            if skill {
                return Ok(Vec::new());
            }
            let value = self
                .inner
                .host_rpc(device_id, methods::LIST_COMMANDS, params.clone())
                .await?;
            serde_json::from_value::<Vec<SlashCommand>>(value)
                .map(|commands| with_zeron_commands(harness_id, commands))
                .map_err(|e| ClientError::HostError(e.to_string()))
        };
        let skills = async {
            let value = self
                .inner
                .host_rpc(device_id, methods::LIST_SKILLS, params.clone())
                .await?;
            serde_json::from_value::<Option<Vec<Skill>>>(value)
                .map_err(|e| ClientError::HostError(e.to_string()))
        };
        let compaction = async {
            // The engine resolves the probe without an instance, so `acp` always fails.
            let harness = harness_id.filter(|h| !skill && *h != HarnessId::Acp)?;
            let value = self
                .inner
                .host_rpc(
                    device_id,
                    methods::HARNESS_COMPACTION,
                    serde_json::json!({ "harness": harness, "targetDeviceId": device_id }),
                )
                .await
                .ok()?;
            let probe: HarnessCompactionResult = serde_json::from_value(value).ok()?;
            (probe.compaction == CompactionKind::None).then(|| compaction_unsupported(harness))
        };
        let (commands, skills, compact_unsupported) = futures::join!(commands, skills, compaction);
        let references = self
            .inner
            .workspace
            .snapshot()
            .device(device_id)
            .is_some_and(|device| {
                device
                    .capabilities
                    .iter()
                    .any(|c| c == capabilities::COMPOSER_REFERENCES_V1)
            });
        let message =
            |err: &ClientError, skills| catalog_error_message(failure(err), skills).into();
        let zeron = |entries| {
            if skill {
                entries
            } else {
                with_workspace_commands(entries, chat_id.is_some())
            }
        };
        let compact_unsupported = compact_unsupported.filter(|_| !skill);
        match merge_invocation_results(commands, skills, skill) {
            Ok((mut entries, supported, warning)) => {
                let prefix = if skill { '$' } else { '/' };
                entries.retain(|entry| entry.invocation.prefix() == prefix);
                let compact_unsupported = compact_unsupported
                    .filter(|_| entries.iter().any(|e| e.name == COMPACT_COMMAND));
                InvocationCatalog {
                    entries: zeron(entries),
                    supported,
                    references,
                    warning: warning.map(|(err, skills)| message(&err, skills)),
                    compact_unsupported,
                }
            }
            Err(err) => InvocationCatalog {
                entries: zeron(Vec::new()),
                supported: true,
                references,
                warning: Some(message(&err, skill)),
                compact_unsupported: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use zeron_proto::command_catalog::WorkspaceCommand;

    use crate::Client;
    use crate::config::{ClientConfig, Credentials, DemoOptions};
    use crate::events::NullListener;

    #[tokio::test]
    async fn every_demo_harness_loads_its_catalog_with_zeron_commands() {
        let dir = tempfile::tempdir().unwrap();
        let config = ClientConfig::new("https://edge.invalid", dir.path());
        let client = Client::new(
            config,
            Credentials::Demo(DemoOptions::default()),
            Arc::new(NullListener),
        )
        .unwrap();
        let mac = "dev-mac";
        for harness in ["claude-code", "codex", "opencode", "pi", "acp", "cursor"] {
            let slash = client
                .invocation_catalog(mac, harness, Some("chat-home"), None, false)
                .await;
            assert!(slash.warning.is_none(), "{harness}: {:?}", slash.warning);
            assert!(
                slash
                    .entries
                    .iter()
                    .any(|e| e.workspace_command == Some(WorkspaceCommand::Stop)),
                "{harness}"
            );
            assert_eq!(
                slash.compact_unsupported.is_some(),
                harness == "cursor",
                "{harness}"
            );
        }
        let codex = client
            .invocation_catalog(mac, "codex", None, None, false)
            .await;
        let names: Vec<_> = codex.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"diff") && !names.contains(&"zeron:diff"));
        assert!(names.contains(&"zeron:new") && !names.contains(&"stop"));
        let failed = client
            .invocation_catalog(mac, "antigravity", None, None, false)
            .await;
        assert!(failed.warning.is_some());
        assert!(failed.entries.iter().all(|e| e.workspace_command.is_some()));
        let skills = client.invocation_catalog(mac, "pi", None, None, true).await;
        assert!(!skills.supported && skills.entries.is_empty());
    }
}
