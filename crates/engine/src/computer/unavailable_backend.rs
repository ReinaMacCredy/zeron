use async_trait::async_trait;
use serde_json::Value;
use zeron_proto::computer::{
    ComputerApp, ComputerAvailability, ComputerCapabilities, ComputerHealth, ComputerHealthFailure,
    ComputerHealthStatus, ComputerInputModifier, ComputerLaunchAppResult, ComputerPoint,
    ComputerRect, ComputerScreenSize, ComputerScreenshot, ComputerState, ComputerVerifyStateResult,
    ComputerWindow, ComputerZoomResult,
};

use super::backend::{
    ComputerAccessibilityTreeSnapshot, ComputerBackend, ComputerBackendActionResult,
    ComputerBackendError, ComputerBackendMethod, ComputerBrowserBackend, ComputerBrowserCall,
    ComputerBrowserCallResult, ComputerCaptureRequest, ComputerCursorReading, ComputerError,
    ComputerFrameListener, ComputerLaunchOptions, ComputerMenuBackendTarget,
    ComputerResolvedTarget, ComputerStateOptions, ComputerTextRange, DEFAULT_COMPUTER_ID,
    NO_COMPUTER_CAPABILITIES, clamp_computer_message,
};

const FALLBACK_MESSAGE: &str = "The Zeron computer backend is unavailable for an unstated reason.";

#[derive(Debug, Clone, Default)]
pub struct UnavailableComputerBackendOptions {
    pub computer_id: Option<String>,
    pub now_ms: Option<i64>,
    pub availability: Option<ComputerAvailability>,
}

pub struct UnavailableComputerBackend {
    computer_id: String,
    message: String,
    at: String,
    availability: Option<ComputerAvailability>,
    browser: UnavailableBrowser,
}

struct UnavailableBrowser {
    message: String,
}

impl UnavailableComputerBackend {
    pub fn new(message: &str, options: UnavailableComputerBackendOptions) -> Self {
        let message = clamp_computer_message(message, FALLBACK_MESSAGE);
        let at = options
            .now_ms
            .and_then(chrono::DateTime::from_timestamp_millis)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        Self {
            computer_id: options
                .computer_id
                .unwrap_or_else(|| DEFAULT_COMPUTER_ID.to_owned()),
            browser: UnavailableBrowser {
                message: message.clone(),
            },
            message,
            at,
            availability: options.availability,
        }
    }

    fn refuse<T>(&self) -> Result<T, ComputerError> {
        Err(refusal(&self.message))
    }
}

fn refusal(message: &str) -> ComputerError {
    ComputerBackendError::new(message).retryable(false).into()
}

#[async_trait]
impl ComputerBrowserBackend for UnavailableBrowser {
    async fn call(
        &self,
        _call: ComputerBrowserCall,
    ) -> Result<ComputerBrowserCallResult, ComputerError> {
        Err(refusal(&self.message))
    }
}

#[async_trait]
impl ComputerBackend for UnavailableComputerBackend {
    fn computer_id(&self) -> &str {
        &self.computer_id
    }

    fn supports(&self, method: ComputerBackendMethod) -> bool {
        use ComputerBackendMethod as M;
        matches!(
            method,
            M::ListApps
                | M::SetWindowFrame
                | M::InvokeMenu
                | M::SetWindowMinimized
                | M::SetAppVisibility
                | M::VerifyState
                | M::ZoomWindow
                | M::GetAccessibilityTree
                | M::GetCursorPosition
                | M::KillApp
                | M::TripleClick
        )
    }

    async fn availability(&self, _refresh: bool) -> Result<ComputerAvailability, ComputerError> {
        Ok(self
            .availability
            .clone()
            .unwrap_or_else(|| ComputerAvailability::BackendUnavailable {
                message: self.message.clone(),
            }))
    }

    async fn probe_availability(&self) -> Result<ComputerAvailability, ComputerError> {
        self.availability(false).await
    }

    fn health(&self) -> ComputerHealth {
        ComputerHealth {
            status: ComputerHealthStatus::Unavailable,
            consecutive_failures: 1,
            reconnects: 0,
            last_failure: Some(ComputerHealthFailure {
                message: self.message.clone(),
                at: self.at.clone(),
            }),
            capture_available: false,
            background_input_degraded: None,
        }
    }

    fn capabilities(&self) -> ComputerCapabilities {
        NO_COMPUTER_CAPABILITIES
    }

    async fn list_windows(&self) -> Result<Vec<ComputerWindow>, ComputerError> {
        self.refuse()
    }

