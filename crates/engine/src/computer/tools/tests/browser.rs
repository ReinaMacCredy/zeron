use super::*;

use std::path::PathBuf;
use std::time::Duration;

use futures::FutureExt;
use tokio::sync::Notify;
use zeron_proto::computer::{ComputerDeliveryMode, CuaContent, McpContent};

use crate::computer::audit_log::COMPUTER_AUDIT_FILE_NAME;
use crate::computer::backend::{ComputerBrowserCall, ComputerBrowserCallResult};
use crate::computer::desktop_operation_queue::{DesktopOperationError, desktop_delivery_mode};
use crate::computer::fake_backend::{FakeBrowserHandler, FakeComputerOptIn};
use crate::computer::manager::ComputerConsentScope;
use crate::computer::model_desktop_observation::is_model_desktop_observation_active;
use crate::computer::tools::browser::{
    COMPUTER_BROWSER_TOOL_NAMES, ComputerBrowserTools, computer_browser_driver_name,
    computer_browser_tool_requires_approval,
};
use crate::computer::tools::{ComputerToolFamily, computer_tool_families};

const THREAD_BROWSER: &str = THREAD;

struct Browser {
    h: Harness,
    dir: tempfile::TempDir,
}

impl Browser {
    async fn audit_rows(&self) -> Vec<Value> {
        self.h.manager.dispose().await;
        std::fs::read_to_string(self.dir.path().join(COMPUTER_AUDIT_FILE_NAME))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn build(
    browser: FakeComputerOptIn<FakeBrowserHandler>,
    mode: Option<PermissionMode>,
    families: Option<Vec<Arc<dyn ComputerToolFamily>>>,
) -> Browser {
    let dir = tempfile::tempdir().unwrap();
    let backend = fake_with(crate::computer::fake_backend::FakeComputerBackendOptions {
        browser,
        ..Default::default()
    });
    let gate = ComputerApprovalGate::default();
    let approvals = Arc::new(Approvals {
        mode: Mutex::new(mode),
        decide: Mutex::new(None),
        gate: gate.clone(),
        cards: Mutex::default(),
        turn_inactive: AtomicBool::new(false),
    });
    let mut options = ComputerManagerOptions::new(backend.clone());
    options.approval_gate = gate;
    options.seams = approvals.clone();
    options.action_settle_ms = Some(0);
    options.env = ComputerManagerEnv::default();
    options.data_dir = Some(dir.path().to_path_buf());
    let manager = ComputerManager::new(options);
    let families = families.unwrap_or_else(|| computer_tool_families(&manager));
    Browser {
        h: Harness {
            tools: ComputerTools::new(manager.clone(), families),
            manager,
            backend,
            approvals,
            seams: Arc::new(ToolSeams::default()),
        },
        dir,
    }
}

fn built_in() -> Browser {
    build(FakeComputerOptIn::BuiltIn, Some(PermissionMode::Full), None)
}

fn replying(
    reply: impl Fn(&ComputerBrowserCall) -> ComputerBrowserCallResult + Send + Sync + 'static,
) -> Browser {
    build(handler(reply), Some(PermissionMode::Full), None)
}

fn handler(
    reply: impl Fn(&ComputerBrowserCall) -> ComputerBrowserCallResult + Send + Sync + 'static,
) -> FakeComputerOptIn<FakeBrowserHandler> {
    let reply = Arc::new(reply);
    FakeComputerOptIn::Handler(Arc::new(move |call| {
        let result = reply(&call);
        async move { Ok(Some(result)) }.boxed()
    }))
}

fn structured(value: Value) -> ComputerBrowserCallResult {
    ComputerBrowserCallResult {
        structured_content: value.as_object().cloned(),
        ..ComputerBrowserCallResult::default()
    }
}

fn with_text(value: Value, text: &str) -> ComputerBrowserCallResult {
    ComputerBrowserCallResult {
        content: Some(vec![CuaContent {
            kind: "text".into(),
            text: Some(text.into()),
            data: None,
            mime_type: None,
        }]),
        ..structured(value)
    }
}

fn ok() -> ComputerBrowserCallResult {
    structured(json!({ "status": "ok" }))
}

fn text_of(result: &McpToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|part| match part {
            McpContent::Text { text } => Some(text.as_str()),
            McpContent::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn subset(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => expected
            .iter()
            .all(|(key, value)| actual.get(key).is_some_and(|found| subset(found, value))),
        _ => actual == expected,
    }
}

fn binding(tabs: Value) -> ComputerBrowserCallResult {
    let count = tabs.as_array().map_or(0, Vec::len);
    with_text(
        json!({
            "status": "ok",
            "mode": "bind",
            "binding_quality": "exact",
            "native_title": "about:blank",
            "target_id": "bt-1",
            "tabs": tabs,
        }),
        &format!("bound target bt-1 (exact) with {count} tab(s)"),
    )
}

fn binds(tabs: Value) -> Browser {
    replying(move |call| {
        if call.name == "get_browser_state" {
            binding(tabs.clone())
        } else {
            ok()
        }
    })
}

fn one_tab() -> Value {
    json!([{ "tab_id": "tab-1", "active": true, "title": "about:blank", "url": "about:blank" }])
}

fn browser_methods(h: &Harness) -> Vec<String> {
    h.backend
        .methods()
        .into_iter()
        .filter(|method| method.starts_with("browser."))
        .collect()
}

fn recorded(calls: &Arc<Mutex<Vec<ComputerBrowserCall>>>) -> Vec<ComputerBrowserCall> {
    calls.lock().unwrap().clone()
}

fn seeing(
    reply: impl Fn(&ComputerBrowserCall) -> ComputerBrowserCallResult + Send + Sync + 'static,
) -> (Browser, Arc<Mutex<Vec<ComputerBrowserCall>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let browser = replying(move |call| {
        sink.lock().unwrap().push(call.clone());
        reply(call)
    });
    (browser, seen)
}

#[tokio::test]
async fn authorizes_fresh_browser_observations_only_while_the_model_state_call_runs() {
    let scopes = Arc::new(Mutex::new(Vec::new()));
    let sink = scopes.clone();
    let Browser { h, .. } = replying(move |_| {
        sink.lock()
            .unwrap()
            .push(is_model_desktop_observation_active());
        ok()
    });
    h.call("computer_browser_state", json!({ "pid": 123 }))
        .await;
    h.call("computer_browser_prepare", json!({ "allow_launch": true }))
        .await;
    assert_eq!(*scopes.lock().unwrap(), [true, false]);
    assert!(!is_model_desktop_observation_active());
    h.dispose().await;
}

#[tokio::test]
async fn registers_the_whole_family_with_state_as_the_only_read_only_tool() {
    let tools = ComputerBrowserTools::default();
    let entries = tools.entries();
    let names: Vec<&str> = entries.iter().map(|entry| entry.name()).collect();
    assert_eq!(names, COMPUTER_BROWSER_TOOL_NAMES);
    for entry in &entries {
        assert!(!entry.discovery_only, "{}", entry.name());
        let read_only = entry.definition.annotations.as_ref().unwrap()["readOnlyHint"].clone();
        assert_eq!(
            read_only,
            json!(entry.name() == "computer_browser_state"),
            "{}",
            entry.name()
        );
    }
}

#[tokio::test]
async fn states_the_rev_30_31_browser_contract() {
    let tools = ComputerBrowserTools::default();
    let entries = tools.entries();
    let by_name = |name: &str| {
        entries
            .iter()
            .find(|entry| entry.name() == name)
            .unwrap()
            .definition
            .clone()
    };
    let prepare = by_name("computer_browser_prepare");
    for needle in [
        "headless by default",
        "windowed:true",
        "isolated_named",
        "confirmed direct-X11 Escape listener",
        "only owned isolated headless targets support mutation",
        "Wayland/XWayland and standalone hosts permit reads/passive prepare only",
        "Linux refuses visible launch and personal-profile control",
        "browser_consent_required",
    ] {
        assert!(prepare.description.contains(needle), "{needle}");
    }
    assert!(
        !prepare
            .description
            .contains("Linux cannot launch headlessly")
    );
    assert!(
        prepare.input_schema["properties"]["windowed"]["description"]
            .as_str()
            .unwrap()
            .contains("confirmed direct-X11 Escape listener, and refuses true")
    );
    let state = by_name("computer_browser_state");
    assert!(state.description.contains("driver_owned_headless"));
    let window_id = state.input_schema["properties"]["window_id"]["description"]
        .as_str()
        .unwrap();
    assert!(window_id.contains("Omit it for a driver-owned headless browser"));
    assert!(!window_id.contains("required with pid"));
    assert_eq!(
        by_name("computer_browser_type").input_schema["properties"]["input_route"]["enum"],
        json!(["trusted", "dom_event"])
    );
}

#[test]
fn covers_every_gateway_name_with_a_driver_name() {
    for name in COMPUTER_BROWSER_TOOL_NAMES {
        assert!(computer_browser_driver_name(name).is_some(), "{name}");
    }
    assert_eq!(
        computer_browser_driver_name("computer_browser_press"),
        Some("browser_type")
    );
    assert_eq!(computer_browser_driver_name("computer_click"), None);
}

#[test]
fn requires_approval_for_everything_except_state_and_dialog_inspect() {
    assert!(!computer_browser_tool_requires_approval(
        "computer_browser_state",
        &ToolArgs::new()
    ));
    assert!(!computer_browser_tool_requires_approval(
        "computer_browser_dialog",
        &args(json!({ "action": "inspect" }))
    ));
    assert!(computer_browser_tool_requires_approval(
        "computer_browser_dialog",
        &args(json!({ "action": "accept" }))
    ));
    for name in COMPUTER_BROWSER_TOOL_NAMES {
        if name == "computer_browser_state" || name == "computer_browser_dialog" {
            continue;
        }
        assert!(
            computer_browser_tool_requires_approval(name, &ToolArgs::new()),
            "{name}"
        );
    }
}

#[tokio::test]
async fn binds_and_snapshots_without_an_approval_gate_passing_the_driver_reply_through() {
    let Browser { h, .. } = build(FakeComputerOptIn::BuiltIn, None, None);
    let result = h
        .call(
            "computer_browser_state",
            json!({ "target_id": "t-1", "tab_id": "tab-1" }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(
        result.structured_content.as_ref().unwrap()["target_id"],
        json!(format!("fake-browser-{THREAD_BROWSER}"))
    );
    assert_eq!(h.backend.calls_for("browser.get_browser_state").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn refuses_mutating_calls_before_dispatch_when_the_caller_has_no_permission_mode() {
    let Browser { h, .. } = build(FakeComputerOptIn::BuiltIn, None, None);
    let result = h
        .call(
            "computer_browser_click",
            json!({ "target_id": "t", "tab_id": "tab", "ref": "p1:0" }),
        )
        .await;
    assert!(result.is_error);
    assert!(text_of(&result).contains("denied"), "{result:?}");
    assert!(browser_methods(&h).is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn surfaces_a_denial_without_dispatching() {
    let Browser { h, .. } = built_in();
    h.decide(ApprovalDecision::Decline);
    let result = h
        .call(
            "computer_browser_type",
            json!({ "target_id": "t", "tab_id": "tab", "ref": "p1:0", "text": "hello" }),
        )
        .await;
    assert!(result.is_error);
    assert!(text_of(&result).contains("denied"));
    assert!(h.backend.calls_for("browser.browser_type").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_visible_browser_launch_despite_full_action_access_when_visibility_is_not_requested()
 {
    let browser = built_in();
    let h = &browser.h;
    h.decide(ApprovalDecision::Accept);
    h.authorize_visible_use(false);
    let result = h
        .call(
            "computer_browser_prepare",
            json!({ "allow_launch": true, "windowed": true, "profile": { "mode": "isolated_new" } }),
        )
        .await;
    assert!(result.is_error);
    assert!(
        text_of(&result).contains("foreground_not_requested"),
        "{result:?}"
    );
    assert!(h.approvals.cards.lock().unwrap().is_empty());
    assert!(h.backend.calls_for("browser.browser_prepare").is_empty());
    let rows = browser.audit_rows().await;
    assert!(
        rows.iter().any(|row| subset(
            row,
            &json!({ "effect": "not-dispatched", "code": "foreground_not_requested" })
        )),
        "{rows:?}"
    );
}

#[tokio::test]
async fn allows_an_explicitly_requested_visible_browser_and_checks_the_next_call_again() {
    let modes = Arc::new(Mutex::new(Vec::new()));
    let sink = modes.clone();
    let Browser { h, .. } = replying(move |_| {
        sink.lock().unwrap().push(desktop_delivery_mode());
        ok()
    });
    let arguments =
        json!({ "allow_launch": true, "windowed": true, "profile": { "mode": "isolated_new" } });
    let allowed = h.call("computer_browser_prepare", arguments.clone()).await;
    assert!(!allowed.is_error, "{allowed:?}");
    assert_eq!(h.backend.calls_for("browser.browser_prepare").len(), 1);
    h.authorize_visible_use(false);
    let revoked = h.call("computer_browser_prepare", arguments.clone()).await;
    assert!(text_of(&revoked).contains("foreground_not_requested"));
    assert_eq!(h.backend.calls_for("browser.browser_prepare").len(), 1);
    assert_eq!(h.seams.authorization_reads.load(Ordering::SeqCst), 3);
    assert_eq!(*modes.lock().unwrap(), [ComputerDeliveryMode::Foreground]);
    let mut headless = arguments;
    headless["windowed"] = json!(false);
    h.call("computer_browser_prepare", headless).await;
    assert_eq!(
        *modes.lock().unwrap(),
        [
            ComputerDeliveryMode::Foreground,
            ComputerDeliveryMode::Background
        ]
    );
    h.dispose().await;
}

#[tokio::test]
async fn refuses_when_visible_use_authorization_changes_while_ordinary_approval_is_pending() {
    let Browser { h, .. } = built_in();
    *h.approvals.mode.lock().unwrap() = Some(PermissionMode::Ask);
    let pending = h.call(
        "computer_browser_prepare",
        json!({ "allow_launch": true, "windowed": true, "profile": { "mode": "isolated_new" } }),
    );
    let answer = async {
        let card = loop {
            if let Some(card) = h.approvals.cards.lock().unwrap().first().cloned() {
                break card;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        };
        h.authorize_visible_use(false);
        h.approvals
            .gate
            .respond(&card.chat_id, &card.request_id, ApprovalDecision::Accept);
    };
    let (result, ()) = tokio::join!(pending, answer);
    assert!(
        text_of(&result).contains("foreground_not_requested"),
        "{result:?}"
    );
    assert!(h.backend.calls_for("browser.browser_prepare").is_empty());
    assert_eq!(h.seams.authorization_reads.load(Ordering::SeqCst), 2);
    h.dispose().await;
}

#[tokio::test]
async fn keeps_a_headless_launch_independent_of_visibility() {
    for windowed in [None, Some(false)] {
        let Browser { h, .. } = built_in();
        h.authorize_visible_use(false);
        let mut arguments = json!({ "allow_launch": true, "profile": { "mode": "isolated_new" } });
        if let Some(windowed) = windowed {
            arguments["windowed"] = json!(windowed);
        }
        let result = h.call("computer_browser_prepare", arguments).await;
        assert!(!result.is_error, "{windowed:?}: {result:?}");
        assert_eq!(h.backend.calls_for("browser.browser_prepare").len(), 1);
        assert_eq!(h.seams.authorization_reads.load(Ordering::SeqCst), 0);
        h.dispose().await;
    }
}

#[tokio::test]
async fn refuses_a_visible_launch_when_its_authorization_changes_in_the_browser_queue() {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let names = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, release_rx, sink) = (entered.clone(), release.clone(), names.clone());
    let browser: FakeBrowserHandler = Arc::new(move |call| {
        sink.lock().unwrap().push(call.name.clone());
        let (entered, release) = (entered_tx.clone(), release_rx.clone());
        async move {
            if call.name == "get_browser_state" {
                entered.notify_one();
                release.notified().await;
            }
            Ok(Some(ok()))
        }
        .boxed()
    });
    let Browser { h, .. } = build(
        FakeComputerOptIn::Handler(browser),
        Some(PermissionMode::Full),
        None,
    );
    let first = h.call(
        "computer_browser_state",
        json!({ "target_id": "t", "tab_id": "tab" }),
    );
    let visible = async {
        entered.notified().await;
        h.call(
            "computer_browser_prepare",
            json!({ "allow_launch": true, "windowed": true, "profile": { "mode": "isolated_new" } }),
        )
        .await
    };
    let driver = async {
        while h.seams.authorization_reads.load(Ordering::SeqCst) < 1 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        h.authorize_visible_use(false);
        release.notify_one();
    };
    let (_, result, ()) = tokio::join!(first, visible, driver);
    assert!(
        text_of(&result).contains("foreground_not_requested"),
        "{result:?}"
    );
    assert_eq!(h.seams.authorization_reads.load(Ordering::SeqCst), 2);
    assert_eq!(*names.lock().unwrap(), ["get_browser_state"]);
    h.dispose().await;
}

#[tokio::test]
async fn does_not_dispatch_after_cancellation_during_the_admitted_browser_check() {
    let Browser { h, .. } = built_in();
    let entered = Notify::new();
    let release = Notify::new();
    let token = CancellationToken::new();
    let pending = h.manager.browser_call(
        THREAD,
        Some(TURN),
        "browser_prepare",
        args(json!({ "windowed": true })),
        Some(token.clone().into()),
        Some(|| async {
            entered.notify_one();
            release.notified().await;
            Ok(())
        }),
    );
    let cancel = async {
        entered.notified().await;
        token.cancel();
        release.notify_one();
    };
    let (result, ()) = tokio::join!(pending, cancel);
    assert!(
        matches!(
            result,
            Err(ComputerError::Operation(DesktopOperationError::Aborted))
        ),
        "{result:?}"
    );
    assert!(browser_methods(&h).is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn audits_driver_replies_without_upgrading_their_effect() {
    let rows = [
        (
            "nested refusal code",
            structured(json!({ "status": "refused", "refusal": { "code": "browser_ref_stale" } })),
            json!({ "effect": "refused", "code": "browser_ref_stale" }),
        ),
        (
            "legacy top-level refusal code",
            structured(json!({ "status": "refused", "code": "browser_requires_setup" })),
            json!({ "effect": "refused", "code": "browser_requires_setup" }),
        ),
        (
            "typed refusal carrying isError",
            ComputerBrowserCallResult {
                is_error: Some(true),
                ..structured(
                    json!({ "status": "refused", "refusal": { "code": "browser_consent_required" } }),
                )
            },
            json!({ "effect": "refused", "code": "browser_consent_required" }),
        ),
        (
            "successful dispatch without effect proof",
            ok(),
            json!({ "effect": "dispatched-unknown" }),
        ),
        (
            "closed native action refusal",
            structured(json!({ "effect": "refused", "route": "dom" })),
            json!({ "effect": "refused", "code": "browser_refused" }),
        ),
        (
            "DOM readback without application effect proof",
            structured(json!({
                "status": "ok",
                "effect": "unverifiable",
                "route": "dom_event",
                "dispatched": true,
                "readback": "matched",
            })),
            json!({ "effect": "dispatched-unknown" }),
        ),
    ];
    for (label, reply, expected) in rows {
        let expected_structured = reply.structured_content.clone().map(Value::Object);
        let browser = replying(move |_| reply.clone());
        let result = browser
            .h
            .call(
                "computer_browser_type",
                json!({ "target_id": "t", "tab_id": "tab", "ref": "p1:0", "text": "private text" }),
            )
            .await;
        assert_eq!(result.structured_content, expected_structured, "{label}");
        let rows = browser.audit_rows().await;
        let mut wanted = expected.clone();
        wanted["tool"] = json!("computer_browser_type");
        let row = rows
            .iter()
            .find(|row| row["tool"] == json!("computer_browser_type"))
            .unwrap_or_else(|| panic!("{label}: {rows:?}"));
        assert!(subset(row, &wanted), "{label}: {row}");
        if wanted.get("code").is_none() {
            assert!(row.get("code").is_none(), "{label}: {row}");
        }
        assert!(
            !serde_json::to_string(&rows)
                .unwrap()
                .contains("private text"),
            "{label}"
        );
    }
}

#[tokio::test]
async fn records_a_proven_navigation_without_issuing_another_browser_call() {
    let browser = replying(|_| {
        structured(json!({
            "status": "ok",
            "target_id": "bt-1",
            "tab_id": "tab-1",
            "url": "https://example.test/",
            "verification": { "scope": "navigation", "method": "page_frame_tree", "status": "confirmed" },
        }))
    });
    browser
        .h
        .call(
            "computer_browser_navigate",
            json!({ "target_id": "bt-1", "tab_id": "tab-1", "url": "https://example.test/" }),
        )
        .await;
    assert_eq!(browser_methods(&browser.h).len(), 1);
    let rows = browser.audit_rows().await;
    assert!(
        rows.iter().any(|row| subset(
            row,
            &json!({ "tool": "computer_browser_navigate", "effect": "verified" })
        )),
        "{rows:?}"
    );
}

#[tokio::test]
async fn reports_field_readback_without_claiming_submission_succeeded() {
    let reply = json!({
        "effect": "unverifiable",
        "route": "dom",
        "evidence": [{ "kind": "value_readback" }],
    });
    let expected = reply.clone();
    let browser = replying(move |_| with_text(reply.clone(), "legacy dispatch summary"));
    let result = browser
        .h
        .call(
            "computer_browser_type",
            json!({
                "target_id": "bt-1",
                "tab_id": "tab-1",
                "ref": "p1:2",
                "text": "private value",
                "input_route": "dom_event",
                "replace": true,
            }),
        )
        .await;
    assert_eq!(result.structured_content, Some(expected));
    let text = text_of(&result);
    assert!(text.contains("Field value matched; application effect unverified"));
    assert!(!text.contains("private value"));
    let rows = browser.audit_rows().await;
    assert!(
        rows.iter()
            .any(|row| subset(row, &json!({ "effect": "dispatched-unknown" }))),
        "{rows:?}"
    );
}

#[tokio::test]
async fn offers_one_honest_next_step_after_unsupported_profile_attachment_without_launching_it() {
    let Browser { h, .. } = replying(|_| {
        structured(json!({
            "status": "refused",
            "refusal": { "code": "browser_consent_required", "message": "consent provider absent" },
        }))
    });
    let result = h
        .call("computer_browser_prepare", json!({ "pid": 42 }))
        .await;
    let structured = result.structured_content.clone().unwrap();
    assert!(subset(
        &structured,
        &json!({ "status": "refused", "refusal": { "code": "browser_consent_required" } })
    ));
    let text = text_of(&result);
    assert!(text.contains("cannot attach to your existing browser profile"));
    assert!(
        text.contains("computer_browser_prepare({allow_launch:true,profile:{mode:\"isolated_new\"")
    );
    assert!(text.contains("without your cookies"));
    assert!(text.contains("Do not substitute it when the task requires your existing profile"));
    assert_eq!(h.backend.calls_for("browser.browser_prepare").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn asks_the_gate_once_per_mutating_call_and_dispatches_the_mapped_driver_name() {
    let (Browser { h, .. }, seen) = seeing(|_| ok());
    h.decide(ApprovalDecision::Accept);
    let result = h
        .call(
            "computer_browser_click",
            json!({ "target_id": "t", "tab_id": "tab", "ref": "p1:0" }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let text = text_of(&result);
    assert!(!text.contains("disclosure"), "{text}");
    assert!(
        !text.contains(crate::computer::tools::COMPUTER_CONTROL_FIRST_MUTATION_DISCLOSURE),
        "{text}"
    );
    let cards: Vec<String> = h
        .approvals
        .cards
        .lock()
        .unwrap()
        .iter()
        .map(|card| card.tool_name.clone())
        .collect();
    assert_eq!(cards, ["computer_browser_click"]);
    let seen = recorded(&seen);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].name, "browser_click");
    assert_eq!(seen[0].task.thread_id, THREAD);
    assert_eq!(seen[0].task.turn_id.as_deref(), Some(TURN));
    assert!(seen[0].mutation);
    h.dispose().await;
}

#[tokio::test]
async fn treats_a_driver_refusal_as_a_result_never_a_tool_error() {
    let Browser { h, .. } = build(
        handler(|_| {
            with_text(
                json!({
                    "status": "refused",
                    "refusal": { "code": "browser_requires_setup", "message": "Prepare a browser first." },
                }),
                "refused (browser_requires_setup)",
            )
        }),
        None,
        None,
    );
    let result = h
        .call(
            "computer_browser_state",
            json!({ "target_id": "t", "tab_id": "tab" }),
        )
        .await;
    assert!(!result.is_error);
    assert!(subset(
        result.structured_content.as_ref().unwrap(),
        &json!({ "status": "refused", "refusal": { "code": "browser_requires_setup" } })
    ));
    h.dispose().await;
}

#[tokio::test]
async fn marks_get_browser_state_non_mutating_on_the_wire_but_mutating_otherwise() {
    let (Browser { h, .. }, seen) = seeing(|_| ComputerBrowserCallResult::default());
    h.call(
        "computer_browser_state",
        json!({ "target_id": "t", "tab_id": "tab" }),
    )
    .await;
    h.call(
        "computer_browser_dialog",
        json!({ "target_id": "t", "tab_id": "tab", "action": "inspect" }),
    )
    .await;
    h.call(
        "computer_browser_navigate",
        json!({ "target_id": "t", "tab_id": "tab", "url": "https://example.com/" }),
    )
    .await;
    let wire: Vec<(String, bool)> = recorded(&seen)
        .into_iter()
        .map(|call| (call.name, call.mutation))
        .collect();
    assert_eq!(
        wire,
        [
            ("get_browser_state".to_owned(), false),
            ("browser_dialog".to_owned(), true),
            ("browser_navigate".to_owned(), true),
        ]
    );
    h.dispose().await;
}

fn workspace() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn set_root(h: &Harness, root: &std::path::Path) {
    *h.seams.workspace_root.lock().unwrap() = Some(root.to_path_buf());
}

#[tokio::test]
async fn refuses_an_upload_that_resolves_outside_the_workspace_before_it_reaches_the_driver() {
    let root = workspace();
    let outside = workspace();
    let file = outside.path().join("secret.txt");
    std::fs::write(&file, "x").unwrap();
    let Browser { h, .. } = built_in();
    set_root(&h, root.path());
    let result = h
        .call(
            "computer_browser_upload",
            json!({ "target_id": "t", "tab_id": "tab", "ref": "p1:0", "files": [file] }),
        )
        .await;
    assert!(result.is_error);
    assert!(
        text_of(&result).contains("outside the active workspace"),
        "{result:?}"
    );
    assert!(
        h.backend
            .calls_for("browser.browser_set_input_files")
            .is_empty()
    );
    h.dispose().await;
}

#[tokio::test]
async fn canonicalizes_workspace_upload_paths_before_dispatch() {
    let root = workspace();
    let file = root.path().join("attach.txt");
    std::fs::write(&file, "x").unwrap();
    let (Browser { h, .. }, seen) = seeing(|_| ok());
    set_root(&h, root.path());
    let result = h
        .call(
            "computer_browser_upload",
            json!({ "target_id": "t", "tab_id": "tab", "ref": "p1:0", "files": [file] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let seen = recorded(&seen);
    assert_eq!(seen[0].name, "browser_set_input_files");
    let canonical: PathBuf = std::fs::canonicalize(&file).unwrap();
    assert_eq!(seen[0].args["files"], json!([canonical]));
    h.dispose().await;
}

#[tokio::test]
async fn refuses_file_transfer_tools_outright_when_no_workspace_boundary_exists() {
    let Browser { h, .. } = built_in();
    let upload = h
        .call(
            "computer_browser_upload",
            json!({ "target_id": "t", "tab_id": "tab", "ref": "p1:0", "files": ["/tmp/anything.txt"] }),
        )
        .await;
    assert!(upload.is_error);
    assert!(text_of(&upload).contains("No canonical workspace"));
    let download = h
        .call(
            "computer_browser_download",
            json!({ "target_id": "t", "tab_id": "tab", "ref": "p1:0", "destination_root": "/tmp" }),
        )
        .await;
    assert!(download.is_error);
    assert!(browser_methods(&h).is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn bounds_the_download_destination_to_the_workspace() {
    let root = workspace();
    let (Browser { h, .. }, seen) = seeing(|_| ok());
    set_root(&h, root.path());
    let denied = h
        .call(
            "computer_browser_download",
            json!({
                "target_id": "t",
                "tab_id": "tab",
                "ref": "p1:0",
                "destination_root": std::env::temp_dir(),
            }),
        )
        .await;
    assert!(denied.is_error);
    let allowed = h
        .call(
            "computer_browser_download",
            json!({
                "target_id": "t",
                "tab_id": "tab",
                "ref": "p1:0",
                "destination_root": root.path(),
            }),
        )
        .await;
    assert!(!allowed.is_error, "{allowed:?}");
    let names: Vec<String> = recorded(&seen).into_iter().map(|call| call.name).collect();
    assert_eq!(names, ["browser_download"]);
    h.dispose().await;
}

#[tokio::test]
async fn reports_no_browser_route_on_a_desktop_only_backend() {
    let Browser { h, .. } = build(
        FakeComputerOptIn::Off,
        Some(PermissionMode::Full),
        Some(vec![Arc::new(ComputerBrowserTools::default())]),
    );
    assert!(!h.manager.supports_browser());
    let result = h
        .call(
            "computer_browser_state",
            json!({ "target_id": "t", "tab_id": "tab" }),
        )
        .await;
    assert!(result.is_error);
    assert!(
        text_of(&result).contains("does not provide browser automation"),
        "{result:?}"
    );
    h.dispose().await;
}

#[tokio::test]
async fn ends_the_driver_browser_session_when_the_thread_is_removed() {
    let Browser { h, .. } = built_in();
    h.call(
        "computer_browser_state",
        json!({ "target_id": "t", "tab_id": "tab" }),
    )
    .await;
    h.manager.handle_thread_removed(THREAD).await;
    let ended: Vec<Value> = h
        .backend
        .calls_for("browser.endThread")
        .into_iter()
        .map(|call| call.args[0].clone())
        .collect();
    assert_eq!(ended, [json!(THREAD)]);
    h.dispose().await;
}

#[tokio::test]
async fn in_ask_mode_a_browser_click_raises_the_task_card_while_a_dialog_inspect_does_not() {
    let Browser { h, .. } = built_in();
    h.decide(ApprovalDecision::Accept);
    let inspected = h
        .call(
            "computer_browser_dialog",
            json!({ "target_id": "t", "tab_id": "tab", "action": "inspect" }),
        )
        .await;
    assert!(!inspected.is_error, "{inspected:?}");
    assert!(h.approvals.cards.lock().unwrap().is_empty());
    assert_eq!(h.backend.calls_for("browser.browser_dialog").len(), 1);
    let clicked = h
        .call(
            "computer_browser_click",
            json!({ "target_id": "t", "tab_id": "tab", "ref": "p1:0" }),
        )
        .await;
    assert!(!clicked.is_error, "{clicked:?}");
    let cards = h.approvals.cards.lock().unwrap().clone();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].tool_name, "computer_browser_click");
    assert_eq!(cards[0].scope, ComputerConsentScope::Task);
    assert_eq!(h.backend.calls_for("browser.browser_click").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn refuses_native_window_identities_on_browser_actions_before_approval_or_dispatch() {
    for scope in [
        json!({ "pid": 42, "window_id": 99 }),
        json!({ "target_id": "", "tab_id": "tab-1" }),
        json!({ "target_id": "bt-1", "tab_id": "tab-1", "pid": 42, "window_id": 99 }),
    ] {
        let browser = built_in();
        browser.h.decide(ApprovalDecision::Accept);
        let mut arguments = scope.clone();
        arguments["url"] = json!("https://example.com/");
        let result = browser.h.call("computer_browser_navigate", arguments).await;
        assert!(
            subset(
                result.structured_content.as_ref().unwrap(),
                &json!({ "status": "refused", "refusal": { "code": "browser_target_required" } })
            ),
            "{scope}"
        );
        let text = text_of(&result);
        assert!(text.contains("If the bind was refused"));
        assert!(!text.contains("computer_browser_prepare"));
        assert!(browser.h.approvals.cards.lock().unwrap().is_empty());
        assert!(
            browser
                .h
                .backend
                .calls_for("browser.browser_navigate")
                .is_empty()
        );
        let rows = browser.audit_rows().await;
        assert!(
            rows.iter().any(|row| subset(
                row,
                &json!({ "effect": "refused", "code": "browser_target_required" })
            )),
            "{scope}: {rows:?}"
        );
    }
}

#[tokio::test]
async fn explains_a_pid_only_bind_refusal_without_suggesting_an_invalid_navigation_scope() {
    let Browser { h, .. } = replying(|_| {
        structured(json!({
            "status": "refused",
            "refusal": {
                "code": "browser_wrong_target_refused",
                "detail": { "headless_driver_owned": false },
            },
        }))
    });
    let result = h.call("computer_browser_state", json!({ "pid": 42 })).await;
    assert!(subset(
        result.structured_content.as_ref().unwrap(),
        &json!({
            "status": "refused",
            "refusal": {
                "code": "browser_wrong_target_refused",
                "detail": { "headless_driver_owned": false },
            },
        })
    ));
    let text = text_of(&result);
    assert!(text.contains("computer_browser_state({pid,window_id})"));
    assert!(text.contains("Only a successful bind returns target_id"));
    assert!(text.contains("do not send pid/window_id to browser actions"));
    assert!(!text.contains("computer_browser_prepare"));
    h.dispose().await;
}

#[tokio::test]
async fn labels_target_id_and_tab_id_in_the_bind_result() {
    let Browser { h, .. } = binds(one_tab());
    let result = h
        .call(
            "computer_browser_state",
            json!({ "pid": 33_526, "window_id": 8_196 }),
        )
        .await;
    assert!(!result.is_error);
    assert!(subset(
        result.structured_content.as_ref().unwrap(),
        &json!({ "target_id": "bt-1", "tab_id": "tab-1" })
    ));
    let text = text_of(&result);
    assert!(text.contains("target_id=bt-1"));
    assert!(text.contains("tab_id=tab-1"));
    h.dispose().await;
}

#[tokio::test]
async fn resolves_an_omitted_tab_id_in_navigate_from_the_last_bind() {
    let Browser { h, .. } = binds(one_tab());
    h.call(
        "computer_browser_state",
        json!({ "pid": 33_526, "window_id": 8_196 }),
    )
    .await;
    let result = h
        .call(
            "computer_browser_navigate",
            json!({ "target_id": "bt-1", "url": "https://www.newegg.com/" }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let navigations = h.backend.calls_for("browser.browser_navigate");
    assert_eq!(navigations.len(), 1);
    assert!(subset(
        &navigations[0].args[0],
        &json!({ "target_id": "bt-1", "tab_id": "tab-1" })
    ));
    h.dispose().await;
}

#[tokio::test]
async fn resolves_an_omitted_tab_id_in_a_snapshot_from_the_same_bind() {
    let Browser { h, .. } = binds(one_tab());
    h.call(
        "computer_browser_state",
        json!({ "pid": 33_526, "window_id": 8_196 }),
    )
    .await;
    h.call("computer_browser_state", json!({ "target_id": "bt-1" }))
        .await;
    let states = h.backend.calls_for("browser.get_browser_state");
    assert_eq!(states.len(), 2);
    assert!(subset(
        &states[1].args[0],
        &json!({ "target_id": "bt-1", "tab_id": "tab-1" })
    ));
    h.dispose().await;
}

#[tokio::test]
async fn defaults_to_the_single_active_tab_when_several_are_open() {
    let Browser { h, .. } = binds(json!([
        { "tab_id": "tab-a", "active": false },
        { "tab_id": "tab-b", "active": true },
    ]));
    let bound = h
        .call(
            "computer_browser_state",
            json!({ "pid": 1, "window_id": 2 }),
        )
        .await;
    assert_eq!(
        bound.structured_content.as_ref().unwrap()["tab_id"],
        json!("tab-b")
    );
    h.call(
        "computer_browser_navigate",
        json!({ "target_id": "bt-1", "url": "https://example.com/" }),
    )
    .await;
    assert_eq!(
        h.backend.calls_for("browser.browser_navigate")[0].args[0]["tab_id"],
        json!("tab-b")
    );
    h.dispose().await;
}

#[tokio::test]
async fn refuses_an_ambiguous_target_with_its_tab_listing_and_dispatches_nothing() {
    let Browser { h, .. } = binds(json!([
        { "tab_id": "tab-a", "active": false },
        { "tab_id": "tab-b", "active": false },
    ]));
    h.call(
        "computer_browser_state",
        json!({ "pid": 1, "window_id": 2 }),
    )
    .await;
    let result = h
        .call(
            "computer_browser_navigate",
            json!({ "target_id": "bt-1", "url": "https://example.com/" }),
        )
        .await;
    assert!(!result.is_error);
    assert!(subset(
        result.structured_content.as_ref().unwrap(),
        &json!({ "status": "refused", "refusal": { "code": "browser_tab_required" } })
    ));
    let text = text_of(&result);
    assert!(text.contains("tab-a"));
    assert!(text.contains("tab-b"));
    assert!(h.backend.calls_for("browser.browser_navigate").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn refuses_an_omitted_tab_id_for_a_target_this_thread_never_bound() {
    let Browser { h, .. } = built_in();
    let result = h
        .call(
            "computer_browser_navigate",
            json!({ "target_id": "bt-never-bound", "url": "https://example.com/" }),
        )
        .await;
    assert!(!result.is_error);
    assert!(subset(
        result.structured_content.as_ref().unwrap(),
        &json!({ "status": "refused", "refusal": { "code": "browser_tab_required" } })
    ));
    assert!(h.backend.calls_for("browser.browser_navigate").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn does_not_resolve_a_target_that_belongs_to_another_thread() {
    let Browser { h, .. } = binds(one_tab());
    h.call(
        "computer_browser_state",
        json!({ "pid": 1, "window_id": 2 }),
    )
    .await;
    let other = h.context_for("other-thread", None);
    let result = h
        .call_in(
            "computer_browser_navigate",
            json!({ "target_id": "bt-1", "url": "https://example.com/" }),
            &other,
        )
        .await;
    assert!(subset(
        result.structured_content.as_ref().unwrap(),
        &json!({ "refusal": { "code": "browser_tab_required" } })
    ));
    assert!(h.backend.calls_for("browser.browser_navigate").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn explains_a_swapped_target_and_tab_id_when_the_driver_cannot_find_the_tab() {
    let Browser { h, .. } = replying(|_| {
        structured(json!({
            "status": "refused",
            "refusal": {
                "code": "browser_tab_not_found",
                "message": "tab bt-85991064 is not known for target bt-85991064",
            },
        }))
    });
    let result = h
        .call(
            "computer_browser_navigate",
            json!({ "target_id": "bt-85991064", "tab_id": "bt-85991064", "url": "https://example.com/" }),
        )
        .await;
    assert!(!result.is_error);
    assert!(subset(
        result.structured_content.as_ref().unwrap(),
        &json!({ "status": "refused", "refusal": { "code": "browser_tab_not_found" } })
    ));
    let text = text_of(&result);
    assert!(text.contains("is a target id, not a tab id"));
    assert!(text.contains("tab-"));
    h.dispose().await;
}

#[tokio::test]
async fn labels_the_bind_key_on_a_prepare_result_and_leaves_tab_id_optional_on_target_tools() {
    let Browser { h, .. } = replying(|call| {
        if call.name == "browser_prepare" {
            structured(json!({
                "status": "ok",
                "prepared": true,
                "prepared_pid": 33_526,
                "action": "launched_isolated_browser",
            }))
        } else {
            ok()
        }
    });
    let prepared = h
        .call(
            "computer_browser_prepare",
            json!({ "allow_launch": true, "profile": { "mode": "isolated_new" } }),
        )
        .await;
    let text = text_of(&prepared);
    assert!(text.contains("prepared_pid=33526"), "{text}");
    assert!(text.contains("computer_browser_state"));
    assert!(text.contains("takes pid alone"));
    for name in &COMPUTER_BROWSER_TOOL_NAMES[2..] {
        let required = h.tools.entry(name).unwrap().definition.input_schema["required"].clone();
        let required: Vec<&str> = required
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(required.contains(&"target_id"), "{name}");
        assert!(!required.contains(&"tab_id"), "{name}");
    }
    h.dispose().await;
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn native_browser_uses_live_task_identity_and_preserves_host_errors() {
    let (browser, calls) = seeing(|_| ComputerBrowserCallResult {
        is_error: Some(true),
        ..with_text(json!({"status":"refused"}), "The user took over this tab.")
    });
    let result = browser
        .h
        .call(
            "computer_zeron_browser",
            json!({"action":"state", "tab_id":"native-tab", "control_token":"token"}),
        )
        .await;
    assert!(result.is_error);
    assert!(text_of(&result).contains("took over"));
    let seen = recorded(&calls);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].name, "zeron_browser");
    assert_eq!(seen[0].task.thread_id, THREAD);
    assert_eq!(seen[0].args["tab_id"], "native-tab");
    browser.h.seams.turn_inactive.store(true, Ordering::SeqCst);
    let stale = browser
        .h
        .call(
            "computer_zeron_browser",
            json!({"action":"open", "url":"http://localhost:3000"}),
        )
        .await;
    assert!(stale.is_error);
    assert_eq!(
        recorded(&calls).len(),
        1,
        "ended turn must not reach native host"
    );
    browser.h.dispose().await;
}
