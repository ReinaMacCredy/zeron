use serde::Deserialize;
use zeron_doc::MessagePart;
use zeron_proto::{ChatConfig, HarnessId};

use crate::EngineError;
use crate::doc_host::DocHost;
use crate::sessions::SessionsEngine;
use crate::workspace_host::WorkspaceHost;

/// A host-written transcript boundary riding on a config mutation: the
/// divider label plus the hint shown behind its info glyph.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemNote {
    pub text: String,
    #[serde(default)]
    pub hint: Option<String>,
}

/// Switch a chat's config, closing the old harness session and writing the divider with the handoff packet.
pub async fn apply(
    doc_host: &DocHost,
    sessions: &SessionsEngine,
    workspace: &WorkspaceHost,
    chat_id: &str,
    config: &ChatConfig,
    note: Option<SystemNote>,
) -> Result<(), EngineError> {
    let left_config = workspace.chat_config(&chat_id);
    let left_instance = left_config.as_ref().and_then(|c| c.instance_id.clone());
    let left_model = left_config.and_then(|c| c.model);
    let left = doc_host.harness_for(&chat_id);
    // Another instance of the same driver is another home and login: its session cannot resume.
    let switching = left != config.harness || left_instance != config.instance_id;
    if switching {
        sessions.switch_harness(&chat_id).await?;
    }
    let handle = (note.is_some() && doc_host.is_host(&chat_id))
        .then(|| doc_host.open(&chat_id))
        .transpose()?;
    // A change on top of a handoff nobody has sent to yet folds
    // into that divider instead of stacking a second one: the chat
    // still left the harness the first divider named, and the one
    // in between never ran, so it leaves no mark either.
    let replaced = handle
        .as_ref()
        .and_then(|handle| unanswered_switch_note(handle));
    if replaced.is_some() && switching {
        workspace.set_chat_config_after_unsent_switch(&chat_id, &config)
    } else {
        workspace.set_chat_config(&chat_id, &config)
    }?;
    let (Some(handle), Some(mut note)) = (handle, note) else {
        return Ok(());
    };
    let handoff = match replaced {
        Some(previous) => {
            handle.doc().truncate_from(&previous.id)?;
            match fold_switch_note(&previous, left, &config, &note) {
                Some((folded, route)) => {
                    note = folded;
                    Some((previous.packet, route))
                }
                // Back where it started: the transcript shows no
                // switch, and the closed session hands over on the
                // next turn like any re-sent first turn.
                None => return Ok(()),
            }
        }
        // The packet is built now, at the switch, so the divider
        // shows it at once; the first run sends this same text.
        None => switching.then(|| {
            let packet = sessions.provider_switch_packet(chat_id).unwrap_or_default();
            let route = zeron_doc::HandoffRoute {
                from_harness: Some(left),
                from_model: left_model,
                to_model: config.model.clone(),
                ..Default::default()
            };
            (packet, route)
        }),
    };
    handle.write_system_note_with(
        &crate::new_id(),
        &note.text,
        note.hint.as_deref(),
        handoff
            .as_ref()
            .map(|(packet, route)| (config.harness, packet.as_str(), route.clone())),
        crate::now_ms(),
    )?;
    Ok(())
}

/// A handoff divider nothing followed: the chat's newest entry is a system
/// note carrying a packet whose route names the harness the chat left.
struct SwitchNote {
    id: String,
    text: String,
    hint: Option<String>,
    packet: String,
    route: zeron_doc::HandoffRoute,
}

fn unanswered_switch_note(handle: &crate::doc_host::ChatDocHandle) -> Option<SwitchNote> {
    let entries = handle.doc().read_entries().ok()?;
    let last = entries.last()?;
    if last.role != zeron_doc::MessageRole::System {
        return None;
    }
    let (packet, route) = last.parts.iter().find_map(|part| match part {
        MessagePart::Handoff { text, route, .. } if route.from_harness.is_some() => {
            Some((text.clone(), route.clone()))
        }
        _ => None,
    })?;
    let mut texts = last.parts.iter().filter_map(|part| match part {
        MessagePart::Text { text, .. } => Some(text.clone()),
        _ => None,
    });
    Some(SwitchNote {
        id: last.id.clone(),
        text: texts.next().unwrap_or_default(),
        hint: texts.next(),
        packet,
        route,
    })
}

