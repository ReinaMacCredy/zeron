use gpui::{App, Entity};
use zeron_proto::computer::ThreadComputerState;

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
}

impl ChatPresence {
    pub fn holds_desktop(&self) -> bool {
        matches!(
            self.tone,
            PresenceTone::Driving | PresenceTone::Paused | PresenceTone::NeedsYou
        )
    }
}

fn pause_target(state: &ThreadComputerState) -> Option<String> {
    let window_id = state.input_pause.as_ref()?.window_id.as_deref()?;
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
    let own_pause =
        state.filter(|state| state.input_pause.is_some() && !state.controlled_by_other_thread);
    let tone = if chat.revoked_latched {
        PresenceTone::Revoked {
            summing_up: state.is_some_and(|state| state.input_stopped == Some(true)),
        }
    } else if own_pause.is_some() {
        PresenceTone::Paused
    } else if state.is_some_and(preview::agent_active) {
        PresenceTone::Driving
    } else {
        return None;
    };
    let (app, window_id) = match own_pause.filter(|_| tone == PresenceTone::Paused) {
        Some(state) => (
            pause_target(state),
            state
                .input_pause
                .as_ref()
                .and_then(|pause| pause.window_id.clone())
                .or_else(|| chat.last_window_id.clone()),
        ),
        None => (chat.last_app.clone(), chat.last_window_id.clone()),
    };
    Some(ChatPresence {
        chat_id: chat_id.to_owned(),
        app,
        window_id,
        activity: state.and_then(|state| state.activity.clone()),
        tone,
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

#[derive(Debug, Default)]
pub struct ComputerPresenceMap {
    chats: Vec<ChatPresence>,
}

impl ComputerPresenceMap {
    pub fn set(&mut self, next: Vec<ChatPresence>) -> bool {
        if self.chats == next {
            return false;
        }
        self.chats = next;
        true
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
    fn an_unreachable_window_pauses_and_names_its_app() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        let mut paused = thread_state("a", true);
        paused.input_pause = Some(ComputerInputPause {
            window_id: Some("w1".into()),
            pid: None,
            message: "Screen locked".into(),
        });
        state(&mut store, paused.clone());
        let presence = chat_presence(&store, "a").unwrap();
        assert_eq!(presence.tone, PresenceTone::Paused);
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
