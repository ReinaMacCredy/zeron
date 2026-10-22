use crate::{
    BackgroundTask, BackgroundTaskKind, Chat, DelegationStatus, HarnessId, ReasoningLevel, Session,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Working,
    Completed,
    Stopped,
    Failed,
}

impl RunState {
    pub fn word(self) -> &'static str {
        match self {
            RunState::Working => "Working",
            RunState::Completed => "Completed",
            RunState::Stopped => "Stopped",
            RunState::Failed => "Failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentRun {
    pub part_id: String,
    pub doc_id: String,
    pub title: String,
    pub model: Option<String>,
    pub reasoning: Option<ReasoningLevel>,
    pub state: RunState,
    pub started_ms: i64,
    pub delegated: bool,
    pub harness: Option<HarnessId>,
}

impl AgentRun {
    pub fn can_stop(&self, stoppable: bool) -> bool {
        (stoppable || self.delegated) && self.state == RunState::Working
    }

    pub fn tag(&self, harness: Option<HarnessId>) -> Option<String> {
        self.model.clone().or_else(|| {
            self.harness
                .or(harness)
                .map(|h| h.display_name().to_string())
        })
    }

    pub fn line(&self, harness: Option<HarnessId>, now_ms: i64) -> String {
        let elapsed = (self.state == RunState::Working)
            .then(|| elapsed_label((now_ms - self.started_ms) / 1000));
        [
            Some(self.state.word().to_string()),
            elapsed,
            self.tag(harness),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    }
}

/// Only Claude Code can stop one of its own subagents or shells mid-run.
pub fn stoppable(harness: Option<HarnessId>) -> bool {
    harness == Some(HarnessId::ClaudeCode)
}

pub fn delegate_runs(
    chats: &[Chat],
    owner: &str,
    turn_start_ms: i64,
    keep: Option<&str>,
) -> Vec<AgentRun> {
    chats
        .iter()
        .filter_map(|chat| {
            let delegation = chat
                .delegation
                .as_ref()
                .filter(|d| d.delegated_by_chat_id == owner)?;
            let state = match delegation.status {
                DelegationStatus::Running => RunState::Working,
                DelegationStatus::Completed => RunState::Completed,
                DelegationStatus::Cancelled | DelegationStatus::Interrupted => RunState::Stopped,
                DelegationStatus::Failed => RunState::Failed,
            };
            let started_ms = delegation.started_at.timestamp_millis();
            let kept = keep == Some(chat.id.as_str());
            (state == RunState::Working || started_ms >= turn_start_ms || kept).then(|| AgentRun {
                part_id: chat.id.clone(),
                doc_id: chat.id.clone(),
                title: chat
                    .title
                    .clone()
                    .unwrap_or_else(|| "Delegated task".into()),
                model: chat.config.as_ref().and_then(|c| c.model.clone()),
                reasoning: chat.config.as_ref().and_then(|c| c.reasoning),
                state,
                started_ms,
                delegated: true,
                harness: chat.config.as_ref().map(|c| c.harness),
            })
        })
        .collect()
}

pub fn shell_tasks(session: Option<&Session>) -> Vec<BackgroundTask> {
    session
        .map(|session| {
            session
                .background_tasks
                .iter()
                .filter(|task| task.kind == BackgroundTaskKind::Shell)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

pub fn aggregate(runs: &[AgentRun]) -> RunState {
    if runs.iter().any(|run| run.state == RunState::Working) {
        RunState::Working
    } else if runs.iter().any(|run| run.state == RunState::Failed) {
        RunState::Failed
    } else if runs.iter().any(|run| run.state == RunState::Stopped) {
        RunState::Stopped
    } else {
        RunState::Completed
    }
}

fn working_count(runs: &[AgentRun]) -> usize {
    runs.iter()
        .filter(|run| run.state == RunState::Working)
        .count()
}

pub fn agents_label(runs: &[AgentRun]) -> (Option<String>, String) {
    let working = working_count(runs);
    match runs {
        [one] => (Some(one.title.clone()), format!("· {}", one.state.word())),
        _ if working > 0 => (None, format!("{working} Working")),
        _ => (None, settled_counts(runs)),
    }
}

fn settled_counts(runs: &[AgentRun]) -> String {
    [RunState::Completed, RunState::Stopped, RunState::Failed]
        .into_iter()
        .filter_map(|state| {
            let count = runs.iter().filter(|run| run.state == state).count();
            (count > 0).then(|| format!("{count} {}", state.word()))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn roster_heading(runs: &[AgentRun]) -> String {
    match working_count(runs) {
        0 => "Subagents".to_string(),
        working => format!("{working} of {} working", runs.len()),
    }
}

pub fn subagents_section(runs: &[AgentRun]) -> String {
    match working_count(runs) {
        0 => "Subagents".to_string(),
        _ => format!("Subagents · {}", roster_heading(runs)),
    }
}

pub fn shells_label(count: usize) -> String {
    format!("{count} Running")
}

pub fn shells_section(count: usize) -> String {
    format!("Background shells · {}", shells_label(count))
}

pub fn roster_summary(runs: &[AgentRun], shells: usize) -> Option<String> {
    let agents = match working_count(runs) {
        _ if runs.is_empty() => None,
        0 => Some(settled_counts(runs)),
        _ => Some(roster_heading(runs)),
    };
    let shells =
        (shells > 0).then(|| format!("{shells} shell{}", if shells == 1 { "" } else { "s" }));
    let parts: Vec<String> = [agents, shells].into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub fn shell_line(task: &BackgroundTask, now_ms: i64) -> String {
    match task.started_at {
        Some(started) => format!(
            "{} · {}",
            RunState::Working.word(),
            elapsed_label((now_ms - started.timestamp_millis()) / 1000)
        ),
        None => RunState::Working.word().to_string(),
    }
}

pub fn elapsed_label(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m {:02}s", secs % 60);
    }
    format!("{}h {:02}m", mins / 60, mins % 60)
}

pub fn latest_line(lines: &[String]) -> Option<&str> {
    lines
        .iter()
        .rev()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionStatus;
    use chrono::{DateTime, Utc};

    fn session(tasks: Vec<BackgroundTask>) -> Session {
        Session {
            last_completed_turn: None,
            chat_id: "c1".into(),
            device_id: "dev".into(),
            status: SessionStatus::Idle,
            held_by_subagents: false,
            compacting: false,
            background_tasks: tasks,
            started_at: None,
            updated_at: Utc::now(),
            running_subagents: 0,
            compact_plan: None,
            classifying: None,
        }
    }

    fn task(id: &str, kind: BackgroundTaskKind) -> BackgroundTask {
        BackgroundTask {
            id: id.into(),
            kind,
            label: format!("cargo test {id}"),
            started_at: None,
            output_path: None,
        }
    }

    fn run(title: &str, state: RunState) -> AgentRun {
        AgentRun {
            part_id: title.into(),
            doc_id: title.into(),
            title: title.into(),
            model: None,
            reasoning: None,
            state,
            started_ms: 0,
            delegated: false,
            harness: None,
        }
    }

    #[test]
    fn delegated_children_join_the_roster_on_their_own_harness() {
        let child = |id: &str, owner: &str, status, started_ms: i64| {
            let mut chat: Chat = serde_json::from_value(serde_json::json!({
                "id": id, "deviceId": "dev", "archived": false,
                "title": format!("Task {id}"),
                "createdAt": "2026-10-03T10:00:00Z",
                "config": { "harness": "codex", "model": "gpt-5", "sandbox": "workspace-write" },
            }))
            .unwrap();
            chat.delegation = Some(crate::Delegation {
                delegated_by_chat_id: owner.into(),
                depth: 1,
                anchor: "task".into(),
                client_request_id: None,
                owner_turn_started_at: None,
                wake: crate::DelegationWake::Always,
                status,
                cancel_requested: false,
                delivery: None,
                summary: None,
                started_at: DateTime::from_timestamp_millis(started_ms).unwrap(),
                completed_at: None,
                finding_outcomes: Default::default(),
                findings_handed: Default::default(),
                findings_announced: Default::default(),
                route: None,
            });
            chat
        };
        use DelegationStatus::*;
        let chats = vec![
            child("old-running", "owner", Running, 1_000),
            child("old-done", "owner", Completed, 1_000),
            child("new-failed", "owner", Failed, 5_000),
            child("new-stopped", "owner", Interrupted, 5_000),
            child("elsewhere", "other", Running, 5_000),
        ];
        let runs = delegate_runs(&chats, "owner", 4_000, Some("old-done"));
        let ids: Vec<&str> = runs.iter().map(|run| run.doc_id.as_str()).collect();
        assert_eq!(
            ids,
            ["old-running", "old-done", "new-failed", "new-stopped"]
        );
        assert!(runs.iter().all(|run| run.delegated));
        assert_eq!(runs[0].harness, Some(HarnessId::Codex));
        assert_eq!(runs[0].state, RunState::Working);
        assert_eq!(runs[2].state, RunState::Failed);
        assert_eq!(runs[3].state, RunState::Stopped);
    }

    #[test]
    fn labels_for_one_many_and_settled_runs() {
        assert_eq!(
            agents_label(&[run("Audit drag", RunState::Working)]),
            (Some("Audit drag".into()), "· Working".to_string())
        );
        assert_eq!(
            agents_label(&[run("Audit drag", RunState::Completed)]),
            (Some("Audit drag".into()), "· Completed".to_string())
        );
        assert_eq!(
            agents_label(&[run("Audit drag", RunState::Failed)]),
            (Some("Audit drag".into()), "· Failed".to_string())
        );
        let mixed = [
            run("a", RunState::Working),
            run("b", RunState::Completed),
            run("c", RunState::Working),
        ];
        assert_eq!(agents_label(&mixed), (None, "2 Working".to_string()));
        assert_eq!(roster_heading(&mixed), "2 of 3 working");
        assert_eq!(aggregate(&mixed), RunState::Working);
        let settled = [run("a", RunState::Completed), run("b", RunState::Failed)];
        assert_eq!(
            agents_label(&settled),
            (None, "1 Completed · 1 Failed".to_string())
        );
        let stopped = [run("a", RunState::Stopped), run("b", RunState::Stopped)];
        assert_eq!(agents_label(&stopped), (None, "2 Stopped".to_string()));
        assert_eq!(aggregate(&stopped), RunState::Stopped);
        assert_eq!(roster_heading(&settled), "Subagents");
        assert_eq!(aggregate(&settled), RunState::Failed);
        assert_eq!(
            aggregate(&[run("a", RunState::Completed)]),
            RunState::Completed
        );
        assert_eq!(shells_label(2), "2 Running");
    }

    #[test]
    fn the_phone_roster_reads_like_the_mockup() {
        let mixed = [
            run("a", RunState::Working),
            run("b", RunState::Completed),
            run("c", RunState::Working),
        ];
        assert_eq!(subagents_section(&mixed), "Subagents · 2 of 3 working");
        assert_eq!(
            subagents_section(&[run("a", RunState::Completed)]),
            "Subagents"
        );
        assert_eq!(shells_section(1), "Background shells · 1 Running");
        assert_eq!(
            roster_summary(&mixed, 1).as_deref(),
            Some("2 of 3 working · 1 shell")
        );
        assert_eq!(
            roster_summary(&[run("a", RunState::Completed)], 2).as_deref(),
            Some("1 Completed · 2 shells")
        );
        assert_eq!(roster_summary(&[], 0), None);
        assert_eq!(roster_summary(&[], 1).as_deref(), Some("1 shell"));

        let mut working = run("Audit drag", RunState::Working);
        working.started_ms = 1_000;
        working.model = Some("Sonnet 5".into());
        assert_eq!(working.line(None, 73_000), "Working · 1m 12s · Sonnet 5");
        working.model = None;
        assert_eq!(
            working.line(Some(HarnessId::Codex), 73_000),
            "Working · 1m 12s · Codex"
        );
        working.state = RunState::Completed;
        assert_eq!(working.line(None, 73_000), "Completed");

        let mut shell = task("s1", BackgroundTaskKind::Shell);
        assert_eq!(shell_line(&shell, 50_000), "Working");
        shell.started_at = chrono::DateTime::from_timestamp_millis(9_000);
        assert_eq!(shell_line(&shell, 50_000), "Working · 41s");
    }

    #[test]
    fn only_claude_code_stops_its_own_runs() {
        assert!(stoppable(Some(HarnessId::ClaudeCode)));
        assert!(!stoppable(Some(HarnessId::Codex)));
        let mut delegated = run("d", RunState::Working);
        delegated.delegated = true;
        assert!(delegated.can_stop(false));
        assert!(!run("a", RunState::Working).can_stop(false));
        assert!(!run("a", RunState::Completed).can_stop(true));
    }

    #[test]
    fn shell_list_keeps_only_shell_tasks() {
        let session = session(vec![
            task("s1", BackgroundTaskKind::Shell),
            task("a1", BackgroundTaskKind::Agent),
            task("o1", BackgroundTaskKind::Other),
            task("s2", BackgroundTaskKind::Shell),
        ]);
        let ids: Vec<String> = shell_tasks(Some(&session))
            .into_iter()
            .map(|task| task.id)
            .collect();
        assert_eq!(ids, ["s1", "s2"]);
        assert!(shell_tasks(None).is_empty());
    }

    #[test]
    fn the_live_line_is_the_last_non_blank_one() {
        let with_blank = vec!["  Finished dev ".to_string(), "  ".to_string()];
        assert_eq!(latest_line(&with_blank), Some("Finished dev"));
        assert_eq!(latest_line(&[]), None);
    }

    #[test]
    fn elapsed_text() {
        assert_eq!(elapsed_label(7), "7s");
        assert_eq!(elapsed_label(74), "1m 14s");
        assert_eq!(elapsed_label(3_725), "1h 02m");
        assert_eq!(elapsed_label(-3), "0s");
    }
}
