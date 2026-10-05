use zeron_proto::ToolCall;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceTool {
    List,
    Open,
    Screenshot,
    Close,
}

const DEVICE_TOOLS: [(&str, DeviceTool, [&str; 4]); 4] = [
    (
        "device_list",
        DeviceTool::List,
        ["List", "Listing", "Listed", "simulators and emulators"],
    ),
    (
        "device_open",
        DeviceTool::Open,
        ["Open", "Opening", "Opened", "a device in the Device panel"],
    ),
    (
        "device_screenshot",
        DeviceTool::Screenshot,
        [
            "Take a screenshot of",
            "Taking a screenshot of",
            "Took a screenshot of",
            "the device",
        ],
    ),
    (
        "device_close",
        DeviceTool::Close,
        ["Close", "Closing", "Closed", "a device"],
    ),
];

pub fn device_call(call: &ToolCall) -> Option<DeviceTool> {
    let found = |matches: &dyn Fn(&str) -> bool| {
        DEVICE_TOOLS
            .iter()
            .find(|(name, _, _)| matches(name))
            .map(|(_, tool, _)| *tool)
    };
    match call {
        ToolCall::Mcp { server, tool, .. } if server == "zeron" => found(&|name| tool == name),
        ToolCall::Unknown { name: raw, .. } => {
            let normalized = raw
                .trim()
                .to_lowercase()
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_");
            found(&|name| {
                normalized
                    .strip_suffix(name)
                    .map(|head| head.trim_end_matches('_'))
                    .is_some_and(|head| head == "zeron" || head.ends_with("_zeron"))
            })
        }
        _ => None,
    }
}

pub fn device_call_label(tool: DeviceTool, resolved: bool, failed: bool) -> String {
    let [action, running, done, object] = DEVICE_TOOLS
        .iter()
        .find(|(_, known, _)| *known == tool)
        .map(|(_, _, labels)| *labels)
        .unwrap_or_default();
    let verb = match (failed, resolved) {
        (true, _) => action,
        (false, true) => done,
        (false, false) => running,
    };
    format!("{verb} {object}")
}

pub fn device_call_detail(tool: DeviceTool, output: &str) -> Option<String> {
    if tool != DeviceTool::Screenshot {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(output.get(output.find('{')?..)?).ok()?;
    let screenshot = &value["screenshot"];
    Some(format!(
        "{} · {}x{} PNG",
        value["device"]["name"].as_str()?,
        screenshot["width"].as_u64()?,
        screenshot["height"].as_u64()?
    ))
}

pub fn device_group_label(count: usize) -> String {
    let times = if count == 1 { "time" } else { "times" };
    format!("used device controls {count} {times}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_calls_read_like_t3code_rows() {
        let open = ToolCall::Mcp {
            server: "zeron".into(),
            tool: "device_open".into(),
            input: None,
        };
        assert_eq!(device_call(&open), Some(DeviceTool::Open));
        assert_eq!(
            device_call_label(DeviceTool::Open, true, false),
            "Opened a device in the Device panel"
        );
        assert_eq!(
            device_call_label(DeviceTool::Open, false, false),
            "Opening a device in the Device panel"
        );
        assert_eq!(
            device_call_label(DeviceTool::Open, true, true),
            "Open a device in the Device panel"
        );
        assert_eq!(
            device_call_label(DeviceTool::Screenshot, true, false),
            "Took a screenshot of the device"
        );
        assert_eq!(
            device_call_label(DeviceTool::List, false, false),
            "Listing simulators and emulators"
        );
        assert_eq!(
            device_call_label(DeviceTool::Close, true, false),
            "Closed a device"
        );
        let unknown = |name: &str| ToolCall::Unknown {
            name: name.into(),
            input: None,
        };
        assert_eq!(
            device_call(&unknown("mcp__zeron__device_screenshot")),
            Some(DeviceTool::Screenshot)
        );
        assert_eq!(device_call(&unknown("device_close")), None);
        assert_eq!(device_call(&unknown("mcp__other__device_close")), None);
        let other_server = ToolCall::Mcp {
            server: "other".into(),
            tool: "device_open".into(),
            input: None,
        };
        assert_eq!(device_call(&other_server), None);
        assert_eq!(
            device_call_detail(
                DeviceTool::Screenshot,
                r#"{"device":{"name":"iPhone 17"},"screenshot":{"mimeType":"image/png","width":1206,"height":2622}}"#
            )
            .as_deref(),
            Some("iPhone 17 · 1206x2622 PNG")
        );
        assert_eq!(device_call_detail(DeviceTool::Open, "{}"), None);
        assert_eq!(device_group_label(1), "used device controls 1 time");
        assert_eq!(device_group_label(3), "used device controls 3 times");
    }
}
