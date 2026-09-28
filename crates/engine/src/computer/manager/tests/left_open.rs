use super::*;
use crate::computer::manager::{ComputerLeftOpen, left_open_note};

type Listing = Arc<Mutex<Option<Vec<ComputerWindow>>>>;

fn listed(backend: &Scripted) -> Listing {
    let listing: Listing = Arc::default();
    let live = listing.clone();
    backend.hooks().list_windows = Some(Arc::new(move |fake, ()| {
        let live = live.clone();
        async move {
            let windows = live.lock().unwrap().clone();
            match windows {
                Some(windows) => Ok(windows),
                None => fake.list_windows().await,
            }
        }
        .boxed()
    }));
    listing
}

fn editor(id: &str, pid: u32, width: f64, height: f64) -> ComputerWindow {
    ComputerWindow {
        app_name: Some(if pid == 220 { "Editor B" } else { "Editor A" }.into()),
        pid: Some(pid),
        ..window(id, id, Some(rect(40.0, 40.0, width, height)))
    }
}

fn desktop_with(extra: Vec<ComputerWindow>) -> Vec<ComputerWindow> {
    let mut windows = vec![
        editor("editor-a", 110, 360.0, 300.0),
        editor("editor-b", 220, 360.0, 300.0),
        editor("editor-a-other", 110, 360.0, 300.0),
    ];
    windows.extend(extra);
    windows
}

async fn press(manager: &Arc<ComputerManager>, turn: &str) {
    manager
        .with_agent_activity(
            "a",
            || manager.press_key(Some("a"), "enter", Some("editor-a"), None),
            None,
            Some(turn),
            None,
        )
        .await
        .unwrap();
}

async fn setup() -> (Arc<ComputerManager>, Arc<Scripted>, Listing) {
    let backend = background_target_backend();
    let listing = listed(&backend);
    let Built { manager, .. } = manager(backend.clone());
    manager.list_windows().await.unwrap();
    (manager, backend, listing)
}

#[tokio::test]
async fn windows_opened_during_the_turn_and_still_open_are_reported_once() {
    let (manager, _, listing) = setup().await;
    press(&manager, "t1").await;
    *listing.lock().unwrap() = Some(desktop_with(vec![
        editor("editor-a-new", 110, 900.0, 700.0),
        editor("editor-a-new-2", 110, 900.0, 700.0),
    ]));
    let left = manager.left_open_windows("a", "t1").await;
    assert_eq!(
        left,
        vec![ComputerLeftOpen {
            app: "Editor A".into(),
            count: 2,
        }]
    );
    assert_eq!(left_open_note(&left[0]), "Left 2 Editor A windows open");
    assert_eq!(
        left_open_note(&ComputerLeftOpen {
            count: 1,
            ..left[0].clone()
        }),
        "Left 1 Editor A window open"
    );
    assert!(
        manager.left_open_windows("a", "t1").await.is_empty(),
        "a turn reports once"
    );
    manager.dispose().await;
}

