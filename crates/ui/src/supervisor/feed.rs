use std::collections::HashMap;
use std::rc::Rc;

use chrono::{DateTime, Duration, Utc};
use zeron_proto::supervisor::{
    InputAsker, InterventionChip, InterventionKind, LeadQuestion, NeedsYouRow, QuestionState,
    ResourceState, SharedResource, SupervisorIntervention, SupervisorSnapshot,
};

use super::{no_effect_yet, row_key, row_origin};

const OPEN_HOURS: i64 = 24;
const MEMORY_ROWS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Needs,
    Question,
    Intervention,
    Resource,
}

impl Kind {
    #[cfg(not(windows))]
    pub(crate) const CHIPS: &[Kind] = &[
        Kind::Needs,
        Kind::Question,
        Kind::Intervention,
        Kind::Resource,
    ];
    #[cfg(windows)]
    pub(crate) const CHIPS: &[Kind] = &[Kind::Needs, Kind::Question, Kind::Intervention];

    pub(crate) fn chip(self) -> &'static str {
        match self {
            Kind::Needs => "Needs you",
            Kind::Question => "Questions",
            Kind::Intervention => "Interventions",
            Kind::Resource => "Resources",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Filter {
    #[default]
    All,
    Only(Kind),
}

impl Filter {
    pub(crate) fn pick(self, chip: Option<Kind>) -> Self {
        match chip {
            Some(kind) if self != Filter::Only(kind) => Filter::Only(kind),
            _ => Filter::All,
        }
    }

    pub(crate) fn shows(self, kind: Kind) -> bool {
        self == Filter::All || self == Filter::Only(kind)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Answered {
    pub row: NeedsYouRow,
    pub at: DateTime<Utc>,
    pub choice: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Entry {
    Needs(NeedsYouRow),
    Question(LeadQuestion),
    Intervention(SupervisorIntervention),
    Resource(SharedResource),
    Answered(Answered),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    pub key: String,
    pub kind: Kind,
    pub at: DateTime<Utc>,
    pub entry: Entry,
    pub group: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Feed {
    pub open: Vec<Rc<Row>>,
    pub declared: Vec<Rc<Row>>,
    pub earlier: Vec<Rc<Row>>,
}

impl Feed {
    pub(crate) fn count(&self, kind: Kind) -> usize {
        self.open.iter().filter(|row| row.kind == kind).count()
    }

    pub(crate) fn rows(&self, filter: Filter) -> impl Iterator<Item = (u8, &Rc<Row>, bool)> {
        let declared = filter == Filter::Only(Kind::Resource);
        self.open
            .iter()
            .map(move |row| {
                (
                    u8::from(row.kind != Kind::Needs),
                    row,
                    filter.shows(row.kind),
                )
            })
            .chain(self.declared.iter().map(move |row| (2, row, declared)))
    }
}

/// A watched plain tree has no task, so its threads' asks group under the tree's root chat.
pub(crate) fn needs_group(
    row: &NeedsYouRow,
    root_of: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    match row {
        NeedsYouRow::Input(input)
            if input.task_id.is_none() && input.asker != InputAsker::Supervisor =>
        {
            root_of(&input.chat_id)
        }
        row => row_origin(row).0.map(str::to_owned),
    }
}

fn group_of(entry: &Entry, root_of: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    match entry {
        Entry::Needs(row) | Entry::Answered(Answered { row, .. }) => needs_group(row, root_of),
        Entry::Question(question) => Some(question.task_id.clone()),
        Entry::Intervention(intervention) => intervention.task_id.clone(),
        Entry::Resource(_) => None,
    }
}

pub(crate) struct Group<'a> {
    pub task: Option<&'a str>,
    pub rows: Vec<&'a Row>,
}

impl Group<'_> {
    pub(crate) fn key(&self) -> String {
        self.task.unwrap_or_default().to_owned()
    }
}

pub(crate) fn grouped<'a>(rows: impl IntoIterator<Item = &'a Row>) -> Vec<Group<'a>> {
    let mut groups: Vec<Group<'a>> = Vec::new();
    for row in rows {
        let task = row.group.as_deref();
        match groups.iter_mut().find(|group| group.task == task) {
            Some(group) => group.rows.push(row),
            None => groups.push(Group {
                task,
                rows: vec![row],
            }),
        }
    }
    for group in &mut groups {
        group.rows.sort_by(|a, b| {
            (a.kind != Kind::Needs, std::cmp::Reverse(a.at), &a.key).cmp(&(
                b.kind != Kind::Needs,
                std::cmp::Reverse(b.at),
                &b.key,
            ))
        });
    }
    let needs = |group: &Group| group.rows.iter().any(|row| row.kind == Kind::Needs);
    let newest = |group: &Group| group.rows.iter().map(|row| row.at).max();
    groups.sort_by(|a, b| {
        (
            !needs(a),
            a.task.is_none(),
            std::cmp::Reverse(newest(a)),
            a.task,
        )
            .cmp(&(
                !needs(b),
                b.task.is_none(),
                std::cmp::Reverse(newest(b)),
                b.task,
            ))
    });
    groups
}

fn needs_at(row: &NeedsYouRow) -> DateTime<Utc> {
    match row {
        NeedsYouRow::Item(item) => item.at,
        NeedsYouRow::Input(input) => input.at,
    }
}

fn question_key(question: &LeadQuestion) -> String {
    format!("question:{}:{}", question.chat_id, question.request_id)
}

pub(crate) fn resource_open(state: ResourceState) -> bool {
    matches!(
        state,
        ResourceState::Contended | ResourceState::NotVerified | ResourceState::CheckFailed
    )
}

/// An unchecked resource has no time of its own, so it sorts below everything dated.
fn resource_at(resource: &SharedResource) -> DateTime<Utc> {
    resource
        .check
        .as_ref()
        .map_or(DateTime::<Utc>::MIN_UTC, |check| check.at)
}

fn work_effect(chip: &InterventionChip) -> bool {
    matches!(
        chip,
        InterventionChip::PlanEdited { .. }
            | InterventionChip::StepCompletedTurn { .. }
            | InterventionChip::ResourceFreed
    )
}

pub(crate) fn intervention_open(row: &SupervisorIntervention, now: DateTime<Utc>) -> bool {
    if row.kind == InterventionKind::Pause {
        return row.chips.contains(&InterventionChip::StepPaused);
    }
    !row.chips.iter().any(work_effect)
        && (now - row.at < Duration::hours(OPEN_HOURS) || no_effect_yet(row, now))
}

pub(crate) fn next_expiry(
    rows: &[SupervisorIntervention],
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    rows.iter()
        .filter(|row| row.kind != InterventionKind::Pause)
        .map(|row| row.at + Duration::hours(OPEN_HOURS))
        .filter(|at| *at > now)
        .min()
}

fn newest_first(rows: &mut [Row]) {
    rows.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.key.cmp(&b.key)));
}

