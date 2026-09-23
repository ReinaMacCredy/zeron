use std::future::Future;

use zeron_proto::computer::{
    ComputerActionResult, ComputerApp, ComputerPoint, ComputerTarget, ComputerWindow,
};

use super::{
    COMPUTER_DENYLIST_APP_CACHE_MS, ComputerManager, agent_thread_id, error_message,
    occluded_target_error, refused_injection_error, semantic_point_target, window_not_found_error,
};
use crate::computer::backend::{
    ComputerAgentDialect, ComputerBackendActionResult, ComputerBackendMethod, ComputerError,
    ComputerResolvedTarget, ComputerStateOptions, computer_backend_action_result,
};
use crate::computer::call_context::{
    ComputerActionProof, current_computer_call, timed_computer_leg,
};
use crate::computer::denylist::{
    ComputerDenylistError, ComputerDenylistMatch, computer_denylist_match,
};
use crate::computer::desktop_operation_queue::assert_desktop_operation_active;
use crate::computer::element_identity::{ObservedComputerTarget, observed_computer_target_node};
use crate::computer::geometry::{
    rect_contains_point, topmost_window_at_point, windows_covering_point,
};
use crate::computer::space_broker::ComputerSpaceOwner;
use crate::computer::task_context::current_computer_task;
use crate::computer::ui_tree_targeting::{
    ComputerTargetError, ComputerTargetErrorCode, activation_point_for_node,
    computer_target_candidates, resolve_computer_point, resolve_computer_semantic_target,
    resolve_computer_unique_text_target,
};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ResolvedPointTarget {
    pub point: ComputerPoint,
    pub window_id: Option<String>,
    pub covering: Option<Vec<ComputerWindow>>,
    pub semantic: Option<ComputerResolvedTarget>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct PreparedTarget {
    pub point: Option<ComputerPoint>,
    pub window_id: Option<String>,
    pub covering: Option<Vec<ComputerWindow>>,
}

impl From<&ResolvedPointTarget> for PreparedTarget {
    fn from(resolved: &ResolvedPointTarget) -> Self {
        Self {
            point: Some(resolved.point),
            window_id: resolved.window_id.clone(),
            covering: resolved.covering.clone(),
        }
    }
}

pub(super) fn has_coordinates(target: &ComputerTarget) -> bool {
    target.x.is_some() && target.y.is_some()
}

pub(super) fn has_label_fields(target: &ComputerTarget) -> bool {
    target.label.is_some() || target.role.is_some()
}

fn has_semantic_fields(target: &ComputerTarget) -> bool {
    has_label_fields(target) || target.window_id.is_some()
}

fn denied(found: ComputerDenylistMatch) -> ComputerError {
    ComputerDenylistError::from(found).into()
}

impl ComputerManager {
    fn space_owner(&self, owner: &str) -> ComputerSpaceOwner {
        ComputerSpaceOwner {
            thread_id: owner.to_owned(),
            turn_id: current_computer_task().and_then(|task| task.turn_id),
        }
    }

    pub(super) fn assert_driven_app_allowed(&self, app: &str) -> Result<(), ComputerError> {
        if let Some(found) = computer_denylist_match(Some(app), None) {
            return Err(denied(found));
        }
        let lowered = app.trim().to_lowercase();
        let Some(pid) = lowered
            .strip_prefix("pid ")
            .filter(|digits| {
                !digits.is_empty()
                    && !digits.starts_with('0')
                    && digits.chars().all(|c| c.is_ascii_digit())
            })
            .and_then(|digits| digits.parse::<u32>().ok())
        else {
            return Ok(());
        };
        let owner = self
            .state()
            .denied_apps_cache
            .as_ref()
            .and_then(|(_, apps)| apps.iter().find(|candidate| candidate.pid == pid).cloned());
        let Some(owner) = owner else {
            return Ok(());
        };
        match computer_denylist_match(Some(&owner.name), owner.bundle_id.as_deref()) {
            Some(found) => Err(denied(found)),
            None => Ok(()),
        }
    }

    pub(super) async fn running_apps_for_denylist(&self) -> Vec<ComputerApp> {
        if !self.backend.supports(ComputerBackendMethod::ListApps) {
            return self
                .state()
                .denied_apps_cache
                .as_ref()
                .map(|(_, apps)| apps.clone())
                .unwrap_or_default();
        }
        let now = self.now_ms();
        let cached = self.state().denied_apps_cache.clone();
        if let Some((at, apps)) = &cached
            && now.saturating_sub(*at) < COMPUTER_DENYLIST_APP_CACHE_MS
        {
            return apps.clone();
        }
        match self.backend.list_apps().await {
            Ok(apps) => {
                self.state().denied_apps_cache = Some((now, apps.clone()));
                apps
            }
            Err(_) => cached.map(|(_, apps)| apps).unwrap_or_default(),
        }
    }

    pub(super) async fn denied_match_for_window(
        &self,
        window: &ComputerWindow,
    ) -> Option<ComputerDenylistMatch> {
        if let Some(found) = computer_denylist_match(window.app_name.as_deref(), None) {
            return Some(found);
        }
        let pid = window.pid?;
        let owner = self
            .running_apps_for_denylist()
            .await
            .into_iter()
            .find(|candidate| candidate.pid == pid)?;
        computer_denylist_match(Some(&owner.name), owner.bundle_id.as_deref())
    }

    pub(super) async fn denied_match_for_pid(
        &self,
        pid: u32,
        name: Option<&str>,
    ) -> Option<ComputerDenylistMatch> {
        if let Some(found) = computer_denylist_match(name, None) {
            return Some(found);
        }
        let owner = self
            .running_apps_for_denylist()
            .await
            .into_iter()
            .find(|candidate| candidate.pid == pid)?;
        computer_denylist_match(Some(&owner.name), owner.bundle_id.as_deref())
    }

    pub(super) async fn assert_window_input_allowed(
        &self,
        thread_id: Option<&str>,
        window_id: &str,
    ) -> Result<(), ComputerError> {
        if agent_thread_id(thread_id).is_none() {
            return Ok(());
        }
        let windows = self.read_windows().await?;
        let Some(window) = windows
            .into_iter()
            .find(|candidate| candidate.id == window_id)
        else {
            return Ok(());
        };
        self.assert_window_input_allowed_window(thread_id, &window)
            .await
    }

    pub(super) async fn assert_window_input_allowed_window(
        &self,
        thread_id: Option<&str>,
        window: &ComputerWindow,
    ) -> Result<(), ComputerError> {
        let Some(owner) = agent_thread_id(thread_id) else {
            return Ok(());
        };
        if let Some(found) = self.denied_match_for_window(window).await {
            return Err(denied(found));
        }
        self.space_broker
            .assert_window_allowed(&self.space_owner(owner), window)
            .await
    }

    pub(super) async fn assert_space_app_mutation_allowed(
        &self,
        thread_id: Option<&str>,
        pid: u32,
    ) -> Result<(), ComputerError> {
        let Some(owner) = agent_thread_id(thread_id) else {
            return Ok(());
        };
        self.space_broker
            .assert_app_mutation_allowed(&self.space_owner(owner), pid)
            .await
    }

    pub(super) async fn assert_window_content_allowed(
        &self,
        window_id: &str,
    ) -> Result<(), ComputerError> {
        let windows = self.read_windows().await?;
        let Some(window) = windows
            .into_iter()
            .find(|candidate| candidate.id == window_id)
        else {
            return Ok(());
        };
        match self.denied_match_for_window(&window).await {
            Some(found) => Err(denied(found)),
            None => Ok(()),
        }
    }

    pub(super) async fn denied_visible_windows(
        &self,
    ) -> Result<Vec<(ComputerWindow, ComputerDenylistMatch)>, ComputerError> {
        let mut denied = Vec::new();
        for window in self.read_windows().await? {
            if !window.visible || window.minimized {
                continue;
            }
            if let Some(found) = self.denied_match_for_window(&window).await {
                denied.push((window, found));
            }
        }
        Ok(denied)
    }

    pub(super) async fn assert_no_denied_visible_window(&self) -> Result<(), ComputerError> {
        match self.denied_visible_windows().await?.into_iter().next() {
            Some((_, found)) => Err(denied(found)),
            None => Ok(()),
        }
    }

    pub(super) async fn window_is_denied(&self, window_id: &str) -> Result<bool, ComputerError> {
        let windows = self.read_windows().await?;
        match windows
            .into_iter()
            .find(|candidate| candidate.id == window_id)
        {
            Some(window) => Ok(self.denied_match_for_window(&window).await.is_some()),
            None => Ok(false),
        }
    }

    pub(super) async fn admit_window_target(
        &self,
        thread_id: Option<&str>,
        target: &ComputerWindow,
    ) -> Result<(), ComputerError> {
        self.assert_driven_app_allowed(target.app_name.as_deref().unwrap_or(&target.id))?;
        self.assert_window_input_allowed_window(thread_id, target)
            .await
    }

    pub(super) async fn resolve_window_target(
        &self,
        thread_id: Option<&str>,
        window_id: &str,
    ) -> Result<ComputerWindow, ComputerError> {
        let windows = timed_computer_leg("resolve", self.read_windows()).await?;
        let target = windows
            .into_iter()
            .find(|candidate| candidate.id == window_id)
            .ok_or_else(|| window_not_found_error(window_id))?;
        self.admit_window_target(thread_id, &target).await?;
        Ok(target)
    }

    pub(super) async fn assert_scoped_window_readable(
        &self,
        window_id: &str,
    ) -> Result<(), ComputerError> {
        let windows = self.read_windows().await?;
        if !windows.iter().any(|candidate| candidate.id == window_id) {
            return Err(window_not_found_error(window_id));
        }
        self.assert_window_content_allowed(window_id).await
    }

    pub(super) fn assert_target_can_use_coordinates(
        &self,
        target: &ObservedComputerTarget,
    ) -> Result<(), ComputerError> {
        if observed_computer_target_node(target).is_some() {
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::Refused,
                "This observed element does not support the requested exact pointer action. Use an advertised semantic action or explicitly target a current screenshot; no coordinate fallback was sent.",
                Vec::new(),
            )
            .into());
        }
        Ok(())
    }

    pub(super) async fn resolve_point_target(
        &self,
        target: &ObservedComputerTarget,
        thread_id: Option<&str>,
    ) -> Result<ResolvedPointTarget, ComputerError> {
        let plain = &target.target;
        if has_coordinates(plain) && !has_label_fields(plain) {
            self.space_broker
                .assert_target_bound(agent_thread_id(thread_id), plain.window_id.as_deref())?;
            let point = self.resolve_coordinate_point(plain).await?;
            let Some(window_id) = plain.window_id.as_deref() else {
                let windows = self.read_windows().await?;
                match topmost_window_at_point(&windows, point) {
                    Some(topmost) => {
                        let topmost = topmost.id.clone();
                        self.assert_window_input_allowed(thread_id, &topmost)
                            .await?;
                    }
                    None => {
                        for window in &windows {
                            if !window.visible
                                || window.minimized
                                || !rect_contains_point(window.bounds.as_ref(), point)
                            {
                                continue;
                            }
                            self.assert_window_input_allowed_window(thread_id, window)
                                .await?;
                        }
                    }
                }
                return Ok(ResolvedPointTarget {
                    point,
                    window_id: None,
                    covering: None,
                    semantic: None,
                });
            };
            let (covering, windows, ranked) = self.scoped_point_occlusion(point, window_id).await?;
            self.assert_window_input_allowed(thread_id, window_id)
                .await?;
            if agent_thread_id(thread_id).is_some() {
                let suspects: Vec<&ComputerWindow> = if !covering.is_empty() {
                    covering.iter().collect()
                } else if ranked {
                    Vec::new()
                } else {
                    windows
                        .iter()
                        .filter(|window| {
                            window.id != window_id
                                && window.visible
                                && !window.minimized
                                && rect_contains_point(window.bounds.as_ref(), point)
                        })
                        .collect()
                };
                for window in suspects {
                    self.assert_window_input_allowed_window(thread_id, window)
                        .await?;
                }
            }
            return Ok(ResolvedPointTarget {
                point,
                window_id: Some(window_id.to_owned()),
                covering: Some(covering),
                semantic: None,
            });
        }
        if has_semantic_fields(plain) {
            let resolved = self.resolve_semantic_target(target, false).await?;
            if let Some(window_id) = resolved.node.window_id.as_deref() {
                self.assert_window_input_allowed(thread_id, window_id)
                    .await?;
            }
            return Ok(ResolvedPointTarget {
                point: resolved.point,
                window_id: resolved.node.window_id.clone(),
                covering: None,
                semantic: Some(resolved),
            });
        }
        Err(ComputerTargetError::new(
            ComputerTargetErrorCode::Invalid,
            "Computer actions require x/y coordinates or a labelled target.",
            Vec::new(),
        )
        .into())
    }

    async fn resolve_coordinate_point(
        &self,
        target: &ComputerTarget,
    ) -> Result<ComputerPoint, ComputerError> {
        if let (Some(window_id), Some(x), Some(y)) =
            (target.window_id.as_deref(), target.x, target.y)
        {
            let window = self
                .read_windows()
                .await?
                .into_iter()
                .find(|window| window.id == window_id);
            let Some(window) = window else {
                return Err(ComputerTargetError::new(
                    ComputerTargetErrorCode::NotFound,
                    "The exact window closed or moved since this screenshot. Observe again before acting.",
                    Vec::new(),
                )
                .into());
            };
            let Some(bounds) = window.bounds else {
                return Err(ComputerTargetError::new(
                    ComputerTargetErrorCode::Offscreen,
                    "The target window exposes no geometry.",
                    Vec::new(),
                )
                .into());
            };
            if x < bounds.x
                || y < bounds.y
                || x >= bounds.x + bounds.width
                || y >= bounds.y + bounds.height
            {
                return Err(ComputerTargetError::new(
                    ComputerTargetErrorCode::Offscreen,
                    "The coordinate is outside the exact target window.",
                    Vec::new(),
                )
                .into());
            }
            return Ok(ComputerPoint { x, y });
        }
        let screen = self.backend.get_screen_size().await?;
        match resolve_computer_point(target, &screen) {
            Ok(point) => Ok(point),
            Err(error) if error.code == ComputerTargetErrorCode::Offscreen => {
                let state = self
                    .backend
                    .get_state(ComputerStateOptions {
                        include_tree: true,
                        window_id: target.window_id.clone(),
                        ..ComputerStateOptions::default()
                    })
                    .await
                    .ok();
                let candidates = state
                    .and_then(|state| state.root)
                    .map(|root| computer_target_candidates(&root))
                    .unwrap_or_default();
                Err(ComputerTargetError {
                    candidates,
                    ..error
                }
                .into())
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn scoped_point_occlusion(
        &self,
        point: ComputerPoint,
        window_id: &str,
    ) -> Result<(Vec<ComputerWindow>, Vec<ComputerWindow>, bool), ComputerError> {
        let windows = self.read_windows().await?;
        let window = windows
            .iter()
            .find(|candidate| candidate.id == window_id)
            .ok_or_else(|| window_not_found_error(window_id))?;
        let Some(bounds) = window.bounds else {
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::Offscreen,
                format!(
                    "This desktop reports no geometry for window {}, so a coordinate cannot be checked against it. Drop window_id to click whatever is topmost at that point, or target the control by label instead.",
                    super::json_string(window_id)
                ),
                Vec::new(),
            )
            .into());
        };
        if !rect_contains_point(Some(&bounds), point) {
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::Offscreen,
                format!(
                    "Computer target ({}, {}) is outside window {}, which covers {}x{} at ({}, {}). Pass a coordinate inside those bounds, or drop window_id to click whatever is topmost.",
                    super::js_number(point.x),
                    super::js_number(point.y),
                    super::json_string(window_id),
                    super::js_number(bounds.width),
                    super::js_number(bounds.height),
                    super::js_number(bounds.x),
                    super::js_number(bounds.y)
                ),
                Vec::new(),
            )
            .into());
        }
        let ranked = window.stacking_index.is_some();
        let covering: Vec<ComputerWindow> = windows_covering_point(&windows, window_id, point)
            .into_iter()
            .cloned()
            .collect();
        Ok((covering, windows, ranked))
    }

    pub(super) async fn prepare_resolved_target(
        &self,
        target: Option<&PreparedTarget>,
        thread_id: Option<&str>,
    ) -> Result<(), ComputerError> {
        let window_id = target.and_then(|target| target.window_id.as_deref());
        self.space_broker
            .assert_target_bound(agent_thread_id(thread_id), window_id)?;
        let Some(window_id) = window_id else {
            assert_desktop_operation_active()?;
            self.backend.clear_focus_window().await?;
            return Ok(());
        };
        self.assert_window_input_allowed(thread_id, window_id)
            .await?;
        self.reveal_target(target).await?;
        assert_desktop_operation_active()?;
        self.backend.focus_window(window_id).await?;
        Ok(())
    }

    pub(super) async fn reveal_target(
        &self,
        target: Option<&PreparedTarget>,
    ) -> Result<(), ComputerError> {
        if self.backend.agent_dialect() == ComputerAgentDialect::Macos {
            return Ok(());
        }
        let Some(target) = target else {
            return Ok(());
        };
        let Some(window_id) = target.window_id.as_deref() else {
            return Ok(());
        };
        assert_desktop_operation_active()?;
        let failure = self.raise_target_window(window_id).await;
        if let (Some(failure), Some(point)) = (failure, target.point) {
            let covering = match &target.covering {
                Some(covering) => covering.clone(),
                None => self.covering_windows_at(point, window_id).await,
            };
            if !covering.is_empty() {
                return Err(occluded_target_error(window_id, point, &covering, &failure));
            }
        }
        Ok(())
    }

    async fn prepare_keyboard_target(
        &self,
        window_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<(), ComputerError> {
        self.space_broker
            .assert_target_bound(agent_thread_id(thread_id), window_id)?;
        let Some(window_id) = window_id else {
            return Ok(());
        };
        let windows = self.read_windows().await?;
        let window = windows
            .into_iter()
            .find(|candidate| candidate.id == window_id)
            .ok_or_else(|| window_not_found_error(window_id))?;
        if self.can_use_background_target() {
            return self.admit_window_target(thread_id, &window).await;
        }
        self.prepare_resolved_target(
            Some(&PreparedTarget {
                window_id: Some(window_id.to_owned()),
                ..PreparedTarget::default()
            }),
            thread_id,
        )
        .await
    }

    pub(super) async fn run_keyboard_dispatch<Fut>(
        &self,
        thread_id: Option<&str>,
        window_id: Option<&str>,
        dispatch: impl FnOnce() -> Fut,
    ) -> Result<ComputerBackendActionResult, ComputerError>
    where
        Fut: Future<Output = Result<ComputerBackendActionResult, ComputerError>>,
    {
        timed_computer_leg(
            "resolve",
            self.prepare_keyboard_target(window_id, thread_id),
        )
        .await?;
        assert_desktop_operation_active()?;
        timed_computer_leg("dispatch", dispatch()).await
    }

    pub(super) async fn prepare_semantic_dispatch(
        &self,
        target: &ObservedComputerTarget,
        thread_id: Option<&str>,
        allow_unique_text_target: bool,
    ) -> Result<ComputerResolvedTarget, ComputerError> {
        let resolved = timed_computer_leg(
            "resolve",
            self.resolve_semantic_target(target, allow_unique_text_target),
        )
        .await?;
        match target.target.window_id.as_deref() {
            Some(window_id) if self.can_use_background_target() => {
                self.assert_window_input_allowed(thread_id, window_id)
                    .await?;
            }
            _ => {
                let prepared = semantic_point_target(&resolved);
                timed_computer_leg(
                    "resolve",
                    self.prepare_resolved_target(Some(&prepared), thread_id),
                )
                .await?;
            }
        }
        assert_desktop_operation_active()?;
        Ok(resolved)
    }

    async fn raise_target_window(&self, window_id: &str) -> Option<String> {
        if !self.backend.supports(ComputerBackendMethod::RaiseWindow) {
            return Some("this backend exposes no stacking control".into());
        }
        self.backend
            .raise_window(window_id)
            .await
            .err()
            .map(|error| error_message(&error))
    }

    pub(super) async fn inject_scoped<Fut>(
        &self,
        action: &str,
        target: &PreparedTarget,
        inject: impl FnOnce() -> Fut,
    ) -> Result<ComputerBackendActionResult, ComputerError>
    where
        Fut: Future<Output = Result<ComputerBackendActionResult, ComputerError>>,
    {
        let result = async {
            assert_desktop_operation_active()?;
            {
                let mut state = self.state();
                if let Some(known) = state.last_known_window_ids.clone() {
                    state.pre_action_window_ids = Some(known);
                }
            }
            timed_computer_leg("dispatch", inject()).await
        }
        .await;
        let error = match result {
            Ok(result) => return Ok(result),
            Err(error) => error,
        };
        let (Some(window_id), Some(point)) = (target.window_id.as_deref(), target.point) else {
            return Err(error);
        };
        if error
            .backend()
            .is_none_or(|backend| backend.rejected_operation.is_none())
        {
            return Err(error);
        }
        Err(refused_injection_error(action, window_id, point))
    }

    async fn covering_windows_at(
        &self,
        point: ComputerPoint,
        window_id: &str,
    ) -> Vec<ComputerWindow> {
        let windows = self.read_windows().await.unwrap_or_default();
        windows_covering_point(&windows, window_id, point)
            .into_iter()
            .cloned()
            .collect()
    }

    pub(super) async fn resolve_semantic_target(
        &self,
        target: &ObservedComputerTarget,
        allow_unique_text_target: bool,
    ) -> Result<ComputerResolvedTarget, ComputerError> {
        let plain = &target.target;
        let unnamed = plain.label.is_none() && plain.role.is_none();
        if unnamed && (!allow_unique_text_target || plain.window_id.is_none()) {
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::Invalid,
                "This target does not name a control: window_id or coordinates alone match everything in scope. Pass label (optionally with role and window_id) to pick a control, or use x/y coordinates with the pointer tools. Only computer_scroll takes window_id alone, scrolling that window itself.",
                Vec::new(),
            )
            .into());
        }
        match plain.window_id.as_deref() {
            Some(window_id) => self.assert_window_content_allowed(window_id).await?,
            None if self.agent_dialect() != ComputerAgentDialect::Macos => {
                self.assert_no_denied_visible_window().await?;
            }
            None => {}
        }
        if let Some(observed) = observed_computer_target_node(target) {
            if observed.window_id.is_none() || observed.window_id != plain.window_id {
                return Err(ComputerTargetError::new(
                    ComputerTargetErrorCode::Invalid,
                    "The observed element and target name different windows; nothing was sent.",
                    Vec::new(),
                )
                .into());
            }
            return Ok(ComputerResolvedTarget {
                target: target.clone(),
                point: activation_point_for_node(observed),
                node: observed.clone(),
            });
        }
        let resolve = |root: &zeron_proto::computer::ComputerUiNode| {
            let matched = if unnamed {
                resolve_computer_unique_text_target(
                    root,
                    plain.window_id.as_deref().unwrap_or_default(),
                    allow_unique_text_target,
                )
            } else {
                resolve_computer_semantic_target(root, plain, allow_unique_text_target)
            };
            matched.map(|matched| ComputerResolvedTarget {
                target: target.clone(),
                point: matched.point,
                node: matched.node.clone(),
            })
        };
        let state = self
            .backend
            .get_state(ComputerStateOptions {
                include_tree: true,
                reuse_recent_tree: true,
                window_id: plain.window_id.clone(),
                ..ComputerStateOptions::default()
            })
            .await?;
        let Some(root) = state.root else {
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::NotFound,
                "Computer accessibility state did not include a target tree.",
                Vec::new(),
            )
            .into());
        };
        let mut error = match resolve(&root) {
            Ok(resolved) => return Ok(resolved),
            Err(error) => error,
        };
        let mut truncated = root.truncated == Some(true);
        if error.code == ComputerTargetErrorCode::NotFound {
            let fresh = self
                .backend
                .get_state(ComputerStateOptions {
                    include_tree: true,
                    window_id: plain.window_id.clone(),
                    ..ComputerStateOptions::default()
                })
                .await?;
            truncated = fresh
                .root
                .as_ref()
                .is_some_and(|root| root.truncated == Some(true));
            if let Some(root) = &fresh.root {
                match resolve(root) {
                    Ok(resolved) => return Ok(resolved),
                    Err(fresh_error) => error = fresh_error,
                }
            }
        }
        if error.code == ComputerTargetErrorCode::NotFound && truncated {
            let message = format!(
                "{} The accessibility tree was truncated; use computer_get_state with label_contains to narrow the list and check whether the control is present.",
                error.message
            );
            return Err(ComputerTargetError {
                message,
                not_found: true,
                ..error
            }
            .into());
        }
        Err(error.into())
    }

    pub(super) fn keyboard_target_window(
        &self,
        window_id: Option<&str>,
        target: Option<&ObservedComputerTarget>,
    ) -> Result<Option<String>, ComputerError> {
        let target_window = target.and_then(|target| target.target.window_id.as_deref());
        if let (Some(window_id), Some(target_window)) = (window_id, target_window)
            && window_id != target_window
        {
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::Invalid,
                "The keyboard window and element target name different windows; nothing was sent.",
                Vec::new(),
            )
            .into());
        }
        Ok(window_id.or(target_window).map(str::to_owned))
    }

    pub(super) fn action_result(
        &self,
        thread_id: Option<&str>,
        action: &str,
        point: Option<ComputerPoint>,
        result: Option<ComputerBackendActionResult>,
        window_id: Option<&str>,
    ) -> ComputerActionResult {
        let proof = result.as_ref().map(|result| ComputerActionProof {
            effect: result.effect,
            verified: result.verified,
        });
        let mut merged = result.unwrap_or_default();
        if merged.point.is_none() {
            merged.point = point;
        }
        if merged.window_id.is_none() {
            merged.window_id = window_id.map(str::to_owned);
        }
        let merged = computer_backend_action_result(&self.computer_id, action, merged);
        if let Some(call) = current_computer_call() {
            if let Some(timing) = &call.timing {
                timing.set_operation(action);
            }
            call.record_action_proof(proof.unwrap_or_default());
        }
        self.emit_action(thread_id, action, Some(&merged));
        let attributed = agent_thread_id(thread_id);
        if let (Some(attributed), Some(point)) = (attributed, merged.point) {
            let present = {
                let mut state = self.state();
                match state.thread_mut(attributed) {
                    Some(runtime) => {
                        runtime.cursor = Some(point);
                        true
                    }
                    None => false,
                }
            };
            if present {
                self.publish_cached(attributed);
            }
        }
        merged
    }
}
