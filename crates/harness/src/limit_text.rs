use zeron_proto::{AgentEvent, DoneStatus};

const PHRASES: [&str; 8] = [
    "rate limit",
    "resource exhausted",
    "usage limit",
    "quota exceeded",
    "quota exhausted",
    "exceeded your quota",
    "out of credits",
    "insufficient credits",
];
const LOCAL_QUOTAS: [&str; 2] = ["disk quota", "storage quota"];
const THROTTLES: [&str; 3] = ["too many requests", "per minute", "slow down"];
const PLAN_WORDS: [&str; 5] = ["usage limit", "quota", "credit", "exhausted", "plan"];
const THROTTLE_MS: i64 = 60_000;

pub fn is_usage_limit(message: &str) -> bool {
    let lower = message.to_lowercase();
    let spaced = lower.replace(['_', '-'], " ");
    has_status_429(&lower)
        || (PHRASES.iter().any(|phrase| spaced.contains(phrase))
            && !LOCAL_QUOTAS.iter().any(|phrase| spaced.contains(phrase)))
}

/// A per-minute throttle passes in about a minute, not the hour an unknown plan limit is held for.
fn throttled(message: &str) -> bool {
    let lower = message.to_lowercase().replace(['_', '-'], " ");
    (has_status_429(&lower) || THROTTLES.iter().any(|phrase| lower.contains(phrase)))
        && !PLAN_WORDS.iter().any(|word| lower.contains(word))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

pub fn done_events(
    status: DoneStatus,
    error: Option<String>,
    session_id: Option<String>,
) -> Vec<AgentEvent> {
    let limit = error
        .as_deref()
        .filter(|error| status == DoneStatus::Errored && is_usage_limit(error))
        .map(|error| AgentEvent::UsageLimit {
            resets_at: throttled(error).then(|| now_ms() + THROTTLE_MS),
        });
    limit
        .into_iter()
        .chain([AgentEvent::Done {
            status,
            result: None,
            error,
            session_id,
        }])
        .collect()
}

fn has_status_429(text: &str) -> bool {
    let bytes = text.as_bytes();
    let joins = |next: Option<&u8>, beyond: Option<&u8>| {
        next.is_some_and(|&b| {
            b.is_ascii_alphanumeric()
                || b == b'_'
                || b == b'-'
                || (b == b'.' && beyond.is_some_and(u8::is_ascii_digit))
        })
    };
    text.match_indices("429").any(|(at, _)| {
        let before = |back: usize| at.checked_sub(back).and_then(|ix| bytes.get(ix));
        !joins(before(1), before(2)) && !joins(bytes.get(at + 3), bytes.get(at + 4))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_text_classifies_provider_limits() {
        for text in [
            "You've hit your usage limit Get Cursor Pro for more Agent usage, unlimited Tab, and more. [code=PRO_USER_USAGE_LIMIT, requestId=4f1c]",
            "User Provided API Key Rate Limit Exceeded. User API Key Rate limit exceeded — connect Cursor in Settings → Accounts (its login is separate from `cursor-agent login`), or set CURSOR_API_KEY from cursor.com/settings.",
            "You have exceeded the rate limit of 30 requests per minute for the get_models endpoint",
            "Rate limited during API key exchange. [code=resource_exhausted]",
            "Permission denied: Permission denied: all API providers are over their global rate limit for trial users (trace ID: a6e9a7a3e2e958d43ac8e194787a79b8)",
            "harness protocol error: session/prompt: Quota exhausted.",
            "You've hit the rate limit for your plan. Upgrade your account or try again later.",
            "You’ve reached your free Grok Build usage limit for now. Get SuperGrok for much higher limits, or try again later.",
            "out of credits or over your spending limit. Add credits and retry.",
            "You’ve hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at Oct 5th, 2026 5:08 AM.",
            "Error 429, Message: Resource has been exhausted (e.g. check quota)., Status: RESOURCE_EXHAUSTED",
            "API call failed after 3 retries: HTTP 429: slow down",
            "harness protocol error: session/prompt: 429 Too Many Requests",
            "request failed with status 429.",
        ] {
            assert!(is_usage_limit(text), "{text}");
        }
    }

    #[test]
    fn limit_text_leaves_auth_and_tool_errors_alone() {
        for text in [
            "Invalid API key. Unauthorized User API key — connect Cursor in Settings → Accounts (its login is separate from `cursor-agent login`), or set CURSOR_API_KEY from cursor.com/settings.",
            "Failed to authenticate: OAuth session expired and could not be refreshed",
            "Not logged in. Please run 'chisel auth' first.",
            "no xAI credential available; run `grok login` or set XAI_API_KEY",
            "harness protocol error: session/prompt: 401 Unauthorized",
            "You have an unpaid invoice Visit [cursor.com/dashboard](https://cursor.com/dashboard) and pay your invoice in Stripe to resume requests.",
            "Named models unavailable Free plans can only use Auto. Switch to Auto or upgrade plans to continue.",
            "[internal] Failed to run step, exceeded max retries",
            "API Error: 529 Overloaded. This is a server-side issue, usually temporary — try again in a moment",
            "API call failed after 3 retries. Connection error. | provider=custom model=cx/gpt-5.5",
            "Error: listen EADDRINUSE: address already in use 127.0.0.1:4290",
            "request abc-429-def failed [code=internal, requestId=req_429a]",
            "build 1.429.0 crashed after 0.429s",
            "The agent failed to complete the turn.",
            "Error: EDQUOT: Disk quota exceeded, write '/tmp/x'",
            "Storage quota exceeded for this workspace",
        ] {
            assert!(!is_usage_limit(text), "{text}");
        }
    }

    #[test]
    fn a_throttle_resets_in_a_minute_and_a_plan_limit_stays_unknown() {
        let reset = |text: &str| match done_events(DoneStatus::Errored, Some(text.into()), None)[..]
        {
            [
                AgentEvent::UsageLimit { resets_at },
                AgentEvent::Done { .. },
            ] => resets_at,
            ref other => panic!("{text}: {other:?}"),
        };
        for text in [
            "harness protocol error: session/prompt: 429 Too Many Requests",
            "You have exceeded the rate limit of 30 requests per minute for the get_models endpoint",
        ] {
            let at = reset(text).expect(text);
            assert!((at - now_ms() - THROTTLE_MS).abs() < 5_000, "{text}");
        }
        for text in [
            "harness protocol error: session/prompt: Quota exhausted.",
            "Error 429, Message: Resource has been exhausted (e.g. check quota)., Status: RESOURCE_EXHAUSTED",
            "You've hit the rate limit for your plan. Upgrade your account or try again later.",
        ] {
            assert_eq!(reset(text), None, "{text}");
        }
    }
}
