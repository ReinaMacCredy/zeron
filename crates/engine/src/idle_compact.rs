use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use zeron_proto::{Session, SessionStats, SessionStatus};

use crate::{now_ms, registry::HarnessRegistry, sessions::SessionsEngine};

// Claude Code's own defaults (CLI 2.1.299, `tengu_sunny_locket`): only a 1h
// cache, fire at 90% of its life, at most a minute late, from 200K tokens.
const CACHE_TTL_SECS: u32 = 3600;
const FIRE_AT_FRACTION: f64 = 0.9;
const LATE_MS: i64 = 60_000;
const MIN_TOKENS: u64 = 200_000;
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

pub(crate) fn spawn(sessions: SessionsEngine, registry: Arc<HarnessRegistry>) {
    let min_tokens = std::env::var("ZERON_IDLE_COMPACT_MIN_TOKENS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(MIN_TOKENS);
    tokio::spawn(async move {
        let mut statuses = sessions.watch_sessions();
        let mut seen: HashMap<String, DateTime<Utc>> = HashMap::new();
        let mut armed: HashMap<String, Armed> = HashMap::new();
        let mut fired: HashMap<String, i64> = HashMap::new();
        loop {
            let idle: Vec<Session> = statuses
                .borrow_and_update()
                .iter()
                .filter(|session| settled(session))
                .cloned()
                .collect();
            seen.retain(|chat, _| idle.iter().any(|s| &s.chat_id == chat));
            armed.retain(|chat, _| seen.contains_key(chat));
            for session in &idle {
                if seen.get(&session.chat_id) == Some(&session.updated_at) {
                    continue;
                }
                seen.insert(session.chat_id.clone(), session.updated_at);
                let next = sessions
                    .cache_state(&session.chat_id)
                    .and_then(|(stats, tokens)| arm(&stats, tokens, min_tokens))
                    .filter(|next| fired.get(&session.chat_id) != Some(&next.touched_at));
                match next {
                    Some(next) => armed.insert(session.chat_id.clone(), next),
                    None => armed.remove(&session.chat_id),
                };
            }

            let now = now_ms();
            let due: Vec<(String, Armed)> = armed
                .iter()
                .filter(|(_, at)| at.fire_at <= now)
                .map(|(chat, at)| (chat.clone(), *at))
                .collect();
            for (chat_id, at) in due {
                armed.remove(&chat_id);
                fired.insert(chat_id.clone(), at.touched_at);
                if !registry.idle_compaction_settings().enabled || now - at.fire_at > LATE_MS {
                    continue;
                }
                let still = sessions
                    .cache_state(&chat_id)
                    .and_then(|(stats, tokens)| arm(&stats, tokens, min_tokens));
                if still != Some(at) {
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
                            tracing::info!(chat = %chat_id, error = %err, "idle compaction skipped")
                        }
                    }
                });
            }

            let nap = armed
                .values()
                .map(|at| Duration::from_millis((at.fire_at - now).max(0) as u64))
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
