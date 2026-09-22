use async_trait::async_trait;
use zeron_proto::computer::{ComputerHostCall, CuaReply};

/// The desktop host that owns `cua-driver`: the UI implements it, and the
/// engine's Computer backend calls through it.
#[async_trait]
pub trait ComputerHost: Send + Sync + 'static {
    async fn request(&self, call: ComputerHostCall) -> CuaReply;
    fn cancel(&self, correlation_id: &str);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use zeron_proto::computer::ComputerHostMethod;

    #[derive(Default)]
    struct Recorder {
        cancelled: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ComputerHost for Recorder {
        async fn request(&self, call: ComputerHostCall) -> CuaReply {
            CuaReply {
                ok: call.method == ComputerHostMethod::Probe,
                ..CuaReply::default()
            }
        }

        fn cancel(&self, correlation_id: &str) {
            self.cancelled.lock().unwrap().push(correlation_id.into());
        }
    }

    #[tokio::test]
    async fn is_usable_as_a_shared_trait_object() {
        let recorder = Arc::new(Recorder::default());
        let host: Arc<dyn ComputerHost> = recorder.clone();
        let reply = host
            .request(ComputerHostCall {
                correlation_id: "c1".into(),
                method: ComputerHostMethod::Probe,
                name: None,
                args: serde_json::Value::Null,
                task: None,
                delivery_mode: None,
                model_observation: None,
            })
            .await;
        assert!(reply.ok);
        host.cancel("c1");
        assert_eq!(*recorder.cancelled.lock().unwrap(), vec!["c1".to_string()]);
    }
}
