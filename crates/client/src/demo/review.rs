use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use zeron_proto::patch::{LineKind, line_anchor, parse_patch};
use zeron_proto::review::{self, ReviewScope, ReviewerChoice, ReviewerOptions};
use zeron_proto::{
    AgentProfilesSnapshot, Chat, ChatConfig, Delegation, DelegationStatus, DelegationWake,
    FindingOutcome, FindingOutcomeKind, HarnessId,
};
use zeron_rpc::methods as m;

use super::DemoHost;
use crate::client::ClientInner;
use crate::error::{ClientError, Result};
use crate::lock;

pub(super) const OWNER: &str = "chat-tabs";
const SEEDED: &str = "chat-tabs-review";
const REVIEW_DELAY: Duration = Duration::from_secs(4);
const FIX_DELAY: Duration = Duration::from_secs(5);
const FIX_SHA: &str = "4e1c9a7f2b3d5e6f7a8b9c0d1e2f3a4b5c6d7e8f";

const NOTES: [(&str, &str, &str); 6] = [
    (
        "bug",
        "Status cache is never invalidated on reconnect",
        "A relay reconnect keeps the stale entry, so the board shows the old CI state until a restart.",
    ),
    (
        "risk",
        "The row reads the cache on the main thread",
        "Each scroll frame takes the cache lock; a slow writer stalls the list.",
    ),
    ("nit", "Name the closure after what it returns", ""),
    (
        "bug",
        "Dropped guard lets a missing status crash the row",
        "The removed early return was the only check before the force unwrap.",
    ),
    (
        "risk",
        "The poll interval is hardcoded",
        "Read it from the host settings so a slow relay can back off.",
    ),
    ("nit", "Duplicate import", ""),
];

