use zeron_proto::command_catalog::{
    CatalogFailure, InvocationCandidate, catalog_error_message, merge_invocation_results,
    with_zeron_commands,
};
use zeron_proto::invocation::Skill;
use zeron_proto::workspace::capabilities;
use zeron_proto::{HarnessId, SlashCommand};
use zeron_rpc::methods;

use crate::Client;
use crate::error::ClientError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationCatalog {
    pub entries: Vec<InvocationCandidate>,
    pub supported: bool,
    pub references: bool,
    pub warning: Option<String>,
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
        let (commands, skills) = futures::join!(commands, skills);
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
        match merge_invocation_results(commands, skills, skill) {
            Ok((mut entries, supported, warning)) => {
                let prefix = if skill { '$' } else { '/' };
                entries.retain(|entry| entry.invocation.prefix() == prefix);
                InvocationCatalog {
                    entries,
                    supported,
                    references,
                    warning: warning.map(|(err, skills)| message(&err, skills)),
                }
            }
            Err(err) => InvocationCatalog {
                entries: Vec::new(),
                supported: true,
                references,
                warning: Some(message(&err, skill)),
            },
        }
    }
}
