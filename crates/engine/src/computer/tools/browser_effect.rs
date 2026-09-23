use serde_json::{Map, Value};
use zeron_proto::computer::ComputerAuditEffect;

use super::results::AuditOutcome;
use super::tool_input::ToolArgs;
use crate::computer::backend::ComputerBrowserCallResult;

fn record(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value.and_then(Value::as_object)
}

fn str_of<'a>(map: Option<&'a Map<String, Value>>, key: &str) -> Option<&'a str> {
    map.and_then(|map| map.get(key)).and_then(Value::as_str)
}

pub fn computer_browser_effect(
    name: &str,
    args: &ToolArgs,
    result: &ComputerBrowserCallResult,
) -> AuditOutcome {
    let structured = result.structured_content.as_ref();
    let refusal = record(structured.and_then(|map| map.get("refusal")));
    let detail = record(refusal.and_then(|map| map.get("detail")));
    let status = str_of(structured, "status");
    let effect = str_of(structured, "effect");
    let delivered_prefix = detail
        .and_then(|detail| detail.get("delivered_chars"))
        .and_then(Value::as_f64)
        .is_some_and(|chars| chars > 0.0);
    if effect == Some("partial")
        || (status == Some("refused")
            && str_of(refusal, "code") == Some("browser_input_incomplete")
            && delivered_prefix)
    {
        return AuditOutcome::new(ComputerAuditEffect::DispatchedUnknown, None);
    }
    if status == Some("refused") || effect == Some("refused") {
        let code = str_of(refusal, "code")
            .or_else(|| str_of(structured, "code"))
            .unwrap_or("browser_refused");
        return AuditOutcome::new(ComputerAuditEffect::Refused, Some(code));
    }
    if result.is_error == Some(true) {
        let code = str_of(structured, "error").unwrap_or("browser_error");
        return AuditOutcome::new(ComputerAuditEffect::Error, Some(code));
    }
    let bytes = structured
        .and_then(|map| map.get("bytes"))
        .and_then(Value::as_f64)
        .is_some_and(|bytes| {
            bytes >= 0.0 && bytes.fract() == 0.0 && bytes <= 9_007_199_254_740_991.0
        });
    if name == "computer_browser_download"
        && status == Some("completed")
        && str_of(structured, "download_id").is_some_and(|id| !id.is_empty())
        && bytes
    {
        return AuditOutcome::new(ComputerAuditEffect::Verified, None);
    }
    let verification = record(structured.and_then(|map| map.get("verification")));
    let echoes = |key: &str| {
        let sent = args.get(key).and_then(Value::as_str);
        sent.is_some() && str_of(structured, key) == sent
    };
    if name == "computer_browser_navigate"
        && status == Some("ok")
        && echoes("target_id")
        && echoes("tab_id")
        && echoes("url")
        && str_of(verification, "scope") == Some("navigation")
        && str_of(verification, "method") == Some("page_frame_tree")
        && str_of(verification, "status") == Some("confirmed")
    {
        return AuditOutcome::new(ComputerAuditEffect::Verified, None);
    }
    AuditOutcome::new(ComputerAuditEffect::DispatchedUnknown, None)
}

