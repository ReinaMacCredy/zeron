use zeron_proto::{
    AccessMode, AgentProfile, AgentProfileDraft, AgentProfilesSnapshot, HarnessId, ReasoningLevel,
    profile_line,
};
use zeron_rpc::methods;

use super::{Page, Row, RowAction, RowKind, Section};
use crate::catalog::{HarnessInfo, ModelInfo, reasoning_label};
use crate::client::Client;
use crate::error::{ClientError, Result};

async fn snapshot(client: &Client, device_id: &str) -> Result<AgentProfilesSnapshot> {
    client
        .host_call(
            device_id,
            methods::LIST_AGENT_PROFILES,
            serde_json::json!({}),
        )
        .await
}

async fn system_prompt(client: &Client, device_id: &str) -> Result<String> {
    let value: serde_json::Value = client
        .host_call(device_id, methods::GET_SYSTEM_PROMPT, serde_json::json!({}))
        .await?;
    Ok(value["prompt"].as_str().unwrap_or_default().to_owned())
}

fn wire<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn parse<T: serde::de::DeserializeOwned>(value: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(value.to_owned())).ok()
}

fn model_label(models: &[ModelInfo], id: Option<&str>) -> Option<String> {
    let id = id?;
    Some(
        models
            .iter()
            .find(|m| m.id == id)
            .map_or_else(|| id.to_owned(), |m| m.label.clone()),
    )
}

pub(super) fn count_line(snapshot: &AgentProfilesSnapshot) -> String {
    match snapshot.profiles.len() {
        0 => "None yet".into(),
        1 => "1 profile".into(),
        n => format!("{n} profiles"),
    }
}

fn split(page: &str) -> Option<(&str, Option<&str>)> {
    let rest = page.strip_prefix("profile:")?;
    Some(match rest.split_once(':') {
        Some((id, field)) => (id, Some(field)),
        None => (rest, None),
    })
}

pub(super) async fn page(client: &Client, device_id: &str, page: &str) -> Result<Page> {
    let snapshot = snapshot(client, device_id).await?;
    if page == "profiles" {
        let prompt = system_prompt(client, device_id).await.unwrap_or_default();
        let mut labels = Vec::new();
        for profile in &snapshot.profiles {
            let models = client.list_models(device_id, &wire(profile.harness)).await;
            labels.push(model_label(&models, profile.model.as_deref()));
        }
        return Ok(list_page(&snapshot, &labels, &prompt));
    }
    let (id, field) = split(page).ok_or_else(|| ClientError::NotFound(page.to_owned()))?;
    let profile = snapshot
        .profiles
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| ClientError::NotFound(id.to_owned()))?;
    let models = client.list_models(device_id, &wire(profile.harness)).await;
    Ok(match field {
        None => editor_page(profile, &models),
        Some("agent") => agent_page(profile, &client.list_harnesses(device_id).await),
        Some("model") => model_page(profile, &models),
        Some("mode") => {
            let harnesses = client.list_harnesses(device_id).await;
            let harness = harnesses.iter().find(|h| h.id == wire(profile.harness));
            mode_page(profile, harness.and_then(|h| h.access_modes.as_deref()))
        }
        Some("effort") => effort_page(profile, &models),
        Some(tools) if tools == "tools" || tools.starts_with("tools:") => {
            super::profile_tools::page(client, device_id, profile, tools).await?
        }
        Some(other) => return Err(ClientError::NotFound(other.to_owned())),
    })
}

