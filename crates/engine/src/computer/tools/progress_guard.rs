use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use sha2::{Digest, Sha256};
use zeron_proto::computer::ComputerAuditEffect;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerProgressScope {
    pub thread_id: String,
    pub turn_id: String,
    pub session_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerProgressAction {
    pub scope: ComputerProgressScope,
    pub target_key: String,
    pub action_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerProgressOutcome {
    pub effect: ComputerAuditEffect,
    pub code: Option<String>,
}

impl ComputerProgressOutcome {
    pub fn new(effect: ComputerAuditEffect, code: Option<&str>) -> Self {
        Self {
            effect,
            code: code.map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerProgressBlockCode {
    RepeatedComputerRefusal,
    RepeatedUnverifiedAction,
}

impl ComputerProgressBlockCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RepeatedComputerRefusal => "repeated_computer_refusal",
            Self::RepeatedUnverifiedAction => "repeated_unverified_action",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerProgressBlock {
    pub code: ComputerProgressBlockCode,
    pub message: String,
    pub effect: &'static str,
    pub retryable: bool,
    pub previous_input_may_have_taken_effect: bool,
}

#[derive(Debug, Clone)]
struct RefusedAction {
    action: String,
    reason: String,
}

#[derive(Debug, Clone, Default)]
struct TargetHistory {
    uncertain_actions: Vec<String>,
    refusals: Vec<RefusedAction>,
}

const HISTORY_LIMIT: usize = 12;
const REFUSAL_LIMIT: usize = 3;
const CYCLE_REPETITIONS: usize = 3;
const MAXIMUM_CYCLE_LENGTH: usize = 4;
const TRANSIENT_ADMISSION_CODES: &[&str] = &[
    "computer_controlled_by_other_thread",
    "computer_busy",
    "native_input_busy",
    "computer_input_paused",
    "desktop_input_paused",
    "input_admission_closed",
    "input_monitor_unavailable",
    "computer_control_revoked",
    "computer_stopped",
    "computer_permission_required",
    "computer_setup_required",
    "browser_requires_setup",
    "approval_denied",
    "approval_unavailable",
    "approval_queue_full",
    "target_not_on_active_space",
    "auth_sheet_focused",
];

fn digest(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}

fn scope_key(scope: &ComputerProgressScope) -> String {
    digest(
        &serde_json::to_string(&(&scope.thread_id, &scope.turn_id, &scope.session_key))
            .unwrap_or_default(),
    )
}

fn append_bounded<T>(entries: &mut Vec<T>, value: T) {
    entries.push(value);
    if entries.len() > HISTORY_LIMIT {
        entries.remove(0);
    }
}

fn repeats_uncertain_cycle(actions: &[String], next_action: &str) -> bool {
    let len = actions.len();
    if len >= 2 && actions[len - 1] == next_action && actions[len - 2] == next_action {
        return true;
    }
    for period in 2..=MAXIMUM_CYCLE_LENGTH {
        let length = period * CYCLE_REPETITIONS;
        let Some(start) = len.checked_sub(length) else {
            continue;
        };
        if actions[start] != next_action {
            continue;
        }
        if (start + period..len).all(|index| actions[index] == actions[index - period]) {
            return true;
        }
    }
    false
}

fn repeats_refusal(refusals: &[RefusedAction], action: &str) -> bool {
    let count = |reason: &str| refusals.iter().filter(|r| r.reason == reason).count();
    refusals
        .iter()
        .any(|refusal| refusal.action == action && count(&refusal.reason) >= REFUSAL_LIMIT)
}

fn touch<'a, T: Default>(entries: &'a mut Vec<(String, T)>, key: &str, limit: usize) -> &'a mut T {
    let value = entries
        .iter()
        .position(|(k, _)| k == key)
        .map(|index| entries.remove(index).1)
        .unwrap_or_default();
    entries.push((key.to_owned(), value));
    while entries.len() > limit {
        entries.remove(0);
    }
    &mut entries.last_mut().expect("just pushed").1
}

#[derive(Debug)]
pub struct ComputerProgressGuard {
    scopes: Vec<(String, Vec<(String, TargetHistory)>)>,
    maximum_scopes: usize,
    maximum_targets_per_scope: usize,
}

impl Default for ComputerProgressGuard {
    fn default() -> Self {
        Self::new(256, 8).expect("default limits are positive")
    }
}

impl ComputerProgressGuard {
    pub fn new(maximum_scopes: usize, maximum_targets_per_scope: usize) -> Result<Self, String> {
        if maximum_scopes < 1 || maximum_targets_per_scope < 1 {
            return Err("Computer progress history limits must be positive integers.".into());
        }
        Ok(Self {
            scopes: Vec::new(),
            maximum_scopes,
            maximum_targets_per_scope,
        })
    }

    pub fn check(&self, action: &ComputerProgressAction) -> Option<ComputerProgressBlock> {
        let scope = scope_key(&action.scope);
        let target = digest(&action.target_key);
        let history = self
            .scopes
            .iter()
            .find(|(key, _)| *key == scope)?
            .1
            .iter()
            .find(|(key, _)| *key == target)
            .map(|(_, history)| history)?;
        let key = digest(&action.action_key);
        let uncertain = repeats_uncertain_cycle(&history.uncertain_actions, &key);
        if !uncertain && !repeats_refusal(&history.refusals, &key) {
            return None;
        }
        let previous_input_may_have_taken_effect = !history.uncertain_actions.is_empty();
        let mut message = String::from(if uncertain {
            "A repeated action sequence has no verified effect. This call was not sent. "
        } else {
            "This target repeatedly refused the same approaches for the same reason. This call was not sent. "
        });
        message.push_str(if previous_input_may_have_taken_effect {
            "Earlier input may have taken effect: do not replay it or switch to foreground to retry it. "
        } else {
            "Do not repeat these refused actions. "
        });
        message.push_str("Inspect existing evidence and correct the target or use a genuinely different approach; otherwise stop and report the blocker. A fresh screenshot alone does not establish progress.");
        Some(ComputerProgressBlock {
            code: if uncertain {
                ComputerProgressBlockCode::RepeatedUnverifiedAction
            } else {
                ComputerProgressBlockCode::RepeatedComputerRefusal
            },
            message,
            effect: "not-dispatched",
            retryable: false,
            previous_input_may_have_taken_effect,
        })
    }

    pub fn record(&mut self, action: &ComputerProgressAction, outcome: &ComputerProgressOutcome) {
        if outcome.effect == ComputerAuditEffect::Verified {
            self.record_verified_progress(&action.scope, &action.target_key);
            return;
        }
        let refused = matches!(
            outcome.effect,
            ComputerAuditEffect::Refused | ComputerAuditEffect::NotDispatched
        );
        if refused
            && outcome
                .code
                .as_deref()
                .is_none_or(|code| code.is_empty() || TRANSIENT_ADMISSION_CODES.contains(&code))
        {
            return;
        }
        let scope = scope_key(&action.scope);
        let target = digest(&action.target_key);
        let action_key = digest(&action.action_key);
        let targets_limit = self.maximum_targets_per_scope;
        let targets = touch(&mut self.scopes, &scope, self.maximum_scopes);
        let history = touch(targets, &target, targets_limit);
        if refused {
            let reason = digest(outcome.code.as_deref().unwrap_or_default());
            append_bounded(
                &mut history.refusals,
                RefusedAction {
                    action: action_key,
                    reason,
                },
            );
        } else {
            append_bounded(&mut history.uncertain_actions, action_key);
        }
    }

    pub fn record_verified_progress(&mut self, scope: &ComputerProgressScope, target_key: &str) {
        let scope = scope_key(scope);
        let target = digest(target_key);
        let Some(index) = self.scopes.iter().position(|(key, _)| *key == scope) else {
            return;
        };
        let targets = &mut self.scopes[index].1;
        targets.retain(|(key, _)| *key != target);
        if targets.is_empty() {
            self.scopes.remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ComputerAuditEffect as Effect;

    fn action_on(action_key: &str, target_key: &str) -> ComputerProgressAction {
        ComputerProgressAction {
            scope: ComputerProgressScope {
                thread_id: "thread-1".into(),
                turn_id: "turn-1".into(),
                session_key: Some("session-1".into()),
            },
            target_key: target_key.into(),
            action_key: action_key.into(),
        }
    }

    fn action(action_key: &str) -> ComputerProgressAction {
        action_on(action_key, "window-1:field-1")
    }

    fn uncertain() -> ComputerProgressOutcome {
        ComputerProgressOutcome::new(Effect::DispatchedUnknown, None)
    }

    fn refusal() -> ComputerProgressOutcome {
        ComputerProgressOutcome::new(Effect::NotDispatched, Some("same_pid_keyboard_ambiguity"))
    }

    fn code(
        guard: &ComputerProgressGuard,
        action: &ComputerProgressAction,
    ) -> Option<&'static str> {
        guard.check(action).map(|block| block.code.as_str())
    }

    #[test]
    fn blocks_a_third_identical_uncertain_mutation_before_it_can_be_sent() {
        let mut guard = ComputerProgressGuard::default();
        let enter = action("press-key:enter");
        assert!(guard.check(&enter).is_none());
        guard.record(&enter, &uncertain());
        assert!(guard.check(&enter).is_none());
        guard.record(&enter, &uncertain());
        let block = guard.check(&enter).unwrap();
        assert_eq!(
            block.code,
            ComputerProgressBlockCode::RepeatedUnverifiedAction
        );
        assert_eq!(block.effect, "not-dispatched");
        assert!(!block.retryable);
        assert!(block.previous_input_may_have_taken_effect);
    }

    #[test]
    fn blocks_alternating_clicks_and_keys_after_three_same_reason_target_refusals() {
        let mut guard = ComputerProgressGuard::default();
        let click = action("click");
        let enter = action("press-key:enter");
        guard.record(&click, &refusal());
        guard.record(&enter, &refusal());
        assert!(guard.check(&click).is_none());
        guard.record(&click, &refusal());
        for next in [&click, &enter] {
            let block = guard.check(next).unwrap();
            assert_eq!(
                block.code,
                ComputerProgressBlockCode::RepeatedComputerRefusal
            );
            assert!(!block.previous_input_may_have_taken_effect);
            assert!(!block.retryable);
        }
        assert!(guard.check(&action("select-text:exact-field")).is_none());
        assert!(
            guard
                .check(&action_on("click", "window-1:corrected-field"))
                .is_none()
        );
    }

    #[test]
    fn counts_persistent_server_target_refusals_as_well_as_native_pre_dispatch_refusals() {
        let mut guard = ComputerProgressGuard::default();
        let click = action("click");
        for _ in 0..3 {
            guard.record(
                &click,
                &ComputerProgressOutcome::new(Effect::Refused, Some("computer_target_ambiguous")),
            );
        }
        assert_eq!(code(&guard, &click), Some("repeated_computer_refusal"));
    }

    #[test]
    fn permits_admission_recovery_after_repeated_transient_refusals() {
        for transient in TRANSIENT_ADMISSION_CODES {
            let mut guard = ComputerProgressGuard::default();
            let enter = action("press-key:enter");
            for attempt in 0..4 {
                let effect = if attempt % 2 == 0 {
                    Effect::Refused
                } else {
                    Effect::NotDispatched
                };
                guard.record(
                    &enter,
                    &ComputerProgressOutcome::new(effect, Some(transient)),
                );
                assert!(guard.check(&enter).is_none(), "{transient}");
            }
            guard.record(&enter, &uncertain());
            assert!(guard.check(&enter).is_none(), "{transient}");
        }
        assert_eq!(TRANSIENT_ADMISSION_CODES.len(), 17);
    }

    #[test]
    fn does_not_erase_uncertainty_or_trust_a_transient_looking_code_after_dispatch() {
        let mut guard = ComputerProgressGuard::default();
        let enter = action("press-key:enter");
        guard.record(&enter, &uncertain());
        for _ in 0..4 {
            guard.record(
                &enter,
                &ComputerProgressOutcome::new(Effect::Refused, Some("computer_input_paused")),
            );
        }
        guard.record(
            &enter,
            &ComputerProgressOutcome::new(Effect::DispatchedUnknown, Some("computer_input_paused")),
        );
        assert_eq!(code(&guard, &enter), Some("repeated_unverified_action"));
    }

    #[test]
    fn does_not_conflate_distinct_refusal_reasons_or_unrelated_targets() {
        let mut guard = ComputerProgressGuard::default();
        let enter = action("press-key:enter");
        guard.record(&enter, &refusal());
        guard.record(
            &enter,
            &ComputerProgressOutcome::new(Effect::Refused, Some("stale_target")),
        );
        guard.record(
            &action_on("press-key:enter", "window-2:field-1"),
            &refusal(),
        );
        assert!(guard.check(&enter).is_none());
    }

    #[test]
    fn detects_a_repeated_uncertain_cycle_of_every_supported_period() {
        for cycle in [
            vec!["click", "press-key:enter"],
            vec!["click", "press-key:enter", "press-key:tab"],
            vec!["click", "press-key:enter", "press-key:tab", "type-text"],
        ] {
            let mut guard = ComputerProgressGuard::default();
            for _ in 0..3 {
                for key in &cycle {
                    let next = action(key);
                    assert!(guard.check(&next).is_none());
                    guard.record(&next, &uncertain());
                }
            }
            assert_eq!(
                code(&guard, &action(cycle[0])),
                Some("repeated_unverified_action")
            );
            assert!(
                guard
                    .check(&action("different-semantic-approach"))
                    .is_none()
            );
        }
    }

    #[test]
    fn allows_a_normal_calculator_sequence_and_long_sequences_of_different_actions() {
        let mut guard = ComputerProgressGuard::default();
        for key in ["clear", "1", "2", "3", "multiply", "4", "5", "equals"] {
            let next = action(&format!("calculator:{key}"));
            assert!(guard.check(&next).is_none());
            guard.record(&next, &uncertain());
        }
        for index in 0..100 {
            let next = action(&format!("different-control:{index}"));
            assert!(guard.check(&next).is_none());
            guard.record(&next, &uncertain());
        }
    }

    #[test]
    fn does_not_let_reads_no_ops_or_intervening_refusals_erase_earlier_uncertainty() {
        let mut guard = ComputerProgressGuard::default();
        let enter = action("press-key:enter");
        guard.record(&enter, &uncertain());
        assert!(guard.check(&action("observation:screenshot-2")).is_none());
        guard.record(
            &action("click"),
            &ComputerProgressOutcome::new(Effect::NotDispatched, None),
        );
        guard.record(&action("click"), &refusal());
        guard.record(&enter, &uncertain());
        assert_eq!(code(&guard, &enter), Some("repeated_unverified_action"));
    }

    #[test]
    fn treats_errors_without_dispatch_proof_as_potentially_delivered_input() {
        let mut guard = ComputerProgressGuard::default();
        let enter = action("press-key:enter");
        guard.record(
            &enter,
            &ComputerProgressOutcome::new(Effect::Error, Some("computer_backend_error")),
        );
        guard.record(&enter, &uncertain());
        assert!(
            guard
                .check(&enter)
                .unwrap()
                .previous_input_may_have_taken_effect
        );
    }

    #[test]
    fn only_a_verified_effect_clears_both_refusal_and_uncertain_history_for_that_target() {
        let mut guard = ComputerProgressGuard::default();
        let enter = action("press-key:enter");
        guard.record(&enter, &uncertain());
        guard.record(&enter, &uncertain());
        for _ in 0..3 {
            guard.record(&enter, &refusal());
        }
        assert!(guard.check(&enter).is_some());
        guard.record(
            &action("verified-correction"),
            &ComputerProgressOutcome::new(Effect::Verified, None),
        );
        assert!(guard.check(&enter).is_none());
        guard.record(&enter, &uncertain());
        assert!(guard.check(&enter).is_none());
    }

    #[test]
    fn explicit_verification_clears_its_target_without_clearing_unrelated_uncertainty() {
        let mut guard = ComputerProgressGuard::default();
        let first = action("press-key:enter");
        let second = action_on("click", "window-2:field-1");
        for next in [&first, &second] {
            guard.record(next, &uncertain());
            guard.record(next, &uncertain());
        }
        guard.record_verified_progress(&first.scope, &first.target_key);
        assert!(guard.check(&first).is_none());
        assert_eq!(code(&guard, &second), Some("repeated_unverified_action"));
    }

    #[test]
    fn isolates_new_turns_threads_and_provider_sessions() {
        let mut guard = ComputerProgressGuard::default();
        let enter = action("press-key:enter");
        guard.record(&enter, &uncertain());
        guard.record(&enter, &uncertain());
        let mut turn = enter.clone();
        turn.scope.turn_id = "turn-2".into();
        let mut thread = enter.clone();
        thread.scope.thread_id = "thread-2".into();
        let mut session = enter.clone();
        session.scope.session_key = Some("session-2".into());
        for other in [turn, thread, session] {
            assert!(guard.check(&other).is_none());
        }
        assert!(guard.check(&enter).is_some());
    }

    #[test]
    fn keeps_unknown_effect_warnings_even_when_repeated_refusals_trigger_the_block() {
        let mut guard = ComputerProgressGuard::default();
        let enter = action("press-key:enter");
        guard.record(&action("earlier-click"), &uncertain());
        for _ in 0..3 {
            guard.record(&enter, &refusal());
        }
        let blocked = guard.check(&enter).unwrap();
        assert_eq!(
            blocked.code,
            ComputerProgressBlockCode::RepeatedComputerRefusal
        );
        assert!(blocked.previous_input_may_have_taken_effect);
        assert!(
            blocked
                .message
                .contains("do not replay it or switch to foreground")
        );
        assert!(
            blocked
                .message
                .contains("A fresh screenshot alone does not establish progress")
        );
    }

    #[test]
    fn evicts_old_scopes_and_targets_while_preserving_recently_active_histories() {
        let mut guard = ComputerProgressGuard::new(2, 2).unwrap();
        let first = action_on("click", "target-1");
        let second = action_on("click", "target-2");
        let third = action_on("click", "target-3");
        for next in [&first, &second, &first, &third, &third] {
            guard.record(next, &uncertain());
        }
        assert!(guard.check(&first).is_some());
        assert!(guard.check(&second).is_none());
        assert!(guard.check(&third).is_some());

        let mut other_scope = first.clone();
        other_scope.scope.turn_id = "turn-2".into();
        guard.record(&other_scope, &uncertain());
        guard.record(&first, &uncertain());
        let mut third_scope = first.clone();
        third_scope.scope.turn_id = "turn-3".into();
        guard.record(&third_scope, &uncertain());
        assert!(guard.check(&first).is_some());
        assert!(guard.check(&other_scope).is_none());
    }

    #[test]
    fn rejects_invalid_retention_limits() {
        assert!(ComputerProgressGuard::new(0, 1).is_err());
        assert!(ComputerProgressGuard::new(1, 0).is_err());
    }
}
