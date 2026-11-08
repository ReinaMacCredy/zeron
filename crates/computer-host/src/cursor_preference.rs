use serde::Serialize;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
