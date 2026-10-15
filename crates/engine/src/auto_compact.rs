use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, Utc};
use zeron_proto::{CompactPlan, Session, SessionStats, SessionStatus};

use crate::{doc_host::DocHost, now_ms, registry::HarnessRegistry, sessions::SessionsEngine};

// Claude Code's own idle defaults (CLI 2.1.299, `tengu_sunny_locket`): only a
// 1h cache, fire at 90% of its life, at most a minute late.
const CACHE_TTL_SECS: u32 = 3600;
const FIRE_AT_FRACTION: f64 = 0.9;
const LATE_MS: i64 = 60_000;
/// A finished background shell makes the CLI open its own turn seconds after the chat settles.
const AGENT_SETTLE_MS: i64 = 10_000;
/// Monotonic sleeps stop while the Mac sleeps; re-read the wall clock often.
const MAX_NAP: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Armed {
    touched_at: i64,
    fire_at: i64,
}

fn arm(stats: &SessionStats, tokens: u64, min_tokens: u64) -> Option<Armed> {
    if stats.cache_ttl_secs != Some(CACHE_TTL_SECS) || tokens < min_tokens {
        return None;
    }
    let touched_at = stats.cache_touched_at?;
    let life_ms = f64::from(CACHE_TTL_SECS) * 1000.0;
    Some(Armed {
        touched_at,
        fire_at: touched_at + (life_ms * FIRE_AT_FRACTION) as i64,
    })
}

fn settled(session: &Session) -> bool {
    session.status == SessionStatus::Idle
        && !session.compacting
        && !session.held_by_subagents
        && session.running_subagents == 0
        && session.background_tasks.is_empty()
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
) {
    if !running
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(chat_id.to_string())
    {
        return;
    }
    let (sessions, host, running, chat_id) = (
        sessions.clone(),
        host.clone(),
        running.clone(),
        chat_id.to_string(),
    );
    tokio::spawn(async move {
        match sessions.compact_for_agent(&chat_id).await {
            Ok(Some((outcome, continue_with))) => {
                tracing::info!(chat = %chat_id, compacted = outcome.compacted, "agent compaction ran");
                if let Some(text) = continue_with
                    && let Err(err) = host.queue_auto_continue(&chat_id, &text).await
                {
                    tracing::warn!(chat = %chat_id, error = %err, "agent auto-continue not sent");
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
}

pub(crate) fn spawn(sessions: SessionsEngine, registry: Arc<HarnessRegistry>, host: DocHost) {
    let min_tokens_override: Option<u64> = std::env::var("ZERON_IDLE_COMPACT_MIN_TOKENS")
        .ok()
        .and_then(|value| value.parse().ok());
    tokio::spawn(async move {
        let mut statuses = sessions.watch_sessions();
        let mut seen: HashMap<String, DateTime<Utc>> = HashMap::new();
        let mut armed: HashMap<String, Armed> = HashMap::new();
        let mut fired: HashMap<String, i64> = HashMap::new();
        let mut agent_due: HashMap<String, i64> = HashMap::new();
        let running = Arc::new(Mutex::new(HashSet::new()));
        let mut last_settings = registry.compaction_settings();
        loop {
            let settings = registry.compaction_settings();
            if settings != last_settings {
                seen.clear();
                last_settings = settings;
            }
            let min_tokens = min_tokens_override.unwrap_or(settings.min_tokens);
            let idle: Vec<Session> = statuses
                .borrow_and_update()
                .iter()
                .filter(|session| settled(session))
                .cloned()
                .collect();
            seen.retain(|chat, _| idle.iter().any(|s| &s.chat_id == chat));
            agent_due.retain(|chat, _| seen.contains_key(chat));
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
                    agent_due.insert(session.chat_id.clone(), now_ms() + AGENT_SETTLE_MS);
                    continue;
                }
                let next = sessions
                    .cache_state(&session.chat_id)
                    .and_then(|(stats, tokens)| arm(&stats, tokens, min_tokens))
                    .filter(|_| settings.enabled)
                    .filter(|next| fired.get(&session.chat_id) != Some(&next.touched_at));
                match next {
                    Some(next) => {
                        armed.insert(session.chat_id.clone(), next);
                        sessions.set_compact_plan(
                            &session.chat_id,
                            Some(CompactPlan::Idle { at: next.fire_at }),
                        );
                    }
                    None => {
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
                agent_due.remove(&chat_id);
                run_agent_compaction(&sessions, &host, &running, &chat_id);
            }
            let due: Vec<(String, Armed)> = armed
                .iter()
                .filter(|(_, at)| at.fire_at <= now)
                .map(|(chat, at)| (chat.clone(), *at))
                .collect();
            for (chat_id, at) in due {
                armed.remove(&chat_id);
                fired.insert(chat_id.clone(), at.touched_at);
                let still = sessions
                    .cache_state(&chat_id)
                    .and_then(|(stats, tokens)| arm(&stats, tokens, min_tokens));
                if !registry.compaction_settings().enabled
                    || now - at.fire_at > LATE_MS
                    || still != Some(at)
                {
                    clear_idle_plan(&sessions, &chat_id);
                    continue;
                }
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
                .min(MAX_NAP);
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
        assert_eq!(
            arm(&stats(Some(3600), Some(1_000)), 250_000, MIN_TOKENS),
            Some(Armed {
                touched_at: 1_000,
                fire_at: 1_000 + 3_240_000,
            })
        );
    }

    #[test]
    fn skips_short_caches_small_contexts_and_untouched_caches() {
        assert_eq!(
            arm(&stats(Some(300), Some(1_000)), 250_000, MIN_TOKENS),
            None
        );
        assert_eq!(arm(&stats(None, Some(1_000)), 250_000, MIN_TOKENS), None);
        assert_eq!(
            arm(&stats(Some(3600), Some(1_000)), 199_999, MIN_TOKENS),
            None
        );
        assert_eq!(arm(&stats(Some(3600), None), 250_000, MIN_TOKENS), None);
    }
}
