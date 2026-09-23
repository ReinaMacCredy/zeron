use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use futures::future::BoxFuture;
use zeron_proto::computer::{
    ComputerSpace, ComputerSpaceErrorCode, ComputerSpaceInventory, ComputerSpaceReservation,
    ComputerWindow,
};

use super::ui_tree_targeting::ComputerTargetError;

const MAX_RESERVATIONS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerSpaceOwner {
    pub thread_id: String,
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerSpaceSnapshot {
    pub inventory: ComputerSpaceInventory,
    pub windows: Vec<ComputerWindow>,
}

fn same_owner(reservation: &ComputerSpaceReservation, owner: &ComputerSpaceOwner) -> bool {
    reservation.thread_id == owner.thread_id && reservation.turn_id == owner.turn_id
}

fn space_error(code: ComputerSpaceErrorCode, message: &str) -> ComputerTargetError {
    ComputerTargetError::space(code, message)
}

type ReadSnapshot<E> =
    Box<dyn Fn() -> BoxFuture<'static, Result<ComputerSpaceSnapshot, E>> + Send + Sync>;
type AssertActive<E> = Box<dyn Fn() -> Result<(), E> + Send + Sync>;

#[derive(Default)]
struct BrokerState {
    reservations: HashMap<String, ComputerSpaceReservation>,
    revision: u64,
    disposed: bool,
}

impl BrokerState {
    fn assert_unchanged(&self, revision: u64) -> Result<(), ComputerTargetError> {
        if self.revision == revision {
            return Ok(());
        }
        Err(space_error(
            ComputerSpaceErrorCode::ComputerSpaceReservationChanged,
            "Space reservations changed while reading the desktop. Observe the current reservation before continuing.",
        ))
    }

    fn reconcile(&mut self, inventory: &ComputerSpaceInventory) {
        let mut changed = 0;
        for entry in self.reservations.values_mut() {
            if entry.invalid_reason.is_some() {
                continue;
            }
            let mut matches = inventory.spaces.iter().filter(|s| s.id == entry.space_id);
            let space = match (matches.next(), matches.next()) {
                (Some(space), None) => Some(space),
                _ => None,
            };
            let reason = match space {
                Some(space) if space.current == Some(true) => {
                    Some(ComputerSpaceErrorCode::ComputerSpaceCurrent)
                }
                Some(space)
                    if inventory.complete
                        && space.uuid.as_deref() == Some(entry.space_uuid.as_str())
                        && space.display_id == entry.display_id
                        && space.current == Some(false) =>
                {
                    None
                }
                _ => Some(ComputerSpaceErrorCode::ComputerSpaceReservationChanged),
            };
            if reason.is_some() {
                entry.invalid_reason = reason;
                changed += 1;
            }
        }
        self.revision += changed;
    }

    fn reservation_for(&self, owner: &ComputerSpaceOwner) -> Option<ComputerSpaceReservation> {
        self.reservations
            .get(&owner.thread_id)
            .filter(|reservation| same_owner(reservation, owner))
            .cloned()
    }

    fn require_reservation(
        &self,
        owner: &ComputerSpaceOwner,
    ) -> Result<ComputerSpaceReservation, ComputerTargetError> {
        let reservation = self.reservation_for(owner).ok_or_else(|| {
            space_error(
                ComputerSpaceErrorCode::ComputerSpaceReservationRequired,
                "Reserve an existing noncurrent Space explicitly designated by the user before selecting its windows.",
            )
        })?;
        if let Some(reason) = reservation.invalid_reason {
            return Err(space_error(
                reason,
                "The reserved Space became current or its identity changed. No input was sent. Inspect Spaces and explicitly reserve a safe target again; Zeron will not move the user away.",
            ));
        }
        Ok(reservation)
    }

    fn reserves_any(&self, space_ids: &[u64]) -> bool {
        self.reservations
            .values()
            .any(|entry| entry.invalid_reason.is_none() && space_ids.contains(&entry.space_id))
    }
}

fn find_space(
    inventory: &ComputerSpaceInventory,
    id: u64,
) -> Result<&ComputerSpace, ComputerTargetError> {
    let mut matches = inventory.spaces.iter().filter(|space| space.id == id);
    match (matches.next(), matches.next()) {
        (Some(space), None) => Ok(space),
        _ => Err(space_error(
            ComputerSpaceErrorCode::ComputerSpaceNotFound,
            "That Space is absent or ambiguous in the current managed-display inventory. List Spaces again.",
        )),
    }
}

fn reservable_space(
    inventory: &ComputerSpaceInventory,
    id: u64,
) -> Result<&ComputerSpace, ComputerTargetError> {
    let space = find_space(inventory, id)?;
    if space.current == Some(true) {
        return Err(space_error(
            ComputerSpaceErrorCode::ComputerSpaceCurrent,
            "The user is currently on this Space. Zeron will not reserve or switch it. Choose an existing noncurrent Space explicitly designated by the user.",
        ));
    }
    if !inventory.complete
        || space.current.is_none()
        || space.uuid.as_deref().is_none_or(str::is_empty)
        || space.kind != zeron_proto::computer::ComputerSpaceKind::Desktop
    {
        return Err(space_error(
            ComputerSpaceErrorCode::ComputerSpaceIdentityUnproven,
            "This Space has incomplete identity, current-state information or unsupported fullscreen behavior. It cannot be reserved safely.",
        ));
    }
    Ok(space)
}

fn noncurrent_window<'a>(
    snapshot: &'a ComputerSpaceSnapshot,
    id: &str,
    space_id: u64,
) -> Result<&'a ComputerWindow, ComputerTargetError> {
    let window = snapshot.windows.iter().find(|window| window.id == id);
    let Some(window) = window.filter(|window| {
        window.pid.is_some()
            && window.space_ids.as_ref().is_some_and(|ids| ids.len() == 1)
            && window.on_current_space == Some(false)
    }) else {
        return Err(space_error(
            ComputerSpaceErrorCode::ComputerSpaceTargetMembershipUnproven,
            "This window's exact noncurrent Space membership is not proven. List windows again; do not switch Spaces as a workaround.",
        ));
    };
    let space_ids = window.space_ids.as_deref().unwrap_or_default();
    if space_ids[0] != space_id || window.current_space_id == Some(space_id) {
        return Err(space_error(
            ComputerSpaceErrorCode::ComputerSpaceTargetOutsideReservation,
            "This window is outside the reserved Space. Select an existing window in that Space; Zeron will not move it there.",
        ));
    }
    Ok(window)
}

