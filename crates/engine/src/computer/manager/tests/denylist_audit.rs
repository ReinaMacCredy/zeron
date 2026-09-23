use super::*;

use zeron_proto::computer::ComputerAuditEffect;

use super::super::{ComputerClickGesture, ComputerStateRequest};
use crate::computer::audit_log::{COMPUTER_AUDIT_FILE_NAME, ComputerAuditEntry};
use crate::computer::backend::ComputerCaptureRequest;

const DENIED_WINDOW_ID: &str = "fake-password-manager";

fn denied_window() -> ComputerWindow {
    ComputerWindow {
        app_name: Some("1Password".into()),
        pid: Some(2_001),
        ..window(
            DENIED_WINDOW_ID,
            "1Password",
            Some(rect(200.0, 100.0, 800.0, 600.0)),
        )
    }
}

fn denied_fixture() -> Arc<Scripted> {
    Scripted::new(FakeComputerBackendOptions {
        windows: Some(vec![
            ComputerWindow {
                app_name: Some("org.kde.kate".into()),
                pid: Some(1_000),
                focused: true,
                ..window(
                    "fake-editor",
                    "Editor",
                    Some(rect(1_100.0, 40.0, 700.0, 600.0)),
                )
            },
            denied_window(),
        ]),
        apps: Some(vec![
            ComputerApp {
                window_count: Some(1),
                ..app(1_000, "Kate", "org.kde.kate", true)
            },
            ComputerApp {
                window_count: Some(1),
                ..app(2_001, "1Password", "com.1password.1password", false)
            },
        ]),
        ..FakeComputerBackendOptions::default()
    })
}

fn refused<T: std::fmt::Debug>(result: Result<T, ComputerError>) {
    let error = result.unwrap_err();
    assert_eq!(code(&error), Some("computer_denylist_refused"), "{error}");
}

async fn launch(manager: &ComputerManager, app: &str) -> Result<(), ComputerError> {
    manager
        .launch_app(
            Some("thread-1"),
            app,
            &[],
            0,
            ComputerLaunchOptions::default(),
        )
        .await
        .map(|_| ())
}

async fn click(
    manager: &ComputerManager,
    thread_id: Option<&str>,
    target: ComputerTarget,
) -> Result<zeron_proto::computer::ComputerActionResult, ComputerError> {
    manager
        .click(thread_id, target, &[], ComputerClickGesture::default())
        .await
}

