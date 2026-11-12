use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use tokio::sync::watch;
pub use zeron_proto::prediction::ComposerPrediction as Prediction;

// Suggestions outlive their runs, so only the most recent are kept.
const KEPT: usize = 256;

#[derive(Default)]
struct ChatSlot {
    tx: Option<watch::Sender<Option<Prediction>>>,
    generation: u64,
    offered: u64,
}

impl ChatSlot {
    fn sender(&mut self) -> &watch::Sender<Option<Prediction>> {
        self.tx.get_or_insert_with(|| watch::channel(None).0)
    }

    fn holds(&self) -> bool {
        self.tx.as_ref().is_some_and(|tx| tx.borrow().is_some())
    }

    fn take(&mut self) {
        if let Some(tx) = &self.tx {
            tx.send_if_modified(|value| value.take().is_some());
        }
    }
}

#[derive(Default)]
struct Chats {
    slots: HashMap<String, ChatSlot>,
    offers: u64,
}

#[derive(Default)]
pub(crate) struct Predictions {
    chats: Mutex<Chats>,
}

impl Predictions {
    fn chats(&self) -> MutexGuard<'_, Chats> {
        self.chats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn watch(&self, chat_id: &str) -> watch::Receiver<Option<Prediction>> {
        self.chats()
            .slots
            .entry(chat_id.to_string())
            .or_default()
            .sender()
            .subscribe()
    }

    // A suggestion offered with this lands only if nothing cleared the chat since.
    pub(crate) fn ticket(&self, chat_id: &str) -> u64 {
        self.chats()
            .slots
            .entry(chat_id.to_string())
            .or_default()
            .generation
    }

    pub(crate) fn offer(&self, chat_id: &str, ticket: u64, entry_id: String, text: String) {
        let mut chats = self.chats();
        chats.offers += 1;
        let offers = chats.offers;
        let Some(slot) = chats
            .slots
            .get_mut(chat_id)
            .filter(|slot| slot.generation == ticket)
        else {
            return;
        };
        slot.offered = offers;
        slot.sender()
            .send_replace(Some(Prediction { entry_id, text }));
        let held = chats.slots.values().filter(|slot| slot.holds()).count();
        if held > KEPT
            && let Some(oldest) = chats
                .slots
                .values_mut()
                .filter(|slot| slot.holds())
                .min_by_key(|slot| slot.offered)
        {
            oldest.take();
        }
    }

    pub(crate) fn clear(&self, chat_id: &str) {
        let mut chats = self.chats();
        let slot = chats.slots.entry(chat_id.to_string()).or_default();
        slot.generation += 1;
        slot.take();
    }

    pub(crate) fn clear_all(&self) {
        for slot in self.chats().slots.values_mut() {
            slot.generation += 1;
            slot.take();
        }
    }

    pub(crate) fn forget(&self, chat_id: &str) {
        if let Some(mut slot) = self.chats().slots.remove(chat_id) {
            slot.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(rx: &mut watch::Receiver<Option<Prediction>>) -> Option<String> {
        rx.borrow_and_update().as_ref().map(|p| p.text.clone())
    }

    #[test]
    fn a_suggestion_lands_unless_the_chat_was_cleared_since_its_turn() {
        let predictions = Predictions::default();
        let ticket = predictions.ticket("c");
        predictions.offer("c", ticket, "a1".into(), "ship it".into());
        let mut rx = predictions.watch("c");
        assert_eq!(value(&mut rx).as_deref(), Some("ship it"));

        let ticket = predictions.ticket("c");
        predictions.clear("c");
        assert_eq!(value(&mut rx), None);
        predictions.offer("c", ticket, "a2".into(), "too late".into());
        assert_eq!(value(&mut rx), None);
    }

    #[test]
    fn a_forgotten_chat_takes_no_suggestion() {
        let predictions = Predictions::default();
        let ticket = predictions.ticket("c");
        let mut rx = predictions.watch("c");
        predictions.offer("c", ticket, "a1".into(), "next".into());
        predictions.forget("c");
        assert!(rx.has_changed().unwrap_or(true));
        predictions.offer("c", ticket, "a1".into(), "next".into());
        assert_eq!(value(&mut predictions.watch("c")), None);
    }

    #[test]
    fn only_the_most_recent_suggestions_are_kept() {
        let predictions = Predictions::default();
        for ix in 0..=KEPT {
            let chat = format!("c{ix}");
            let ticket = predictions.ticket(&chat);
            predictions.offer(&chat, ticket, "a".into(), format!("next {ix}"));
        }
        assert_eq!(value(&mut predictions.watch("c0")), None);
        assert_eq!(
            value(&mut predictions.watch(&format!("c{KEPT}"))),
            Some(format!("next {KEPT}"))
        );
        assert_eq!(
            value(&mut predictions.watch("c1")).as_deref(),
            Some("next 1")
        );
    }
}
