use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};
use zeron_proto::{LimitOffer, LimitResume, LimitResumes, OfferChoice, RouteTarget};

/// Windows reset on the provider's clock, not ours.
const RESET_SLACK_MS: i64 = 15_000;
/// Monotonic sleeps stop while the Mac sleeps; re-read the wall clock often.
const MAX_NAP: Duration = Duration::from_secs(60);
/// A restart that never came (a failed install) must not continue old turns
/// at some unrelated later launch.
const RESTART_FRESH_MS: i64 = 10 * 60 * 1000;
const RESET_KEPT_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Continuation {
    pub chat_id: String,
    #[serde(default)]
    pub after_user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Limited {
    #[serde(flatten)]
    at: Continuation,
    #[serde(default)]
    resets_at: Option<i64>,
    auto: bool,
    #[serde(default)]
    offer: Option<LimitOffer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Deferred {
    chat_id: String,
    at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OfferResolution {
    Switch,
    Wait,
    Hold,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Due {
    Resume(Continuation),
    Start(String),
    Switch(String, RouteTarget),
}

/// A limit whose reset passed with nothing left to resume it; gone once the chat runs again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reset {
    chat_id: String,
    resets_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    #[serde(default = "enabled")]
    auto_resume: bool,
    #[serde(default)]
    limited: Vec<Limited>,
    #[serde(default)]
    after_restart: Vec<Continuation>,
    #[serde(default)]
    restart_recorded_at: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    reset: Vec<Reset>,
    #[serde(default)]
    deferred: Vec<Deferred>,
}

fn enabled() -> bool {
    true
}

impl Default for Stored {
    fn default() -> Self {
        Self {
            auto_resume: true,
            limited: Vec::new(),
            after_restart: Vec::new(),
            restart_recorded_at: 0,
            reset: Vec::new(),
            deferred: Vec::new(),
        }
    }
}

impl Stored {
    fn view(&self) -> LimitResumes {
        LimitResumes {
            auto_resume: self.auto_resume,
            chats: self
                .limited
                .iter()
                .map(|entry| LimitResume {
                    chat_id: entry.at.chat_id.clone(),
                    resets_at: entry.resets_at,
                    auto: entry.auto,
                    offer: entry.offer.clone(),
                })
                .collect(),
        }
    }

    fn take_due(&mut self, now: i64) -> (Vec<Due>, Option<i64>) {
        let mut due = Vec::new();
        let mut next: Option<i64> = None;
        let mut reset = Vec::new();
        self.deferred.retain(|entry| {
            if entry.at <= now {
                due.push(Due::Start(entry.chat_id.clone()));
                return false;
            }
            next = Some(next.map_or(entry.at, |next| next.min(entry.at)));
            true
        });
        self.limited.retain_mut(|entry| {
            if let Some(offer) = &entry.offer
                && let Some(act_at) = offer.act_at
            {
                if act_at > now {
                    next = Some(next.map_or(act_at, |next| next.min(act_at)));
                } else {
                    match (offer.preselect, offer.to.clone()) {
                        (OfferChoice::Switch, Some(to)) => {
                            due.push(Due::Switch(entry.at.chat_id.clone(), to));
                            return false;
                        }
                        _ => entry.offer = None,
                    }
                }
            }
            let Some(resets_at) = entry.resets_at else {
                return true;
            };
            let at = match entry.auto {
                true => resets_at + RESET_SLACK_MS,
                false => resets_at,
            };
            if at > now {
                if entry.auto {
                    next = Some(next.map_or(at, |next| next.min(at)));
                }
                return true;
            }
            if !entry.auto && entry.offer.is_some() {
                return true;
            }
            if entry.auto {
                due.push(Due::Resume(entry.at.clone()));
            }
            reset.push(Reset {
                chat_id: entry.at.chat_id.clone(),
                resets_at,
            });
            false
        });
        self.reset.extend(reset);
        self.reset
            .retain(|reset| now - reset.resets_at < RESET_KEPT_MS);
        (due, next)
    }

    fn shape(&self) -> (usize, usize, usize, usize) {
        let offers = self.limited.iter().filter(|e| e.offer.is_some()).count();
        (
            self.limited.len(),
            offers,
            self.deferred.len(),
            self.reset.len(),
        )
    }
}

#[derive(Clone)]
pub struct ResumeSchedule {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    stored: Mutex<Stored>,
    tx: watch::Sender<LimitResumes>,
    wake: Notify,
}

impl ResumeSchedule {
    pub fn open(data_dir: &Path) -> Self {
        let path = data_dir.join("resume-schedule.json");
        let stored: Stored = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let (tx, _) = watch::channel(stored.view());
        Self {
            inner: Arc::new(Inner {
                path,
                stored: Mutex::new(stored),
                tx,
                wake: Notify::new(),
            }),
        }
    }

    pub fn watch(&self) -> watch::Receiver<LimitResumes> {
        self.inner.tx.subscribe()
    }

    pub fn snapshot(&self) -> LimitResumes {
        lock(&self.inner.stored).view()
    }

    /// A reset time that is missing or already past never arms: the provider
    /// would refuse the continuation again, and again.
    pub fn note_limit(
        &self,
        chat_id: &str,
        resets_at: Option<i64>,
        after_user: Option<String>,
        now: i64,
    ) {
        self.update(|stored| {
            let auto = stored.auto_resume && resets_at.is_some_and(|at| at > now);
            stored.limited.retain(|entry| entry.at.chat_id != chat_id);
            stored.reset.retain(|reset| reset.chat_id != chat_id);
            stored.limited.push(Limited {
                at: Continuation {
                    chat_id: chat_id.to_owned(),
                    after_user,
                },
                resets_at,
                auto,
                offer: None,
            });
            true
        });
    }

    pub fn set_offer(&self, chat_id: &str, offer: LimitOffer) {
        self.update(|stored| {
            let Some(entry) = stored
                .limited
                .iter_mut()
                .find(|entry| entry.at.chat_id == chat_id)
            else {
                return false;
            };
            entry.offer = Some(offer);
            true
        });
    }

    pub fn resolve_offer(&self, chat_id: &str, choice: OfferResolution) -> Option<LimitOffer> {
        let mut taken = None;
        self.update(|stored| {
            let Some(at) = stored.limited.iter().position(|entry| {
                entry.at.chat_id == chat_id
                    && entry.offer.as_ref().is_some_and(|offer| {
                        choice != OfferResolution::Switch || offer.to.is_some()
                    })
            }) else {
                return false;
            };
            match choice {
                OfferResolution::Switch => taken = stored.limited.remove(at).offer,
                OfferResolution::Wait => taken = stored.limited[at].offer.take(),
                OfferResolution::Hold => {
                    if let Some(offer) = stored.limited[at].offer.as_mut() {
                        offer.act_at = None;
                        taken = Some(offer.clone());
                    }
                }
            }
            true
        });
        taken
    }

    pub fn holds(&self, chat_id: &str, now: i64) -> bool {
        lock(&self.inner.stored).limited.iter().any(|entry| {
            entry.at.chat_id == chat_id
                && (entry.offer.is_some()
                    || (entry.auto && entry.resets_at.is_some_and(|at| at + RESET_SLACK_MS > now)))
        })
    }

    pub fn set_auto(&self, chat_id: &str, auto: bool, now: i64) -> bool {
        self.update(|stored| {
            let Some(entry) = stored
                .limited
                .iter_mut()
                .find(|entry| entry.at.chat_id == chat_id)
            else {
                return false;
            };
            let auto = auto && entry.resets_at.is_some_and(|at| at > now);
            let changed = entry.auto != auto;
            entry.auto = auto;
            changed
        })
    }

    pub fn set_auto_resume(&self, enabled: bool) {
        self.update(|stored| {
            let changed = stored.auto_resume != enabled;
            stored.auto_resume = enabled;
            changed
        });
    }

    pub fn defer_start(&self, chat_id: &str, at: i64) {
        self.update(|stored| {
            stored.deferred.retain(|entry| entry.chat_id != chat_id);
            stored.deferred.push(Deferred {
                chat_id: chat_id.to_owned(),
                at,
            });
            true
        });
    }

    pub fn deferred_until(&self, chat_id: &str) -> Option<i64> {
        lock(&self.inner.stored)
            .deferred
            .iter()
            .find(|entry| entry.chat_id == chat_id)
            .map(|entry| entry.at)
    }

    pub fn clear(&self, chat_id: &str) {
        self.update(|stored| {
            let before = stored.limited.len() + stored.reset.len();
            stored.limited.retain(|entry| entry.at.chat_id != chat_id);
            stored.reset.retain(|reset| reset.chat_id != chat_id);
            stored.limited.len() + stored.reset.len() != before
        });
    }

    /// Chats whose limit reset by `now` with no auto-resume left pending, and that have not run since.
    pub fn past_resets(&self, now: i64) -> Vec<(String, i64)> {
        let stored = lock(&self.inner.stored);
        let elapsed = stored.limited.iter().filter_map(|entry| {
            let resets_at = entry.resets_at.filter(|at| *at <= now && !entry.auto)?;
            Some((entry.at.chat_id.clone(), resets_at))
        });
        let reset = stored
            .reset
            .iter()
            .map(|reset| (reset.chat_id.clone(), reset.resets_at));
        elapsed.chain(reset).collect()
    }

    /// Kept on disk for the next boot only; this engine never sends them.
    pub fn record_restart(&self, chats: Vec<Continuation>, now: i64) {
        self.update(|stored| {
            stored.after_restart = chats;
            stored.restart_recorded_at = now;
            true
        });
    }

    pub fn restart_continuation(&self, chat_id: &str) -> Option<Option<String>> {
        lock(&self.inner.stored)
            .after_restart
            .iter()
            .find(|entry| entry.chat_id == chat_id)
            .map(|entry| entry.after_user.clone())
    }

    pub fn take_restart(&self, now: i64) -> Vec<Continuation> {
        let mut taken = Vec::new();
        self.update(|stored| {
            taken = std::mem::take(&mut stored.after_restart);
            if now - stored.restart_recorded_at > RESTART_FRESH_MS {
                taken.clear();
            }
            stored.restart_recorded_at = 0;
            true
        });
        taken
    }

    pub fn spawn<F, Fut>(&self, fire: F)
    where
        F: Fn(Due) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let schedule = self.clone();
        tokio::spawn(async move {
            loop {
                let now = chrono::Utc::now().timestamp_millis();
                let (due, next) = schedule.tick(now);
                for due in due {
                    fire(due).await;
                }
                let nap = next.map_or(MAX_NAP, |at| {
                    Duration::from_millis(u64::try_from(at - now).unwrap_or(0)).min(MAX_NAP)
                });
                tokio::select! {
                    _ = tokio::time::sleep(nap) => {}
                    _ = schedule.inner.wake.notified() => {}
                }
            }
        });
    }

    fn tick(&self, now: i64) -> (Vec<Due>, Option<i64>) {
        let mut due = Vec::new();
        let mut next = None;
        self.update(|stored| {
            let before = stored.shape();
            (due, next) = stored.take_due(now);
            stored.shape() != before
        });
        (due, next)
    }

    fn update(&self, change: impl FnOnce(&mut Stored) -> bool) -> bool {
        let mut stored = lock(&self.inner.stored);
        if !change(&mut stored) {
            return false;
        }
        match serde_json::to_string_pretty(&*stored) {
            Ok(json) => {
                let temp = self.inner.path.with_extension("json.tmp");
                if let Err(error) = std::fs::write(&temp, json)
                    .and_then(|()| std::fs::rename(&temp, &self.inner.path))
                {
                    tracing::warn!(%error, "resume schedule save failed");
                }
            }
            Err(error) => tracing::warn!(%error, "resume schedule serialize failed"),
        }
        self.inner.tx.send_replace(stored.view());
        drop(stored);
        self.inner.wake.notify_one();
        true
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule() -> (tempfile::TempDir, ResumeSchedule) {
        let dir = tempfile::tempdir().unwrap();
        let schedule = ResumeSchedule::open(dir.path());
        (dir, schedule)
    }

    #[test]
    fn only_a_future_reset_arms_and_it_fires_after_the_slack() {
        let (_dir, schedule) = schedule();
        schedule.note_limit("past", Some(900), None, 1_000);
        schedule.note_limit("unknown", None, None, 1_000);
        schedule.note_limit("future", Some(5_000), Some("u1".into()), 1_000);
        let armed: Vec<_> = schedule
            .snapshot()
            .chats
            .into_iter()
            .map(|chat| (chat.chat_id, chat.auto))
            .collect();
        assert_eq!(
            armed,
            [
                ("past".to_owned(), false),
                ("unknown".to_owned(), false),
                ("future".to_owned(), true)
            ]
        );
        let mut stored = lock(&schedule.inner.stored);
        assert_eq!(
            stored.take_due(5_000),
            (vec![], Some(5_000 + RESET_SLACK_MS))
        );
        let (due, _) = stored.take_due(5_000 + RESET_SLACK_MS);
        assert_eq!(
            due,
            [Due::Resume(Continuation {
                chat_id: "future".into(),
                after_user: Some("u1".into())
            })]
        );
        let left: Vec<_> = stored
            .limited
            .iter()
            .map(|e| e.at.chat_id.as_str())
            .collect();
        assert_eq!(left, ["unknown"], "an elapsed unarmed limit expires");
        drop(stored);
        let mut past = schedule.past_resets(5_000 + RESET_SLACK_MS);
        past.sort();
        assert_eq!(
            past,
            [("future".to_owned(), 5_000), ("past".to_owned(), 900)],
            "both resets stay known until their chats run again"
        );
        schedule.clear("past");
        schedule.note_limit("future", Some(9_000), None, 6_000);
        assert!(schedule.past_resets(6_000).is_empty());
    }

    #[test]
    fn the_off_switch_and_restart_list_survive_reopen() {
        let (dir, schedule) = schedule();
        schedule.set_auto_resume(false);
        schedule.note_limit("c", Some(5_000), None, 1_000);
        assert!(!schedule.snapshot().chats[0].auto);
        assert!(schedule.set_auto("c", true, 1_000));
        let restart = vec![Continuation {
            chat_id: "r".into(),
            after_user: None,
        }];
        schedule.record_restart(restart.clone(), 1_000);
        let reopened = ResumeSchedule::open(dir.path());
        assert!(!reopened.snapshot().auto_resume);
        assert!(reopened.snapshot().chats[0].auto);
        assert_eq!(reopened.restart_continuation("r"), Some(None));
        assert_eq!(reopened.take_restart(2_000), restart);
        assert!(
            ResumeSchedule::open(dir.path())
                .take_restart(2_000)
                .is_empty()
        );
        reopened.record_restart(restart, 1_000);
        assert!(
            reopened
                .take_restart(1_000 + RESTART_FRESH_MS + 1)
                .is_empty()
        );
    }

    #[test]
    fn a_deferred_start_fires_at_its_time_and_survives_reopen() {
        let (dir, schedule) = schedule();
        schedule.defer_start("child", 9_000);
        let reopened = ResumeSchedule::open(dir.path());
        let mut stored = lock(&reopened.inner.stored);
        assert_eq!(stored.take_due(8_999), (vec![], Some(9_000)));
        assert_eq!(stored.take_due(9_000).0, [Due::Start("child".into())]);
        assert!(stored.deferred.is_empty());
    }

    fn offer(preselect: OfferChoice, act_at: Option<i64>) -> LimitOffer {
        LimitOffer {
            to: Some(RouteTarget {
                harness: zeron_proto::HarnessId::ClaudeCode,
                model: Some("claude-opus-5-5".into()),
                reasoning: None,
            }),
            preselect,
            act_at,
        }
    }

    #[test]
    fn a_due_switch_offer_fires_once_and_a_wait_offer_falls_to_auto_resume() {
        let (_dir, schedule) = schedule();
        schedule.note_limit("switch", Some(90_000), None, 1_000);
        schedule.set_offer("switch", offer(OfferChoice::Switch, Some(2_000)));
        schedule.note_limit("wait", Some(90_000), None, 1_000);
        schedule.set_offer("wait", offer(OfferChoice::Wait, Some(2_000)));
        assert!(schedule.holds("switch", 1_500) && schedule.holds("wait", 1_500));
        let mut stored = lock(&schedule.inner.stored);
        assert_eq!(stored.take_due(1_500), (vec![], Some(2_000)));
        let (due, next) = stored.take_due(2_000);
        assert_eq!(
            due,
            [Due::Switch(
                "switch".into(),
                offer(OfferChoice::Switch, None).to.unwrap()
            )]
        );
        assert_eq!(next, Some(90_000 + RESET_SLACK_MS));
        let wait = stored
            .limited
            .iter()
            .find(|e| e.at.chat_id == "wait")
            .unwrap();
        assert!(
            wait.offer.is_none() && wait.auto,
            "today's auto-resume stays armed"
        );
    }

    #[test]
    fn an_expired_wait_offer_is_saved_and_broadcast_cleared() {
        let (dir, schedule) = schedule();
        schedule.note_limit("wait", Some(90_000), None, 1_000);
        schedule.set_offer("wait", offer(OfferChoice::Wait, Some(2_000)));
        let limits = schedule.watch();
        assert_eq!(
            schedule.tick(2_000),
            (vec![], Some(90_000 + RESET_SLACK_MS))
        );
        assert!(limits.has_changed().unwrap());
        assert_eq!(limits.borrow().chats[0].offer, None);
        assert_eq!(
            ResumeSchedule::open(dir.path()).snapshot().chats[0].offer,
            None
        );
    }

    #[test]
    fn an_unarmed_limit_past_its_reset_waits_for_its_offer() {
        let (_dir, schedule) = schedule();
        schedule.note_limit("c", Some(5_000), None, 6_000);
        schedule.set_offer("c", offer(OfferChoice::Switch, None));
        assert_eq!(schedule.tick(7_000), (vec![], None));
        assert!(schedule.snapshot().chats[0].offer.is_some());
        assert!(schedule.holds("c", 7_000));
        schedule.resolve_offer("c", OfferResolution::Wait);
        assert_eq!(schedule.tick(7_000), (vec![], None));
        assert!(schedule.snapshot().chats.is_empty());
        assert_eq!(schedule.past_resets(7_000), [("c".to_owned(), 5_000)]);
    }

    #[test]
    fn hold_clears_the_countdown_and_switch_takes_the_entry() {
        let (_dir, schedule) = schedule();
        schedule.note_limit("c", Some(90_000), None, 1_000);
        schedule.set_offer("c", offer(OfferChoice::Switch, Some(2_000)));
        let held = schedule.resolve_offer("c", OfferResolution::Hold).unwrap();
        assert_eq!(held.act_at, None);
        assert_eq!(schedule.snapshot().chats[0].offer, Some(held));
        assert!(
            schedule
                .resolve_offer("c", OfferResolution::Switch)
                .is_some()
        );
        assert!(schedule.snapshot().chats.is_empty());
        assert!(!schedule.holds("c", 1_000));
        schedule.note_limit("c", Some(90_000), None, 1_000);
        schedule.set_offer(
            "c",
            LimitOffer {
                to: None,
                ..offer(OfferChoice::Wait, None)
            },
        );
        assert_eq!(schedule.resolve_offer("c", OfferResolution::Switch), None);
        assert!(schedule.snapshot().chats[0].offer.is_some());
    }
}
