use zeron_proto::review::{self, ReviewSettings, ReviewerChoice, ReviewerOptions};
use zeron_proto::{HarnessId, ReasoningLevel};

use super::routing::{harness_of, wire};
use super::{Page, Row, RowKind, Section};
use crate::catalog::ModelInfo;
use crate::client::Client;
use crate::error::{ClientError, Result};

struct Loaded {
    settings: ReviewSettings,
    options: ReviewerOptions,
    models: Vec<ModelInfo>,
}

async fn load(client: &Client, device_id: &str, harness: Option<HarnessId>) -> Result<Loaded> {
    let (settings, options) = futures::join!(
        client.review_settings(device_id),
        client.reviewer_options(device_id)
    );
    let settings = settings?;
    let harness = harness.or(match &settings.reviewer {
        ReviewerChoice::Harness { harness, .. } => Some(*harness),
        _ => None,
    });
    let models = match harness {
        Some(harness) => client.list_models(device_id, &wire(harness)).await,
        None => Vec::new(),
    };
    Ok(Loaded {
        settings,
        options,
        models,
    })
}

fn model_label(models: &[ModelInfo], id: &str) -> String {
    models
        .iter()
        .find(|model| model.id == id)
        .map_or_else(|| id.to_owned(), |model| model.label.clone())
}

fn stops(loaded: &Loaded, harness: HarnessId, model: Option<&str>) -> Vec<ReasoningLevel> {
    let levels = model
        .and_then(|id| loaded.models.iter().find(|m| m.id == id))
        .map(crate::session::model_levels);
    loaded.options.reasoning_stops(harness, levels.as_deref())
}

fn choice_id(choice: &ReviewerChoice) -> String {
    match choice {
        ReviewerChoice::Auto => "reviewer:auto".into(),
        ReviewerChoice::Harness { harness, .. } => format!("reviewer:harness:{}", wire(*harness)),
        ReviewerChoice::Profile { id } => format!("reviewer:profile:{id}"),
    }
}

pub(super) fn summary(settings: &ReviewSettings, options: &ReviewerOptions) -> String {
    review::choice_name(Some(options), &settings.reviewer, None)
}

pub(super) async fn page(client: &Client, device_id: &str, page: &str) -> Result<Page> {
    let loaded = load(client, device_id, None).await?;
    match page {
        "review" => Ok(main_page(&loaded)),
        "review:reviewer" => Ok(reviewer_page(&loaded)),
        "review:model" => Ok(model_page(&loaded)),
        "review:thinking" => Ok(thinking_page(&loaded)),
        other => Err(ClientError::NotFound(other.to_owned())),
    }
}

pub(super) async fn act(
    client: &Client,
    device_id: &str,
    action: &str,
    value: Option<String>,
) -> Result<()> {
    let picked = action
        .strip_prefix("reviewer:harness:")
        .and_then(harness_of);
    let loaded = load(client, device_id, picked).await?;
    let mut settings = loaded.settings.clone();
    if !edit(&mut settings, &loaded, action, value) {
        return Err(ClientError::InvalidArgument(action.to_owned()));
    }
    client.set_review_settings(device_id, &settings).await?;
    Ok(())
}

fn edit(
    settings: &mut ReviewSettings,
    loaded: &Loaded,
    action: &str,
    value: Option<String>,
) -> bool {
    if action == "instructions" {
        settings.instructions = value.unwrap_or_default().trim().to_owned();
        return true;
    }
    if action.starts_with("reviewer:") {
        let Some(choice) = loaded
            .options
            .choices()
            .into_iter()
            .find(|choice| choice_id(choice) == action)
        else {
            return false;
        };
        settings.reviewer = choice;
        return true;
    }
    let ReviewerChoice::Harness {
        harness,
        model,
        reasoning,
    } = &mut settings.reviewer
    else {
        return false;
    };
    if let Some(id) = action.strip_prefix("model:") {
        if !id.is_empty() && !loaded.models.iter().any(|m| m.id == id) {
            return false;
        }
        *model = (!id.is_empty()).then(|| id.to_owned());
        return true;
    }
    if let Some(level) = action.strip_prefix("reasoning:") {
        let stops = stops(loaded, *harness, model.as_deref());
        *reasoning = match level {
            "" => None,
            level => match stops.into_iter().find(|stop| wire(*stop) == level) {
                Some(stop) => Some(stop),
                None => return false,
            },
        };
        return true;
    }
    false
}

