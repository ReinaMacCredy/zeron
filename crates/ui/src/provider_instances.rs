use gpui::App;
use serde_json::{Value, json};
use zeron_proto::{
    AcpAuthMethod, AcpDistribution, AcpPreparedAgent, AcpRegistryAgent, AcpSource, HarnessId,
    InstanceConfig, ProviderInstance, ProviderInstancesSnapshot, validate_instance_id,
};
use zeron_rpc::methods;

use crate::state::EngineHandle;

pub const ACCENTS: [&str; 7] = [
    "#e5484d", "#f5a524", "#46a758", "#3e9cf0", "#8e4ec6", "#d6409f", "#8b8d98",
];

#[derive(Default)]
pub struct ProviderInstancesChanged;

impl gpui::Global for ProviderInstancesChanged {}

pub fn bump_provider_instances(cx: &mut App) {
    cx.default_global::<ProviderInstancesChanged>();
}

pub enum ProviderInstanceWrite {
    Upsert(ProviderInstance),
    Delete { instance_id: String },
}

pub async fn list(
    engine: &EngineHandle,
    target: Option<Value>,
) -> Result<Vec<ProviderInstance>, String> {
    let mut params = json!({});
    if let Some(target) = target {
        params["targetDeviceId"] = target;
    }
    engine
        .client()
        .call_as::<ProviderInstancesSnapshot>(methods::LIST_PROVIDER_INSTANCES, params)
        .await
        .map(|snapshot| snapshot.instances)
        .map_err(|err| err.to_string())
}

pub async fn write(
    engine: &EngineHandle,
    write: ProviderInstanceWrite,
    target: Option<Value>,
) -> Result<Vec<ProviderInstance>, String> {
    let (method, mut params) = match write {
        ProviderInstanceWrite::Upsert(instance) => (
            methods::UPSERT_PROVIDER_INSTANCE,
            json!({ "instance": instance }),
        ),
        ProviderInstanceWrite::Delete { instance_id } => (
            methods::DELETE_PROVIDER_INSTANCE,
            json!({ "instanceId": instance_id }),
        ),
    };
    if let Some(target) = target {
        params["targetDeviceId"] = target;
    }
    engine
        .client()
        .call_as::<ProviderInstancesSnapshot>(method, params)
        .await
        .map(|snapshot| snapshot.instances)
        .map_err(|err| err.to_string())
}

fn with_target(mut params: Value, target: Option<Value>) -> Value {
    if let Some(target) = target {
        params["targetDeviceId"] = target;
    }
    params
}

pub async fn search_registry(
    engine: &EngineHandle,
    query: &str,
    target: Option<Value>,
) -> Result<Vec<AcpRegistryAgent>, String> {
    engine
        .client()
        .call_as(
            methods::SEARCH_ACP_REGISTRY,
            with_target(json!({ "query": query }), target),
        )
        .await
        .map_err(|err| err.to_string())
}

pub async fn prepare_agent(
    engine: &EngineHandle,
    agent_id: &str,
    target: Option<Value>,
) -> Result<AcpPreparedAgent, String> {
    engine
        .client()
        .call_as(
            methods::PREPARE_ACP_AGENT,
            with_target(json!({ "agentId": agent_id }), target),
        )
        .await
        .map_err(|err| err.to_string())
}

pub async fn auth_methods(
    engine: &EngineHandle,
    instance_id: &str,
    target: Option<Value>,
) -> Result<Vec<AcpAuthMethod>, String> {
    engine
        .client()
        .call_as(
            methods::LIST_ACP_AUTH_METHODS,
            with_target(json!({ "instanceId": instance_id }), target),
        )
        .await
        .map_err(|err| err.to_string())
}

pub enum InstanceIcon {
    Brand(&'static str, Option<gpui::Hsla>),
    File(gpui::SharedString),
}

impl InstanceIcon {
    pub fn of(instance: &ProviderInstance) -> Self {
        if let InstanceConfig::Acp {
            icon_path: Some(path),
            ..
        } = &instance.config
        {
            return InstanceIcon::File(path.clone().into());
        }
        let (mark, tint) = crate::pickers::harness_brand_icon(instance.driver);
        InstanceIcon::Brand(mark, tint)
    }

    pub fn registry(icon_path: Option<&str>) -> Self {
        match icon_path {
            Some(path) => InstanceIcon::File(path.to_owned().into()),
            None => InstanceIcon::Brand(crate::icons::BOT, None),
        }
    }

