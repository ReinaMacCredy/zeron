use gpui::{AnyElement, App, ClickEvent, FontWeight, Hsla, Window, div, prelude::*, px};
use zeron_engine::computer::engine_seams::{
    COMPUTER_FOREGROUND_ALLOW, COMPUTER_FOREGROUND_DECLINE,
};
use zeron_proto::computer::{ComputerPendingKind, ComputerPendingRequest};

use super::presence::{ChatPresence, PresenceTone};
use crate::theme::Theme;

pub const STRIP_HEIGHT: f32 = 34.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripCopy {
    pub lead: Option<String>,
    pub detail: String,
}

impl StripCopy {
    pub fn text(&self) -> String {
        match &self.lead {
            Some(lead) => format!("{lead} · {}", self.detail),
            None => self.detail.clone(),
        }
    }
}

pub fn foreground_request(presence: &ChatPresence) -> Option<&ComputerPendingRequest> {
    presence
        .pending
        .as_ref()
        .filter(|pending| matches!(pending.kind, ComputerPendingKind::Foreground { .. }))
}

pub fn strip_copy(presence: &ChatPresence) -> StripCopy {
    if foreground_request(presence).is_some() {
        return StripCopy {
            lead: Some(format!(
                "{} needs to come to the front to type",
                presence.app.as_deref().unwrap_or("The app")
            )),
            detail: "It will cover your screen; don't type while the agent works".into(),
        };
    }
    match presence.tone {
        PresenceTone::Driving | PresenceTone::NeedsYou => StripCopy {
            lead: Some(presence.app.clone().unwrap_or_else(|| "Desktop".into())),
            detail: presence
                .activity
                .clone()
                .unwrap_or_else(|| "Working".into()),
        },
        PresenceTone::Paused if presence.user_takeover => StripCopy {
            lead: Some(format!(
                "You're using {}",
                presence.app.as_deref().unwrap_or("the target window")
            )),
            detail: "the agent will re-read the screen when you stop".into(),
        },
        PresenceTone::Paused => StripCopy {
            lead: Some(format!(
                "{} isn't reachable",
                presence.app.as_deref().unwrap_or("The target window")
            )),
            detail: "bring it onto this desktop or exit full screen".into(),
        },
        PresenceTone::Revoked { summing_up: true } => StripCopy {
            lead: None,
            detail: "Desktop access revoked · the agent is summing up".into(),
        },
        PresenceTone::Revoked { summing_up: false } => StripCopy {
            lead: None,
            detail: "Desktop access revoked".into(),
        },
    }
}

pub fn shows_stop(presence: &ChatPresence) -> bool {
    presence.tone == PresenceTone::Driving
}

fn tone_color(tone: PresenceTone, theme: &Theme) -> Option<Hsla> {
    match tone {
        PresenceTone::Driving | PresenceTone::NeedsYou => Some(theme.accent),
        PresenceTone::Paused => Some(theme.warning),
        PresenceTone::Revoked { .. } => None,
    }
}

pub fn chat_glyph(
    surface: &str,
    chat_id: &str,
    size: f32,
    opacity: f32,
    theme: &Theme,
    cx: &App,
) -> Option<AnyElement> {
    let tone = super::presence::tone(chat_id, cx)?;
    let color = match tone {
        PresenceTone::Driving | PresenceTone::NeedsYou => theme.accent,
        PresenceTone::Paused => theme.warning,
        PresenceTone::Revoked { .. } => return None,
    };
    let selector = format!("{surface}-computer-{chat_id}");
    Some(
        div()
            .debug_selector(move || selector)
            .flex_none()
            .size(px(size))
            .child(
                crate::icons::icon(crate::icons::CURSOR)
                    .size(px(size))
                    .text_color(color.opacity(opacity)),
            )
            .into_any_element(),
    )
}

type OnForeground = Box<dyn Fn(bool, &mut Window, &mut App)>;

