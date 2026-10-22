use zeron_proto::model_switch::{ModelSwitchPrompt, model_switch_prompt, provider_switch_prompt};
use zeron_proto::picker::{catalog_error_text, detach_profile, savable};
use zeron_proto::{
    AgentProfile, AgentProfilesSnapshot, ChatConfig, HarnessId, ProviderInstance,
    ProviderInstancesSnapshot,
};
use zeron_rpc::methods;

use crate::Client;
use crate::catalog::{self, ModelInfo};
use crate::error::{ClientError, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum InstanceCatalog {
    Ready(Vec<ModelInfo>),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileRow {
    pub id: String,
    pub name: String,
    pub line: String,
    pub active: bool,
    pub drift: Option<String>,
    pub savable: bool,
}

pub fn harness_wire(harness: HarnessId) -> String {
    serde_json::to_value(harness)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn decode<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T> {
    serde_json::from_value(value).map_err(|err| ClientError::HostError(err.to_string()))
}

impl Client {
    pub async fn list_provider_instances(&self, device_id: &str) -> Result<Vec<ProviderInstance>> {
        let reply = self
            .inner
            .host_rpc(
                device_id,
                methods::LIST_PROVIDER_INSTANCES,
                serde_json::json!({}),
            )
            .await?;
        Ok(decode::<ProviderInstancesSnapshot>(reply)?.instances)
    }

    /// An instance has no cached or curated fallback: a failed discovery is
    /// shown as such, with Retry.
    pub async fn list_instance_models(
        &self,
        device_id: &str,
        harness: &str,
        instance_id: &str,
        force: bool,
    ) -> InstanceCatalog {
        let params =
            serde_json::json!({ "harness": harness, "instanceId": instance_id, "force": force });
        let reply = self
            .inner
            .host_rpc(device_id, methods::LIST_MODELS, params)
            .await
            .and_then(|value| {
                catalog::decode_models(value).map_err(|err| ClientError::HostError(err.to_string()))
            });
        match reply {
            Ok(models) => InstanceCatalog::Ready(catalog::normalize_models(harness, models)),
            Err(ClientError::HostError(message)) => {
                InstanceCatalog::Failed(catalog_error_text(&message).to_owned())
            }
            Err(err) => InstanceCatalog::Failed(err.to_string()),
        }
    }

    pub async fn list_agent_profiles(&self, device_id: &str) -> Result<Vec<AgentProfile>> {
        let reply = self
            .inner
            .host_rpc(
                device_id,
                methods::LIST_AGENT_PROFILES,
                serde_json::json!({}),
            )
            .await?;
        Ok(decode::<AgentProfilesSnapshot>(reply)?.profiles)
    }

    fn chat_device_and_config(&self, chat_id: &str) -> Result<(String, ChatConfig)> {
        let chat = self
            .inner
            .workspace
            .chat(chat_id)
            .ok_or_else(|| ClientError::NotFound(chat_id.to_owned()))?;
        let config = chat
            .config
            .ok_or_else(|| ClientError::NotFound(chat_id.to_owned()))?;
        Ok((chat.device_id, config))
    }

    async fn chat_profile(
        &self,
        chat_id: &str,
        profile_id: &str,
    ) -> Result<(String, ChatConfig, AgentProfile)> {
        let (device_id, config) = self.chat_device_and_config(chat_id)?;
        let profile = self
            .list_agent_profiles(&device_id)
            .await?
            .into_iter()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| ClientError::NotFound(profile_id.to_owned()))?;
        Ok((device_id, config, profile))
    }

    pub async fn profile_rows(&self, chat_id: &str) -> Result<Vec<ProfileRow>> {
        let (device_id, mut config) = self.chat_device_and_config(chat_id)?;
        let profiles = self.list_agent_profiles(&device_id).await?;
        let harness = harness_wire(config.harness);
        let models = self.list_models(&device_id, &harness).await;
        let options = config
            .model
            .as_deref()
            .and_then(|id| models.iter().find(|m| m.id == id))
            .map(|m| m.options.clone())
            .unwrap_or_default();
        for option in &options {
            config
                .model_options
                .entry(option.id.clone())
                .or_insert_with(|| serde_json::Value::String(option.default_choice.clone()));
        }
        Ok(profiles
            .iter()
            .map(|profile| {
                let wire = harness_wire(profile.harness);
                let label = |model: &str| catalog::model_label(&wire, model);
                let active = config.profile_id.as_deref() == Some(profile.id.as_str());
                let drift = if active {
                    profile.drift(&config)
                } else {
                    Vec::new()
                };
                ProfileRow {
                    id: profile.id.clone(),
                    name: profile.name.clone(),
                    line: profile.line(profile.model.as_deref().map(label).as_deref()),
                    active,
                    drift: (!drift.is_empty()).then(|| {
                        profile.drift_line(&drift, &config, label, |key| {
                            options
                                .iter()
                                .find(|option| option.id == key)
                                .map_or(key, |option| option.label.as_str())
                                .to_owned()
                        })
                    }),
                    savable: savable(&drift),
                }
            })
            .collect())
    }

    pub async fn profile_switch_prompt(
        &self,
        chat_id: &str,
        profile_id: &str,
    ) -> Result<Option<ModelSwitchPrompt>> {
        let (_, config, profile) = self.chat_profile(chat_id, profile_id).await?;
        let harness_changes = config.harness != profile.harness;
        let model_changes = profile
            .model
            .as_ref()
            .is_some_and(|model| config.model.as_ref() != Some(model));
        if !(harness_changes || model_changes) || !self.chat_has_history(chat_id) {
            return Ok(None);
        }
        let from_model = config
            .model
            .as_deref()
            .map(|model| catalog::model_label(&harness_wire(config.harness), model))
            .unwrap_or_default();
        let to_model = profile
            .model
            .as_deref()
            .map(|model| catalog::model_label(&harness_wire(profile.harness), model))
            .unwrap_or_else(|| profile.harness.display_name().to_owned());
        Ok(Some(if harness_changes {
            provider_switch_prompt(
                config.harness.display_name(),
                profile.harness.display_name(),
                &from_model,
                &to_model,
            )
        } else {
            model_switch_prompt(Some(config.harness), &from_model, &to_model)
        }))
    }

    pub async fn apply_agent_profile(
        &self,
        chat_id: &str,
        profile_id: &str,
        note: Option<(String, String)>,
    ) -> Result<()> {
        let (_, mut config, profile) = self.chat_profile(chat_id, profile_id).await?;
        profile.apply_to(&mut config);
        match note {
            Some((note, hint)) => {
                self.set_session_config_noted(chat_id, &config, &note, &hint)
                    .await
            }
            None => self.set_session_config(chat_id, &config),
        }
    }

    pub fn detach_agent_profile(&self, chat_id: &str) -> Result<()> {
        let (_, mut config) = self.chat_device_and_config(chat_id)?;
        detach_profile(&mut config);
        self.set_session_config(chat_id, &config)
    }

    pub async fn save_chat_into_profile(&self, chat_id: &str, profile_id: &str) -> Result<()> {
        let (device_id, config, profile) = self.chat_profile(chat_id, profile_id).await?;
        let params =
            serde_json::json!({ "profileId": profile.id, "profile": profile.absorb(&config) });
        self.inner
            .host_rpc(&device_id, methods::UPSERT_AGENT_PROFILE, params)
            .await?;
        Ok(())
    }
}
