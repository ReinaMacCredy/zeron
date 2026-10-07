use std::sync::Arc;

use futures::FutureExt;
use tokio_util::sync::CancellationToken;
use zeron_proto::PermissionMode;
use zeron_proto::computer::{ComputerAllowedApp, ComputerConsentApp};

use super::{
    ComputerCaller, ComputerConsent, ComputerConsentCard, ComputerConsentScope,
    ComputerConsentTarget, ComputerManager,
};
use crate::computer::approval_gate::{AppGrant, ApprovalPublish, ApprovalRequest};
use crate::computer::backend::ComputerError;

pub const COMPUTER_BROWSER_PROFILE_CONSENT_TOOL: &str = "computer_browser_prepare";
pub const COMPUTER_BROWSER_PROFILE_CONSENT_PARAMS: &str = r#"{"strategy":"existing_profile"}"#;
const PROFILE_GRANT_PREFIX: &str = "browser-profile:";

impl ComputerManager {
    pub async fn authorize_browser_profile(
        &self,
        caller: &ComputerCaller,
        signal: CancellationToken,
        pid: u32,
    ) -> Result<ComputerConsent, ComputerError> {
        if self
            .seams
            .caller(&caller.chat_id, caller.turn_id.as_deref())
            .await?
            .is_none()
        {
            return Ok(ComputerConsent::Denied(None));
        }
        let browser = self
            .resolve_consent_app(&ComputerConsentTarget::Pid(pid))
            .await;
        let identity = browser
            .app
            .bundle_id
            .clone()
            .unwrap_or_else(|| browser.key.clone());
        let app = ComputerConsentApp {
            name: format!("{} profile", browser.app.name),
            bundle_id: browser.app.bundle_id.clone(),
            pid: Some(pid),
        };
        let task_consent = caller.turn_id.is_some();
        let scope = if task_consent {
            ComputerConsentScope::Task
        } else {
            ComputerConsentScope::Call
        };
        let seams = self.seams.clone();
        let (chat_id, turn_id, card_app) =
            (caller.chat_id.clone(), caller.turn_id.clone(), app.clone());
        let publish: ApprovalPublish = Arc::new(move |request_id, decision| {
            let seams = seams.clone();
            let card = ComputerConsentCard {
                request_id,
                chat_id: chat_id.clone(),
                turn_id: turn_id.clone(),
                tool_name: COMPUTER_BROWSER_PROFILE_CONSENT_TOOL.to_owned(),
                params_display: COMPUTER_BROWSER_PROFILE_CONSENT_PARAMS.to_owned(),
                scope,
                app: Some(card_app.clone()),
            };
            async move { seams.publish_consent(card, decision).await }.boxed()
        });
        let request = ApprovalRequest {
            chat_id: caller.chat_id.clone(),
            turn_id: caller.turn_id.clone(),
            mode: PermissionMode::Ask,
            signal,
            publish,
            app: Some(AppGrant {
                key: format!("{PROFILE_GRANT_PREFIX}{identity}"),
                always: browser
                    .app
                    .bundle_id
                    .filter(|_| task_consent)
                    .map(|bundle_id| ComputerAllowedApp {
                        bundle_id: format!("{PROFILE_GRANT_PREFIX}{bundle_id}"),
                        name: Some(app.name.clone()),
                    }),
            }),
        };
        let approved = if task_consent {
            self.approval_gate.request_task(request).await?
        } else {
            self.approval_gate.request(request).await?
        };
        Ok(if approved {
            ComputerConsent::Granted
        } else {
            ComputerConsent::Denied(Some(app))
        })
    }
}
