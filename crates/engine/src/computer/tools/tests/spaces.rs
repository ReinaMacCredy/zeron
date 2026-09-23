use super::*;

use futures::FutureExt;
use zeron_proto::computer::{
    ComputerSpace, ComputerSpaceInventory, ComputerSpaceKind, ComputerSpaceSource,
};

use crate::computer::space_broker::ComputerSpaceOwner;

fn inventory() -> ComputerSpaceInventory {
    ComputerSpaceInventory {
        source: ComputerSpaceSource::MacosManagedSpaces,
        complete: true,
        spaces: vec![ComputerSpace {
            id: 2,
            uuid: Some("uuid-2".into()),
            display_id: "display-a".into(),
            kind: ComputerSpaceKind::Desktop,
            current: Some(false),
        }],
    }
}

struct Spaces {
    h: Harness,
    list_spaces: Arc<AtomicUsize>,
}

fn setup(designation: Vec<u64>) -> Spaces {
    let backend = Scripted::default_fake();
    let list_spaces = Arc::new(AtomicUsize::new(0));
    let counted = list_spaces.clone();
    backend.hooks().list_spaces = Some(Arc::new(move |_, ()| {
        counted.fetch_add(1, Ordering::SeqCst);
        async { Ok(inventory()) }.boxed()
    }));
    let h = harness_with(HarnessOptions {
        backend,
        ..HarnessOptions::default()
    });
    *h.seams.designation.lock().unwrap() = designation;
    h.decide(ApprovalDecision::Accept);
    Spaces { h, list_spaces }
}

async fn inspect(h: &Harness, arguments: Value) -> McpToolResult {
    h.call(
        "computer_inspect",
        json!({ "tool": "computer_spaces", "arguments": arguments }),
    )
    .await
}

#[tokio::test]
async fn has_a_real_inspect_and_help_route_and_adds_no_advertised_tool() {
    let Spaces { h, .. } = setup(vec![2]);
    assert_eq!(
        h.tools
            .entry("computer_spaces")
            .map(|entry| entry.discovery_only),
        Some(true)
    );
    assert!(
        !h.tools
            .catalog()
            .iter()
            .any(|tool| tool.name == "computer_spaces")
    );
    let help = h
        .call("computer_help", json!({ "tool": "computer_spaces" }))
        .await;
    assert!(result_json(&help).to_string().contains("computer_inspect"));
    let result = inspect(&h, json!({ "operation": "list" })).await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(
        payload["inventory"],
        serde_json::to_value(inventory()).unwrap()
    );
    assert_eq!(payload["reservation"], Value::Null);
    assert_eq!(payload["changedDesktop"], json!(false));
    h.dispose().await;
}

#[tokio::test]
async fn reserves_only_after_fresh_user_designation_without_repeated_desktop_approval() {
    let Spaces { h, .. } = setup(vec![2]);
    let result = inspect(&h, json!({ "operation": "reserve", "space_id": 2 })).await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(payload["reservation"]["spaceId"], json!(2));
    assert_eq!(payload["changedDesktop"], json!(false));
    let text = payload.to_string();
    assert!(!text.contains("\"threadId\""), "{text}");
    assert!(!text.contains("\"turnId\""), "{text}");
    assert_eq!(h.seams.designation_reads.load(Ordering::SeqCst), 2);
    assert!(h.approvals.cards.lock().unwrap().is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn full_access_cannot_invent_user_designation_and_late_revocation_prevents_commit() {
    let Spaces { h, .. } = setup(Vec::new());
    let denied = inspect(&h, json!({ "operation": "reserve", "space_id": 2 })).await;
    assert_eq!(
        result_json(&denied)["error"]["code"],
        json!("computer_space_not_designated")
    );
    h.seams
        .designation_script
        .lock()
        .unwrap()
        .extend([vec![2], Vec::new()]);
    let revoked = inspect(&h, json!({ "operation": "reserve", "space_id": 2 })).await;
    assert_eq!(
        result_json(&revoked)["error"]["code"],
        json!("computer_space_not_designated")
    );
    assert!(
        h.manager
            .space_broker()
            .reservation_for(&ComputerSpaceOwner {
                thread_id: THREAD.into(),
                turn_id: Some(TURN.into()),
            })
            .is_none()
    );
    h.dispose().await;
}

#[tokio::test]
async fn native_space_mutations_are_an_honest_refusal_without_native_calls() {
    for operation in ["create", "move", "switch", "follow"] {
        let Spaces { h, list_spaces } = setup(vec![2]);
        let result = inspect(&h, json!({ "operation": operation })).await;
        assert_eq!(
            result_json(&result)["error"]["code"],
            json!("computer_space_operation_unsupported"),
            "{operation}"
        );
        assert_eq!(list_spaces.load(Ordering::SeqCst), 0, "{operation}");
        assert!(h.backend.calls_for("raiseWindow").is_empty(), "{operation}");
        h.dispose().await;
    }
}

#[tokio::test]
async fn validates_inspect_arguments_using_the_canonical_schema() {
    let Spaces { h, .. } = setup(vec![2]);
    assert!(
        inspect(
            &h,
            json!({ "operation": "reserve", "space_id": 2, "user_designated": true })
        )
        .await
        .is_error
    );
    assert!(
        inspect(&h, json!({ "operation": "reserve", "space_id": 2.1 }))
            .await
            .is_error
    );
    h.dispose().await;
}

#[tokio::test]
async fn is_callable_by_exact_name_outside_computer_inspect() {
    let Spaces { h, .. } = setup(vec![2]);
    let result = h.call("computer_spaces", json!({})).await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(result_json(&result)["operation"], json!("list"));
    h.dispose().await;
}
