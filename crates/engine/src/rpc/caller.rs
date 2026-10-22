use std::sync::{Arc, Mutex, PoisonError};

use super::*;

const UNBOUND_CALLER: &str = "fromChatId needs a connection bound to that chat's run (BindRun): only the chat's own zeron mcp speaks for it";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BindRunParams {
    chat_id: String,
    run_secret: String,
}

/// One connection to the engine: after `BindRun` it speaks for that chat and no other.
struct ConnectionRpc {
    rpc: EngineRpc,
    run: Mutex<Option<(String, String)>>,
}

impl ConnectionRpc {
    fn bound(&self) -> Option<(String, String)> {
        self.run
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn check(&self, params: &serde_json::Value) -> Result<(), RpcError> {
        let named = |key: &str| params.get(key).and_then(serde_json::Value::as_str);
        let (from, owner) = (named("fromChatId"), named("ownerChatId"));
        if from.is_none() && owner.is_none() {
            return Ok(());
        }
        let Some((chat, secret)) = self.bound() else {
            return match from {
                Some(_) => Err(RpcError::Failed(UNBOUND_CALLER.into())),
                None => Ok(()),
            };
        };
        if [from, owner].into_iter().flatten().any(|id| id != chat) {
            return Err(RpcError::Failed(format!(
                "this connection speaks for thread {chat} only"
            )));
        }
        if self.rpc.sessions.computer_run(&chat, &secret).is_none() {
            return Err(RpcError::Failed(format!(
                "thread {chat}'s run has ended, so its tools no longer act for it"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl RpcService for ConnectionRpc {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<RpcReply, RpcError> {
        if method == methods::BIND_RUN {
            let p: BindRunParams = parse_params(params)?;
            if self
                .rpc
                .sessions
                .computer_run(&p.chat_id, &p.run_secret)
                .is_none()
            {
                return Err(RpcError::Failed(format!(
                    "no live run of thread {} holds that secret",
                    p.chat_id
                )));
            }
            let mut run = self.run.lock().unwrap_or_else(PoisonError::into_inner);
            if run.as_ref().is_some_and(|(chat, _)| *chat != p.chat_id) {
                return Err(RpcError::Failed(
                    "this connection is already bound to another thread".into(),
                ));
            }
            *run = Some((p.chat_id, p.run_secret));
            return RpcReply::value(&serde_json::json!({}));
        }
        self.check(&params)?;
        self.rpc.handle(method, params).await
    }
}

pub(super) fn connection(rpc: &EngineRpc) -> Arc<dyn RpcService> {
    Arc::new(ConnectionRpc {
        rpc: rpc.clone(),
        run: Mutex::default(),
    })
}

impl EngineRpc {
    /// A mission belongs to its lead: another thread may not take it over, drop it, finish it or delete it.
    pub(super) fn refuse_stranger(
        &self,
        task_id: &str,
        from: Option<&str>,
    ) -> Result<(), RpcError> {
        let (Some(from), Some(task)) = (from, self.workspace.task(task_id)) else {
            return Ok(());
        };
        match task.chat_id.as_deref() {
            Some(lead) if task.mode == zeron_proto::TaskMode::Orchestrated && lead != from => {
                Err(RpcError::Failed(
                    "Only this mission's lead thread or the person changes it, reassigns it or deletes it".into(),
                ))
            }
            _ => Ok(()),
        }
    }

    pub(super) fn refuse_step_as_lead(&self, task_id: &str, chat_id: &str) -> Result<(), RpcError> {
        if self
            .workspace
            .task_steps_of(task_id)
            .iter()
            .any(|step| step.chat_id.as_deref() == Some(chat_id))
        {
            return Err(RpcError::Failed(
                "A step thread of this mission cannot take the mission over".into(),
            ));
        }
        Ok(())
    }
}