fn strip_button(
    id: &'static str,
    label: &'static str,
    primary: bool,
    on_foreground: &std::rc::Rc<OnForeground>,
    allow: bool,
    theme: &Theme,
) -> AnyElement {
    let on_foreground = on_foreground.clone();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_none()
        .h(px(24.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(7.0))
        .border_1()
        .text_size(px(12.0))
        .cursor_pointer()
        .map(|button| {
            if primary {
                button
                    .border_color(theme.accent)
                    .bg(theme.accent)
                    .text_color(gpui::white())
                    .hover(|s| s.opacity(0.9))
            } else {
                button
                    .border_color(crate::theme::hairline(0.11))
                    .bg(crate::theme::ink(0.09))
                    .hover(|s| s.bg(crate::theme::ink(0.14)))
            }
        })
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_foreground(allow, window, cx)
        })
        .child(label)
        .into_any_element()
}

pub fn computer_strip(
    presence: &ChatPresence,
    show_esc: bool,
    on_stop: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
    on_foreground: OnForeground,
    theme: &Theme,
) -> AnyElement {
    let copy = strip_copy(presence);
    let tint = tone_color(presence.tone, theme);
    let asking = foreground_request(presence).is_some();
    let (bg, border, glyph) = match tint {
        Some(tint) => (
            tint.opacity(if asking { 0.16 } else { 0.13 }),
            tint.opacity(0.38),
            tint,
        ),
        None => (
            crate::theme::ink(0.05),
            crate::theme::hairline(0.11),
            theme.text_muted,
        ),
    };
    let stop = shows_stop(presence);
    let on_foreground = std::rc::Rc::new(on_foreground);
    div()
        .id("computer-strip")
        .debug_selector(|| "computer-strip".into())
        .map(|strip| {
            if asking {
                strip.min_h(px(STRIP_HEIGHT)).py(px(7.0))
            } else {
                strip.h(px(STRIP_HEIGHT))
            }
        })
        .flex()
        .flex_row()
        .items_center()
        .gap(px(9.0))
        .pl(px(11.0))
        .pr(px(6.0))
        .rounded_t(px(11.0))
        .border_t_1()
        .border_l_1()
        .border_r_1()
        .border_color(border)
        .bg(bg)
        .overflow_hidden()
        .text_size(px(12.5))
        .text_color(theme.text)
        .child(
            crate::icons::icon(crate::icons::CURSOR)
                .flex_none()
                .size(px(14.0))
                .text_color(glyph),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_row()
                .items_center()
                .map(|row| {
                    if asking {
                        row.flex_wrap()
                    } else {
                        row.whitespace_nowrap()
                    }
                })
                .when_some(copy.lead, |row, lead| {
                    row.child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(lead),
                    )
                    .when(!asking, |row| {
                        row.child(div().flex_none().child("\u{a0}·\u{a0}"))
                    })
                })
                .child(
                    div()
                        .min_w_0()
                        .when(!asking, |detail| detail.truncate())
                        .text_color(theme.text.opacity(0.74))
                        .child(copy.detail),
                ),
        )
        .when(asking, |row| {
            row.child(strip_button(
                "computer-strip-allow-foreground",
                COMPUTER_FOREGROUND_ALLOW,
                true,
                &on_foreground,
                true,
                theme,
            ))
            .child(strip_button(
                "computer-strip-decline-foreground",
                COMPUTER_FOREGROUND_DECLINE,
                false,
                &on_foreground,
                false,
                theme,
            ))
        })
        .when(stop && show_esc, |row| {
            row.child(
                div()
                    .flex_none()
                    .px(px(5.0))
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(crate::theme::hairline(0.11))
                    .font_family(theme.font_mono.clone())
                    .text_size(px(11.0))
                    .text_color(theme.text_muted)
                    .child("esc"),
            )
        })
        .when(stop, |row| {
            row.child(
                div()
                    .id("computer-strip-stop")
                    .debug_selector(|| "computer-strip-stop".into())
                    .flex_none()
                    .h(px(24.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(crate::theme::hairline(0.11))
                    .bg(crate::theme::ink(0.09))
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.danger.opacity(0.25)).border_color(theme.danger))
                    .on_click(move |event, window, cx| {
                        cx.stop_propagation();
                        on_stop(event, window, cx)
                    })
                    .child("Stop"),
            )
        })
        .into_any_element()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn presence(tone: PresenceTone) -> ChatPresence {
        ChatPresence {
            chat_id: "a".into(),
            app: Some("Ghostty".into()),
            window_id: Some("cua:42:7".into()),
            activity: Some("Typing".into()),
            tone,
            user_takeover: false,
            pending: None,
        }
    }

    #[test]
    fn the_strip_says_app_and_activity_and_stops_only_while_driving() {
        let driving = presence(PresenceTone::Driving);
        assert_eq!(strip_copy(&driving).text(), "Ghostty · Typing");
        assert!(shows_stop(&driving));
        let bare = ChatPresence {
            app: None,
            activity: None,
            ..driving.clone()
        };
        assert_eq!(strip_copy(&bare).text(), "Desktop · Working");
        let paused = presence(PresenceTone::Paused);
        assert_eq!(
            strip_copy(&paused).text(),
            "Ghostty isn't reachable · bring it onto this desktop or exit full screen"
        );
        assert!(!shows_stop(&paused));
        let taken = ChatPresence {
            user_takeover: true,
            ..paused.clone()
        };
        assert_eq!(
            strip_copy(&taken).text(),
            "You're using Ghostty · the agent will re-read the screen when you stop"
        );
        assert!(!shows_stop(&taken));
        assert_eq!(
            strip_copy(&ChatPresence { app: None, ..taken }).text(),
            "You're using the target window · the agent will re-read the screen when you stop"
        );
        let summing = presence(PresenceTone::Revoked { summing_up: true });
        assert_eq!(
            strip_copy(&summing).text(),
            "Desktop access revoked · the agent is summing up"
        );
        assert!(!shows_stop(&summing));
        assert_eq!(
            strip_copy(&presence(PresenceTone::Revoked { summing_up: false })).text(),
            "Desktop access revoked"
        );
    }

    pub(crate) fn foreground_pending(app: Option<&str>) -> ComputerPendingRequest {
        ComputerPendingRequest {
            input_id: "computer-foreground-1".into(),
            question_id: "foreground".into(),
            kind: ComputerPendingKind::Foreground {
                app: app.map(Into::into),
            },
        }
    }

    #[test]
    fn a_foreground_request_asks_to_bring_the_app_forward_and_offers_no_stop() {
        let asking = ChatPresence {
            pending: Some(foreground_pending(Some("Ghostty"))),
            ..presence(PresenceTone::NeedsYou)
        };
        assert_eq!(
            strip_copy(&asking).text(),
            "Ghostty needs to come to the front to type · It will cover your screen; don't type while the agent works"
        );
        assert!(!shows_stop(&asking));
        assert!(foreground_request(&asking).is_some());
        assert!(foreground_request(&presence(PresenceTone::NeedsYou)).is_none());
        assert_eq!(
            strip_copy(&ChatPresence {
                app: None,
                ..asking
            })
            .lead
            .as_deref(),
            Some("The app needs to come to the front to type")
        );
    }

    #[test]
    fn strip_labels_never_carry_tool_ids_typed_text_or_pids() {
        use zeron_proto::computer::ComputerEvent;

        use crate::computer::presence::presences;
        use crate::computer::preview::ComputerPreviewStore;
        use crate::computer::preview::tests::{driving, thread_state};

        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        store.apply_event(&ComputerEvent::Action {
            window_id: Some("w1".into()),
            delivery: None,
            action: "computer_type_text".into(),
            ok: true,
            message: Some("typed hunter2-secret".into()),
            thread_id: Some("a".into()),
            restored_window_id: None,
            restore_status: None,
            masked: false,
        });
        let mut typing = thread_state("a", true);
        typing.activity = Some("Typing".into());
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(typing),
        });
        let mut texts = Vec::new();
        for (tone, user_takeover) in [
            (PresenceTone::Driving, false),
            (PresenceTone::Paused, false),
            (PresenceTone::Paused, true),
            (PresenceTone::NeedsYou, false),
            (PresenceTone::Revoked { summing_up: true }, false),
        ] {
            let mut presence = presences(&store).remove(0);
            presence.tone = tone;
            presence.user_takeover = user_takeover;
            texts.push(strip_copy(&presence).text());
        }
        let mut asking = presences(&store).remove(0);
        asking.tone = PresenceTone::NeedsYou;
        asking.pending = Some(foreground_pending(Some("Calculator")));
        texts.push(strip_copy(&asking).text());
        assert_eq!(texts[0], "Calculator · Typing");
        for text in texts {
            assert!(!text.contains("computer_"), "{text}");
            assert!(!text.contains("hunter2"), "{text}");
            assert!(!text.contains("w1"), "{text}");
        }
    }
}
