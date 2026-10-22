//! Context occupancy is read from the replicated chat snapshot, never local CLI state.
use crate::popover;
use crate::session_stats::CompactView;
use crate::state::AppState;
use crate::theme::Theme;
use gpui::{
    Context, Entity, IntoElement, PathBuilder, Render, SharedString, Subscription, WeakEntity,
    Window, canvas, div, point, prelude::*, px,
};
use zeron_proto::ContextUsage;

pub struct FooterContext {
    state: Entity<AppState>,
    composer: WeakEntity<crate::composer::Composer>,
    chat: Option<String>,
    popup: popover::Popup<()>,
    ticker: Option<gpui::Task<()>>,
    _state: Subscription,
}

impl FooterContext {
    pub fn new(
        state: Entity<AppState>,
        composer: WeakEntity<crate::composer::Composer>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _state: cx.observe(&state, |_, _, cx| cx.notify()),
            state,
            composer,
            chat: None,
            popup: popover::Popup::default(),
            ticker: None,
        }
    }

    pub fn track(&mut self, chat: Option<String>) {
        self.chat = chat;
    }

    /// A trigger click: open the card, or close it when the press found it
    /// open (the card's mouse-down-out already began that close — see
    /// `Popup::note_trigger_press`).
    fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.popup.take_press_was_open() || self.popup.as_open().is_some() {
            self.dismiss(cx);
            return;
        }
        self.popup.open(());
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.popup.begin_close() {
            popover::reap_popup(cx, |ring: &mut Self| &mut ring.popup);
        }
        cx.notify();
    }

    fn compact(&self, cx: &Context<Self>) -> Option<CompactView> {
        let state = self.state.read(cx);
        let chat = self.chat.as_deref().or(state.selected_chat.as_deref())?;
        let session = state.session_for(chat)?;
        Some(CompactView {
            plan: session.compact_plan,
            compacting: state.chat_compacting(chat),
            stats: state.session_stats_for(self.chat.as_deref()),
        })
        .filter(CompactView::shown)
    }

    fn context_card(
        &self,
        context: Option<ContextUsage>,
        compact: Option<CompactView>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let theme = Theme::of(cx).for_popup();
        let card = card(context, &theme).when_some(compact, |card, compact| {
            card.child(crate::session_stats::compact_section(&compact, &theme))
        });
        let Some(composer) = self.composer.upgrade() else {
            return card;
        };
        let reason = composer.read(cx).compact_disabled_reason(cx);
        let resume = composer.read(cx).resume_compaction(cx);
        card.child(
            div()
                .px(px(8.0))
                .pb(px(8.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .when_some(resume, |col, offer| {
                    col.child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.warning)
                            .child(format!(
                                "Idle for {}; the prompt cache has expired.",
                                offer.idle_label()
                            )),
                    )
                })
                .child(
                    div()
                        .id("compact-context")
                        .h(px(26.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(crate::theme::hairline(0.1))
                        .text_size(px(12.0))
                        .text_color(theme.text)
                        .when(reason.is_some(), |button| button.opacity(0.45))
                        .when(reason.is_none(), |button| {
                            button
                                .cursor_pointer()
                                .hover(|s| s.bg(crate::theme::ink(0.06)))
                                .on_click(cx.listener(move |ring, _, _, cx| {
                                    composer.update(cx, |composer, cx| {
                                        composer.request_compaction(None, cx)
                                    });
                                    ring.dismiss(cx);
                                }))
                        })
                        .child("Compact context"),
                )
                // Never hidden: the reason is why the button is there at all.
                .when_some(reason, |col, reason| {
                    col.child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.text_muted.opacity(0.8))
                            .child(reason),
                    )
                }),
        )
    }
}

impl Render for FooterContext {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let context = self.state.read(cx).context_usage_for(self.chat.as_deref());
        let compact = self.compact(cx);
        if self.popup.get().is_none() {
            self.ticker = None;
        } else if self.ticker.is_none() && compact.is_some() {
            self.ticker = Some(crate::session_stats::tick(1, cx, |_, cx| cx.notify()));
        }
        let ring = has_window(context).then(|| {
            let chip = chip(context, compact, self.popup.get().is_some(), &theme)
                .relative()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|ring, _, _, _| ring.popup.note_trigger_press()),
                )
                .on_click(cx.listener(|ring, _, _, cx| ring.toggle(cx)));
            if self.popup.get().is_none() {
                return chip.into_any_element();
            }
            let content = self
                .context_card(context, compact, cx)
                .on_mouse_down_out(cx.listener(|ring, _, _, cx| ring.dismiss(cx)))
                .into_any_element();
            chip.child(popover::anchored_menu_above_end(
                "context-usage-menu",
                content,
                self.popup.closing_since(),
            ))
            .into_any_element()
        });
        div().flex().items_center().children(ring)
    }
}