fn findings_json(patch: &str) -> Vec<Value> {
    let lines: Vec<(String, &'static str, u32, String)> = parse_patch(patch)
        .iter()
        .flat_map(|file| {
            file.hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .filter(|line| matches!(line.kind, LineKind::Add | LineKind::Del))
                .filter_map(|line| {
                    let (side, no) = line_anchor(line)?;
                    Some((file.path.clone(), side.tag(), no, line.text.clone()))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let step = (lines.len() / NOTES.len()).max(1);
    NOTES
        .iter()
        .enumerate()
        .map(|(ix, (severity, title, body))| {
            let (path, side, line, text) = &lines[(ix * step + ix % 2) % lines.len()];
            json!({
                "path": path, "side": side, "line": line, "text": text,
                "severity": severity, "title": title, "body": body,
            })
        })
        .collect()
}

fn summary(findings: &[Value]) -> String {
    format!(
        "I read every changed file. {} things are worth a look.\n\n```zeron-review\n{}\n```\n",
        findings.len(),
        serde_json::to_string_pretty(findings).unwrap_or_default()
    )
}

fn reviewer_config(
    owner: &Chat,
    harness: Option<HarnessId>,
    model: Option<String>,
) -> Option<ChatConfig> {
    let harness = harness.unwrap_or(HarnessId::ClaudeCode);
    let model = model.unwrap_or_else(|| {
        match harness {
            HarnessId::Codex => "gpt-5.5",
            _ => "claude-opus-5-5",
        }
        .to_owned()
    });
    owner.config.clone().map(|base| ChatConfig {
        harness,
        model: Some(model),
        reasoning: None,
        profile_id: None,
        instance_id: None,
        ..base
    })
}

struct Child<'a> {
    owner: &'a Chat,
    id: String,
    request: String,
    config: Option<ChatConfig>,
    status: DelegationStatus,
    summary: Option<String>,
    started_at: DateTime<Utc>,
    outcomes: BTreeMap<String, FindingOutcome>,
}

fn child(spec: Child<'_>) -> Chat {
    let done = spec.status != DelegationStatus::Running;
    Chat {
        id: spec.id,
        title: Some("Review changes".into()),
        archived: false,
        config: spec.config,
        last_message_preview: spec.summary.as_ref().map(|_| "Review finished".into()),
        last_message_at: Some(spec.started_at),
        created_at: spec.started_at,
        last_seen_at: Some(spec.started_at),
        harness_session_id: None,
        harness_session_cwd: None,
        sidechat_source_chat_id: None,
        created_by_chat_id: Some(spec.owner.id.clone()),
        forked_from_chat_id: None,
        previous_harnesses: Vec::new(),
        settle: None,
        unsettled_at: None,
        delegation: Some(Delegation {
            delegated_by_chat_id: spec.owner.id.clone(),
            depth: 1,
            anchor: "m1".into(),
            client_request_id: Some(spec.request),
            owner_turn_started_at: None,
            wake: DelegationWake::Never,
            status: spec.status,
            cancel_requested: false,
            delivery: None,
            summary: spec.summary,
            started_at: spec.started_at,
            completed_at: done.then(|| spec.started_at + chrono::Duration::seconds(48)),
            finding_outcomes: spec.outcomes,
            findings_handed: Default::default(),
            findings_announced: Default::default(),
            route: None,
        }),
        ..spec.owner.clone()
    }
}

fn outcome(kind: FindingOutcomeKind, note: Option<&str>, commit: Option<&str>) -> FindingOutcome {
    FindingOutcome {
        outcome: kind,
        note: note.map(str::to_owned),
        commit: commit.map(str::to_owned),
    }
}

pub(super) fn seeded_child(owner: &Chat) -> Chat {
    let patch = super::git::GitDemo::new()
        .checkout_diff(&json!({ "mode": ReviewScope::Uncommitted.mode() }))
        .map(|diff| diff.patch)
        .unwrap_or_default();
    let outcomes = BTreeMap::from([
        (
            "F2".to_owned(),
            outcome(FindingOutcomeKind::Fixed, None, Some(FIX_SHA)),
        ),
        (
            "F3".to_owned(),
            outcome(
                FindingOutcomeKind::Skipped,
                Some("The closure is one line and reads fine where it is."),
                None,
            ),
        ),
        (
            "F5".to_owned(),
            outcome(
                FindingOutcomeKind::NoChange,
                Some("The relay retry already backs off; the interval only sets the first wait."),
                None,
            ),
        ),
    ]);
    child(Child {
        owner,
        id: SEEDED.into(),
        request: review::request_id(ReviewScope::Uncommitted, None, "demo"),
        config: reviewer_config(owner, Some(HarnessId::ClaudeCode), None),
        status: DelegationStatus::Completed,
        summary: Some(summary(&findings_json(&patch))),
        started_at: owner.last_message_at.unwrap_or_else(Utc::now) - chrono::Duration::minutes(6),
        outcomes,
    })
}

fn upsert(client: &Arc<ClientInner>, chat: &Chat) -> Result<()> {
    client.registry_write(|doc| doc.upsert_chat(chat))
}

fn delegated_child(client: &ClientInner, owner: &str, task: &str) -> Result<(Chat, Delegation)> {
    client
        .workspace
        .chat(task)
        .and_then(|chat| {
            let delegation = chat
                .delegation
                .clone()
                .filter(|d| d.delegated_by_chat_id == owner)?;
            Some((chat, delegation))
        })
        .ok_or_else(|| ClientError::NotFound(task.to_owned()))
}

fn save(client: &Arc<ClientInner>, mut chat: Chat, delegation: Delegation) -> Result<()> {
    chat.delegation = Some(delegation);
    upsert(client, &chat)
}

fn text<'a>(params: &'a Value, key: &str) -> &'a str {
    params[key].as_str().unwrap_or_default()
}

impl DemoHost {
    pub(super) fn review_call(
        self: &Arc<Self>,
        client: &Arc<ClientInner>,
        device_id: &str,
        method: &str,
        params: &Value,
    ) -> Option<Result<Value>> {
        Some(match method {
            m::LIST_BRANCHES => Ok(json!(["main", "feat/pr-status", "fix/tool-colors"])),
            m::START_REVIEW => self.start_review(client, device_id, params),
            m::TASK_STATUS => {
                delegated_child(client, text(params, "chatId"), text(params, "taskId")).map(
                    |(chat, delegation)| {
                        json!({
                            "taskId": chat.id,
                            "status": delegation.status,
                            "summary": delegation.summary.unwrap_or_default(),
                        })
                    },
                )
            }
            m::TASK_CANCEL => {
                delegated_child(client, text(params, "chatId"), text(params, "taskId")).and_then(
                    |(chat, mut delegation)| {
                        if delegation.is_running() {
                            delegation.status = DelegationStatus::Cancelled;
                            delegation.completed_at = Some(Utc::now());
                        }
                        let reply = json!({ "taskId": chat.id, "status": delegation.status });
                        save(client, chat, delegation)?;
                        Ok(reply)
                    },
                )
            }
            m::HAND_REVIEW_FINDINGS => {
                delegated_child(client, text(params, "ownerChatId"), text(params, "taskId"))
                    .and_then(|(chat, mut delegation)| {
                        let ids: Vec<String> =
                            serde_json::from_value(params["ids"].clone()).unwrap_or_default();
                        let withdraw = params["withdraw"].as_bool().unwrap_or(false);
                        for id in &ids {
                            if withdraw {
                                delegation.findings_handed.remove(id);
                            } else {
                                delegation.findings_handed.insert(id.clone());
                            }
                        }
                        save(client, chat, delegation)?;
                        Ok(json!({ "handed": ids }))
                    })
            }
            _ => return None,
        })
    }

    fn reviewer_options(&self, device_id: &str) -> ReviewerOptions {
        let profiles = lock(&self.settings)
            .call(device_id, m::LIST_AGENT_PROFILES, &Value::Null)
            .ok()
            .and_then(|value| serde_json::from_value::<AgentProfilesSnapshot>(value).ok())
            .map(|snapshot| snapshot.profiles)
            .unwrap_or_default();
        ReviewerOptions {
            offered: vec![HarnessId::ClaudeCode, HarnessId::Codex],
            profiles,
            levels: Default::default(),
        }
    }

    fn start_review(
        self: &Arc<Self>,
        client: &Arc<ClientInner>,
        device_id: &str,
        params: &Value,
    ) -> Result<Value> {
        let owner = client
            .workspace
            .chat(text(params, "ownerChatId"))
            .ok_or_else(|| ClientError::HostError("chat not found".into()))?;
        let scope = ReviewScope::from_key(text(params, "scope"))
            .ok_or_else(|| ClientError::HostError("unknown review scope".into()))?;
        let commit = params["commitSha"].as_str().map(str::to_owned);
        let diff = lock(&self.git).checkout_diff(&json!({
            "mode": scope.mode(),
            "commitSha": commit,
        }))?;
        if diff.files.is_empty() {
            return Err(ClientError::HostError("Nothing to review".into()));
        }
        let choice: ReviewerChoice = serde_json::from_value(params["reviewer"].clone())
            .ok()
            .or_else(|| {
                lock(&self.settings)
                    .call(device_id, m::GET_REVIEW_SETTINGS, &Value::Null)
                    .ok()
                    .and_then(|value| serde_json::from_value::<review::ReviewSettings>(value).ok())
                    .map(|settings| settings.reviewer)
            })
            .unwrap_or_default();
        let reviewer = review::resolve_reviewer(
            &choice,
            owner.config.as_ref().map(|config| config.harness),
            &self.reviewer_options(device_id),
        );
        let task_id = format!("review-{}", crate::new_id());
        let running = child(Child {
            owner: &owner,
            id: task_id.clone(),
            request: review::request_id(scope, commit.as_deref(), &crate::new_id()),
            config: reviewer_config(&owner, reviewer.harness, reviewer.model.clone()),
            status: DelegationStatus::Running,
            summary: None,
            started_at: Utc::now(),
            outcomes: BTreeMap::new(),
        });
        upsert(client, &running)?;
        let findings = summary(&findings_json(&diff.patch));
        let weak = Arc::downgrade(client);
        let task = task_id.clone();
        crate::runtime::shared().spawn(async move {
            tokio::time::sleep(REVIEW_DELAY).await;
            let Some(client) = weak.upgrade() else { return };
            let Some(mut chat) = client.workspace.chat(&task) else {
                return;
            };
            let Some(d) = chat.delegation.as_mut().filter(|d| d.is_running()) else {
                return;
            };
            d.status = DelegationStatus::Completed;
            d.completed_at = Some(Utc::now());
            d.summary = Some(findings);
            chat.last_message_preview = Some("Review finished".into());
            let _ = upsert(&client, &chat);
        });
        Ok(json!({
            "taskId": task_id,
            "patch": diff.patch,
            "note": reviewer.note,
            "reviewer": reviewer.name(),
            "scope": scope.key(),
            "commitSha": commit,
        }))
    }
}

fn cited_findings(prompt: &str) -> Vec<String> {
    prompt
        .split("[F")
        .skip(1)
        .filter_map(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            (!digits.is_empty() && rest[digits.len()..].starts_with(']'))
                .then(|| format!("F{digits}"))
        })
        .collect()
}

pub(super) fn prompt_sent(client: &Arc<ClientInner>, owner: &str, prompt: &str) {
    let ids = cited_findings(prompt);
    if ids.is_empty() {
        return;
    }
    let weak = Arc::downgrade(client);
    let owner = owner.to_owned();
    crate::runtime::shared().spawn(async move {
        tokio::time::sleep(FIX_DELAY).await;
        let Some(client) = weak.upgrade() else { return };
        let (workspace, _) = client.workspace.state();
        let Some(mut chat) = review::reviewer_child(&workspace.chats, &owner, None).cloned() else {
            return;
        };
        let Some(d) = chat.delegation.as_mut() else {
            return;
        };
        for id in ids {
            if d.findings_handed.contains(&id) && !d.finding_outcomes.contains_key(&id) {
                d.finding_outcomes
                    .insert(id, outcome(FindingOutcomeKind::Fixed, None, Some(FIX_SHA)));
            }
        }
        let _ = upsert(&client, &chat);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_findings_place_in_the_demo_working_tree() {
        let patch = super::super::git::GitDemo::new()
            .checkout_diff(&json!({ "mode": "workingTree" }))
            .unwrap()
            .patch;
        let mut run = review::ReviewRun::new(ReviewScope::Uncommitted);
        run.apply_read(
            review::read_outcome("completed", &summary(&findings_json(&patch))),
            Some(&patch),
            true,
        );
        assert_eq!(run.findings.len(), NOTES.len());
        assert!(
            run.findings
                .iter()
                .all(|finding| finding.placement == review::Placement::Fresh)
        );
    }

    #[test]
    fn cited_findings_reads_the_comment_block_ids() {
        let prompt = "Address the review comments below.\n\n- a.rs:3 (R): Bug: [F1] x\n- b.rs:9 (L): Nit: [F12] y\n- [Fx] no";
        assert_eq!(cited_findings(prompt), vec!["F1", "F12"]);
    }
}
