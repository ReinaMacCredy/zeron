use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext};

use super::{TaskCenter, TaskCenterEvent, TasksEvent, TasksPage};
use crate::state::AppState;
use crate::theme::Theme;

struct Uploads;

#[async_trait::async_trait]
impl zeron_rpc::RpcService for Uploads {
    async fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<zeron_rpc::RpcReply, zeron_rpc::RpcError> {
        let reply = match method {
            zeron_rpc::methods::UPLOAD_COMMIT => serde_json::json!({
                "path": format!("/uploads/{}", params["fileName"].as_str().unwrap_or_default()),
            }),
            _ => serde_json::json!({}),
        };
        zeron_rpc::RpcReply::value(&reply)
    }
}

struct Lab {
    _dir: tempfile::TempDir,
    png: PathBuf,
    text: PathBuf,
}

fn lab(cx: &mut TestAppContext) -> (Lab, Entity<AppState>) {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        gpui_base::init(cx);
        cx.set_global(Theme::dark());
        let mut spine = crate::settings::UiSettings::default();
        spine.tree_view = crate::settings::TreeView::Spine;
        crate::settings::init(spine, dir.path(), cx);
    });
    cx.executor().allow_parking();
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::new(2, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png_path = dir.path().join("shot.png");
    std::fs::write(&png_path, png.into_inner()).unwrap();
    let text = dir.path().join("notes.txt");
    std::fs::write(&text, "not an image").unwrap();
    let state = cx.new(|_| AppState::new());
    state.update(cx, |state, _| {
        state.set_test_engine(crate::state::EngineHandle::from_test_client(
            zeron_rpc::memory_client(Arc::new(Uploads)),
        ));
    });
    (
        Lab {
            _dir: dir,
            png: png_path,
            text,
        },
        state,
    )
}

fn settle(cx: &mut VisualTestContext, done: impl Fn(&mut VisualTestContext) -> bool) {
    for _ in 0..200 {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("condition never settled");
}

fn drop_on(cx: &mut VisualTestContext, selector: &'static str, paths: Vec<PathBuf>) {
    let to = cx.debug_bounds(selector).unwrap().center();
    cx.simulate_event(gpui::FileDropEvent::Entered {
        position: to,
        paths: gpui::ExternalPaths(paths.into()),
    });
    cx.update(|window, cx| window.draw(cx).clear());
    cx.simulate_event(gpui::FileDropEvent::Pending { position: to });
    cx.update(|window, cx| window.draw(cx).clear());
    cx.simulate_event(gpui::FileDropEvent::Submit { position: to });
    cx.run_until_parked();
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = cx.debug_bounds(selector).unwrap().center();
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
}

fn mission(state: &Entity<AppState>, cx: &mut TestAppContext) {
    state.update(cx, |state, _| {
        state.tasks = vec![
            serde_json::from_value(serde_json::json!({
                "id": "orch", "title": "Build it", "orderKey": "a", "chatId": "lead",
                "mode": "orchestrated",
            }))
            .unwrap(),
        ];
        state.chats = vec![
            serde_json::from_value(serde_json::json!({
                "id": "lead", "title": "lead", "deviceId": "mac", "archived": false,
                "createdAt": "2026-10-18T00:00:00Z",
            }))
            .unwrap(),
        ];
        state.task_steps = vec![
            serde_json::from_value(serde_json::json!({
                "id": "s1", "taskId": "orch", "title": "s1", "orderKey": "1",
            }))
            .unwrap(),
        ];
    });
}

fn edit_step(cx: &mut VisualTestContext) {
    let at = cx.debug_bounds("task-center-step-s1").unwrap().center();
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
    click(cx, "task-center-menu-edit");
    std::thread::sleep(crate::motion::MENU_OUT.total());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert!(cx.debug_bounds("task-center-editor").is_some());
}

#[gpui::test]
fn dropped_and_picked_images_reach_the_step_edit_card_and_the_plan_edit(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _guard = runtime.enter();
    let (lab, state) = lab(cx);
    mission(&state, cx);
    let (center, cx) = cx.add_window_view(|_, cx| TaskCenter::new(state, "orch".into(), cx));
    cx.update(|_, cx| crate::motion::set_reduced_motion(cx, true));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let sink = sent.clone();
    cx.update(|_, cx| {
        cx.subscribe(&center, move |_, event: &TaskCenterEvent, _| {
            if let TaskCenterEvent::EditPlan { edit, .. } = event {
                sink.borrow_mut().push(edit.steps.clone());
            }
        })
        .detach();
    });

    edit_step(cx);
    drop_on(
        cx,
        "task-center-editor",
        vec![lab.text.clone(), lab.png.clone()],
    );
    settle(cx, |cx| {
        cx.debug_bounds("task-center-edit-image-0").is_some()
    });
    assert!(
        cx.debug_bounds("task-center-edit-image-1").is_none(),
        "the text file is refused"
    );

    click(cx, "task-center-edit-add-image");
    assert!(cx.did_prompt_for_paths());
    let png = lab.png.clone();
    cx.simulate_path_prompt_response(move |options| {
        assert!(options.files && options.multiple && !options.directories);
        Some(vec![png])
    });
    settle(cx, |cx| {
        cx.debug_bounds("task-center-edit-image-1").is_some()
    });

    click(cx, "task-center-edit-save");
    assert!(cx.debug_bounds("task-center-spec-s1-image-1").is_some());
    click(cx, "task-center-send");
    let sent = sent.borrow();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0][0].images,
        ["/uploads/shot.png", "/uploads/shot.png"]
    );
}