/// The context ring's trigger chip; [`FooterContext`] opens [`card`] from it
/// on click.
fn chip(
    usage: Option<ContextUsage>,
    compact: Option<CompactView>,
    open: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let fraction = usage.and_then(ContextUsage::fraction);
    let color = match fraction {
        _ if compact.is_some_and(|c| c.compacting) => theme.busy,
        _ if compact.is_some() => theme.accent,
        Some(f) if f >= 0.9 => theme.danger,
        Some(f) if f >= 0.75 => theme.warning,
        Some(_) => theme.text_muted,
        None => theme.text_faint,
    };
    let label = fraction
        .map(|f| format!("{:.0}%", f * 100.0))
        .unwrap_or_else(|| "—".into());
    ring_chip(
        "context-usage",
        fraction.unwrap_or(0.0) as f32,
        color,
        color,
        label,
        open,
        theme,
    )
    .when(compact.is_some(), |chip| {
        chip.relative().child(
            div()
                .absolute()
                .left(px(19.0))
                .top(px(1.0))
                .size(px(5.0))
                .rounded_full()
                .bg(color),
        )
    })
}

/// One footer ring indicator: ring + percent. `arc` colours the ring's fill,
/// `text` the label; `open` holds the hover wash while its popover is up.
fn ring_chip(
    id: &'static str,
    fraction: f32,
    arc: gpui::Hsla,
    text: gpui::Hsla,
    label: String,
    open: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(5.0))
        .h(px(24.0))
        .px(px(6.0))
        .rounded(px(6.0))
        .text_size(px(11.0))
        .text_color(text)
        .cursor_pointer()
        .when(open, |s| s.bg(crate::theme::ink(0.05)))
        .hover(|s| s.bg(crate::theme::ink(0.05)))
        .child(ring(fraction, arc, theme))
        .child(SharedString::from(label))
}

/// The footer's 16px progress ring: a faint full track under a `fraction` arc
/// starting at twelve o'clock.
fn ring(fraction: f32, color: gpui::Hsla, theme: &Theme) -> impl IntoElement {
    progress_ring(
        16.0,
        6.0,
        1.8,
        fraction,
        color,
        theme.text_faint.opacity(0.25),
    )
}

pub(crate) fn progress_ring(
    size: f32,
    radius: f32,
    stroke: f32,
    fraction: f32,
    color: gpui::Hsla,
    track: gpui::Hsla,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let mut arc = |fraction: f32, color| {
                if fraction <= 0.0 {
                    return;
                }
                let steps = (64.0 * fraction).ceil().max(2.0) as usize;
                let mut path = PathBuilder::stroke(px(stroke));
                for i in 0..=steps {
                    let angle = -std::f32::consts::FRAC_PI_2
                        + std::f32::consts::TAU * fraction * i as f32 / steps as f32;
                    let p = point(
                        center.x + px(radius * angle.cos()),
                        center.y + px(radius * angle.sin()),
                    );
                    if i == 0 {
                        path.move_to(p);
                    } else {
                        path.line_to(p);
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            };
            arc(1.0, track);
            arc(fraction.clamp(0.0, 1.0), color);
        },
    )
    .size(px(size))
}

/// Claude reports the model's whole window, and only from its next turn; the
/// chat's own picks (200K, or a custom auto-compact window) are the budget.
pub(crate) fn with_chat_budget(
    usage: ContextUsage,
    config: Option<&zeron_proto::ChatConfig>,
) -> ContextUsage {
    ContextUsage {
        window: config
            .and_then(zeron_harness::claude::catalog::chat_budget)
            .or(usage.window),
        ..usage
    }
}

