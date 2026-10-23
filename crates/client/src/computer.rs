use serde_json::Value;
use zeron_proto::computer::ComputerStillFrame;

const COMPUTER_USE_COMMAND: &str = "/computer-use";

pub fn is_computer_invocation(text: &str) -> bool {
    let lead = text.len() - text.trim_start_matches(' ').len();
    if lead > 3 {
        return false;
    }
    let rest = &text[lead..];
    let Some(head) = rest.get(..COMPUTER_USE_COMMAND.len()) else {
        return false;
    };
    let after = &rest[COMPUTER_USE_COMMAND.len()..];
    head.eq_ignore_ascii_case(COMPUTER_USE_COMMAND)
        && (after.is_empty() || after.starts_with([' ', '\t', '\r', '\n']))
}

pub fn enabling_computer_use(draft: &str) -> String {
    if is_computer_invocation(draft) {
        draft.to_owned()
    } else if draft.trim().is_empty() {
        format!("{COMPUTER_USE_COMMAND} ")
    } else {
        format!("{COMPUTER_USE_COMMAND} {draft}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerStill {
    pub sequence: u64,
    pub image: Vec<u8>,
}

pub fn parse_still_reply(reply: &Value) -> Option<ComputerStill> {
    let frame: ComputerStillFrame = serde_json::from_value(reply.get("frame")?.clone()).ok()?;
    let image = crate::live::b64::decode(&frame.data)?;
    (!image.is_empty()).then_some(ComputerStill {
        sequence: frame.sequence,
        image,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_prefixes_the_draft_once_like_the_mac() {
        for (draft, expected) in [
            ("", "/computer-use "),
            ("  \n", "/computer-use "),
            ("open Calculator", "/computer-use open Calculator"),
            (
                "/computer-use open Calculator",
                "/computer-use open Calculator",
            ),
            ("/computer-use ", "/computer-use "),
            ("/COMPUTER-USE x", "/COMPUTER-USE x"),
            ("   /computer-use", "   /computer-use"),
            ("/computer-use\nline two", "/computer-use\nline two"),
            ("    /computer-use", "/computer-use     /computer-use"),
            ("/computer-used", "/computer-use /computer-used"),
            ("é", "/computer-use é"),
        ] {
            assert_eq!(enabling_computer_use(draft), expected, "{draft:?}");
        }
    }

    #[test]
    fn a_still_reply_decodes_its_image() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let still = parse_still_reply(&serde_json::json!({
            "frame": { "sequence": 7, "timestampMs": 12.5, "keyframe": true, "data": png }
        }))
        .expect("frame");
        assert_eq!(still.sequence, 7);
        assert_eq!(&still.image[1..4], b"PNG");
        assert_eq!(
            parse_still_reply(&serde_json::json!({ "frame": null })),
            None
        );
        assert_eq!(parse_still_reply(&serde_json::json!({})), None);
    }
}
