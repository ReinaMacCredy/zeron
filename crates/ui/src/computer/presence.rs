use gpui::{App, Entity};
use zeron_proto::computer::{ComputerPendingKind, ComputerPendingRequest, ThreadComputerState};

use super::preview::{self, ComputerPreviewStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceTone {
    Driving,
    Paused,
    NeedsYou,
    Revoked { summing_up: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatPresence {
    pub chat_id: String,
    pub app: Option<String>,
    pub window_id: Option<String>,
    pub activity: Option<String>,
    pub tone: PresenceTone,
    pub user_takeover: bool,
    pub pending: Option<ComputerPendingRequest>,
}

impl ChatPresence {
    pub fn holds_desktop(&self) -> bool {
        matches!(
            self.tone,
            PresenceTone::Driving | PresenceTone::Paused | PresenceTone::NeedsYou
        )
    }
}

fn window_target(state: &ThreadComputerState, window_id: &str) -> Option<String> {
    let window = state.windows.iter().find(|window| window.id == window_id)?;
    window
        .app_name
        .clone()
        .or_else(|| Some(window.title.clone()))
        .filter(|target| !target.trim().is_empty())
}

pub fn chat_presence(store: &ComputerPreviewStore, chat_id: &str) -> Option<ChatPresence> {
    let chat = store.chat_preview(chat_id)?;
    let state = chat.state.as_ref();
    let own = state.filter(|state| !state.controlled_by_other_thread);
    let own_pause = own.and_then(|state| state.input_pause.as_ref());
    let pending = own
        .and_then(|state| state.pending_request.clone())
        .filter(|_| !chat.revoked_latched);
    let tone = if chat.revoked_latched {
        PresenceTone::Revoked {
            summing_up: state.is_some_and(|state| state.input_stopped == Some(true)),
        }
    } else if pending.is_some() {
        PresenceTone::NeedsYou
    } else if own_pause.is_some() || chat.user_paused {
        PresenceTone::Paused
    } else if state.is_some_and(preview::agent_active) {
        PresenceTone::Driving
    } else {
        return None;
    };
    let paused = tone == PresenceTone::Paused;
    let (app, window_id) = match (own_pause.filter(|_| paused), state) {
        (Some(pause), Some(state)) => (
            pause
                .window_id
                .as_deref()
                .and_then(|window_id| window_target(state, window_id)),
            pause
                .window_id
                .clone()
                .or_else(|| chat.last_window_id.clone()),
        ),
        _ if paused => {
            let window_id = chat
                .user_paused_window
                .clone()
                .or_else(|| chat.last_window_id.clone());
            let app = window_id
                .as_deref()
                .and_then(|window_id| state.and_then(|state| window_target(state, window_id)))
                .or_else(|| {
                    (window_id == chat.last_window_id)
                        .then(|| chat.last_app.clone())
                        .flatten()
                });
            (app, window_id)
        }
        _ => {
            let pending_app = pending.as_ref().and_then(|pending| match &pending.kind {
                ComputerPendingKind::Foreground { app } => app.clone(),
                ComputerPendingKind::Consent { app, .. } => {
                    app.as_ref().map(|app| app.name.clone())
                }
            });
            (
                pending_app.or_else(|| chat.last_app.clone()),
                chat.last_window_id.clone(),
            )
        }
    };
    Some(ChatPresence {
        chat_id: chat_id.to_owned(),
        app,
        window_id,
        activity: state.and_then(|state| state.activity.clone()),
        tone,
        user_takeover: paused
            && (chat.user_paused || own_pause.is_some_and(|pause| pause.user_takeover)),
        pending,
    })
}

pub fn presences(store: &ComputerPreviewStore) -> Vec<ChatPresence> {
    let mut presences: Vec<_> = store
        .chat_ids()
        .filter_map(|chat_id| chat_presence(store, chat_id))
        .collect();
    presences.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
    presences
}

pub const RING_LIMIT: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingTone {
    Accent,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RingSpec {
    pub chat_id: String,
    pub pid: i32,
    pub window: u32,
    pub tone: RingTone,
    pub dashed: bool,
    pub label: String,
}

fn cua_window(window_id: &str) -> Option<(i32, u32)> {
    let (pid, window) = window_id.strip_prefix("cua:")?.split_once(':')?;
    let valid = |part: &str| {
        !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) && !part.starts_with('0')
    };
    if !valid(pid) || !valid(window) {
        return None;
    }
    Some((pid.parse().ok()?, window.parse().ok()?))
}

pub fn ring_specs(presences: &[ChatPresence]) -> Vec<RingSpec> {
    presences
        .iter()
        .filter_map(|presence| {
            let (pid, window) = cua_window(presence.window_id.as_deref()?)?;
            let (tone, dashed, label) = match presence.tone {
                PresenceTone::Driving => (
                    RingTone::Accent,
                    false,
                    presence.activity.as_deref().unwrap_or("Working").to_owned(),
                ),
                PresenceTone::Paused if presence.user_takeover => {
                    (RingTone::Warning, false, "You're using it".to_owned())
                }
                PresenceTone::Paused => (RingTone::Warning, false, "Paused".to_owned()),
                PresenceTone::NeedsYou => (RingTone::Accent, true, "Waiting for you".to_owned()),
                PresenceTone::Revoked { .. } => return None,
            };
            Some(RingSpec {
                chat_id: presence.chat_id.clone(),
                pid,
                window,
                tone,
                dashed,
                label: format!("Zeron · {label}"),
            })
        })
        .take(RING_LIMIT)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailEntry {
    pub chat_id: String,
    pub app: Option<String>,
    pub thumbnail: bool,
}

pub fn rail_entries(store: &ComputerPreviewStore) -> Vec<RailEntry> {
    let mut entries: Vec<_> = store
        .chat_ids()
        .filter_map(|chat_id| {
            let chat = store.chat_preview(chat_id)?;
            chat.phase.is_some().then(|| RailEntry {
                chat_id: chat_id.to_owned(),
                app: chat.last_app.clone(),
                thumbnail: store.thumbnail_shown(chat_id),
            })
        })
        .collect();
    entries.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
    entries
}

#[derive(Debug, Default)]
pub struct ComputerPresenceMap {
    chats: Vec<ChatPresence>,
    rail: Vec<RailEntry>,
}

impl ComputerPresenceMap {
    pub fn set(&mut self, next: Vec<ChatPresence>) -> bool {
        if self.chats == next {
            return false;
        }
        self.chats = next;
        true
    }

    pub fn set_rail(&mut self, next: Vec<RailEntry>) -> bool {
        if self.rail == next {
            return false;
        }
        self.rail = next;
        true
    }

    pub fn rail_entry(&self, chat_id: &str) -> Option<&RailEntry> {
        self.rail.iter().find(|entry| entry.chat_id == chat_id)
    }

    pub fn get(&self, chat_id: &str) -> Option<&ChatPresence> {
        self.chats
            .iter()
            .find(|presence| presence.chat_id == chat_id)
    }

    pub fn chats(&self) -> &[ChatPresence] {
        &self.chats
    }
}

pub fn map(cx: &App) -> Option<Entity<ComputerPresenceMap>> {
    preview::presence_map(cx)
}

pub fn for_chat(chat_id: &str, cx: &App) -> Option<ChatPresence> {
    map(cx)?.read(cx).get(chat_id).cloned()
}

pub fn tone(chat_id: &str, cx: &App) -> Option<PresenceTone> {
    map(cx)?.read(cx).get(chat_id).map(|presence| presence.tone)
}

pub fn rail_entry(chat_id: &str, cx: &App) -> Option<RailEntry> {
    map(cx)?.read(cx).rail_entry(chat_id).cloned()
}

#[cfg(test)]
mod tests {
    use zeron_proto::computer::{ComputerEvent, ComputerInputPause};

    use super::*;
    use crate::computer::preview::tests::{driving, image, thread_state};

    fn state(store: &mut ComputerPreviewStore, state: ThreadComputerState) -> bool {
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(state),
        })
    }

    fn action(store: &mut ComputerPreviewStore, thread: &str, window: &str, tool: &str) -> bool {
        store.apply_event(&ComputerEvent::Action {
            window_id: Some(window.into()),
            delivery: None,
            action: tool.into(),
            ok: true,
            message: Some("typed hunter2 into the prompt".into()),
            thread_id: Some(thread.into()),
            restored_window_id: None,
            restore_status: None,
            masked: false,
        })
    }

    #[test]
    fn driving_names_the_app_and_activity() {
        let mut store = ComputerPreviewStore::default();
        assert_eq!(chat_presence(&store, "a"), None);
        driving(&mut store, "a");
        let presence = chat_presence(&store, "a").expect("driving");
        assert_eq!(presence.tone, PresenceTone::Driving);
        assert_eq!(presence.app, None, "no action yet, no app");
        assert!(action(&mut store, "a", "w1", "computer_type_text"));
        let mut typing = thread_state("a", true);
        typing.activity = Some("Typing".into());
        state(&mut store, typing);
        assert_eq!(
            chat_presence(&store, "a"),
            Some(ChatPresence {
                chat_id: "a".into(),
                app: Some("Calculator".into()),
                window_id: Some("w1".into()),
                activity: Some("Typing".into()),
                tone: PresenceTone::Driving,
                user_takeover: false,
                pending: None,
            })
        );
        assert!(
            !action(&mut store, "a", "w1", "computer_type_text"),
            "the same window and label change nothing"
        );
        assert!(action(&mut store, "a", "w9", "computer_type_text"));
        let presence = chat_presence(&store, "a").unwrap();
        assert_eq!(
            (presence.app, presence.window_id.as_deref()),
            (None, Some("w9")),
            "an unknown window never keeps the old app"
        );
        state(&mut store, thread_state("a", false));
        assert_eq!(chat_presence(&store, "a"), None, "idle chats have none");
    }

    #[test]
    fn another_chats_lease_is_not_this_chats_presence() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        let mut bystander = thread_state("b", true);
        bystander.controlled_by_other_thread = true;
        bystander.control_owner_thread_id = Some("a".into());
        bystander.input_pause = Some(ComputerInputPause {
            window_id: Some("w1".into()),
            pid: None,
            message: "Screen locked".into(),
            user_takeover: false,
        });
        state(&mut store, bystander);
        assert_eq!(chat_presence(&store, "b"), None);
        let all = presences(&store);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].chat_id, "a");
    }

    #[test]
    fn the_revoked_latch_holds_until_a_send() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        let mut revoked = thread_state("a", false);
        revoked.input_stopped = Some(true);
        state(&mut store, revoked);
        assert_eq!(
            chat_presence(&store, "a").unwrap().tone,
            PresenceTone::Revoked { summing_up: true }
        );
        state(&mut store, thread_state("a", false));
        assert_eq!(
            chat_presence(&store, "a").unwrap().tone,
            PresenceTone::Revoked { summing_up: false },
            "the turn ended; the latch stays until the next send"
        );
        assert!(store.clear_revoked("a"));
        assert!(!store.clear_revoked("a"));
        assert_eq!(chat_presence(&store, "a"), None);

        let mut revoked = thread_state("a", false);
        revoked.input_stopped = Some(true);
        state(&mut store, revoked.clone());
        store.clear_revoked("a");
        state(&mut store, revoked);
        assert_eq!(
            chat_presence(&store, "a"),
            None,
            "a repeat of the same revoked state does not re-latch after a send"
        );

        let mut revoked = thread_state("b", true);
        revoked.input_stopped = Some(true);
        state(&mut store, revoked);
        state(&mut store, thread_state("b", false));
        driving(&mut store, "b");
        assert_eq!(
            chat_presence(&store, "b").unwrap().tone,
            PresenceTone::Driving,
            "a new drive clears the latch"
        );
    }

    #[test]
    fn a_tap_frame_does_not_change_presence() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        action(&mut store, "a", "w1", "computer_click");
        let mut map = ComputerPresenceMap::default();
        assert!(map.set(presences(&store)));
        for seq in 1..=5 {
            assert!(store.accept_tap_frame("a", image(seq, 960, 600)));
            assert!(!map.set(presences(&store)), "frame {seq} re-rendered");
        }
        assert!(!map.set(presences(&store)));
        assert!(store.remove_chat("a"));
        assert!(map.set(presences(&store)));
        assert!(map.chats().is_empty());
    }

    #[test]
    fn the_rail_keeps_every_chat_that_drove_and_ignores_frames() {
        let mut store = ComputerPreviewStore::default();
        let mut map = ComputerPresenceMap::default();
        assert!(!map.set_rail(rail_entries(&store)));
        driving(&mut store, "a");
        action(&mut store, "a", "w1", "computer_click");
        assert!(map.set_rail(rail_entries(&store)));
        let entry = RailEntry {
            chat_id: "a".into(),
            app: Some("Calculator".into()),
            thumbnail: true,
        };
        assert_eq!(map.rail_entry("a"), Some(&entry));
        for seq in 1..=3 {
            store.accept_tap_frame("a", image(seq, 960, 600));
            assert!(!map.set_rail(rail_entries(&store)));
        }
        state(&mut store, thread_state("a", false));
        assert!(
            !map.set_rail(rail_entries(&store)),
            "the entry outlives the drive"
        );
        assert_eq!(chat_presence(&store, "a"), None);
        store.toggle_thumbnail("a");
        assert!(map.set_rail(rail_entries(&store)));
        assert!(!map.rail_entry("a").unwrap().thumbnail);
        assert!(map.rail_entry("b").is_none());
        store.remove_chat("a");
        assert!(map.set_rail(rail_entries(&store)));
        assert!(map.rail_entry("a").is_none());
    }

    fn active(store: &mut ComputerPreviewStore, thread: &str, activity: Option<&str>) {
        let mut driving = thread_state(thread, true);
        driving.activity = activity.map(Into::into);
        state(store, driving);
    }

    #[test]
    fn a_ring_follows_each_driving_chat_target() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        assert!(ring_specs(&presences(&store)).is_empty(), "no target yet");
        action(&mut store, "a", "cua:4242:9191", "computer_type_text");
        active(&mut store, "a", Some("Typing"));
        driving(&mut store, "b");
        action(&mut store, "b", "cua:77:5", "computer_click");
        driving(&mut store, "c");
        action(&mut store, "c", "w1", "computer_click");
        assert_eq!(
            ring_specs(&presences(&store)),
            vec![
                RingSpec {
                    chat_id: "a".into(),
                    pid: 4242,
                    window: 9191,
                    tone: RingTone::Accent,
                    dashed: false,
                    label: "Zeron · Typing".into(),
                },
                RingSpec {
                    chat_id: "b".into(),
                    pid: 77,
                    window: 5,
                    tone: RingTone::Accent,
                    dashed: false,
                    label: "Zeron · Working".into(),
                },
            ],
            "a window id that is not a desktop window gets no ring"
        );
        action(&mut store, "a", "cua:4242:9192", "computer_click");
        assert_eq!(ring_specs(&presences(&store))[0].window, 9192);
        for bad in [
            "cua:0:1",
            "cua:12:01",
            "cua:12",
            "cua::3",
            "cua:-1:2",
            "cua:1:2:3",
        ] {
            assert_eq!(cua_window(bad), None, "{bad}");
        }
        for chat in ["d", "e", "f", "g"] {
            driving(&mut store, chat);
            action(&mut store, chat, "cua:9:9", "computer_click");
        }
        assert_eq!(ring_specs(&presences(&store)).len(), RING_LIMIT);
        let labels: Vec<_> = ring_specs(&presences(&store))
            .into_iter()
            .map(|spec| spec.label)
            .collect();
        assert!(
            labels.iter().all(|label| !label.contains("hunter2")
                && !label.contains("computer_")
                && !label.contains("4242")),
            "{labels:?}"
        );
    }

    #[test]
    fn no_ring_after_revoke_or_idle() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        action(&mut store, "a", "cua:42:7", "computer_click");
        assert_eq!(ring_specs(&presences(&store)).len(), 1);
        let mut revoked = thread_state("a", true);
        revoked.input_stopped = Some(true);
        state(&mut store, revoked);
        assert!(ring_specs(&presences(&store)).is_empty(), "revoked");
        state(&mut store, thread_state("a", false));
        store.clear_revoked("a");
        driving(&mut store, "b");
        action(&mut store, "b", "cua:42:8", "computer_click");
        state(&mut store, thread_state("b", false));
        assert!(ring_specs(&presences(&store)).is_empty(), "idle");
    }

    #[test]
    fn paused_rings_are_warning() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        action(&mut store, "a", "cua:42:7", "computer_click");
        let mut paused = thread_state("a", true);
        paused.input_pause = Some(ComputerInputPause {
            window_id: Some("cua:42:7".into()),
            pid: Some(42),
            message: "Screen locked".into(),
            user_takeover: false,
        });
        state(&mut store, paused);
        let specs = ring_specs(&presences(&store));
        assert_eq!(specs.len(), 1);
        assert_eq!(
            (specs[0].tone, specs[0].dashed, specs[0].label.as_str()),
            (RingTone::Warning, false, "Zeron · Paused")
        );
        let mut waiting = presences(&store).remove(0);
        waiting.tone = PresenceTone::NeedsYou;
        let specs = ring_specs(&[waiting]);
        assert_eq!(
            (specs[0].tone, specs[0].dashed, specs[0].label.as_str()),
            (RingTone::Accent, true, "Zeron · Waiting for you")
        );
    }

    #[test]
    fn a_takeover_event_turns_the_chat_paused_and_resumed_clears_it() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        action(&mut store, "a", "w1", "computer_type_text");
        assert!(store.set_user_paused("a", Some("w1".into())));
        assert!(
            !store.set_user_paused("a", Some("w1".into())),
            "more keys in the same episode change nothing"
        );
        let presence = chat_presence(&store, "a").unwrap();
        assert_eq!(
            (
                presence.tone,
                presence.user_takeover,
                presence.app.as_deref(),
                presence.window_id.as_deref()
            ),
            (PresenceTone::Paused, true, Some("Calculator"), Some("w1"))
        );
        assert!(store.set_user_paused("a", Some("cua:42:7".into())));
        let ring = ring_specs(&presences(&store));
        assert_eq!(
            (ring[0].tone, ring[0].label.as_str(), ring[0].window),
            (RingTone::Warning, "Zeron · You're using it", 7)
        );
        assert!(store.clear_user_paused("a"));
        assert!(!store.clear_user_paused("a"));
        let presence = chat_presence(&store, "a").unwrap();
        assert_eq!(
            (presence.tone, presence.user_takeover),
            (PresenceTone::Driving, false)
        );

        let mut engine_paused = thread_state("a", true);
        engine_paused.input_pause = Some(ComputerInputPause {
            window_id: Some("w1".into()),
            pid: None,
            message: "The user used this window and has stopped.".into(),
            user_takeover: true,
        });
        state(&mut store, engine_paused);
        let presence = chat_presence(&store, "a").unwrap();
        assert_eq!(
            (presence.tone, presence.user_takeover),
            (PresenceTone::Paused, true),
            "the engine's takeover pause also reads as the user's"
        );

        driving(&mut store, "b");
        store.set_user_paused("b", None);
        state(&mut store, thread_state("b", false));
        assert_eq!(chat_presence(&store, "b"), None);
        driving(&mut store, "b");
        assert_eq!(
            chat_presence(&store, "b").unwrap().tone,
            PresenceTone::Driving,
            "the drive ending cleared the held pause"
        );
        assert!(!store.set_user_paused("nobody", None), "no drive, no pause");
    }

    #[test]
    fn a_pending_request_needs_you_and_names_its_app() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        action(&mut store, "a", "cua:42:7", "computer_click");
        let pending = crate::computer::strip::tests::foreground_pending(Some("Ghostty"));
        let mut asking = thread_state("a", false);
        asking.pending_request = Some(pending.clone());
        state(&mut store, asking.clone());
        let presence = chat_presence(&store, "a").expect("waiting for the user");
        assert_eq!(
            (
                presence.tone,
                presence.app.as_deref(),
                presence.window_id.as_deref(),
                presence.pending.as_ref()
            ),
            (
                PresenceTone::NeedsYou,
                Some("Ghostty"),
                Some("cua:42:7"),
                Some(&pending)
            )
        );
        assert!(presence.holds_desktop());
        assert_eq!(ring_specs(&[presence])[0].label, "Zeron · Waiting for you");

        let mut consent = asking.clone();
        consent.pending_request = Some(crate::computer::strip::tests::consent_pending(
            zeron_proto::computer::ComputerConsentScope::Task,
            Some("Calculator"),
            true,
        ));
        state(&mut store, consent);
        let presence = chat_presence(&store, "a").unwrap();
        assert_eq!(
            (presence.tone, presence.app.as_deref()),
            (PresenceTone::NeedsYou, Some("Calculator"))
        );

        let mut bystander = asking.clone();
        bystander.thread_id = "b".into();
        bystander.controlled_by_other_thread = true;
        state(&mut store, bystander);
        assert_eq!(chat_presence(&store, "b"), None);

        let mut revoked = asking;
        revoked.input_stopped = Some(true);
        state(&mut store, revoked);
        assert_eq!(
            chat_presence(&store, "a").unwrap(),
            ChatPresence {
                chat_id: "a".into(),
                app: None,
                window_id: Some("cua:42:7".into()),
                activity: None,
                tone: PresenceTone::Revoked { summing_up: true },
                user_takeover: false,
                pending: None,
            },
            "a revoke outranks the question"
        );
        store.clear_revoked("a");
        state(&mut store, thread_state("a", false));
        assert_eq!(chat_presence(&store, "a"), None, "answered, idle");
    }

    #[test]
    fn an_unreachable_window_pauses_and_names_its_app() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        let mut paused = thread_state("a", true);
        paused.input_pause = Some(ComputerInputPause {
            window_id: Some("w1".into()),
            pid: None,
            message: "Screen locked".into(),
            user_takeover: false,
        });
        state(&mut store, paused.clone());
        let presence = chat_presence(&store, "a").unwrap();
        assert_eq!(presence.tone, PresenceTone::Paused);
        assert!(!presence.user_takeover);
        assert_eq!(presence.app.as_deref(), Some("Calculator"));
        assert_eq!(presence.window_id.as_deref(), Some("w1"));
        paused.input_pause.as_mut().unwrap().window_id = Some("gone".into());
        state(&mut store, paused);
        assert_eq!(chat_presence(&store, "a").unwrap().app, None);
        driving(&mut store, "a");
        assert_eq!(
            chat_presence(&store, "a").unwrap().tone,
            PresenceTone::Driving,
            "the pause clears with the next state"
        );
    }
}
