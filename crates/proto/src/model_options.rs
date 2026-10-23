use crate::{ChatConfig, HarnessId, ModelOption, ModelOptionChoice};

pub const CONTEXT_WINDOW: &str = "contextWindow";
pub const AUTO_COMPACT: &str = "autoCompact";

/// The CLI compacts this far below the chosen window (live-probed on 2.1.293).
pub const AUTO_COMPACT_HEADROOM: u64 = 33_000;

pub fn context_window_tokens(choice_id: &str) -> Option<u64> {
    match choice_id {
        "200k" => Some(200_000),
        "1m" => Some(1_000_000),
        _ => None,
    }
}

pub fn auto_compact_tokens(choice_id: &str) -> Option<u64> {
    match choice_id {
        "300k" => Some(300_000),
        "400k" => Some(400_000),
        "600k" => Some(600_000),
        "800k" => Some(800_000),
        _ => None,
    }
}

/// Where a chat with these picks compacts; a custom window applies on 1M only.
pub fn compact_threshold(window_choice: &str, compact_choice: &str) -> Option<u64> {
    let window = context_window_tokens(window_choice)?;
    let chosen = auto_compact_tokens(compact_choice)
        .filter(|_| window > 200_000)
        .map_or(window, |tokens| tokens.min(window));
    Some(chosen.saturating_sub(AUTO_COMPACT_HEADROOM))
}

/// The CLI reports the model's whole window, never the smaller one the chat's picks set.
pub fn chat_budget(config: &ChatConfig) -> Option<u64> {
    if config.harness != HarnessId::ClaudeCode {
        return None;
    }
    let pick = |id: &str| config.model_options.get(id).and_then(|v| v.as_str());
    let window = pick(CONTEXT_WINDOW).and_then(context_window_tokens);
    let compact = pick(AUTO_COMPACT)
        .and_then(auto_compact_tokens)
        .filter(|_| window != Some(200_000));
    compact.or(window)
}

pub fn compact_applies(window_choice: &str) -> bool {
    context_window_tokens(window_choice).is_some_and(|tokens| tokens > 200_000)
}

pub fn compact_tokens(tokens: u64) -> String {
    if tokens >= 999_500 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else {
        format!("{}K", (tokens + 500) / 1_000)
    }
}

pub fn context_window_warning(
    choice_id: &str,
    selected: bool,
    tokens: Option<u64>,
) -> Option<String> {
    let capacity = context_window_tokens(choice_id)?;
    let tokens = tokens?;
    let in_use = compact_tokens(tokens);
    (tokens > capacity).then(|| {
        if selected {
            format!("~{in_use} in use; the next message compacts it")
        } else {
            format!(
                "Switching to {} compacts this chat (~{in_use} in use)",
                compact_tokens(capacity)
            )
        }
    })
}

// Auto is the model's tuned 1M window, so it closes the track after the explicit sizes.
pub fn context_long_stops<T>(choices: &[T], id: impl Fn(&T) -> &str) -> Vec<&T> {
    let (auto, sized): (Vec<_>, Vec<_>) = choices.iter().partition(|c| id(c) == "auto");
    sized.into_iter().chain(auto).collect()
}

/// Bolt candidates: `(option, the choice that means fast)`.
pub const SPEED_TOGGLES: [(&str, &str); 2] = [("fastMode", "on"), ("serviceTier", "fast")];

/// The model's speed select as the bolt sees it: `(option, off, on)`. A richer
/// tier list (Flex, Ultra Fast) has no bolt shape.
pub fn speed_toggle_option(
    options: &[ModelOption],
) -> Option<(&ModelOption, &ModelOptionChoice, &ModelOptionChoice)> {
    SPEED_TOGGLES.into_iter().find_map(|(id, on_id)| {
        let option = options.iter().find(|o| o.id == id)?;
        if option.choices.len() != 2 {
            return None;
        }
        let on = option.choices.iter().find(|c| c.id == on_id)?;
        let off = option.choices.iter().find(|c| c.id != on_id)?;
        Some((option, off, on))
    })
}

/// Fast mode's `(on, off)` choices, whatever form a harness gives it: a
/// `fastMode`/`fast_mode` on/off toggle (Claude), Cursor's `fast` true/false,
/// or a tier/speed option offering `fast` (Codex, Devin). Off is the default
/// when fast isn't, else the other choice: Cursor runs some models fast by
/// default.
pub fn fast_mode_values(option: &ModelOption) -> Option<(&str, &str)> {
    let has = |id: &str| option.choices.iter().any(|choice| choice.id == id);
    let on = if matches!(option.id.as_str(), "fastMode" | "fast_mode") && has("on") {
        "on"
    } else if option.id == "fast" && has("true") {
        "true"
    } else if has("fast") {
        "fast"
    } else {
        return None;
    };
    let off = if option.default_choice != on && has(&option.default_choice) {
        option.default_choice.as_str()
    } else {
        option
            .choices
            .iter()
            .map(|choice| choice.id.as_str())
            .find(|id| *id != on)?
    };
    Some((on, off))
}

