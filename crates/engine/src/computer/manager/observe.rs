use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use zeron_proto::computer::{
    ComputerDeliveryVerification, ComputerPoint, ComputerRect, ComputerScreenshot,
    ComputerScrollDelta, ComputerScrollTelemetry, ComputerTarget, ComputerWindow, CuaEffect,
};

use super::targeting::{ResolvedPointTarget, has_label_fields};
use super::{
    COMPUTER_ACTION_OBSERVER_SETTLE_TIMEOUT_MS, ComputerActionObservation, ComputerCapturedWindow,
    ComputerManager, ComputerScrollOptions, ComputerScrollOutcome, ComputerSettleMode,
    ComputerWaitSettleResult, ObserverSettle, SCROLL_PROBE_PX, SCROLL_PROBE_TRIGGER_PX,
    SCROLL_SETTLE_ARRIVAL_MIN_PX, SCROLL_SETTLE_ARRIVAL_TOLERANCE, agent_thread_id, round2,
    settle_permanently_unsupported, window_not_found_error,
};
use crate::computer::backend::{
    COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION, ComputerBackendActionResult, ComputerBackendMethod,
    ComputerCaptureRequest, ComputerError, ComputerSettleOptions, ComputerStateOptions,
};
use crate::computer::call_context::{
    current_computer_call, mark_computer_call, timed_computer_leg,
};
use crate::computer::element_identity::ObservedComputerTarget;
use crate::computer::geometry::topmost_window_at_point;
use crate::computer::scroll_calibration::js_sign;
use crate::computer::ui_tree_targeting::{
    ComputerTargetError, ComputerTargetErrorCode, resolve_computer_window_target,
};

// macOS adds a ~66x20 "window sharing" pill over the traffic lights while a window is captured;
// it is a new window of the target's pid, never an outcome of the action.
const MIN_OPENED_WINDOW_SIDE: f64 = 28.0;

fn count(name: &str) {
    if let Some(call) = current_computer_call()
        && let Some(timing) = &call.timing
    {
        timing.count(name, 1);
    }
}

fn stacking(window: &ComputerWindow) -> u64 {
    window.stacking_index.unwrap_or(u64::MAX)
}

impl ComputerManager {
    pub async fn wait_for_settle(
        &self,
        window_id: &str,
        timeout_ms: f64,
    ) -> Result<ComputerWaitSettleResult, ComputerError> {
        self.engage_backend();
        self.with_computer_call(async {
            mark_computer_call("computer_wait_settle");
            let windows = self.read_windows().await?;
            if !windows.iter().any(|entry| entry.id == window_id) {
                return Err(window_not_found_error(window_id));
            }
            let timeout = timeout_ms
                .floor()
                .min((COMPUTER_ACTION_OBSERVER_SETTLE_TIMEOUT_MS * 6) as f64)
                .max(0.0);
            let observer = self.state().observer_settle;
            if observer != ObserverSettle::Unsupported
                && self.backend.supports(ComputerBackendMethod::WaitForSettle)
            {
                match self
                    .backend
                    .wait_for_settle(ComputerSettleOptions {
                        window_id: window_id.to_owned(),
                        timeout_ms: timeout,
                        quiet_ms: self.action_settle_ms as f64,
                    })
                    .await
                {
                    Ok(outcome) => {
                        self.state().observer_settle = ObserverSettle::Supported;
                        count(if outcome.settled {
                            "settle_observer_settled"
                        } else {
                            "settle_observer_timeout"
                        });
                        return Ok(ComputerWaitSettleResult {
                            settled: outcome.settled,
                            waited_ms: outcome.waited_ms,
                            events_seen: outcome.events_seen,
                            mode: ComputerSettleMode::Observer,
                        });
                    }
                    Err(error) if settle_permanently_unsupported(&error) => {
                        self.state().observer_settle = ObserverSettle::Unsupported;
                    }
                    Err(error) => return Err(error),
                }
            }
            let waited = timeout.min(self.action_settle_ms as f64).max(0.0);
            tokio::time::sleep(Duration::from_millis(waited as u64)).await;
            Ok(ComputerWaitSettleResult {
                settled: true,
                waited_ms: waited,
                events_seen: None,
                mode: ComputerSettleMode::Fixed,
            })
        })
        .await
    }

