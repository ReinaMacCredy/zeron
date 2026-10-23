use std::rc::Rc;

use gpui::{AnyElement, App, Hsla, SharedString, Window, div, prelude::*, px};
use zeron_proto::{HarnessId, LimitOffer, OfferChoice};
use zeron_rpc::methods;

use crate::route_badge::{brand_mark, target_text};
use crate::state::EngineHandle;
use crate::theme::Theme;

pub(crate) fn resolve_offer(
    engine: EngineHandle,
    chat_id: String,
    device: String,
    choice: &'static str,
    cx: &mut App,
) {
    let params = serde_json::json!({
        "chatId": chat_id,
        "choice": choice,
        "targetDeviceId": device,
    });
    cx.spawn(async move |_| {
        if let Err(error) = engine
            .client()
            .call(methods::RESOLVE_LIMIT_OFFER, params)
            .await
        {
            tracing::warn!(%error, "resolving the limit offer failed");
        }
    })
    .detach();
}

pub(crate) struct LimitCard<'a> {
    pub id: SharedString,
    pub harness: Option<HarnessId>,
    pub offer: &'a LimitOffer,
    pub resets_at: Option<i64>,
    pub auto: bool,
    pub selected: OfferChoice,
    pub countdown: Option<(i64, f32)>,
    pub on_select: Rc<dyn Fn(OfferChoice, &mut Window, &mut App)>,
    pub on_resolve: Rc<dyn Fn(&'static str, &mut Window, &mut App)>,
}

fn radio(selected: bool, tint: Hsla, theme: &Theme) -> AnyElement {
    div()
        .size(px(14.0))
        .flex_none()
        .rounded_full()
        .border_1()
        .border_color(if selected { tint } else { theme.text_faint })
        .flex()
        .items_center()
        .justify_center()
        .when(selected, |el| {
            el.child(div().size(px(6.0)).rounded_full().bg(tint))
        })
        .into_any_element()
}

fn button(
    id: SharedString,
    label: &'static str,
    primary: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label)
        .h(px(26.0))
        .px(px(12.0))
        .flex()
        .items_center()
        .rounded(px(13.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .cursor_pointer()
        .map(|button| {
            if primary {
                button
                    .bg(theme.text)
                    .text_color(theme.on_solid)
                    .hover(|s| s.opacity(0.9))
            } else {
                button
                    .text_color(theme.text.opacity(0.85))
                    .hover(|s| s.bg(crate::theme::ink(0.06)))
            }
        })
        .child(label)
}

pub(crate) fn limit_card(card: LimitCard, theme: &Theme, cx: &App) -> AnyElement {
    let LimitCard {
        id,
        harness,
        offer,
        resets_at,
        auto,
        selected,
        countdown,
        on_select,
        on_resolve,
    } = card;
    let tint = crate::route_badge::fell_back_tint(theme);
    let name = harness.map_or("The agent", HarnessId::display_name);
    let now = chrono::Utc::now().timestamp_millis();
    let reset = resets_at.and_then(crate::route_badge::clock);
    let target = offer.to.as_ref().map(|to| target_text(to, cx));
    let copy = zeron_proto::transcript::limit_card_copy(
        name,
        reset.as_deref(),
        resets_at.map(|ms| ms - now),
        target.as_deref(),
        auto,
    );
    let option =
        |choice: OfferChoice, lead: &str, mark: Option<AnyElement>, title: String, sub: String| {
            let on_select = on_select.clone();
            let chosen = selected == choice;
            div()
                .id(SharedString::from(format!(
                    "{id}-{}",
                    if choice == OfferChoice::Switch {
                        "switch"
                    } else {
                        "wait"
                    }
                )))
                .role(gpui::Role::RadioButton)
                .aria_label(SharedString::from(format!("{lead}{title}")))
                .px(px(10.0))
                .py(px(7.0))
                .flex()
                .flex_row()
                .items_start()
                .gap(px(8.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(if chosen {
                    tint.opacity(0.7)
                } else {
                    crate::theme::hairline(0.08)
                })
                .when(chosen, |el| el.bg(tint.opacity(0.08)))
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    on_select(choice, window, cx);
                })
                .child(div().pt(px(2.0)).child(radio(chosen, tint, theme)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_start()
                                .gap(px(5.0))
                                .text_color(theme.text)
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .when(!lead.is_empty(), |el| {
                                    el.child(
                                        div()
                                            .flex_none()
                                            .child(SharedString::from(lead.to_string())),
                                    )
                                })
                                .when_some(mark, |el, mark| {
                                    el.child(div().flex_none().pt(px(2.0)).child(mark))
                                })
                                .child(div().flex_1().min_w_0().child(SharedString::from(title))),
                        )
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(11.5))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(sub)),
                        ),
                )
        };
    let switch = offer
        .to
        .as_ref()
        .zip(copy.switch)
        .map(|(to, (title, sub))| {
            option(
                OfferChoice::Switch,
                "Switch to ",
                Some(brand_mark(to, 12.0, theme)),
                title,
                sub,
            )
        });
    let (wait_title, wait_sub) = copy.wait;
    let wait = option(OfferChoice::Wait, "", None, wait_title, wait_sub);
    let act = (selected.wire(), selected.act_label());
    let footer = {
        let cancel = on_resolve.clone();
        let now_resolve = on_resolve.clone();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .when_some(countdown, |el, (secs, fraction)| {
                let verb = offer.preselect.countdown_verb();
                el.child(
                    div()
                        .flex_none()
                        .text_size(crate::typography::ui_rems(11.5))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(format!("{verb} in {secs} s"))),
                )
                .child(
                    div()
                        .flex_1()
                        .h(px(4.0))
                        .rounded(px(2.0))
                        .bg(crate::theme::ink(0.08))
                        .child(
                            div()
                                .h_full()
                                .rounded(px(2.0))
                                .w(gpui::relative(fraction.clamp(0.0, 1.0)))
                                .bg(tint),
                        ),
                )
                .child(
                    button(
                        SharedString::from(format!("{id}-cancel")),
                        "Cancel",
                        false,
                        theme,
                    )
                    .debug_selector(|| "limit-card-cancel".into())
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        cancel("hold", window, cx);
                    }),
                )
            })
            .when(countdown.is_none(), |el| el.child(div().flex_1()))
            .child(
                button(SharedString::from(format!("{id}-act")), act.1, true, theme)
                    .debug_selector(|| "limit-card-act".into())
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        now_resolve(act.0, window, cx);
                    }),
            )
    };
    div()
        .py(px(4.0))
        .w_full()
        .child(
            div()
                .id(id.clone())
                .debug_selector(|| "limit-card".into())
                .role(gpui::Role::Group)
                .aria_label(SharedString::from(copy.title.clone()))
                .w_full()
                .p(px(12.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(theme.warning.opacity(0.45))
                .bg(theme.warning.opacity(0.05))
                .text_size(crate::typography::ui_rems(12.5))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            crate::icons::icon(crate::icons::CLOCK_CIRCLE)
                                .size(px(14.0))
                                .flex_none()
                                .text_color(theme.warning),
                        )
                        .child(
                            div()
                                .flex_1()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(SharedString::from(copy.title.clone())),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(crate::typography::ui_rems(11.5))
                                .text_color(theme.text_faint)
                                .child(SharedString::from(copy.resets)),
                        ),
                )
                .children(switch)
                .child(wait)
                .child(footer),
        )
        .into_any_element()
}