pub struct ComputerSpaceBroker<E> {
    read_snapshot: ReadSnapshot<E>,
    assert_active: AssertActive<E>,
    state: Mutex<BrokerState>,
}

impl<E: From<ComputerTargetError>> ComputerSpaceBroker<E> {
    pub fn new(
        read_snapshot: impl Fn() -> BoxFuture<'static, Result<ComputerSpaceSnapshot, E>>
        + Send
        + Sync
        + 'static,
        assert_active: impl Fn() -> Result<(), E> + Send + Sync + 'static,
    ) -> Self {
        Self {
            read_snapshot: Box::new(read_snapshot),
            assert_active: Box::new(assert_active),
            state: Mutex::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, BrokerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn revision(&self) -> u64 {
        self.state().revision
    }

    fn closed(message: &str) -> E {
        space_error(
            ComputerSpaceErrorCode::ComputerSpaceReservationChanged,
            message,
        )
        .into()
    }

    async fn snapshot(&self) -> Result<ComputerSpaceSnapshot, E> {
        (self.assert_active)()?;
        if self.state().disposed {
            return Err(Self::closed("The Space broker has closed."));
        }
        let snapshot = (self.read_snapshot)().await?;
        (self.assert_active)()?;
        if self.state().disposed {
            return Err(Self::closed(
                "The Space broker closed while reading the desktop.",
            ));
        }
        Ok(snapshot)
    }

    pub async fn inspect(&self, space_id: Option<u64>) -> Result<ComputerSpaceSnapshot, E> {
        let snapshot = self.snapshot().await?;
        self.state().reconcile(&snapshot.inventory);
        let Some(space_id) = space_id else {
            return Ok(snapshot);
        };
        find_space(&snapshot.inventory, space_id)?;
        Ok(ComputerSpaceSnapshot {
            inventory: ComputerSpaceInventory {
                spaces: snapshot
                    .inventory
                    .spaces
                    .iter()
                    .filter(|space| space.id == space_id)
                    .cloned()
                    .collect(),
                ..snapshot.inventory.clone()
            },
            windows: snapshot
                .windows
                .into_iter()
                .filter(|window| {
                    window
                        .space_ids
                        .as_ref()
                        .is_some_and(|ids| ids.contains(&space_id))
                })
                .collect(),
        })
    }

    pub fn reservation_for(&self, owner: &ComputerSpaceOwner) -> Option<ComputerSpaceReservation> {
        self.state().reservation_for(owner)
    }

    pub async fn reserve(
        &self,
        owner: &ComputerSpaceOwner,
        space_id: u64,
        user_designated_space_ids: &[u64],
        window_id: Option<&str>,
        recheck_designation: Option<BoxFuture<'_, bool>>,
    ) -> Result<ComputerSpaceReservation, E> {
        if !user_designated_space_ids.contains(&space_id) {
            return Err(space_error(
                ComputerSpaceErrorCode::ComputerSpaceNotDesignated,
                &format!(
                    "The user has not designated this existing Space for this task. Ask them to say “Use Space ID {space_id} for this task.” A tool argument or full access is not that designation."
                ),
            )
            .into());
        }
        let revision = self.revision();
        let snapshot = self.snapshot().await?;
        let (reservation, admission_revision) = {
            let mut state = self.state();
            state.assert_unchanged(revision)?;
            state.reconcile(&snapshot.inventory);
            let space = reservable_space(&snapshot.inventory, space_id)?;
            let occupied = state.reservations.values().any(|entry| {
                entry.invalid_reason.is_none()
                    && entry.space_id == space_id
                    && !same_owner(entry, owner)
            });
            if occupied {
                return Err(space_error(
                    ComputerSpaceErrorCode::ComputerSpaceReserved,
                    "Another Zeron task reserved this Space. Use a different user-designated Space or wait for that task to finish.",
                )
                .into());
            }
            if !state.reservations.contains_key(&owner.thread_id)
                && state.reservations.len() >= MAX_RESERVATIONS
            {
                return Err(space_error(
                    ComputerSpaceErrorCode::ComputerSpaceReservationLimit,
                    "Too many tasks have Space reservations. Release an unused reservation first.",
                )
                .into());
            }
            let selected = window_id
                .map(|window_id| noncurrent_window(&snapshot, window_id, space_id))
                .transpose()?;
            let reservation = ComputerSpaceReservation {
                thread_id: owner.thread_id.clone(),
                turn_id: owner.turn_id.clone(),
                space_id,
                space_uuid: space.uuid.clone().unwrap_or_default(),
                display_id: space.display_id.clone(),
                selected_window_id: selected.map(|window| window.id.clone()),
                selected_pid: selected.and_then(|window| window.pid),
                invalid_reason: None,
            };
            (reservation, state.revision)
        };
        if let Some(recheck) = recheck_designation
            && !recheck.await
        {
            return Err(space_error(
                ComputerSpaceErrorCode::ComputerSpaceNotDesignated,
                "The user's current Space designation changed while reading the desktop. No reservation was created.",
            )
            .into());
        }
        (self.assert_active)()?;
        let mut state = self.state();
        state.assert_unchanged(admission_revision)?;
        state
            .reservations
            .insert(owner.thread_id.clone(), reservation.clone());
        state.revision += 1;
        Ok(reservation)
    }

    pub async fn select(
        &self,
        owner: &ComputerSpaceOwner,
        window_id: &str,
    ) -> Result<ComputerSpaceReservation, E> {
        let revision = self.revision();
        let snapshot = self.snapshot().await?;
        let selected = {
            let mut state = self.state();
            state.assert_unchanged(revision)?;
            state.reconcile(&snapshot.inventory);
            let reservation = state.require_reservation(owner)?;
            let window = noncurrent_window(&snapshot, window_id, reservation.space_id)?;
            ComputerSpaceReservation {
                selected_window_id: Some(window.id.clone()),
                selected_pid: window.pid,
                ..reservation
            }
        };
        (self.assert_active)()?;
        let mut state = self.state();
        state
            .reservations
            .insert(owner.thread_id.clone(), selected.clone());
        state.revision += 1;
        Ok(selected)
    }

    pub fn release(&self, thread_id: &str, turn_id: Option<&str>) {
        let mut state = self.state();
        state.revision += 1;
        if let Some(reservation) = state.reservations.get(thread_id)
            && turn_id.is_some()
            && reservation.turn_id.as_deref() != turn_id
        {
            return;
        }
        state.reservations.remove(thread_id);
    }

    fn any_reserved(&self) -> bool {
        !self.state().reservations.is_empty()
    }

    pub fn assert_native_launch_allowed(
        &self,
        thread_id: Option<&str>,
    ) -> Result<(), ComputerTargetError> {
        if thread_id.is_some() && self.any_reserved() {
            return Err(space_error(
                ComputerSpaceErrorCode::ComputerSpaceOperationUnsupported,
                "This backend cannot launch a native app into the reserved Space. Drive an existing window there in place, or use an isolated headless browser; no app was launched.",
            ));
        }
        Ok(())
    }

    pub fn assert_foreground_allowed(
        &self,
        thread_id: Option<&str>,
    ) -> Result<(), ComputerTargetError> {
        if thread_id.is_some() && self.any_reserved() {
            return Err(space_error(
                ComputerSpaceErrorCode::ComputerSpaceOperationUnsupported,
                "A foreground excursion or visible browser launch cannot be confined to a reserved Space. Drive the existing window in place or use an isolated headless browser; no desktop was activated.",
            ));
        }
        Ok(())
    }

    pub fn assert_target_bound(
        &self,
        thread_id: Option<&str>,
        window_id: Option<&str>,
    ) -> Result<(), ComputerTargetError> {
        if thread_id.is_some() && self.any_reserved() && window_id.is_none() {
            return Err(space_error(
                ComputerSpaceErrorCode::ComputerSpaceWindowNotSelected,
                "While a task holds a Space reservation, native input must name an exact current window. Select an existing window and pass its window_id; unscoped input was not sent.",
            ));
        }
        Ok(())
    }

    pub async fn assert_app_mutation_allowed(
        &self,
        owner: &ComputerSpaceOwner,
        pid: u32,
    ) -> Result<(), E> {
        {
            let state = self.state();
            if state.reservations.is_empty() {
                return Ok(());
            }
            if state.reservations.contains_key(&owner.thread_id) {
                return Err(space_error(
                    ComputerSpaceErrorCode::ComputerSpaceOperationUnsupported,
                    "Application-wide or window-visibility changes cannot be confined to a reserved Space. Drive the selected existing window in place; no app or window was changed.",
                )
                .into());
            }
        }
        let revision = self.revision();
        let snapshot = self.snapshot().await?;
        let mut state = self.state();
        state.assert_unchanged(revision)?;
        state.reconcile(&snapshot.inventory);
        for window in snapshot.windows.iter().filter(|w| w.pid == Some(pid)) {
            let Some(space_ids) = window.space_ids.as_deref().filter(|ids| !ids.is_empty()) else {
                return Err(space_error(
                    ComputerSpaceErrorCode::ComputerSpaceTargetMembershipUnproven,
                    "This application's Space membership is unknown while a task holds a reservation. No application-wide action was sent.",
                )
                .into());
            };
            if state.reserves_any(space_ids) {
                return Err(space_error(
                    ComputerSpaceErrorCode::ComputerSpaceReserved,
                    "This application has a window in another task's reserved Space. No application-wide action was sent.",
                )
                .into());
            }
        }
        Ok(())
    }

    pub async fn assert_window_allowed(
        &self,
        owner: &ComputerSpaceOwner,
        window: &ComputerWindow,
    ) -> Result<(), E> {
        if !self.any_reserved() {
            return Ok(());
        }
        let revision = self.revision();
        let snapshot = self.snapshot().await?;
        let mut state = self.state();
        state.assert_unchanged(revision)?;
        state.reconcile(&snapshot.inventory);
        if let Some(own) = state.reservations.get(&owner.thread_id).cloned() {
            if !same_owner(&own, owner) {
                return Err(space_error(
                    ComputerSpaceErrorCode::ComputerSpaceReservationChanged,
                    "This Space reservation belongs to another turn. Reserve it again for the current task.",
                )
                .into());
            }
            state.require_reservation(owner)?;
            let current = noncurrent_window(&snapshot, &window.id, own.space_id)?;
            if current.pid != window.pid
                || current.pid != own.selected_pid
                || Some(&current.id) != own.selected_window_id.as_ref()
            {
                return Err(space_error(
                    ComputerSpaceErrorCode::ComputerSpaceWindowNotSelected,
                    "Select this exact current window with computer_spaces before driving it. Space selection does not activate or move the window.",
                )
                .into());
            }
            return Ok(());
        }
        let space_ids = snapshot
            .windows
            .iter()
            .find(|candidate| candidate.id == window.id && candidate.pid == window.pid)
            .and_then(|current| current.space_ids.as_deref())
            .filter(|ids| !ids.is_empty());
        let Some(space_ids) = space_ids else {
            return Err(space_error(
                ComputerSpaceErrorCode::ComputerSpaceTargetMembershipUnproven,
                "The target's current Space membership is unknown while another task holds a reservation. Read fresh window state before choosing a target.",
            )
            .into());
        };
        if state.reserves_any(space_ids) {
            return Err(space_error(
                ComputerSpaceErrorCode::ComputerSpaceReserved,
                "This window belongs to a Space reserved by another Zeron task. No input was sent.",
            )
            .into());
        }
        Ok(())
    }

    pub fn dispose(&self) {
        let mut state = self.state();
        state.disposed = true;
        state.revision += 1;
        state.reservations.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::FutureExt;
    use tokio::sync::oneshot;
    use zeron_proto::computer::{ComputerSpaceKind, ComputerSpaceSource};

    use super::*;
    use crate::computer::ui_tree_targeting::ComputerTargetErrorCode;

    type Broker = ComputerSpaceBroker<ComputerTargetError>;
    use ComputerSpaceErrorCode as Code;

    fn owner() -> ComputerSpaceOwner {
        ComputerSpaceOwner {
            thread_id: "a".into(),
            turn_id: Some("one".into()),
        }
    }

    fn other() -> ComputerSpaceOwner {
        ComputerSpaceOwner {
            thread_id: "b".into(),
            turn_id: Some("two".into()),
        }
    }

    fn window() -> ComputerWindow {
        ComputerWindow {
            id: "cua:10:20".into(),
            title: "Fixture".into(),
            app_name: Some("Fixture".into()),
            pid: Some(10),
            bounds: None,
            focused: false,
            keyboard_focused: None,
            active: None,
            minimized: false,
            visible: false,
            space_ids: Some(vec![2]),
            current_space_id: Some(1),
            on_current_space: Some(false),
            stacking_index: None,
            occluded_by: None,
        }
    }

    fn inventory(current: u64) -> ComputerSpaceInventory {
        ComputerSpaceInventory {
            source: ComputerSpaceSource::MacosManagedSpaces,
            complete: true,
            spaces: [1, 2, 3]
                .into_iter()
                .map(|id| ComputerSpace {
                    id,
                    uuid: Some(format!("uuid-{id}")),
                    display_id: "display-a".into(),
                    kind: ComputerSpaceKind::Desktop,
                    current: Some(id == current),
                })
                .collect(),
        }
    }

    fn snapshot(
        inventory: ComputerSpaceInventory,
        windows: Vec<ComputerWindow>,
    ) -> ComputerSpaceSnapshot {
        ComputerSpaceSnapshot { inventory, windows }
    }

    fn patched(patch: impl Fn(&mut ComputerSpace)) -> ComputerSpaceInventory {
        let mut value = inventory(1);
        for space in &mut value.spaces {
            if space.id == 2 {
                patch(space);
            }
        }
        value
    }

    #[derive(Default)]
    struct Desktop {
        snapshot: Mutex<Option<ComputerSpaceSnapshot>>,
        gates: Mutex<VecDeque<oneshot::Receiver<ComputerSpaceSnapshot>>>,
        reads: AtomicUsize,
    }

    impl Desktop {
        fn set(&self, value: ComputerSpaceSnapshot) {
            *self.snapshot.lock().unwrap() = Some(value);
        }

        fn current(&self) -> ComputerSpaceSnapshot {
            self.snapshot.lock().unwrap().clone().unwrap()
        }

        fn hold(&self) -> oneshot::Sender<ComputerSpaceSnapshot> {
            let (sender, receiver) = oneshot::channel();
            self.gates.lock().unwrap().push_back(receiver);
            sender
        }
    }

    fn setup() -> (Broker, Arc<Desktop>) {
        let desktop = Arc::new(Desktop::default());
        desktop.set(snapshot(inventory(1), vec![window()]));
        let reader = desktop.clone();
        let broker = Broker::new(
            move || {
                reader.reads.fetch_add(1, Ordering::SeqCst);
                let gate = reader.gates.lock().unwrap().pop_front();
                let current = reader.current();
                async move {
                    Ok(match gate {
                        Some(gate) => gate.await.unwrap(),
                        None => current,
                    })
                }
                .boxed()
            },
            || Ok(()),
        );
        (broker, desktop)
    }

    fn code<T: std::fmt::Debug>(result: Result<T, ComputerTargetError>) -> ComputerTargetErrorCode {
        result.unwrap_err().code
    }

    const fn space(code: Code) -> ComputerTargetErrorCode {
        ComputerTargetErrorCode::Space(code)
    }

    #[tokio::test]
    async fn lists_native_empty_spaces_and_reserves_selects_without_any_desktop_mutation() {
        let (broker, _) = setup();
        assert_eq!(
            broker.inspect(None).await.unwrap().inventory.spaces.len(),
            3
        );
        let reservation = broker
            .reserve(&owner(), 2, &[2], Some(&window().id), None)
            .await
            .unwrap();
        assert_eq!(reservation.space_id, 2);
        assert_eq!(reservation.selected_window_id.as_deref(), Some("cua:10:20"));
        assert_eq!(reservation.selected_pid, Some(10));
        assert!(
            broker
                .assert_window_allowed(&owner(), &window())
                .await
                .is_ok()
        );
        assert!(broker.inspect(Some(3)).await.unwrap().windows.is_empty());
    }

    #[tokio::test]
    async fn requires_user_designation_before_reading_or_reserving() {
        let (broker, desktop) = setup();
        assert_eq!(
            code(
                broker
                    .reserve(&owner(), 2, &[], Some(&window().id), None)
                    .await
            ),
            space(Code::ComputerSpaceNotDesignated)
        );
        assert_eq!(desktop.reads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn refuses_current_unproven_and_unsupported_space_identities() {
        let (broker, desktop) = setup();
        assert_eq!(
            code(broker.reserve(&owner(), 1, &[1], None, None).await),
            space(Code::ComputerSpaceCurrent)
        );
        let patches: [fn(&mut ComputerSpace); 3] = [
            |s| s.uuid = None,
            |s| s.current = None,
            |s| s.kind = ComputerSpaceKind::Fullscreen,
        ];
        for patch in patches {
            desktop.set(snapshot(patched(patch), vec![window()]));
            assert_eq!(
                code(broker.reserve(&owner(), 2, &[2], None, None).await),
                space(Code::ComputerSpaceIdentityUnproven)
            );
        }
    }

    #[tokio::test]
    async fn excludes_other_tasks_and_requires_one_exact_noncurrent_window() {
        let (broker, desktop) = setup();
        broker.reserve(&owner(), 2, &[2], None, None).await.unwrap();
        assert_eq!(
            code(broker.reserve(&other(), 2, &[2], None, None).await),
            space(Code::ComputerSpaceReserved)
        );
        assert_eq!(
            code(broker.assert_window_allowed(&other(), &window()).await),
            space(Code::ComputerSpaceReserved)
        );
        assert_eq!(
            code(broker.assert_window_allowed(&owner(), &window()).await),
            space(Code::ComputerSpaceWindowNotSelected)
        );
        for candidate in [
            ComputerWindow {
                space_ids: Some(vec![1, 2]),
                ..window()
            },
            ComputerWindow {
                on_current_space: Some(true),
                ..window()
            },
            ComputerWindow {
                space_ids: Some(vec![]),
                ..window()
            },
        ] {
            desktop.set(snapshot(inventory(1), vec![candidate.clone()]));
            assert_eq!(
                code(broker.select(&owner(), &candidate.id).await),
                space(Code::ComputerSpaceTargetMembershipUnproven)
            );
        }
    }

    #[tokio::test]
    async fn invalidates_on_current_space_entry_and_does_not_silently_resume_after_departure() {
        let (broker, desktop) = setup();
        broker
            .reserve(&owner(), 2, &[2], Some(&window().id), None)
            .await
            .unwrap();
        desktop.set(snapshot(
            inventory(2),
            vec![ComputerWindow {
                on_current_space: Some(true),
                ..window()
            }],
        ));
        assert_eq!(
            code(broker.assert_window_allowed(&owner(), &window()).await),
            space(Code::ComputerSpaceCurrent)
        );
        desktop.set(snapshot(inventory(1), vec![window()]));
        assert_eq!(
            code(broker.assert_window_allowed(&owner(), &window()).await),
            space(Code::ComputerSpaceCurrent)
        );
        broker
            .reserve(&owner(), 2, &[2], Some(&window().id), None)
            .await
            .unwrap();
        assert!(
            broker
                .assert_window_allowed(&owner(), &window())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn invalidates_session_id_reuse_or_display_churn() {
        let patches: [fn(&mut ComputerSpace); 2] = [
            |s| s.uuid = Some("new-uuid".into()),
            |s| s.display_id = "display-b".into(),
        ];
        for patch in patches {
            let (broker, desktop) = setup();
            broker
                .reserve(&owner(), 2, &[2], Some(&window().id), None)
                .await
                .unwrap();
            desktop.set(snapshot(patched(patch), vec![window()]));
            assert_eq!(
                code(broker.assert_window_allowed(&owner(), &window()).await),
                space(Code::ComputerSpaceReservationChanged)
            );
        }
    }

    #[tokio::test]
    async fn rejects_a_replaced_pid_and_a_window_moved_out_of_the_reservation() {
        let (broker, desktop) = setup();
        broker
            .reserve(&owner(), 2, &[2], Some(&window().id), None)
            .await
            .unwrap();
        desktop.set(snapshot(
            inventory(1),
            vec![ComputerWindow {
                pid: Some(11),
                ..window()
            }],
        ));
        assert_eq!(
            code(broker.assert_window_allowed(&owner(), &window()).await),
            space(Code::ComputerSpaceWindowNotSelected)
        );
        desktop.set(snapshot(
            inventory(1),
            vec![ComputerWindow {
                space_ids: Some(vec![3]),
                ..window()
            }],
        ));
        assert_eq!(
            code(broker.assert_window_allowed(&owner(), &window()).await),
            space(Code::ComputerSpaceTargetOutsideReservation)
        );
    }

    #[tokio::test]
    async fn fences_an_old_noncurrent_read_after_a_newer_observation_invalidates_the_reservation() {
        let (broker, desktop) = setup();
        broker
            .reserve(&owner(), 2, &[2], Some(&window().id), None)
            .await
            .unwrap();
        let old = desktop.current();
        let gate = desktop.hold();
        let owner = owner();
        let id = window().id;
        let mut pending = std::pin::pin!(broker.reserve(&owner, 2, &[2], Some(&id), None));
        assert!(futures::poll!(pending.as_mut()).is_pending());
        desktop.set(snapshot(inventory(2), vec![window()]));
        broker.inspect(None).await.unwrap();
        gate.send(old).unwrap();
        assert_eq!(
            code(pending.await),
            space(Code::ComputerSpaceReservationChanged)
        );
        assert_eq!(
            broker.reservation_for(&owner).unwrap().invalid_reason,
            Some(Code::ComputerSpaceCurrent)
        );
    }

    #[tokio::test]
    async fn turn_teardown_fences_pending_replacement_without_releasing_a_different_stored_turn() {
        let (broker, desktop) = setup();
        broker
            .reserve(&owner(), 2, &[2], Some(&window().id), None)
            .await
            .unwrap();
        let gate = desktop.hold();
        let next = ComputerSpaceOwner {
            turn_id: Some("next".into()),
            ..owner()
        };
        let mut pending = std::pin::pin!(broker.reserve(&next, 3, &[3], None, None));
        assert!(futures::poll!(pending.as_mut()).is_pending());
        broker.release("a", Some("next"));
        assert!(broker.reservation_for(&owner()).is_some());
        gate.send(desktop.current()).unwrap();
        assert_eq!(
            code(pending.await),
            space(Code::ComputerSpaceReservationChanged)
        );
        broker.release("a", Some("one"));
        assert!(broker.reservation_for(&owner()).is_none());
    }

    #[tokio::test]
    async fn disposal_and_latest_user_revocation_prevent_pending_reservation_commit() {
        let (broker, desktop) = setup();
        assert_eq!(
            code(
                broker
                    .reserve(
                        &owner(),
                        2,
                        &[2],
                        Some(&window().id),
                        Some(async { false }.boxed())
                    )
                    .await
            ),
            space(Code::ComputerSpaceNotDesignated)
        );
        let gate = desktop.hold();
        let owner = owner();
        let mut pending = std::pin::pin!(broker.reserve(&owner, 2, &[2], None, None));
        assert!(futures::poll!(pending.as_mut()).is_pending());
        broker.dispose();
        gate.send(desktop.current()).unwrap();
        assert_eq!(
            code(pending.await),
            space(Code::ComputerSpaceReservationChanged)
        );
    }

    #[tokio::test]
    async fn refuses_native_launch_app_wide_changes_and_unscoped_input_while_reserved() {
        let (broker, _) = setup();
        broker
            .reserve(&owner(), 2, &[2], Some(&window().id), None)
            .await
            .unwrap();
        let unsupported = space(Code::ComputerSpaceOperationUnsupported);
        assert_eq!(
            code(broker.assert_native_launch_allowed(Some("a"))),
            unsupported
        );
        assert_eq!(
            code(broker.assert_native_launch_allowed(Some("b"))),
            unsupported
        );
        assert_eq!(
            code(broker.assert_foreground_allowed(Some("b"))),
            unsupported
        );
        assert_eq!(
            code(broker.assert_target_bound(Some("a"), None)),
            space(Code::ComputerSpaceWindowNotSelected)
        );
        assert_eq!(
            code(broker.assert_app_mutation_allowed(&owner(), 10).await),
            unsupported
        );
        assert_eq!(
            code(broker.assert_app_mutation_allowed(&other(), 10).await),
            space(Code::ComputerSpaceReserved)
        );
    }

    #[tokio::test]
    async fn does_no_inventory_work_on_ordinary_input_when_no_task_has_a_reservation() {
        let (broker, desktop) = setup();
        broker
            .assert_window_allowed(&owner(), &window())
            .await
            .unwrap();
        broker
            .assert_app_mutation_allowed(&owner(), 10)
            .await
            .unwrap();
        broker.assert_native_launch_allowed(Some("a")).unwrap();
        broker.assert_foreground_allowed(Some("a")).unwrap();
        broker.assert_target_bound(Some("a"), None).unwrap();
        assert_eq!(desktop.reads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn ambiguous_duplicate_inventory_cannot_preserve_a_reservation() {
        let (broker, desktop) = setup();
        broker
            .reserve(&owner(), 2, &[2], Some(&window().id), None)
            .await
            .unwrap();
        let mut value = inventory(1);
        let duplicate = ComputerSpace {
            current: Some(true),
            ..value.spaces[1].clone()
        };
        value.spaces.push(duplicate);
        desktop.set(snapshot(value, vec![window()]));
        assert_eq!(
            code(broker.assert_window_allowed(&owner(), &window()).await),
            space(Code::ComputerSpaceReservationChanged)
        );
    }
}