    pub async fn capture_focused_window(
        &self,
        max_dimension: Option<u32>,
        agent_focus_only: bool,
    ) -> Result<ComputerCapturedWindow, ComputerError> {
        self.engage_backend();
        if let Some(window) = self.focused_capturable_window(agent_focus_only).await? {
            if let Some(found) = self.denied_match_for_window(&window).await {
                return Err(crate::computer::denylist::ComputerDenylistError::from(found).into());
            }
            return Ok(ComputerCapturedWindow {
                screenshot: self
                    .backend
                    .capture_screenshot(ComputerCaptureRequest::Window {
                        window_id: window.id.clone(),
                        max_dimension,
                    })
                    .await?,
                window_id: Some(window.id),
            });
        }
        self.assert_no_denied_visible_window().await?;
        let screen = self.backend.get_screen_size().await?;
        Ok(ComputerCapturedWindow {
            screenshot: self
                .backend
                .capture_screenshot(ComputerCaptureRequest::Region {
                    region: ComputerRect {
                        x: 0.0,
                        y: 0.0,
                        width: f64::from(screen.width),
                        height: f64::from(screen.height),
                    },
                    max_dimension,
                })
                .await?,
            window_id: None,
        })
    }

    pub async fn capture_action_screenshot(
        &self,
        window_id_hint: Option<&str>,
        action_point: Option<ComputerPoint>,
        settle: bool,
    ) -> Option<ComputerActionObservation> {
        self.with_computer_call(async {
            mark_computer_call("computer_observe");
            if !self.backend.capabilities().capture {
                return Ok(None);
            }
            self.engage_backend();
            if settle && self.action_settle_ms > 0 {
                if self.action_effect_already_proven() {
                    count("settle_skipped");
                } else {
                    timed_computer_leg("settle", self.settle_after_action(window_id_hint)).await;
                }
            }
            Ok(timed_computer_leg(
                "observe",
                self.capture_action_observation(window_id_hint, action_point),
            )
            .await)
        })
        .await
        .ok()
        .flatten()
    }

    fn action_effect_already_proven(&self) -> bool {
        if !self.env.conditional_settle {
            return false;
        }
        let Some(proof) = current_computer_call().and_then(|call| call.take_action_proof()) else {
            return false;
        };
        proof.effect == Some(CuaEffect::Verified)
            || proof.verified == Some(ComputerDeliveryVerification::Confirmed)
    }

