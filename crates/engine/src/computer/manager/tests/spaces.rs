use serde_json::Map;
use zeron_proto::computer::{ComputerSpace, ComputerSpaceKind, ComputerSpaceSource};

use super::*;
use crate::computer::backend::{ComputerLaunchOptions, ComputerMenuTarget};
use crate::computer::fake_backend::FakeComputerOptIn;
use crate::computer::space_broker::ComputerSpaceOwner;
use crate::computer::task_context::with_computer_task;

const WINDOW: &str = "cua:10:20";

fn fixture_window() -> ComputerWindow {
    ComputerWindow {
        app_name: Some("Fixture".into()),
        pid: Some(10),
        visible: false,
        space_ids: Some(vec![2]),
        current_space_id: Some(1),
        on_current_space: Some(false),
        ..window(WINDOW, "Fixture", Some(rect(20.0, 20.0, 300.0, 200.0)))
    }
}

fn inventory(current: u64) -> ComputerSpaceInventory {
    ComputerSpaceInventory {
        source: ComputerSpaceSource::MacosManagedSpaces,
        complete: true,
        spaces: [1, 2]
            .into_iter()
            .map(|id| ComputerSpace {
                id,
                uuid: Some(format!("uuid-{id}")),
                display_id: "display-a".into(),
                kind: ComputerSpaceKind::Desktop,
                current: Some(id == current),
            })
            .collect(),
    }
}

fn owner(turn_id: Option<&str>) -> ComputerSpaceOwner {
    ComputerSpaceOwner {
        thread_id: "a".into(),
        turn_id: turn_id.map(str::to_owned),
    }
}

struct Reserved {
    manager: Arc<ComputerManager>,
    backend: Arc<Scripted>,
    current_space: Arc<Mutex<u64>>,
}

async fn setup() -> Reserved {
    let backend = Scripted::new(FakeComputerBackendOptions {
        windows: Some(vec![fixture_window()]),
        agent_dialect: Some(ComputerAgentDialect::Macos),
        browser: FakeComputerOptIn::BuiltIn,
        ..FakeComputerBackendOptions::default()
    });
    let current_space = Arc::new(Mutex::new(1));
    let read = current_space.clone();
    backend.hooks().list_spaces = Some(Arc::new(move |_, ()| {
        let current = *read.lock().unwrap();
        async move { Ok(inventory(current)) }.boxed()
    }));
    let Built {
        manager, backend, ..
    } = settled(backend);
    manager
        .space_broker()
        .reserve(&owner(None), 2, &[2], Some(WINDOW), None)
        .await
        .unwrap();
    Reserved {
        manager,
        backend,
        current_space,
    }
}

const NATIVE: [&str; 7] = [
    "raiseWindow",
    "launchApp",
    "invokeMenu",
    "setAppVisibility",
    "setWindowMinimized",
    "setWindowFrame",
    "killApp",
];

