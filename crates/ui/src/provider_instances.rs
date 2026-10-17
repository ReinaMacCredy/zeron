use serde_json::{Value, json};
use zeron_proto::{ProviderInstance, ProviderInstancesSnapshot};
use zeron_rpc::methods;

use crate::state::EngineHandle;

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
