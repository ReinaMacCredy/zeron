use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use zeron_proto::supervisor::{
    InterventionChip, InterventionKind, LeadQuestion, NeedsYouRow, QuestionState, ResourceState,
    SharedResource, SupervisorIntervention, SupervisorSnapshot,
};

use super::{no_effect_yet, row_key};

pub(crate) const OPEN_HOURS: i64 = 24;
const MEMORY_ROWS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Needs,
    Question,
    Intervention,
    Resource,
}

impl Kind {
    pub(crate) fn chips() -> Vec<Kind> {
        let mut kinds = vec![Kind::Needs, Kind::Question, Kind::Intervention];
        if !cfg!(windows) {
            kinds.push(Kind::Resource);
        }
        kinds
    }

    pub(crate) fn tag(self) -> &'static str {
        match self {
            Kind::Needs => "Needs you",
            Kind::Question => "Question",
            Kind::Intervention => "Intervention",
            Kind::Resource => "Resource",
        }
    }

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
}

#[derive(Debug, Default)]
pub(crate) struct Feed {
    pub open: Vec<Row>,
    pub declared: Vec<Row>,
    pub earlier: Vec<Row>,
}

impl Feed {
    pub(crate) fn count(&self, kind: Kind) -> usize {
        self.open.iter().filter(|row| row.kind == kind).count()
    }

    pub(crate) fn rows(&self, filter: Filter) -> impl Iterator<Item = (u8, &Row, bool)> {
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

pub(crate) fn needs_at(row: &NeedsYouRow) -> DateTime<Utc> {
    match row {
        NeedsYouRow::Item(item) => item.at,
        NeedsYouRow::Input(input) => input.at,
    }
}

pub(crate) fn question_key(question: &LeadQuestion) -> String {
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

pub(crate) fn feed(snapshot: &SupervisorSnapshot, memory: &Memory, now: DateTime<Utc>) -> Feed {
    let mut open: Vec<Row> = snapshot
        .needs_you
        .iter()
        .map(|row| Row {
            key: row_key(row),
            kind: Kind::Needs,
            at: needs_at(row),
            entry: Entry::Needs(row.clone()),
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
                }),
        );
    }
    earlier.extend(memory.answered.iter().map(|answered| Row {
        key: row_key(&answered.row),
        kind: Kind::Needs,
        at: answered.at,
        entry: Entry::Answered(answered.clone()),
    }));
    newest_first(&mut rest);
    newest_first(&mut earlier);
    open.extend(rest);
    Feed {
        open,
        declared,
        earlier,
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
            text: "Use the mock.".into(),
            trigger: None,
            chips,
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

    fn keys(rows: &[Row]) -> Vec<&str> {
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
        let feed = feed(&snapshot, &Memory::default(), now);
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
        let feed = feed(&snapshot, &Memory::default(), now);
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
        let earlier = feed(&third, &memory, now).earlier;
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
            !feed(&fourth, &memory, now)
                .earlier
                .iter()
                .any(|row| row.kind == Kind::Resource)
        );
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
        assert!(Kind::chips().iter().all(|kind| Filter::All.shows(*kind)));
    }
}
