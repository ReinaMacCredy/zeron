//! The workspace registry replica and its derived [`WorkspaceSnapshot`].
//!
//! The replica is a [`RegistryDoc`] shared (`Arc<Mutex<_>>`) with the
//! `zeron_sync::RegistryClient` in live mode; Demo mode's replica is
//! local-only (writes fold straight into its rows). Reads materialize
//! the rows once per registry *generation* (cached), so the 1 Hz time-driven
//! re-derivation (staleness, presence expiry, send grace) never re-decodes.

mod tasks;
mod today;
mod view;

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use zeron_doc::{RegistryDoc, WorkspaceState};
use zeron_proto::{
    AgentAccountsSnapshot, ChangeRequestListItem, CheckoutChangeRequestStatus, LimitResume,
    LimitResumes, Session, SidebarPreferences, TodayRuns,
};

pub use tasks::{ContextChat, HostRef, OutlineRow, TaskDetail, TaskDraft, TaskRow, TasksPage};
pub(crate) use tasks::{
    detail as task_detail, outline as task_outline, outline_docs as task_outline_docs,
    trees as task_trees,
};
pub(crate) use today::Ask;
pub use today::{TodayPage, TodayPart, TodayPullRequests, TodaySlot, TodayTask, TodayUsage};
pub(crate) use view::{DeriveContext, TodayRefresh, device_online};
pub use view::{
    DeviceView, FrontGroup, FrontPage, PROJECT_COLOR_COUNT, ProjectRef, ProjectView, PullRequestGroups,
    SearchField, SearchHit, SectionView, SessionRow, SidebarView, WorkspaceSnapshot,
    project_color_index, relative_time_label,
};

use crate::{lock, read, write};

struct StateCache {
    generation: u64,
    state: Arc<WorkspaceState>,
    prefs: Option<SidebarPreferences>,
}

pub(crate) struct WorkspaceStore {
    doc: Arc<Mutex<RegistryDoc>>,
    presence: Mutex<HashMap<String, i64>>,
    change_requests: Mutex<Vec<CheckoutChangeRequestStatus>>,
    limits: Mutex<HashMap<String, LimitResumes>>,
    limits_rev: AtomicU64,
    host_sessions: Mutex<BTreeMap<String, Vec<Session>>>,
    settle_days: Mutex<HashMap<String, u32>>,
    sidebar: Mutex<SidebarView>,
    today_refresh: Mutex<TodayRefresh>,
    today_runs: Mutex<HashMap<String, TodayRuns>>,
    accounts: Mutex<HashMap<String, AgentAccountsSnapshot>>,
    today_prs: Mutex<HashMap<String, Vec<ChangeRequestListItem>>>,
    cache: Mutex<Option<StateCache>>,
    snapshot: RwLock<Arc<WorkspaceSnapshot>>,
    recompute: Mutex<()>,
    revision: AtomicU64,
}

impl WorkspaceStore {
    pub(crate) fn new(doc: RegistryDoc) -> Self {
        Self {
            doc: Arc::new(Mutex::new(doc)),
            presence: Mutex::new(HashMap::new()),
            change_requests: Mutex::new(Vec::new()),
            limits: Mutex::new(HashMap::new()),
            limits_rev: AtomicU64::new(0),
            host_sessions: Mutex::new(BTreeMap::new()),
            settle_days: Mutex::new(HashMap::new()),
            sidebar: Mutex::new(SidebarView::default()),
            today_refresh: Mutex::new(TodayRefresh::default()),
            today_runs: Mutex::new(HashMap::new()),
            accounts: Mutex::new(HashMap::new()),
            today_prs: Mutex::new(HashMap::new()),
            cache: Mutex::new(None),
            snapshot: RwLock::new(Arc::new(WorkspaceSnapshot::default())),
            recompute: Mutex::new(()),
            revision: AtomicU64::new(0),
        }
    }

    /// The shared replica (handed to `RegistryClient` in live mode).
    #[allow(dead_code)] // live registry client
    pub(crate) fn doc(&self) -> &Arc<Mutex<RegistryDoc>> {
        &self.doc
    }

    pub(crate) fn mutate<R>(&self, f: impl FnOnce(&mut RegistryDoc) -> R) -> R {
        f(&mut lock(&self.doc))
    }

    /// Materialized rows + sidebar prefs, re-read only when the replica's
    /// generation moved.
    pub(crate) fn state(&self) -> (Arc<WorkspaceState>, Option<SidebarPreferences>) {
        let doc = lock(&self.doc);
        let generation = doc.generation();
        let mut cache = lock(&self.cache);
        if let Some(cached) = cache.as_ref()
            && cached.generation == generation
        {
            return (cached.state.clone(), cached.prefs.clone());
        }
        let state = Arc::new(doc.read_all().unwrap_or_else(|err| {
            tracing::warn!(error = %err, "registry read failed");
            WorkspaceState {
                devices: vec![],
                spaces: vec![],
                chats: vec![],
                sessions: vec![],
                tasks: vec![],
            }
        }));
        let prefs = doc.sidebar_preferences();
        *cache = Some(StateCache {
            generation,
            state: state.clone(),
            prefs: prefs.clone(),
        });
        (state, prefs)
    }

    pub(crate) fn chat(&self, chat_id: &str) -> Option<zeron_proto::Chat> {
        self.state()
            .0
            .chats
            .iter()
            .find(|c| c.id == chat_id)
            .cloned()
    }

