use async_trait::async_trait;
use zeron_proto::PermissionMode;

use super::approval_gate::ApprovalDecision;
use super::backend::ComputerError;
use super::manager::{ComputerConsentCard, ComputerManagerSeams};

pub struct EngineComputerSeams;

#[async_trait]
impl ComputerManagerSeams for EngineComputerSeams {
    async fn caller(
        &self,
        _chat_id: &str,
        _turn_id: Option<&str>,
    ) -> Result<Option<PermissionMode>, ComputerError> {
        Ok(None)
    }

    async fn publish_consent(
        &self,
        _card: ComputerConsentCard,
        _decision: Option<ApprovalDecision>,
    ) -> Result<(), String> {
        Err("Computer consent cards are not wired to chats yet.".to_owned())
    }

    async fn disclose_control(&self, _chat_id: &str, _turn_id: Option<&str>) {}

    fn generation_bumped(&self, _chat_id: &str, _generation: u64) {}
}
