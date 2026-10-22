use serde_json::{Map, Value};
use zeron_proto::model_options::{
    self, AUTO_COMPACT, CONTEXT_WINDOW, apply_pick, compact_applies, selected_choice,
};

use crate::catalog::ModelOption;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCard {
    pub long_context: bool,
    pub short_label: String,
    pub window_label: String,
    pub threshold_label: String,
    pub stops: Vec<String>,
    pub position: usize,
    pub warning: Option<String>,
}

fn context_options(options: &[ModelOption]) -> Option<(&ModelOption, &ModelOption)> {
    let find = |id: &str| options.iter().find(|o| o.id == id);
    Some((find(CONTEXT_WINDOW)?, find(AUTO_COMPACT)?))
}

pub fn context_card(
    options: &[ModelOption],
    picks: &Map<String, Value>,
    tokens: Option<u64>,
) -> Option<ContextCard> {
    let (window, compact) = context_options(options)?;
    let window_choice = selected_choice(window, picks);
    let compact_choice = selected_choice(compact, picks);
    let long_context = compact_applies(window_choice);
    let label = |id: &str| {
        window
            .choices
            .iter()
            .find(|c| c.id == id)
            .map_or_else(|| id.to_uppercase(), |c| c.label.clone())
    };
    let long = model_options::context_long_stops(&compact.choices, |c| &c.id);
    let position = if long_context {
        long.iter()
            .position(|c| c.id == compact_choice)
            .map_or(0, |ix| ix + 1)
    } else {
        0
    };
    let threshold = model_options::compact_threshold(window_choice, compact_choice)
        .map(model_options::compact_tokens)
        .unwrap_or_default();
    Some(ContextCard {
        long_context,
        short_label: label("200k"),
        window_label: format!("{} window", label(if long_context { "1m" } else { "200k" })),
        threshold_label: format!("compacts at ~{threshold}"),
        stops: long
            .iter()
            .map(|c| {
                if c.id == "auto" {
                    label("1m")
                } else {
                    c.label.clone()
                }
            })
            .collect(),
        position,
        warning: window.choices.iter().find_map(|c| {
            model_options::context_window_warning(&c.id, c.id == window_choice, tokens)
        }),
    })
}

pub fn pick_context(options: &[ModelOption], picks: &mut Map<String, Value>, position: usize) {
    let Some((window, compact)) = context_options(options) else {
        return;
    };
    let long = model_options::context_long_stops(&compact.choices, |c| &c.id);
    let offers = |id: &str| window.choices.iter().any(|c| c.id == id);
    match position.min(long.len()).checked_sub(1) {
        None => {
            if offers("200k") {
                apply_pick(picks, window, "200k");
            }
        }
        Some(stop) => {
            if offers("1m") {
                apply_pick(picks, window, "1m");
            }
            apply_pick(picks, compact, &long[stop].id);
        }
    }
}

pub fn toggle_long_context(options: &[ModelOption], picks: &mut Map<String, Value>) {
    let Some(card) = context_card(options, picks, None) else {
        return;
    };
    let position = if card.long_context {
        0
    } else {
        card.stops.len()
    };
    pick_context(options, picks, position);
}

pub fn speed_toggle(options: &[ModelOption], picks: &Map<String, Value>) -> Option<bool> {
    let (option, _, on) = model_options::speed_toggle_option(options)?;
    Some(selected_choice(option, picks) == on.id)
}

pub fn toggle_speed(options: &[ModelOption], picks: &mut Map<String, Value>) {
    let Some((option, off, on)) = model_options::speed_toggle_option(options) else {
        return;
    };
    let next = if selected_choice(option, picks) == on.id {
        off
    } else {
        on
    };
    apply_pick(picks, option, &next.id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ModelOptionChoice;

    fn option(id: &str, choices: &[(&str, &str)], default: &str) -> ModelOption {
        ModelOption {
            id: id.into(),
            label: id.into(),
            choices: choices
                .iter()
                .map(|(id, label)| ModelOptionChoice {
                    id: (*id).into(),
                    label: (*label).into(),
                })
                .collect(),
            default_choice: default.into(),
        }
    }

    fn claude() -> Vec<ModelOption> {
        vec![
            option(CONTEXT_WINDOW, &[("200k", "200K"), ("1m", "1M")], "1m"),
            option(
                AUTO_COMPACT,
                &[
                    ("auto", "Auto"),
                    ("300k", "300K"),
                    ("400k", "400K"),
                    ("600k", "600K"),
                    ("800k", "800K"),
                ],
                "auto",
            ),
            option("fastMode", &[("off", "Off"), ("on", "On")], "off"),
        ]
    }

    #[test]
    fn the_context_track_runs_200k_then_the_long_stops_with_auto_last() {
        let options = claude();
        let mut picks = Map::new();
        let card = context_card(&options, &picks, None).unwrap();
        assert!(card.long_context);
        assert_eq!(card.stops, ["300K", "400K", "600K", "800K", "1M"]);
        assert_eq!(card.position, 5);
        assert_eq!(card.window_label, "1M window");
        assert_eq!(card.threshold_label, "compacts at ~967K");

        pick_context(&options, &mut picks, 2);
        assert_eq!(
            picks.get(AUTO_COMPACT).and_then(Value::as_str),
            Some("400k")
        );
        assert!(!picks.contains_key(CONTEXT_WINDOW), "1M is the default");
        assert_eq!(
            context_card(&options, &picks, None)
                .unwrap()
                .threshold_label,
            "compacts at ~367K"
        );

        toggle_long_context(&options, &mut picks);
        let card = context_card(&options, &picks, Some(312_000)).unwrap();
        assert!(!card.long_context);
        assert_eq!(card.position, 0);
        assert_eq!(card.window_label, "200K window");
        assert_eq!(
            card.warning.as_deref(),
            Some("~312K in use; the next message compacts it")
        );

        toggle_long_context(&options, &mut picks);
        assert!(!picks.contains_key(CONTEXT_WINDOW));
        assert!(!picks.contains_key(AUTO_COMPACT), "auto is the default");
    }

    #[test]
    fn fast_toggles_and_falls_back_to_no_key_at_default() {
        let options = claude();
        let mut picks = Map::new();
        assert_eq!(speed_toggle(&options, &picks), Some(false));
        toggle_speed(&options, &mut picks);
        assert_eq!(speed_toggle(&options, &picks), Some(true));
        assert!(model_options::fast_mode_on(&options, &picks));
        toggle_speed(&options, &mut picks);
        assert!(picks.is_empty());
        assert_eq!(speed_toggle(&options[..2], &picks), None);
    }
}