    async fn get_screen_size(&self) -> Result<ComputerScreenSize, ComputerError> {
        self.refuse()
    }

    async fn get_state(
        &self,
        _options: ComputerStateOptions,
    ) -> Result<ComputerState, ComputerError> {
        self.refuse()
    }

    async fn capture_screenshot(
        &self,
        _request: ComputerCaptureRequest,
    ) -> Result<ComputerScreenshot, ComputerError> {
        self.refuse()
    }

    async fn launch_app(
        &self,
        _app: &str,
        _args: &[String],
        _options: ComputerLaunchOptions,
    ) -> Result<ComputerLaunchAppResult, ComputerError> {
        self.refuse()
    }

    async fn list_apps(&self) -> Result<Vec<ComputerApp>, ComputerError> {
        self.refuse()
    }

    async fn set_window_frame(
        &self,
        _window_id: &str,
        _frame: ComputerRect,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn invoke_menu(
        &self,
        _target: &ComputerMenuBackendTarget,
        _path: &[String],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn set_window_minimized(
        &self,
        _window_id: &str,
        _minimized: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn set_app_visibility(
        &self,
        _pid: u32,
        _hidden: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn verify_state(
        &self,
        _window_id: &str,
        _expect: &[Value],
    ) -> Result<ComputerVerifyStateResult, ComputerError> {
        self.refuse()
    }

    async fn zoom_window(
        &self,
        _window_id: &str,
        _region: ComputerRect,
    ) -> Result<ComputerZoomResult, ComputerError> {
        self.refuse()
    }

    async fn get_accessibility_tree(
        &self,
        _window_id: Option<&str>,
    ) -> Result<ComputerAccessibilityTreeSnapshot, ComputerError> {
        self.refuse()
    }

    async fn get_cursor_position(
        &self,
        _window_id: Option<&str>,
    ) -> Result<ComputerCursorReading, ComputerError> {
        self.refuse()
    }

    async fn kill_app(&self, _pid: u32) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn click(
        &self,
        _point: ComputerPoint,
        _window_id: Option<&str>,
        _modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn double_click(
        &self,
        _point: ComputerPoint,
        _window_id: Option<&str>,
        _modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn triple_click(
        &self,
        _point: ComputerPoint,
        _window_id: Option<&str>,
        _modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn right_click(
        &self,
        _point: ComputerPoint,
        _window_id: Option<&str>,
        _modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn move_cursor(
        &self,
        _point: ComputerPoint,
        _window_id: Option<&str>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn drag(
        &self,
        _from: ComputerPoint,
        _to: ComputerPoint,
        _duration_ms: f64,
        _window_id: Option<&str>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn scroll(
        &self,
        _point: Option<ComputerPoint>,
        _delta_x: f64,
        _delta_y: f64,
        _window_id: Option<&str>,
        _modifiers: &[ComputerInputModifier],
        _target: Option<&ComputerResolvedTarget>,
        _prepared_bounds: Option<ComputerRect>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn type_text(
        &self,
        _text: &str,
        _window_id: Option<&str>,
        _target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn press_key(
        &self,
        _key: &str,
        _window_id: Option<&str>,
        _target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn hotkey(
        &self,
        _keys: &[String],
        _window_id: Option<&str>,
        _target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn set_value(
        &self,
        _target: &ComputerResolvedTarget,
        _value: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn perform_action(
        &self,
        _target: &ComputerResolvedTarget,
        _action: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn select_text(
        &self,
        _target: &ComputerResolvedTarget,
        _range: ComputerTextRange,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.refuse()
    }

    async fn attach_stream(&self, _listener: ComputerFrameListener) -> Result<(), ComputerError> {
        self.refuse()
    }

    async fn detach_stream(&self) -> Result<(), ComputerError> {
        Ok(())
    }

    fn browser(&self) -> Option<&dyn ComputerBrowserBackend> {
        Some(&self.browser)
    }

    async fn dispose(&self) {}
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use zeron_proto::computer::ComputerHealthFailure;

    use super::*;

    const REASON: &str = "No computer backend is available.";

    fn backend(options: UnavailableComputerBackendOptions) -> UnavailableComputerBackend {
        UnavailableComputerBackend::new(REASON, options)
    }

    fn origin() -> ComputerPoint {
        ComputerPoint { x: 0.0, y: 0.0 }
    }

    #[tokio::test]
    async fn reports_the_same_sentence_through_availability_health_and_every_action() {
        let backend = backend(UnavailableComputerBackendOptions {
            now_ms: Some(1_700_000_000_000),
            ..Default::default()
        });
        assert_eq!(
            backend.availability(false).await.unwrap(),
            ComputerAvailability::BackendUnavailable {
                message: REASON.into()
            }
        );
        assert_eq!(
            backend.health(),
            ComputerHealth {
                status: ComputerHealthStatus::Unavailable,
                consecutive_failures: 1,
                reconnects: 0,
                last_failure: Some(ComputerHealthFailure {
                    message: REASON.into(),
                    at: "2023-11-14T22:13:20.000Z".into(),
                }),
                capture_available: false,
                background_input_degraded: None,
            }
        );
        assert_eq!(
            backend.list_windows().await.unwrap_err().to_string(),
            REASON
        );
        assert_eq!(
            backend
                .click(origin(), None, &[])
                .await
                .unwrap_err()
                .to_string(),
            REASON
        );
        let target = ComputerResolvedTarget {
            target: zeron_proto::computer::ComputerTarget::default().into(),
            point: origin(),
            node: serde_json::from_value(serde_json::json!({
                "role": "AXTextField",
                "label": null,
                "value": null,
                "description": null,
                "frame": { "x": 0, "y": 0, "width": 1, "height": 1 },
                "activationPoint": null,
                "onScreen": true,
                "windowId": null,
                "children": [],
            }))
            .unwrap(),
        };
        assert_eq!(
            backend
                .select_text(
                    &target,
                    ComputerTextRange {
                        start: 0,
                        length: 0
                    }
                )
                .await
                .unwrap_err()
                .to_string(),
            REASON
        );
        let browser = backend.browser().unwrap();
        let refused = browser
            .call(ComputerBrowserCall {
                name: "browser_click".into(),
                args: serde_json::Map::new(),
                task: zeron_proto::computer::CuaComputerTask {
                    thread_id: "t".into(),
                    turn_id: None,
                    label: None,
                },
                mutation: true,
                signal: None,
            })
            .await
            .unwrap_err();
        assert_eq!(refused.to_string(), REASON);
    }

    #[tokio::test]
    async fn refuses_non_retryably_because_nothing_about_a_missing_backend_will_change() {
        let error = backend(UnavailableComputerBackendOptions::default())
            .get_screen_size()
            .await
            .unwrap_err();
        assert!(matches!(error, ComputerError::Backend(_)));
        assert!(!error.retryable());
    }

    #[test]
    fn advertises_no_capabilities_so_the_panel_offers_nothing_it_cannot_do() {
        assert_eq!(
            backend(UnavailableComputerBackendOptions::default()).capabilities(),
            NO_COMPUTER_CAPABILITIES
        );
    }

    #[test]
    fn answers_to_the_shared_desktop_id() {
        assert_eq!(
            backend(UnavailableComputerBackendOptions::default()).computer_id(),
            "desktop"
        );
        assert_eq!(
            backend(UnavailableComputerBackendOptions {
                computer_id: Some("custom".into()),
                ..Default::default()
            })
            .computer_id(),
            "custom"
        );
    }

    #[tokio::test]
    async fn degrades_an_empty_reason_rather_than_emitting_a_message_the_contract_rejects() {
        let availability = UnavailableComputerBackend::new("   ", Default::default())
            .availability(false)
            .await
            .unwrap();
        let ComputerAvailability::BackendUnavailable { message } = availability else {
            panic!("expected backend-unavailable");
        };
        assert!(!message.is_empty());
        let wire =
            serde_json::to_value(ComputerAvailability::BackendUnavailable { message }).unwrap();
        assert!(serde_json::from_value::<ComputerAvailability>(wire).is_ok());
    }

    #[tokio::test]
    async fn detaching_and_disposing_are_no_ops_so_a_failed_boot_still_tears_down_cleanly() {
        let backend = backend(UnavailableComputerBackendOptions::default());
        backend.on_event(Arc::new(|_| {})).unsubscribe();
        assert!(backend.detach_stream().await.is_ok());
        backend.dispose().await;
    }

    #[tokio::test]
    async fn keeps_a_supplied_verdict_for_unsupported_platforms() {
        let verdict = ComputerAvailability::UnsupportedPlatform {
            platform: "ios".into(),
        };
        let backend = backend(UnavailableComputerBackendOptions {
            availability: Some(verdict.clone()),
            ..Default::default()
        });
        assert_eq!(backend.probe_availability().await.unwrap(), verdict);
        assert!(backend.supports(ComputerBackendMethod::ListApps));
        assert!(!backend.supports(ComputerBackendMethod::Provision));
    }
}