#[gpui::test]
fn dropped_and_picked_images_go_with_a_task_from_the_add_input(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _guard = runtime.enter();
    let (lab, state) = lab(cx);
    let (page, cx) = cx.add_window_view(|_, cx| TasksPage::new(state, cx));
    cx.update(|_, cx| crate::motion::set_reduced_motion(cx, true));
    let created = Rc::new(RefCell::new(Vec::new()));
    let sink = created.clone();
    cx.update(|_, cx| {
        cx.subscribe(&page, move |_, event: &TasksEvent, _| {
            if let TasksEvent::Create { title, images, .. } = event {
                sink.borrow_mut().push((title.clone(), images.clone()));
            }
        })
        .detach();
    });
    cx.run_until_parked();

    drop_on(cx, "tasks-add-row", vec![lab.png.clone(), lab.text.clone()]);
    settle(cx, |cx| cx.debug_bounds("tasks-add-image-0").is_some());
    assert!(cx.debug_bounds("tasks-add-image-1").is_none());

    click(cx, "tasks-add-image");
    assert!(cx.did_prompt_for_paths());
    let png = lab.png.clone();
    cx.simulate_path_prompt_response(move |_| Some(vec![png]));
    settle(cx, |cx| cx.debug_bounds("tasks-add-image-1").is_some());

    page.update_in(cx, |page, window, cx| {
        page.add
            .update(cx, |input, cx| input.set_text("Ship it", cx));
        page.submit_add(false, Some(window), cx);
    });
    cx.run_until_parked();
    assert_eq!(
        *created.borrow(),
        [(
            "Ship it".to_string(),
            vec!["/uploads/shot.png".to_string(), "/uploads/shot.png".into()]
        )]
    );
    assert!(cx.debug_bounds("tasks-add-tray").is_none());
}

#[gpui::test]
fn the_steps_window_button_closes_its_own_window(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _guard = runtime.enter();
    let (_lab, state) = lab(cx);
    mission(&state, cx);
    state.update(cx, |state, _| state.tasks[0].plan_pending = true);
    let (_center, cx) = cx.add_window_view(|_, cx| TaskCenter::new(state, "orch".into(), cx));
    cx.update(|_, cx| crate::motion::set_reduced_motion(cx, true));
    cx.run_until_parked();
    click(cx, "task-center-pop-waiting");
    assert!(cx.debug_bounds("task-center-pop").is_some());
    click(cx, "task-center-pop-waiting");
    assert!(cx.debug_bounds("task-center-pop").is_none());
    click(cx, "task-center-pop-waiting");
    assert!(cx.debug_bounds("task-center-pop").is_some());
}

#[gpui::test]
fn an_idle_tasks_page_stops_asking_for_frames(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _guard = runtime.enter();
    let (_lab, state) = lab(cx);
    mission(&state, cx);
    let (_page, cx) = cx.add_window_view(|_, cx| TasksPage::new(state, cx));
    cx.update(|_, cx| crate::motion::set_reduced_motion(cx, true));
    cx.run_until_parked();
    let frames: Vec<usize> = (0..6)
        .map(|_| {
            cx.update(|window, cx| window.draw(cx).clear());
            let queued = cx.update(|window, cx| window.simulate_next_frame(cx));
            cx.run_until_parked();
            queued
        })
        .collect();
    assert_eq!(frames[1..], [0; 5], "{frames:?}");
}