#[tokio::test]
async fn pre_existing_windows_of_the_same_app_are_not_reported() {
    let (manager, _, listing) = setup().await;
    press(&manager, "t1").await;
    *listing.lock().unwrap() = Some(desktop_with(Vec::new()));
    assert!(manager.left_open_windows("a", "t1").await.is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn windows_the_agent_closed_are_not_reported() {
    let (manager, _, listing) = setup().await;
    press(&manager, "t1").await;
    let opened = desktop_with(vec![editor("editor-a-new", 110, 900.0, 700.0)]);
    *listing.lock().unwrap() = Some(opened.clone());
    manager.list_windows().await.unwrap();
    *listing.lock().unwrap() = Some(desktop_with(Vec::new()));
    assert!(
        manager.window_app_name("editor-a-new").is_some(),
        "the cache still holds the closed window"
    );
    assert!(
        manager.left_open_windows("a", "t1").await.is_empty(),
        "only a fresh read drops it"
    );
    manager.dispose().await;
}

#[tokio::test]
async fn a_window_opened_without_any_event_is_found_by_the_turn_end_read() {
    let (manager, _, listing) = setup().await;
    press(&manager, "t1").await;
    *listing.lock().unwrap() = Some(desktop_with(vec![editor(
        "editor-a-new",
        110,
        900.0,
        700.0,
    )]));
    assert_eq!(
        manager.window_app_name("editor-a-new"),
        None,
        "nothing refreshed the cache during the turn"
    );
    assert_eq!(manager.left_open_windows("a", "t1").await.len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn accessory_strips_and_other_apps_are_not_reported_but_minimized_and_other_space_windows_are()
 {
    let (manager, _, listing) = setup().await;
    press(&manager, "t1").await;
    let hidden = |id: &str, width, height| ComputerWindow {
        title: String::new(),
        visible: false,
        ..editor(id, 110, width, height)
    };
    let minimized = ComputerWindow {
        visible: false,
        minimized: true,
        ..editor("editor-a-min", 110, 900.0, 700.0)
    };
    let strip_with_spaces = ComputerWindow {
        minimized: true,
        ..hidden("strip-spaces", 1512.0, 33.0)
    };
    let other_space = ComputerWindow {
        visible: false,
        on_current_space: Some(false),
        ..editor("editor-a-space", 110, 900.0, 700.0)
    };
    *listing.lock().unwrap() = Some(desktop_with(vec![
        hidden("strip-1512", 1512.0, 33.0),
        hidden("strip-1920", 1920.0, 30.0),
        hidden("hidden-real", 900.0, 700.0),
        editor("stray-66x20", 110, 66.0, 20.0),
        editor("editor-b-new", 220, 900.0, 700.0),
        strip_with_spaces,
        minimized,
        other_space,
    ]));
    let left = manager.left_open_windows("a", "t1").await;
    assert_eq!(
        left,
        vec![ComputerLeftOpen {
            app: "Editor A".into(),
            count: 2,
        }],
        "the minimized window and the one on another Space; strips (even one the driver reads as minimized), the 66x20 stray, the hidden one and an app the agent never touched are left out"
    );
    manager.dispose().await;
}

#[tokio::test]
async fn another_turn_or_an_idle_thread_reports_nothing() {
    let (manager, _, listing) = setup().await;
    press(&manager, "t1").await;
    *listing.lock().unwrap() = Some(desktop_with(vec![editor(
        "editor-a-new",
        110,
        900.0,
        700.0,
    )]));
    assert!(manager.left_open_windows("a", "t0").await.is_empty());
    assert!(manager.left_open_windows("z", "t1").await.is_empty());
    manager.list_windows().await.unwrap();
    press(&manager, "t2").await;
    assert!(
        manager.left_open_windows("a", "t2").await.is_empty(),
        "a new turn takes a new baseline"
    );
    manager.dispose().await;
}

#[tokio::test]
async fn a_turn_with_no_listing_before_it_takes_the_baseline_from_its_first_read() {
    let backend = background_target_backend();
    let listing = listed(&backend);
    let Built { manager, .. } = manager(backend);
    press(&manager, "t1").await;
    *listing.lock().unwrap() = Some(desktop_with(vec![editor(
        "editor-a-new",
        110,
        900.0,
        700.0,
    )]));
    assert_eq!(manager.left_open_windows("a", "t1").await.len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn a_listing_from_before_the_turn_is_never_the_baseline() {
    let (manager, _, listing) = setup().await;
    *listing.lock().unwrap() = Some(desktop_with(vec![editor(
        "editor-a-user",
        110,
        900.0,
        700.0,
    )]));
    press(&manager, "t1").await;
    assert!(
        manager.left_open_windows("a", "t1").await.is_empty(),
        "a window the user opened before the turn is not the agent's"
    );
    manager.dispose().await;
}

#[tokio::test]
async fn a_failed_turn_end_read_reports_nothing() {
    let (manager, backend, _) = setup().await;
    press(&manager, "t1").await;
    backend.hooks().list_windows = Some(Arc::new(|_, ()| {
        async { Err(ComputerError::Other("host gone".into())) }.boxed()
    }));
    assert!(manager.left_open_windows("a", "t1").await.is_empty());
    manager.dispose().await;
}

#[test]
fn left_open_and_pause_labels_never_carry_a_bundle_id() {
    use crate::computer::manager::display_app_name;
    assert_eq!(
        display_app_name(Some("org.kde.konsole")).as_deref(),
        Some("Konsole")
    );
    assert_eq!(
        display_app_name(Some("com.mitchellh.ghostty")).as_deref(),
        Some("Ghostty")
    );
    assert_eq!(
        display_app_name(Some("/Applications/Safari.app")).as_deref(),
        Some("Safari")
    );
    assert_eq!(
        display_app_name(Some("Visual Studio Code")).as_deref(),
        Some("Visual Studio Code")
    );
    assert_eq!(display_app_name(Some("  ")), None);
    assert_eq!(display_app_name(None), None);
}
