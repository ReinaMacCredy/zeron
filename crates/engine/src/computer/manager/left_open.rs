use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use zeron_proto::computer::ComputerWindow;

use super::{ComputerManager, State, display_app_name};
use crate::computer::cua_backend::parse_cua_window_id;

// Ghostty's tab-bar and titlebar accessory windows list as 1512x33 and 1920x30 strips.
const LEFT_OPEN_MIN_SIDE: f64 = 48.0;
const LEFT_OPEN_READ_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) struct TurnWindows {
    turn_id: String,
    baseline: Option<HashSet<String>>,
    acted_pids: HashSet<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerLeftOpen {
    pub app: String,
    pub count: usize,
}

pub fn left_open_note(left: &ComputerLeftOpen) -> String {
    let noun = if left.count == 1 { "window" } else { "windows" };
    format!("Left {} {} {noun} open", left.count, left.app)
}

pub(super) fn begin_turn(state: &mut State, owner: &str, turn_id: &str) {
    if state
        .turn_windows
        .get(owner)
        .is_some_and(|turn| turn.turn_id == turn_id)
    {
        return;
    }
    state.turn_windows.insert(
        owner.to_owned(),
        TurnWindows {
            turn_id: turn_id.to_owned(),
            baseline: None,
            acted_pids: HashSet::new(),
        },
    );
}

pub(super) fn fill_baselines(state: &mut State) {
    for turn in state.turn_windows.values_mut() {
        if turn.baseline.is_none() {
            turn.baseline = state.last_known_window_ids.clone();
        }
    }
}

fn is_left_open(window: &ComputerWindow, baseline: &HashSet<String>, acted: &HashSet<u32>) -> bool {
    window.pid.is_some_and(|pid| acted.contains(&pid))
        && !baseline.contains(&window.id)
        && (window.visible || window.minimized || window.on_current_space == Some(false))
        && window.bounds.is_some_and(|bounds| {
            bounds.width >= LEFT_OPEN_MIN_SIDE && bounds.height >= LEFT_OPEN_MIN_SIDE
        })
}

impl ComputerManager {
    pub(super) fn record_acted_pid(&self, owner: &str, pid: Option<u32>) {
        let Some(pid) = pid else {
            return;
        };
        if let Some(turn) = self.state().turn_windows.get_mut(owner) {
            turn.acted_pids.insert(pid);
        }
    }

    pub(super) fn record_acted_window(&self, owner: &str, window_id: &str) {
        let pid = self
            .state()
            .last_known_windows
            .get(window_id)
            .and_then(|window| window.pid)
            .or_else(|| {
                parse_cua_window_id(window_id).and_then(|(pid, _)| u32::try_from(pid).ok())
            });
        self.record_acted_pid(owner, pid);
    }

    pub async fn left_open_windows(&self, thread_id: &str, turn_id: &str) -> Vec<ComputerLeftOpen> {
        let turn = {
            let mut state = self.state();
            match state.turn_windows.get(thread_id) {
                Some(turn) if turn.turn_id == turn_id => state.turn_windows.remove(thread_id),
                _ => None,
            }
        };
        let Some(TurnWindows {
            baseline: Some(baseline),
            acted_pids,
            ..
        }) = turn
        else {
            return Vec::new();
        };
        if acted_pids.is_empty() {
            return Vec::new();
        }
        let windows = match tokio::time::timeout(LEFT_OPEN_READ_TIMEOUT, self.read_windows()).await
        {
            Ok(Ok(windows)) => windows,
            Ok(Err(error)) => {
                tracing::warn!(thread = %thread_id, %error, "computer left-open read failed");
                return Vec::new();
            }
            Err(_) => {
                tracing::warn!(thread = %thread_id, "computer left-open read timed out");
                return Vec::new();
            }
        };
        let mut apps: BTreeMap<String, usize> = BTreeMap::new();
        for app in windows
            .iter()
            .filter(|window| is_left_open(window, &baseline, &acted_pids))
            .filter_map(|window| display_app_name(window.app_name.as_deref()))
        {
            *apps.entry(app).or_default() += 1;
        }
        apps.into_iter()
            .map(|(app, count)| ComputerLeftOpen { app, count })
            .collect()
    }
}