    pub(crate) fn set_presence(&self, device_id: &str, at_ms: i64) {
        lock(&self.presence).insert(device_id.to_owned(), at_ms);
    }

    #[allow(dead_code)] // live registry client
    pub(crate) fn replace_presence(&self, presence: HashMap<String, i64>) {
        *lock(&self.presence) = presence;
    }

    pub(crate) fn presence(&self) -> HashMap<String, i64> {
        lock(&self.presence).clone()
    }

    /// Latest-wins upsert of one checkout's PR resolution.
    pub(crate) fn put_change_request(&self, status: CheckoutChangeRequestStatus) {
        let mut all = lock(&self.change_requests);
        all.retain(|s| {
            !(s.device_id == status.device_id
                && s.cwd == status.cwd
                && s.branch == status.branch
                && s.checkout_id == status.checkout_id)
        });
        all.push(status);
    }

    pub(crate) fn put_limits(&self, device_id: &str, limits: LimitResumes) -> bool {
        let mut all = lock(&self.limits);
        if all.get(device_id) == Some(&limits) {
            return false;
        }
        all.insert(device_id.to_owned(), limits);
        self.limits_rev.fetch_add(1, Ordering::AcqRel);
        true
    }

    pub(crate) fn limit(&self, chat_id: &str) -> Option<LimitResume> {
        lock(&self.limits)
            .values()
            .flat_map(|l| &l.chats)
            .find(|c| c.chat_id == chat_id)
            .cloned()
    }

    pub(crate) fn limits(&self) -> HashMap<String, LimitResumes> {
        lock(&self.limits).clone()
    }

    pub(crate) fn limits_rev(&self) -> u64 {
        self.limits_rev.load(Ordering::Acquire)
    }

    pub(crate) fn put_settle_days(&self, device_id: &str, days: u32) {
        lock(&self.settle_days).insert(device_id.to_owned(), days);
    }

    pub(crate) fn put_today(
        &self,
        device_id: &str,
        runs: Option<TodayRuns>,
        accounts: Option<AgentAccountsSnapshot>,
        prs: Option<Vec<ChangeRequestListItem>>,
    ) {
        if let Some(runs) = runs {
            lock(&self.today_runs).insert(device_id.to_owned(), runs);
        }
        if let Some(accounts) = accounts {
            lock(&self.accounts).insert(device_id.to_owned(), accounts);
        }
        if let Some(prs) = prs {
            lock(&self.today_prs).insert(device_id.to_owned(), prs);
        }
    }

    pub(crate) fn set_sidebar_view(&self, view: SidebarView) {
        *lock(&self.sidebar) = view;
    }

    pub(crate) fn set_today_refresh(&self, loaded: bool) {
        let mut refresh = lock(&self.today_refresh);
        if loaded {
            refresh.updated = Some(chrono::Local::now());
        }
        refresh.failed = !loaded;
    }

    /// A host's stream also carries other devices' registry copies, whose
    /// background work is always empty: only its own chats are kept.
    pub(crate) fn put_host_sessions(&self, device_id: &str, sessions: Vec<Session>) -> bool {
        let own: Vec<Session> = sessions
            .into_iter()
            .filter(|s| s.device_id == device_id)
            .collect();
        let mut all = lock(&self.host_sessions);
        if all.get(device_id) == Some(&own) {
            return false;
        }
        all.insert(device_id.to_owned(), own);
        true
    }

    pub(crate) fn drop_host_sessions(&self, device_id: &str) -> bool {
        lock(&self.host_sessions).remove(device_id).is_some()
    }

    pub(crate) fn snapshot(&self) -> Arc<WorkspaceSnapshot> {
        read(&self.snapshot).clone()
    }

    /// Re-derive; returns the new revision when content changed.
    pub(crate) fn recompute(
        &self,
        self_device_id: &str,
        send_states: &HashMap<String, crate::SendState>,
        asks: &HashMap<String, Ask>,
        synced: bool,
    ) -> Option<u64> {
        let _serial = lock(&self.recompute);
        let (state, prefs) = self.state();
        let presence = self.presence();
        let change_requests = lock(&self.change_requests).clone();
        let host_sessions = lock(&self.host_sessions).clone();
        let settle_days = lock(&self.settle_days).clone();
        let today_runs = lock(&self.today_runs).clone();
        let accounts = lock(&self.accounts).clone();
        let today_prs = lock(&self.today_prs).clone();
        let sidebar = *lock(&self.sidebar);
        let today_refresh = *lock(&self.today_refresh);
        let previous = self.snapshot();
        let mut next = view::derive(
            &state,
            prefs.as_ref(),
            &DeriveContext {
                self_device_id,
                now: chrono::Utc::now(),
                presence: &presence,
                change_requests: &change_requests,
                host_sessions: &host_sessions,
                send_states,
                asks,
                synced,
                settle_days: &settle_days,
                sidebar,
                today_refresh,
                today_runs: &today_runs,
                accounts: &accounts,
                today_prs: &today_prs,
                previous: Some(&previous),
            },
        );
        if previous.revision > 0 && next.content_hash() == previous.content_hash() {
            return None;
        }
        let revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        next.revision = revision;
        *write(&self.snapshot) = Arc::new(next);
        Some(revision)
    }
}
