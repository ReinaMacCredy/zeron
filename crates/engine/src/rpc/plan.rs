use super::tasks::{failed, refused};
use super::*;
use zeron_proto::{Chat, ChatPlan, RouteTarget, TaskStep, TaskStepPatch};

const MAX_RESOURCE_KEYS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResourceKey {
    Port(u16),
    Slot(String),
    Path(String),
}

impl ResourceKey {
    fn parse(key: &str) -> Option<Self> {
        let (kind, value) = key.trim().split_once(':')?;
        let value = value.trim();
        match kind.trim() {
            "port" => value.parse().ok().filter(|port| *port > 0).map(Self::Port),
            "slot" => (!value.is_empty()
                && value.len() <= 64
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
            .then(|| Self::Slot(value.to_string())),
            "path" => {
                if value.starts_with('/') || value.starts_with('~') {
                    return None;
                }
                let parts: Vec<&str> = value
                    .split('/')
                    .filter(|part| !part.is_empty() && *part != ".")
                    .collect();
                (!parts.is_empty() && !parts.contains(&"..")).then(|| Self::Path(parts.join("/")))
            }
            _ => None,
        }
    }
}

impl std::fmt::Display for ResourceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Port(port) => write!(f, "port:{port}"),
            Self::Slot(name) => write!(f, "slot:{name}"),
            Self::Path(path) => write!(f, "path:{path}"),
        }
    }
}

fn checked_keys(keys: &[String]) -> Result<Vec<String>, String> {
    let mut parsed: Vec<ResourceKey> = Vec::new();
    for key in keys {
        let key = ResourceKey::parse(key).ok_or_else(|| {
            format!("\"{key}\" is not a resource: use port:<1-65535>, slot:<name> or path:<path inside the project>")
        })?;
        if !parsed.contains(&key) {
            parsed.push(key);
        }
    }
    if parsed.len() > MAX_RESOURCE_KEYS {
        return Err(format!(
            "A plan declares at most {MAX_RESOURCE_KEYS} resources"
        ));
    }
    Ok(parsed.iter().map(ResourceKey::to_string).collect())
}

