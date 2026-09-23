use std::fmt;

use serde_json::{Map, Value};

pub type ToolArgs = Map<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInputError(pub String);

impl ToolInputError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ToolInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ToolInputError {}

fn present<'a>(args: &'a ToolArgs, name: &str) -> Option<&'a Value> {
    args.get(name).filter(|value| !value.is_null())
}

pub fn read_string_arg(
    args: &ToolArgs,
    name: &str,
    required: bool,
) -> Result<Option<String>, ToolInputError> {
    let Some(value) = present(args, name) else {
        if required {
            return Err(ToolInputError::new(format!(
                "Missing required argument \"{name}\"."
            )));
        }
        return Ok(None);
    };
    match value.as_str() {
        Some(text) if !text.trim().is_empty() => Ok(Some(text.trim().to_owned())),
        _ => Err(ToolInputError::new(format!(
            "Argument \"{name}\" must be a non-empty string."
        ))),
    }
}

pub fn read_required_string_arg(args: &ToolArgs, name: &str) -> Result<String, ToolInputError> {
    read_string_arg(args, name, true).map(Option::unwrap_or_default)
}

pub fn read_verbatim_string_arg(
    args: &ToolArgs,
    name: &str,
) -> Result<Option<String>, ToolInputError> {
    let Some(value) = present(args, name) else {
        return Ok(None);
    };
    match value.as_str() {
        Some(text) if !text.trim().is_empty() => Ok(Some(text.to_owned())),
        _ => Err(ToolInputError::new(format!(
            "Argument \"{name}\" must be a non-empty string."
        ))),
    }
}

pub fn read_number_arg(args: &ToolArgs, name: &str) -> Result<Option<f64>, ToolInputError> {
    let Some(value) = present(args, name) else {
        return Ok(None);
    };
    match value.as_f64() {
        Some(number) if number.is_finite() => Ok(Some(number)),
        _ => Err(ToolInputError::new(format!(
            "Argument \"{name}\" must be a number."
        ))),
    }
}

pub fn read_boolean_arg(args: &ToolArgs, name: &str) -> Result<Option<bool>, ToolInputError> {
    let Some(value) = present(args, name) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| ToolInputError::new(format!("Argument \"{name}\" must be a boolean.")))
}

pub fn read_record_arg<'a>(
    args: &'a ToolArgs,
    name: &str,
) -> Result<Option<&'a ToolArgs>, ToolInputError> {
    let Some(value) = present(args, name) else {
        return Ok(None);
    };
    value
        .as_object()
        .map(Some)
        .ok_or_else(|| ToolInputError::new(format!("Argument \"{name}\" must be an object.")))
}

pub fn read_string_array_arg(
    args: &ToolArgs,
    name: &str,
) -> Result<Option<Vec<String>>, ToolInputError> {
    let Some(value) = present(args, name) else {
        return Ok(None);
    };
    let invalid = || {
        ToolInputError::new(format!(
            "Argument \"{name}\" must be an array of non-empty strings."
        ))
    };
    let entries = value.as_array().ok_or_else(invalid)?;
    entries
        .iter()
        .map(|entry| match entry.as_str() {
            Some(text) if !text.trim().is_empty() => Ok(text.trim().to_owned()),
            _ => Err(invalid()),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

pub fn is_safe_integer(value: f64) -> bool {
    value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn args(value: Value) -> ToolArgs {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn strings_trim_refuse_blanks_and_treat_null_as_absent() {
        let input = args(json!({ "a": "  x  ", "b": "   ", "c": null, "d": 3 }));
        assert_eq!(
            read_string_arg(&input, "a", false).unwrap().as_deref(),
            Some("x")
        );
        assert!(read_string_arg(&input, "b", false).is_err());
        assert_eq!(read_string_arg(&input, "c", false).unwrap(), None);
        assert_eq!(
            read_string_arg(&input, "c", true).unwrap_err().0,
            "Missing required argument \"c\"."
        );
        assert!(read_string_arg(&input, "d", false).is_err());
        assert_eq!(
            read_verbatim_string_arg(&input, "a").unwrap().as_deref(),
            Some("  x  ")
        );
        assert!(read_verbatim_string_arg(&input, "b").is_err());
    }

    #[test]
    fn numbers_booleans_records_and_arrays_keep_their_types() {
        let input = args(json!({
            "n": 1.5, "s": "1", "b": true, "o": { "k": 1 }, "a": [" x ", "y"], "bad": ["x", ""]
        }));
        assert_eq!(read_number_arg(&input, "n").unwrap(), Some(1.5));
        assert!(read_number_arg(&input, "s").is_err());
        assert_eq!(read_boolean_arg(&input, "b").unwrap(), Some(true));
        assert!(read_boolean_arg(&input, "n").is_err());
        assert!(read_record_arg(&input, "o").unwrap().is_some());
        assert!(read_record_arg(&input, "a").is_err());
        assert_eq!(
            read_string_array_arg(&input, "a").unwrap(),
            Some(vec!["x".to_owned(), "y".to_owned()])
        );
        assert!(read_string_array_arg(&input, "bad").is_err());
    }
}