fn main_page(loaded: &Loaded) -> Page {
    let settings = &loaded.settings;
    let mut rows = vec![
        Row::link("review:reviewer", "Reviewer", None).value(summary(settings, &loaded.options)),
    ];
    let mut footer = vec![review::REVIEWER_HINT];
    if let ReviewerChoice::Harness {
        harness,
        model,
        reasoning,
    } = &settings.reviewer
    {
        rows.push(
            Row::link("review:model", "Model", None).value(model.as_deref().map_or_else(
                || "Default".to_owned(),
                |id| model_label(&loaded.models, id),
            )),
        );
        footer.push(review::MODEL_HINT);
        if !stops(loaded, *harness, model.as_deref()).is_empty() {
            rows.push(Row::link("review:thinking", "Thinking", None).value(
                reasoning.map_or_else(|| "Default".to_owned(), |level| level.label().to_owned()),
            ));
            footer.push(review::THINKING_HINT);
        }
    }
    footer.push(review::OVERRIDE_NOTE);
    Page {
        title: "Code review".into(),
        sections: vec![
            Section {
                title: None,
                footer: Some(footer.join(" ")),
                rows,
            },
            Section {
                title: Some("Review instructions".into()),
                footer: Some(review::INSTRUCTIONS_HINT.into()),
                rows: vec![
                    Row::new(
                        "instructions",
                        "Review instructions",
                        RowKind::Text {
                            multiline: true,
                            placeholder: Some(review::INSTRUCTIONS_PLACEHOLDER.into()),
                        },
                    )
                    .value(Some(settings.instructions.clone())),
                ],
            },
        ],
    }
}

fn reviewer_page(loaded: &Loaded) -> Page {
    let rows = review::reviewer_rows(
        Some(&loaded.options),
        &loaded.settings.reviewer,
        None,
        |_, id| model_label(&loaded.models, id),
    )
    .into_iter()
    .map(|row| {
        let profile = matches!(row.choice, ReviewerChoice::Profile { .. });
        Row::new(
            choice_id(&row.choice),
            row.label,
            RowKind::Check { on: row.selected },
        )
        .subtitle(row.detail.or_else(|| profile.then(|| "Profile".to_owned())))
    })
    .collect();
    Page {
        title: "Reviewer".into(),
        sections: vec![Section {
            title: None,
            footer: Some(review::REVIEWER_HINT.into()),
            rows,
        }],
    }
}

fn model_page(loaded: &Loaded) -> Page {
    let current = match &loaded.settings.reviewer {
        ReviewerChoice::Harness { model, .. } => model.clone(),
        _ => None,
    };
    let rows = std::iter::once(Row::new(
        "model:",
        "Default",
        RowKind::Check {
            on: current.is_none(),
        },
    ))
    .chain(loaded.models.iter().map(|model| {
        Row::new(
            format!("model:{}", model.id),
            model.label.clone(),
            RowKind::Check {
                on: current.as_deref() == Some(model.id.as_str()),
            },
        )
    }))
    .collect();
    Page {
        title: "Model".into(),
        sections: vec![Section {
            title: None,
            footer: Some(review::MODEL_HINT.into()),
            rows,
        }],
    }
}

fn thinking_page(loaded: &Loaded) -> Page {
    let (stops, current) = match &loaded.settings.reviewer {
        ReviewerChoice::Harness {
            harness,
            model,
            reasoning,
        } => (stops(loaded, *harness, model.as_deref()), *reasoning),
        _ => (Vec::new(), None),
    };
    let rows = std::iter::once(Row::new(
        "reasoning:",
        "Default",
        RowKind::Check {
            on: current.is_none(),
        },
    ))
    .chain(stops.into_iter().map(|stop| {
        Row::new(
            format!("reasoning:{}", wire(stop)),
            stop.label(),
            RowKind::Check {
                on: current == Some(stop),
            },
        )
    }))
    .collect();
    Page {
        title: "Thinking".into(),
        sections: vec![Section {
            title: None,
            footer: Some(review::THINKING_HINT.into()),
            rows,
        }],
    }
}
