use chrono::{DateTime, Utc};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};
use zeron_rpc::methods;

use super::Composer;
use super::background::{frosted_pill, pill_frame, pill_leading};
use crate::icons::{self, icon};
use crate::theme::Theme;

fn label(resets_at: Option<i64>, auto: bool, now: DateTime<Utc>) -> (String, Option<&'static str>) {
    let reset = resets_at.and_then(DateTime::from_timestamp_millis);
    let when = crate::settings::accounts::format_reset(reset, now)
        .map(|when| when.trim_start_matches("resets ").to_owned());
    zeron_proto::transcript::limit_pill_label(
        when.as_deref(),
        reset.is_some_and(|at| at > now),
        auto,
    )
}

impl Composer {
    pub(super) fn render_limit_pill(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let chat_id = self.bound_chat_id(cx)?;
        let state = self.state.read(cx);
        let (device, limit) = state.limit_resume(&chat_id)?;
        let (text, action) = label(limit.resets_at, limit.auto, Utc::now());
        let auto = limit.auto;
        let switch_to = limit.offer.as_ref().and_then(|offer| offer.to.clone());
        let device = device.to_owned();
        let engine = state.engine().cloned();
        let pill = pill_frame("limit-pill", theme)
            .debug_selector(|| "limit-pill".into())
            .cursor_default()
            .child(pill_leading(
                icon(icons::CLOCK_CIRCLE)
                    .size(px(14.0))
                    .text_color(theme.text_muted)
                    .into_any_element(),
            ))
            .child(div().text_color(theme.text).child(SharedString::from(text)))
            .when_some(switch_to.zip(engine.clone()), |el, (to, engine)| {
                let model = to.model.as_deref().map_or_else(
                    || to.harness.display_name().to_string(),
                    |model| crate::pickers::model_display_label(to.harness, model, cx),
                );
                let chat_id = chat_id.clone();
                let device = device.clone();
                el.child(
                    div()
                        .id("limit-pill-switch")
                        .debug_selector(|| "limit-pill-switch".into())
                        .ml(px(6.0))
                        .text_color(theme.accent)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .child(SharedString::from(format!("Switch to {model}")))
                        .on_click(move |_, _, cx| {
                            crate::limit_card::resolve_offer(
                                engine.clone(),
                                chat_id.clone(),
                                device.clone(),
                                "switch",
                                cx,
                            );
                        }),
                )
            })
            .when_some(action.zip(engine), |el, (action, engine)| {
                el.child(
                    div()
                        .id("limit-pill-action")
                        .ml(px(6.0))
                        .text_color(theme.accent)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .child(action)
                        .on_click(move |_, _, cx| {
                            let params = serde_json::json!({
                                "chatId": chat_id,
                                "auto": !auto,
                                "targetDeviceId": device,
                            });
                            let engine = engine.clone();
                            cx.spawn(async move |_| {
                                if let Err(error) = engine
                                    .client()
                                    .call(methods::SET_LIMIT_RESUME, params)
                                    .await
                                {
                                    tracing::warn!(%error, "limit resume toggle failed");
                                }
                            })
                            .detach();
                        }),
                )
            });
        Some(frosted_pill(pill, 1.0).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_offers_only_what_the_reset_allows() {
        let now = Utc::now();
        let soon = (now + chrono::Duration::minutes(30)).timestamp_millis();
        let past = (now - chrono::Duration::minutes(1)).timestamp_millis();
        assert_eq!(label(Some(soon), true, now).1, Some("Cancel"));
        assert_eq!(label(Some(soon), false, now).1, Some("Resume at reset"));
        assert_eq!(label(Some(past), false, now).1, None);
        assert_eq!(label(None, false, now).1, None);
    }
}