pub fn fast_mode_on(
    options: &[ModelOption],
    picks: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    options.iter().any(|option| {
        fast_mode_values(option).is_some_and(|(on, _)| selected_choice(option, picks) == on)
    })
}

/// Picking an option's default removes the key instead of storing it.
pub fn apply_pick(
    picks: &mut serde_json::Map<String, serde_json::Value>,
    option: &ModelOption,
    choice_id: &str,
) {
    if option.default_choice == choice_id {
        picks.remove(&option.id);
    } else {
        picks.insert(
            option.id.clone(),
            serde_json::Value::String(choice_id.to_owned()),
        );
    }
}

pub fn selected_choice<'a>(
    option: &'a ModelOption,
    picks: &'a serde_json::Map<String, serde_json::Value>,
) -> &'a str {
    picks
        .get(&option.id)
        .and_then(|v| v.as_str())
        .unwrap_or(&option.default_choice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_threshold_sits_33k_under_the_effective_window() {
        let at = |window, compact| compact_threshold(window, compact).map(compact_tokens);
        assert_eq!(at("1m", "auto").as_deref(), Some("967K"));
        assert_eq!(at("1m", "300k").as_deref(), Some("267K"));
        assert_eq!(at("1m", "400k").as_deref(), Some("367K"));
        assert_eq!(at("200k", "auto").as_deref(), Some("167K"));
        assert_eq!(at("200k", "800k").as_deref(), Some("167K"));
        assert_eq!(at("standard", "auto"), None);
    }

    #[test]
    fn the_200k_warning_fires_only_past_capacity() {
        assert_eq!(
            context_window_warning("200k", true, Some(230_412)).as_deref(),
            Some("~230K in use; the next message compacts it")
        );
        assert_eq!(
            context_window_warning("200k", false, Some(230_412)).as_deref(),
            Some("Switching to 200K compacts this chat (~230K in use)")
        );
        assert_eq!(context_window_warning("200k", false, Some(200_000)), None);
        assert_eq!(context_window_warning("1m", true, Some(230_412)), None);
        assert_eq!(context_window_warning("200k", false, None), None);
        assert_eq!(compact_tokens(200_001), "200K");
        assert_eq!(compact_tokens(1_240_000), "1.2M");
    }

    #[test]
    fn every_fast_mode_encoding_gets_the_same_controls() {
        let option = |id: &str, choices: &[&str], default: &str| ModelOption {
            id: id.into(),
            label: id.into(),
            choices: choices
                .iter()
                .map(|c| ModelOptionChoice {
                    id: (*c).into(),
                    label: (*c).into(),
                })
                .collect(),
            default_choice: default.into(),
        };
        // Codex tier, Claude toggle, ACP speed and the snake_case toggle.
        assert_eq!(
            fast_mode_values(&option("serviceTier", &["default", "fast"], "default")),
            Some(("fast", "default"))
        );
        assert_eq!(
            fast_mode_values(&option("fastMode", &["off", "on"], "off")),
            Some(("on", "off"))
        );
        assert_eq!(
            fast_mode_values(&option("speed", &["standard", "fast"], "standard")),
            Some(("fast", "standard"))
        );
        assert_eq!(
            fast_mode_values(&option("fast_mode", &["off", "on"], "off")),
            Some(("on", "off"))
        );
        // Cursor's true/false switch, off by default (Opus) and on by
        // default (Grok, Composer): both still toggle between the two.
        assert_eq!(
            fast_mode_values(&option("fast", &["false", "true"], "false")),
            Some(("true", "false"))
        );
        assert_eq!(
            fast_mode_values(&option("fast", &["false", "true"], "true")),
            Some(("true", "false"))
        );
        assert_eq!(
            fast_mode_values(&option("serviceTier", &["default", "fast"], "fast")),
            Some(("fast", "default"))
        );
        // Not fast mode: other toggles, including Cursor's true/false ones.
        assert_eq!(
            fast_mode_values(&option("thinking", &["off", "on"], "off")),
            None
        );
        assert_eq!(
            fast_mode_values(&option("thinking", &["false", "true"], "true")),
            None
        );
        assert_eq!(
            fast_mode_values(&option("contextWindow", &["200k", "1m"], "200k")),
            None
        );
    }
}