fn with_separators(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// whether the indicator has anything to measure against: harnesses that
/// never report a window (antigravity) get no indicator at all, rather than a
/// permanently empty ring.
pub fn has_window(usage: Option<ContextUsage>) -> bool {
    usage
        .and_then(|usage| usage.window)
        .is_some_and(|window| window > 0)
}

fn details(usage: Option<ContextUsage>) -> String {
    match usage.unwrap_or_default() {
        ContextUsage {
            tokens: Some(tokens),
            window: Some(window),
        } if window > 0 => {
            format!(
                "{} / {} tokens\n{} tokens remaining",
                with_separators(tokens),
                with_separators(window),
                with_separators(window.saturating_sub(tokens))
            )
        }
        ContextUsage {
            tokens: Some(tokens),
            ..
        } => format!(
            "{} tokens used\nContext limit not reported",
            with_separators(tokens)
        ),
        ContextUsage {
            window: Some(window),
            ..
        } if window > 0 => format!(
            "{} token capacity\nWaiting for context usage",
            with_separators(window)
        ),
        _ => "Context usage not reported by this harness yet".into(),
    }
}

/// The context ring's popover content.
pub fn card(usage: Option<ContextUsage>, theme: &Theme) -> gpui::Div {
    crate::popover::popover_card(theme)
        .flex()
        .flex_col()
        .child(crate::popover::menu_heading(theme, "Context window"))
        .child(
            div()
                .px(px(8.0))
                .pb(px(6.0))
                .text_size(px(12.0))
                .line_height(px(19.0))
                .whitespace_nowrap()
                .text_color(theme.text_muted)
                .child(SharedString::from(details(usage))),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indicator_needs_a_reported_window() {
        assert!(!has_window(None));
        assert!(!has_window(Some(ContextUsage {
            tokens: Some(1_200),
            window: None,
        })));
        assert!(!has_window(Some(ContextUsage {
            tokens: Some(1_200),
            window: Some(0),
        })));
        assert!(has_window(Some(ContextUsage {
            tokens: None,
            window: Some(200_000),
        })));
    }

    #[test]
    fn missing_usage_is_distinct_from_zero_and_overflow() {
        assert!(details(None).contains("not reported"));
        assert!(
            details(Some(ContextUsage {
                tokens: Some(0),
                window: Some(200)
            }))
            .contains("200 tokens remaining")
        );
        assert!(
            details(Some(ContextUsage {
                tokens: Some(250),
                window: Some(200)
            }))
            .contains("0 tokens remaining")
        );
        assert!(
            details(Some(ContextUsage {
                tokens: Some(10),
                window: Some(0)
            }))
            .contains("limit not reported")
        );
    }

    #[test]
    fn a_claude_chats_picks_set_its_budget() {
        let reported = ContextUsage {
            tokens: Some(83_378),
            window: Some(1_000_000),
        };
        let config = |harness, picks: &[(&str, &str)]| zeron_proto::ChatConfig {
            harness,
            model: None,
            reasoning: None,
            model_options: picks
                .iter()
                .map(|(k, v)| ((*k).to_owned(), serde_json::json!(v)))
                .collect(),
            sandbox: zeron_proto::SandboxLevel::WorkspaceWrite,
            permission_mode: Default::default(),
            instructions: String::new(),
            profile_id: None,
            instance_id: None,
            disabled_tools: Vec::new(),
        };
        let window =
            |config: Option<&zeron_proto::ChatConfig>| with_chat_budget(reported, config).window;
        let claude = zeron_proto::HarnessId::ClaudeCode;
        assert_eq!(window(None), Some(1_000_000));
        assert_eq!(window(Some(&config(claude, &[]))), Some(1_000_000));
        assert_eq!(
            window(Some(&config(claude, &[("autoCompact", "400k")]))),
            Some(400_000)
        );
        assert_eq!(
            window(Some(&config(claude, &[("contextWindow", "200k")]))),
            Some(200_000)
        );
        assert_eq!(
            window(Some(&config(
                claude,
                &[("contextWindow", "200k"), ("autoCompact", "400k")]
            ))),
            Some(200_000)
        );
        assert_eq!(
            window(Some(&config(
                zeron_proto::HarnessId::Codex,
                &[("autoCompact", "400k")]
            ))),
            Some(1_000_000)
        );
        assert_eq!(
            details(Some(with_chat_budget(
                reported,
                Some(&config(claude, &[("autoCompact", "400k")]))
            ))),
            "83,378 / 400,000 tokens\n316,622 tokens remaining"
        );
    }

    #[test]
    fn token_counts_are_grouped_by_thousands() {
        assert_eq!(with_separators(0), "0");
        assert_eq!(with_separators(999), "999");
        assert_eq!(with_separators(5417), "5,417");
        assert_eq!(with_separators(1_048_576), "1,048,576");
        assert_eq!(
            details(Some(ContextUsage {
                tokens: Some(5417),
                window: Some(1_048_576)
            })),
            "5,417 / 1,048,576 tokens\n1,043,159 tokens remaining"
        );
    }
}
