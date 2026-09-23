//! Pure cleanup policy: restore the captured key window without undoing human focus changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreDecision {
    Restore,
    UserChanged,
    Unobservable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusStamp {
    pub process: [u8; 8],
    pub space: u64,
}

/// A zero AX messaging timeout restores the platform default instead of timing
/// out immediately. Do not start a remote read with an exhausted/tiny budget.
pub fn bounded_ax_timeout(remaining: std::time::Duration) -> Option<f32> {
    (remaining >= std::time::Duration::from_millis(1)).then(|| {
        remaining
            .min(std::time::Duration::from_millis(25))
            .as_secs_f32()
    })
}

pub fn focus_sample_is_fresh(before: FocusStamp, after: FocusStamp, within_budget: bool) -> bool {
    within_budget && before == after
}

pub fn decide_restore(
    same_space: bool,
    front_is_prior: Option<bool>,
    front_is_target: Option<bool>,
    focused_window: Option<u32>,
    prior_window: u32,
    target_window: u32,
    same_process: bool,
) -> RestoreDecision {
    if !same_space {
        return RestoreDecision::UserChanged;
    }
    if front_is_prior == Some(true) {
        // None includes a key window lost to the synthetic defocus record.
        return match focused_window {
            Some(wid) if wid != prior_window && !(same_process && wid == target_window) => {
                if same_process {
                    RestoreDecision::Unobservable
                } else {
                    RestoreDecision::UserChanged
                }
            }
            _ => RestoreDecision::Restore,
        };
    }
    if front_is_target == Some(true) {
        return match focused_window {
            // The action itself can open a sheet/window. Preserve the new
            // focus, but do not claim this proves a human changed it.
            Some(wid) if wid != target_window => RestoreDecision::Unobservable,
            _ => RestoreDecision::Restore,
        };
    }
    if front_is_prior == Some(false) && front_is_target == Some(false) {
        RestoreDecision::UserChanged
    } else {
        RestoreDecision::Unobservable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_activation_restores_the_captured_window() {
        assert_eq!(
            decide_restore(true, Some(false), Some(true), Some(20), 10, 20, false),
            RestoreDecision::Restore
        );
    }

    #[test]
    fn same_process_sibling_and_lost_key_window_are_restored() {
        for focused in [Some(10), Some(20), None] {
            assert_eq!(
                decide_restore(true, Some(true), Some(true), focused, 10, 20, true),
                RestoreDecision::Restore
            );
        }
    }

    #[test]
    fn unexpected_app_window_and_space_changes_are_preserved() {
        for facts in [
            (true, Some(false), Some(false), Some(30), false),
            (true, Some(true), Some(false), Some(30), false),
            (false, Some(false), Some(true), Some(20), false),
        ] {
            assert_eq!(
                decide_restore(facts.0, facts.1, facts.2, facts.3, 10, 20, facts.4),
                RestoreDecision::UserChanged
            );
        }
    }

    #[test]
    fn unexpected_target_sheet_is_uncertain_not_proven_human_takeover() {
        assert_eq!(
            decide_restore(true, Some(false), Some(true), Some(30), 10, 20, false),
            RestoreDecision::Unobservable
        );
        assert_eq!(
            decide_restore(true, Some(true), Some(true), Some(30), 10, 20, true),
            RestoreDecision::Unobservable
        );
    }

    #[test]
    fn ax_timeout_never_resets_to_platform_default_or_exceeds_budget() {
        use std::time::Duration;
        assert_eq!(bounded_ax_timeout(Duration::ZERO), None);
        assert_eq!(bounded_ax_timeout(Duration::from_micros(500)), None);
        assert_eq!(bounded_ax_timeout(Duration::from_millis(1)), Some(0.001));
        assert_eq!(bounded_ax_timeout(Duration::from_millis(10)), Some(0.010));
        assert_eq!(bounded_ax_timeout(Duration::from_secs(1)), Some(0.025));
    }

    #[test]
    fn focus_sample_expires_on_slow_ax_or_intervening_process_or_space_change() {
        let before = FocusStamp {
            process: [1; 8],
            space: 7,
        };
        assert!(focus_sample_is_fresh(before, before, true));
        assert!(!focus_sample_is_fresh(before, before, false));
        assert!(!focus_sample_is_fresh(
            before,
            FocusStamp {
                process: [2; 8],
                ..before
            },
            true
        ));
        assert!(!focus_sample_is_fresh(
            before,
            FocusStamp { space: 8, ..before },
            true
        ));
    }

    #[test]
    fn unknown_windowserver_state_cannot_authorize_a_restore() {
        assert_eq!(
            decide_restore(true, None, None, None, 10, 20, false),
            RestoreDecision::Unobservable
        );
        // NSWorkspace need not have observed the target activation: a direct
        // WindowServer match plus the lost key-window state still repairs it.
        assert_eq!(
            decide_restore(true, Some(true), Some(false), None, 10, 20, false),
            RestoreDecision::Restore
        );
    }
}
