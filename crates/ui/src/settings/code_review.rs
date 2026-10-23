use std::collections::HashMap;

use gpui::{AnyElement, Context, Entity, Subscription, Task, Window, div, prelude::*, px};
use zeron_proto::{HarnessId, Model};

use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::popover::{self, Loadable};
use crate::review::{ReviewSettings, ReviewerChoice, ReviewerOptions};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

pub struct CodeReviewPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    options: Loadable<ReviewerOptions>,
    settings: ReviewSettings,
    save_task: Option<Task<()>>,
    models: HashMap<HarnessId, Vec<Model>>,
    reviewer_select: widgets::SelectState,
    model_select: widgets::SelectState,
    thinking: crate::pickers::LevelSlider,
    instructions: Entity<ComposerInput>,
    task: Option<Task<()>>,
    model_task: Option<Task<()>>,
    _state: Subscription,
    _instructions: Subscription,
}

impl CodeReviewPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let state_sub = cx.observe(&state, |this: &mut Self, _, cx| {
            if matches!(this.options, Loadable::Idle) {
                this.load(cx);
            }
        });
        let instructions = cx.new(|cx| {
            ComposerInput::new("Focus on correctness and security. Ignore formatting.", cx)
        });
        let instructions_events = cx.subscribe(
            &instructions,
            |_: &mut Self, _, _: &ComposerInputEvent, cx| cx.notify(),
        );
        let mut card = Self {
            state,
            instructions,
            _instructions: instructions_events,
            scroll: widgets::PageScroll::default(),
            options: Loadable::Idle,
            settings: ReviewSettings::default(),
            save_task: None,
            models: HashMap::new(),
            reviewer_select: widgets::SelectState::default(),
            model_select: widgets::SelectState::default(),
            thinking: Default::default(),
            task: None,
            model_task: None,
            _state: state_sub,
        };
        card.load(cx);
        card
    }

    pub(crate) fn load(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.options = Loadable::Loading;
        self.task = Some(cx.spawn(async move |this, cx| {
            let options = crate::review::reviewer_options(&engine).await;
            let settings = crate::review::review_settings(&engine).await;
            this.update(cx, |card, cx| {
                card.instructions.update(cx, |input, cx| {
                    input.set_text(settings.instructions.clone(), cx)
                });
                card.settings = settings;
                card.options = Loadable::Ready(options);
                card.ensure_models(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn ensure_models(&mut self, cx: &mut Context<Self>) {
        let ReviewerChoice::Harness { harness, .. } = self.settings.reviewer.clone() else {
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
        self.settings.reviewer = choice;
        self.store(cx);
        self.ensure_models(cx);
        cx.notify();
    }

    fn store(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let settings = self.settings.clone();
        self.save_task = Some(cx.spawn(async move |this, cx| {
            let saved = engine
                .client()
                .call_as::<ReviewSettings>(
                    zeron_rpc::methods::SET_REVIEW_SETTINGS,
                    serde_json::to_value(&settings).unwrap_or_default(),
                )
                .await;
            this.update(cx, |page, cx| {
                if let Ok(saved) = saved {
                    page.settings = saved;
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
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

impl CodeReviewPage {
    fn render_card(
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let theme = theme.clone();
        let card = widgets::section_card(&theme).mt(px(0.0));
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
        let current = self.settings.reviewer.clone();
        let mut choices = options.choices();
        let same_slot = |choice: &ReviewerChoice| match (choice, &current) {
            (
                ReviewerChoice::Harness { harness: a, .. },
                ReviewerChoice::Harness { harness: b, .. },
            ) => a == b,
            _ => *choice == current,
        };
        if !choices.iter().any(same_slot) {
            choices.insert(1, current.clone());
        }
        let selected = choices.iter().position(same_slot).unwrap_or(0);
        let auto = crate::review::resolve_reviewer(&ReviewerChoice::Auto, None, &options).name();
        let reviewer_select: AnyElement = widgets::select(
            "code-review-reviewer",
            "Reviewer",
            &theme,
            |card: &mut Self| &mut card.reviewer_select,
        )
        .options(
            choices.iter().map(|choice| {
                let (mark, tint) = crate::review::brand(&options, choice, None);
                let color = tint.unwrap_or(theme.text_muted);
                let option = widgets::SelectOption::new(options.label(choice)).leading(move || {
                    crate::icons::icon(mark)
                        .size(px(13.0))
                        .text_color(color)
                        .into_any_element()
                });
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
        let mut thinking_row = None;
        let model_row = match &current {
            ReviewerChoice::Harness {
                harness,
                model,
                reasoning,
            } => {
                let harness = *harness;
                let reasoning = *reasoning;
                let model_entry = model
                    .as_deref()
                    .and_then(|id| self.models.get(&harness)?.iter().find(|m| m.id == id));
                let levels = options.reasoning_stops(harness, model_entry);
                if !levels.is_empty() {
                    let labels: Vec<gpui::SharedString> =
                        std::iter::once(gpui::SharedString::from("Default"))
                            .chain(levels.iter().map(|level| {
                                gpui::SharedString::from(crate::pickers::reasoning_label(*level))
                            }))
                            .collect();
                    let selected = reasoning
                        .and_then(|level| levels.iter().position(|l| *l == level))
                        .map_or(0, |ix| ix + 1);
                    let value = labels[selected].clone();
                    let chosen_model = model.clone();
                    let slider = self.thinking.render(
                        "code-review-thinking",
                        "Reviewer thinking",
                        &labels,
                        selected,
                        |page: &mut Self| &mut page.thinking,
                        move |page, ix, cx| {
                            page.save(
                                ReviewerChoice::Harness {
                                    harness,
                                    model: chosen_model.clone(),
                                    reasoning: ix
                                        .checked_sub(1)
                                        .and_then(|ix| levels.get(ix).copied()),
                                },
                                cx,
                            );
                        },
                        &theme,
                        window,
                        cx,
                    );
                    thinking_row = Some(
                        Self::row(
                            &theme,
                            false,
                            "Thinking",
                            "How hard the reviewer thinks. Default leaves it to the agent."
                                .to_string(),
                        )
                        .child(
                            div()
                                .w(px(196.0))
                                .flex()
                                .flex_col()
                                .gap(px(4.0))
                                .child(slider)
                                .child(
                                    div()
                                        .text_size(crate::typography::ui_rems(11.0))
                                        .text_color(theme.text_muted)
                                        .child(value),
                                ),
                        ),
                    );
                }
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
                    card.save(
                        ReviewerChoice::Harness {
                            harness,
                            model,
                            reasoning,
                        },
                        cx,
                    );
                })
                .render(&self.model_select, cx);
                Some(
                    Self::row(
                        &theme,
                        false,
                        "Model",
                        "Shown when the reviewer is an agent.".to_string(),
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
                "Reviewer",
                "Auto picks a profile named reviewer, else the other agent family from the \
                 chat's."
                    .to_string(),
            )
            .child(reviewer_select),
        )
        .children(model_row)
        .children(thinking_row)
    }
}

impl CodeReviewPage {
    fn render_instructions(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let saved = self.settings.instructions.clone();
        let current = self.instructions.read(cx).text().trim().to_string();
        let dirty = saved.trim() != current;
        div()
            .flex()
            .flex_col()
            .child(widgets::section_label(theme, "Review instructions").mt(px(28.0)))
            .child(
                widgets::section_card(theme).mt(px(8.0)).child(
                    widgets::card_row(theme, true)
                        .flex_col()
                        .items_stretch()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child(
                                    "Added to every review prompt, for every reviewer, before the \
                                     findings format it must follow.",
                                ),
                        )
                        .child(popover::dialog_field(
                            crate::composer::growing_field(
                                self.instructions.clone(),
                                72.0,
                                320.0,
                                cx,
                            )
                            .into_any_element(),
                        ))
                        .child(
                            div().flex().justify_end().child(
                                widgets::text_action(theme, widgets::ActionTone::Filled, "Save")
                                    .id("code-review-instructions-save")
                                    .when(!dirty, |el| el.opacity(0.4).cursor_default())
                                    .when(dirty, |el| {
                                        el.on_click(cx.listener(|this, _, _, cx| {
                                            let text = this
                                                .instructions
                                                .read(cx)
                                                .text()
                                                .trim()
                                                .to_string();
                                            this.settings.instructions = text;
                                            this.store(cx);
                                            cx.notify();
                                        }))
                                    }),
                            ),
                        ),
                ),
            )
            .into_any_element()
    }
}

impl popover::ScrollRailHost for CodeReviewPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for CodeReviewPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let card = self.render_card(&theme, window, cx);
        let scrollbar = popover::rail(self, "code-review-scrollbar", &theme, cx);
        div()
            .id("code-review-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("code-review-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(widgets::page_header(&theme, "Code review", None))
                                .child(widgets::page_subtitle(
                                    &theme,
                                    "Who reviews changes when you press Review in the composer.",
                                ))
                                .child(card.mt(px(16.0)))
                                .child(
                                    div()
                                        .mt(px(10.0))
                                        .px(px(8.0))
                                        .text_size(crate::typography::ui_rems(12.0))
                                        .text_color(theme.text_muted)
                                        .child(
                                            "A pick in the Review button's menu applies to that \
                                             chat only and overrides this default.",
                                        ),
                                )
                                .child(self.render_instructions(&theme, cx)),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}
