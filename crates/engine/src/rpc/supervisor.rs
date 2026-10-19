use super::*;
use crate::supervisor::SupervisorStore;
use zeron_proto::supervisor::{CHAT_PREFIX, CHAT_TITLE, SupervisorSettings};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnsureSupervisorParams {
    config: ChatConfig,
}

fn failed(error: crate::EngineError) -> RpcError {
    RpcError::Failed(error.to_string())
}

impl EngineRpc {
    fn supervisor_store(&self) -> Result<&SupervisorStore, RpcError> {
        self.doc_host
            .supervisor()
            .ok_or_else(|| RpcError::Failed("This engine has no Supervisor".into()))
    }

    pub(super) async fn supervisor_rpc(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<RpcReply, RpcError> {
        match method {
            methods::ENSURE_SUPERVISOR => {
                let p: EnsureSupervisorParams = parse_params(params)?;
                let chat_id = self.ensure_supervisor(p.config)?;
                RpcReply::value(&serde_json::json!({ "chatId": chat_id }))
            }
            methods::WATCH_SUPERVISOR => Ok(RpcReply::Stream(watch_stream(
                self.supervisor_store()?.watch(),
            ))),
            methods::GET_SUPERVISOR_SETTINGS => {
                RpcReply::value(&self.registry.supervisor_settings())
            }
            methods::SET_SUPERVISOR_SETTINGS => {
                let p: SupervisorSettings = parse_params(params)?;
                self.registry.set_supervisor_settings(p);
                self.supervisor_store()?.poke();
                RpcReply::value(&self.registry.supervisor_settings())
            }
            _ => Err(RpcError::UnknownMethod(method.to_string())),
        }
    }

    fn ensure_supervisor(&self, config: ChatConfig) -> Result<String, RpcError> {
        let store = self.supervisor_store()?;
        let _ensuring = store.ensuring();
        let device = self.doc_host.device_id().to_string();
        let chats = self.workspace.read_chats().map_err(failed)?;
        if let Some(chat) = crate::doc_host::newest_supervisor(&chats, &device) {
            return Ok(chat.id.clone());
        }
        let chat_id = format!("{CHAT_PREFIX}{}", uuid::Uuid::new_v4());
        self.workspace
            .create_chat(&chat_id, None, Some(&device), Some(config), None)
            .map_err(failed)?;
        self.workspace
            .rename_chat(&chat_id, CHAT_TITLE)
            .map_err(failed)?;
        store.set_chat_id(Some(chat_id.clone()));
        store.poke();
        Ok(chat_id)
    }

    pub(super) fn set_mission_supervised(
        &self,
        task_id: &str,
        supervised: bool,
    ) -> Result<(), RpcError> {
        self.supervisor_store()?
            .set_supervised(task_id, supervised)
            .map_err(failed)
    }
}
