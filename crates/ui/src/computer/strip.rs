use gpui::{
    AnyElement, App, ClickEvent, FontWeight, Hsla, SharedString, Window, div, prelude::*, px,
};
use zeron_engine::computer::engine_seams::{
    COMPUTER_FOREGROUND_ALLOW, COMPUTER_FOREGROUND_DECLINE,
};
use zeron_proto::UserInputAnswer;
use zeron_proto::computer::{
    ComputerConsentApp, ComputerConsentScope, ComputerPendingKind, ComputerPendingRequest,
};

use super::cards::{
    ConsentChoice, consent_action_sentence, consent_answers, strip_consent_choices,
};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentAsk {
    pub input_id: String,
    pub question_id: String,
    pub scope: ComputerConsentScope,
    pub app: Option<ComputerConsentApp>,
    pub sentence: String,
}

pub fn consent_request(presence: &ChatPresence) -> Option<ConsentAsk> {
    let pending = presence.pending.as_ref()?;
    let ComputerPendingKind::Consent {
        scope,
        app,
        tool,
        params,
    } = &pending.kind
    else {
        return None;
    };
    let sentence = match tool {
        Some(tool) => consent_action_sentence(tool, params.as_deref(), app.as_ref()),
        None => match app {
            Some(app) => format!("Use {}", app.name),
            None => "Use this Mac".into(),
        },
    };
    Some(ConsentAsk {
        input_id: pending.input_id.clone(),
        question_id: pending.question_id.clone(),
        scope: *scope,
        app: app.clone(),
        sentence,
    })
}

pub fn consent_buttons(ask: &ConsentAsk) -> Vec<(ConsentChoice, String, Vec<UserInputAnswer>)> {
    strip_consent_choices(ask.scope, ask.app.as_ref())
        .into_iter()
        .map(|(choice, label)| {
            let answers = consent_answers(&ask.question_id, &choice);
            (choice, label, answers)
        })
        .collect()
}

pub fn strip_copy(presence: &ChatPresence) -> StripCopy {
    if let Some(ask) = consent_request(presence) {
        return StripCopy {
            lead: Some(
                ask.app
                    .map_or_else(|| "This Mac".to_owned(), |app| app.name),
            ),
            detail: ask.sentence,
        };
    }
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

pub type OnAnswer = Box<dyn Fn(String, Vec<UserInputAnswer>, &mut Window, &mut App)>;

fn strip_button(
    id: &'static str,
    label: SharedString,
    primary: bool,
    input_id: &str,
    answers: Vec<UserInputAnswer>,
    on_answer: &std::rc::Rc<OnAnswer>,
    theme: &Theme,
) -> AnyElement {
    let on_answer = on_answer.clone();
    let input_id = input_id.to_owned();
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
            on_answer(input_id.clone(), answers.clone(), window, cx)
        })
        .child(label)
        .into_any_element()
}

fn foreground_answers(question_id: &str, label: &str) -> Vec<UserInputAnswer> {
    vec![UserInputAnswer {
        question_id: question_id.to_owned(),
        labels: vec![label.to_owned()],
    }]
}

fn answer_buttons(
    presence: &ChatPresence,
    on_answer: &std::rc::Rc<OnAnswer>,
    theme: &Theme,
) -> Vec<AnyElement> {
    if let Some(ask) = consent_request(presence) {
        return consent_buttons(&ask)
            .into_iter()
            .map(|(choice, label, answers)| {
                let (id, primary) = match choice {
                    ConsentChoice::Allow => ("computer-strip-allow-consent", true),
                    ConsentChoice::AlwaysAllow(_) => ("computer-strip-always-consent", false),
                    ConsentChoice::Decline => ("computer-strip-decline-consent", false),
                };
                strip_button(
                    id,
                    label.into(),
                    primary,
                    &ask.input_id,
                    answers,
                    on_answer,
                    theme,
                )
            })
            .collect();
    }
    let Some(pending) = foreground_request(presence) else {
        return Vec::new();
    };
    [
        (
            "computer-strip-allow-foreground",
            COMPUTER_FOREGROUND_ALLOW,
            true,
        ),
        (
            "computer-strip-decline-foreground",
            COMPUTER_FOREGROUND_DECLINE,
            false,
        ),
    ]
    .into_iter()
    .map(|(id, label, primary)| {
        strip_button(
            id,
            label.into(),
            primary,
            &pending.input_id,
            foreground_answers(&pending.question_id, label),
            on_answer,
            theme,
        )
    })
    .collect()
}

