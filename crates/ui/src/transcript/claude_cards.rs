use std::collections::HashSet;
use std::sync::Arc;

use gpui::{AnyElement, ClipboardItem, Context, Hsla, SharedString, div, prelude::*, px};
use zeron_proto::claude_cards::{
    LOCAL_FOOTNOTE, MCP_EMPTY, MCP_FOOTNOTE, MODEL_FOOTNOTE, RUNNING, Series, TerminalAction,
    mcp_servers, parse_context, percent, selected_model, stale_card, terminal_action,
    terminal_only, usage_subtitle,
};
use zeron_proto::claude_config::CommandCard;
use zeron_proto::usage::format_tokens;

use super::{Transcript, TranscriptEvent};
use crate::composer::claude_config::{meter, parse_usage};
use crate::icons::{self, icon};
use crate::theme::Theme;

#[derive(Default)]
pub(super) struct CardUi {
    open: HashSet<String>,
    raw: bool,
    week: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaudeCardAction {
    Compact,
    ManageTools,
    ContextSettings,
    OpenUsage,
    Model(String),
    Status,
    Permissions,
    Help,
    Hooks,
    Retry { message_id: String, command: String },
}

fn series(theme: &Theme, name: &str) -> Hsla {
    match zeron_proto::claude_cards::series(name) {
        Series::System => theme.accent,
        Series::Mcp => theme.syntax.function,
        Series::Agents => theme.syntax.type_name,
        Series::Memory => theme.warning,
        Series::Skills => theme.success,
        Series::Messages => theme.danger,
        Series::Other => theme.text_muted,
    }
}

fn text_size(size: f32) -> gpui::Rems {
    crate::typography::ui_rems(size)
}

fn button(
    id: SharedString,
    label: &str,
    primary: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let selector = id.to_string();
    div()
        .id(id)
        .debug_selector(move || selector)
        .role(gpui::Role::Button)
        .flex_none()
        .px(px(10.0))
        .h(px(26.0))
        .flex()
        .items_center()
        .rounded(px(7.0))
        .cursor_pointer()
        .text_size(text_size(12.5))
        .map(|el| {
            if primary {
                el.bg(theme.accent.opacity(0.14)).text_color(theme.accent)
            } else {
                el.border_1()
                    .border_color(theme.border)
                    .text_color(theme.text)
            }
        })
        .hover(|s| s.bg(theme.glass_hover()))
        .child(label.to_string())
}

impl Transcript {
    fn card_ui(&mut self, row: &SharedString) -> &mut CardUi {
        self.claude_cards.entry(row.clone()).or_default()
    }

    fn toggle_card(
        &mut self,
        row: &SharedString,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut CardUi),
    ) {
        f(self.card_ui(row));
        if let Some(ix) = self.rows.iter().position(|r| &r.id == row) {
            self.list.remeasure_items(ix..ix + 1);
        }
        cx.notify();
    }

    fn card_action(&self, action: ClaudeCardAction, cx: &mut Context<Self>) {
        if let Some(chat_id) = self.chat_id.clone() {
            cx.emit(TranscriptEvent::ClaudeCard { chat_id, action });
        }
    }