    pub fn svg(&self, color: gpui::Hsla) -> gpui::Svg {
        use gpui::Styled;
        match self {
            InstanceIcon::Brand(mark, tint) => {
                crate::icons::icon(mark).text_color(tint.unwrap_or(color))
            }
            InstanceIcon::File(path) => gpui::svg()
                .external_path(path.clone())
                .flex_none()
                .text_color(color),
        }
    }
}

pub fn acp_source_summary(instance: &ProviderInstance) -> Option<String> {
    match &instance.config {
        InstanceConfig::Acp {
            source:
                AcpSource::Registry {
                    agent_id,
                    version,
                    distribution,
                },
            ..
        } => Some(format!(
            "{agent_id} · v{version} · {}",
            distribution_name(*distribution)
        )),
        InstanceConfig::Acp {
            source: AcpSource::Local { command, .. },
            ..
        } => Some(command.clone()),
        InstanceConfig::Native { .. } => None,
    }
}

pub fn uses_registry_agent(instances: &[ProviderInstance], id: &str) -> bool {
    instances.iter().any(|instance| {
        matches!(
            &instance.config,
            InstanceConfig::Acp {
                source: AcpSource::Registry { agent_id, .. },
                ..
            } if agent_id == id
        )
    })
}

pub fn distribution_name(distribution: AcpDistribution) -> &'static str {
    match distribution {
        AcpDistribution::Npx => "npx",
        AcpDistribution::Uvx => "uvx",
        AcpDistribution::Binary => "binary",
    }
}

pub fn accent_color(accent: &str) -> Option<gpui::Hsla> {
    let hex = accent.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16)
        .ok()
        .map(|rgb| gpui::rgb(rgb).into())
}

pub fn instance_accent(instance: &ProviderInstance) -> Option<gpui::Hsla> {
    instance.accent.as_deref().and_then(accent_color)
}

pub fn has_own_login(instance: &ProviderInstance) -> bool {
    matches!(
        instance.driver,
        HarnessId::ClaudeCode | HarnessId::Codex | HarnessId::Pi
    ) && matches!(
        &instance.config,
        InstanceConfig::Native {
            home_dir: Some(_),
            ..
        }
    )
}

fn is_reserved(id: &str) -> bool {
    serde_json::from_value::<HarnessId>(Value::String(id.to_ascii_lowercase())).is_ok()
}

fn wire_slug(driver: HarnessId) -> String {
    serde_json::to_value(driver)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// T3 copy: a bare driver slug is the default instance, so it reads as taken.
pub fn instance_id_error(id: &str, taken: impl Fn(&str) -> bool) -> Option<String> {
    let exists = || Some(format!("An instance named '{id}' already exists."));
    match validate_instance_id(id) {
        Err(_) if is_reserved(id) => exists(),
        Err(error) => Some(error),
        Ok(()) if taken(id) => exists(),
        Ok(()) => None,
    }
}

/// T3: the driver's own name derives from the bare slug, which is always taken.
pub fn default_instance_id(
    driver: HarnessId,
    driver_name: &str,
    label: &str,
    taken: impl Fn(&str) -> bool,
) -> String {
    let unavailable = |id: &str| taken(id) || is_reserved(id);
    if label.trim().is_empty() || label.trim() == driver_name {
        let base = wire_slug(driver);
        return (2..)
            .map(|n| format!("{base}_{n}"))
            .find(|id| !unavailable(id))
            .unwrap_or_default();
    }
    zeron_proto::derive_instance_id(driver, label, unavailable).unwrap_or_default()
}

pub fn next_numbered(
    driver: HarnessId,
    driver_name: &str,
    taken: impl Fn(&str) -> bool,
) -> (String, String) {
    let base = wire_slug(driver);
    let n = (2..)
        .find(|n| {
            let id = format!("{base}_{n}");
            !taken(&id) && !is_reserved(&id)
        })
        .unwrap_or(2);
    (format!("{driver_name} {n}"), format!("{base}_{n}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_follow_t3_derivation_and_copy() {
        let none = |_: &str| false;
        assert_eq!(
            default_instance_id(HarnessId::Codex, "Codex", "Codex", none),
            "codex_2"
        );
        assert_eq!(
            default_instance_id(HarnessId::Codex, "Codex", "", |id| id == "codex_2"),
            "codex_3"
        );
        assert_eq!(
            default_instance_id(HarnessId::Codex, "Codex", "Codex Work", none),
            "codex_codex_work"
        );
        assert_eq!(
            default_instance_id(HarnessId::Codex, "Codex", "Work", |id| id == "codex_work"),
            "codex_work_2"
        );
        assert_eq!(
            default_instance_id(HarnessId::Codex, "Codex", "日本", none),
            ""
        );
        assert_eq!(
            instance_id_error("codex", none).as_deref(),
            Some("An instance named 'codex' already exists.")
        );
        assert_eq!(
            instance_id_error("codex_work", |id| id == "codex_work").as_deref(),
            Some("An instance named 'codex_work' already exists.")
        );
        assert_eq!(
            instance_id_error("", none).as_deref(),
            Some("Instance ID is required.")
        );
        assert!(instance_id_error("2work", none).is_some());
        assert_eq!(instance_id_error("codex_work", none), None);
        assert_eq!(
            next_numbered(HarnessId::Codex, "Codex", |id| id == "codex_2"),
            ("Codex 3".to_string(), "codex_3".to_string())
        );
        assert_eq!(accent_color("#3e9cf0"), Some(gpui::rgb(0x3e9cf0).into()));
        assert_eq!(accent_color("blue"), None);
    }
}