#[tokio::test]
async fn refuses_to_drive_a_password_manager_the_denylist_has_no_consent_override() {
    let Built { manager, .. } = settled(denied_fixture());
    refused(launch(&manager, "1Password").await);
    refused(launch(&manager, "com.bitwarden.desktop").await);
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_to_launch_a_denylisted_app() {
    let Built {
        manager, backend, ..
    } = settled(denied_fixture());
    refused(launch(&manager, "1Password").await);
    refused(launch(&manager, "Keychain Access").await);
    assert!(backend.calls_for("launchApp").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_a_bare_point_click_inside_a_denylisted_window() {
    let Built {
        manager, backend, ..
    } = settled(denied_fixture());
    refused(click(&manager, Some("thread-1"), point(400.0, 300.0)).await);
    assert!(backend.calls_for("click").is_empty());
    let result = click(&manager, Some("thread-1"), point(1_200.0, 200.0))
        .await
        .unwrap();
    assert_eq!(result.action, "computer_click");
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_a_window_scoped_click_move_frame_and_keystroke_on_the_denied_window() {
    let Built {
        manager, backend, ..
    } = settled(denied_fixture());
    refused(
        click(
            &manager,
            Some("thread-1"),
            scoped(400.0, 300.0, DENIED_WINDOW_ID),
        )
        .await,
    );
    refused(
        manager
            .set_window_frame(
                Some("thread-1"),
                DENIED_WINDOW_ID,
                rect(0.0, 0.0, 400.0, 400.0),
            )
            .await,
    );
    refused(
        manager
            .type_text(Some("thread-1"), "master password", Some(DENIED_WINDOW_ID))
            .await,
    );
    refused(manager.kill_app(Some("thread-1"), DENIED_WINDOW_ID).await);
    assert!(backend.calls_for("click").is_empty());
    assert!(backend.calls_for("setWindowFrame").is_empty());
    assert!(backend.calls_for("typeText").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_a_scoped_state_read_of_a_denylisted_window_but_still_lists_it() {
    let Built { manager, .. } = settled(denied_fixture());
    let listed = manager.list_windows().await.unwrap();
    assert!(
        listed
            .windows
            .iter()
            .any(|window| window.id == DENIED_WINDOW_ID)
    );
    refused(
        manager
            .get_state(ComputerStateRequest {
                window_id: Some(DENIED_WINDOW_ID.into()),
                ..ComputerStateRequest::default()
            })
            .await,
    );
    refused(
        manager
            .capture_screenshot(ComputerCaptureRequest::Window {
                window_id: DENIED_WINDOW_ID.into(),
                max_dimension: None,
            })
            .await,
    );
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_a_workspace_screenshot_while_a_denylisted_window_is_visible() {
    let Built { manager, .. } = settled(denied_fixture());
    refused(
        manager
            .get_state(ComputerStateRequest {
                include_screenshot: Some(true),
                ..ComputerStateRequest::default()
            })
            .await,
    );
    let state = manager
        .get_state(ComputerStateRequest::default())
        .await
        .unwrap();
    assert_eq!(state.computer_id, "desktop");
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_semantic_targeting_into_a_denied_window_and_while_one_is_visible() {
    let Built { manager, .. } = settled(denied_fixture());
    refused(
        click(
            &manager,
            Some("thread-1"),
            ComputerTarget {
                window_id: Some(DENIED_WINDOW_ID.into()),
                ..labelled("Unlock", None)
            },
        )
        .await,
    );
    refused(click(&manager, Some("thread-1"), labelled("Unlock", None)).await);
    manager.dispose().await;
}

#[tokio::test]
async fn resolves_a_windows_denied_app_through_its_pid_when_the_name_alone_is_clean() {
    let backend = Scripted::new(FakeComputerBackendOptions {
        windows: Some(vec![ComputerWindow {
            app_name: Some("Notes".into()),
            pid: Some(3_001),
            ..window(
                "fake-renamed",
                "Notes",
                Some(rect(100.0, 100.0, 600.0, 500.0)),
            )
        }]),
        apps: Some(vec![ComputerApp {
            window_count: Some(1),
            ..app(3_001, "Notes", "com.bitwarden.desktop", false)
        }]),
        ..FakeComputerBackendOptions::default()
    });
    let Built { manager, .. } = settled(backend);
    refused(click(&manager, Some("thread-1"), point(300.0, 300.0)).await);
    refused(
        manager
            .get_state(ComputerStateRequest {
                window_id: Some("fake-renamed".into()),
                ..ComputerStateRequest::default()
            })
            .await,
    );
    manager.dispose().await;
}

#[tokio::test]
async fn leaves_the_panes_own_input_exempt_from_the_input_denylist() {
    let Built { manager, .. } = settled(denied_fixture());
    let result = click(&manager, None, point(400.0, 300.0)).await.unwrap();
    assert_eq!(result.action, "computer_click");
    manager.dispose().await;
}

fn audited(dir: &tempfile::TempDir) -> Built {
    build(
        Scripted::default_fake(),
        Setup {
            action_settle_ms: Some(0),
            data_dir: Some(dir.path().to_path_buf()),
            ..Setup::default()
        },
    )
}

fn refused_click(thread_id: &str) -> ComputerAuditEntry {
    ComputerAuditEntry {
        thread_id: Some(thread_id.into()),
        args: Some(json!({ "x": 1, "y": 1 }).as_object().unwrap().clone()),
        code: Some("computer_control_revoked".into()),
        ..ComputerAuditEntry::new("computer_click", ComputerAuditEffect::Refused)
    }
}

#[tokio::test]
async fn a_removed_thread_records_nothing_even_for_the_refusal_that_stopped_it() {
    let dir = tempfile::tempdir().unwrap();
    let Built { manager, .. } = audited(&dir);
    let thread_id = "disabled-thread";
    manager.handle_thread_removed(thread_id).await;
    manager.record_computer_audit(&refused_click(thread_id));
    manager.dispose().await;
    let path = dir.path().join(COMPUTER_AUDIT_FILE_NAME);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
}

#[tokio::test]
async fn a_removed_thread_records_nothing_at_all_no_lifecycle_row_survives_the_drop() {
    let dir = tempfile::tempdir().unwrap();
    let Built { manager, .. } = audited(&dir);
    let thread_id = "disabled-thread";
    manager.handle_thread_removed(thread_id).await;
    manager.record_computer_audit(&refused_click(thread_id));
    manager.dispose().await;
    let path = dir.path().join(COMPUTER_AUDIT_FILE_NAME);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
}

#[tokio::test]
async fn records_through_the_manager_once_control_is_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let Built { manager, .. } = audited(&dir);
    manager.record_computer_audit(&ComputerAuditEntry {
        thread_id: Some("enabled-thread".into()),
        ..ComputerAuditEntry::new("computer_click", ComputerAuditEffect::Verified)
    });
    manager.dispose().await;
    let saved = std::fs::read_to_string(dir.path().join(COMPUTER_AUDIT_FILE_NAME)).unwrap();
    let lines: Vec<&str> = saved.trim().split('\n').collect();
    assert_eq!(lines.len(), 1);
    let entry: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(entry["effect"], json!("verified"));
}
