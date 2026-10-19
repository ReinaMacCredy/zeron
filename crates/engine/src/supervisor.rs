use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};
use zeron_proto::PermissionMode;
use zeron_proto::supervisor::{
    SupervisorEventKind, SupervisorEventSummary, SupervisorSettings, SupervisorSnapshot,
    SupervisorState, SupervisorWake,
};

use crate::EngineError;
use crate::project_actions::persist_json;

const STORE_FILE: &str = "supervisor.json";
const STORE_VERSION: u32 = 1;
const KEPT_WAKES: usize = 200;
const SNAPSHOT_WAKES: usize = 50;
const HOUR_MS: i64 = 60 * 60 * 1000;
pub(crate) const WAKES_PER_HOUR: usize = 12;
pub(crate) const MISSION_SILENT_MS: i64 = 30 * 60 * 1000;
pub(crate) const LEAD_SILENT_MS: i64 = 10 * 60 * 1000;

pub(crate) fn fold_ms() -> i64 {
    std::env::var("ZERON_SUPERVISOR_FOLD_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(60_000)
}

pub(crate) fn utc(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

#[derive(Debug, Clone)]
pub(crate) struct SupervisorEvent {
    pub summary: SupervisorEventSummary,
    pub detail: Vec<String>,
    pub actor: Option<String>,
    pub request_id: Option<String>,
    pub since: Option<i64>,
}

impl SupervisorEvent {
    pub fn new(
        kind: SupervisorEventKind,
        key: String,
        task_id: Option<&str>,
        label: String,
    ) -> Self {
        Self {
            summary: SupervisorEventSummary {
                kind,
                key,
                task_id: task_id.map(str::to_owned),
                step_id: None,
                thread_id: None,
                label,
                at: Utc::now(),
            },
            detail: Vec::new(),
            actor: None,
            request_id: None,
            since: None,
        }
    }

    fn task_id(&self) -> Option<&str> {
        self.summary.task_id.as_deref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SupervisorFile {
    version: u32,
    #[serde(default)]
    wakes: Vec<SupervisorWake>,
    #[serde(default)]
    unwatched: BTreeSet<String>,
    #[serde(default)]
    items: Vec<serde_json::Value>,
    #[serde(default)]
    holds: Vec<serde_json::Value>,
}

#[derive(Default)]
struct Router {
    chat_id: Option<String>,
    pending: Vec<SupervisorEvent>,
    reported: HashSet<String>,
    failed_at: HashMap<String, i64>,
    held: Option<(String, Vec<SupervisorEvent>)>,
}

struct State {
    file: SupervisorFile,
    router: Router,
}

impl State {
    fn snapshot(&self, chat_id: Option<String>, state: SupervisorState) -> SupervisorSnapshot {
        SupervisorSnapshot {
            chat_id,
            state,
            wakes: self
                .file
                .wakes
                .iter()
                .rev()
                .take(SNAPSHOT_WAKES)
                .cloned()
                .collect(),
            unwatched: self.file.unwatched.iter().cloned().collect(),
        }
    }

    fn reported_keys(wakes: &[SupervisorWake]) -> HashSet<String> {
        wakes
            .iter()
            .flat_map(|wake| wake.events.iter().chain(&wake.dropped))
            .map(|event| event.key.clone())
            .collect()
    }
}

struct Inner {
    path: PathBuf,
    state: Mutex<State>,
    wake: Notify,
    snapshot: watch::Sender<SupervisorSnapshot>,
    ensuring: Mutex<()>,
}

/// Written only at a flush or a toggle: events arrive on every step turn end and input raise.
#[derive(Clone)]
pub struct SupervisorStore {
    inner: Arc<Inner>,
}

impl SupervisorStore {
    pub fn open(profile_store_root: &Path) -> Result<Self, EngineError> {
        std::fs::create_dir_all(profile_store_root)?;
        let path = profile_store_root.join(STORE_FILE);
        let file = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<SupervisorFile>(&bytes) {
                Ok(file) if file.version == STORE_VERSION => file,
                Ok(_) | Err(_) => {
                    tracing::warn!(path = %path.display(), "unreadable supervisor store; starting empty");
                    empty_file()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => empty_file(),
            Err(err) => return Err(err.into()),
        };
        let state = State {
            router: Router {
                reported: State::reported_keys(&file.wakes),
                ..Router::default()
            },
            file,
        };
        let (snapshot, _) = watch::channel(state.snapshot(None, SupervisorState::Off));
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                state: Mutex::new(state),
                wake: Notify::new(),
                snapshot,
                ensuring: Mutex::new(()),
            }),
        })
    }

    pub(crate) fn ensuring(&self) -> MutexGuard<'_, ()> {
        lock(&self.inner.ensuring)
    }

    pub(crate) fn watch(&self) -> watch::Receiver<SupervisorSnapshot> {
        self.inner.snapshot.subscribe()
    }

    pub(crate) fn poke(&self) {
        self.inner.wake.notify_one();
    }

    pub(crate) async fn poked(&self) {
        self.inner.wake.notified().await;
    }

    pub(crate) fn chat_id(&self) -> Option<String> {
        lock(&self.inner.state).router.chat_id.clone()
    }

    pub(crate) fn set_chat_id(&self, chat_id: Option<String>) {
        lock(&self.inner.state).router.chat_id = chat_id;
    }

    pub(crate) fn is_unwatched(&self, task_id: &str) -> bool {
        lock(&self.inner.state).file.unwatched.contains(task_id)
    }

    pub(crate) fn offer(&self, event: SupervisorEvent) {
        {
            let mut state = lock(&self.inner.state);
            let key = &event.summary.key;
            if event
                .task_id()
                .is_some_and(|task| state.file.unwatched.contains(task))
                || state.router.reported.contains(key)
                || state.router.pending.iter().any(|e| &e.summary.key == key)
            {
                return;
            }
            state.router.pending.push(event);
        }
        self.poke();
    }

    pub(crate) fn reported(&self, key: &str) -> bool {
        let state = lock(&self.inner.state);
        state.router.reported.contains(key)
            || state.router.pending.iter().any(|e| e.summary.key == key)
    }

    pub(crate) fn set_supervised(
        &self,
        task_id: &str,
        supervised: bool,
    ) -> Result<(), EngineError> {
        let mut state = lock(&self.inner.state);
        let mut file = state.file.clone();
        let changed = if supervised {
            file.unwatched.remove(task_id)
        } else {
            file.unwatched.insert(task_id.to_string())
        };
        if !changed {
            return Ok(());
        }
        persist_json(&self.inner.path, &file)?;
        state.file = file;
        if !supervised {
            state
                .router
                .pending
                .retain(|event| event.task_id() != Some(task_id));
        }
        let unwatched: Vec<String> = state.file.unwatched.iter().cloned().collect();
        self.inner
            .snapshot
            .send_modify(|snapshot| snapshot.unwatched = unwatched);
        Ok(())
    }

    pub(crate) fn note_failed(&self, step_id: &str, at: i64) {
        lock(&self.inner.state)
            .router
            .failed_at
            .insert(step_id.to_string(), at);
    }

    pub(crate) fn failed_at(&self, step_id: &str) -> Option<i64> {
        lock(&self.inner.state)
            .router
            .failed_at
            .get(step_id)
            .copied()
    }

    pub(crate) fn has_pending(&self) -> bool {
        !lock(&self.inner.state).router.pending.is_empty()
    }

    pub(crate) fn take_pending(&self) -> Vec<SupervisorEvent> {
        std::mem::take(&mut lock(&self.inner.state).router.pending)
    }

    pub(crate) fn requeue(&self, events: Vec<SupervisorEvent>) {
        let mut state = lock(&self.inner.state);
        let later = std::mem::replace(&mut state.router.pending, events);
        state.router.pending.extend(later);
    }

    pub(crate) fn wake_times(&self) -> Vec<i64> {
        lock(&self.inner.state)
            .file
            .wakes
            .iter()
            .filter(|wake| wake.woke())
            .map(|wake| wake.at.timestamp_millis())
            .collect()
    }

    pub(crate) fn held(&self) -> Option<(String, Vec<SupervisorEvent>)> {
        lock(&self.inner.state).router.held.clone()
    }

    pub(crate) fn record(&self, wake: SupervisorWake, delivered: Vec<SupervisorEvent>) {
        let mut state = lock(&self.inner.state);
        let keys = wake.events.iter().chain(&wake.dropped);
        let keys: Vec<String> = keys.map(|event| event.key.clone()).collect();
        if wake.woke() {
            state.router.held = Some((wake.id.clone(), delivered));
        }
        state.file.wakes.push(wake);
        let excess = state.file.wakes.len().saturating_sub(KEPT_WAKES);
        if excess > 0 {
            state.file.wakes.drain(..excess);
            state.router.reported = State::reported_keys(&state.file.wakes);
        } else {
            state.router.reported.extend(keys);
        }
        self.persist(&state);
    }

    pub(crate) fn merge(
        &self,
        wake_id: &str,
        delivered: Vec<SupervisorEvent>,
        dropped: Vec<SupervisorEventSummary>,
    ) {
        let mut state = lock(&self.inner.state);
        let added: Vec<SupervisorEventSummary> = delivered
            .iter()
            .map(|event| event.summary.clone())
            .collect();
        state
            .router
            .reported
            .extend(added.iter().chain(&dropped).map(|event| event.key.clone()));
        if let Some(wake) = state.file.wakes.iter_mut().find(|wake| wake.id == wake_id) {
            wake.events.extend(added);
            wake.dropped.extend(dropped);
        }
        if let Some((_, held)) = state.router.held.as_mut() {
            held.extend(delivered);
        }
        self.persist(&state);
    }

    pub(crate) fn publish(&self, chat_id: Option<String>, live: SupervisorState) {
        let next = lock(&self.inner.state).snapshot(chat_id, live);
        self.inner.snapshot.send_if_modified(|snapshot| {
            if *snapshot == next {
                return false;
            }
            *snapshot = next;
            true
        });
    }

    fn persist(&self, state: &State) {
        if let Err(err) = persist_json(&self.inner.path, &state.file) {
            tracing::warn!(error = %err, "saving the supervisor wake log failed");
        }
    }
}