pub(super) fn written(error: crate::EngineError) -> RpcError {
    match error {
        crate::EngineError::Doc(zeron_doc::DocError::Schema(message)) => refused(&message),
        other => failed(other),
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlanEdit {
    #[serde(default)]
    pub step_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub images: Option<Vec<String>>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default, deserialize_with = "zeron_proto::present_nullable")]
    pub target: Option<Option<RouteTarget>>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlannedStep {
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub target: Option<RouteTarget>,
}

const REORDER_REFUSED: &str =
    "Only steps that have not started can move, and only below the last started step";

fn new_images(before: &[String], after: &[String]) -> Vec<String> {
    after
        .iter()
        .filter(|path| !before.contains(path))
        .cloned()
        .collect()
}

fn image_change(before: &[String], after: &[String]) -> Option<String> {
    let added = new_images(before, after).len();
    let removed = before.iter().filter(|path| !after.contains(path)).count();
    match (added, removed) {
        (0, 0) => None,
        (1, 0) => Some("1 new image".into()),
        (n, 0) => Some(format!("{n} new images")),
        (0, 1) => Some("1 image removed".into()),
        (0, n) => Some(format!("{n} images removed")),
        (added, removed) => Some(format!("{added} new and {removed} removed images")),
    }
}

fn count_images(n: usize) -> String {
    match n {
        1 => "1 image".into(),
        n => format!("{n} images"),
    }
}

fn canonical_role(name: &str) -> Option<String> {
    crate::routing::Role::parse(name).map(|role| role.name().to_string())
}

fn namesake<'a, T: 'a>(
    before: impl IntoIterator<Item = &'a TaskStep>,
    title: &str,
    field: impl Fn(&'a TaskStep) -> &'a Option<T>,
) -> Option<&'a T> {
    let mut namesakes = before
        .into_iter()
        .filter(|old| old.title.trim() == title.trim())
        .filter_map(|old| field(old).as_ref());
    match (namesakes.next(), namesakes.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

fn target_change(target: Option<&RouteTarget>) -> String {
    let Some(target) = target else {
        return "has no agent of its own again: it runs where its role routes, else on yours."
            .into();
    };
    let model = target
        .model
        .as_deref()
        .map_or(String::new(), |model| format!(" {model}"));
    let reasoning = target.reasoning.map_or(String::new(), |level| {
        format!(" at {} reasoning", level.label())
    });
    format!(
        "runs on {}{model}{reasoning}: start it with delegate_task and its stepId, without target or profile.",
        target.harness.display_name()
    )
}

fn carry_choices(before: &[TaskStep], steps: &mut [TaskStep]) {
    for step in steps.iter_mut() {
        if step.role.is_none() {
            step.role = namesake(before, &step.title, |old| &old.role).cloned();
        }
        if step.target.is_none() {
            step.target = namesake(before, &step.title, |old| &old.target).cloned();
        }
    }
}

fn carry_images(
    before: &[TaskStep],
    started: impl Fn(&TaskStep) -> bool,
    steps: &mut [TaskStep],
) -> (Vec<String>, Vec<String>) {
    let replaced: Vec<&TaskStep> = before.iter().filter(|step| !started(step)).collect();
    let mut moved = Vec::new();
    let mut orphans = Vec::new();
    for old in replaced.iter().filter(|step| !step.images.is_empty()) {
        let title = old.title.trim();
        let namesakes = replaced
            .iter()
            .filter(|step| step.title.trim() == title)
            .count();
        let mut matches = steps.iter_mut().filter(|step| step.title.trim() == title);
        let what = count_images(old.images.len());
        match (namesakes, matches.next(), matches.next()) {
            (1, Some(new), None) => {
                new.images = old.images.clone();
                moved.push(format!(
                    "Step \"{}\" (stepId {}) had {what}, now on the new step with its title (stepId {}).",
                    old.title, old.id, new.id
                ));
            }
            _ => {
                orphans.extend(old.images.iter().cloned());
                moved.push(format!(
                    "Step \"{}\" (stepId {}) had {what}, now on the overview: no single new step has its title.",
                    old.title, old.id
                ));
            }
        }
    }
    (moved, orphans)
}

fn reorder_keys(
    steps: &[(&TaskStep, bool)],
    order: &[String],
    mut nonce: impl FnMut() -> String,
) -> Result<Vec<(String, String)>, &'static str> {
    let listed: Vec<(&TaskStep, bool)> = order
        .iter()
        .filter_map(|id| steps.iter().find(|(step, _)| &step.id == id).copied())
        .collect();
    let unique: std::collections::HashSet<&str> = order.iter().map(String::as_str).collect();
    if order.len() != steps.len() || listed.len() != steps.len() || unique.len() != steps.len() {
        return Err("The new order must list every remaining step once");
    }
    let key = |step: &TaskStep| (step.order_key.is_empty(), step.order_key.clone());
    let fixed = listed
        .iter()
        .rposition(|(_, started)| *started)
        .map_or(0, |last| last + 1);
    if listed[..fixed]
        .windows(2)
        .any(|pair| key(pair[0].0) >= key(pair[1].0))
    {
        return Err(REORDER_REFUSED);
    }
    let floor = fixed.checked_sub(1).map(|ix| key(listed[ix].0));
    let tail = &listed[fixed..];
    let eligible = |ix: usize| floor.as_ref().is_none_or(|floor| key(tail[ix].0) > *floor);
    let mut best: Vec<(usize, Option<usize>)> = Vec::with_capacity(tail.len());
    for ix in 0..tail.len() {
        let run = (0..ix)
            .filter(|&before| eligible(before) && key(tail[before].0) < key(tail[ix].0))
            .max_by_key(|&before| best[before].0)
            .map_or((0, None), |before| (best[before].0, Some(before)));
        best.push(if eligible(ix) {
            (run.0 + 1, run.1)
        } else {
            (0, None)
        });
    }
    let mut kept = vec![false; tail.len()];
    let mut at = (0..tail.len())
        .filter(|&ix| best[ix].0 > 0)
        .max_by_key(|&ix| best[ix].0);
    while let Some(ix) = at {
        kept[ix] = true;
        at = best[ix].1;
    }
    let mut lower = fixed
        .checked_sub(1)
        .map(|ix| listed[ix].0.order_key.clone());
    let mut moves = Vec::new();
    for (ix, (step, _)) in tail.iter().enumerate() {
        if kept[ix] {
            lower = Some(step.order_key.clone());
            continue;
        }
        let upper = (ix + 1..tail.len())
            .find(|&next| kept[next])
            .map(|next| tail[next].0.order_key.as_str());
        let key = zeron_proto::pin_order_key_between(lower.as_deref(), upper, &nonce())
            .map_err(|_| "This plan's steps cannot be reordered")?;
        moves.push((step.id.clone(), key.clone()));
        lower = Some(key);
    }
    Ok(moves)
}

fn too_long(text: &str, max: usize) -> bool {
    text.chars().count() > max
}

const OVERVIEW_TOO_LONG: &str = "The overview is limited to 10,000 characters";

impl EngineRpc {
    fn chat_or_refuse(&self, chat_id: &str) -> Result<Chat, RpcError> {
        self.workspace
            .chat(chat_id)
            .map_err(failed)?
            .ok_or_else(|| refused("That chat no longer exists"))
    }

    pub(super) fn step_or_refuse(&self, step_id: &str) -> Result<TaskStep, RpcError> {
        self.workspace
            .task_step(step_id)
            .ok_or_else(|| refused("Step no longer exists"))
    }

    fn check_step_threads(&self, lead: &str, threads: &[Option<String>]) -> Result<(), RpcError> {
        if threads.iter().all(Option::is_none) {
            return Ok(());
        }
        let steps = self.workspace.read_task_steps().map_err(failed)?;
        for (n, thread) in threads.iter().enumerate() {
            let Some(thread) = thread.as_deref() else {
                continue;
            };
            if threads[..n].iter().flatten().any(|seen| seen == thread) {
                return Err(refused(&format!("threadId {thread} is on two steps")));
            }
            let chat = self
                .workspace
                .chat(thread)
                .map_err(failed)?
                .ok_or_else(|| refused(&format!("threadId {thread} no longer exists")))?;
            if chat.delegated_by() != Some(lead) {
                return Err(refused(
                    &if chat.created_by_chat_id.as_deref() == Some(lead) {
                        format!(
                            "threadId {thread} is a peer thread, not one this chat owns; run the step on its own thread with delegate_task and its stepId"
                        )
                    } else {
                        format!("threadId {thread} is not a thread this chat owns")
                    },
                ));
            }
            if let Some(holder) = steps.iter().find(|step| {
                !step.deleted && step.outcome.is_none() && step.chat_id.as_deref() == Some(thread)
            }) {
                return Err(refused(&format!(
                    "threadId {thread} already runs step \"{}\"",
                    holder.title
                )));
            }
        }
        Ok(())
    }

    pub(super) fn plan_steps(
        &self,
        chat_id: &str,
        steps: Vec<PlannedStep>,
        overview: Option<String>,
        resources: Option<Vec<String>>,
        from_chat_id: Option<&str>,
    ) -> Result<Vec<String>, RpcError> {
        if from_chat_id.is_some_and(|from| from != chat_id) {
            return Err(refused("A chat writes only its own plan"));
        }
        let chat = self.chat_or_refuse(chat_id)?;
        if chat.sidechat_source_chat_id.is_some() {
            return Err(refused("A side chat cannot hold a plan"));
        }
        if overview
            .as_deref()
            .is_some_and(|text| too_long(text, zeron_doc::TASK_NOTES_MAX_CHARS))
        {
            return Err(refused(OVERVIEW_TOO_LONG));
        }
        if steps.iter().any(|step| {
            step.role
                .as_deref()
                .is_some_and(|role| canonical_role(role).is_none())
        }) {
            return Err(refused(&format!(
                "A step's role must be one of {}",
                crate::routing::Role::names()
            )));
        }
        let threads: Vec<Option<String>> =
            steps.iter().map(|step| step.thread_id.clone()).collect();
        self.check_step_threads(chat_id, &threads)?;
        let resources = resources
            .map(|keys| checked_keys(&keys))
            .transpose()
            .map_err(|message| refused(&message))?;
        let before = self.workspace.steps_of(chat_id);
        let now = chrono::Utc::now();
        let mut steps: Vec<TaskStep> = steps
            .into_iter()
            .map(|step| TaskStep {
                id: crate::new_id(),
                owner_chat_id: chat_id.to_string(),
                title: step.title,
                notes: step.notes,
                order_key: String::new(),
                turn: None,
                chat_id: step.thread_id,
                outcome: None,
                created_at: now,
                updated_at: now,
                deleted: false,
                images: Vec::new(),
                role: step.role.as_deref().and_then(canonical_role),
                route: None,
                target: step.target,
            })
            .collect();
        carry_choices(&before, &mut steps);
        let (moved, orphans) = carry_images(
            &before,
            |step| self.workspace.step_started(step),
            &mut steps,
        );
        let _plans = self.plan_writes();
        let mut plan = self.chat_or_refuse(chat_id)?.plan.unwrap_or_default();
        let mission = chat.mission();
        if let Some(mission) = mission {
            plan.same_agents = mission.same_agents;
        }
        if let Some(overview) = overview {
            plan.overview = overview;
        }
        if let Some(resources) = resources {
            plan.resources = resources;
        }
        let mut left_out = 0;
        for image in orphans {
            if plan.images.contains(&image) {
                continue;
            }
            if plan.images.len() < zeron_doc::TASK_IMAGES_MAX {
                plan.images.push(image);
            } else {
                left_out += 1;
            }
        }
        if left_out > 0 {
            return Err(refused(&format!(
                "This plan replaces steps whose images do not fit on the overview, which holds {}: {left_out} would be lost. Keep those steps' titles so their images stay with them, or ask the person which images to drop.",
                zeron_doc::TASK_IMAGES_MAX,
            )));
        }
        let mission_turn = !steps.is_empty() && self.sessions.mission_turn_open(chat_id);
        let pending = mission_turn && mission.is_some_and(|mission| mission.approve);
        self.workspace
            .write_plan(chat_id, &plan, &steps, pending)
            .map_err(written)?;
        if mission_turn {
            self.sessions.set_mission_turn(chat_id, None);
        }
        Ok(moved)
    }

    pub(super) fn update_task_step(
        &self,
        step_id: &str,
        patch: &TaskStepPatch,
        from_chat_id: Option<&str>,
    ) -> Result<(), RpcError> {
        let step = self.step_or_refuse(step_id)?;
        if from_chat_id.is_some_and(|from| from != step.owner_chat_id) {
            return Err(refused("Only the chat whose plan holds this step marks it"));
        }
        match self
            .workspace
            .update_task_step(step_id, patch)
            .map_err(failed)?
        {
            true => Ok(()),
            false => Err(refused("Step no longer exists")),
        }
    }

    async fn tell_lead(
        &self,
        lead: &str,
        text: &str,
        images: Vec<String>,
    ) -> Result<String, RpcError> {
        let (text, images) = crate::doc_host::images_here(text, images);
        self.doc_host
            .queue_thread_message_with(lead, None, &text, images)
            .await
            .map_err(failed)
    }

    pub(super) async fn approve_plan(&self, chat_id: &str) -> Result<String, RpcError> {
        let chat = self.chat_or_refuse(chat_id)?;
        if !chat.plan_pending {
            return Err(refused("This plan is not waiting for approval"));
        }
        if self.workspace.steps_of(chat_id).is_empty() {
            return Err(refused("The plan has no steps yet"));
        }
        self.workspace
            .set_plan_pending(chat_id, false)
            .map_err(failed)?;
        let sent = self
            .tell_lead(chat_id, zeron_proto::PLAN_APPROVED_PROMPT, Vec::new())
            .await;
        if sent.is_err() {
            self.workspace.set_plan_pending(chat_id, true).ok();
        }
        sent
    }

    pub(super) async fn retry_step(&self, step_id: &str) -> Result<(), RpcError> {
        let step = self.step_or_refuse(step_id)?;
        let lead = self.chat_or_refuse(&step.owner_chat_id)?;
        let (title, id) = (&step.title, &step.id);
        let text = match step.chat_id {
            Some(thread) => format!(
                "Step \"{title}\" needs another run (stepId {id}). Check its thread {thread} with read_thread, then send it what to fix with send_message_to_thread, or start the step again with delegate_task and stepId {id}."
            ),
            None => format!(
                "Step \"{title}\" has not started (stepId {id}). Run it with delegate_task and stepId {id}."
            ),
        };
        self.tell_lead(&lead.id, &text, Vec::new()).await.map(drop)
    }

    fn unstarted(&self, step: &TaskStep) -> bool {
        !self.workspace.step_started(step)
    }

    pub(super) async fn edit_plan(
        &self,
        chat_id: &str,
        overview: Option<String>,
        overview_images: Option<Vec<String>>,
        steps: Vec<PlanEdit>,
        remove_step_ids: &[String],
        order: &[String],
    ) -> Result<Option<String>, RpcError> {
        let Some((text, attached)) = self.write_plan_edit(
            chat_id,
            overview,
            overview_images,
            steps,
            remove_step_ids,
            order,
        )?
        else {
            return Ok(None);
        };
        self.tell_lead(chat_id, &text, attached).await.map(Some)
    }

    /// Every edit is checked before the first write, so a refusal applies nothing.
    fn write_plan_edit(
        &self,
        chat_id: &str,
        overview: Option<String>,
        overview_images: Option<Vec<String>>,
        steps: Vec<PlanEdit>,
        remove_step_ids: &[String],
        order: &[String],
    ) -> Result<Option<(String, Vec<String>)>, RpcError> {
        let _plans = self.plan_writes();
        let chat = self.chat_or_refuse(chat_id)?;
        let plan = chat.plan.clone().unwrap_or_default();
        let current = self.workspace.steps_of(chat_id);
        let find = |id: &str| {
            current
                .iter()
                .find(|step| step.id == id)
                .ok_or_else(|| refused("Step no longer exists"))
        };
        let mut removed = Vec::new();
        for id in remove_step_ids {
            let step = find(id)?;
            if !self.unstarted(step) {
                return Err(refused(&format!(
                    "Step \"{}\" has started, so it cannot be removed; skip it instead",
                    step.title
                )));
            }
            removed.push(step);
        }
        if overview
            .as_deref()
            .is_some_and(|text| too_long(text, zeron_doc::TASK_NOTES_MAX_CHARS))
        {
            return Err(refused(OVERVIEW_TOO_LONG));
        }
        let images_ok = |images: &Option<Vec<String>>| {
            images
                .as_deref()
                .map_or(Ok(()), zeron_doc::check_images)
                .map_err(|error| match error {
                    zeron_doc::DocError::Schema(message) => refused(&message),
                    other => failed(other.into()),
                })
        };
        images_ok(&overview_images)?;
        for edit in &steps {
            images_ok(&edit.images)?;
            let title = edit.title.trim();
            if title.is_empty() || too_long(title, zeron_doc::TASK_TITLE_MAX_CHARS) {
                return Err(refused("A step needs a title of up to 500 characters"));
            }
            if too_long(&edit.notes, zeron_doc::TASK_NOTES_MAX_CHARS) {
                return Err(refused("A step's spec is limited to 10,000 characters"));
            }
            if let Some(id) = &edit.step_id {
                find(id)?;
                if remove_step_ids.contains(id) {
                    return Err(refused("A step cannot be both edited and removed"));
                }
            }
        }
        let mut moves = Vec::new();
        if !order.is_empty() {
            let mut remaining = Vec::new();
            for step in current
                .iter()
                .filter(|step| !remove_step_ids.contains(&step.id))
            {
                remaining.push((step, !self.unstarted(step)));
            }
            moves = reorder_keys(&remaining, order, crate::new_id).map_err(refused)?;
        }
        let mut changes = Vec::new();
        let overview = overview.filter(|overview| *overview != plan.overview);
        let overview_images = overview_images.filter(|images| *images != plan.images);
        let attached = overview_images
            .as_deref()
            .map(|images| new_images(&plan.images, images))
            .unwrap_or_default();
        if overview.is_some() {
            changes.push("The overview changed.".to_string());
        }
        if let Some(what) = overview_images
            .as_deref()
            .and_then(|images| image_change(&plan.images, images))
        {
            let note = if attached.is_empty() {
                ""
            } else {
                "; the new ones are attached to this message"
            };
            changes.push(format!("The overview has {what}{note}."));
        }
        let edited = (overview.is_some() || overview_images.is_some()).then(|| ChatPlan {
            overview: overview.unwrap_or_else(|| plan.overview.clone()),
            images: overview_images.unwrap_or_else(|| plan.images.clone()),
            ..plan.clone()
        });
        let mut updates: Vec<(String, TaskStepPatch)> = moves
            .iter()
            .map(|(id, key)| {
                let patch = TaskStepPatch {
                    order_key: Some(key.clone()),
                    ..Default::default()
                };
                (id.clone(), patch)
            })
            .collect();
        let mut creates = Vec::new();
        if !moves.is_empty() {
            let title = |id: &str| {
                steps
                    .iter()
                    .find(|edit| edit.step_id.as_deref() == Some(id))
                    .map(|edit| edit.title.trim().to_string())
                    .or_else(|| find(id).ok().map(|step| step.title.clone()))
                    .unwrap_or_default()
            };
            let listed: Vec<String> = order
                .iter()
                .enumerate()
                .map(
                    |(ix, id)| match moves.iter().any(|(moved, _)| moved == id) {
                        true => format!("{}. \"{}\" (moved, stepId {id})", ix + 1, title(id)),
                        false => format!("{}. \"{}\"", ix + 1, title(id)),
                    },
                )
                .collect();
            changes.push(format!(
                "The steps are in a new order: {}.",
                listed.join(", ")
            ));
        }
        for edit in steps {
            let title = edit.title.trim().to_string();
            let Some(id) = edit.step_id else {
                let id = crate::new_id();
                let images = edit.images.unwrap_or_default();
                let with = match images.len() {
                    0 => String::new(),
                    n => format!(" with {}", count_images(n)),
                };
                let role = edit.role.as_deref().and_then(canonical_role).or_else(|| {
                    namesake(removed.iter().copied(), &title, |old| &old.role).cloned()
                });
                let target = match edit.target {
                    Some(target) => target,
                    None => namesake(removed.iter().copied(), &title, |old| &old.target).cloned(),
                };
                let now = chrono::Utc::now();
                creates.push(TaskStep {
                    id: id.clone(),
                    owner_chat_id: chat_id.to_string(),
                    title: title.clone(),
                    notes: edit.notes,
                    order_key: String::new(),
                    chat_id: None,
                    outcome: None,
                    created_at: now,
                    updated_at: now,
                    deleted: false,
                    turn: None,
                    images,
                    role,
                    route: None,
                    target,
                });
                changes.push(format!("New step \"{title}\" (stepId {id}){with}."));
                continue;
            };
            let step = find(&id)?;
            let images = edit.images.filter(|images| *images != step.images);
            let started = !self.unstarted(step);
            let image_note = images.as_deref().and_then(|images| {
                let what = image_change(&step.images, images)?;
                Some(
                    match started && !new_images(&step.images, images).is_empty() {
                        true => {
                            format!("{what} (its thread already started and does not get them)")
                        }
                        false => what,
                    },
                )
            });
            let patch = TaskStepPatch {
                title: (title != step.title).then(|| title.clone()),
                notes: (edit.notes != step.notes).then_some(edit.notes),
                images,
                role: edit
                    .role
                    .as_deref()
                    .and_then(canonical_role)
                    .filter(|role| step.role.as_deref() != Some(role.as_str()))
                    .map(Some),
                target: edit.target.filter(|target| *target != step.target),
                ..Default::default()
            };
            let text = match (patch.title.is_some(), patch.notes.is_some()) {
                (true, true) => Some(format!("is now \"{title}\", and its spec changed")),
                (true, false) => Some(format!("is now \"{title}\"")),
                (false, true) => Some("has a new spec".to_string()),
                (false, false) => None,
            };
            let what = match (text, image_note) {
                (Some(text), Some(note)) => Some(format!("{text}, and has {note}")),
                (Some(text), None) => Some(text),
                (None, Some(note)) => Some(format!("has {note}")),
                (None, None) if patch.role.is_some() || patch.target.is_some() => None,
                (None, None) => continue,
            };
            if let Some(target) = &patch.target {
                changes.push(format!(
                    "Step \"{}\" (stepId {id}) {}",
                    step.title,
                    target_change(target.as_ref())
                ));
            }
            updates.push((id.clone(), patch));
            if let Some(what) = what {
                changes.push(format!("Step \"{}\" (stepId {id}) {what}.", step.title));
            }
        }
        let deletes: Vec<String> = removed.iter().map(|step| step.id.clone()).collect();
        self.workspace
            .edit_plan(chat_id, edited.as_ref(), &updates, &creates, &deletes)
            .map_err(written)?;
        for step in removed {
            changes.push(format!(
                "Step \"{}\" (stepId {}) is removed.",
                step.title, step.id
            ));
        }
        if changes.is_empty() {
            return Ok(None);
        }
        let next = if chat.plan_pending {
            "The plan still waits for the person's approval: do not run any step until the approval message comes."
        } else {
            "Run the new and changed steps from this plan, and send a step whose thread is already running what changed with send_message_to_thread."
        };
        let text = format!(
            "The person edited your plan:\n- {}\n\nRead the whole plan with the plan tool, action \"read\". {next}",
            changes.join("\n- ")
        );
        Ok(Some((text, attached)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: &str, key: &str) -> TaskStep {
        TaskStep {
            id: id.into(),
            owner_chat_id: "lead".into(),
            title: id.into(),
            notes: String::new(),
            order_key: key.into(),
            chat_id: None,
            outcome: None,
            created_at: Default::default(),
            updated_at: Default::default(),
            deleted: false,
            turn: None,
            images: Vec::new(),
            role: None,
            route: None,
            target: None,
        }
    }

    fn reorder(plan: &[(&str, bool)], order: &[&str]) -> Result<Vec<String>, &'static str> {
        let rows: Vec<TaskStep> = plan
            .iter()
            .enumerate()
            .map(|(ix, (id, _))| step(id, &format!("{}8", ix + 1)))
            .collect();
        let steps: Vec<(&TaskStep, bool)> = rows
            .iter()
            .zip(plan)
            .map(|(row, (_, started))| (row, *started))
            .collect();
        let order: Vec<String> = order.iter().map(|id| id.to_string()).collect();
        let mut n = 0;
        let moves = reorder_keys(&steps, &order, || {
            n += 1;
            format!("n{n}")
        })?;
        let mut keyed: Vec<(String, String)> = rows
            .iter()
            .map(|row| {
                let key = moves
                    .iter()
                    .find(|(id, _)| *id == row.id)
                    .map_or(row.order_key.clone(), |(_, key)| key.clone());
                (key, row.id.clone())
            })
            .collect();
        keyed.sort();
        assert_eq!(
            keyed.iter().map(|(_, id)| id.as_str()).collect::<Vec<_>>(),
            order,
            "the new keys sort into the new order"
        );
        Ok(moves.into_iter().map(|(id, _)| id).collect())
    }

    #[test]
    fn only_the_moved_step_gets_a_new_key() {
        let plan = [
            ("a", true),
            ("b", true),
            ("c", false),
            ("d", false),
            ("e", false),
        ];
        assert_eq!(
            reorder(&plan, &["a", "b", "e", "c", "d"]),
            Ok(vec!["e".into()])
        );
        assert_eq!(
            reorder(&plan, &["a", "b", "d", "e", "c"]),
            Ok(vec!["c".into()])
        );
        assert_eq!(reorder(&plan, &["a", "b", "c", "d", "e"]), Ok(vec![]));
    }

    #[test]
    fn an_unstarted_step_above_a_running_one_may_drop_below_it() {
        let plan = [("a", false), ("b", true), ("c", false)];
        assert_eq!(reorder(&plan, &["b", "a", "c"]), Ok(vec!["a".into()]));
        assert_eq!(reorder(&plan, &["b", "c", "a"]), Ok(vec!["a".into()]));
    }

    #[test]
    fn started_steps_keep_their_place() {
        let plan = [("a", true), ("b", true), ("c", false)];
        assert_eq!(reorder(&plan, &["b", "a", "c"]), Err(REORDER_REFUSED));
        assert_eq!(reorder(&plan, &["a", "c", "b"]), Err(REORDER_REFUSED));
        assert_eq!(reorder(&plan, &["c", "a", "b"]), Err(REORDER_REFUSED));
    }

    #[test]
    fn the_order_lists_every_step_once() {
        let plan = [("a", false), ("b", false)];
        assert!(reorder(&plan, &["a"]).is_err());
        assert!(reorder(&plan, &["a", "a"]).is_err());
        assert!(reorder(&plan, &["a", "x"]).is_err());
        let started = [("s0", false), ("s1", true), ("s2", false)];
        assert!(reorder(&started, &["s2", "s0", "s0", "x"]).is_err());
    }
}
