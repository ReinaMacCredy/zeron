use std::collections::HashMap;

use gpui::{AnyElement, Context, Entity, Subscription, Task, div, prelude::*, px};
use zeron_proto::{HarnessId, Model};

use crate::popover::Loadable;
use crate::review::{ReviewerChoice, ReviewerOptions};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

pub struct CodeReviewCard {
    state: Entity<AppState>,
    options: Loadable<ReviewerOptions>,
    models: HashMap<HarnessId, Vec<Model>>,
    reviewer_select: widgets::SelectState,
    model_select: widgets::SelectState,
    task: Option<Task<()>>,
    model_task: Option<Task<()>>,
    _state: Subscription,
}

impl CodeReviewCard {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let state_sub = cx.observe(&state, |this: &mut Self, _, cx| {
            if matches!(this.options, Loadable::Idle) {
                this.load(cx);
            }
        });
        let mut card = Self {
            state,
            options: Loadable::Idle,
            models: HashMap::new(),
            reviewer_select: widgets::SelectState::default(),
            model_select: widgets::SelectState::default(),
            task: None,
            model_task: None,
            _state: state_sub,
        };
        card.load(cx);
        card
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.options = Loadable::Loading;
        self.task = Some(cx.spawn(async move |this, cx| {
            let options = crate::review::reviewer_options(&engine).await;
            this.update(cx, |card, cx| {
                card.options = Loadable::Ready(options);
                card.ensure_models(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn ensure_models(&mut self, cx: &mut Context<Self>) {
        let ReviewerChoice::Harness { harness, .. } = crate::settings::review_reviewer(cx) else {
            return;
        };
        if self.models.contains_key(&harness) {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.model_task = Some(cx.spawn(async move |this, cx| {
            let models = crate::review::list_models(&engine, harness).await;
            this.update(cx, |card, cx| {
                card.models.insert(harness, models);
                cx.notify();
            })
            .ok();
        }));
    }

    fn save(&mut self, choice: ReviewerChoice, cx: &mut Context<Self>) {
        crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
            settings.review_reviewer = choice
        });
        self.ensure_models(cx);
        cx.notify();
    }

    fn row(theme: &Theme, first: bool, title: &str, description: String) -> gpui::Div {
        widgets::card_row(theme, first).child(
            div()
                .flex_1()
                .min_w(px(160.0))
                .child(widgets::row_title(theme, title.to_string()))
                .child(widgets::meta_line(
                    theme,
                    vec![div().child(description).into_any_element()],
                )),
        )
    }
}

impl Render for CodeReviewCard {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let card = widgets::section_card(&theme).mt(px(8.0));
        let Some(options) = self.options.ready().cloned() else {
            return card.child(
                widgets::card_row(&theme, true).child(div().text_color(theme.text_muted).child(
                    match &self.options {
                        Loadable::Error(error) => error.clone(),
                        _ => "Loading…".to_string(),
                    },
                )),
            );
        };
        let current = crate::settings::review_reviewer(cx);
        let choices = options.choices();
        let selected = choices
            .iter()
            .position(|choice| match (choice, &current) {
                (
                    ReviewerChoice::Harness { harness: a, .. },
                    ReviewerChoice::Harness { harness: b, .. },
                ) => a == b,
                _ => *choice == current,
            })
            .unwrap_or(0);
        let auto = crate::review::resolve_reviewer(&ReviewerChoice::Auto, None, &options).name();
        let reviewer_select: AnyElement = widgets::select(
            "code-review-reviewer",
            "Reviewer",
            &theme,
            |card: &mut Self| &mut card.reviewer_select,
        )
        .options(
            choices.iter().map(|choice| {
                let option = widgets::SelectOption::new(options.label(choice));
                match choice {
                    ReviewerChoice::Auto => option.detail(auto.clone()),
                    ReviewerChoice::Profile { .. } => option.detail("Profile"),
                    ReviewerChoice::Harness { .. } => option,
                }
            }),
            selected,
        )
        .width(196.0)
        .on_select(move |card, ix, _, cx| {
            if let Some(choice) = choices.get(ix) {
                card.save(choice.clone(), cx);
            }
        })
        .render(&self.reviewer_select, cx)
        .into_any_element();
        let model_row = match &current {
            ReviewerChoice::Harness { harness, model } => {
                let harness = *harness;
                let models = self.models.get(&harness).cloned().unwrap_or_default();
                let model_ix = model
                    .as_deref()
                    .and_then(|id| models.iter().position(|m| m.id == id))
                    .map_or(0, |ix| ix + 1);
                let select = widgets::select(
                    "code-review-model",
                    "Reviewer model",
                    &theme,
                    |card: &mut Self| &mut card.model_select,
                )
                .options(
                    std::iter::once(widgets::SelectOption::new("Default")).chain(
                        models
                            .iter()
                            .map(|m| widgets::SelectOption::new(m.label.clone())),
                    ),
                    model_ix,
                )
                .width(196.0)
                .on_select(move |card, ix, _, cx| {
                    let model = ix
                        .checked_sub(1)
                        .and_then(|ix| models.get(ix))
                        .map(|m| m.id.clone());
                    card.save(ReviewerChoice::Harness { harness, model }, cx);
                })
                .render(&self.model_select, cx);
                Some(
                    Self::row(
                        &theme,
                        false,
                        "Model",
                        "The model the reviewer runs on.".to_string(),
                    )
                    .child(select),
                )
            }
            _ => None,
        };
        card.child(
            Self::row(
                &theme,
                true,
                "Code reviewer",
                "Who reviews changes from the composer's Review button. Auto prefers a profile \
                 named reviewer, then the other agent family."
                    .to_string(),
            )
            .child(reviewer_select),
        )
        .children(model_row)
    }
}
