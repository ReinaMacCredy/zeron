use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};
use zeron_proto::computer::normalize_cursor_hex_color;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CuaCursorStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rim: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<String>,
}

fn channel(value: Option<&str>) -> Option<String> {
    Some(normalize_cursor_hex_color(value?)).filter(|color| !color.is_empty())
}

pub fn normalize_cua_cursor_style(style: Option<&CuaCursorStyle>) -> Option<CuaCursorStyle> {
    let style = style?;
    let normalized = CuaCursorStyle {
        fill: channel(style.fill.as_deref()),
        rim: channel(style.rim.as_deref()),
        shadow: channel(style.shadow.as_deref()),
    };
    (normalized != CuaCursorStyle::default()).then_some(normalized)
}

pub fn normalize_agent_cursor_style_preference(value: &Value) -> Option<CuaCursorStyle> {
    let candidate = value.as_object()?;
    let field = |key: &str| {
        candidate
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    normalize_cua_cursor_style(Some(&CuaCursorStyle {
        fill: field("fill"),
        rim: field("rim"),
        shadow: field("shadow"),
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedAgentCursorPreference {
    pub style: Option<CuaCursorStyle>,
}

pub fn parse_agent_cursor_preference(value: &Value) -> Option<PersistedAgentCursorPreference> {
    let candidate = value.as_object()?;
    if candidate.get("version").and_then(Value::as_f64) != Some(1.0) {
        return None;
    }
    let style = candidate.get("style")?;
    Some(PersistedAgentCursorPreference {
        style: normalize_agent_cursor_style_preference(style),
    })
}

pub fn read_agent_cursor_preference(path: &Path) -> Option<CuaCursorStyle> {
    let bytes = std::fs::read(path).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    parse_agent_cursor_preference(&value)?.style
}

pub fn write_agent_cursor_preference(
    path: &Path,
    style: Option<&CuaCursorStyle>,
) -> std::io::Result<()> {
    let Some(normalized) = normalize_cua_cursor_style(style) else {
        return match std::fs::remove_file(path) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        };
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let payload = json!({ "version": 1, "style": normalized });
    let text = serde_json::to_string_pretty(&payload).map_err(std::io::Error::other)?;
    std::fs::write(path, format!("{text}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(fill: Option<&str>, rim: Option<&str>) -> CuaCursorStyle {
        CuaCursorStyle {
            fill: fill.map(Into::into),
            rim: rim.map(Into::into),
            shadow: None,
        }
    }

    #[test]
    fn keeps_usable_channels_lowercased_and_drops_everything_else() {
        assert_eq!(
            normalize_agent_cursor_style_preference(
                &json!({ "fill": " #AABBCC ", "rim": "#0f0f0f", "shadow": "red" })
            ),
            Some(style(Some("#aabbcc"), Some("#0f0f0f")))
        );
        for value in [
            json!({ "fill": "#12345" }),
            json!({ "fill": 12 }),
            json!({}),
            Value::Null,
            json!("not-a-style"),
            json!(["#aabbcc"]),
        ] {
            assert_eq!(
                normalize_agent_cursor_style_preference(&value),
                None,
                "{value}"
            );
        }
    }

    #[test]
    fn accepts_only_the_versioned_shape_and_normalizes_the_style_through_it() {
        assert_eq!(
            parse_agent_cursor_preference(&json!({ "version": 1, "style": { "fill": "#AABBCC" } })),
            Some(PersistedAgentCursorPreference {
                style: Some(style(Some("#aabbcc"), None))
            })
        );
        assert_eq!(
            parse_agent_cursor_preference(&json!({ "version": 1, "style": { "fill": "bad" } })),
            Some(PersistedAgentCursorPreference { style: None })
        );
        assert_eq!(
            parse_agent_cursor_preference(&json!({ "version": 2, "style": { "fill": "#aabbcc" } })),
            None
        );
        assert_eq!(
            parse_agent_cursor_preference(&json!({ "style": { "fill": "#aabbcc" } })),
            None
        );
        assert_eq!(parse_agent_cursor_preference(&Value::Null), None);
    }

    #[test]
    fn serializes_only_the_channels_that_carry_a_color() {
        let normalized = normalize_cua_cursor_style(Some(&CuaCursorStyle {
            fill: Some("#ABCDEF".into()),
            rim: Some("not-a-color".into()),
            shadow: None,
        }));
        assert_eq!(
            serde_json::to_value(normalized).unwrap(),
            json!({ "fill": "#abcdef" })
        );
        assert_eq!(normalize_cua_cursor_style(None), None);
    }

    #[test]
    fn round_trips_custom_colors_and_restores_stock_by_deleting_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("agent-cursor-colors.json");
        assert_eq!(read_agent_cursor_preference(&path), None);
        write_agent_cursor_preference(&path, Some(&style(Some("#AABBCC"), Some("#112233"))))
            .unwrap();
        assert_eq!(
            read_agent_cursor_preference(&path),
            Some(style(Some("#aabbcc"), Some("#112233")))
        );
        let stored: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            stored,
            json!({ "version": 1, "style": { "fill": "#aabbcc", "rim": "#112233" } })
        );
        write_agent_cursor_preference(&path, None).unwrap();
        assert!(!path.exists());
        assert_eq!(read_agent_cursor_preference(&path), None);
    }

    #[test]
    fn reads_stock_from_malformed_or_arbitrary_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-cursor-colors.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(read_agent_cursor_preference(&path), None);
        std::fs::write(
            &path,
            json!({ "version": 1, "style": { "rim": "purple" } }).to_string(),
        )
        .unwrap();
        assert_eq!(read_agent_cursor_preference(&path), None);
    }
}
