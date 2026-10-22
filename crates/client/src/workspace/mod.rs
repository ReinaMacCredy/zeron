//! The workspace registry replica and its derived [`WorkspaceSnapshot`].
//!
//! The replica is a [`RegistryDoc`] shared (`Arc<Mutex<_>>`) with the
//! `zeron_sync::RegistryClient` in live mode; Demo mode's replica is
//! local-only (writes fold straight into its rows). Reads materialize
//! the rows once per registry *generation* (cached), so the 1 Hz time-driven
//! re-derivation (staleness, presence expiry, send grace) never re-decodes.

mod tasks;
mod view;

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use zeron_doc::{RegistryDoc, WorkspaceState};
use zeron_proto::supervisor::SupervisorSnapshot;
use zeron_proto::{CheckoutChangeRequestStatus, SidebarPreferences, TaskStep};

pub use tasks::{
    ApprovalView, HostRef, ItemSend, MissionStrip, NeedsYouEntry, NeedsYouKind, StepView,
    SupervisorHost, SupervisorPage, TaskDetail, TaskRow, TasksPage, WakeEntry,
};
pub(crate) use tasks::{Ask, detail as task_detail, strip as mission_strip};
pub(crate) use view::{DeriveContext, device_online};
pub use view::{
    DeviceView, FrontPage, PROJECT_COLOR_COUNT, ProjectRef, ProjectView, PullRequestGroups,
    SearchField, SearchHit, SectionView, SessionRow, WorkspaceSnapshot, project_color_index,
    relative_time_label,
};

use crate::{lock, read, write};

struct StateCache {
    generation: u64,
    state: Arc<WorkspaceState>,
    steps: Arc<Vec<TaskStep>>,
    prefs: Option<SidebarPreferences>,
}

pub(crate) struct WorkspaceStore {
    doc: Arc<Mutex<RegistryDoc>>,
    presence: Mutex<HashMap<String, i64>>,
    change_requests: Mutex<Vec<CheckoutChangeRequestStatus>>,
    supervisors: Mutex<BTreeMap<String, SupervisorSnapshot>>,
    item_sends: Mutex<HashMap<String, ItemSend>>,
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
            supervisors: Mutex::new(BTreeMap::new()),
            item_sends: Mutex::new(HashMap::new()),
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
        let (state, _, prefs) = self.state_and_steps();
        (state, prefs)
    }

    pub(crate) fn steps(&self) -> Arc<Vec<TaskStep>> {
        self.state_and_steps().1
    }

    fn state_and_steps(
        &self,
    ) -> (
        Arc<WorkspaceState>,
        Arc<Vec<TaskStep>>,
        Option<SidebarPreferences>,
    ) {
        let doc = lock(&self.doc);
        let generation = doc.generation();
        let mut cache = lock(&self.cache);
        if let Some(cached) = cache.as_ref()
            && cached.generation == generation
        {
            return (
                cached.state.clone(),
                cached.steps.clone(),
                cached.prefs.clone(),
            );
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
        let steps = Arc::new(doc.read_task_steps().unwrap_or_else(|err| {
            tracing::warn!(error = %err, "task steps read failed");
            Vec::new()
        }));
        let prefs = doc.sidebar_preferences();
        *cache = Some(StateCache {
            generation,
            state: state.clone(),
            steps: steps.clone(),
            prefs: prefs.clone(),
        });
        (state, steps, prefs)
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

    pub(crate) fn put_supervisor(&self, device_id: &str, snapshot: SupervisorSnapshot) {
        let open: Vec<String> = snapshot
            .needs_you
            .iter()
            .map(|row| format!("{device_id}/{}", row.key()))
            .collect();
        let prefix = format!("{device_id}/");
        lock(&self.item_sends).retain(|key, _| !key.starts_with(&prefix) || open.contains(key));
        lock(&self.supervisors).insert(device_id.to_owned(), snapshot);
    }

    pub(crate) fn set_item_send(&self, key: String, send: Option<ItemSend>) {
        let mut sends = lock(&self.item_sends);
        match send {
            Some(send) => sends.insert(key, send),
            None => sends.remove(&key),
        };
    }

    pub(crate) fn drop_supervisor(&self, device_id: &str) -> bool {
        lock(&self.supervisors).remove(device_id).is_some()
    }

    pub(crate) fn supervisors(&self) -> BTreeMap<String, SupervisorSnapshot> {
        lock(&self.supervisors).clone()
    }

    pub(crate) fn update_supervisor(
        &self,
        device_id: &str,
        f: impl FnOnce(&mut SupervisorSnapshot),
    ) -> bool {
        lock(&self.supervisors).get_mut(device_id).map(f).is_some()
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
        let (state, steps, prefs) = self.state_and_steps();
        let presence = self.presence();
        let change_requests = lock(&self.change_requests).clone();
        let supervisors = self.supervisors();
        let item_sends = lock(&self.item_sends).clone();
        let previous = self.snapshot();
        let mut next = view::derive(
            &state,
            prefs.as_ref(),
            &DeriveContext {
                self_device_id,
                now: chrono::Utc::now(),
                presence: &presence,
                change_requests: &change_requests,
                steps: &steps,
                supervisors: &supervisors,
                item_sends: &item_sends,
                send_states,
                asks,
                synced,
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
