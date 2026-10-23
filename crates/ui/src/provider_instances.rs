use std::time::Duration;

use gpui::{App, AsyncApp, Context, Task, WeakEntity};
use serde_json::{Value, json};
use zeron_proto::{
    AcpAuthMethod, AcpPreparedAgent, AcpRegistryAgent, AgentLoginPoll, AgentLoginStatus, Chat,
    HarnessId, ProviderInstance, ProviderInstancesSnapshot,
};
use zeron_rpc::methods;

use crate::state::{AppState, EngineHandle};

#[derive(Default)]
pub struct ProviderInstancesChanged {
    device_id: Option<String>,
    instances: Vec<ProviderInstance>,
}

impl gpui::Global for ProviderInstancesChanged {}

// Remote-device writes only; the local list comes from the watch.
pub fn bump_provider_instances(cx: &mut App) {
    cx.default_global::<ProviderInstancesChanged>();
}

pub fn local_instances(cx: &App) -> Option<&[ProviderInstance]> {
    let local = cx.try_global::<ProviderInstancesChanged>()?;
    local.device_id.as_ref().map(|_| local.instances.as_slice())
}

// Only an icon file counts, so builtin harnesses keep their model-maker marks.
pub fn chat_icon(chat: &Chat, cx: &App) -> Option<InstanceIcon> {
    let id = chat.config.as_ref()?.instance_id.as_deref()?;
    let local = cx.try_global::<ProviderInstancesChanged>()?;
    if local.device_id.as_deref() != Some(chat.device_id.as_str()) {
        return None;
    }
    local
        .instances
        .iter()
        .find(|instance| instance.id == id)
        .map(InstanceIcon::of)
        .filter(|icon| matches!(icon, InstanceIcon::File(_)))
}

pub fn spawn_watch(cx: &mut Context<AppState>, handle: EngineHandle) -> Task<()> {
    let device_id = handle.engine_info().device_id.clone();
    // Side chat states attach the same engine; only a different one drops the list.
    if cx
        .try_global::<ProviderInstancesChanged>()
        .and_then(|local| local.device_id.as_ref())
        != Some(&device_id)
    {
        cx.set_global(ProviderInstancesChanged::default());
    }
    cx.spawn(async move |this, cx| {
        const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
        loop {
            match handle
                .client()
                .subscribe(methods::WATCH_PROVIDER_INSTANCES, json!({}))
                .await
            {
                Ok(mut rx) => {
                    while let Some(value) = rx.recv().await {
                        let instances = match serde_json::from_value::<ProviderInstancesSnapshot>(
                            value,
                        ) {
                            Ok(snapshot) => snapshot.instances,
                            Err(err) => {
                                tracing::warn!(error = %err, "dropping malformed instances frame");
                                continue;
                            }
                        };
                        let alive = this.update(cx, |_, cx| {
                            let current = cx.global::<ProviderInstancesChanged>();
                            if current.device_id.as_ref() == Some(&device_id)
                                && current.instances == instances
                            {
                                return;
                            }
                            cx.set_global(ProviderInstancesChanged {
                                device_id: Some(device_id.clone()),
                                instances,
                            });
                            cx.notify();
                        });
                        if alive.is_err() {
                            return;
                        }
                    }
                }
                Err(err) => {
                    tracing::debug!(error = %err, "instances watch unavailable; retrying");
                }
            }
            if this.update(cx, |_, _| {}).is_err() {
                return;
            }
            cx.background_executor().timer(RETRY_DELAY).await;
        }
    })
}

pub enum ProviderInstanceWrite {
    Upsert(ProviderInstance),
    Delete { instance_id: String },
}

pub async fn list(
    engine: &EngineHandle,
    target: Option<Value>,
) -> Result<Vec<ProviderInstance>, String> {
    engine
        .client()
        .call_as::<ProviderInstancesSnapshot>(
            methods::LIST_PROVIDER_INSTANCES,
            with_target(json!({}), target),
        )
        .await
        .map(|snapshot| snapshot.instances)
        .map_err(|err| err.to_string())
}