/// The divider of a change folded onto an unanswered handoff: it keeps the
/// origin the first divider recorded and takes the new destination. `None`
/// when that is where the chat already was. A seeded thread's divider is its
/// only copy of the source conversation, so it is never dropped.
fn fold_switch_note(
    previous: &SwitchNote,
    between: HarnessId,
    config: &zeron_proto::ChatConfig,
    next: &SystemNote,
) -> Option<(SystemNote, zeron_doc::HandoffRoute)> {
    let origin = previous.route.from_harness?;
    let route = zeron_doc::HandoffRoute {
        to_model: config.model.clone(),
        ..previous.route.clone()
    };
    let seeded = route.source_chat_id.is_some();
    if !seeded && config.harness == origin && config.model == route.from_model {
        return None;
    }
    let note = if seeded || between == config.harness {
        SystemNote {
            text: previous.text.clone(),
            hint: previous.hint.clone(),
        }
    } else if config.harness == origin {
        SystemNote {
            text: format!(
                "Model changed from {} to {}.",
                route.from_model.as_deref().unwrap_or("default"),
                route.to_model.as_deref().unwrap_or("default"),
            ),
            hint: None,
        }
    } else {
        SystemNote {
            text: format!(
                "Provider changed from {} to {}.",
                origin.display_name(),
                config.harness.display_name()
            ),
            hint: next
                .hint
                .as_ref()
                .map(|hint| hint.replace(between.display_name(), origin.display_name())),
        }
    };
    Some((note, route))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folded_switch_note_keeps_the_harness_the_chat_left() {
        let config = |harness: &str, model: &str| -> zeron_proto::ChatConfig {
            serde_json::from_value(serde_json::json!({
                "harness": harness, "model": model, "modelOptions": {},
                "sandbox": "workspace-write",
            }))
            .unwrap()
        };
        let previous = SwitchNote {
            id: "n1".into(),
            text: "Provider changed from Mock to Codex.".into(),
            hint: Some("Mock session closed. Handed to Codex.".into()),
            packet: "packet".into(),
            route: zeron_doc::HandoffRoute {
                from_harness: Some(HarnessId::Mock),
                from_model: Some("m1".into()),
                to_model: Some("gpt".into()),
                ..Default::default()
            },
        };
        let next = SystemNote {
            text: "Provider changed from Codex to Grok.".into(),
            hint: Some(
                "Codex session closed. Handed to Grok; Codex's files were not carried.".into(),
            ),
        };
        let (folded, route) =
            fold_switch_note(&previous, HarnessId::Codex, &config("grok", "g"), &next).unwrap();
        assert_eq!(folded.text, "Provider changed from Mock to Grok.");
        assert_eq!(
            folded.hint.as_deref(),
            Some("Mock session closed. Handed to Grok; Mock's files were not carried.")
        );
        assert_eq!(route.from_harness, Some(HarnessId::Mock));
        assert_eq!(route.from_model.as_deref(), Some("m1"));
        assert_eq!(route.to_model.as_deref(), Some("g"));

        // Back where it started, harness and model: no divider at all.
        assert!(
            fold_switch_note(&previous, HarnessId::Codex, &config("mock", "m1"), &next).is_none()
        );
        // The origin harness on another model is still a handoff, worded as the model change.
        let (back, _) =
            fold_switch_note(&previous, HarnessId::Codex, &config("mock", "m2"), &next).unwrap();
        assert_eq!(back.text, "Model changed from m1 to m2.");
        assert_eq!(back.hint, None);

        // A model change on the harness the handoff reached keeps its divider.
        let (same, route) = fold_switch_note(
            &previous,
            HarnessId::Codex,
            &config("codex", "gpt-2"),
            &SystemNote {
                text: "Model changed from gpt to gpt-2.".into(),
                hint: None,
            },
        )
        .unwrap();
        assert_eq!(same.text, previous.text);
        assert_eq!(same.hint, previous.hint);
        assert_eq!(route.to_model.as_deref(), Some("gpt-2"));

        // A seeded thread's divider survives a return to its source's agent.
        let seeded = SwitchNote {
            route: zeron_doc::HandoffRoute {
                source_chat_id: Some("source".into()),
                ..previous.route.clone()
            },
            ..previous
        };
        assert!(
            fold_switch_note(&seeded, HarnessId::Codex, &config("mock", "m1"), &next).is_some()
        );
    }
}
