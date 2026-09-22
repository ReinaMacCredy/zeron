use super::*;
use crate::computer::host::{ComputerShieldEngagement, ComputerShieldHost};
use zeron_proto::computer::CuaShieldFrame;

type ShieldCall = (
    &'static str,
    Option<ComputerShieldEngagement>,
    Option<CuaComputerTask>,
);

#[derive(Default)]
struct RecordingShield {
    calls: Mutex<Vec<ShieldCall>>,
}

impl RecordingShield {
    fn methods(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().iter().map(|c| c.0).collect()
    }

    fn record(
        &self,
        method: &'static str,
        request: Option<ComputerShieldEngagement>,
        task: Option<CuaComputerTask>,
    ) {
        self.calls.lock().unwrap().push((method, request, task));
    }
}

#[async_trait::async_trait(?Send)]
impl ComputerShieldHost for RecordingShield {
    async fn engage(
        &self,
        request: ComputerShieldEngagement,
        task: Option<CuaComputerTask>,
    ) -> Result<(), String> {
        self.record("engage", Some(request), task);
        Ok(())
    }
    async fn release(&self, _shield_id: String) -> Result<(), String> {
        self.record("release", None, None);
        Ok(())
    }
    async fn release_all(&self) -> Result<u64, String> {
        self.record("release_all", None, None);
        Ok(self.methods().iter().filter(|m| **m == "engage").count() as u64)
    }
    async fn end_task(&self, task: CuaComputerTask) -> Result<(), String> {
        self.record("end_task", None, Some(task));
        Ok(())
    }
    async fn stop(&self) -> Result<(), String> {
        self.record("stop", None, None);
        Ok(())
    }
    async fn dispose(&self) -> Result<(), String> {
        self.record("dispose", None, None);
        Ok(())
    }
}

fn owner() -> CuaComputerTask {
    task("thread", Some("turn"))
}

fn engage_args() -> Value {
    json!({
        "action": "engage",
        "shield_id": "shield-abc123",
        "frame": { "x": 100, "y": 50, "width": 400, "height": 300 },
        "window_id": 4242,
        "pid": 777,
        "label": "Zeron activating Calculator",
    })
}

fn shield_call(args: Value) -> ComputerHostCall {
    method(ComputerHostMethod::Shield).args(args)
}

fn shielded() -> (Fixture, Arc<RecordingShield>) {
    let shield = Arc::new(RecordingShield::default());
    let host_shield = shield.clone();
    let f = fixture_with(Driver::default(), |o| o.shield = Some(host_shield));
    (f, shield)
}

#[tokio::test]
async fn routes_engage_to_the_shield_host_with_parsed_args_and_task_attribution() {
    let (f, shield) = shielded();
    let reply = f.send(shield_call(engage_args()).task(&owner())).await;
    assert!(reply.ok);
    assert_eq!(extra(&reply, "engaged"), Some(json!(true)));
    assert_eq!(extra(&reply, "shield_id"), Some(json!("shield-abc123")));
    let calls = shield.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "engage");
    assert_eq!(
        calls[0].1,
        Some(ComputerShieldEngagement {
            shield_id: "shield-abc123".into(),
            frame: CuaShieldFrame {
                x: 100.0,
                y: 50.0,
                width: 400.0,
                height: 300.0,
            },
            window_id: 4242,
            pid: 777,
            label: Some("Zeron activating Calculator".into()),
        })
    );
    assert_eq!(calls[0].2, Some(owner()));
    drop(calls);
    assert!(f.events().is_empty());
}

#[tokio::test]
async fn refuses_engage_when_no_shield_surface_is_configured() {
    let f = fixture(Driver::default());
    let reply = f.send(shield_call(engage_args()).task(&owner())).await;
    assert_not_dispatched(&reply);
    assert!(reply.error.as_deref().unwrap().contains("not available"));
}

#[tokio::test]
async fn refuses_engage_while_the_desktop_is_paused_but_still_accepts_release() {
    let (f, shield) = shielded();
    f.host.pause_desktop("screen-lock").await.unwrap();
    let reply = f.send(shield_call(engage_args()).task(&owner())).await;
    assert!(!reply.ok);
    assert!(reply.error.as_deref().unwrap().contains("paused"));
    let released = f
        .send(shield_call(
            json!({ "action": "release", "shield_id": "shield-abc123" }),
        ))
        .await;
    assert!(released.ok);
    assert_eq!(shield.methods(), ["stop", "release"]);
    f.host.resume_desktop("screen-lock");
}

#[tokio::test]
async fn rejects_malformed_shield_args_before_touching_the_surface() {
    let (f, shield) = shielded();
    for args in [
        json!({ "action": "engage", "shield_id": "bad id with spaces" }),
        json!({ "action": "engage", "shield_id": "shield-1", "frame": { "x": 0, "y": 0, "width": -4, "height": 4 }, "window_id": 1, "pid": 1 }),
        json!({ "action": "engage", "shield_id": "shield-1", "frame": { "x": 0, "y": 0, "width": 4, "height": 4 }, "window_id": 0, "pid": 1 }),
        json!({ "action": "release" }),
        json!({ "action": "detonate" }),
        json!("engage"),
    ] {
        let reply = f.send(shield_call(args.clone()).task(&owner())).await;
        assert_not_dispatched(&reply);
    }
    assert!(shield.methods().is_empty());
}

#[tokio::test]
async fn release_all_is_the_forced_release_path_and_reports_the_live_count() {
    let (f, shield) = shielded();
    f.send(shield_call(engage_args()).task(&owner())).await;
    let reply = f
        .send(shield_call(json!({ "action": "release_all" })))
        .await;
    assert!(reply.ok);
    assert_eq!(extra(&reply, "released"), Some(json!(1)));
    assert_eq!(shield.methods(), ["engage", "release_all"]);
}

#[tokio::test]
async fn end_task_releases_the_tasks_shields() {
    let (f, shield) = shielded();
    f.send(method(ComputerHostMethod::EndTask).task(&owner()))
        .await;
    let calls = shield.calls.lock().unwrap();
    assert_eq!(calls.iter().map(|c| c.0).collect::<Vec<_>>(), ["end_task"]);
    assert_eq!(calls[0].2, Some(owner()));
}

#[tokio::test]
async fn stop_and_dispose_release_the_whole_shield_surface() {
    let (f, shield) = shielded();
    f.host.stop().await.unwrap();
    assert!(shield.methods().contains(&"stop"));
    f.host.dispose().await.unwrap();
    assert!(shield.methods().contains(&"dispose"));
}
