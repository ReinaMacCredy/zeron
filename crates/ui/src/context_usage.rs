//! Context occupancy is read from the replicated chat snapshot, never local CLI state.
use crate::theme::Theme;
use gpui::{
    Context, IntoElement, PathBuilder, Render, SharedString, Window, canvas, div, point,
    prelude::*, px,
};
use zeron_proto::ContextUsage;

pub fn render(
    usage: Option<ContextUsage>,
    state: gpui::Entity<crate::state::AppState>,
    composer: gpui::Entity<crate::composer::Composer>,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let fraction = usage.and_then(ContextUsage::fraction);
    let color = match fraction {
        Some(f) if f >= 0.9 => theme.danger,
        Some(f) if f >= 0.75 => theme.warning,
        Some(_) => theme.text_muted,
        None => theme.text_faint,
    };
    let track = theme.text_faint.opacity(0.25);
    let ring = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let mut arc = |fraction: f32, color| {
                if fraction <= 0.0 {
                    return;
                }
                let steps = (64.0 * fraction).ceil().max(2.0) as usize;
                let mut path = PathBuilder::stroke(px(1.8));
                for i in 0..=steps {
                    let angle = -std::f32::consts::FRAC_PI_2
                        + std::f32::consts::TAU * fraction * i as f32 / steps as f32;
                    let p = point(
                        center.x + px(6.0 * angle.cos()),
                        center.y + px(6.0 * angle.sin()),
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
            arc(fraction.unwrap_or(0.0).clamp(0.0, 1.0) as f32, color);
        },
    )
    .size(px(16.0));
    let label = fraction
        .map(|f| format!("{:.0}%", f * 100.0))
        .unwrap_or_else(|| "—".into());
    div()
        .id("context-usage")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(5.0))
        .h(px(24.0))
        .px(px(6.0))
        .rounded(px(6.0))
        .text_size(px(11.0))
        .text_color(color)
        .hover(|s| s.bg(crate::theme::ink(0.05)))
        .child(ring)
        .child(SharedString::from(label))
        // The card carries an action, so it must survive the pointer
        // travelling into it.
        .hoverable_tooltip(move |_, cx| {
            cx.new(|cx| UsageCard {
                _subscription: cx.observe(&state, |_, _, cx| cx.notify()),
                // The compaction's pending state is composer-local; without
                // this the button stays live through its own run.
                _composer_subscription: cx.observe(&composer, |_, _, cx| cx.notify()),
                state: state.clone(),
                composer: composer.clone(),
            })
            .into()
        })
}

struct UsageCard {
    state: gpui::Entity<crate::state::AppState>,
    composer: gpui::Entity<crate::composer::Composer>,
    _subscription: gpui::Subscription,
    _composer_subscription: gpui::Subscription,
}

fn details(usage: Option<ContextUsage>) -> String {
    match usage.unwrap_or_default() {
        ContextUsage {
            tokens: Some(tokens),
            window: Some(window),
        } if window > 0 => {
            format!(
                "{} / {} tokens\n{} tokens remaining",
                tokens,
                window,
                window.saturating_sub(tokens)
            )
        }
        ContextUsage {
            tokens: Some(tokens),
            ..
        } => format!("{tokens} tokens used\nContext limit not reported"),
        ContextUsage {
            window: Some(window),
            ..
        } if window > 0 => format!("{window} token capacity\nWaiting for context usage"),
        _ => "Context usage not reported by this harness yet".into(),
    }
}

impl Render for UsageCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &Theme::of(cx).for_popup();
        let card = crate::popover::popover_card(theme)
            .w(px(260.0))
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child("Context window"),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .line_height(px(19.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(details(
                        self.state.read(cx).context_usage,
                    ))),
            );
        let reason = self.composer.read(cx).compact_disabled_reason(cx);
        let composer = self.composer.clone();
        let card = card
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
                            .on_click(move |_, _, cx| {
                                composer.update(cx, |composer, cx| {
                                    composer.request_compaction(None, cx)
                                });
                            })
                    })
                    .child("Compact context"),
            )
            // Never hidden: the reason is why the button is there at all.
            .when_some(reason, |card, reason| {
                card.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(theme.text_muted.opacity(0.8))
                        .child(reason),
                )
            });
        crate::frost::frosted(crate::popover::CARD_RADIUS, crate::frost::MENU_BLUR, card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