pub(crate) fn feed(
    snapshot: &SupervisorSnapshot,
    memory: &Memory,
    now: DateTime<Utc>,
    root_of: &dyn Fn(&str) -> Option<String>,
) -> Feed {
    let mut open: Vec<Row> = snapshot
        .needs_you
        .iter()
        .map(|row| Row {
            key: row_key(row),
            kind: Kind::Needs,
            at: needs_at(row),
            entry: Entry::Needs(row.clone()),
            group: None,
        })
        .collect();
    newest_first(&mut open);
    let (mut rest, mut earlier, mut declared) = (Vec::new(), Vec::new(), Vec::new());
    for question in &snapshot.questions {
        let row = Row {
            key: question_key(question),
            kind: Kind::Question,
            at: question.at,
            entry: Entry::Question(question.clone()),
            group: None,
        };
        match question.state {
            QuestionState::Open => rest.push(row),
            _ => earlier.push(row),
        }
    }
    for intervention in &snapshot.interventions {
        let row = Row {
            key: format!("intervention:{}", intervention.id),
            kind: Kind::Intervention,
            at: intervention.at,
            entry: Entry::Intervention(intervention.clone()),
            group: None,
        };
        if intervention_open(intervention, now) {
            rest.push(row);
        } else {
            earlier.push(row);
        }
    }
    if !cfg!(windows) {
        for resource in &snapshot.resources {
            let row = Row {
                key: format!("resource:{}", resource.key),
                kind: Kind::Resource,
                at: resource_at(resource),
                entry: Entry::Resource(resource.clone()),
                group: None,
            };
            if resource_open(resource.state) {
                rest.push(row);
            } else {
                declared.push(row);
            }
        }
        let open_keys: Vec<&str> = snapshot
            .resources
            .iter()
            .filter(|resource| resource_open(resource.state))
            .map(|resource| resource.key.as_str())
            .collect();
        earlier.extend(
            memory
                .cleared
                .iter()
                .filter(|(resource, _)| !open_keys.contains(&resource.key.as_str()))
                .map(|(resource, at)| Row {
                    key: format!("resource:{}", resource.key),
                    kind: Kind::Resource,
                    at: *at,
                    entry: Entry::Resource(resource.clone()),
                    group: None,
                }),
        );
    }
    earlier.extend(memory.answered.iter().map(|answered| Row {
        key: row_key(&answered.row),
        kind: Kind::Needs,
        at: answered.at,
        entry: Entry::Answered(answered.clone()),
        group: None,
    }));
    newest_first(&mut rest);
    newest_first(&mut earlier);
    open.extend(rest);
    let shared = |rows: Vec<Row>| {
        rows.into_iter()
            .map(|mut row| {
                row.group = group_of(&row.entry, root_of);
                Rc::new(row)
            })
            .collect()
    };
    Feed {
        open: shared(open),
        declared: shared(declared),
        earlier: shared(earlier),
    }
}

