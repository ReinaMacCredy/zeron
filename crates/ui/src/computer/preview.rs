use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Entity, Global, Point, RenderImage, Task};
use zeron_proto::computer::{
    ComputerEvent, ComputerHealth, ComputerHealthStatus, ComputerPreviewPlacement,
    ComputerPreviewSize, ComputerSettings, ThreadComputerState,
};
use zeron_rpc::{RpcError, methods};

use super::presence::{ComputerPresenceMap, DrivenApp, cua_window};
use crate::state::EngineHandle;

pub const STILL_POLL_INTERVAL: Duration = Duration::from_millis(1_000);
pub const COMPLETION_FLASH: Duration = Duration::from_millis(1_600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewPhase {
    Armed,
    Live,
    HiddenForTask,
    Ended,
}

fn phase_on_agent_edge(phase: Option<PreviewPhase>, rose: bool) -> Option<PreviewPhase> {
    if !rose {
        return phase.map(|_| PreviewPhase::Ended);
    }
    Some(if phase == Some(PreviewPhase::Live) {
        PreviewPhase::Live
    } else {
        PreviewPhase::Armed
    })
}

fn phase_on_surface_request(phase: Option<PreviewPhase>) -> PreviewPhase {
    match phase {
        Some(PreviewPhase::Live) => PreviewPhase::Live,
        _ => PreviewPhase::Armed,
    }
}

fn phase_on_viewed(phase: Option<PreviewPhase>) -> Option<PreviewPhase> {
    match phase {
        Some(PreviewPhase::Armed) => Some(PreviewPhase::Live),
        other => other,
    }
}

fn phase_on_hide(phase: Option<PreviewPhase>) -> Option<PreviewPhase> {
    match phase {
        Some(PreviewPhase::Armed | PreviewPhase::Live) => Some(PreviewPhase::HiddenForTask),
        other => other,
    }
}

#[derive(Clone)]
pub struct PreviewImage {
    pub seq: u64,
    pub image: Arc<RenderImage>,
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for PreviewImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PreviewImage#{} {}x{}",
            self.seq, self.width, self.height
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFailure {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSource {
    Tap,
    Stills,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeTone {
    Danger,
    Warning,
    Muted,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBadge {
    pub label: String,
    pub title: Option<String>,
    pub tone: BadgeTone,
    pub pulse: bool,
}

#[derive(Debug, Default)]
pub(super) struct ChatPreview {
    pub(super) phase: Option<PreviewPhase>,
    last_action_label: Option<String>,
    pub(super) last_app: Option<String>,
    pub(super) last_window_id: Option<String>,
    pub(super) revoked_latched: bool,
    pub(super) user_paused: bool,
    pub(super) user_paused_window: Option<String>,
    pub(super) state: Option<ThreadComputerState>,
    agent_active: bool,
    tap: Option<PreviewImage>,
    still: Option<PreviewImage>,
    failure: Option<PreviewFailure>,
    armed_by_still: bool,
    floating: Option<Point<f32>>,
    thumb_closed: bool,
    user_opened: bool,
    pub(super) apps: Vec<DrivenApp>,
    pub(super) stopped_apps: Vec<String>,
    finished_at: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct PreviewView {
    pub phase: Option<PreviewPhase>,
    pub open: bool,
    pub frame: Option<PreviewImage>,
    pub source: FrameSource,
    pub failure: Option<PreviewFailure>,
    pub connecting: bool,
    pub app: Option<String>,
    pub placement: ComputerPreviewPlacement,
    pub status_label: Option<String>,
    pub badge: Option<StatusBadge>,
    pub wants_stills: bool,
    pub floating: Option<Point<f32>>,
    pub completed: bool,
}

impl PreviewView {
    pub fn visually_open(&self) -> bool {
        self.open && (self.frame.is_some() || self.failure.is_some() || self.connecting)
    }
}

#[derive(Default)]
pub struct ComputerPreviewStore {
    chats: HashMap<String, ChatPreview>,
    events_live: bool,
    settings: ComputerSettings,
    settings_stale: bool,
    evicted: Vec<Arc<RenderImage>>,
}

pub fn agent_active(state: &ThreadComputerState) -> bool {
    state.control_owner_thread_id.as_deref() == Some(state.thread_id.as_str())
        || (state.agent_active && !state.controlled_by_other_thread)
}

fn driving(state: Option<&ThreadComputerState>) -> bool {
    state.is_none_or(|state| state.shared_preview_unavailable != Some(true) && agent_active(state))
}

pub fn status_label(
    agent_active: bool,
    input_stopped: bool,
    activity: Option<&str>,
    last_action_label: Option<&str>,
) -> Option<String> {
    if input_stopped {
        return Some("Desktop access revoked".into());
    }
    if agent_active {
        return Some(activity.or(last_action_label).unwrap_or("Live").to_owned());
    }
    last_action_label.map(str::to_owned)
}

fn health_detail(health: &ComputerHealth) -> String {
    let mut parts = vec![if health.status == ComputerHealthStatus::Reconnecting {
        "The desktop backend dropped out and is being reconnected.".to_owned()
    } else {
        "The desktop backend is not connected.".to_owned()
    }];
    if let Some(failure) = &health.last_failure {
        parts.push(format!("Last failure: {}", failure.message));
    }
    if health.consecutive_failures > 0 {
        parts.push(format!(
            "Failed attempts since the last connection: {}.",
            health.consecutive_failures
        ));
    }
    match health.reconnects {
        0 => {}
        1 => parts.push("Reconnected once since startup.".into()),
        n => parts.push(format!("Reconnected {n} times since startup.")),
    }
    parts.join(" ")
}

pub fn status_badge(
    state: Option<&ThreadComputerState>,
    agent_active: bool,
    visible_desktop: bool,
) -> Option<StatusBadge> {
    if let Some(health) = state.map(|state| &state.health)
        && health.status != ComputerHealthStatus::Connected
    {
        let reconnecting = health.status == ComputerHealthStatus::Reconnecting;
        let lazy =
            !reconnecting && health.consecutive_failures == 0 && health.last_failure.is_none();
        if !lazy {
            return Some(StatusBadge {
                label: if reconnecting {
                    "Reconnecting to desktop"
                } else {
                    "Desktop unavailable"
                }
                .into(),
                title: Some(health_detail(health)),
                tone: if reconnecting {
                    BadgeTone::Warning
                } else {
                    BadgeTone::Danger
                },
                pulse: reconnecting,
            });
        }
    }
    if let Some(pause) = state.and_then(|state| state.input_pause.as_ref()) {
        return Some(StatusBadge {
            label: "Input paused".into(),
            title: Some(pause.message.clone()),
            tone: BadgeTone::Warning,
            pulse: false,
        });
    }
    if let Some(state) = state.filter(|state| state.controlled_by_other_thread) {
        return Some(StatusBadge {
            label: format!(
                "{} is controlling",
                state
                    .control_owner_label
                    .as_deref()
                    .unwrap_or("Another conversation")
            ),
            title: Some(
                "Only one conversation can drive the desktop at a time. This one can still watch it."
                    .into(),
            ),
            tone: BadgeTone::Muted,
            pulse: false,
        });
    }
    agent_active.then(|| StatusBadge {
        label: state
            .and_then(|state| state.activity.clone())
            .unwrap_or_else(|| {
                if visible_desktop {
                    "Agent controlling this computer"
                } else {
                    "Agent controlling"
                }
                .into()
            }),
        title: None,
        tone: BadgeTone::Active,
        pulse: true,
    })
}

pub fn action_status_label(
    action: &str,
    app: Option<&str>,
    delivery_path: Option<&str>,
) -> Option<String> {
    let label = match super::presentation::computer_tool_name(action) {
        Some(tool) => super::presentation::COMPUTER_TOOL_TITLES
            .iter()
            .find(|(name, _)| *name == tool)
            .map(|(_, title)| (*title).to_owned())?,
        None => {
            let trimmed = action
                .strip_prefix("computer_")
                .or_else(|| action.strip_prefix("computer."))
                .unwrap_or(action);
            let words = trimmed
                .split(['_', '.'])
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let mut chars = words.chars();
            let first = chars.next()?;
            first.to_uppercase().chain(chars).collect()
        }
    };
    let delivery = delivery_path.map(|path| {
        if path.contains("foreground") {
            "Temporary foreground"
        } else {
            "Background action"
        }
    });
    Some(
        [Some(label.as_str()), app, delivery]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
    )
}

impl ComputerPreviewStore {
    fn chat(&mut self, chat_id: &str) -> &mut ChatPreview {
        self.chats.entry(chat_id.to_owned()).or_default()
    }

    fn set_phase(&mut self, chat_id: &str, phase: Option<PreviewPhase>) -> bool {
        let chat = self.chats.entry(chat_id.to_owned()).or_default();
        if chat.phase == phase {
            return false;
        }
        let fresh_task = phase == Some(PreviewPhase::Armed)
            && !matches!(chat.phase, Some(PreviewPhase::Armed | PreviewPhase::Live));
        chat.phase = phase;
        if fresh_task {
            chat.last_app = None;
            chat.last_window_id = None;
            chat.user_paused = false;
            chat.user_paused_window = None;
            chat.thumb_closed = false;
            chat.user_opened = false;
            chat.apps.clear();
            chat.stopped_apps.clear();
            self.evicted
                .extend(chat.tap.take().map(|frame| frame.image));
            self.evicted
                .extend(chat.still.take().map(|frame| frame.image));
            chat.failure = None;
            self.settings_stale = true;
        }
        true
    }

    pub fn apply_event(&mut self, event: &ComputerEvent) -> bool {
        match event {
            ComputerEvent::ThreadState { state } => {
                let active = agent_active(state);
                let chat = self.chat(&state.thread_id);
                let was = std::mem::replace(&mut chat.agent_active, active);
                let stopped = |state: &ThreadComputerState| state.input_stopped == Some(true);
                if stopped(state) {
                    chat.revoked_latched |= !chat.state.as_ref().is_some_and(stopped);
                } else if active && !was {
                    chat.revoked_latched = false;
                }
                chat.state = Some((**state).clone());
                chat.user_opened &= active || !was;
                if was && !active {
                    chat.user_paused = false;
                    chat.user_paused_window = None;
                    chat.finished_at = (!stopped(state)).then(Instant::now);
                }
                if active && !was {
                    chat.apps.clear();
                    chat.stopped_apps.clear();
                    chat.finished_at = None;
                }
                if active != was {
                    let next = phase_on_agent_edge(chat.phase, active);
                    self.set_phase(&state.thread_id, next);
                }
                true
            }
            ComputerEvent::OpenPaneRequested { thread_id } => {
                let next = phase_on_surface_request(self.chat(thread_id).phase);
                self.set_phase(thread_id, Some(next))
            }
            ComputerEvent::Action {
                thread_id: Some(thread_id),
                window_id,
                delivery,
                action,
                ..
            } => {
                let mut changed = self.chat(thread_id).phase.is_none()
                    && self.set_phase(thread_id, Some(PreviewPhase::Armed));
                let chat = self.chat(thread_id);
                let window = chat.state.as_ref().and_then(|state| {
                    state
                        .windows
                        .iter()
                        .find(|window| Some(&window.id) == window_id.as_ref())
                });
                let app = window.and_then(|window| window.app_name.clone());
                if let Some(name) = app.as_deref().filter(|name| !name.trim().is_empty())
                    && !chat.apps.iter().any(|known| known.same_app(name))
                {
                    let pid = window.and_then(|window| window.pid).or_else(|| {
                        window_id
                            .as_deref()
                            .and_then(cua_window)
                            .and_then(|(pid, _)| u32::try_from(pid).ok())
                    });
                    chat.apps.push(DrivenApp {
                        name: name.to_owned(),
                        pid,
                    });
                    changed = true;
                }
                if let Some(window_id) = window_id {
                    let same_window = chat.last_window_id.as_ref() == Some(window_id);
                    let next_app = app
                        .clone()
                        .or_else(|| same_window.then(|| chat.last_app.clone()).flatten());
                    changed |= !same_window || chat.last_app != next_app;
                    chat.last_window_id = Some(window_id.clone());
                    chat.last_app = next_app;
                }
                let Some(label) = action_status_label(
                    action,
                    app.as_deref(),
                    delivery.as_ref().map(|delivery| delivery.path.as_str()),
                ) else {
                    return changed;
                };
                if chat.last_action_label.as_deref() == Some(label.as_str()) {
                    return changed;
                }
                chat.last_action_label = Some(label);
                true
            }
            _ => false,
        }
    }

    pub fn set_events_live(&mut self, live: bool) {
        self.events_live = live;
    }

    pub fn events_live(&self) -> bool {
        self.events_live
    }

    pub fn accept_tap_frame(&mut self, thread_id: &str, frame: PreviewImage) -> bool {
        let chat = self.chat(thread_id);
        if chat.tap.as_ref().is_some_and(|held| held.seq >= frame.seq) {
            self.evicted.push(frame.image);
            return false;
        }
        chat.failure = None;
        let previous = chat.tap.replace(frame);
        self.evicted.extend(previous.map(|frame| frame.image));
        true
    }

    pub fn accept_tap_failure(&mut self, thread_id: &str, failure: PreviewFailure) -> bool {
        let chat = self.chat(thread_id);
        if chat.failure.as_ref() == Some(&failure) {
            return false;
        }
        chat.failure = Some(failure);
        true
    }

    pub fn accept_still(&mut self, chat_id: &str, frame: Option<PreviewImage>) -> bool {
        let events_live = self.events_live;
        let chat = self.chat(chat_id);
        let Some(frame) = frame else {
            if chat.armed_by_still && chat.phase.is_some() {
                chat.armed_by_still = false;
                return self.set_phase(chat_id, Some(PreviewPhase::Ended));
            }
            return false;
        };
        if chat
            .still
            .as_ref()
            .is_some_and(|held| held.seq >= frame.seq)
        {
            self.evicted.push(frame.image);
            return false;
        }
        let arm = !events_live
            && !matches!(
                chat.phase,
                Some(PreviewPhase::Armed | PreviewPhase::Live | PreviewPhase::HiddenForTask)
            );
        if arm {
            self.set_phase(chat_id, Some(PreviewPhase::Armed));
        }
        let chat = self.chat(chat_id);
        chat.armed_by_still |= arm;
        let previous = chat.still.replace(frame);
        self.evicted.extend(previous.map(|frame| frame.image));
        true
    }

    pub fn set_floating(&mut self, chat_id: &str, position: Option<Point<f32>>) -> bool {
        if position.is_none() && !self.chats.contains_key(chat_id) {
            return false;
        }
        std::mem::replace(&mut self.chat(chat_id).floating, position) != position
    }

    pub fn move_floating(&mut self, chat_id: &str, position: Point<f32>) -> bool {
        match self
            .chats
            .get_mut(chat_id)
            .and_then(|chat| chat.floating.as_mut())
        {
            Some(held) if *held != position => {
                *held = position;
                true
            }
            _ => false,
        }
    }

    pub fn remove_chat(&mut self, chat_id: &str) -> bool {
        let Some(chat) = self.chats.remove(chat_id) else {
            return false;
        };
        self.evicted.extend(
            chat.tap
                .into_iter()
                .chain(chat.still)
                .map(|frame| frame.image),
        );
        true
    }

    pub fn mark_live(&mut self, chat_id: &str) -> bool {
        let next = phase_on_viewed(self.chats.get(chat_id).and_then(|chat| chat.phase));
        self.set_phase(chat_id, next)
    }

    pub fn hide_for_task(&mut self, chat_id: &str) -> bool {
        let opened = self
            .chats
            .get_mut(chat_id)
            .is_some_and(|chat| std::mem::take(&mut chat.user_opened));
        let next = phase_on_hide(self.chats.get(chat_id).and_then(|chat| chat.phase));
        self.set_phase(chat_id, next) | opened
    }

    pub fn hide_all_active(&mut self) -> bool {
        let active: Vec<String> = self
            .chats
            .iter()
            .filter(|(_, chat)| {
                matches!(chat.phase, Some(PreviewPhase::Armed | PreviewPhase::Live))
            })
            .map(|(chat_id, _)| chat_id.clone())
            .collect();
        active.iter().fold(false, |changed, chat_id| {
            self.hide_for_task(chat_id) | changed
        })
    }

    pub fn stop_app(&mut self, chat_id: &str, app: &str) -> bool {
        let Some(chat) = self.chats.get_mut(chat_id) else {
            return false;
        };
        if chat
            .stopped_apps
            .iter()
            .any(|stopped| stopped.eq_ignore_ascii_case(app))
        {
            return false;
        }
        chat.stopped_apps.push(app.to_owned());
        true
    }

    pub fn reopen(&mut self, chat_id: &str) -> bool {
        let Some(chat) = self
            .chats
            .get_mut(chat_id)
            .filter(|chat| chat.phase.is_some())
        else {
            return false;
        };
        let opened = !std::mem::replace(&mut chat.user_opened, true);
        let hidden = chat.phase == Some(PreviewPhase::HiddenForTask);
        (hidden && self.set_phase(chat_id, Some(PreviewPhase::Live))) | opened
    }

    pub fn toggle_thumbnail(&mut self, chat_id: &str) -> bool {
        let shown = self.thumbnail_shown(chat_id);
        let Some(chat) = self
            .chats
            .get_mut(chat_id)
            .filter(|chat| chat.phase.is_some())
        else {
            return false;
        };
        chat.thumb_closed = shown;
        chat.user_opened |= !shown;
        true
    }

    pub fn thumbnail_shown(&self, chat_id: &str) -> bool {
        self.placement() == ComputerPreviewPlacement::Rail
            && self.chats.get(chat_id).is_some_and(|chat| {
                chat.phase.is_some()
                    && chat.floating.is_none()
                    && !chat.thumb_closed
                    && (self.settings.auto_open_computer_pane || chat.user_opened)
            })
    }

    pub fn placement(&self) -> ComputerPreviewPlacement {
        self.settings.computer_preview_placement
    }

    pub fn set_user_paused(&mut self, chat_id: &str, window_id: Option<String>) -> bool {
        let Some(chat) = self.chats.get_mut(chat_id).filter(|chat| chat.agent_active) else {
            return false;
        };
        let changed = !chat.user_paused || chat.user_paused_window != window_id;
        chat.user_paused = true;
        chat.user_paused_window = window_id;
        changed
    }

    pub fn clear_user_paused(&mut self, chat_id: &str) -> bool {
        self.chats.get_mut(chat_id).is_some_and(|chat| {
            chat.user_paused_window = None;
            std::mem::take(&mut chat.user_paused)
        })
    }

    pub fn clear_revoked(&mut self, chat_id: &str) -> bool {
        self.chats
            .get_mut(chat_id)
            .is_some_and(|chat| std::mem::take(&mut chat.revoked_latched))
    }

    pub(super) fn chat_preview(&self, chat_id: &str) -> Option<&ChatPreview> {
        self.chats.get(chat_id)
    }

    pub(super) fn chat_ids(&self) -> impl Iterator<Item = &str> {
        self.chats.keys().map(String::as_str)
    }

    pub fn settings(&self) -> &ComputerSettings {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: ComputerSettings) -> bool {
        self.settings_stale = false;
        std::mem::replace(&mut self.settings, settings) != self.settings
    }

    pub fn settings_stale(&self) -> bool {
        self.settings_stale
    }

    pub fn clear_settings_stale(&mut self) {
        self.settings_stale = false;
    }

    pub fn take_evicted(&mut self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.evicted)
    }

    pub fn view(&self, chat_id: &str) -> PreviewView {
        let chat = self.chats.get(chat_id);
        let state = chat.and_then(|chat| chat.state.as_ref());
        let phase = chat.and_then(|chat| chat.phase);
        let user_opened = chat.is_some_and(|chat| chat.user_opened);
        let shown = self.settings.auto_open_computer_pane || user_opened;
        let open = match phase {
            Some(PreviewPhase::Live) => shown,
            Some(PreviewPhase::Ended) => user_opened,
            _ => false,
        };
        let active = chat.is_some_and(|chat| {
            chat.agent_active
                || (chat.armed_by_still
                    && matches!(phase, Some(PreviewPhase::Armed | PreviewPhase::Live)))
        });
        let input_stopped = state.is_some_and(|state| state.input_stopped == Some(true));
        let ended = phase == Some(PreviewPhase::Ended)
            && state.is_none_or(|state| state.shared_preview_unavailable != Some(true));
        let tap = chat
            .and_then(|chat| chat.tap.clone())
            .filter(|_| driving(state) || ended);
        let source = if tap.is_some() {
            FrameSource::Tap
        } else {
            FrameSource::Stills
        };
        let frame = tap.or_else(|| chat.and_then(|chat| chat.still.clone()));
        let failure = chat
            .and_then(|chat| chat.failure.clone())
            .filter(|_| frame.is_none());
        let wanted =
            shown && (open || (!self.events_live && phase != Some(PreviewPhase::HiddenForTask)));
        PreviewView {
            phase,
            open,
            source,
            connecting: open && frame.is_none() && failure.is_none() && active,
            app: chat.and_then(|chat| chat.last_app.clone()),
            placement: self.placement(),
            failure,
            status_label: status_label(
                active,
                input_stopped,
                state.and_then(|state| state.activity.as_deref()),
                chat.and_then(|chat| chat.last_action_label.as_deref()),
            ),
            badge: status_badge(state, active, true),
            wants_stills: wanted && source == FrameSource::Stills,
            floating: chat.and_then(|chat| chat.floating),
            completed: chat
                .and_then(|chat| chat.finished_at)
                .is_some_and(|at| at.elapsed() < COMPLETION_FLASH),
            frame,
        }
    }

    pub fn preview_size(&self) -> ComputerPreviewSize {
        self.settings.computer_preview_size
    }
}

// gpui's RenderImage expects BGRA.
pub fn decode_frame(seq: u64, bytes: &[u8]) -> Option<PreviewImage> {
    let mut pixels = image::load_from_memory(bytes).ok()?.to_rgba8();
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let (width, height) = pixels.dimensions();
    (width > 0 && height > 0).then(|| PreviewImage {
        seq,
        image: Arc::new(RenderImage::new([image::Frame::new(pixels)])),
        width,
        height,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TapEvent {
    Frame {
        thread_id: String,
        seq: u64,
        jpeg: Vec<u8>,
    },
    Failed {
        thread_id: String,
        code: String,
        message: String,
    },
    UserPaused {
        thread_id: String,
        window_id: Option<String>,
    },
    UserResumed {
        thread_id: String,
    },
}

pub struct ComputerPreview {
    pub store: Entity<ComputerPreviewStore>,
    presence: Entity<ComputerPresenceMap>,
    taps: mpsc::UnboundedSender<TapEvent>,
    _pump: Task<()>,
}

impl Global for ComputerPreview {}

pub fn store(cx: &App) -> Option<Entity<ComputerPreviewStore>> {
    cx.try_global::<ComputerPreview>()
        .map(|preview| preview.store.clone())
}

pub fn presence_map(cx: &App) -> Option<Entity<ComputerPresenceMap>> {
    cx.try_global::<ComputerPreview>()
        .map(|preview| preview.presence.clone())
}

pub fn update_store(
    store: &Entity<ComputerPreviewStore>,
    cx: &mut App,
    change: impl FnOnce(&mut ComputerPreviewStore) -> bool,
) {
    let (changed, evicted) = store.update(cx, |store, cx| {
        let changed = change(store);
        if changed {
            cx.notify();
        }
        (changed, store.take_evicted())
    });
    for image in evicted {
        gpui::ImageSource::Render(image).evict(None, cx);
    }
    if changed
        && let Some(preview) = cx.try_global::<ComputerPreview>()
        && preview.store == *store
    {
        let map = preview.presence.clone();
        let next = super::presence::presences(store.read(cx));
        let rail = super::presence::rail_entries(store.read(cx));
        super::protected_surfaces::publish(
            map.entity_id(),
            next.iter().any(|presence| presence.pending.is_some()),
        );
        map.update(cx, |map, cx| {
            if map.set(next) | map.set_rail(rail) {
                cx.notify();
            }
        });
    }
}

pub fn clear_revoked(chat_id: &str, cx: &mut App) {
    if let Some(store) = store(cx) {
        update_store(&store, cx, |store| store.clear_revoked(chat_id));
    }
}

fn coalesce(batch: Vec<TapEvent>) -> Vec<TapEvent> {
    let mut newest: HashMap<String, u64> = HashMap::new();
    for event in &batch {
        if let TapEvent::Frame { thread_id, seq, .. } = event {
            let held = newest.entry(thread_id.clone()).or_insert(*seq);
            *held = (*held).max(*seq);
        }
    }
    batch
        .into_iter()
        .filter(|event| match event {
            TapEvent::Frame { thread_id, seq, .. } => newest.get(thread_id) == Some(seq),
            TapEvent::Failed { .. }
            | TapEvent::UserPaused { .. }
            | TapEvent::UserResumed { .. } => true,
        })
        .collect()
}

pub fn init(cx: &mut App) {
    if cx.has_global::<ComputerPreview>() {
        return;
    }
    let store = cx.new(|_| ComputerPreviewStore::default());
    let presence = cx.new(|_| ComputerPresenceMap::default());
    let (taps, mut received) = mpsc::unbounded::<TapEvent>();
    let pump_store = store.downgrade();
    let pump = cx.spawn(async move |cx| {
        while let Some(first) = received.next().await {
            let mut batch = vec![first];
            while let Ok(event) = received.try_recv() {
                batch.push(event);
            }
            for event in coalesce(batch) {
                let Some(store) = pump_store.upgrade() else {
                    return;
                };
                match event {
                    TapEvent::Frame {
                        thread_id,
                        seq,
                        jpeg,
                    } => {
                        let decoded = cx
                            .background_executor()
                            .spawn(async move { decode_frame(seq, &jpeg) })
                            .await;
                        if let Some(frame) = decoded {
                            cx.update(|cx| {
                                update_store(&store, cx, |store| {
                                    store.accept_tap_frame(&thread_id, frame)
                                })
                            });
                        }
                    }
                    TapEvent::Failed {
                        thread_id,
                        code,
                        message,
                    } => cx.update(|cx| {
                        update_store(&store, cx, |store| {
                            store.accept_tap_failure(&thread_id, PreviewFailure { code, message })
                        })
                    }),
                    TapEvent::UserPaused {
                        thread_id,
                        window_id,
                    } => cx.update(|cx| {
                        update_store(&store, cx, |store| {
                            store.set_user_paused(&thread_id, window_id)
                        })
                    }),
                    TapEvent::UserResumed { thread_id } => cx.update(|cx| {
                        update_store(&store, cx, |store| store.clear_user_paused(&thread_id))
                    }),
                }
            }
        }
    });
    cx.set_global(ComputerPreview {
        store,
        presence,
        taps,
        _pump: pump,
    });
}

pub fn tap_sender(cx: &App) -> Option<mpsc::UnboundedSender<TapEvent>> {
    cx.try_global::<ComputerPreview>()
        .map(|preview| preview.taps.clone())
}

// Runs on SC/encoder threads, so it only enqueues.
#[cfg(target_os = "macos")]
pub fn native_sink(cx: &App) -> super::native::frame_tap::PreviewSink {
    use super::native::frame_tap::ComputerPreviewEvent;
    let Some(taps) = tap_sender(cx) else {
        return Arc::new(|_| {});
    };
    Arc::new(move |event| {
        let event = match event {
            ComputerPreviewEvent::Frame(frame) => TapEvent::Frame {
                thread_id: frame.thread_id,
                seq: frame.seq,
                jpeg: frame.jpeg,
            },
            ComputerPreviewEvent::Failed {
                thread_id,
                code,
                message,
                ..
            } => TapEvent::Failed {
                thread_id,
                code,
                message,
            },
        };
        let _ = taps.unbounded_send(event);
    })
}

#[cfg(target_os = "macos")]
pub fn takeover_sink(cx: &App) -> Option<Box<dyn Fn(super::host::TakeoverEvent) + Send>> {
    use super::host::TakeoverEvent;
    let taps = tap_sender(cx)?;
    Some(Box::new(move |event| {
        let event = match event {
            TakeoverEvent::Paused {
                thread_id,
                pid,
                window_id,
            } => TapEvent::UserPaused {
                thread_id,
                window_id: window_id.map(|window| format!("cua:{pid}:{window}")),
            },
            TakeoverEvent::Resumed { thread_id } => TapEvent::UserResumed { thread_id },
        };
        let _ = taps.unbounded_send(event);
    }))
}

pub fn watch_events(engine: EngineHandle, cx: &mut App) -> Option<Task<()>> {
    let store = store(cx)?.downgrade();
    Some(cx.spawn(async move |cx| {
        loop {
            let subscribed = engine
                .client()
                .subscribe(methods::WATCH_COMPUTER_EVENTS, serde_json::json!({}))
                .await;
            let mut events = match subscribed {
                Ok(events) => events,
                Err(RpcError::UnknownMethod(_)) => return,
                Err(_) => {
                    cx.background_executor().timer(Duration::from_secs(2)).await;
                    continue;
                }
            };
            let Some(live) = store.upgrade() else {
                return;
            };
            cx.update(|cx| {
                update_store(&live, cx, |store| {
                    store.set_events_live(true);
                    false
                })
            });
            while let Some(value) = events.recv().await {
                let Ok(event) = serde_json::from_value::<ComputerEvent>(value) else {
                    continue;
                };
                let Some(live) = store.upgrade() else {
                    return;
                };
                cx.update(|cx| update_store(&live, cx, |store| store.apply_event(&event)));
            }
            let Some(live) = store.upgrade() else {
                return;
            };
            cx.update(|cx| {
                update_store(&live, cx, |store| {
                    store.set_events_live(false);
                    false
                })
            });
            cx.background_executor().timer(Duration::from_secs(2)).await;
        }
    }))
}

pub fn fetch_settings(
    engine: EngineHandle,
    store: Entity<ComputerPreviewStore>,
    cx: &mut App,
) -> Task<()> {
    cx.spawn(async move |cx| {
        let reply = engine
            .client()
            .call(methods::GET_COMPUTER_SETTINGS, serde_json::json!({}))
            .await;
        let settings = reply
            .ok()
            .and_then(|value| serde_json::from_value::<ComputerSettings>(value).ok());
        cx.update(|cx| {
            update_store(&store, cx, |store| match settings {
                Some(settings) => store.set_settings(settings),
                None => {
                    store.clear_settings_stale();
                    false
                }
            })
        });
    })
}

pub fn forget_chats(chat_ids: &[String], cx: &mut App) {
    let Some(store) = store(cx).filter(|_| !chat_ids.is_empty()) else {
        return;
    };
    update_store(&store, cx, |store| {
        chat_ids.iter().fold(false, |changed, chat_id| {
            store.remove_chat(chat_id) | changed
        })
    });
}

pub async fn read_still(engine: &EngineHandle, chat_id: &str) -> Option<(u64, Vec<u8>)> {
    let reply = engine
        .client()
        .call(
            methods::GET_COMPUTER_STILL_FRAME,
            serde_json::json!({ "chatId": chat_id }),
        )
        .await
        .ok()?;
    parse_still(&reply)
}

pub fn parse_still(reply: &serde_json::Value) -> Option<(u64, Vec<u8>)> {
    use base64::Engine as _;
    let frame: zeron_proto::computer::ComputerStillFrame =
        serde_json::from_value(reply.get("frame")?.clone()).ok()?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(frame.data.as_bytes())
        .ok()?;
    Some((frame.sequence, bytes))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn thread_state(thread: &str, agent_active: bool) -> ThreadComputerState {
        serde_json::from_value(serde_json::json!({
            "threadId": thread,
            "version": 1,
            "computerId": "local",
            "windows": [{ "id": "w1", "title": "Calc", "appName": "Calculator", "focused": true, "minimized": false, "visible": true }],
            "screenSize": { "width": 1440, "height": 900 },
            "agentActive": agent_active,
            "controlledByOtherThread": false,
            "availability": { "kind": "available", "backend": "mac" },
            "health": { "status": "connected", "consecutiveFailures": 0, "reconnects": 0, "captureAvailable": true },
            "capabilities": {
                "windows": true, "windowBounds": true, "stacking": false, "capture": true, "input": true,
                "clipboard": false, "focus": true, "raise": true, "ghostCursor": true, "visibleDesktop": true
            }
        }))
        .expect("thread state")
    }

    pub(crate) fn image(seq: u64, width: u32, height: u32) -> PreviewImage {
        PreviewImage {
            seq,
            image: Arc::new(RenderImage::new([image::Frame::new(
                image::RgbaImage::new(width, height),
            )])),
            width,
            height,
        }
    }

    pub(crate) fn driving(store: &mut ComputerPreviewStore, thread: &str) {
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(thread_state(thread, true)),
        });
    }

    #[test]
    fn the_newest_tap_frame_wins_and_older_ones_are_evicted() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        assert!(store.accept_tap_frame("a", image(2, 960, 600)));
        assert!(!store.accept_tap_frame("a", image(1, 960, 600)));
        assert!(!store.accept_tap_frame("a", image(2, 960, 600)));
        assert_eq!(store.view("a").frame.unwrap().seq, 2);
        assert_eq!(store.take_evicted().len(), 2, "stale frames never linger");
        assert!(store.accept_tap_frame("a", image(3, 960, 600)));
        assert_eq!(store.view("a").frame.unwrap().seq, 3);
        assert_eq!(
            store.take_evicted().len(),
            1,
            "the replaced frame is released"
        );
    }

    #[test]
    fn frames_stay_with_their_own_chat() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        driving(&mut store, "b");
        store.accept_tap_frame("a", image(1, 960, 600));
        assert!(store.view("a").frame.is_some());
        assert!(store.view("b").frame.is_none(), "b never draws a's window");
        store.accept_still("b", Some(image(7, 640, 400)));
        assert_eq!(store.view("a").frame.unwrap().seq, 1);
        assert_eq!(store.view("b").frame.unwrap().seq, 7);
    }

    #[test]
    fn a_bystander_chat_never_draws_a_tap_frame() {
        let mut store = ComputerPreviewStore::default();
        let mut state = thread_state("b", true);
        state.controlled_by_other_thread = true;
        state.control_owner_thread_id = Some("a".into());
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(state),
        });
        store.accept_tap_frame("b", image(1, 960, 600));
        assert!(store.view("b").frame.is_none());
        assert_eq!(
            store.view("b").badge.unwrap().label,
            "Another conversation is controlling"
        );
    }

    #[test]
    fn the_session_arms_goes_live_hides_for_the_task_and_rearms_on_the_next_turn() {
        let mut store = ComputerPreviewStore::default();
        assert_eq!(store.view("a").phase, None);
        driving(&mut store, "a");
        assert_eq!(store.view("a").phase, Some(PreviewPhase::Armed));
        assert!(!store.view("a").open, "armed renders closed until viewed");
        store.mark_live("a");
        store.accept_tap_frame("a", image(1, 960, 600));
        assert!(store.view("a").visually_open());
        assert!(store.hide_for_task("a"));
        assert_eq!(store.view("a").phase, Some(PreviewPhase::HiddenForTask));
        assert!(!store.view("a").open);
        driving(&mut store, "a");
        store.apply_event(&ComputerEvent::OpenPaneRequested {
            thread_id: "a".into(),
        });
        assert_eq!(
            store.view("a").phase,
            Some(PreviewPhase::Armed),
            "a new lease re-arms a hidden preview"
        );
        store.hide_for_task("a");
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(thread_state("a", true)),
        });
        assert_eq!(
            store.view("a").phase,
            Some(PreviewPhase::HiddenForTask),
            "the same task keeps it hidden"
        );
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(thread_state("a", false)),
        });
        assert_eq!(store.view("a").phase, Some(PreviewPhase::Ended));
        driving(&mut store, "a");
        assert_eq!(store.view("a").phase, Some(PreviewPhase::Armed));
        assert!(store.view("a").frame.is_none(), "a new task starts clean");
    }

    #[test]
    fn a_closed_window_keeps_the_last_frame_or_shows_the_error() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        store.mark_live("a");
        let gone = PreviewFailure {
            code: "target_unavailable".into(),
            message: "The selected window is no longer available.".into(),
        };
        store.accept_tap_failure("a", gone.clone());
        let view = store.view("a");
        assert!(view.frame.is_none());
        assert_eq!(view.failure.as_ref(), Some(&gone));
        assert!(view.visually_open(), "a first-frame error opens the card");
        store.accept_tap_frame("a", image(1, 960, 600));
        store.accept_tap_failure("a", gone);
        let view = store.view("a");
        assert_eq!(view.frame.unwrap().seq, 1, "the last frame stays up");
        assert!(view.failure.is_none());
        assert!(
            !view.wants_stills,
            "a held tap frame is never replaced by stills"
        );
    }

    #[test]
    fn a_non_host_viewer_falls_back_to_stills_and_arms_from_them() {
        let mut store = ComputerPreviewStore::default();
        assert!(!store.events_live());
        assert!(
            store.view("a").wants_stills,
            "a remote viewer looks for stills"
        );
        assert!(store.accept_still("a", Some(image(4, 640, 400))));
        assert_eq!(store.view("a").phase, Some(PreviewPhase::Armed));
        store.mark_live("a");
        let view = store.view("a");
        assert_eq!(view.source, FrameSource::Stills);
        assert_eq!(view.frame.unwrap().seq, 4);
        assert_eq!(view.status_label.as_deref(), Some("Live"));
        store.accept_still("a", None);
        assert_eq!(store.view("a").phase, Some(PreviewPhase::Ended));
    }

    #[test]
    fn the_host_uses_stills_only_until_the_tap_draws() {
        let mut store = ComputerPreviewStore::default();
        store.set_events_live(true);
        assert!(
            !store.view("a").wants_stills,
            "the host arms from events only"
        );
        assert!(store.accept_still("a", Some(image(1, 640, 400))));
        assert_eq!(
            store.view("a").phase,
            None,
            "a still never arms on the host"
        );
        driving(&mut store, "a");
        store.mark_live("a");
        assert!(store.view("a").wants_stills);
        store.accept_still("a", Some(image(2, 640, 400)));
        assert_eq!(store.view("a").source, FrameSource::Stills);
        store.accept_tap_frame("a", image(1, 960, 600));
        let view = store.view("a");
        assert_eq!(view.source, FrameSource::Tap);
        assert_eq!(view.frame.unwrap().width, 960);
        assert!(!view.wants_stills);
    }

    #[test]
    fn auto_open_off_keeps_the_card_closed_and_stops_polling() {
        let mut store = ComputerPreviewStore::default();
        store.set_settings(ComputerSettings {
            auto_open_computer_pane: false,
            ..ComputerSettings::default()
        });
        driving(&mut store, "a");
        store.mark_live("a");
        store.accept_tap_frame("a", image(1, 960, 600));
        let view = store.view("a");
        assert!(!view.open);
        assert!(!view.visually_open());
        assert!(!view.wants_stills);
    }

    #[test]
    fn reopen_brings_a_hidden_preview_back() {
        let mut store = ComputerPreviewStore::default();
        assert!(
            !store.reopen("a"),
            "a chat that never drove has nothing to reopen"
        );
        driving(&mut store, "a");
        store.mark_live("a");
        store.hide_for_task("a");
        assert!(!store.view("a").open);
        assert!(store.reopen("a"));
        assert_eq!(store.view("a").phase, Some(PreviewPhase::Live));
        assert!(store.view("a").open);
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(thread_state("a", false)),
        });
        assert_eq!(store.view("a").phase, Some(PreviewPhase::Ended));
        assert!(!store.view("a").open, "the drive's end closes it");
        assert!(store.reopen("a"), "an ended drive reopens too");
        assert!(store.view("a").open);
        assert!(store.hide_for_task("a"));
        assert!(!store.view("a").open, "hiding undoes the reopen");

        store.set_settings(ComputerSettings {
            auto_open_computer_pane: false,
            ..ComputerSettings::default()
        });
        driving(&mut store, "a");
        store.mark_live("a");
        assert!(!store.view("a").open, "a new task forgets the reopen");
        assert!(store.reopen("a"));
        let view = store.view("a");
        assert!(view.open, "a click opens it even with auto-open off");
        assert!(view.connecting, "open and driving with no frame yet");
        store.accept_tap_frame("a", image(1, 960, 600));
        assert!(!store.view("a").connecting);
    }

    #[test]
    fn the_thumbnail_toggles_per_chat() {
        let mut store = ComputerPreviewStore::default();
        assert_eq!(store.placement(), ComputerPreviewPlacement::Rail);
        assert!(!store.thumbnail_shown("a"));
        assert!(!store.toggle_thumbnail("a"));
        driving(&mut store, "a");
        driving(&mut store, "b");
        assert!(store.thumbnail_shown("a") && store.thumbnail_shown("b"));
        assert!(store.toggle_thumbnail("a"));
        assert!(!store.thumbnail_shown("a"));
        assert!(store.thumbnail_shown("b"), "b keeps its own thumbnail");
        store.toggle_thumbnail("a");
        assert!(store.thumbnail_shown("a"));
        store.set_floating("a", Some(gpui::point(10.0, 10.0)));
        assert!(!store.thumbnail_shown("a"), "a popped-out card replaces it");
        store.set_floating("a", None);
        store.toggle_thumbnail("a");
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(thread_state("a", false)),
        });
        driving(&mut store, "a");
        assert!(store.thumbnail_shown("a"), "a new task shows it again");

        store.set_settings(ComputerSettings {
            auto_open_computer_pane: false,
            ..ComputerSettings::default()
        });
        assert!(!store.thumbnail_shown("a"));
        store.toggle_thumbnail("a");
        assert!(
            store.thumbnail_shown("a"),
            "a click shows it with auto-open off"
        );
        store.set_settings(ComputerSettings {
            computer_preview_placement: ComputerPreviewPlacement::Card,
            ..ComputerSettings::default()
        });
        assert!(
            !store.thumbnail_shown("a"),
            "the card placement has no thumbnail"
        );
    }

    #[test]
    fn a_revoke_marks_only_its_own_chat() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        driving(&mut store, "b");
        let mut revoked = thread_state("a", true);
        revoked.input_stopped = Some(true);
        assert!(store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(revoked),
        }));
        assert!(!store.apply_event(&ComputerEvent::InputStopped { stopped: true }));
        assert_eq!(
            store.view("a").status_label.as_deref(),
            Some("Desktop access revoked")
        );
        assert_eq!(store.view("b").status_label.as_deref(), Some("Live"));
        driving(&mut store, "a");
        assert_eq!(store.view("a").status_label.as_deref(), Some("Live"));
    }

    #[test]
    fn labels_follow_synaras_popover_and_badge() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        assert_eq!(store.view("a").status_label.as_deref(), Some("Live"));
        store.apply_event(&ComputerEvent::Action {
            window_id: Some("w1".into()),
            delivery: serde_json::from_value(
                serde_json::json!({ "path": "background_ax", "verified": "unconfirmed" }),
            )
            .ok(),
            action: "computer_click".into(),
            ok: true,
            message: None,
            thread_id: Some("a".into()),
            restored_window_id: None,
            restore_status: None,
            masked: false,
        });
        assert_eq!(
            store.view("a").status_label.as_deref(),
            Some("Click · Calculator · Background action")
        );
        let mut active = thread_state("a", true);
        active.activity = Some("Typing in Calculator".into());
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(active.clone()),
        });
        assert_eq!(
            store.view("a").status_label.as_deref(),
            Some("Typing in Calculator")
        );
        active.input_stopped = Some(true);
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(active.clone()),
        });
        assert_eq!(
            store.view("a").status_label.as_deref(),
            Some("Desktop access revoked")
        );
        active.input_stopped = None;
        assert_eq!(
            status_badge(Some(&active), true, true).unwrap().label,
            "Typing in Calculator"
        );
        active.activity = None;
        assert_eq!(
            status_badge(Some(&active), true, true).unwrap().label,
            "Agent controlling this computer"
        );
        active.input_pause = Some(zeron_proto::computer::ComputerInputPause {
            window_id: None,
            pid: None,
            message: "Screen locked".into(),
            user_takeover: false,
        });
        let badge = status_badge(Some(&active), true, true).unwrap();
        assert_eq!(
            (badge.label.as_str(), badge.tone),
            ("Input paused", BadgeTone::Warning)
        );
        active.health.status = ComputerHealthStatus::Unavailable;
        assert_eq!(
            status_badge(Some(&active), true, true).unwrap().label,
            "Input paused",
            "a lazy backend with no failure is not badged"
        );
        active.health.consecutive_failures = 2;
        let badge = status_badge(Some(&active), true, true).unwrap();
        assert_eq!(
            (badge.label.as_str(), badge.tone),
            ("Desktop unavailable", BadgeTone::Danger)
        );
        assert!(
            badge
                .title
                .unwrap()
                .contains("Failed attempts since the last connection: 2.")
        );
        assert_eq!(
            status_badge(Some(&thread_state("a", false)), false, true),
            None
        );
        assert_eq!(
            action_status_label("computer_press_key", Some("Ghostty"), Some("foreground"))
                .as_deref(),
            Some("Press a key · Ghostty · Temporary foreground")
        );
    }

    #[test]
    fn a_floating_card_moves_only_while_detached_and_survives_the_task() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        assert!(
            !store.move_floating("a", gpui::point(1.0, 1.0)),
            "a docked card does not drag"
        );
        assert!(store.set_floating("a", Some(gpui::point(40.0, 60.0))));
        assert!(store.move_floating("a", gpui::point(50.0, 70.0)));
        assert!(!store.move_floating("a", gpui::point(50.0, 70.0)));
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(thread_state("a", false)),
        });
        driving(&mut store, "a");
        assert_eq!(store.view("a").floating, Some(gpui::point(50.0, 70.0)));
        assert!(store.set_floating("a", None));
        assert!(store.view("a").floating.is_none());
        assert!(!store.set_floating("gone", None));
        assert!(!store.remove_chat("gone"));
        store.accept_tap_frame("a", image(1, 4, 4));
        store.accept_still("a", Some(image(1, 4, 4)));
        store.take_evicted();
        assert!(store.remove_chat("a"));
        assert_eq!(
            store.take_evicted().len(),
            2,
            "a removed chat frees its frames"
        );
        assert_eq!(store.view("a").phase, None);
    }

    #[test]
    fn decoded_frames_are_bgra_for_gpui() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let frame = decode_frame(9, &png).unwrap();
        assert_eq!((frame.seq, frame.width, frame.height), (9, 4, 2));
        let bytes = frame.image.as_bytes(0).unwrap();
        assert_eq!(&bytes[..4], &[0, 0, 255, 255]);
        assert!(decode_frame(1, b"not an image").is_none());
    }

    #[test]
    fn a_still_reply_decodes_and_null_means_none() {
        use base64::Engine as _;
        let data = base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]);
        let reply = serde_json::json!({ "frame": { "sequence": 5, "timestampMs": 1.0, "keyframe": true, "data": data } });
        assert_eq!(parse_still(&reply), Some((5, vec![1, 2, 3])));
        assert_eq!(parse_still(&serde_json::json!({ "frame": null })), None);
    }

    #[test]
    fn a_batch_keeps_the_newest_frame_per_chat() {
        let frame = |thread: &str, seq| TapEvent::Frame {
            thread_id: thread.into(),
            seq,
            jpeg: Vec::new(),
        };
        let failed = TapEvent::Failed {
            thread_id: "a".into(),
            code: "target_unavailable".into(),
            message: "gone".into(),
        };
        let kept = coalesce(vec![
            frame("a", 1),
            frame("b", 2),
            frame("a", 3),
            failed.clone(),
        ]);
        assert_eq!(kept, vec![frame("b", 2), frame("a", 3), failed]);
    }
}