pub fn computer_browser_field_readback(
    name: &str,
    args: &ToolArgs,
    result: &ComputerBrowserCallResult,
) -> bool {
    let structured = result.structured_content.as_ref();
    let mode = args.get("mode");
    name == "computer_browser_type"
        && args.get("text").is_some_and(Value::is_string)
        && (mode.is_none() || mode.and_then(Value::as_str) == Some("insert_text"))
        && args.get("replace") == Some(&Value::Bool(true))
        && args.get("input_route").and_then(Value::as_str) == Some("dom_event")
        && result.is_error != Some(true)
        && str_of(structured, "effect") == Some("unverifiable")
        && str_of(structured, "route") == Some("dom")
        && structured
            .and_then(|map| map.get("evidence"))
            .and_then(Value::as_array)
            .is_some_and(|evidence| {
                evidence.iter().any(|item| {
                    item.as_object()
                        .and_then(|item| item.get("kind"))
                        .and_then(Value::as_str)
                        == Some("value_readback")
                })
            })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn result(structured: Value) -> ComputerBrowserCallResult {
        ComputerBrowserCallResult {
            structured_content: structured.as_object().cloned(),
            ..ComputerBrowserCallResult::default()
        }
    }

    fn failed(structured: Value) -> ComputerBrowserCallResult {
        ComputerBrowserCallResult {
            is_error: Some(true),
            ..result(structured)
        }
    }

    fn args(value: Value) -> ToolArgs {
        value.as_object().cloned().unwrap_or_default()
    }

    fn outcome(effect: ComputerAuditEffect, code: Option<&str>) -> AuditOutcome {
        AuditOutcome::new(effect, code)
    }

    fn navigation_args() -> ToolArgs {
        args(json!({ "target_id": "bt-1", "tab_id": "tab-1", "url": "https://example.test/" }))
    }

    fn navigation_proof() -> Value {
        json!({
            "status": "ok",
            "target_id": "bt-1",
            "tab_id": "tab-1",
            "url": "https://example.test/",
            "verification": { "scope": "navigation", "method": "page_frame_tree", "status": "confirmed" },
        })
    }

    #[test]
    fn does_not_promote_generic_browser_dispatch_evidence() {
        for structured in [
            json!({ "status": "ok" }),
            json!({ "effect": "confirmed", "evidence": [{ "kind": "value_readback" }] }),
            json!({ "effect": "unverifiable", "route": "dom", "evidence": [{ "kind": "value_readback" }] }),
            json!({ "effect": "partial", "route": "trusted_input", "delivery": { "delivered_count": 2 } }),
            json!({ "effect": "suspected_noop" }),
            json!({ "status": "completed", "download_id": "some-file", "bytes": 200 }),
            json!({ "status": "ok", "delivered_chars": 12, "requested_chars": 12, "readback": "matched" }),
        ] {
            assert_eq!(
                computer_browser_effect(
                    "computer_browser_click",
                    &ToolArgs::new(),
                    &result(structured.clone())
                ),
                outcome(ComputerAuditEffect::DispatchedUnknown, None),
                "{structured}"
            );
        }
    }

    #[test]
    fn recognizes_legacy_and_current_driver_refusals() {
        for (structured, code) in [
            (
                json!({ "status": "refused", "refusal": { "code": "browser_ref_stale" } }),
                "browser_ref_stale",
            ),
            (
                json!({ "status": "refused", "code": "browser_requires_setup" }),
                "browser_requires_setup",
            ),
            (
                json!({ "effect": "refused", "route": "dom" }),
                "browser_refused",
            ),
        ] {
            let expected = outcome(ComputerAuditEffect::Refused, Some(code));
            assert_eq!(
                computer_browser_effect(
                    "computer_browser_type",
                    &ToolArgs::new(),
                    &result(structured.clone())
                ),
                expected
            );
            assert_eq!(
                computer_browser_effect(
                    "computer_browser_type",
                    &ToolArgs::new(),
                    &failed(structured)
                ),
                expected
            );
        }
    }

    #[test]
    fn accepts_the_native_download_proof_only_for_the_download_operation() {
        let proof = |bytes: Value| {
            computer_browser_effect(
                "computer_browser_download",
                &ToolArgs::new(),
                &result(
                    json!({ "status": "completed", "download_id": "opaque-file", "bytes": bytes }),
                ),
            )
        };
        for bytes in [json!(0), json!(7_200)] {
            assert_eq!(proof(bytes), outcome(ComputerAuditEffect::Verified, None));
        }
        for bytes in [
            json!(-1),
            json!(0.5),
            json!(f64::INFINITY),
            json!(f64::NAN),
            json!("7200"),
            Value::Null,
        ] {
            assert_eq!(
                proof(bytes.clone()),
                outcome(ComputerAuditEffect::DispatchedUnknown, None),
                "{bytes}"
            );
        }
        assert_eq!(
            computer_browser_effect(
                "computer_browser_download",
                &ToolArgs::new(),
                &result(json!({ "status": "completed", "bytes": 7200 })),
            ),
            outcome(ComputerAuditEffect::DispatchedUnknown, None)
        );
    }

    #[test]
    fn preserves_a_delivered_prefix_instead_of_reporting_a_safe_to_retry_refusal() {
        assert_eq!(
            computer_browser_effect(
                "computer_browser_type",
                &ToolArgs::new(),
                &result(json!({
                    "status": "refused",
                    "refusal": { "code": "browser_input_incomplete", "detail": { "delivered_chars": 2 } },
                })),
            ),
            outcome(ComputerAuditEffect::DispatchedUnknown, None)
        );
    }

    #[test]
    fn verifies_only_a_matching_navigation_with_explicit_fresh_native_evidence() {
        let args = navigation_args();
        assert_eq!(
            computer_browser_effect(
                "computer_browser_navigate",
                &args,
                &result(navigation_proof())
            ),
            outcome(ComputerAuditEffect::Verified, None)
        );
        for key in ["target_id", "tab_id", "url"] {
            let mut proof = navigation_proof();
            proof[key] = json!("different");
            assert_eq!(
                computer_browser_effect("computer_browser_navigate", &args, &result(proof)),
                outcome(ComputerAuditEffect::DispatchedUnknown, None),
                "{key}"
            );
        }
        for verification in [
            Value::Null,
            json!({}),
            json!({ "scope": "navigation", "method": "page_frame_tree", "status": "unconfirmed" }),
            json!({ "scope": "navigation", "method": "url_echo", "status": "confirmed" }),
            json!({ "scope": "field", "method": "page_frame_tree", "status": "confirmed" }),
        ] {
            let mut proof = navigation_proof();
            proof["verification"] = verification;
            assert_eq!(
                computer_browser_effect("computer_browser_navigate", &args, &result(proof)),
                outcome(ComputerAuditEffect::DispatchedUnknown, None)
            );
        }
        assert_eq!(
            computer_browser_effect("computer_browser_click", &args, &result(navigation_proof())),
            outcome(ComputerAuditEffect::DispatchedUnknown, None)
        );
    }

    #[test]
    fn never_lets_proof_override_a_reported_error() {
        assert_eq!(
            computer_browser_effect(
                "computer_browser_navigate",
                &navigation_args(),
                &failed(navigation_proof())
            ),
            outcome(ComputerAuditEffect::Error, Some("browser_error"))
        );
    }

    #[test]
    fn keeps_absent_structured_data_unknown() {
        assert_eq!(
            computer_browser_effect(
                "computer_browser_navigate",
                &navigation_args(),
                &ComputerBrowserCallResult::default(),
            ),
            outcome(ComputerAuditEffect::DispatchedUnknown, None)
        );
    }

    fn readback_args() -> Value {
        json!({ "text": "private value", "replace": true, "input_route": "dom_event" })
    }

    fn readback_result() -> Value {
        json!({ "effect": "unverifiable", "route": "dom", "evidence": [{ "kind": "value_readback" }] })
    }

    #[test]
    fn recognizes_field_evidence_without_promoting_the_application_effect() {
        let args = args(readback_args());
        assert!(computer_browser_field_readback(
            "computer_browser_type",
            &args,
            &result(readback_result())
        ));
        assert_eq!(
            computer_browser_effect("computer_browser_type", &args, &result(readback_result())),
            outcome(ComputerAuditEffect::DispatchedUnknown, None)
        );
    }

    #[test]
    fn does_not_treat_field_readback_as_proof_for_other_tools() {
        for name in [
            "computer_browser_press",
            "computer_browser_click",
            "computer_browser_upload",
        ] {
            assert!(
                !computer_browser_field_readback(
                    name,
                    &args(readback_args()),
                    &result(readback_result())
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn requires_the_exact_replacement_route_and_actual_value_evidence() {
        let with = |patch: Value| {
            let mut merged = readback_args();
            for (key, value) in patch.as_object().unwrap() {
                merged[key] = value.clone();
            }
            args(merged)
        };
        let good = result(readback_result());
        assert!(!computer_browser_field_readback(
            "computer_browser_type",
            &with(json!({ "replace": false })),
            &good
        ));
        assert!(!computer_browser_field_readback(
            "computer_browser_type",
            &with(json!({ "mode": "keystrokes" })),
            &good
        ));
        assert!(!computer_browser_field_readback(
            "computer_browser_type",
            &with(json!({ "input_route": "trusted" })),
            &good
        ));
        assert!(!computer_browser_field_readback(
            "computer_browser_type",
            &args(readback_args()),
            &failed(readback_result())
        ));
        for evidence in [
            Value::Null,
            json!([]),
            json!([{ "kind": "window_change" }]),
            json!([{ "kind": "url_echo" }]),
        ] {
            let mut structured = readback_result();
            structured["evidence"] = evidence;
            assert!(!computer_browser_field_readback(
                "computer_browser_type",
                &args(readback_args()),
                &result(structured)
            ));
        }
    }
}
