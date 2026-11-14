use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, Utc};
use zeron_proto::{CompactHold, CompactPlan, Session, SessionStats, SessionStatus};

use crate::{
    doc_host::DocHost,
    now_ms,
    registry::HarnessRegistry,
    sessions::{AgentCompactRun, SessionsEngine},
};

// Claude Code's own idle defaults (CLI 2.1.299, `tengu_sunny_locket`): only a
// 1h cache, fire at 90% of its life.
const CACHE_TTL_SECS: u32 = 3600;
const FIRE_AT_FRACTION: f64 = 0.9;
/// The compaction's own request has to read the cache before it expires.
const EXPIRY_MARGIN_MS: i64 = 60_000;
/// Harnesses that restart to compact: a finished background shell makes the
/// CLI open its own turn seconds after the chat settles.
const AGENT_SETTLE_MS: i64 = 10_000;
/// An agent compaction that finds the chat's slot taken retries; the row may not change again.
const AGENT_RETRY_MS: i64 = 1_000;
/// Monotonic sleeps stop while the Mac sleeps; re-read the wall clock often.
const MAX_NAP: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Armed {
    touched_at: i64,
    fire_at: i64,
}

impl Armed {
    fn expired(self, now: i64) -> bool {
        now > self.touched_at + i64::from(CACHE_TTL_SECS) * 1000 - EXPIRY_MARGIN_MS
    }
}

fn arm(stats: &SessionStats, tokens: u64, min_tokens: u64) -> Result<Armed, CompactHold> {
    match stats.cache_ttl_secs {
        None => return Err(CompactHold::NoCache),
        Some(ttl) if ttl != CACHE_TTL_SECS => return Err(CompactHold::ShortCache),
        Some(_) => {}
    }
    if tokens < min_tokens {
        return Err(CompactHold::Small { min: min_tokens });
    }
    let touched_at = stats.cache_touched_at.ok_or(CompactHold::NoCache)?;
    let life_ms = f64::from(CACHE_TTL_SECS) * 1000.0;
    Ok(Armed {
        touched_at,
        fire_at: touched_at + (life_ms * FIRE_AT_FRACTION) as i64,
    })
}

/// A harness that compacts inside its live run leaves background work running.
fn settled(session: &Session, in_place: impl FnOnce() -> bool) -> bool {
    if session.compacting {
        return false;
    }
    let idle = session.status == SessionStatus::Idle;
    let quiet = !session.held_by_subagents
        && session.running_subagents == 0
        && session.background_tasks.is_empty();
    (idle && quiet) || ((idle || session.held_by_subagents) && in_place())
}

/// The agent's turn is over: idle, or parked while background sub-agents run.
fn turn_over(session: &Session) -> bool {
    !session.compacting && (session.status == SessionStatus::Idle || session.held_by_subagents)
}

fn clear_idle_plan(sessions: &SessionsEngine, chat_id: &str) {
    if !sessions.agent_compact_waiting(chat_id) {
        sessions.set_compact_plan(chat_id, None);
    }
}

