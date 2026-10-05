use gpui::{
    AnyElement, Context, Entity, Focusable, MouseButton, SharedString, Subscription, Window, div,
    prelude::*, px,
};

use super::permission_guide::{choice_button, row};
use super::widgets;
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::pickers::{CommitModelPicked, Pickers};
use crate::popover;
use crate::settings::{
    self, DEFAULT_COMMIT_INSTRUCTION, GitAgentBusyPolicy, PrChatHistory, PrChatStyle, SavePolicy,
    UiSettings,
};
use crate::state::AppState;
use crate::theme::Theme;

pub const COMMIT_MESSAGE_PRESETS: [(&str, &str, &str); 3] = [
    (
        "source-control-preset-default",
        "Default",
        DEFAULT_COMMIT_INSTRUCTION,
    ),
    (
        "source-control-preset-conventional",
        "Conventional",
        "Write a Conventional Commits message (type(scope): subject) for this staged diff, with a short bullet body.",
    ),
    (
        "source-control-preset-repo-style",
        "Match repo style",
        "Write a commit message for this staged diff in the same style as the recent subjects: \"Area: what changed\", then a short paragraph on why.",
    ),
];

const BUSY_POLICIES: [(GitAgentBusyPolicy, &str, &str, &str); 3] = [
    (
        GitAgentBusyPolicy::SafeOnly,
        "safe-only",
        "Safe actions only",
        "Stage, unstage, commit staged changes and push, with a warning. Everything else waits for the agent.",
    ),
    (
        GitAgentBusyPolicy::BlockAll,
        "block-all",
        "Read-only",
        "The Git tab makes no changes until the agent stops.",
    ),
    (
        GitAgentBusyPolicy::AllowAll,
        "allow-all",
        "Allow everything",
        "Every action runs, with a warning banner while the agent works.",
    ),
];

pub struct SourceControlPage {
    scroll: widgets::PageScroll,
    instruction: Entity<ComposerInput>,
    picker: Entity<Pickers>,
    _instruction_edits: Subscription,
    _model_picks: Subscription,
}

impl SourceControlPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let current = settings::current(cx);
        let instruction = cx.new(|cx| {
            let mut input = ComposerInput::new(DEFAULT_COMMIT_INSTRUCTION, cx);
            input.set_text(current.commit_message_instruction.clone(), cx);
            input
        });
        let instruction_edits =
            cx.subscribe(&instruction, |_, input, event: &ComposerInputEvent, cx| {
                if matches!(event, ComposerInputEvent::Edited) {
                    let text = input.read(cx).text().to_string();
                    settings::update(SavePolicy::Debounced, cx, |settings| {
                        settings.commit_message_instruction = text
                    });
                    cx.notify();
                }
            });
        let picker = cx.new(|cx| {
            Pickers::new_for_commit_messages(state, current.commit_message_model.as_ref(), cx)
        });
        let model_picks = cx.subscribe(&picker, |_, _, event: &CommitModelPicked, cx| {
            let choice = event.0.clone();
            settings::update(SavePolicy::Immediate, cx, |settings| {
                settings.commit_message_model = Some(choice)
            });
            cx.notify();
        });
        Self {
            scroll: widgets::PageScroll::default(),
            instruction,
            picker,
            _instruction_edits: instruction_edits,
            _model_picks: model_picks,
        }
    }

    fn apply_preset(&mut self, text: &'static str, cx: &mut Context<Self>) {
        self.instruction
            .update(cx, |input, cx| input.set_text(text, cx));
        self.write(cx, |settings| {
            settings.commit_message_instruction = text.to_string()
        });
    }

    fn reset_model(&mut self, cx: &mut Context<Self>) {
        self.picker
            .update(cx, |picker, cx| picker.set_commit_model(None, cx));
        self.write(cx, |settings| settings.commit_message_model = None);
    }

    fn write(&mut self, cx: &mut Context<Self>, mutate: impl FnOnce(&mut UiSettings)) {
        settings::update(SavePolicy::Immediate, cx, mutate);
        cx.notify();
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    fn render_policy_options(
        &self,
        theme: &Theme,
        selected: GitAgentBusyPolicy,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = theme.accent;
        div()
            .px(px(12.0))
            .pb(px(12.0))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(
                BUSY_POLICIES
                    .into_iter()
                    .map(|(policy, slug, title, copy)| {
                        let on = policy == selected;
                        let id = format!("source-control-busy-{slug}");
                        div()
                            .id(SharedString::from(id.clone()))
                            .debug_selector(move || id.clone())
                            .role(gpui::Role::Button)
                            .aria_label(title)
                            .aria_toggled(if on {
                                gpui::Toggled::True
                            } else {
                                gpui::Toggled::False
                            })
                            .tab_index(0)
                            .focus_visible(move |style| style.border_2().border_color(accent))
                            .px(px(8.0))
                            .py(px(8.0))
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .when(on, |el| el.bg(crate::theme::ink(0.05)))
                            .hover(|style| style.bg(crate::theme::ink(0.03)))
                            .flex()
                            .flex_row()
                            .items_start()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .mt(px(2.0))
                                    .size(px(14.0))
                                    .flex_none()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(if on { accent } else { theme.border })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(on, |el| {
                                        el.child(div().size(px(6.0)).rounded_full().bg(accent))
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .child(widgets::row_title(theme, title))
                                    .child(
                                        div()
                                            .text_size(crate::typography::ui_rems(12.0))
                                            .line_height(px(18.0))
                                            .text_color(theme.text_muted)
                                            .child(copy),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.write(cx, |settings| settings.git_agent_busy_policy = policy)
                            }))
                    }),
            )
            .into_any_element()
    }
}

