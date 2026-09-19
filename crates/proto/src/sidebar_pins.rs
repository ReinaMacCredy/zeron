//! Per-pin intents and dense, collision-free hexadecimal ordering keys.
use serde::{Deserialize, Serialize};

pub const MAX_PIN_ORDER_KEY_BYTES: usize = 8192;
const NONCE_BYTES: usize = 149; // Maximum wire HLC: 13 + 1 + 6 + 1 + 128.

pub fn valid_pin_order_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_PIN_ORDER_KEY_BYTES
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && !key.ends_with('0')
}

/// Allocate a whole prefix interval strictly between the bounds, then append
/// a fixed-width encoding of the unique operation HLC. Unlike random jitter,
/// different operation clocks cannot collide, even while writers are offline.
pub fn pin_order_key_between(
    lower: Option<&str>,
    upper: Option<&str>,
    nonce: &str,
) -> Result<String, &'static str> {
    if lower.is_some_and(|k| !valid_pin_order_key(k))
        || upper.is_some_and(|k| !valid_pin_order_key(k))
        || lower.zip(upper).is_some_and(|(a, b)| a >= b)
        || nonce.is_empty()
        || nonce.len() > NONCE_BYTES
        || !nonce.is_ascii()
        || nonce.as_bytes().contains(&0)
    {
        return Err("Invalid pin ordering bounds");
    }
    let lower = lower.unwrap_or("").as_bytes();
    let mut upper = upper.map(str::as_bytes);
    let mut key = String::new();
    let digit = |b: u8| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
    let hex = b"0123456789abcdef";
    for i in 0..MAX_PIN_ORDER_KEY_BYTES {
        let lo = lower.get(i).copied().map(digit).unwrap_or(0);
        let hi = upper
            .and_then(|u| u.get(i))
            .copied()
            .map(digit)
            .unwrap_or(16);
        if hi > lo + 1 {
            key.push(hex[((lo + hi) / 2) as usize] as char);
            for i in 0..NONCE_BYTES {
                let byte = nonce.as_bytes().get(i).copied().unwrap_or(0);
                key.push(hex[(byte >> 4) as usize] as char);
                key.push(hex[(byte & 15) as usize] as char);
            }
            key.push('8');
            return if key.len() <= MAX_PIN_ORDER_KEY_BYTES {
                Ok(key)
            } else {
                Err("Pin ordering key is too long")
            };
        }
        key.push(hex[lo as usize] as char);
        if lo != hi {
            upper = None;
        }
    }
    Err("Pin ordering key is too long")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum SidebarPinChange {
    #[serde(rename_all = "camelCase")]
    Pin {
        session_id: String,
        after: Option<String>,
        before: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Move {
        session_id: String,
        after: Option<String>,
        before: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Unpin { session_id: String },
}

impl SidebarPinChange {
    pub fn session_id(&self) -> &str {
        match self {
            Self::Pin { session_id, .. }
            | Self::Move { session_id, .. }
            | Self::Unpin { session_id } => session_id,
        }
    }
    /// Rebase a pending intent on the latest confirmed projection. A stale
    /// move never resurrects an unpinned item. A surviving right anchor wins;
    /// otherwise use the left anchor, or append when both disappeared.
    pub fn project(&self, ids: &mut Vec<String>) {
        let id = self.session_id();
        if matches!(self, Self::Move { .. }) && !ids.iter().any(|v| v == id) {
            return;
        }
        ids.retain(|v| v != id);
        let (after, before) = match self {
            Self::Unpin { .. } => return,
            Self::Pin { after, before, .. } | Self::Move { after, before, .. } => (after, before),
        };
        let index = before
            .as_ref()
            .and_then(|v| ids.iter().position(|i| i == v))
            .or_else(|| {
                after
                    .as_ref()
                    .and_then(|v| ids.iter().position(|i| i == v).map(|i| i + 1))
            })
            .unwrap_or(ids.len());
        ids.insert(index, id.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_keys_are_distinct_and_dense() {
        let a = pin_order_key_between(None, None, "0000000000001-000000-device-a").unwrap();
        let nonce = "0000000000001-000000-device-a";
        let encoded: String = nonce.bytes().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            a,
            format!("8{encoded}{}8", "0".repeat((149 - nonce.len()) * 2))
        );
        let b = pin_order_key_between(None, None, "0000000000001-000000-device-b").unwrap();
        assert!(a < b);
        let mid =
            pin_order_key_between(Some(&a), Some(&b), "0000000000002-000000-device-c").unwrap();
        assert!(a < mid && mid < b);
        let mut upper = a.clone();
        for i in 0..1000 {
            let key = pin_order_key_between(None, Some(&upper), &format!("{i:013}-000000-device"))
                .unwrap();
            assert!(key < upper && valid_pin_order_key(&key));
            upper = key;
        }
    }
    #[test]
    fn rejects_invalid_keys_and_bounds() {
        assert!(pin_order_key_between(Some("80"), None, "x").is_err());
        assert!(pin_order_key_between(Some("8"), Some("8"), "x").is_err());
    }
    #[test]
    fn pending_move_does_not_revive_unpinned_item() {
        let mut ids = vec!["remote".into()];
        SidebarPinChange::Move {
            session_id: "gone".into(),
            after: None,
            before: None,
        }
        .project(&mut ids);
        assert_eq!(ids, vec!["remote"]);
    }
}