#[derive(Debug, Default)]
pub(crate) struct Memory {
    needs: Option<Vec<NeedsYouRow>>,
    open_resources: Option<Vec<SharedResource>>,
    answered: Vec<Answered>,
    cleared: Vec<(SharedResource, DateTime<Utc>)>,
}

impl Memory {
    pub(crate) fn observe(
        &mut self,
        snapshot: Option<&SupervisorSnapshot>,
        choices: &HashMap<String, String>,
        now: DateTime<Utc>,
    ) -> Vec<String> {
        let Some(snapshot) = snapshot else {
            self.needs = None;
            self.open_resources = None;
            return Vec::new();
        };
        let keys: Vec<String> = snapshot.needs_you.iter().map(row_key).collect();
        let mut arrived = Vec::new();
        if let Some(previous) = self.needs.take() {
            let before: Vec<String> = previous.iter().map(row_key).collect();
            arrived = keys
                .iter()
                .filter(|key| !before.contains(key))
                .cloned()
                .collect();
            for (row, key) in previous.into_iter().zip(before) {
                if keys.contains(&key) {
                    continue;
                }
                self.answered.retain(|known| row_key(&known.row) != key);
                self.answered.push(Answered {
                    choice: choices.get(&key).cloned(),
                    row,
                    at: now,
                });
            }
        }
        self.answered
            .retain(|known| !keys.contains(&row_key(&known.row)));
        self.needs = Some(snapshot.needs_you.clone());
        let open: Vec<SharedResource> = snapshot
            .resources
            .iter()
            .filter(|resource| resource_open(resource.state))
            .cloned()
            .collect();
        if let Some(previous) = self.open_resources.take() {
            for resource in previous {
                if open.iter().any(|now_open| now_open.key == resource.key) {
                    continue;
                }
                let latest = snapshot
                    .resources
                    .iter()
                    .find(|declared| declared.key == resource.key)
                    .cloned()
                    .unwrap_or(resource);
                self.cleared.retain(|(known, _)| known.key != latest.key);
                self.cleared.push((latest, now));
            }
        }
        self.cleared
            .retain(|(known, _)| !open.iter().any(|resource| resource.key == known.key));
        self.open_resources = Some(open);
        trim(&mut self.answered);
        trim(&mut self.cleared);
        arrived
    }
}