pub async fn write(
    engine: &EngineHandle,
    write: ProviderInstanceWrite,
    target: Option<Value>,
) -> Result<Vec<ProviderInstance>, String> {
    let (method, params) = match write {
        ProviderInstanceWrite::Upsert(instance) => (
            methods::UPSERT_PROVIDER_INSTANCE,
            json!({ "instance": instance }),
        ),
        ProviderInstanceWrite::Delete { instance_id } => (
            methods::DELETE_PROVIDER_INSTANCE,
            json!({ "instanceId": instance_id }),
        ),
    };
    engine
        .client()
        .call_as::<ProviderInstancesSnapshot>(method, with_target(params, target))
        .await
        .map(|snapshot| snapshot.instances)
        .map_err(|err| err.to_string())
}

pub(crate) fn with_target(mut params: Value, target: Option<Value>) -> Value {
    if let Some(target) = target {
        params["targetDeviceId"] = target;
    }
    params
}

pub(crate) enum LoginPoll {
    Done,
    Failed(String),
    Pending {
        url: Option<String>,
        message: Option<String>,
    },
}

pub(crate) fn fold_poll(poll: Result<AgentLoginPoll, String>) -> LoginPoll {
    match poll {
        Ok(poll) => match poll.status {
            AgentLoginStatus::Done => LoginPoll::Done,
            AgentLoginStatus::Error => LoginPoll::Failed(
                poll.message
                    .unwrap_or_else(|| "The sign-in failed.".to_string()),
            ),
            AgentLoginStatus::Pending => LoginPoll::Pending {
                url: poll.url,
                message: poll.message,
            },
        },
        Err(error) => LoginPoll::Failed(format!("Lost track of the sign-in: {error}")),
    }
}

/// Polls an agent login until `on_poll` returns true or its entity is gone.
pub(crate) async fn poll_login<T: 'static>(
    engine: &EngineHandle,
    params: Value,
    this: &WeakEntity<T>,
    cx: &mut AsyncApp,
    mut on_poll: impl FnMut(&mut T, LoginPoll, &mut Context<T>) -> bool,
) {
    loop {
        cx.background_executor()
            .timer(Duration::from_millis(1500))
            .await;
        let poll = engine
            .client()
            .call_as::<AgentLoginPoll>(methods::POLL_AGENT_LOGIN, params.clone())
            .await
            .map_err(|error| error.to_string());
        let poll = fold_poll(poll);
        match this.update(cx, |entity, cx| on_poll(entity, poll, cx)) {
            Ok(false) => {}
            Ok(true) | Err(_) => break,
        }
    }
}

pub(crate) async fn cancel_login(engine: EngineHandle, params: Value) {
    if let Err(error) = engine
        .client()
        .call(methods::CANCEL_AGENT_LOGIN, params)
        .await
    {
        tracing::debug!(%error, "CancelAgentLogin failed (best-effort)");
    }
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
    allow_unverified: bool,
    target: Option<Value>,
) -> Result<AcpPreparedAgent, String> {
    engine
        .client()
        .call_as(
            methods::PREPARE_ACP_AGENT,
            with_target(
                json!({ "agentId": agent_id, "allowUnverified": allow_unverified }),
                target,
            ),
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstanceIcon {
    Brand(&'static str, Option<gpui::Hsla>),
    File(gpui::SharedString),
}

impl InstanceIcon {
    pub fn of(instance: &ProviderInstance) -> Self {
        match instance.icon_path() {
            Some(path) => InstanceIcon::File(path.to_owned().into()),
            None => Self::harness(instance.driver),
        }
    }

    pub fn harness(harness: HarnessId) -> Self {
        let (mark, tint) = crate::pickers::harness_brand_icon(harness);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_parse_only_six_digit_hex() {
        assert_eq!(accent_color("#3e9cf0"), Some(gpui::rgb(0x3e9cf0).into()));
        assert_eq!(accent_color("blue"), None);
    }
}