fn strip_glyph(presence: &ChatPresence, color: Hsla) -> AnyElement {
    let icon = consent_request(presence)
        .and_then(|ask| ask.app?.bundle_id)
        .and_then(|bundle| crate::appshots::bundle_icon(&bundle));
    match icon {
        Some(icon) => gpui::img(icon)
            .flex_none()
            .size(px(16.0))
            .into_any_element(),
        None => crate::icons::icon(crate::icons::CURSOR)
            .flex_none()
            .size(px(14.0))
            .text_color(color)
            .into_any_element(),
    }
}

pub fn computer_strip(
    presence: &ChatPresence,
    show_esc: bool,
    on_stop: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
    on_answer: OnAnswer,
    theme: &Theme,
) -> AnyElement {
    let copy = strip_copy(presence);
    let tint = tone_color(presence.tone, theme);
    let on_answer = std::rc::Rc::new(on_answer);
    let buttons = answer_buttons(presence, &on_answer, theme);
    let asking = !buttons.is_empty();
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
        .child(strip_glyph(presence, glyph))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_row()
                .items_center()
                .map(|row| {
                    if asking {
                        row.flex_wrap().gap_x(px(6.0))
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
        .children(buttons)
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

    pub(crate) fn consent_pending(
        scope: ComputerConsentScope,
        app: Option<&str>,
        verified: bool,
    ) -> ComputerPendingRequest {
        ComputerPendingRequest {
            input_id: "computer-consent-1".into(),
            question_id: "consent".into(),
            kind: ComputerPendingKind::Consent {
                scope,
                app: app.map(|name| ComputerConsentApp {
                    name: name.into(),
                    bundle_id: verified.then(|| "com.mitchellh.ghostty".into()),
                    pid: Some(4242),
                }),
                tool: Some("computer_type_text".into()),
                params: Some(r#"{"window_id":"cua:4242:17","text":"hunter2"}"#.into()),
            },
        }
    }

    #[test]
    fn a_consent_request_names_the_app_and_its_first_action_and_offers_no_stop() {
        let asking = ChatPresence {
            pending: Some(consent_pending(
                ComputerConsentScope::Task,
                Some("Ghostty"),
                true,
            )),
            ..presence(PresenceTone::NeedsYou)
        };
        assert_eq!(strip_copy(&asking).text(), "Ghostty · Type in Ghostty");
        assert!(!shows_stop(&asking));
        assert!(foreground_request(&asking).is_none());
        let ask = consent_request(&asking).expect("a consent ask");
        assert_eq!(ask.input_id, "computer-consent-1");
        let buttons = consent_buttons(&ask);
        assert_eq!(
            buttons
                .iter()
                .map(|(_, label, answers)| (label.as_str(), answers[0].labels[0].as_str()))
                .collect::<Vec<_>>(),
            [
                ("Allow for this task", "Allow"),
                ("Always allow Ghostty", "Always allow Ghostty"),
                ("No", "Deny"),
            ]
        );
        assert!(
            buttons
                .iter()
                .all(|(_, _, answers)| answers[0].question_id == "consent")
        );
        let unverified = ChatPresence {
            pending: Some(consent_pending(
                ComputerConsentScope::Task,
                Some("Ghostty"),
                false,
            )),
            ..asking.clone()
        };
        assert_eq!(
            consent_buttons(&consent_request(&unverified).unwrap()).len(),
            2,
            "no Always for an unverified app"
        );
        let bare = ChatPresence {
            pending: Some(consent_pending(ComputerConsentScope::Call, None, false)),
            app: None,
            ..asking
        };
        assert_eq!(strip_copy(&bare).text(), "This Mac · Type");
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
        asking.pending = Some(consent_pending(
            ComputerConsentScope::Task,
            Some("Calculator"),
            true,
        ));
        texts.push(strip_copy(&asking).text());
        texts.extend(
            consent_buttons(&consent_request(&asking).unwrap())
                .into_iter()
                .map(|(_, label, _)| label),
        );
        assert_eq!(texts[0], "Calculator · Typing");
        for text in texts {
            for leak in [
                "computer_",
                "hunter2",
                "w1",
                "4242",
                "cua:",
                "com.mitchellh",
            ] {
                assert!(!text.contains(leak), "{text} carries {leak}");
            }
        }
    }
}