fn trim<T>(rows: &mut Vec<T>) {
    let over = rows.len().saturating_sub(MEMORY_ROWS);
    rows.drain(..over);
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::supervisor::{
        CheckTool, InputAsker, ItemWhy, NeedsYouInput, Raiser, ResourceCheck, SettledBy,
        SupervisorItem,
    };

    fn minutes(now: DateTime<Utc>, ago: i64) -> DateTime<Utc> {
        now - Duration::minutes(ago)
    }

    fn item(id: &str, at: DateTime<Utc>) -> NeedsYouRow {
        NeedsYouRow::Item(SupervisorItem {
            id: id.into(),
            at,
            raised_by: Raiser::Lead,
            raiser_chat_id: "lead".into(),
            task_id: Some("task".into()),
            title: format!("Decide {id}"),
            why: ItemWhy::Decision,
            detail: String::new(),
            options: vec!["Yes".into(), "No".into()],
            answer: None,
        })
    }

    fn input(request: &str, at: DateTime<Utc>) -> NeedsYouRow {
        NeedsYouRow::Input(NeedsYouInput {
            chat_id: "step".into(),
            request_id: request.into(),
            asker: InputAsker::Step,
            task_id: Some("task".into()),
            step_id: Some("s1".into()),
            at,
            questions: Vec::new(),
        })
    }

    fn question(request: &str, at: DateTime<Utc>, state: QuestionState) -> LeadQuestion {
        LeadQuestion {
            task_id: "task".into(),
            step_id: "s2".into(),
            chat_id: "step-2".into(),
            request_id: request.into(),
            at,
            questions: Vec::new(),
            state,
        }
    }

    fn intervention(
        id: &str,
        at: DateTime<Utc>,
        kind: InterventionKind,
        chips: Vec<InterventionChip>,
    ) -> SupervisorIntervention {
        SupervisorIntervention {
            id: id.into(),
            at,
            kind,
            task_id: Some("task".into()),
            step_ids: vec!["s2".into()],
            target: Some("lead".into()),
            message_id: Some("m".into()),
            request_id: None,
            item: None,
            text: "Use the mock.".into(),
            trigger: None,
            chips,
            approval: None,
        }
    }

    fn resource(key: &str, state: ResourceState, at: Option<DateTime<Utc>>) -> SharedResource {
        SharedResource {
            key: key.into(),
            declared_by: vec!["task".into()],
            holders: Vec::new(),
            state,
            check: at.map(|at| ResourceCheck {
                at,
                tool: CheckTool::Lsof,
                contended: state == ResourceState::Contended,
                error: None,
                uses: Vec::new(),
            }),
        }
    }

    fn keys(rows: &[Rc<Row>]) -> Vec<&str> {
        rows.iter().map(|row| row.key.as_str()).collect()
    }

    #[test]
    fn needs_you_leads_newest_first_and_every_other_open_row_follows_by_time() {
        let now = Utc::now();
        let snapshot = SupervisorSnapshot {
            needs_you: vec![
                item("old", minutes(now, 30)),
                item("new", minutes(now, 2)),
                input("ask", minutes(now, 10)),
            ],
            questions: vec![question("q", minutes(now, 1), QuestionState::Open)],
            interventions: vec![intervention(
                "i",
                minutes(now, 5),
                InterventionKind::Send,
                vec![InterventionChip::Sent],
            )],
            resources: vec![
                resource("port:5173", ResourceState::Contended, Some(minutes(now, 3))),
                resource("path:src", ResourceState::NotVerified, None),
            ],
            ..Default::default()
        };
        let feed = feed(&snapshot, &Memory::default(), now, &|_| None);
        let mut want = vec![
            "item:new",
            "input:step:ask",
            "item:old",
            "question:step-2:q",
        ];
        if !cfg!(windows) {
            want.push("resource:port:5173");
        }
        want.push("intervention:i");
        if !cfg!(windows) {
            want.push("resource:path:src");
        }
        assert_eq!(keys(&feed.open), want);
        assert_eq!(feed.count(Kind::Needs), 3);
        assert!(feed.earlier.is_empty());
    }

    #[test]
    fn each_kind_leaves_the_feed_by_its_own_rule() {
        let now = Utc::now();
        let sent = || vec![InterventionChip::Sent];
        let effect = |chip: InterventionChip| vec![InterventionChip::Sent, chip];
        let snapshot = SupervisorSnapshot {
            questions: vec![
                question("open", minutes(now, 1), QuestionState::Open),
                question(
                    "settled",
                    minutes(now, 2),
                    QuestionState::Settled {
                        by: SettledBy::Lead,
                        answers: Vec::new(),
                    },
                ),
                question("dropped", minutes(now, 3), QuestionState::Dropped),
            ],
            interventions: vec![
                intervention("fresh", minutes(now, 4), InterventionKind::Send, sent()),
                intervention(
                    "turned",
                    minutes(now, 5),
                    InterventionKind::Send,
                    effect(InterventionChip::LeadRanTurn),
                ),
                intervention(
                    "edited",
                    minutes(now, 6),
                    InterventionKind::Rebrief,
                    effect(InterventionChip::PlanEdited { steps: vec![2] }),
                ),
                intervention(
                    "completed",
                    minutes(now, 7),
                    InterventionKind::Send,
                    effect(InterventionChip::StepCompletedTurn { steps: vec![2] }),
                ),
                intervention(
                    "freed",
                    minutes(now, 8),
                    InterventionKind::Pause,
                    effect(InterventionChip::ResourceFreed),
                ),
                intervention("resume", minutes(now, 9), InterventionKind::Resume, sent()),
                intervention("paused", minutes(now, 10), InterventionKind::Pause, sent()),
                intervention(
                    "held",
                    minutes(now, 11),
                    InterventionKind::Pause,
                    vec![
                        InterventionChip::Sent,
                        InterventionChip::StepPaused,
                        InterventionChip::ResourceFreed,
                    ],
                ),
                intervention(
                    "stale",
                    minutes(now, 25 * 60),
                    InterventionKind::Send,
                    effect(InterventionChip::LeadRanTurn),
                ),
                intervention(
                    "ignored",
                    minutes(now, 26 * 60),
                    InterventionKind::Send,
                    sent(),
                ),
            ],
            resources: vec![
                resource("port:1", ResourceState::Contended, Some(minutes(now, 1))),
                resource("port:2", ResourceState::CheckFailed, Some(minutes(now, 2))),
                resource("port:3", ResourceState::Held, None),
                resource("port:4", ResourceState::Clear, Some(minutes(now, 3))),
            ],
            ..Default::default()
        };
        let feed = feed(&snapshot, &Memory::default(), now, &|_| None);
        let open: Vec<&str> = feed
            .open
            .iter()
            .filter(|row| row.kind != Kind::Resource)
            .map(|row| row.key.as_str())
            .collect();
        assert_eq!(
            open,
            vec![
                "question:step-2:open",
                "intervention:fresh",
                "intervention:turned",
                "intervention:resume",
                "intervention:held",
                "intervention:ignored",
            ]
        );
        assert_eq!(
            keys(&feed.earlier),
            vec![
                "question:step-2:settled",
                "question:step-2:dropped",
                "intervention:edited",
                "intervention:completed",
                "intervention:freed",
                "intervention:paused",
                "intervention:stale",
            ]
        );
        if !cfg!(windows) {
            assert_eq!(feed.count(Kind::Resource), 2);
            assert_eq!(
                keys(&feed.declared),
                vec!["resource:port:3", "resource:port:4"]
            );
            let listed = |filter| -> Vec<String> {
                feed.rows(filter)
                    .filter(|(_, row, shown)| *shown && row.kind == Kind::Resource)
                    .map(|(_, row, _)| row.key.clone())
                    .collect()
            };
            assert_eq!(listed(Filter::All).len(), 2);
            assert_eq!(
                listed(Filter::Only(Kind::Resource)),
                vec![
                    "resource:port:1",
                    "resource:port:2",
                    "resource:port:3",
                    "resource:port:4"
                ]
            );
        }
        assert_eq!(
            next_expiry(&snapshot.interventions, now),
            Some(minutes(now, 9) + Duration::hours(OPEN_HOURS))
        );
    }

    #[test]
    fn rows_that_leave_while_the_page_lives_land_in_earlier() {
        let now = Utc::now();
        let mut memory = Memory::default();
        let choices = HashMap::from([("item:a".to_owned(), "Yes".to_owned())]);
        let snapshot = |needs: Vec<NeedsYouRow>, state: ResourceState| SupervisorSnapshot {
            needs_you: needs,
            resources: vec![resource("port:5173", state, Some(now))],
            ..Default::default()
        };
        let first = snapshot(vec![item("a", now)], ResourceState::Contended);
        assert!(memory.observe(Some(&first), &choices, now).is_empty());
        let second = snapshot(
            vec![item("a", now), item("b", now)],
            ResourceState::Contended,
        );
        assert_eq!(memory.observe(Some(&second), &choices, now), vec!["item:b"]);
        assert!(memory.observe(None, &choices, now).is_empty());
        assert!(memory.observe(Some(&second), &choices, now).is_empty());
        assert!(memory.answered.is_empty());
        let third = snapshot(vec![item("b", now)], ResourceState::Clear);
        assert!(memory.observe(Some(&third), &choices, now).is_empty());
        let earlier = feed(&third, &memory, now, &|_| None).earlier;
        let answered: Vec<Option<String>> = earlier
            .iter()
            .filter_map(|row| match &row.entry {
                Entry::Answered(answered) => Some(answered.choice.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(answered, vec![Some("Yes".to_owned())]);
        let cleared = earlier.iter().any(|row| {
            matches!(&row.entry, Entry::Resource(resource) if resource.state == ResourceState::Clear)
        });
        assert_eq!(cleared, !cfg!(windows));
        let fourth = snapshot(vec![item("b", now)], ResourceState::Contended);
        memory.observe(Some(&fourth), &choices, now);
        assert!(
            !feed(&fourth, &memory, now, &|_| None)
                .earlier
                .iter()
                .any(|row| row.kind == Kind::Resource)
        );
    }

    #[test]
    fn open_rows_group_by_mission_with_needs_first_and_the_shared_bucket_after_missions() {
        let now = Utc::now();
        let on = |row: NeedsYouRow, task: Option<&str>| match row {
            NeedsYouRow::Item(mut item) => {
                item.task_id = task.map(str::to_owned);
                NeedsYouRow::Item(item)
            }
            other => other,
        };
        let mut quiet = intervention(
            "quiet",
            minutes(now, 1),
            InterventionKind::Send,
            vec![InterventionChip::Sent],
        );
        quiet.task_id = Some("quiet".into());
        let snapshot = SupervisorSnapshot {
            needs_you: vec![
                on(item("old", minutes(now, 30)), Some("a")),
                on(item("shared", minutes(now, 1)), None),
                on(item("b", minutes(now, 5)), Some("b")),
                on(item("new", minutes(now, 2)), Some("a")),
            ],
            interventions: vec![
                intervention(
                    "i",
                    minutes(now, 0),
                    InterventionKind::Send,
                    vec![InterventionChip::Sent],
                ),
                quiet,
            ],
            ..Default::default()
        };
        let first = feed(&snapshot, &Memory::default(), now, &|_| None);
        let groups: Vec<(Option<&str>, Vec<&str>)> = grouped(first.open.iter().map(|row| &**row))
            .into_iter()
            .map(|group| (group.task, keys_of(&group.rows)))
            .collect();
        assert_eq!(
            groups,
            vec![
                (Some("a"), vec!["item:new", "item:old"]),
                (Some("b"), vec!["item:b"]),
                (None, vec!["item:shared"]),
                (Some("task"), vec!["intervention:i"]),
                (Some("quiet"), vec!["intervention:quiet"]),
            ]
        );
        let mut snapshot = snapshot;
        snapshot.interventions[0].task_id = Some("a".into());
        let second = feed(&snapshot, &Memory::default(), now, &|_| None);
        let a = grouped(second.open.iter().map(|row| &**row))
            .into_iter()
            .find(|group| group.task == Some("a"))
            .map(|group| keys_of(&group.rows));
        assert_eq!(
            a,
            Some(vec!["item:new", "item:old", "intervention:i"]),
            "a mission's decisions come before its updates, however new the update"
        );
        let ask = |chat: &str, asker: InputAsker| {
            NeedsYouRow::Input(NeedsYouInput {
                chat_id: chat.into(),
                request_id: chat.into(),
                asker,
                task_id: None,
                step_id: None,
                at: now,
                questions: Vec::new(),
            })
        };
        let plain = SupervisorSnapshot {
            needs_you: vec![
                ask("child", InputAsker::Thread),
                ask("supervisor-1", InputAsker::Supervisor),
            ],
            ..Default::default()
        };
        let root_of = |chat: &str| (chat != "supervisor-1").then(|| "root".to_owned());
        let shown = feed(&plain, &Memory::default(), now, &root_of);
        let groups: Vec<(Option<String>, &str)> = shown
            .open
            .iter()
            .map(|row| (row.group.clone(), row.key.as_str()))
            .collect();
        assert_eq!(
            groups,
            vec![
                (Some("root".into()), "input:child:child"),
                (None, "input:supervisor-1:supervisor-1"),
            ],
            "a plain tree's ask groups under its root; the Supervisor's own stays shared"
        );
    }

    fn keys_of<'a>(rows: &[&'a Row]) -> Vec<&'a str> {
        rows.iter().map(|row| row.key.as_str()).collect()
    }

    #[test]
    fn a_chip_selects_its_kind_and_a_second_click_returns_to_all() {
        let filter = Filter::All.pick(Some(Kind::Question));
        assert_eq!(filter, Filter::Only(Kind::Question));
        assert!(filter.shows(Kind::Question) && !filter.shows(Kind::Needs));
        assert_eq!(filter.pick(Some(Kind::Needs)), Filter::Only(Kind::Needs));
        assert_eq!(filter.pick(Some(Kind::Question)), Filter::All);
        assert_eq!(filter.pick(None), Filter::All);
        assert_eq!(Filter::All.pick(None), Filter::All);
        assert!(Kind::CHIPS.iter().all(|kind| Filter::All.shows(*kind)));
    }
}