#[tokio::test]
async fn guards_exact_target_keys_and_refuses_them_after_the_user_enters_the_space() {
    let Reserved {
        manager,
        backend,
        current_space,
    } = setup().await;
    manager
        .press_key(Some("a"), "Tab", Some(WINDOW), None)
        .await
        .unwrap();
    assert_eq!(backend.calls_for("pressKey").len(), 1);
    assert!(backend.calls_for("raiseWindow").is_empty());
    *current_space.lock().unwrap() = 2;
    let error = manager
        .press_key(Some("a"), "Tab", Some(WINDOW), None)
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_space_current"));
    assert_eq!(backend.calls_for("pressKey").len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_every_native_excursion_before_dispatch_even_with_visible_use_consent() {
    let rows = [
        "activate",
        "foreground",
        "launch",
        "app menu",
        "window menu",
        "visibility",
        "minimize",
        "move",
        "kill",
    ];
    for row in rows {
        let Reserved {
            manager, backend, ..
        } = setup().await;
        let path = ["File".to_owned(), "New".to_owned()];
        let outcome = match row {
            "activate" => manager
                .activate_window(Some("a"), WINDOW, visible_use())
                .await
                .map(|_| ()),
            "foreground" => manager
                .foreground_with_restore(Some("a"), WINDOW, None, visible_use())
                .await
                .map(|_| ()),
            "launch" => manager
                .launch_app(
                    Some("a"),
                    "Fixture",
                    &[],
                    0,
                    ComputerLaunchOptions::default(),
                )
                .await
                .map(|_| ()),
            "app menu" => manager
                .invoke_menu(Some("a"), ComputerMenuTarget::Pid(10), &path, visible_use())
                .await
                .map(|_| ()),
            "window menu" => manager
                .invoke_menu(
                    Some("a"),
                    ComputerMenuTarget::Window(WINDOW.into()),
                    &path,
                    visible_use(),
                )
                .await
                .map(|_| ()),
            "visibility" => manager
                .set_app_visibility(Some("a"), 10, false)
                .await
                .map(|_| ()),
            "minimize" => manager
                .set_window_minimized(Some("a"), WINDOW, true)
                .await
                .map(|_| ()),
            "move" => manager
                .set_window_frame(Some("a"), WINDOW, rect(0.0, 0.0, 300.0, 200.0))
                .await
                .map(|_| ()),
            "kill" => manager.kill_app(Some("a"), WINDOW).await.map(|_| ()),
            _ => unreachable!(),
        };
        let error = outcome.expect_err(row);
        assert_eq!(
            code(&error),
            Some("computer_space_operation_unsupported"),
            "{row}: {error}"
        );
        for method in NATIVE {
            assert!(backend.calls_for(method).is_empty(), "{row}: {method}");
        }
        manager.dispose().await;
    }
}

#[tokio::test]
async fn rejects_unbound_keys_and_coordinates_before_input() {
    let Reserved {
        manager, backend, ..
    } = setup().await;
    let error = manager
        .press_key(Some("a"), "Tab", None, None)
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_space_window_not_selected"));
    let error = manager
        .click(Some("a"), point(5.0, 5.0), &[], Default::default())
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_space_window_not_selected"));
    assert!(backend.calls_for("pressKey").is_empty());
    assert!(backend.calls_for("click").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn generic_foreground_excursions_refuse_before_the_action_or_restore_can_run() {
    let Reserved {
        manager, backend, ..
    } = setup().await;
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    let error = manager
        .with_foreground_restore(
            Some("a"),
            move || async move {
                flag.store(true, Ordering::SeqCst);
                Ok("changed")
            },
            visible_use(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_space_operation_unsupported"));
    assert!(!ran.load(Ordering::SeqCst));
    assert!(backend.calls_for("raiseWindow").is_empty());
    manager.dispose().await;
}

fn prepare(windowed: bool) -> Map<String, Value> {
    json!({ "allow_launch": true, "windowed": windowed })
        .as_object()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn a_visible_browser_launch_refuses_while_isolated_headless_preparation_stays_separate() {
    let Reserved {
        manager, backend, ..
    } = setup().await;
    let error = manager
        .browser_call(
            "a",
            None,
            "browser_prepare",
            prepare(true),
            None,
            Some(|| async { Ok(()) }),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_space_operation_unsupported"));
    assert!(backend.calls_for("browser.browser_prepare").is_empty());
    manager
        .browser_call(
            "a",
            None,
            "browser_prepare",
            prepare(false),
            None,
            None::<fn() -> std::future::Ready<Result<(), ComputerError>>>,
        )
        .await
        .unwrap();
    assert_eq!(backend.calls_for("browser.browser_prepare").len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn matching_turn_release_removal_and_disposal_clear_only_task_bookkeeping() {
    let Reserved {
        manager, backend, ..
    } = setup().await;
    manager.space_broker().release("a", None);
    let task = CuaComputerTask {
        thread_id: "a".into(),
        turn_id: Some("turn-a".into()),
        label: None,
    };
    let turn = owner(Some("turn-a"));
    with_computer_task(
        task,
        manager.space_broker().reserve(&turn, 2, &[2], None, None),
    )
    .await
    .unwrap();
    manager
        .release_desktop_control("a", Some("old-turn"))
        .await
        .unwrap();
    assert!(manager.space_broker().reservation_for(&turn).is_some());
    manager
        .release_desktop_control("a", Some("turn-a"))
        .await
        .unwrap();
    assert!(manager.space_broker().reservation_for(&turn).is_none());
    manager
        .space_broker()
        .reserve(&turn, 2, &[2], None, None)
        .await
        .unwrap();
    manager.handle_thread_removed("a").await;
    assert!(manager.space_broker().reservation_for(&turn).is_none());
    assert!(backend.calls_for("raiseWindow").is_empty());
    manager.dispose().await;
}
