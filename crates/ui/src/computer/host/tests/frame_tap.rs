use super::*;
use crate::computer::host::ComputerFrameTapHost;
use zeron_proto::computer::CuaPreviewTarget;

#[derive(Default)]
struct RecordingTap {
    updates: Mutex<Vec<CuaPreviewTarget>>,
}

#[async_trait::async_trait(?Send)]
impl ComputerFrameTapHost for RecordingTap {
    fn update(&self, target: CuaPreviewTarget) -> Result<(), String> {
        self.updates.lock().unwrap().push(target);
        Ok(())
    }
    async fn end_task(&self, _task: CuaComputerTask) -> Result<(), String> {
        Ok(())
    }
    async fn stop(&self) -> Result<(), String> {
        Ok(())
    }
    async fn dispose(&self) -> Result<(), String> {
        Ok(())
    }
}

impl RecordingTap {
    fn targets(&self) -> Vec<(u32, u32)> {
        self.updates
            .lock()
            .unwrap()
            .iter()
            .map(|t| (t.pid, t.window_id))
            .collect()
    }

    async fn wait_for_update(&self) -> Vec<(u32, u32)> {
        for _ in 0..300 {
            if !self.updates.lock().unwrap().is_empty() {
                break;
            }
            sleep_ms(10).await;
        }
        self.targets()
    }
}

fn owner() -> CuaComputerTask {
    task("thread", Some("turn"))
}

fn calculator() -> Value {
    json!({
        "pid": 101,
        "window_id": 202,
        "app_name": "Calculator",
        "title": "Calculator",
        "bounds": { "x": 0, "y": 0, "width": 400, "height": 600 },
        "is_on_screen": true,
    })
}

fn tapped(driver: Driver) -> (Fixture, Arc<RecordingTap>) {
    let tap = Arc::new(RecordingTap::default());
    let host_tap = tap.clone();
    let f = fixture_with(driver, |o| o.frame_tap = Some(host_tap));
    (f, tap)
}

fn with_windows() -> Driver {
    Driver {
        list_windows: Some(vec![calculator()]),
        ..Driver::default()
    }
}

#[tokio::test]
async fn points_the_tap_at_the_launched_apps_main_window() {
    let (f, tap) = tapped(with_windows());
    let launched = f
        .send(
            call("launch_app")
                .task(&owner())
                .args(json!({ "name": "Calculator" })),
        )
        .await;
    assert!(launched.ok);
    assert_eq!(tap.wait_for_update().await, [(101, 202)]);
}

#[tokio::test]
async fn matches_bundle_ids_by_their_tail_component() {
    let (f, tap) = tapped(with_windows());
    f.send(
        call("launch_app")
            .task(&owner())
            .args(json!({ "bundle_id": "com.apple.Calculator" })),
    )
    .await;
    assert_eq!(tap.wait_for_update().await, [(101, 202)]);
}

#[tokio::test]
async fn stays_quiet_when_no_on_screen_window_matches() {
    let (f, tap) = tapped(with_windows());
    f.send(
        call("launch_app")
            .task(&owner())
            .args(json!({ "name": "TextEdit" })),
    )
    .await;
    sleep_ms(200).await;
    assert!(tap.targets().is_empty());
}

#[tokio::test]
async fn skips_the_prime_for_ended_tasks() {
    let (f, tap) = tapped(with_windows());
    f.send(method(ComputerHostMethod::EndTask).task(&owner()))
        .await;
    f.send(
        call("launch_app")
            .task(&owner())
            .args(json!({ "name": "Calculator" })),
    )
    .await;
    sleep_ms(200).await;
    assert!(tap.targets().is_empty());
}

#[tokio::test]
async fn points_the_tap_at_the_window_a_browser_bind_call_names() {
    let (f, tap) = tapped(Driver::default());
    let bound = f
        .send(
            call("get_browser_state")
                .task(&owner())
                .args(json!({ "pid": 101, "window_id": 202 })),
        )
        .await;
    assert!(bound.ok);
    assert_eq!(tap.wait_for_update().await, [(101, 202)]);
}

#[tokio::test]
async fn keeps_the_tap_parked_on_target_id_only_browser_calls() {
    let (f, tap) = tapped(Driver::default());
    let navigated = f
        .send(call("browser_navigate").task(&owner()).args(
            json!({ "target_id": "cua:1:2", "tab_id": "tab-1", "url": "https://example.test" }),
        ))
        .await;
    assert!(navigated.ok);
    sleep_ms(200).await;
    assert!(tap.targets().is_empty());
}

#[tokio::test]
async fn a_refused_bind_call_proves_nothing_and_leaves_the_tap_parked() {
    let (f, tap) = tapped(Driver {
        browser_refusal: true,
        ..Driver::default()
    });
    let refused = f
        .send(
            call("get_browser_state")
                .task(&owner())
                .args(json!({ "pid": 101, "window_id": 202 })),
        )
        .await;
    assert!(refused.ok);
    sleep_ms(200).await;
    assert!(tap.targets().is_empty());
}
