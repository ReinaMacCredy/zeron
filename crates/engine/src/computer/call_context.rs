use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use zeron_proto::computer::{ComputerDeliveryVerification, CuaEffect};

pub const ZERON_CUA_TIMING_LOG: &str = "ZERON_CUA_TIMING_LOG";
pub const ZERON_CUA_CONDITIONAL_SETTLE: &str = "ZERON_CUA_CONDITIONAL_SETTLE";
pub const ZERON_CUA_ACTION_SETTLE_MS: &str = "ZERON_CUA_ACTION_SETTLE_MS";
pub const ZERON_CUA_CAPTURE_REUSE: &str = "ZERON_CUA_CAPTURE_REUSE";
pub const ZERON_CUA_PREVIEW_STILL_MS: &str = "ZERON_CUA_PREVIEW_STILL_MS";

pub(crate) fn env_flag_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_lowercase().as_str(),
            "1" | "true" | "on" | "yes"
        )
    })
}

fn env_flag_disabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        )
    })
}

fn env_number(value: Option<&str>) -> Option<f64> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    raw.parse::<f64>().ok().filter(|parsed| parsed.is_finite())
}

pub fn cua_timing_log_enabled(value: Option<&str>) -> bool {
    env_flag_enabled(value)
}

pub fn cua_conditional_settle_enabled(value: Option<&str>) -> bool {
    !env_flag_disabled(value)
}

pub fn cua_action_settle_ms_override(value: Option<&str>) -> Option<f64> {
    env_number(value).filter(|parsed| *parsed >= 0.0)
}

pub fn cua_capture_reuse_enabled(value: Option<&str>) -> bool {
    !env_flag_disabled(value)
}

