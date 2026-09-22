use sha2::{Digest, Sha256};

pub fn frame_digest(bytes: &[u8]) -> String {
    let hash: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{}:{hash}", bytes.len())
}

#[derive(Debug, Clone, Default)]
pub struct StillFrameDedupe {
    published_digest: Option<String>,
    pending_force: bool,
}

impl StillFrameDedupe {
    pub fn defer_force(&mut self) {
        self.pending_force = true;
    }

    pub fn take_force(&mut self, explicit: bool) -> bool {
        let force = explicit || self.pending_force;
        self.pending_force = false;
        force
    }

    pub fn force_pending(&self) -> bool {
        self.pending_force
    }

    pub fn should_publish(&mut self, bytes: &[u8], force: bool) -> bool {
        let digest = frame_digest(bytes);
        if !force && self.published_digest.as_deref() == Some(digest.as_str()) {
            return false;
        }
        self.published_digest = Some(digest);
        true
    }

    pub fn reset(&mut self) {
        self.published_digest = None;
        self.pending_force = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: &[u8] = &[1, 2, 3, 4];
    const CHANGED: &[u8] = &[1, 2, 3, 5];
    const SHORTER: &[u8] = &[1, 2, 3];

    #[test]
    fn separates_frames_of_different_sizes_before_hashing_them() {
        assert_ne!(frame_digest(IDLE), frame_digest(SHORTER));
        assert_eq!(frame_digest(IDLE), frame_digest(&[1, 2, 3, 4]));
    }

    #[test]
    fn publishes_the_first_frame_and_suppresses_an_identical_repeat() {
        let mut dedupe = StillFrameDedupe::default();
        let force = dedupe.take_force(false);
        assert!(dedupe.should_publish(IDLE, force));
        let force = dedupe.take_force(false);
        assert!(!dedupe.should_publish(IDLE, force));
        let force = dedupe.take_force(false);
        assert!(dedupe.should_publish(CHANGED, force));
    }

    #[test]
    fn publishes_an_identical_frame_when_the_receiver_has_nothing_to_draw() {
        let mut dedupe = StillFrameDedupe::default();
        let force = dedupe.take_force(false);
        dedupe.should_publish(IDLE, force);
        let force = dedupe.take_force(true);
        assert!(dedupe.should_publish(IDLE, force));
    }

    #[test]
    fn remembers_a_keyframe_asked_for_while_a_capture_was_already_running() {
        let mut dedupe = StillFrameDedupe::default();
        let force = dedupe.take_force(false);
        dedupe.should_publish(IDLE, force);
        dedupe.defer_force();
        assert!(dedupe.force_pending());
        let force = dedupe.take_force(false);
        assert!(force);
        assert!(!dedupe.force_pending());
        assert!(dedupe.should_publish(IDLE, force));
    }

    #[test]
    fn clears_both_the_digest_and_any_owed_keyframe_on_reset() {
        let mut dedupe = StillFrameDedupe::default();
        let force = dedupe.take_force(false);
        dedupe.should_publish(IDLE, force);
        dedupe.defer_force();
        dedupe.reset();
        assert!(!dedupe.force_pending());
        let force = dedupe.take_force(false);
        assert!(dedupe.should_publish(IDLE, force));
    }
}