pub(super) async fn act(
    client: &Client,
    device_id: &str,
    page: &str,
    action: &str,
    value: Option<String>,
) -> Result<bool> {
    if action == "system-prompt" {
        client
            .inner
            .host_rpc(
                device_id,
                methods::SET_SYSTEM_PROMPT,
                serde_json::json!({ "prompt": value.unwrap_or_default() }),
            )
            .await?;
        return Ok(false);
    }
    if action == "new" {
        let snapshot = snapshot(client, device_id).await?;
        let draft = AgentProfileDraft {
            name: unused_name(&snapshot),
            ..blank(HarnessId::ClaudeCode)
        };
        upsert(client, device_id, None, &draft).await?;
        return Ok(false);
    }
    let moved = action
        .strip_prefix("up:")
        .map(|id| (id, true))
        .or_else(|| action.strip_prefix("down:").map(|id| (id, false)));
    if page == "profiles"
        && let Some((id, up)) = moved
    {
        let mut ids: Vec<String> = snapshot(client, device_id)
            .await?
            .profiles
            .into_iter()
            .map(|p| p.id)
            .collect();
        let from = ids
            .iter()
            .position(|p| p == id)
            .ok_or_else(|| ClientError::NotFound(id.to_owned()))?;
        let to = if up {
            from.checked_sub(1)
        } else {
            (from + 1 < ids.len()).then_some(from + 1)
        };
        if let Some(to) = to {
            ids.swap(from, to);
            client
                .inner
                .host_rpc(
                    device_id,
                    methods::REORDER_AGENT_PROFILES,
                    serde_json::json!({ "profileIds": ids }),
                )
                .await?;
        }
        return Ok(false);
    }
    let (id, field) = split(page).ok_or_else(|| ClientError::InvalidArgument(page.to_owned()))?;
    if action == "remove" {
        client
            .inner
            .host_rpc(
                device_id,
                methods::DELETE_AGENT_PROFILE,
                serde_json::json!({ "profileId": id }),
            )
            .await?;
        return Ok(true);
    }
    let snapshot = snapshot(client, device_id).await?;
    let profile = snapshot
        .profiles
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| ClientError::NotFound(id.to_owned()))?;
    let mut draft = profile.draft();
    if let Some(field) = field.filter(|f| f.starts_with("tools")) {
        let on = value.as_deref() == Some("true");
        super::profile_tools::act(
            client,
            device_id,
            &mut draft.disabled_tools,
            field,
            action,
            on,
        )
        .await?;
        upsert(client, device_id, Some(id), &draft).await?;
        return Ok(false);
    }
    let models = match action.strip_prefix("agent:") {
        Some(harness) => client.list_models(device_id, harness).await,
        None => client.list_models(device_id, &wire(profile.harness)).await,
    };
    if !edit(&mut draft, action, value, &models) {
        return Err(ClientError::InvalidArgument(action.to_owned()));
    }
    upsert(client, device_id, Some(id), &draft).await?;
    Ok(false)
}

async fn upsert(
    client: &Client,
    device_id: &str,
    id: Option<&str>,
    draft: &AgentProfileDraft,
) -> Result<()> {
    client
        .inner
        .host_rpc(
            device_id,
            methods::UPSERT_AGENT_PROFILE,
            serde_json::json!({ "profileId": id, "profile": draft }),
        )
        .await?;
    Ok(())
}

fn blank(harness: HarnessId) -> AgentProfileDraft {
    AgentProfileDraft {
        name: String::new(),
        harness,
        model: None,
        reasoning: None,
        access: None,
        model_options: Default::default(),
        notes: String::new(),
        instructions: String::new(),
        disabled_tools: Vec::new(),
    }
}

fn unused_name(snapshot: &AgentProfilesSnapshot) -> String {
    (1..)
        .map(|n| {
            if n == 1 {
                "New profile".to_owned()
            } else {
                format!("New profile {n}")
            }
        })
        .find(|name| !snapshot.profiles.iter().any(|p| &p.name == name))
        .unwrap_or_default()
}

fn edit(
    draft: &mut AgentProfileDraft,
    action: &str,
    value: Option<String>,
    models: &[ModelInfo],
) -> bool {
    let levels = |model: Option<&str>| -> Vec<String> {
        models
            .iter()
            .find(|m| Some(m.id.as_str()) == model)
            .map(|m| m.reasoning_levels.clone())
            .unwrap_or_default()
    };
    match action {
        "name" => draft.name = value.unwrap_or_default(),
        "instructions" => draft.instructions = value.unwrap_or_default(),
        "notes" => draft.notes = value.unwrap_or_default(),
        _ => {
            if let Some(harness) = action.strip_prefix("agent:") {
                let Some(harness) = parse::<HarnessId>(harness) else {
                    return false;
                };
                if harness != draft.harness {
                    draft.harness = harness;
                    draft.model = None;
                    draft.reasoning = None;
                    draft.model_options.clear();
                }
            } else if let Some(model) = action.strip_prefix("model:") {
                draft.model = (!model.is_empty()).then(|| model.to_owned());
                draft.model_options.clear();
                if draft
                    .reasoning
                    .is_some_and(|r| !levels(draft.model.as_deref()).contains(&wire(r)))
                {
                    draft.reasoning = None;
                }
            } else if let Some(mode) = action.strip_prefix("mode:") {
                draft.access = if mode.is_empty() {
                    None
                } else {
                    match parse::<AccessMode>(mode) {
                        Some(mode) => Some(mode),
                        None => return false,
                    }
                };
            } else if let Some(level) = action.strip_prefix("effort:") {
                draft.reasoning = if level.is_empty() {
                    None
                } else {
                    match parse::<ReasoningLevel>(level) {
                        Some(level) => Some(level),
                        None => return false,
                    }
                };
            } else {
                return false;
            }
        }
    }
    true
}

