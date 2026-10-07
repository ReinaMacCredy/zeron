use zeron_proto::PermissionMode;
use zeron_proto::computer::{ComputerApp, ComputerConsentApp, ComputerWindow, CuaEffect};

use super::{COMPUTER_DENYLIST_APP_CACHE_MS, ComputerConsentTarget, ComputerManager};
use crate::computer::backend::{ComputerBackendMethod, ComputerError, CuaActionError};
use crate::computer::cua_backend::parse_cua_window_id;
use crate::computer::task_context::current_computer_task;

pub const COMPUTER_APP_CONSENT_REQUIRED_CODE: &str = "app_consent_required";
pub const COMPUTER_APP_WINDOW_REQUIRED_CODE: &str = "app_window_required";
pub const COMPUTER_APP_WINDOW_REQUIRED_MESSAGE: &str = "In Ask mode, Computer input must name the app it reaches: pass window_id, an element ref, or coordinates on a screenshot of that window. Nothing was sent.";

pub(super) struct ResolvedConsentApp {
    pub app: ComputerConsentApp,
    pub key: String,
}

fn nonempty(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn verified(app: &ComputerApp, pid: Option<u32>) -> ResolvedConsentApp {
    let bundle_id = nonempty(app.bundle_id.as_deref());
    let name = nonempty(Some(&app.name))
        .or_else(|| bundle_id.clone())
        .unwrap_or_default();
    let key = match (&bundle_id, pid) {
        (Some(bundle_id), _) => format!("bundle:{bundle_id}"),
        (None, Some(pid)) => format!("pid:{pid}"),
        (None, None) => format!("app:{}", name.to_lowercase()),
    };
    ResolvedConsentApp {
        app: ComputerConsentApp {
            name,
            bundle_id,
            pid,
        },
        key,
    }
}

fn unresolved(name: String, pid: Option<u32>, key: String) -> ResolvedConsentApp {
    ResolvedConsentApp {
        app: ComputerConsentApp {
            name,
            bundle_id: None,
            pid,
        },
        key,
    }
}

impl ComputerManager {
    async fn consent_apps(&self, pid: Option<u32>) -> Vec<ComputerApp> {
        let apps = self.running_apps_for_denylist().await;
        let Some(pid) = pid else {
            return apps;
        };
        if apps.iter().any(|app| app.pid == pid)
            || !self.backend.supports(ComputerBackendMethod::ListApps)
        {
            return apps;
        }
        let now = self.now_ms();
        let fresh_miss = |at: &u64| now.saturating_sub(*at) < COMPUTER_DENYLIST_APP_CACHE_MS;
        if self
            .state()
            .consent_pid_misses
            .get(&pid)
            .is_some_and(fresh_miss)
        {
            return apps;
        }
        match self.backend.list_apps().await {
            Ok(fresh) => {
                let mut state = self.state();
                state.denied_apps_cache = Some((now, fresh.clone()));
                state.consent_pid_misses.retain(|_, at| fresh_miss(at));
                if !fresh.iter().any(|app| app.pid == pid) {
                    state.consent_pid_misses.insert(pid, now);
                }
                fresh
            }
            Err(_) => apps,
        }
    }

    pub(super) async fn resolve_consent_app(
        &self,
        target: &ComputerConsentTarget,
    ) -> ResolvedConsentApp {
        match target {
            ComputerConsentTarget::Window(window_id) => {
                let known = self.state().last_known_windows.get(window_id).cloned();
                let window = match known {
                    Some(window) => Some(window),
                    None => self
                        .backend
                        .list_windows()
                        .await
                        .ok()
                        .and_then(|windows| windows.into_iter().find(|w| &w.id == window_id)),
                };
                let pid = window.as_ref().and_then(|window| window.pid).or_else(|| {
                    parse_cua_window_id(window_id).and_then(|(pid, _)| u32::try_from(pid).ok())
                });
                let hint = window.as_ref().and_then(|window| {
                    nonempty(window.app_name.as_deref()).or_else(|| nonempty(Some(&window.title)))
                });
                match pid {
                    Some(pid) => self.resolve_consent_pid(pid, hint).await,
                    None => unresolved(
                        hint.unwrap_or_else(|| format!("window {window_id}")),
                        None,
                        format!("window:{window_id}"),
                    ),
                }
            }
            ComputerConsentTarget::Pid(pid) => self.resolve_consent_pid(*pid, None).await,
            ComputerConsentTarget::Unscoped => {
                unresolved("an unnamed app".into(), None, "unscoped".into())
            }
            ComputerConsentTarget::App(wanted) => {
                let wanted = wanted.trim();
                let apps = self.consent_apps(None).await;
                let by_bundle: Vec<&ComputerApp> = apps
                    .iter()
                    .filter(|app| {
                        app.bundle_id
                            .as_deref()
                            .is_some_and(|id| id.eq_ignore_ascii_case(wanted))
                    })
                    .collect();
                let matches = if by_bundle.is_empty() {
                    apps.iter()
                        .filter(|app| app.name.eq_ignore_ascii_case(wanted))
                        .collect()
                } else {
                    by_bundle
                };
                let mut bundles: Vec<&str> = matches
                    .iter()
                    .filter_map(|app| app.bundle_id.as_deref())
                    .collect();
                bundles.sort_unstable();
                bundles.dedup();
                match (matches.first(), bundles.len()) {
                    (Some(app), 1) if matches.iter().all(|m| m.bundle_id.is_some()) => {
                        let pid = matches
                            .iter()
                            .find(|app| app.running && app.pid > 0)
                            .map(|app| app.pid);
                        verified(app, pid)
                    }
                    _ => unresolved(
                        wanted.to_owned(),
                        None,
                        format!("app:{}", wanted.to_lowercase()),
                    ),
                }
            }
        }
    }

    pub async fn foreground_app(
        &self,
        target: &ComputerConsentTarget,
    ) -> (Option<ComputerConsentApp>, Option<String>) {
        if *target == ComputerConsentTarget::Unscoped {
            return (None, None);
        }
        let ResolvedConsentApp { app, key } = self.resolve_consent_app(target).await;
        let key = (key.starts_with("bundle:") || key.starts_with("pid:")).then_some(key);
        let placeholder = match target {
            ComputerConsentTarget::Window(window_id) => app.name == format!("window {window_id}"),
            ComputerConsentTarget::App(_) => app.bundle_id.is_none(),
            _ => false,
        } || app.name.is_empty()
            || app.bundle_id.as_deref() == Some(app.name.as_str())
            || app
                .pid
                .is_some_and(|pid| app.name == format!("process {pid}"));
        ((!placeholder).then_some(app), key)
    }

    pub async fn app_identity(&self, target: &ComputerConsentTarget) -> Option<(String, String)> {
        if *target == ComputerConsentTarget::Unscoped {
            return None;
        }
        let ResolvedConsentApp { app, key } = self.resolve_consent_app(target).await;
        Some((key, app.name))
    }

    pub async fn stopped_app_key(&self, app: &ComputerConsentApp) -> Option<String> {
        let target = match (app.pid, app.bundle_id.as_deref()) {
            (Some(pid), _) => ComputerConsentTarget::Pid(pid),
            (None, Some(bundle_id)) => return Some(format!("bundle:{bundle_id}")),
            (None, None) => ComputerConsentTarget::App(app.name.clone()),
        };
        self.app_identity(&target).await.map(|(key, _)| key)
    }

    async fn ask_turn(&self, thread_id: Option<&str>) -> Option<(String, String)> {
        let owner = super::agent_thread_id(thread_id)?;
        let turn = current_computer_task().and_then(|task| task.turn_id)?;
        let mode = self.seams.caller(owner, Some(&turn)).await.ok().flatten();
        (mode == Some(PermissionMode::Ask)).then(|| (owner.to_owned(), turn))
    }

    pub(super) async fn assert_app_consent(
        &self,
        thread_id: Option<&str>,
        windows: &[&ComputerWindow],
    ) -> Result<(), ComputerError> {
        let Some((owner, turn)) = self.ask_turn(thread_id).await else {
            return Ok(());
        };
        for window in windows {
            let resolved = self
                .resolve_consent_app(&ComputerConsentTarget::Window(window.id.clone()))
                .await;
            if !self.approval_gate.app_granted(
                &owner,
                Some(&turn),
                &resolved.key,
                resolved.app.bundle_id.as_deref(),
            ) {
                let identity = match &resolved.app.bundle_id {
                    Some(bundle_id) => format!("{} ({bundle_id})", resolved.app.name),
                    None => format!("{} (unverified app)", resolved.app.name),
                };
                return Err(CuaActionError::new(
                    format!("This input would reach {identity}, which has no Computer consent in this turn. Nothing was sent; retry so the app can be approved."),
                    CuaEffect::NotDispatched,
                )
                .code(COMPUTER_APP_CONSENT_REQUIRED_CODE)
                .into());
            }
        }
        Ok(())
    }

    pub(super) async fn assert_scoped_input(
        &self,
        thread_id: Option<&str>,
    ) -> Result<(), ComputerError> {
        match self.ask_turn(thread_id).await {
            Some(_) => Err(CuaActionError::new(
                COMPUTER_APP_WINDOW_REQUIRED_MESSAGE,
                CuaEffect::NotDispatched,
            )
            .code(COMPUTER_APP_WINDOW_REQUIRED_CODE)
            .into()),
            None => Ok(()),
        }
    }

    async fn resolve_consent_pid(&self, pid: u32, hint: Option<String>) -> ResolvedConsentApp {
        let apps = self.consent_apps(Some(pid)).await;
        match apps.iter().find(|app| app.pid == pid) {
            Some(app) if nonempty(app.bundle_id.as_deref()).is_some() => verified(app, Some(pid)),
            found => unresolved(
                found
                    .and_then(|app| nonempty(Some(&app.name)))
                    .or(hint)
                    .unwrap_or_else(|| format!("process {pid}")),
                Some(pid),
                format!("pid:{pid}"),
            ),
        }
    }
}