    async fn settle_after_action(&self, window_id: Option<&str>) {
        let observer = self.state().observer_settle;
        if let Some(window_id) = window_id
            && observer != ObserverSettle::Unsupported
            && self.backend.supports(ComputerBackendMethod::WaitForSettle)
        {
            match self
                .backend
                .wait_for_settle(ComputerSettleOptions {
                    window_id: window_id.to_owned(),
                    timeout_ms: COMPUTER_ACTION_OBSERVER_SETTLE_TIMEOUT_MS as f64,
                    quiet_ms: self.action_settle_ms as f64,
                })
                .await
            {
                Ok(outcome) => {
                    self.state().observer_settle = ObserverSettle::Supported;
                    count(if outcome.settled {
                        "settle_observer_settled"
                    } else {
                        "settle_observer_timeout"
                    });
                    return;
                }
                Err(error) => {
                    if settle_permanently_unsupported(&error) {
                        self.state().observer_settle = ObserverSettle::Unsupported;
                    }
                    count("settle_observer_unavailable");
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(self.action_settle_ms)).await;
    }

    async fn capture_window(
        &self,
        window_id: &str,
    ) -> Result<ComputerCapturedWindow, ComputerError> {
        Ok(ComputerCapturedWindow {
            screenshot: self
                .backend
                .capture_screenshot(ComputerCaptureRequest::Window {
                    window_id: window_id.to_owned(),
                    max_dimension: Some(COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION),
                })
                .await?,
            window_id: Some(window_id.to_owned()),
        })
    }

    async fn capture_action_observation(
        &self,
        window_id_hint: Option<&str>,
        action_point: Option<ComputerPoint>,
    ) -> Option<ComputerActionObservation> {
        if let Some(hint) = window_id_hint {
            if self.window_is_denied(hint).await.unwrap_or(false) {
                return None;
            }
            return match self.capture_window(hint).await {
                Ok(capture) => Some(self.observe_action_capture(capture).await),
                Err(_) => match self.read_windows().await {
                    Ok(windows) if !windows.iter().any(|window| window.id == hint) => {
                        Some(ComputerActionObservation::TargetWindowClosed)
                    }
                    _ => None,
                },
            };
        }
        if let Some(point) = action_point
            && let Some(window_id) = self.window_id_at_action_point(point).await
        {
            if self.window_is_denied(&window_id).await.unwrap_or(false) {
                return None;
            }
            if let Ok(capture) = self.capture_window(&window_id).await {
                return Some(self.observe_action_capture(capture).await);
            }
        }
        match self
            .capture_focused_window(Some(COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION), true)
            .await
        {
            Ok(capture) => Some(self.observe_action_capture(capture).await),
            Err(_) => None,
        }
    }

    async fn observe_action_capture(
        &self,
        capture: ComputerCapturedWindow,
    ) -> ComputerActionObservation {
        let Some(appeared) = self
            .window_opened_by_action(capture.window_id.as_deref())
            .await
        else {
            return ComputerActionObservation::Captured(capture);
        };
        if self.denied_match_for_window(&appeared).await.is_some() {
            return ComputerActionObservation::Captured(capture);
        }
        match self.capture_window(&appeared.id).await {
            Ok(opened) => ComputerActionObservation::Captured(opened),
            Err(_) => ComputerActionObservation::Captured(capture),
        }
    }

    async fn window_opened_by_action(&self, exclude: Option<&str>) -> Option<ComputerWindow> {
        let baseline = self.state().pre_action_window_ids.clone()?;
        let windows = self.read_windows().await.ok()?;
        let owner = windows
            .iter()
            .find(|window| Some(window.id.as_str()) == exclude)?
            .clone();
        windows
            .into_iter()
            .filter(|window| {
                !baseline.contains(&window.id)
                    && Some(window.id.as_str()) != exclude
                    && match owner.pid {
                        Some(pid) => window.pid == Some(pid),
                        None => owner.app_name.is_some() && window.app_name == owner.app_name,
                    }
                    && window.bounds.as_ref().is_some_and(|bounds| {
                        bounds.width >= MIN_OPENED_WINDOW_SIDE
                            && bounds.height >= MIN_OPENED_WINDOW_SIDE
                    })
                    && window.visible
                    && !window.minimized
            })
            .min_by_key(stacking)
    }

    async fn window_id_at_action_point(&self, point: ComputerPoint) -> Option<String> {
        let windows = self.read_windows().await.ok()?;
        topmost_window_at_point(&windows, point).map(|window| window.id.clone())
    }

    async fn focused_capturable_window(
        &self,
        agent_focus_only: bool,
    ) -> Result<Option<ComputerWindow>, ComputerError> {
        let candidates: Vec<ComputerWindow> = self
            .read_windows()
            .await?
            .into_iter()
            .filter(|window| window.bounds.is_some() && window.visible && !window.minimized)
            .collect();
        let agent_focused = candidates.iter().find(|window| window.focused).cloned();
        if agent_focused.is_some() || agent_focus_only {
            return Ok(agent_focused);
        }
        Ok(candidates
            .iter()
            .find(|window| window.active == Some(true))
            .cloned()
            .or_else(|| {
                candidates
                    .iter()
                    .min_by_key(|window| stacking(window))
                    .cloned()
            }))
    }

    pub async fn scroll(
        &self,
        thread_id: Option<&str>,
        target: Option<ObservedComputerTarget>,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<zeron_proto::computer::ComputerActionResult, ComputerError> {
        let window_id = target
            .as_ref()
            .and_then(|target| target.target.window_id.clone());
        self.with_background_process_control(thread_id, window_id.as_deref(), || async {
            let resolved = timed_computer_leg(
                "resolve",
                self.prepare_scroll_target(target.as_ref(), thread_id),
            )
            .await?;
            let result = self
                .inject_scroll(resolved.as_ref(), delta_x, delta_y, &[])
                .await?;
            Ok(self.action_result(
                thread_id,
                "computer_scroll",
                resolved.as_ref().map(|resolved| resolved.point),
                Some(result),
                resolved
                    .as_ref()
                    .and_then(|resolved| resolved.window_id.as_deref()),
            ))
        })
        .await
    }

    pub async fn scroll_calibrated(
        &self,
        thread_id: Option<&str>,
        target: Option<ObservedComputerTarget>,
        delta_x: f64,
        delta_y: f64,
        options: ComputerScrollOptions,
    ) -> Result<ComputerScrollOutcome, ComputerError> {
        let window_id = target
            .as_ref()
            .and_then(|target| target.target.window_id.clone());
        self.with_background_process_control(thread_id, window_id.as_deref(), || async {
            let attributed = agent_thread_id(thread_id);
            let cursor_point = match (&target, attributed) {
                (None, Some(attributed)) => self
                    .state()
                    .thread(attributed)
                    .and_then(|thread| thread.cursor),
                _ => None,
            };
            let pre_clear_focus = match &target {
                None => self.agent_focus_window_id().await,
                Some(_) => None,
            };
            let resolved = timed_computer_leg(
                "resolve",
                self.prepare_scroll_target(target.as_ref(), thread_id),
            )
            .await?;
            let mut observed_window_id = resolved
                .as_ref()
                .and_then(|resolved| resolved.window_id.clone());
            if observed_window_id.is_none()
                && let Some(point) = resolved.as_ref().map(|resolved| resolved.point)
            {
                observed_window_id = self.window_id_at_action_point(point).await;
            }
            if observed_window_id.is_none()
                && let Some(cursor) = cursor_point
            {
                observed_window_id = self.window_id_at_action_point(cursor).await;
            }
            if observed_window_id.is_none() {
                observed_window_id = pre_clear_focus;
            }
            if observed_window_id.is_none() {
                observed_window_id = self.agent_focus_window_id().await;
            }
            let before = if options.observe {
                self.capture_for_measurement(observed_window_id.as_deref())
                    .await
            } else {
                None
            };
            let column_center = match (&before, &resolved) {
                (Some(before), Some(resolved)) => before.screenshot.region.and_then(|region| {
                    let fraction = (resolved.point.x - region.x) / region.width;
                    (0.0..=1.0).contains(&fraction).then_some(fraction)
                }),
                _ => None,
            };
            let observed_window = match &observed_window_id {
                Some(id) => self
                    .read_windows()
                    .await?
                    .into_iter()
                    .find(|window| &window.id == id),
                None => None,
            };
            let app_key = observed_window.as_ref().and_then(|window| {
                window
                    .app_name
                    .clone()
                    .or_else(|| window.pid.map(|pid| format!("pid:{pid}")))
            });
            let window_key = |route: &str| {
                observed_window_id
                    .as_ref()
                    .map(|id| format!("{id}|{route}"))
            };
            let durable_key = |route: &str| app_key.as_ref().map(|key| format!("{key}|{route}"));
            let planned_route = |leg_delta_x: f64| {
                if resolved
                    .as_ref()
                    .is_some_and(|resolved| resolved.semantic.is_some())
                    && leg_delta_x == 0.0
                    && options.modifiers.is_empty()
                {
                    "ax"
                } else {
                    "wheel"
                }
            };
            let leg_route = |leg: &ComputerBackendActionResult, planned: &'static str| match leg
                .delivery_path
                .as_deref()
            {
                Some(path) if path.starts_with("cua-ax") => "ax",
                Some(_) => "wheel",
                None => planned,
            };
            let measured = |route: &str| {
                self.state()
                    .scroll_gearing
                    .has(window_key(route).as_deref())
                    || self
                        .durable_gearing(durable_key(route).as_deref())
                        .is_some()
            };
            let plan = |route: &str, requested: f64| {
                let fallback = self.durable_gearing(durable_key(route).as_deref());
                self.state()
                    .scroll_gearing
                    .plan(window_key(route).as_deref(), requested, fallback)
            };

            let mut injected_x = 0.0;
            let mut injected_y = 0.0;
            let mut after: Option<ComputerCapturedWindow> = None;
            let mut traveled_y: Option<f64> = None;
            let mut result: ComputerBackendActionResult;
            let mut routes: Vec<&'static str> = Vec::new();

            if let (Some(before), Some(_)) = (&before, &observed_window_id)
                && !measured(planned_route(0.0))
                && delta_y.abs() > SCROLL_PROBE_TRIGGER_PX
            {
                let probe = delta_y.signum() * SCROLL_PROBE_PX;
                let probe_result = self
                    .inject_scroll(resolved.as_ref(), 0.0, probe, &options.modifiers)
                    .await?;
                let probe_injected = probe_result
                    .scroll_delta
                    .map_or(probe, |delta| delta.delta_y);
                injected_y += probe_injected;
                let probe_route = leg_route(&probe_result, planned_route(0.0));
                routes.push(probe_route);
                result = probe_result;
                let probe_leg = self
                    .settle_and_measure(
                        observed_window_id.as_deref(),
                        before,
                        probe_injected,
                        window_key(probe_route).as_deref(),
                        durable_key(probe_route).as_deref(),
                        column_center,
                    )
                    .await;
                after = probe_leg.0;
                let covered = match probe_leg.1 {
                    Some(traveled) if js_sign(traveled) == js_sign(delta_y) => traveled,
                    _ => probe_injected,
                };
                let remainder = if covered.abs() >= delta_y.abs() {
                    0.0
                } else {
                    delta_y - covered
                };
                let leg_x = plan(planned_route(delta_x), delta_x);
                let leg_y = plan(planned_route(delta_x), remainder);
                if leg_x != 0.0 || leg_y != 0.0 {
                    let remainder_result = self
                        .inject_scroll(resolved.as_ref(), leg_x, leg_y, &options.modifiers)
                        .await?;
                    injected_x += remainder_result.scroll_delta.map_or(leg_x, |d| d.delta_x);
                    let leg_injected_y = remainder_result.scroll_delta.map_or(leg_y, |d| d.delta_y);
                    injected_y += leg_injected_y;
                    let remainder_route = leg_route(&remainder_result, planned_route(delta_x));
                    routes.push(remainder_route);
                    result = remainder_result;
                    if let Some(from) = after.clone() {
                        let remainder_leg = self
                            .settle_and_measure(
                                observed_window_id.as_deref(),
                                &from,
                                leg_injected_y,
                                window_key(remainder_route).as_deref(),
                                durable_key(remainder_route).as_deref(),
                                column_center,
                            )
                            .await;
                        after = remainder_leg.0.or(after);
                        traveled_y = match (probe_leg.1, remainder_leg.1) {
                            (Some(probe), Some(rest)) => Some(probe + rest),
                            _ => None,
                        };
                    }
                } else {
                    traveled_y = probe_leg.1;
                }
            } else {
                let route = planned_route(delta_x);
                injected_x = plan(route, delta_x);
                injected_y = plan(route, delta_y);
                result = self
                    .inject_scroll(
                        resolved.as_ref(),
                        injected_x,
                        injected_y,
                        &options.modifiers,
                    )
                    .await?;
                if let Some(delta) = result.scroll_delta {
                    injected_x = delta.delta_x;
                    injected_y = delta.delta_y;
                }
                let actual_route = leg_route(&result, route);
                routes.push(actual_route);
                if let Some(before) = &before {
                    let leg = self
                        .settle_and_measure(
                            observed_window_id.as_deref(),
                            before,
                            injected_y,
                            window_key(actual_route).as_deref(),
                            durable_key(actual_route).as_deref(),
                            column_center,
                        )
                        .await;
                    after = leg.0;
                    traveled_y = leg.1;
                }
            }
            let reported_gearing = match (&observed_window_id, routes.last()) {
                (Some(_), Some(route)) => {
                    let key = window_key(route);
                    let learned = {
                        let state = self.state();
                        state
                            .scroll_gearing
                            .has(key.as_deref())
                            .then(|| state.scroll_gearing.gearing(key.as_deref()))
                    };
                    Some(learned.unwrap_or_else(|| {
                        self.durable_gearing(durable_key(route).as_deref())
                            .unwrap_or(1.0)
                    }))
                }
                _ => None,
            };
            let mut base = self.action_result(
                thread_id,
                "computer_scroll",
                resolved.as_ref().map(|resolved| resolved.point),
                Some(result),
                resolved
                    .as_ref()
                    .and_then(|resolved| resolved.window_id.as_deref()),
            );
            base.scroll = Some(ComputerScrollTelemetry {
                requested: ComputerScrollDelta { delta_x, delta_y },
                injected: ComputerScrollDelta {
                    delta_x: round2(injected_x),
                    delta_y: round2(injected_y),
                },
                limited_to: None,
                traveled_y: traveled_y.map(round2),
                gearing: reported_gearing.map(round2),
                routes: (!routes.is_empty())
                    .then(|| routes.iter().map(|route| (*route).to_owned()).collect()),
            });
            Ok(ComputerScrollOutcome {
                result: base,
                observation: after.map(ComputerActionObservation::Captured),
            })
        })
        .await
    }

    fn durable_gearing(&self, key: Option<&str>) -> Option<f64> {
        super::lock(&self.scroll_gearing_file).get(key)
    }

    async fn prepare_scroll_target(
        &self,
        target: Option<&ObservedComputerTarget>,
        thread_id: Option<&str>,
    ) -> Result<Option<ResolvedPointTarget>, ComputerError> {
        let resolved = match target {
            Some(target) => Some(self.resolve_scroll_point_target(target, thread_id).await?),
            None => None,
        };
        let prepared = resolved
            .as_ref()
            .map(super::targeting::PreparedTarget::from);
        self.prepare_resolved_target(prepared.as_ref(), thread_id)
            .await?;
        Ok(resolved)
    }

    async fn resolve_scroll_point_target(
        &self,
        target: &ObservedComputerTarget,
        thread_id: Option<&str>,
    ) -> Result<ResolvedPointTarget, ComputerError> {
        let plain: &ComputerTarget = &target.target;
        let Some(window_id) = plain
            .window_id
            .as_deref()
            .filter(|_| plain.x.is_none() && plain.y.is_none() && !has_label_fields(plain))
        else {
            return self.resolve_point_target(target, thread_id).await;
        };
        self.assert_window_input_allowed(thread_id, window_id)
            .await?;
        let state = self
            .backend
            .get_state(ComputerStateOptions::default())
            .await?;
        let matched = match &state.root {
            Some(root) => {
                resolve_computer_window_target(root, window_id)?.map(|matched| matched.point)
            }
            None => None,
        };
        if let Some(point) = matched {
            return Ok(ResolvedPointTarget {
                point,
                window_id: Some(window_id.to_owned()),
                covering: None,
                semantic: None,
            });
        }
        let window = match self
            .read_windows()
            .await?
            .into_iter()
            .find(|w| w.id == window_id)
        {
            Some(window) => Some(window),
            None => self.state().last_known_windows.get(window_id).cloned(),
        };
        let Some(window) = window else {
            return Err(window_not_found_error(window_id));
        };
        let Some(bounds) = window.bounds else {
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::Offscreen,
                format!(
                    "This desktop reports no geometry for window {}, so a scroll point inside it cannot be chosen. Scroll at x/y coordinates instead.",
                    super::json_string(window_id)
                ),
                Vec::new(),
            )
            .into());
        };
        Ok(ResolvedPointTarget {
            point: ComputerPoint {
                x: bounds.x + bounds.width / 2.0,
                y: bounds.y + bounds.height / 2.0,
            },
            window_id: Some(window_id.to_owned()),
            covering: None,
            semantic: None,
        })
    }

    async fn inject_scroll(
        &self,
        resolved: Option<&ResolvedPointTarget>,
        delta_x: f64,
        delta_y: f64,
        modifiers: &[zeron_proto::computer::ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let prepared = resolved
            .map(super::targeting::PreparedTarget::from)
            .unwrap_or_default();
        self.inject_scoped("computer_scroll", &prepared, || {
            self.backend.scroll(
                resolved.map(|resolved| resolved.point),
                delta_x,
                delta_y,
                resolved.and_then(|resolved| resolved.window_id.as_deref()),
                modifiers,
                resolved.and_then(|resolved| resolved.semantic.as_ref()),
            )
        })
        .await
    }

    async fn settle_and_measure(
        &self,
        window_id: Option<&str>,
        from: &ComputerCapturedWindow,
        injected_y: f64,
        window_key: Option<&str>,
        app_key: Option<&str>,
        column_center: Option<f64>,
    ) -> (Option<ComputerCapturedWindow>, Option<f64>) {
        if self.action_settle_ms > 0 && injected_y != 0.0 && self.env.conditional_settle {
            let predicted = {
                let state = self.state();
                state
                    .scroll_gearing
                    .has(window_key)
                    .then(|| state.scroll_gearing.gearing(window_key))
            }
            .or_else(|| self.durable_gearing(app_key));
            if let Some(predicted) = predicted {
                let expected = injected_y * predicted;
                if let Some(early) = self.capture_for_measurement(window_id).await {
                    let traveled = self
                        .measure_leg_travel(
                            &from.screenshot,
                            &early.screenshot,
                            injected_y,
                            column_center,
                        )
                        .await;
                    if let Some(traveled) = traveled
                        && (traveled - expected).abs()
                            <= SCROLL_SETTLE_ARRIVAL_MIN_PX
                                .max(expected.abs() * SCROLL_SETTLE_ARRIVAL_TOLERANCE)
                    {
                        self.learn_leg_travel(
                            window_key.or(window_id),
                            app_key,
                            injected_y,
                            Some(traveled),
                        );
                        count("settle_skipped");
                        return (Some(early), Some(traveled));
                    }
                }
            }
        }
        if self.action_settle_ms > 0 {
            timed_computer_leg("settle", self.settle_after_action(window_id)).await;
        }
        let Some(capture) = self.capture_for_measurement(window_id).await else {
            return (None, None);
        };
        let traveled = self
            .measure_leg_travel(
                &from.screenshot,
                &capture.screenshot,
                injected_y,
                column_center,
            )
            .await;
        self.learn_leg_travel(window_key.or(window_id), app_key, injected_y, traveled);
        (Some(capture), traveled)
    }

    async fn measure_leg_travel(
        &self,
        from: &ComputerScreenshot,
        to: &ComputerScreenshot,
        injected_y: f64,
        column_center: Option<f64>,
    ) -> Option<f64> {
        let measured = self.measure_travel(from, to, column_center).await;
        match measured {
            Some(measured)
                if measured != 0.0
                    && injected_y != 0.0
                    && measured.signum() != injected_y.signum() =>
            {
                None
            }
            other => other,
        }
    }

    fn learn_leg_travel(
        &self,
        key: Option<&str>,
        app_key: Option<&str>,
        injected_y: f64,
        traveled: Option<f64>,
    ) {
        let Some(traveled) = traveled.filter(|_| injected_y != 0.0) else {
            return;
        };
        let learned = self.state().scroll_gearing.learn(key, injected_y, traveled);
        if learned {
            super::lock(&self.scroll_gearing_file).learn(app_key, injected_y, traveled);
        }
    }

    async fn agent_focus_window_id(&self) -> Option<String> {
        self.focused_capturable_window(true)
            .await
            .ok()
            .flatten()
            .map(|window| window.id)
    }

    async fn capture_for_measurement(
        &self,
        window_id: Option<&str>,
    ) -> Option<ComputerCapturedWindow> {
        if !self.backend.capabilities().capture {
            return None;
        }
        self.engage_backend();
        timed_computer_leg("observe", async {
            match window_id {
                None => self
                    .capture_focused_window(Some(COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION), true)
                    .await
                    .ok(),
                Some(window_id) => self.capture_window(window_id).await.ok(),
            }
        })
        .await
    }

    async fn measure_travel(
        &self,
        before: &ComputerScreenshot,
        after: &ComputerScreenshot,
        column_center: Option<f64>,
    ) -> Option<f64> {
        if before.bytes_base64 == after.bytes_base64 {
            return Some(0.0);
        }
        let scale = before.scale?;
        if Some(scale) != after.scale || scale <= 0.0 {
            return None;
        }
        let decode = |screenshot: &ComputerScreenshot| {
            BASE64
                .decode(&screenshot.bytes_base64)
                .unwrap_or_else(|_| screenshot.bytes_base64.as_bytes().to_vec())
        };
        let traveled =
            (self.measure_scroll_travel)(decode(before), decode(after), column_center).await?;
        Some(traveled / scale)
    }
}