fn list_page(snapshot: &AgentProfilesSnapshot, labels: &[Option<String>], prompt: &str) -> Page {
    let last = snapshot.profiles.len().saturating_sub(1);
    let mut rows: Vec<Row> = snapshot
        .profiles
        .iter()
        .zip(labels)
        .enumerate()
        .map(|(ix, (profile, label))| {
            let mut row = Row::new(
                format!("profile:{}", profile.id),
                profile.name.clone(),
                RowKind::Link {
                    page: format!("profile:{}", profile.id),
                },
            )
            .value(profile.by_agent.then(|| "by agent".to_owned()))
            .subtitle(Some(profile_line(
                profile.harness.display_name(),
                label.as_deref(),
                profile.access,
                profile.reasoning,
                profile.disabled_tools.len(),
            )));
            for (dir, title, show) in [("up", "Move up", ix > 0), ("down", "Move down", ix < last)]
            {
                if show {
                    row.actions.push(RowAction {
                        id: format!("{dir}:{}", profile.id),
                        title: title.into(),
                        destructive: false,
                        confirm: None,
                    });
                }
            }
            row
        })
        .collect();
    rows.push(Row::new(
        "new",
        "New profile",
        RowKind::Action { destructive: false },
    ));
    Page {
        title: "Agent profiles".into(),
        sections: vec![
            Section {
                title: None,
                footer: Some(
                    "Saved agent, model, mode and effort. Pick one from the model picker, or let \
                     an agent pick one for delegate_task."
                        .into(),
                ),
                rows,
            },
            Section {
                title: Some("System prompt".into()),
                footer: Some(
                    "Added to every agent's system prompt on this device, before a profile's own \
                     instructions."
                        .into(),
                ),
                rows: vec![
                    Row::new(
                        "system-prompt",
                        "System prompt",
                        RowKind::Text {
                            multiline: true,
                            placeholder: Some("None".into()),
                        },
                    )
                    .value(Some(prompt.to_owned())),
                ],
            },
        ],
    }
}

fn link(profile: &AgentProfile, field: &str, title: &str, value: Option<String>) -> Row {
    Row::new(
        field,
        title,
        RowKind::Link {
            page: format!("profile:{}:{field}", profile.id),
        },
    )
    .value(value)
}

fn text(id: &str, title: &str, value: &str, multiline: bool, placeholder: &str) -> Row {
    Row::new(
        id,
        title,
        RowKind::Text {
            multiline,
            placeholder: Some(placeholder.into()),
        },
    )
    .value(Some(value.to_owned()))
}

fn editor_page(profile: &AgentProfile, models: &[ModelInfo]) -> Page {
    let model = models
        .iter()
        .find(|m| profile.model.as_deref() == Some(m.id.as_str()));
    let model_value = match (model, profile.model.as_deref()) {
        (Some(m), _) if m.is_new => format!("{} · NEW", m.label),
        (Some(m), _) => m.label.clone(),
        (None, Some(id)) => id.to_owned(),
        (None, None) => "Default".to_owned(),
    };
    let tools_off = profile.disabled_tools.len();
    let mut remove = Row::new("remove", "Remove", RowKind::Action { destructive: true });
    remove.actions.push(RowAction {
        id: "remove".into(),
        title: "Remove".into(),
        destructive: true,
        confirm: Some(format!("Remove {}?", profile.name)),
    });
    Page {
        title: profile.name.clone(),
        sections: vec![
            Section {
                title: None,
                footer: None,
                rows: vec![
                    text("name", "Name", &profile.name, false, "Name"),
                    link(
                        profile,
                        "agent",
                        "Agent",
                        Some(profile.harness.display_name().to_owned()),
                    ),
                    link(profile, "model", "Model", Some(model_value)),
                    link(
                        profile,
                        "mode",
                        "Mode",
                        Some(
                            profile
                                .access
                                .map_or("Default", AccessMode::label)
                                .to_owned(),
                        ),
                    ),
                    link(
                        profile,
                        "effort",
                        "Effort",
                        Some(
                            profile
                                .reasoning
                                .map_or("Default", ReasoningLevel::label)
                                .to_owned(),
                        ),
                    ),
                ],
            },
            Section {
                title: None,
                footer: Some("Agents read this to pick a profile for delegate_task.".into()),
                rows: vec![
                    text(
                        "instructions",
                        "Instructions",
                        &profile.instructions,
                        true,
                        "None",
                    ),
                    text("notes", "When to use", &profile.notes, true, "None"),
                ],
            },
            Section {
                title: None,
                footer: None,
                rows: vec![link(
                    profile,
                    "tools",
                    "Tools",
                    Some(if tools_off == 0 {
                        "All on".to_owned()
                    } else {
                        format!("{tools_off} tools off")
                    }),
                )],
            },
            Section {
                title: None,
                footer: None,
                rows: vec![remove],
            },
        ],
    }
}

