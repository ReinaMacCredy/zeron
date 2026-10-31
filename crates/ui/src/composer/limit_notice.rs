use gpui::{AnyElement, App, Context, SharedString, div, prelude::*, px};
use zeron_proto::{HarnessId, LimitResume};
use zeron_rpc::methods;

use super::Composer;
use crate::icons::{self, icon};
use crate::state::EngineHandle;
use crate::theme::Theme;

pub(super) const NOTICE_PAD: f32 = 8.0;
const NOTICE_BOX: f32 = 38.0;

#[derive(Debug, PartialEq, Eq)]
struct NoticeCopy {
    waiting: bool,
    lead: String,
    rest: String,
    wait: bool,
}

fn copy(
    name: &str,
    window: Option<&str>,
    reset: Option<&str>,
    upcoming: bool,
    auto: bool,
) -> NoticeCopy {
    let reset = reset.filter(|_| upcoming);
    let at = |when: &str| {
        if when.contains(':') {
            format!("at {when}")
        } else {
            when.to_owned()
        }
    };
    if auto && let Some(when) = reset {
        return NoticeCopy {
            waiting: true,
            lead: String::new(),
            rest: format!("Waiting for the {name} reset {}.", at(when)),
            wait: false,
        };
    }
    let lead = match window {
        Some(window) => format!("{name} {window} limit reached."),
        None => format!("{name} usage limit reached."),
    };
    let rest = match reset {
        Some(when) => format!("Resets {}.", at(when)),
        None => "Reset time unknown.".to_owned(),
    };
    NoticeCopy {
        waiting: false,
        lead,
        rest,
        wait: reset.is_some(),
    }
}

fn set_auto(engine: EngineHandle, chat_id: String, device: String, auto: bool, cx: &mut App) {
    let params = serde_json::json!({
        "chatId": chat_id,
        "auto": auto,
        "targetDeviceId": device,
    });
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
}

fn exhausted_window(harness: HarnessId, cx: &App) -> Option<&'static str> {
    let account =
        crate::account_usage::active_account(crate::account_usage::local_snapshot(cx)?, harness)?;
    let label = account
        .usage_windows
        .iter()
        .find(|window| window.used_fraction >= 1.0)?
        .label
        .as_str();
    Some(match zeron_proto::account_usage::window_label(label) {
        "5h" => "5-hour",
        "Weekly" => "weekly",
        "Daily" => "daily",
        _ => return None,
    })
}

impl Composer {
    fn bound_limit(&self, cx: &App) -> Option<(String, String, LimitResume)> {
        if self.side_chat {
            return None;
        }
        let chat_id = self.bound_chat_id(cx)?;
        let (device, limit) = self.state.read(cx).limit_resume(&chat_id)?;
        Some((chat_id, device.to_owned(), limit.clone()))
    }

    pub(super) fn limit_notice_height(&self, cx: &App) -> f32 {
        if self.bound_limit(cx).is_some() {
            NOTICE_PAD + NOTICE_BOX
        } else {
            0.0
        }
    }