fn empty_file() -> SupervisorFile {
    SupervisorFile {
        version: STORE_VERSION,
        wakes: Vec::new(),
        unwatched: BTreeSet::new(),
        items: Vec::new(),
        holds: Vec::new(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Flush {
    Deliver,
    Merge(String),
    HoldUntil(i64),
}

pub(crate) fn plan_flush(now: i64, fold: i64, wakes: &[i64], undelivered: Option<&str>) -> Flush {
    if let Some(row) = undelivered {
        return Flush::Merge(row.to_string());
    }
    let mut recent: Vec<i64> = wakes
        .iter()
        .copied()
        .filter(|at| now - at < HOUR_MS)
        .collect();
    recent.sort_unstable();
    let mut until = recent.last().map_or(now, |last| last + fold);
    if recent.len() >= WAKES_PER_HOUR {
        until = until.max(recent[recent.len() - WAKES_PER_HOUR] + HOUR_MS);
    }
    if until <= now {
        Flush::Deliver
    } else {
        Flush::HoldUntil(until)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ThreadFacts {
    pub live: bool,
    pub background: bool,
    pub last_message: Option<i64>,
    pub last_live: Option<i64>,
}

impl ThreadFacts {
    fn last_active(&self) -> Option<i64> {
        self.last_message.max(self.last_live)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FailedStep {
    pub step_id: String,
    pub failed_at: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct MissionFacts {
    pub task_id: String,
    pub waiting_on_person: bool,
    pub lead: ThreadFacts,
    pub steps: Vec<ThreadFacts>,
    pub failed: Vec<FailedStep>,
}

impl MissionFacts {
    pub fn quiet_since(&self) -> Option<i64> {
        let threads = || std::iter::once(&self.lead).chain(&self.steps);
        if self.waiting_on_person || threads().any(|t| t.live || t.background) {
            return None;
        }
        threads().filter_map(ThreadFacts::last_active).max()
    }

    pub fn lead_silent_after(&self, failed_at: i64) -> bool {
        !self.lead.live && self.lead.last_active().is_none_or(|at| at < failed_at)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stall {
    MissionSilent {
        task_id: String,
        since: i64,
    },
    LeadSilent {
        task_id: String,
        step_id: String,
        failed_at: i64,
    },
}

impl Stall {
    pub fn key(&self) -> String {
        match self {
            Self::MissionSilent { task_id, since } => format!("silent:{task_id}:{since}"),
            Self::LeadSilent {
                step_id, failed_at, ..
            } => format!("lead-silent:{step_id}:{failed_at}"),
        }
    }
}

pub(crate) fn plan_stalls(
    now: i64,
    missions: &[MissionFacts],
    reported: impl Fn(&str) -> bool,
) -> (Vec<Stall>, Option<i64>) {
    let mut due = Vec::new();
    let mut next: Option<i64> = None;
    let mut consider = |stall: Stall, at: i64| {
        if at > now {
            next = Some(next.map_or(at, |n| n.min(at)));
        } else if !reported(&stall.key()) {
            due.push(stall);
        }
    };
    for mission in missions {
        if let Some(since) = mission.quiet_since() {
            let stall = Stall::MissionSilent {
                task_id: mission.task_id.clone(),
                since,
            };
            consider(stall, since + MISSION_SILENT_MS);
        }
        for failed in &mission.failed {
            if !mission.lead_silent_after(failed.failed_at) {
                continue;
            }
            let stall = Stall::LeadSilent {
                task_id: mission.task_id.clone(),
                step_id: failed.step_id.clone(),
                failed_at: failed.failed_at,
            };
            consider(stall, failed.failed_at + LEAD_SILENT_MS);
        }
    }
    (due, next)
}

pub(crate) struct WakeEntry<'a> {
    pub event: &'a SupervisorEvent,
    pub mission: Option<String>,
    pub reply: Option<Reply>,
}

pub(crate) struct Reply {
    pub from_lead: bool,
    pub text: Option<String>,
}

pub(crate) struct WakeHeader {
    pub number: usize,
    pub since: i64,
    pub held_by_cap_min: Option<i64>,
}

pub(crate) fn wake_text(
    header: WakeHeader,
    entries: &[WakeEntry],
    dropped: &[SupervisorEventSummary],
    settings: SupervisorSettings,
    permission: PermissionMode,
) -> String {
    let since = utc(header.since).with_timezone(&Local).format("%H:%M:%S");
    let count = match entries.len() {
        1 => "1 event".to_string(),
        n => format!("{n} events"),
    };
    let mut text = format!(
        "Supervisor wake {} of {WAKES_PER_HOUR} this hour · {count} since {since}\n",
        header.number
    );
    if let Some(minutes) = header.held_by_cap_min {
        text.push_str(&format!("Held {minutes} min by the hourly cap.\n"));
    }
    for (n, entry) in entries.iter().enumerate() {
        let summary = &entry.event.summary;
        text.push_str(&format!("\n{}. {}", n + 1, summary.label));
        if let Some(mission) = &entry.mission {
            text.push_str(&format!(" · {mission}"));
        }
        if let Some(step) = &summary.step_id {
            text.push_str(&format!(" · taskStepId {step}"));
        }
        if let Some(thread) = &summary.thread_id {
            text.push_str(&format!(" · thread {thread}"));
        }
        text.push('\n');
        for line in &entry.event.detail {
            text.push_str(&format!("   {line}\n"));
        }
        if let Some(reply) = &entry.reply {
            let whose = if reply.from_lead {
                "from the lead"
            } else {
                "on that thread"
            };
            match &reply.text {
                Some(said) => text.push_str(&format!("   Last reply {whose}: \"{said}\"\n")),
                None => text.push_str(&format!("   No reply {whose} yet.\n")),
            }
        }
    }
    if !dropped.is_empty() {
        let labels: Vec<&str> = dropped.iter().map(|event| event.label.as_str()).collect();
        text.push_str(&format!(
            "\nNo longer true, so left out: {}.\n",
            labels.join("; ")
        ));
    }
    text.push('\n');
    text.push_str(&authority_footer(settings, permission));
    text
}

fn authority_footer(settings: SupervisorSettings, permission: PermissionMode) -> String {
    let on = |allowed: bool| if allowed { "on" } else { "off" };
    let approvals = if settings.message_threads && permission != PermissionMode::Full {
        " (tool approvals need this chat at Full access)"
    } else {
        ""
    };
    let rebrief = if settings.rebrief && settings.message_threads {
        "on, by telling the step's lead what to change"
    } else {
        "off"
    };
    format!(
        "Read more with read_thread, list_threads, or the tasks tool (action \"list\", taskId). What you may do now:\n- message leads and step threads with send_message_to_thread: {}\n- answer their questions and tool approvals with respond_to_input: {}{approvals}\n- re-brief a step: {rebrief}\nA change to a mission's goal or spend is the person's decision: name it in your reply.\nIf nothing needs doing, reply in one line.",
        on(settings.message_threads),
        on(settings.message_threads),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;
    const FOLD: i64 = 60_000;

    #[test]
    fn a_wake_leads_its_window_merges_while_queued_and_holds_at_the_hourly_cap() {
        let now = 10 * HOUR_MS;
        let twelve: Vec<i64> = (0..12).map(|n| now - 55 * MIN + n * 4 * MIN).collect();
        let eleven = &twelve[1..];
        let cases: [(&str, Vec<i64>, Option<&str>, Flush); 7] = [
            ("first event", vec![], None, Flush::Deliver),
            ("window over", vec![now - FOLD], None, Flush::Deliver),
            (
                "inside the window",
                vec![now - 20_000],
                None,
                Flush::HoldUntil(now + 40_000),
            ),
            (
                "queued row",
                vec![now - 20_000],
                Some("w1"),
                Flush::Merge("w1".into()),
            ),
            ("eleven this hour", eleven.to_vec(), None, Flush::Deliver),
            (
                "thirteenth",
                twelve.clone(),
                None,
                Flush::HoldUntil(twelve[0] + HOUR_MS),
            ),
            (
                "oldest left the hour",
                twelve.iter().map(|at| at - 5 * MIN).collect(),
                None,
                Flush::Deliver,
            ),
        ];
        for (name, wakes, undelivered, expected) in cases {
            assert_eq!(
                plan_flush(now, FOLD, &wakes, undelivered),
                expected,
                "{name}"
            );
        }
    }

    fn thread(live: bool, background: bool, last_message: i64) -> ThreadFacts {
        ThreadFacts {
            live,
            background,
            last_message: Some(last_message),
            last_live: None,
        }
    }

    fn mission(lead: ThreadFacts, steps: Vec<ThreadFacts>) -> MissionFacts {
        MissionFacts {
            task_id: "t1".into(),
            waiting_on_person: false,
            lead,
            steps,
            failed: Vec::new(),
        }
    }

    #[test]
    fn a_stall_fires_once_per_silence_and_never_while_work_or_the_person_holds_it() {
        let now = 10 * HOUR_MS;
        let old = now - 31 * MIN;
        let silent = |since| Stall::MissionSilent {
            task_id: "t1".into(),
            since,
        };
        let quiet = mission(
            thread(false, false, old),
            vec![thread(false, false, old - MIN)],
        );
        let mut review = quiet.clone();
        review.waiting_on_person = true;
        let mut failed = mission(
            thread(false, false, now - 12 * MIN),
            vec![thread(false, false, now - 11 * MIN)],
        );
        failed.failed = vec![FailedStep {
            step_id: "s1".into(),
            failed_at: now - 11 * MIN,
        }];
        let mut lead_ran = failed.clone();
        lead_ran.lead.last_live = Some(now - 5 * MIN);
        let mut recent_failure = failed.clone();
        recent_failure.failed[0].failed_at = now - 4 * MIN;
        let lead_silent = Stall::LeadSilent {
            task_id: "t1".into(),
            step_id: "s1".into(),
            failed_at: now - 11 * MIN,
        };
        let cases: [(&str, MissionFacts, bool, Vec<Stall>, Option<i64>); 9] = [
            (
                "silent 31 min",
                quiet.clone(),
                false,
                vec![silent(old)],
                None,
            ),
            ("already reported", quiet.clone(), true, vec![], None),
            (
                "silent 10 min",
                mission(thread(false, false, now - 10 * MIN), vec![]),
                false,
                vec![],
                Some(now + 20 * MIN),
            ),
            (
                "step working",
                mission(thread(false, false, old), vec![thread(true, false, old)]),
                false,
                vec![],
                None,
            ),
            (
                "background task",
                mission(thread(false, true, old), vec![]),
                false,
                vec![],
                None,
            ),
            ("plan review or a question", review, false, vec![], None),
            (
                "lead silent 11 min",
                failed,
                false,
                vec![lead_silent],
                Some(now + 19 * MIN),
            ),
            (
                "lead ran after",
                lead_ran,
                false,
                vec![],
                Some(now + 25 * MIN),
            ),
            (
                "failure 4 min ago",
                recent_failure,
                false,
                vec![],
                Some(now + 6 * MIN),
            ),
        ];
        for (name, facts, reported, stalls, next) in cases {
            let (due, deadline) = plan_stalls(now, &[facts], |_| reported);
            assert_eq!((due, deadline), (stalls, next), "{name}");
        }
    }
}