fn run_agent_compaction(
    sessions: &SessionsEngine,
    host: &DocHost,
    running: &Arc<Mutex<HashSet<String>>>,
    chat_id: &str,
) -> bool {
    if !running
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(chat_id.to_string())
    {
        return false;
    }
    let (sessions, host, running, chat_id) = (
        sessions.clone(),
        host.clone(),
        running.clone(),
        chat_id.to_string(),
    );
    tokio::spawn(async move {
        match sessions.compact_for_agent(&chat_id).await {
            Ok(Some((run, request))) => {
                let reason = match run {
                    AgentCompactRun::Compacted(outcome) => {
                        tracing::info!(chat = %chat_id, compacted = outcome.compacted, "agent compaction ran");
                        None
                    }
                    AgentCompactRun::Declined(reason) => {
                        tracing::info!(chat = %chat_id, "agent compaction declined");
                        Some(reason)
                    }
                };
                if let Some(text) = request.continue_with {
                    match host
                        .queue_auto_continue(&chat_id, &text, request.after_user.as_deref(), || {
                            sessions.take_unsent_continue(&chat_id, reason)
                        })
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::info!(chat = %chat_id, "agent auto-continue held for the agent's next message")
                        }
                        Err(err) => {
                            tracing::warn!(chat = %chat_id, error = %err, "agent auto-continue not sent")
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(err) => tracing::info!(chat = %chat_id, error = %err, "agent compaction dropped"),
        }
        running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&chat_id);
    });
    true
}

pub(crate) fn spawn(sessions: SessionsEngine, registry: Arc<HarnessRegistry>, host: DocHost) {
    let min_tokens_override: Option<u64> = std::env::var("ZERON_IDLE_COMPACT_MIN_TOKENS")
        .ok()
        .and_then(|value| value.parse().ok());
    let worker = host.clone();
    worker.spawn_worker(async move {
        sessions.surface_waiting_agent_compacts();
        let boot_min = min_tokens_override.unwrap_or(registry.compaction_settings().min_tokens);
        sessions.surface_idle_chats(|stats, tokens| {
            arm(stats, tokens, boot_min).is_ok_and(|at| !at.expired(now_ms()))
        });
        let mut statuses = sessions.watch_sessions();
        let mut seen: HashMap<String, DateTime<Utc>> = HashMap::new();
        let mut armed: HashMap<String, Armed> = HashMap::new();
        let mut fired: HashMap<String, i64> = HashMap::new();
        let mut unarmed: HashMap<String, CompactHold> = HashMap::new();
        let mut agent_due: HashMap<String, i64> = HashMap::new();
        let mut agent_seen: HashMap<String, DateTime<Utc>> = HashMap::new();
        let running = Arc::new(Mutex::new(HashSet::new()));
        let mut last_settings = registry.compaction_settings();
        loop {
            let settings = registry.compaction_settings();
            if settings != last_settings {
                seen.clear();
                last_settings = settings.clone();
            }
            let min_tokens = min_tokens_override.unwrap_or(settings.min_tokens);
            let rows: Vec<Session> = statuses.borrow_and_update().clone();
            let (idle, unsettled): (Vec<Session>, Vec<Session>) = rows
                .into_iter()
                .partition(|session| settled(session, || sessions.compacts_in_place(&session.chat_id)));
            for session in &unsettled {
                let busy = session.status == SessionStatus::Idle && !session.compacting;
                sessions.set_compact_hold(&session.chat_id, busy.then_some(CompactHold::Busy));
            }
            seen.retain(|chat, _| idle.iter().any(|s| &s.chat_id == chat));
            let waiting: Vec<Session> = statuses
                .borrow()
                .iter()
                .filter(|session| {
                    turn_over(session) && sessions.agent_compact_waiting(&session.chat_id)
                })
                .cloned()
                .collect();
            agent_seen.retain(|chat, _| waiting.iter().any(|s| &s.chat_id == chat));
            agent_due.retain(|chat, _| agent_seen.contains_key(chat));
            let mut held = false;
            for session in &waiting {
                if agent_seen.get(&session.chat_id) == Some(&session.updated_at) {
                    continue;
                }
                // Its own compaction holds the slot; the row is read again once that run ends.
                if running
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains(&session.chat_id)
                {
                    held = true;
                    continue;
                }
                agent_seen.insert(session.chat_id.clone(), session.updated_at);
                let settle = if sessions.compacts_in_place(&session.chat_id) {
                    0
                } else {
                    AGENT_SETTLE_MS
                };
                agent_due.insert(session.chat_id.clone(), now_ms() + settle);
            }

            let gone: Vec<String> = armed
                .keys()
                .filter(|chat| !seen.contains_key(*chat))
                .cloned()
                .collect();
            for chat_id in gone {
                armed.remove(&chat_id);
                clear_idle_plan(&sessions, &chat_id);
            }
            for session in &idle {
                if seen.get(&session.chat_id) == Some(&session.updated_at) {
                    continue;
                }
                seen.insert(session.chat_id.clone(), session.updated_at);
                if sessions.agent_compact_waiting(&session.chat_id) {
                    armed.remove(&session.chat_id);
                    sessions.set_compact_hold(&session.chat_id, None);
                    continue;
                }
                let next = if !settings.enabled {
                    Err(CompactHold::Off)
                } else if !sessions.auto_compacts(&session.chat_id) {
                    Err(CompactHold::Mission)
                } else {
                    sessions
                        .cache_state(&session.chat_id)
                        .ok_or(CompactHold::NoCache)
                        .and_then(|(stats, tokens)| arm(&stats, tokens, min_tokens))
                        .and_then(|next| {
                            if fired.get(&session.chat_id) == Some(&next.touched_at) {
                                Err(CompactHold::Ran)
                            } else if next.expired(now_ms()) {
                                Err(CompactHold::Expired)
                            } else {
                                Ok(next)
                            }
                        })
                };
                sessions.set_compact_hold(&session.chat_id, next.err());
                match next {
                    Ok(next) => {
                        unarmed.remove(&session.chat_id);
                        if armed.insert(session.chat_id.clone(), next) != Some(next) {
                            tracing::info!(chat = %session.chat_id, at = next.fire_at, "idle compaction armed");
                        }
                        sessions.set_compact_plan(
                            &session.chat_id,
                            Some(CompactPlan::Idle { at: next.fire_at }),
                        );
                    }
                    Err(why) => {
                        if unarmed.insert(session.chat_id.clone(), why) != Some(why) {
                            tracing::info!(chat = %session.chat_id, why = ?why, "idle compaction not armed");
                        }
                        armed.remove(&session.chat_id);
                        clear_idle_plan(&sessions, &session.chat_id);
                    }
                }
            }

            let now = now_ms();
            let agents: Vec<String> = agent_due
                .iter()
                .filter(|(_, at)| **at <= now)
                .map(|(chat, _)| chat.clone())
                .collect();
            for chat_id in agents {
                if run_agent_compaction(&sessions, &host, &running, &chat_id) {
                    agent_due.remove(&chat_id);
                } else {
                    tracing::info!(chat = %chat_id, "agent compaction waits for the chat's other compaction work");
                    agent_due.insert(chat_id, now + AGENT_RETRY_MS);
                }
            }
            let due: Vec<(String, Armed)> = armed
                .iter()
                .filter(|(_, at)| at.fire_at <= now)
                .map(|(chat, at)| (chat.clone(), *at))
                .collect();
            for (chat_id, at) in due {
                armed.remove(&chat_id);
                let still = sessions
                    .cache_state(&chat_id)
                    .and_then(|(stats, tokens)| arm(&stats, tokens, min_tokens).ok());
                let hold = if !registry.compaction_settings().enabled {
                    Some(CompactHold::Off)
                } else if !sessions.auto_compacts(&chat_id) {
                    Some(CompactHold::Mission)
                } else if at.expired(now) {
                    Some(CompactHold::Expired)
                } else {
                    None
                };
                if hold.is_some() || still != Some(at) {
                    tracing::info!(chat = %chat_id, why = ?hold, "idle compaction skipped");
                    sessions.set_compact_hold(&chat_id, hold);
                    clear_idle_plan(&sessions, &chat_id);
                    continue;
                }
                fired.insert(chat_id.clone(), at.touched_at);
                let sessions = sessions.clone();
                tokio::spawn(async move {
                    match sessions.compact_idle(&chat_id).await {
                        Ok(outcome) => tracing::info!(
                            chat = %chat_id,
                            compacted = outcome.compacted,
                            "idle compaction ran before the prompt cache expired"
                        ),
                        Err(err) => {
                            clear_idle_plan(&sessions, &chat_id);
                            tracing::info!(chat = %chat_id, error = %err, "idle compaction skipped")
                        }
                    }
                });
            }

            let nap = armed
                .values()
                .map(|at| at.fire_at)
                .chain(agent_due.values().copied())
                .map(|at| Duration::from_millis((at - now).max(0) as u64))
                .min()
                .unwrap_or(MAX_NAP)
                .min(if held {
                    Duration::from_millis(AGENT_RETRY_MS as u64)
                } else {
                    MAX_NAP
                });
            tokio::select! {
                changed = statuses.changed() => if changed.is_err() { return },
                _ = tokio::time::sleep(nap) => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN_TOKENS: u64 = 200_000;

    fn stats(ttl: Option<u32>, touched_at: Option<i64>) -> SessionStats {
        SessionStats {
            cache_ttl_secs: ttl,
            cache_touched_at: touched_at,
            ..SessionStats::default()
        }
    }

    #[test]
    fn arms_at_ninety_percent_of_a_one_hour_cache() {
        let at = Armed {
            touched_at: 1_000,
            fire_at: 1_000 + 3_240_000,
        };
        assert_eq!(
            arm(&stats(Some(3600), Some(1_000)), 250_000, MIN_TOKENS),
            Ok(at)
        );
        assert!(!at.expired(1_000 + 3_500_000));
        assert!(at.expired(1_000 + 3_560_000));
    }

    #[test]
    fn background_work_holds_off_only_a_harness_that_restarts_to_compact() {
        let mut session: Session = serde_json::from_value(serde_json::json!({
            "chatId": "c",
            "deviceId": "d",
            "status": "idle",
            "startedAt": null,
            "updatedAt": "2026-01-01T00:00:00Z",
            "runningSubagents": 1,
        }))
        .unwrap();
        assert!(!settled(&session, || false));
        assert!(settled(&session, || true));
        session.compacting = true;
        assert!(!settled(&session, || true));
    }

    #[test]
    fn skips_short_caches_small_contexts_and_untouched_caches() {
        assert_eq!(
            arm(&stats(Some(300), Some(1_000)), 250_000, MIN_TOKENS),
            Err(CompactHold::ShortCache)
        );
        assert_eq!(
            arm(&stats(None, Some(1_000)), 250_000, MIN_TOKENS),
            Err(CompactHold::NoCache)
        );
        assert_eq!(
            arm(&stats(Some(3600), Some(1_000)), 199_999, MIN_TOKENS),
            Err(CompactHold::Small { min: MIN_TOKENS })
        );
        assert_eq!(
            arm(&stats(Some(3600), None), 250_000, MIN_TOKENS),
            Err(CompactHold::NoCache)
        );
    }
}