    pub(super) fn render_limit_notice(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (chat_id, device, limit) = self.bound_limit(cx)?;
        let engine = self.state.read(cx).engine().cloned()?;
        let harness = self
            .bound_chat_row(cx)
            .and_then(|chat| chat.config.as_ref())
            .map(|config| config.harness);
        let name = harness.map_or("The agent", HarnessId::display_name);
        let now = chrono::Utc::now().timestamp_millis();
        let reset = limit.resets_at.and_then(crate::route_badge::clock);
        let notice = copy(
            name,
            harness.and_then(|harness| exhausted_window(harness, cx)),
            reset.as_deref(),
            limit.resets_at.is_some_and(|at| at > now),
            limit.auto,
        );
        let offer_to = limit.offer.as_ref().and_then(|offer| offer.to.clone());
        let has_offer = limit.offer.is_some();
        let shows_switch = !notice.waiting || offer_to.is_some();
        let switch = {
            let engine = engine.clone();
            let chat_id = chat_id.clone();
            let device = device.clone();
            let label: SharedString = match &offer_to {
                Some(to) => {
                    let model = to.model.as_deref().map_or_else(
                        || to.harness.display_name().to_string(),
                        |model| crate::pickers::model_display_label(to.harness, model, cx),
                    );
                    format!("Switch to {model}").into()
                }
                None => "Switch model".into(),
            };
            let primary = !notice.waiting;
            crate::limit_card::button("limit-notice-switch".into(), label, primary, theme)
                .debug_selector(|| "limit-notice-switch".into())
                .on_click(cx.listener(move |this, _, window, cx| {
                    if offer_to.is_some() {
                        crate::limit_card::resolve_offer(
                            engine.clone(),
                            chat_id.clone(),
                            device.clone(),
                            "switch",
                            cx,
                        );
                    } else {
                        this.open_model_menu(window, cx);
                    }
                }))
        };
        let wait = notice.wait.then(|| {
            let engine = engine.clone();
            let chat_id = chat_id.clone();
            let device = device.clone();
            let auto = limit.auto;
            crate::limit_card::button("limit-notice-wait".into(), "Wait for reset", false, theme)
                .debug_selector(|| "limit-notice-wait".into())
                .on_click(move |_, _, cx| {
                    if has_offer {
                        crate::limit_card::resolve_offer(
                            engine.clone(),
                            chat_id.clone(),
                            device.clone(),
                            "wait",
                            cx,
                        );
                    }
                    if !auto {
                        set_auto(engine.clone(), chat_id.clone(), device.clone(), true, cx);
                    }
                })
        });
        let cancel = notice.waiting.then(|| {
            crate::limit_card::button("limit-notice-cancel".into(), "Cancel", false, theme)
                .debug_selector(|| "limit-notice-cancel".into())
                .on_click(move |_, _, cx| {
                    set_auto(engine.clone(), chat_id.clone(), device.clone(), false, cx);
                })
        });
        let tint = if notice.waiting {
            theme.text_muted
        } else {
            theme.danger
        };
        let body = div()
            .id("limit-notice")
            .debug_selector(|| "limit-notice".into())
            .role(gpui::Role::Status)
            .aria_label(SharedString::from(
                format!("{} {}", notice.lead, notice.rest).trim().to_owned(),
            ))
            .h(px(NOTICE_BOX))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .pl(px(10.0))
            .pr(px(6.0))
            .rounded(px(10.0))
            .bg(if notice.waiting {
                crate::theme::ink(0.05)
            } else {
                theme.danger.opacity(0.1)
            })
            .text_size(crate::typography::ui_rems(12.5))
            .text_color(theme.text)
            .child(
                icon(if notice.waiting {
                    icons::CLOCK_CIRCLE
                } else {
                    icons::DANGER_TRIANGLE
                })
                .size(px(14.0))
                .flex_none()
                .text_color(tint),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(!notice.lead.is_empty(), |el| {
                        el.child(
                            gpui::StyledText::new(format!("{} {}", notice.lead, notice.rest))
                                .with_highlights([(
                                    0..notice.lead.len(),
                                    gpui::HighlightStyle {
                                        color: Some(theme.danger),
                                        font_weight: Some(gpui::FontWeight::SEMIBOLD),
                                        ..Default::default()
                                    },
                                )]),
                        )
                    })
                    .when(notice.lead.is_empty(), |el| el.child(notice.rest.clone())),
            )
            .when(shows_switch, |el| el.child(switch))
            .children(wait)
            .children(cancel);
        Some(
            div()
                .flex_none()
                .pt(px(NOTICE_PAD))
                .px(px(NOTICE_PAD))
                .child(body)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notice_waits_only_on_an_armed_upcoming_reset() {
        let waiting = copy("Claude Code", Some("5-hour"), Some("4:10 PM"), true, true);
        assert!(waiting.waiting && !waiting.wait);
        assert_eq!(
            waiting.rest,
            "Waiting for the Claude Code reset at 4:10 PM."
        );
        let limited = copy("Claude Code", Some("5-hour"), Some("4:10 PM"), true, false);
        assert!(!limited.waiting && limited.wait);
        assert_eq!(limited.lead, "Claude Code 5-hour limit reached.");
        assert_eq!(limited.rest, "Resets at 4:10 PM.");
        let passed = copy("Codex", None, Some("4:10 PM"), false, true);
        assert!(!passed.waiting && !passed.wait);
        assert_eq!(passed.rest, "Reset time unknown.");
        assert_eq!(
            copy("Codex", None, Some("Mon"), true, false).rest,
            "Resets Mon."
        );
    }
}