fn choice_page(title: &str, rows: Vec<Row>) -> Page {
    Page {
        title: title.into(),
        sections: vec![Section {
            rows,
            ..Section::default()
        }],
    }
}

fn agent_page(profile: &AgentProfile, harnesses: &[HarnessInfo]) -> Page {
    let rows = harnesses
        .iter()
        .filter(|h| h.installed && h.enabled != Some(false))
        .filter(|h| parse::<HarnessId>(&h.id).is_some_and(|id| id != HarnessId::Acp))
        .map(|h| {
            Row::new(
                format!("agent:{}", h.id),
                h.label.clone(),
                RowKind::Check {
                    on: wire(profile.harness) == h.id,
                },
            )
        })
        .collect();
    choice_page("Agent", rows)
}

fn model_page(profile: &AgentProfile, models: &[ModelInfo]) -> Page {
    let mut rows = vec![Row::new(
        "model:",
        "Default",
        RowKind::Check {
            on: profile.model.is_none(),
        },
    )];
    rows.extend(models.iter().map(|m| {
        Row::new(
            format!("model:{}", m.id),
            m.label.clone(),
            RowKind::Check {
                on: profile.model.as_deref() == Some(m.id.as_str()),
            },
        )
        .value(m.is_new.then(|| "NEW".to_owned()))
        .subtitle(m.description.clone())
    }));
    choice_page("Model", rows)
}

fn mode_page(profile: &AgentProfile, offered: Option<&[AccessMode]>) -> Page {
    let mut rows = vec![
        Row::new(
            "mode:",
            "Default",
            RowKind::Check {
                on: profile.access.is_none(),
            },
        )
        .subtitle(Some("The chat's own mode".to_owned())),
    ];
    let offered = offered.filter(|offered| !offered.is_empty());
    rows.extend(
        AccessMode::ALL
            .iter()
            .filter(|mode| offered.is_none_or(|offered| offered.contains(mode)))
            .map(|mode| {
                Row::new(
                    format!("mode:{}", wire(*mode)),
                    mode.label(),
                    RowKind::Check {
                        on: profile.access == Some(*mode),
                    },
                )
                .subtitle(Some(mode.description().to_owned()))
            }),
    );
    choice_page("Mode", rows)
}

fn effort_page(profile: &AgentProfile, models: &[ModelInfo]) -> Page {
    let levels = models
        .iter()
        .find(|m| profile.model.as_deref() == Some(m.id.as_str()))
        .or_else(|| models.first())
        .map(|m| m.reasoning_levels.clone())
        .unwrap_or_default();
    let mut rows = vec![Row::new(
        "effort:",
        "Default",
        RowKind::Check {
            on: profile.reasoning.is_none(),
        },
    )];
    rows.extend(levels.iter().map(|level| {
        Row::new(
            format!("effort:{level}"),
            reasoning_label(level),
            RowKind::Check {
                on: profile.reasoning.map(wire).as_deref() == Some(level.as_str()),
            },
        )
    }));
    choice_page("Effort", rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, levels: &[&str]) -> ModelInfo {
        serde_json::from_value(serde_json::json!({
            "id": id, "label": id, "reasoningLevels": levels,
        }))
        .unwrap()
    }

    #[test]
    fn switching_agent_clears_model_and_effort_and_a_model_keeps_only_its_levels() {
        let mut draft = blank(HarnessId::ClaudeCode);
        let models = [model("big", &["low", "high"]), model("small", &["low"])];
        assert!(edit(&mut draft, "model:big", None, &models));
        assert!(edit(&mut draft, "effort:high", None, &models));
        assert!(edit(&mut draft, "mode:auto-accept-edits", None, &models));
        assert_eq!(
            profile_line(
                "Claude Code",
                draft.model.as_deref(),
                draft.access,
                draft.reasoning,
                0
            ),
            "Claude Code · big · Auto-accept edits · High"
        );
        assert!(edit(&mut draft, "model:small", None, &models));
        assert_eq!(draft.reasoning, None);
        assert!(edit(&mut draft, "agent:codex", None, &[]));
        assert_eq!(draft.harness, HarnessId::Codex);
        assert_eq!(draft.model, None);
        assert!(edit(&mut draft, "name", Some("UI work".into()), &[]));
        assert_eq!(draft.name, "UI work");
        assert!(!edit(&mut draft, "mode:sometimes", None, &[]));
    }

    #[test]
    fn page_ids_split_into_profile_and_field() {
        assert_eq!(split("profile:ui-work"), Some(("ui-work", None)));
        assert_eq!(
            split("profile:ui-work:model"),
            Some(("ui-work", Some("model")))
        );
        assert_eq!(split("profiles"), None);
    }
}