    fn card_header(
        &self,
        row: &SharedString,
        glyph: &'static str,
        title: &str,
        sub: String,
        pill: Option<String>,
        raw: Option<String>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let icon_button = |id: String, glyph: &'static str, label: &'static str| {
            div()
                .id(SharedString::from(id))
                .role(gpui::Role::Button)
                .aria_label(label)
                .size(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(theme.glass_hover()))
                .child(icon(glyph).size(px(13.0)).text_color(theme.text_muted))
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(7.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                icon(glyph)
                    .size(px(15.0))
                    .flex_none()
                    .text_color(theme.text_muted),
            )
            .child(
                div()
                    .flex_none()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title.to_string()),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(text_size(12.5))
                    .text_color(theme.text_muted)
                    .child(sub),
            )
            .child(div().flex_1())
            .when_some(pill, |el, pill| {
                el.child(
                    div()
                        .flex_none()
                        .px(px(7.0))
                        .rounded_full()
                        .border_1()
                        .border_color(theme.border)
                        .text_size(text_size(11.5))
                        .text_color(theme.text_muted)
                        .child(pill),
                )
            })
            .when_some(raw, |el, raw| {
                let raw_row = row.clone();
                let copy_row = row.clone();
                let copied = self.card_copied.as_ref() == Some(row);
                el.child(
                    icon_button(
                        format!("{row}-copy"),
                        if copied { icons::CHECK } else { icons::COPY },
                        if copied { "Copied" } else { "Copy as Markdown" },
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(raw.clone()));
                        this.card_copied = Some(copy_row.clone());
                        this.card_copied_clear = Some(cx.spawn(async move |this, cx| {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(1200))
                                .await;
                            this.update(cx, |this, cx| {
                                this.card_copied = None;
                                this.card_copied_clear = None;
                                cx.notify();
                            })
                            .ok();
                        }));
                        cx.notify();
                    })),
                )
                .child(
                    icon_button(format!("{row}-raw"), icons::FILE_CODE, "Raw").on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.toggle_card(&raw_row, cx, |ui| ui.raw = !ui.raw)
                        }),
                    ),
                )
            })
            .into_any_element()
    }

    fn card_shell(&self, row: &SharedString, theme: &Theme) -> gpui::Stateful<gpui::Div> {
        let selector = format!("claude-card-{row}");
        div()
            .id(row.clone())
            .debug_selector(move || selector)
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.border)
            .bg(crate::theme::ink(0.03))
            .overflow_hidden()
            .text_size(text_size(13.0))
            .text_color(theme.text)
    }

    fn raw_block(&self, text: &str, theme: &Theme) -> AnyElement {
        div()
            .id("claude-card-raw")
            .border_t_1()
            .border_color(theme.border)
            .px(px(12.0))
            .py(px(10.0))
            .max_h(px(220.0))
            .overflow_y_scroll()
            .font_family(theme.font_mono.clone())
            .text_size(text_size(12.0))
            .children(text.lines().map(|l| div().child(l.to_string())))
            .into_any_element()
    }

    fn footer(&self, buttons: Vec<AnyElement>, theme: &Theme) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(6.0))
            .px(px(12.0))
            .py(px(9.0))
            .border_t_1()
            .border_color(theme.border)
            .children(buttons)
            .into_any_element()
    }

    pub(super) fn render_claude_card(
        &mut self,
        row: &SharedString,
        card: Arc<CommandCard>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(name) = terminal_only(&card) {
            return self.render_notice(row, name, theme, cx);
        }
        if card.pending {
            return match stale_card(&card, now_ms()) {
                Some(stale) => self.render_generic(row, &stale, theme, cx),
                None => {
                    let started = card.started_at.unwrap_or_else(now_ms);
                    self.render_pending(row, &card.command, started, theme, cx)
                }
            };
        }
        let body = match (card.error.as_deref(), card.command.as_str()) {
            (Some(_), _) => None,
            (None, "/context") => self.render_context(row, &card, theme, cx),
            (None, "/usage") => self.render_usage(row, &card, theme, cx),
            (None, "/model") => Some(self.render_model(row, &card, theme, cx)),
            (None, "/mcp") => self.render_mcp(row, &card, theme, cx),
            _ => None,
        };
        body.unwrap_or_else(|| self.render_generic(row, &card, theme, cx))
    }

    fn render_generic(
        &mut self,
        row: &SharedString,
        card: &CommandCard,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let text = card.error.clone().unwrap_or_else(|| card.text.clone());
        let header = self.card_header(
            row,
            icons::TERMINAL,
            &card.command,
            "Claude Code · command output".into(),
            None,
            Some(text.clone()),
            theme,
            cx,
        );
        self.card_shell(row, theme)
            .child(header)
            .child(
                div()
                    .id("claude-card-body")
                    .px(px(12.0))
                    .py(px(10.0))
                    .max_h(px(260.0))
                    .overflow_y_scroll()
                    .font_family(theme.font_mono.clone())
                    .text_size(text_size(12.0))
                    .text_color(if card.error.is_some() {
                        theme.warning
                    } else {
                        theme.text
                    })
                    .children(text.lines().map(|l| div().child(l.to_string()))),
            )
            .child(match card.error {
                Some(_) => {
                    let retry = ClaudeCardAction::Retry {
                        message_id: row.to_string(),
                        command: card.command.clone(),
                    };
                    self.footer(
                        vec![
                            button(format!("{row}-retry").into(), "Retry", true, theme)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.card_action(retry.clone(), cx)
                                }))
                                .into_any_element(),
                        ],
                        theme,
                    )
                }
                None => div()
                    .px(px(12.0))
                    .py(px(8.0))
                    .border_t_1()
                    .border_color(theme.border)
                    .text_size(text_size(11.5))
                    .text_color(theme.text_faint)
                    .child(LOCAL_FOOTNOTE)
                    .into_any_element(),
            })
            .into_any_element()
    }

    fn render_pending(
        &mut self,
        row: &SharedString,
        command: &str,
        started: i64,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.card_tick.is_none() {
            self.card_tick = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                this.update(cx, |this, cx| {
                    this.card_tick = None;
                    cx.notify();
                })
                .ok();
            }));
        }
        let elapsed = ((now_ms() - started).max(0) / 1000) as u64;
        let spinner = crate::loaders::mini_glyph_spinner(
            format!("claude-card-spin-{row}"),
            3.0,
            theme.glyph,
            cx.entity_id(),
            cx,
        );
        self.card_shell(row, theme)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .py(px(10.0))
                    .child(div().flex_none().child(spinner))
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme.font_mono.clone())
                            .child(command.to_string()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(text_size(12.5))
                            .text_color(theme.text_muted)
                            .child(RUNNING),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(text_size(12.0))
                            .text_color(theme.text_faint)
                            .child(format!("{elapsed}s")),
                    ),
            )
            .into_any_element()
    }

    fn export_transcript(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if self.chat_id.is_none() || state.selected_chat != self.chat_id {
            return;
        }
        let title = state
            .selected_chat_row()
            .and_then(|chat| chat.title.clone())
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "Chat".into());
        let markdown = zeron_doc::transcript_markdown(&title, &state.transcript);
        let name: String = title
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let downloads = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|home| home.join("Downloads"))
            .unwrap_or_else(std::env::temp_dir);
        let chosen = cx.prompt_for_new_path(&downloads, Some(&format!("{}.md", name.trim())));
        cx.spawn(async move |_, _| {
            if let Ok(Ok(Some(path))) = chosen.await
                && let Err(error) = std::fs::write(&path, markdown.as_bytes())
            {
                tracing::warn!(%error, path = %path.display(), "transcript export not saved");
            }
        })
        .detach();
    }

    fn render_context(
        &mut self,
        row: &SharedString,
        card: &CommandCard,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let report = parse_context(&card.text)?;
        let (raw_on, open) = {
            let ui = self.card_ui(row);
            (ui.raw, ui.open.clone())
        };
        let total = report.total.max(1);
        let width = |n: u64| gpui::relative(report.bar_fraction(n));
        let bar = div()
            .h(px(10.0))
            .rounded(px(5.0))
            .overflow_hidden()
            .flex()
            .flex_row()
            .gap(px(1.0))
            .bg(crate::theme::ink(0.06))
            .children(
                report
                    .categories
                    .iter()
                    .filter(|(_, n)| *n > 0)
                    .map(|(name, n)| div().h_full().w(width(*n)).bg(series(theme, name))),
            )
            .when(report.buffer > 0, |el| {
                el.child(
                    div()
                        .h_full()
                        .w(width(report.buffer))
                        .bg(theme.border_strong.opacity(0.7)),
                )
            });
        let legend_item = |color: Option<Hsla>, name: String, n: u64| {
            div()
                .w(gpui::relative(0.5))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(6.0))
                .py(px(3.0))
                .text_size(text_size(12.5))
                .child(
                    div()
                        .size(px(8.0))
                        .flex_none()
                        .rounded(px(2.0))
                        .map(|el| match color {
                            Some(c) => el.bg(c),
                            None => el.border_1().border_color(theme.border),
                        }),
                )
                .child(div().flex_1().min_w_0().truncate().child(name))
                .child(div().text_color(theme.text_muted).child(format_tokens(n)))
                .child(
                    div()
                        .w(px(44.0))
                        .flex()
                        .justify_end()
                        .text_color(theme.text_muted)
                        .child(percent(n, total)),
                )
        };
        let mut legend: Vec<AnyElement> = report
            .categories
            .iter()
            .map(|(name, n)| {
                legend_item(Some(series(theme, name)), name.clone(), *n).into_any_element()
            })
            .collect();
        if report.buffer > 0 {
            legend.push(
                legend_item(
                    Some(theme.border_strong.opacity(0.7)),
                    "Autocompact buffer".into(),
                    report.buffer,
                )
                .into_any_element(),
            );
        }
        legend.push(legend_item(None, "Free".into(), report.free).into_any_element());
        let tip = report.tip();
        let groups = report.groups.iter().map(|group| {
            let key = group.title.clone();
            let is_open = open.contains(&key);
            let toggle_row = row.clone();
            let header = div()
                .id(SharedString::from(format!("{row}-group-{key}")))
                .role(gpui::Role::Button)
                .aria_expanded(is_open)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(12.0))
                .py(px(8.0))
                .cursor_pointer()
                .text_size(text_size(12.5))
                .hover(|s| s.bg(theme.glass_hover()))
                .child(
                    icon(if is_open {
                        icons::LUCIDE_CHEVRON_DOWN
                    } else {
                        icons::LUCIDE_CHEVRON_RIGHT
                    })
                    .size(px(12.0))
                    .text_color(theme.text_faint),
                )
                .child(div().flex_1().child(group.title.clone()))
                .child(
                    div()
                        .text_color(theme.text_muted)
                        .child(format_tokens(group.total)),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let key = key.clone();
                    this.toggle_card(&toggle_row, cx, move |ui| {
                        if !ui.open.remove(&key) {
                            ui.open.insert(key);
                        }
                    })
                }));
            let rows = is_open.then(|| {
                div()
                    .pb(px(6.0))
                    .children(group.rows.iter().map(|(name, n)| {
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.0))
                            .pl(px(34.0))
                            .pr(px(12.0))
                            .py(px(4.0))
                            .text_size(text_size(12.5))
                            .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                            .child(
                                div()
                                    .w(px(90.0))
                                    .h(px(4.0))
                                    .rounded(px(2.0))
                                    .bg(crate::theme::ink(0.06))
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .h_full()
                                            .w(gpui::relative(
                                                *n as f32 / group.total.max(1) as f32,
                                            ))
                                            .bg(theme.text_muted),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(48.0))
                                    .flex()
                                    .justify_end()
                                    .text_color(theme.text_muted)
                                    .child(format_tokens(*n)),
                            )
                    }))
            });
            div()
                .border_t_1()
                .border_color(theme.border)
                .child(header)
                .children(rows)
                .into_any_element()
        });
        let groups: Vec<AnyElement> = groups.collect();
        let header = self.card_header(
            row,
            icons::SPEEDOMETER,
            "Context",
            format!("Claude Code · {}", report.model),
            Some(report.pill()),
            Some(card.text.clone()),
            theme,
            cx,
        );
        let actions =
            vec![
                button(format!("{row}-compact").into(), "Compact now", true, theme)
                    .on_click(
                        cx.listener(|this, _, _, cx| {
                            this.card_action(ClaudeCardAction::Compact, cx)
                        }),
                    )
                    .into_any_element(),
                button(format!("{row}-tools").into(), "Manage tools", false, theme)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.card_action(ClaudeCardAction::ManageTools, cx)
                    }))
                    .into_any_element(),
                button(
                    format!("{row}-ctx-settings").into(),
                    "Context settings",
                    false,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.card_action(ClaudeCardAction::ContextSettings, cx)
                }))
                .into_any_element(),
            ];
        let footer = self.footer(actions, theme);
        Some(
            self.card_shell(row, theme)
                .child(header)
                .child(
                    div()
                        .p(px(12.0))
                        .flex()
                        .flex_col()
                        .child(bar)
                        .child(
                            div()
                                .mt(px(4.0))
                                .text_size(text_size(11.5))
                                .text_color(theme.text_faint)
                                .child(report.bar_caption()),
                        )
                        .child(
                            div()
                                .mt(px(10.0))
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .children(legend),
                        )
                        .when_some(tip, |el, tip| {
                            el.child(
                                div()
                                    .mt(px(10.0))
                                    .px(px(10.0))
                                    .py(px(8.0))
                                    .rounded(px(8.0))
                                    .bg(crate::theme::ink(0.05))
                                    .text_size(text_size(12.5))
                                    .text_color(theme.text_muted)
                                    .child(tip),
                            )
                        })
                        .when_some(card.note.clone(), |el, note| {
                            el.child(
                                div()
                                    .mt(px(8.0))
                                    .text_size(text_size(11.5))
                                    .text_color(theme.warning)
                                    .child(note),
                            )
                        }),
                )
                .children(groups)
                .when(raw_on, |el| el.child(self.raw_block(&card.text, theme)))
                .child(footer)
                .into_any_element(),
        )
    }

    fn render_usage(
        &mut self,
        row: &SharedString,
        card: &CommandCard,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let report = parse_usage(&card.text);
        if report.meters.is_empty() {
            return None;
        }
        let (raw_on, week) = {
            let ui = self.card_ui(row);
            (ui.raw, ui.week)
        };
        let section = report
            .sections
            .iter()
            .find(|s| {
                s.title
                    .starts_with(if week { "Last 7d" } else { "Last 24h" })
            })
            .or_else(|| report.sections.first());
        let range_row = row.clone();
        let segment = |label: &'static str, on: bool, to_week: bool| {
            let range_row = range_row.clone();
            div()
                .id(SharedString::from(format!("{row}-range-{label}")))
                .role(gpui::Role::Button)
                .aria_selected(on)
                .px(px(10.0))
                .py(px(2.0))
                .cursor_pointer()
                .text_size(text_size(12.5))
                .text_color(if on { theme.text } else { theme.text_muted })
                .when(on, |el| el.bg(crate::theme::ink(0.08)))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_card(&range_row, cx, |ui| ui.week = to_week)
                }))
        };
        let suggestions = section.map(|s| {
            let (pcts, others): (Vec<&String>, Vec<&String>) = s.lines.iter().partition(|l| {
                l.split_whitespace()
                    .next()
                    .is_some_and(|w| w.ends_with('%'))
            });
            let rows = pcts.into_iter().map(|line| {
                let (pct, rest) = line.split_once(' ').unwrap_or((line, ""));
                let value: f32 = pct.trim_end_matches('%').parse().unwrap_or(0.0);
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .py(px(4.0))
                    .text_size(text_size(12.5))
                    .child(
                        div()
                            .w(px(38.0))
                            .flex()
                            .justify_end()
                            .text_color(theme.text_muted)
                            .child(pct.to_string()),
                    )
                    .child(
                        div()
                            .w(px(70.0))
                            .h(px(6.0))
                            .flex_none()
                            .rounded(px(3.0))
                            .bg(crate::theme::ink(0.06))
                            .overflow_hidden()
                            .child(
                                div()
                                    .h_full()
                                    .w(gpui::relative((value / 100.0).clamp(0.0, 1.0)))
                                    .bg(theme.text_muted),
                            ),
                    )
                    .child(div().min_w_0().child(rest.trim().to_string()))
            });
            let title = s
                .title
                .split_once(" · ")
                .map(|(_, rest)| format!("{rest} · this Mac only"))
                .unwrap_or_default();
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(16.0))
                        .mt(px(4.0))
                        .mb(px(8.0))
                        .child(
                            div()
                                .text_size(text_size(12.5))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("What uses your limits"),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(theme.border)
                                .overflow_hidden()
                                .child(segment("24h", !week, false))
                                .child(segment("7d", week, true)),
                        )
                        .child(
                            div()
                                .text_size(text_size(11.5))
                                .text_color(theme.text_faint)
                                .child(title),
                        ),
                )
                .children(rows)
                .children((!others.is_empty()).then(|| {
                    div()
                        .mt(px(8.0))
                        .text_size(text_size(11.5))
                        .text_color(theme.text_faint)
                        .child(
                            others
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(" · "),
                        )
                }))
        });
        let sub = usage_subtitle(report.note.as_deref());
        let header = self.card_header(
            row,
            icons::BOLT,
            "Usage",
            sub,
            None,
            Some(card.text.clone()),
            theme,
            cx,
        );
        let footer = self.footer(
            vec![
                button(
                    format!("{row}-usage-page").into(),
                    "Open Usage page",
                    false,
                    theme,
                )
                .on_click(
                    cx.listener(|this, _, _, cx| this.card_action(ClaudeCardAction::OpenUsage, cx)),
                )
                .into_any_element(),
            ],
            theme,
        );
        Some(
            self.card_shell(row, theme)
                .child(header)
                .child(
                    div()
                        .p(px(12.0))
                        .flex()
                        .flex_col()
                        .children(report.meters.into_iter().map(|m| {
                            div()
                                .mx(px(-14.0))
                                .child(meter(theme, m))
                                .into_any_element()
                        }))
                        .children(suggestions),
                )
                .when(raw_on, |el| el.child(self.raw_block(&card.text, theme)))
                .child(footer)
                .into_any_element(),
        )
    }

    fn render_model(
        &mut self,
        row: &SharedString,
        card: &CommandCard,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let live = self.chat_id.as_deref().and_then(|id| {
            self.state
                .read(cx)
                .chats
                .iter()
                .find(|c| c.id == id)
                .and_then(|c| c.config.as_ref())
                .and_then(|c| c.model.clone())
        });
        let current = live.or_else(|| card.model.clone());
        let selected = selected_model(&card.models, current.as_deref());
        let label = selected
            .map(|ix| card.models[ix].1.clone())
            .or_else(|| current.clone())
            .unwrap_or_else(|| "default".into());
        let header = self.card_header(
            row,
            icons::BOLT,
            "Model",
            "Claude Code".into(),
            Some(label),
            None,
            theme,
            cx,
        );
        let chips = card.models.iter().enumerate().map(|(ix, (id, label))| {
            let on = selected == Some(ix);
            let model = id.clone();
            div()
                .id(SharedString::from(format!("{row}-model-{ix}")))
                .debug_selector(move || format!("claude-card-model-{ix}"))
                .role(gpui::Role::Button)
                .aria_selected(on)
                .px(px(10.0))
                .py(px(3.0))
                .rounded_full()
                .border_1()
                .cursor_pointer()
                .text_size(text_size(12.5))
                .border_color(if on { theme.accent } else { theme.border })
                .text_color(if on { theme.accent } else { theme.text })
                .hover(|s| s.bg(theme.glass_hover()))
                .child(label.clone())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.card_action(ClaudeCardAction::Model(model.clone()), cx)
                }))
        });
        self.card_shell(row, theme)
            .child(header)
            .child(
                div()
                    .p(px(12.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .gap(px(6.0))
                            .children(chips),
                    )
                    .child(
                        div()
                            .mt(px(8.0))
                            .text_size(text_size(11.5))
                            .text_color(theme.text_faint)
                            .child(card.note.clone().unwrap_or_else(|| MODEL_FOOTNOTE.into())),
                    ),
            )
            .into_any_element()
    }

    fn render_mcp(
        &mut self,
        row: &SharedString,
        card: &CommandCard,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let servers = mcp_servers(card)?;
        let connected = servers.iter().filter(|(_, s)| s == "connected").count();
        let raw_on = self.card_ui(row).raw;
        let header = self.card_header(
            row,
            icons::REMOTE_SERVER,
            "MCP servers",
            format!(
                "{connected} connected · {} not connected",
                servers.len() - connected
            ),
            None,
            Some(card.text.clone()),
            theme,
            cx,
        );
        let footer =
            self.footer(
                vec![
                    button(
                        format!("{row}-mcp-settings").into(),
                        "Manage in Settings",
                        false,
                        theme,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.card_action(ClaudeCardAction::ManageTools, cx)
                    }))
                    .into_any_element(),
                ],
                theme,
            );
        Some(
            self.card_shell(row, theme)
                .child(header)
                .child(
                    div()
                        .py(px(6.0))
                        .when(servers.is_empty(), |el| {
                            el.child(
                                div()
                                    .px(px(12.0))
                                    .py(px(5.0))
                                    .text_size(text_size(12.5))
                                    .text_color(theme.text_muted)
                                    .child(MCP_EMPTY),
                            )
                        })
                        .children(servers.iter().map(|(name, status)| {
                            let ok = status == "connected";
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .px(px(12.0))
                                .py(px(5.0))
                                .text_size(text_size(12.5))
                                .child(div().size(px(7.0)).rounded_full().bg(if ok {
                                    theme.success
                                } else {
                                    theme.text_faint
                                }))
                                .child(div().flex_1().child(name.clone()))
                                .child(div().text_color(theme.text_muted).child(if ok {
                                    "connected".to_string()
                                } else {
                                    status.replace('-', " ")
                                }))
                        }))
                        .child(
                            div()
                                .px(px(12.0))
                                .pt(px(4.0))
                                .text_size(text_size(11.5))
                                .text_color(theme.text_faint)
                                .child(MCP_FOOTNOTE),
                        ),
                )
                .when(raw_on, |el| el.child(self.raw_block(&card.text, theme)))
                .child(footer)
                .into_any_element(),
        )
    }

    fn render_notice(
        &mut self,
        row: &SharedString,
        name: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let action = terminal_action(name).map(|(label, action)| {
            (
                label,
                match action {
                    TerminalAction::Status => Some(ClaudeCardAction::Status),
                    TerminalAction::Permissions => Some(ClaudeCardAction::Permissions),
                    TerminalAction::Help => Some(ClaudeCardAction::Help),
                    TerminalAction::Hooks => Some(ClaudeCardAction::Hooks),
                    TerminalAction::Export => None,
                },
            )
        });
        div()
            .id(row.clone())
            .debug_selector(|| "claude-card-notice".into())
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .px(px(12.0))
            .py(px(8.0))
            .rounded(px(10.0))
            .border_1()
            .border_dashed()
            .border_color(theme.border_strong)
            .text_size(text_size(12.5))
            .text_color(theme.text_muted)
            .child(
                icon(icons::TERMINAL)
                    .size(px(15.0))
                    .text_color(theme.text_muted),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(theme.font_mono.clone())
                            .text_color(theme.text)
                            .child(format!("/{name}")),
                    )
                    .child("\u{a0}only runs in Claude Code's terminal."),
            )
            .when_some(action, |el, (label, action)| {
                el.child(
                    button(format!("{row}-notice").into(), label, false, theme).on_click(
                        cx.listener(move |this, _, _, cx| match &action {
                            Some(action) => this.card_action(action.clone(), cx),
                            None => this.export_transcript(cx),
                        }),
                    ),
                )
            })
            .into_any_element()
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
