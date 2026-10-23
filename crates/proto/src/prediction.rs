use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposerPrediction {
    pub entry_id: String,
    pub text: String,
}

/// An empty draft shows the whole prediction in place of the placeholder; a
/// draft it extends shows the rest as ghost text.
pub fn prediction_display<'a>(
    prediction: Option<&'a str>,
    text: &str,
) -> (Option<&'a str>, Option<&'a str>) {
    match prediction {
        Some(prediction) if text.is_empty() => (Some(prediction), None),
        Some(prediction) => (
            None,
            prediction
                .strip_prefix(text)
                .filter(|rest| !rest.is_empty()),
        ),
        None => (None, None),
    }
}

pub fn accepts_prediction(prediction: &str, text: &str) -> bool {
    prediction.starts_with(text) && prediction.len() != text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prediction_display_follows_the_draft() {
        let fix = Some("Fix the spacing.");
        assert_eq!(
            prediction_display(fix, ""),
            (Some("Fix the spacing."), None)
        );
        assert_eq!(
            prediction_display(fix, "Fix"),
            (None, Some(" the spacing."))
        );
        assert_eq!(prediction_display(fix, "Fix the spacing."), (None, None));
        assert_eq!(prediction_display(fix, "Ship"), (None, None));
        assert_eq!(prediction_display(None, ""), (None, None));
    }

    #[test]
    fn only_a_draft_the_prediction_extends_accepts_it() {
        assert!(accepts_prediction("Fix the spacing.", ""));
        assert!(accepts_prediction("Fix the spacing.", "Fix"));
        assert!(!accepts_prediction("Fix the spacing.", "Fix the spacing."));
        assert!(!accepts_prediction("Fix the spacing.", "Ship"));
    }
}