pub fn cua_preview_still_ms_override(value: Option<&str>) -> Option<f64> {
    env_number(value).filter(|parsed| *parsed > 0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComputerActionProof {
    pub effect: Option<CuaEffect>,
    pub verified: Option<ComputerDeliveryVerification>,
}

#[derive(Default)]
struct TimingRecord {
    operation: Option<String>,
    legs: BTreeMap<String, f64>,
    counts: BTreeMap<String, u64>,
    failed: bool,
    finished: bool,
}

pub struct ComputerCallTiming {
    now: Arc<dyn Fn() -> f64 + Send + Sync>,
    started_at: f64,
    record: Mutex<TimingRecord>,
}

impl ComputerCallTiming {
    pub fn new(now: Arc<dyn Fn() -> f64 + Send + Sync>) -> Self {
        let started_at = now();
        Self {
            now,
            started_at,
            record: Mutex::new(TimingRecord::default()),
        }
    }

    fn record(&self) -> std::sync::MutexGuard<'_, TimingRecord> {
        self.record.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_operation(&self, operation: &str) {
        self.record()
            .operation
            .get_or_insert_with(|| operation.to_owned());
    }

    pub fn mark_failed(&self) {
        self.record().failed = true;
    }

    pub fn record_leg(&self, leg: &str, ms: f64) {
        *self.record().legs.entry(leg.to_owned()).or_insert(0.0) += ms;
    }

    pub fn count(&self, name: &str, by: u64) {
        *self.record().counts.entry(name.to_owned()).or_insert(0) += by;
    }

    pub async fn span<F: Future>(&self, leg: &str, run: F) -> F::Output {
        let started = (self.now)();
        let output = run.await;
        self.record_leg(leg, (self.now)() - started);
        output
    }

    pub fn finish(&self) -> Option<String> {
        let mut record = self.record();
        if record.finished {
            return None;
        }
        record.finished = true;
        let mut parts = vec![format!(
            "op={}",
            record.operation.as_deref().unwrap_or("computer_call")
        )];
        parts.extend(
            record
                .legs
                .iter()
                .map(|(leg, ms)| format!("{leg}_ms={ms:.1}")),
        );
        parts.extend(
            record
                .counts
                .iter()
                .map(|(name, count)| format!("{name}={count}")),
        );
        parts.push(format!("total_ms={:.1}", (self.now)() - self.started_at));
        if record.failed {
            parts.push("failed=1".to_owned());
        }
        let line = format!("[computer-timing] {}", parts.join(" "));
        tracing::info!("{line}");
        Some(line)
    }
}

pub struct ComputerCallContext {
    pub timing: Option<ComputerCallTiming>,
    proof: Mutex<Option<ComputerActionProof>>,
}

impl ComputerCallContext {
    pub fn new(timing: Option<ComputerCallTiming>) -> Self {
        Self {
            timing,
            proof: Mutex::new(None),
        }
    }

    pub fn record_action_proof(&self, proof: ComputerActionProof) {
        *self.proof.lock().unwrap_or_else(PoisonError::into_inner) = Some(proof);
    }

    pub fn take_action_proof(&self) -> Option<ComputerActionProof> {
        self.proof
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

tokio::task_local! {
    static COMPUTER_CALL: Arc<ComputerCallContext>;
}

pub fn current_computer_call() -> Option<Arc<ComputerCallContext>> {
    COMPUTER_CALL.try_with(Arc::clone).ok()
}

pub async fn with_computer_call_context<F: Future>(
    context: Arc<ComputerCallContext>,
    run: F,
) -> F::Output {
    COMPUTER_CALL.scope(context, run).await
}

pub fn create_computer_call_context(
    timing_log: bool,
    conditional_settle: bool,
    now: Arc<dyn Fn() -> f64 + Send + Sync>,
) -> Option<Arc<ComputerCallContext>> {
    let timing = timing_log.then(|| ComputerCallTiming::new(now));
    if timing.is_none() && !conditional_settle {
        return None;
    }
    Some(Arc::new(ComputerCallContext::new(timing)))
}

pub async fn timed_computer_leg<F: Future>(leg: &str, run: F) -> F::Output {
    match current_computer_call() {
        Some(context) if context.timing.is_some() => {
            context
                .timing
                .as_ref()
                .expect("timing checked")
                .span(leg, run)
                .await
        }
        _ => run.await,
    }
}

pub fn mark_computer_call(operation: &str) {
    if let Some(timing) = current_computer_call()
        .as_ref()
        .and_then(|context| context.timing.as_ref())
    {
        timing.set_operation(operation);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn clock() -> Arc<dyn Fn() -> f64 + Send + Sync> {
        let ticks = Arc::new(AtomicU64::new(0));
        Arc::new(move || ticks.fetch_add(5, Ordering::SeqCst) as f64)
    }

    #[test]
    fn defaults_every_flag_to_its_shipped_state() {
        assert!(!cua_timing_log_enabled(None));
        assert!(cua_conditional_settle_enabled(None));
        assert!(cua_capture_reuse_enabled(None));
        assert_eq!(cua_action_settle_ms_override(None), None);
        assert_eq!(cua_preview_still_ms_override(None), None);
    }

    #[test]
    fn treats_affirmative_spellings_as_enabled_for_the_boolean_flags() {
        for value in ["1", "true", "on", "yes", " TRUE ", "On"] {
            assert!(cua_timing_log_enabled(Some(value)), "{value}");
            assert!(cua_conditional_settle_enabled(Some(value)), "{value}");
            assert!(cua_capture_reuse_enabled(Some(value)), "{value}");
        }
    }

    #[test]
    fn treats_off_spellings_as_the_graduated_flags_kill_switch() {
        for value in ["0", "false", "off", "no"] {
            assert!(!cua_conditional_settle_enabled(Some(value)), "{value}");
            assert!(!cua_capture_reuse_enabled(Some(value)), "{value}");
        }
    }

    #[test]
    fn treats_anything_else_as_disabled_for_the_opt_in_flag() {
        for value in ["0", "false", "off", "no", "2", "enabled"] {
            assert!(!cua_timing_log_enabled(Some(value)), "{value}");
        }
    }

    #[test]
    fn keeps_the_graduated_flags_on_for_a_non_off_value() {
        for value in ["2", "enabled", "anything"] {
            assert!(cua_conditional_settle_enabled(Some(value)), "{value}");
            assert!(cua_capture_reuse_enabled(Some(value)), "{value}");
        }
    }

    #[test]
    fn parses_the_action_settle_override_as_a_non_negative_number() {
        assert_eq!(cua_action_settle_ms_override(Some("0")), Some(0.0));
        assert_eq!(cua_action_settle_ms_override(Some("175")), Some(175.0));
        assert_eq!(cua_action_settle_ms_override(Some(" 80 ")), Some(80.0));
        for value in ["", "   ", "abc", "-5", "NaN"] {
            assert_eq!(cua_action_settle_ms_override(Some(value)), None, "{value}");
        }
    }

    #[test]
    fn parses_the_preview_still_override_as_a_positive_number() {
        assert_eq!(cua_preview_still_ms_override(Some("4000")), Some(4000.0));
        assert_eq!(cua_preview_still_ms_override(Some(" 250 ")), Some(250.0));
        for value in ["", "abc", "0", "-100", "NaN"] {
            assert_eq!(cua_preview_still_ms_override(Some(value)), None, "{value}");
        }
    }

    #[test]
    fn creates_no_context_at_all_when_both_consumers_are_off() {
        assert!(create_computer_call_context(false, false, clock()).is_none());
    }

    #[test]
    fn creates_a_context_with_a_timing_record_under_the_timing_log() {
        assert!(
            create_computer_call_context(true, false, clock())
                .unwrap()
                .timing
                .is_some()
        );
    }

    #[test]
    fn creates_a_context_without_a_timing_record_under_conditional_settle_alone() {
        let context = create_computer_call_context(false, true, clock()).unwrap();
        assert!(context.timing.is_none());
    }

    #[test]
    fn hands_the_action_proof_to_the_post_action_observer_exactly_once() {
        let context = create_computer_call_context(false, true, clock()).unwrap();
        let proof = ComputerActionProof {
            effect: Some(CuaEffect::Verified),
            verified: Some(ComputerDeliveryVerification::Confirmed),
        };
        context.record_action_proof(proof);
        assert_eq!(context.take_action_proof(), Some(proof));
        assert_eq!(context.take_action_proof(), None);
    }

    #[test]
    fn keeps_only_the_latest_actions_verdict() {
        let context = create_computer_call_context(false, true, clock()).unwrap();
        context.record_action_proof(ComputerActionProof {
            effect: Some(CuaEffect::Verified),
            verified: None,
        });
        context.record_action_proof(ComputerActionProof {
            effect: Some(CuaEffect::DispatchedUnknown),
            verified: None,
        });
        assert_eq!(
            context.take_action_proof(),
            Some(ComputerActionProof {
                effect: Some(CuaEffect::DispatchedUnknown),
                verified: None,
            })
        );
    }

    #[tokio::test]
    async fn shares_one_context_across_a_whole_wrapped_call_and_finishes_it_once() {
        let context = create_computer_call_context(true, true, clock()).unwrap();
        let lines = with_computer_call_context(context.clone(), async {
            let inside = current_computer_call().unwrap();
            assert!(Arc::ptr_eq(&inside, &context));
            mark_computer_call("computer_click");
            timed_computer_leg("dispatch", async {}).await;
            timed_computer_leg("settle", async {}).await;
            let timing = context.timing.as_ref().unwrap();
            timing.count("settle_skipped", 1);
            [timing.finish(), timing.finish()]
        })
        .await;
        assert!(current_computer_call().is_none());
        let [first, second] = lines;
        assert!(second.is_none());
        let line = first.unwrap();
        assert!(line.contains("[computer-timing]"));
        assert!(line.contains("op=computer_click"));
        assert!(line.contains("dispatch_ms="));
        assert!(line.contains("settle_ms="));
        assert!(line.contains("settle_skipped=1"));
        assert!(line.contains("total_ms="));
        assert!(!line.contains("failed=1"));
    }

    #[tokio::test]
    async fn timed_leg_is_a_passthrough_outside_a_call_context() {
        assert!(current_computer_call().is_none());
        assert_eq!(timed_computer_leg("dispatch", async { 7 }).await, 7);
    }

    #[tokio::test]
    async fn marks_failures_on_the_calls_timing_line() {
        let context = create_computer_call_context(true, true, clock()).unwrap();
        let (line, result): (Option<String>, Result<(), &str>) =
            with_computer_call_context(context.clone(), async {
                let timing = context.timing.as_ref().unwrap();
                timing.mark_failed();
                (timing.finish(), Err("boom"))
            })
            .await;
        assert_eq!(result, Err("boom"));
        assert!(line.unwrap().contains("failed=1"));
    }
}