impl popover::ScrollRailHost for SourceControlPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for SourceControlPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let accent = theme.accent;
        let current = settings::current(cx);
        let chip = |id: &'static str, label: &'static str, selected: bool| {
            choice_button(&theme, id, label, selected).debug_selector(move || id.to_string())
        };

        let instruction_text = current.commit_message_instruction.trim();
        let presets =
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.0))
                .children(COMMIT_MESSAGE_PRESETS.into_iter().map(|(id, label, text)| {
                    chip(id, label, instruction_text == text)
                        .on_click(cx.listener(move |this, _, _, cx| this.apply_preset(text, cx)))
                }));
        let textarea = div()
            .id("source-control-instruction")
            .w_full()
            .min_h(px(64.0))
            .px(px(10.0))
            .py(px(8.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.border)
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.instruction.focus_handle(cx), cx);
                }),
            )
            .child(self.instruction.clone());
        let instruction_editor = div()
            .px(px(20.0))
            .pb(px(14.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(textarea)
            .child(presets)
            .into_any_element();

        let model_control = div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .when(current.commit_message_model.is_some(), |el| {
                el.child(
                    widgets::text_action(&theme, widgets::ActionTone::Quiet, "Reset")
                        .id("source-control-model-reset")
                        .debug_selector(|| "source-control-model-reset".into())
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .aria_label("Pick the commit model automatically")
                        .focus_visible(move |style| style.border_2().border_color(accent))
                        .on_click(cx.listener(|this, _, _, cx| this.reset_model(cx))),
                )
            })
            .child(self.picker.clone())
            .into_any_element();

        let commit_card = widgets::section_card(&theme)
            .mt(px(0.0))
            .child(row(
                &theme,
                true,
                "Instruction",
                "Sent to the model with the staged diff when you click Write with AI. Leave it empty to use the default.",
                None,
                None,
                div().into_any_element(),
                Some(instruction_editor),
            ))
            .child(row(
                &theme,
                false,
                "Default model",
                "Automatic uses Claude Code with Haiku 4.5 when available, otherwise the cheapest model. The Git tab can still pick another model for one commit.",
                None,
                None,
                model_control,
                None,
            ));

        let floating = current.pr_chat_style == PrChatStyle::Floating;
        let style_control = div()
            .flex()
            .gap(px(6.0))
            .child(
                chip("source-control-chat-floating", "Floating", floating).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.write(cx, |settings| {
                            settings.pr_chat_style = PrChatStyle::Floating
                        })
                    },
                )),
            )
            .child(
                chip("source-control-chat-bottom-bar", "Bottom bar", !floating).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.write(cx, |settings| {
                            settings.pr_chat_style = PrChatStyle::BottomBar
                        })
                    }),
                ),
            )
            .into_any_element();
        let keep = current.pr_chat_history == PrChatHistory::KeepPerPr;
        let history_control = div()
            .flex()
            .gap(px(6.0))
            .child(
                chip("source-control-history-keep", "Keep per PR", keep).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.write(cx, |settings| {
                            settings.pr_chat_history = PrChatHistory::KeepPerPr
                        })
                    },
                )),
            )
            .child(
                chip("source-control-history-fresh", "Fresh every time", !keep).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.write(cx, |settings| {
                            settings.pr_chat_history = PrChatHistory::Fresh
                        })
                    }),
                ),
            )
            .into_any_element();
        let chat_card = widgets::section_card(&theme)
            .mt(px(0.0))
            .child(row(
                &theme,
                true,
                "Style",
                "How Chat (⌘J) opens on the Pull Requests page.",
                None,
                None,
                style_control,
                None,
            ))
            .child(row(
                &theme,
                false,
                "Chat history",
                "Keep per PR: reopening a PR shows its earlier chats. Fresh every time: closing a chat discards it unless you turn it into a thread.",
                None,
                None,
                history_control,
                None,
            ));

        let git_card = widgets::section_card(&theme).mt(px(0.0)).child(row(
            &theme,
            true,
            "While an agent is running",
            "What the Git tab may do in a checkout where an agent is working.",
            None,
            None,
            div().into_any_element(),
            Some(self.render_policy_options(&theme, current.git_agent_busy_policy, cx)),
        ));

        let scrollbar = popover::rail(self, "source-control-settings-scrollbar", &theme, cx);
        div()
            .id("source-control-settings-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("source-control-settings-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(widgets::page_header(&theme, "Source Control", None))
                                .child(widgets::section(&theme, "Commit message", commit_card))
                                .child(widgets::section(
                                    &theme,
                                    "Pull Requests · Mini chat",
                                    chat_card,
                                ))
                                .child(widgets::section(&theme, "Git", git_card)),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::CommitModelChoice;
    use zeron_proto::HarnessId;

    #[gpui::test]
    fn preset_clicks_and_model_picks_persist(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::settings::init(UiSettings::default(), dir.path(), cx);
        });
        let state = cx.new(|_| AppState::new());
        let (page, cx) = cx.add_window_view(|_, cx| SourceControlPage::new(state, cx));
        cx.update(|window, cx| window.draw(cx).clear());
        let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            let bounds = cx.debug_bounds(selector).expect(selector);
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear());
        };

        click(cx, "source-control-preset-conventional");
        let conventional = COMMIT_MESSAGE_PRESETS[1].2;
        cx.update(|_, cx| {
            assert_eq!(
                settings::current(cx).commit_message_instruction,
                conventional
            );
            assert_eq!(page.read(cx).instruction.read(cx).text(), conventional);
        });

        let choice = CommitModelChoice {
            harness: HarnessId::Codex,
            model: "gpt-5.6-luna".into(),
        };
        cx.update(|_, cx| {
            let picker = page.read(cx).picker.clone();
            picker.update(cx, |_, cx| cx.emit(CommitModelPicked(choice.clone())));
        });
        cx.run_until_parked();
        click(cx, "source-control-chat-bottom-bar");
        click(cx, "source-control-busy-block-all");
        cx.update(|_, cx| crate::settings::flush(cx));
        let saved = UiSettings::load(dir.path());
        assert_eq!(saved.commit_message_instruction, conventional);
        assert_eq!(saved.commit_message_model, Some(choice));
        assert_eq!(saved.pr_chat_style, PrChatStyle::BottomBar);
        assert_eq!(saved.git_agent_busy_policy, GitAgentBusyPolicy::BlockAll);

        click(cx, "source-control-model-reset");
        click(cx, "source-control-preset-default");
        cx.update(|_, cx| crate::settings::flush(cx));
        let saved = UiSettings::load(dir.path());
        assert_eq!(saved.commit_message_model, None);
        assert_eq!(saved.commit_message_instruction, DEFAULT_COMMIT_INSTRUCTION);
    }
}
