//! Spaces sidebar: the space-filter dropdown (searchable, with "All projects"),
//! the filtered Sessions list, and the add-space palette (device
//! tabs + filtered folder browser).
//!
//! A space = a synced (device, folder) pair. Spaces stopped being a
//! navigation spine when tabs went device-local: the dropdown only FILTERS
//! the sidebar's session list (never the tab strip) and hosts space
//! management (add via the palette; rename/delete via row context menus).
//! Child module of `shell` so it renders straight off `Shell`'s private state.

use super::*;
use crate::pickers::{
    breadcrumbs, browser_rows, completion_prefix_len, folder_path_key, parent_path,
    same_folder_path,
};
use gpui::{FocusHandle, Window};
use std::collections::HashSet;
use zeron_proto::{
    ChatIndicator, Device, DriveEntry, DriveListing, FolderListing, HomeFolderMatch,
    HomeFolderSearchResult, SearchHomeFoldersRequest, Space,
};

/// Promote the user's ordered pins above the untouched activity projection.
/// Every unpinned id keeps exactly the relative order supplied by recency.
pub(super) fn project_pinned_first(recency_ids: &[String], pinned_ids: &[String]) -> Vec<String> {
    let active: HashSet<&str> = recency_ids.iter().map(String::as_str).collect();
    let pinned: HashSet<&str> = pinned_ids.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    pinned_ids
        .iter()
        .filter(|id| active.contains(id.as_str()))
        .chain(
            recency_ids
                .iter()
                .filter(|id| !pinned.contains(id.as_str())),
        )
        .filter(|id| seen.insert(id.as_str()))
        .cloned()
        .collect()
}

/// Move only the dragged pin. Every other pin, including hidden/archived pins,
/// keeps its relative order; no other position needs to be written.
pub(super) fn reorder_visible_pins(
    pinned_ids: &[String],
    visible_ids: &[String],
    from: usize,
    to: usize,
) -> Vec<String> {
    if from >= visible_ids.len() || to >= visible_ids.len() || from == to {
        return pinned_ids.to_vec();
    }

    let moved = &visible_ids[from];
    let anchor = &visible_ids[to];
    let mut result: Vec<_> = pinned_ids
        .iter()
        .filter(|id| *id != moved)
        .cloned()
        .collect();
    let Some(index) = result.iter().position(|id| id == anchor) else {
        return pinned_ids.to_vec();
    };
    result.insert(index + usize::from(from < to), moved.clone());
    result
}

/// Remove only ids absent from the workspace. Archived sessions remain known
/// so unarchiving restores their local pin and position.
pub(super) fn retain_known_pins(
    pinned_ids: &mut Vec<String>,
    known_chat_ids: &HashSet<String>,
) -> bool {
    let before = pinned_ids.len();
    let mut seen = HashSet::new();
    pinned_ids.retain(|id| known_chat_ids.contains(id) && seen.insert(id.clone()));
    pinned_ids.len() != before
}

/// Convert viewport coordinates to the first pinned row, below its disclosure.
fn pinned_session_pointer_y(pointer_y: f32, viewport_top: f32, scroll_top: f32) -> f32 {
    pointer_y - viewport_top + scroll_top
        - super::SIDEBAR_LIST_PAD_TOP
        - SIDEBAR_DISCLOSURE_HEADER_HEIGHT
        - SIDEBAR_DISCLOSURE_BODY_INSET
}

#[cfg(test)]
pub(super) fn pinned_session_drop_index(rel_y: f32, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let height = count as f32 * super::SIDEBAR_SESSION_SLOT - super::SIDEBAR_LIST_GAP;
    (rel_y >= 0.0 && rel_y <= height)
        .then(|| ((rel_y / super::SIDEBAR_SESSION_SLOT).floor() as usize).min(count - 1))
}

/// Hit testing shares the exact row metrics used by layout, including mixed PR rows.
pub(super) fn row_drop_index(y: f32, heights: &[f32], clamp: bool) -> Option<usize> {
    if heights.is_empty() {
        return None;
    }
    let total =
        heights.iter().sum::<f32>() + heights.len().saturating_sub(1) as f32 * SIDEBAR_LIST_GAP;
    if !clamp && !(0.0..=total).contains(&y) {
        return None;
    }
    let mut bottom = 0.0;
    for (index, height) in heights.iter().enumerate() {
        bottom += height + SIDEBAR_LIST_GAP;
        if y < bottom {
            return Some(index);
        }
    }
    Some(heights.len() - 1)
}

/// Keep a sidebar-wide drag physically bounded to the pinned section. The
/// strict drop helper above still identifies whether the pointer is actually
/// inside the section; this helper supplies the nearest valid pinned slot
/// while the pointer is over regular sessions.
#[cfg(test)]
pub(super) fn pinned_session_clamped_index(rel_y: f32, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(((rel_y.max(0.0) / super::SIDEBAR_SESSION_SLOT).floor() as usize).min(count - 1))
}

/// A drop can change pin membership or pinned order, never activity ordering.
fn sidebar_session_drop_pins(
    saved: &[String],
    visible: &[String],
    chat_id: &str,
    target: SidebarSessionDrop,
) -> Vec<String> {
    let mut next = saved.to_vec();
    match target {
        SidebarSessionDrop::Regular | SidebarSessionDrop::Section(_) => {
            next.retain(|id| id != chat_id)
        }
        SidebarSessionDrop::Pinned(index) => {
            if let Some(from) = visible.iter().position(|id| id == chat_id) {
                return reorder_visible_pins(saved, visible, from, index.min(visible.len() - 1));
            }
            if saved.iter().any(|id| id == chat_id) {
                return next;
            }
            let insertion = visible
                .get(index)
                .and_then(|anchor| next.iter().position(|id| id == anchor))
                .or_else(|| {
                    visible
                        .last()
                        .and_then(|anchor| next.iter().position(|id| id == anchor))
                        .map(|ix| ix + 1)
                })
                .unwrap_or(next.len());
            next.insert(insertion, chat_id.to_owned());
        }
    }
    next
}

/// Preview geometry only: regular ordering is never persisted by a drag.
fn sidebar_gap_offset(row: usize, source: Option<usize>, boundary: usize, height: f32) -> f32 {
    match source {
        Some(source) if row == source => 0.0,
        Some(source) if row < source && row >= boundary => height,
        Some(source) if row > source && row < boundary => -height,
        None if row >= boundary => height,
        _ => 0.0,
    }
}

pub(super) fn pinned_drag_scroll_delta(
    pointer_y: f32,
    viewport_top: f32,
    viewport_bottom: f32,
) -> f32 {
    if viewport_bottom <= viewport_top {
        return 0.0;
    }
    if pointer_y < viewport_top + super::SIDEBAR_DRAG_SCROLL_BAND {
        let penetration = ((viewport_top + super::SIDEBAR_DRAG_SCROLL_BAND - pointer_y)
            / super::SIDEBAR_DRAG_SCROLL_BAND)
            .clamp(0.0, 1.0);
        -super::SIDEBAR_DRAG_SCROLL_MAX * penetration
    } else if pointer_y > viewport_bottom - super::SIDEBAR_DRAG_SCROLL_BAND {
        let penetration = ((pointer_y - (viewport_bottom - super::SIDEBAR_DRAG_SCROLL_BAND))
            / super::SIDEBAR_DRAG_SCROLL_BAND)
            .clamp(0.0, 1.0);
        super::SIDEBAR_DRAG_SCROLL_MAX * penetration
    } else {
        0.0
    }
}

#[cfg(test)]
mod pinned_session_tests {
    fn pin_change(id: &str) -> zeron_proto::SidebarPinChange {
        zeron_proto::SidebarPinChange::Pin {
            session_id: id.into(),
            after: None,
            before: None,
        }
    }
    use super::{
        pinned_drag_scroll_delta, pinned_drag_scroll_step, pinned_drag_snapshot_is_valid,
        pinned_session_clamped_index, pinned_session_drop_index, project_pinned_first,
        reorder_visible_pins, retain_known_pins,
    };
    use std::collections::HashSet;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn pin_test_shell(
        cx: &mut gpui::TestAppContext,
        path: &std::path::Path,
    ) -> gpui::WindowHandle<super::Shell> {
        use super::*;
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
        });
        cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            let mut shell = Shell::new(
                state,
                EngineBootConfig {
                    data_dir: path.into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: zeron_proto::HarnessId::Mock,
                },
                cx,
            );
            shell.terminals_synced = true;
            shell
        })
    }

    fn remote_pin_state(state: &mut super::AppState, synced: bool, initialized: bool) {
        state.workspace_scope = Some(zeron_proto::WorkspaceScope::Synced);
        state.auth = Some(zeron_proto::AuthState::SignedIn {
            user: zeron_proto::UserProfile {
                id: "user".into(),
                email: "test@example.com".into(),
                name: None,
            },
            org_id: Some("org".into()),
        });
        state.sidebar_preferences.synced = synced;
        state.sidebar_preferences.initialized = initialized;
    }

    fn pin_test_chat(id: &str) -> zeron_proto::Chat {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": id, "deviceId": "local", "archived": false,
            "createdAt": chrono::Utc::now(),
        }))
        .unwrap()
    }

    fn pin_test_engine() -> (
        crate::state::EngineHandle,
        tokio::sync::mpsc::Receiver<String>,
        tokio::sync::mpsc::Sender<String>,
    ) {
        let (out, requests) = tokio::sync::mpsc::channel(16);
        let (replies, inbound) = tokio::sync::mpsc::channel(16);
        (
            crate::state::EngineHandle::from_test_client(zeron_rpc::RpcClient::new(out, inbound)),
            requests,
            replies,
        )
    }

    fn pin_snapshot(revision: u64, pins: &[&str]) -> zeron_proto::SidebarPreferencesState {
        zeron_proto::SidebarPreferencesState {
            sections: vec![],
            revision,
            synced: true,
            initialized: true,
            pinned_session_ids: ids(pins),
        }
    }

    fn deliver_pin_rpc_reply(
        runtime: &tokio::runtime::Runtime,
        replies: &tokio::sync::mpsc::Sender<String>,
        reply: serde_json::Value,
    ) {
        // The current-thread reactor drains the actual RpcClient reader before
        // GPUI resumes. No wall-clock sleeps or hand-invoked completion handlers.
        runtime.block_on(async {
            replies.send(reply.to_string()).await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while replies.capacity() < replies.max_capacity() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        });
    }

    #[gpui::test]
    fn sidebar_rpc_replies_dispatch_each_queued_drop_once_and_recover_rejection(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let (engine, mut requests, replies) = pin_test_engine();
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, _| {
                    remote_pin_state(state, true, true);
                    state.set_test_engine(engine);
                });
                let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
                shell.apply_sidebar_pin_change(key.clone(), pin_change("first"), cx);
                shell.apply_sidebar_pin_change(key, pin_change("second"), cx);
            })
            .unwrap();
        cx.run_until_parked();
        let first: serde_json::Value = serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(
            first["params"]["change"],
            serde_json::to_value(pin_change("first")).unwrap()
        );
        assert!(requests.try_recv().is_err());
        deliver_pin_rpc_reply(
            &runtime,
            &replies,
            serde_json::json!({"id": first["id"], "ok": {"ok": true, "sidebarPreferences": pin_snapshot(1, &["first"])}}),
        );
        cx.run_until_parked();
        let second: serde_json::Value =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(
            second["params"]["change"],
            serde_json::to_value(pin_change("second")).unwrap()
        );
        assert!(requests.try_recv().is_err());
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.active_sidebar_pins(cx), ids(&["first", "second"]));
                assert_eq!(
                    shell.state.read(cx).sidebar_preferences.pinned_session_ids,
                    ids(&["first"])
                );
                shell.state.update(cx, |state, _| {
                    state.apply_sidebar_preferences(pin_snapshot(3, &["remote"]));
                });
            })
            .unwrap();
        deliver_pin_rpc_reply(
            &runtime,
            &replies,
            serde_json::json!({"id": second["id"], "err": "rejected by test registry"}),
        );
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.active_sidebar_pins(cx), ids(&["remote"]));
                assert!(shell.sidebar_pin_write.is_none());
                assert!(
                    shell
                        .sidebar_notice
                        .as_deref()
                        .unwrap()
                        .contains("rejected by test registry")
                );
            })
            .unwrap();
        assert!(requests.try_recv().is_err());
    }

    #[gpui::test]
    fn sidebar_rpc_timeout_blocks_overtaking_until_the_late_reply_arrives(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let (engine, mut requests, replies) = pin_test_engine();
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, _| {
                    remote_pin_state(state, true, true);
                    state.set_test_engine(engine);
                    state.sidebar_preferences = pin_snapshot(1, &["confirmed"]);
                });
                let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
                shell.apply_sidebar_pin_change(key.clone(), pin_change("slow"), cx);
                shell.apply_sidebar_pin_change(key, pin_change("queued"), cx);
            })
            .unwrap();
        cx.run_until_parked();
        let request: serde_json::Value =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(21));
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert!(shell.sidebar_pin_write.as_ref().unwrap().unconfirmed);
                assert_eq!(shell.active_sidebar_pins(cx), ids(&["confirmed"]));
                let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
                assert!(!shell.apply_sidebar_pin_change(key, pin_change("overtaking"), cx));
            })
            .unwrap();
        assert!(
            requests.try_recv().is_err(),
            "queued and fresh drops must not overtake the slow write"
        );
        deliver_pin_rpc_reply(
            &runtime,
            &replies,
            serde_json::json!({"id": request["id"], "ok": {"ok": true, "sidebarPreferences": pin_snapshot(2, &["slow"])}}),
        );
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert!(shell.sidebar_pin_write.is_none());
                assert_eq!(shell.active_sidebar_pins(cx), ids(&["slow"]));
                assert!(
                    shell.sidebar_notice.is_none(),
                    "late success must clear the waiting notice"
                );
                let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
                assert!(shell.apply_sidebar_pin_change(key, pin_change("after-confirmation"), cx));
            })
            .unwrap();
        cx.run_until_parked();
        let next: serde_json::Value = serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(
            next["params"]["change"],
            serde_json::to_value(pin_change("after-confirmation")).unwrap()
        );
        assert!(requests.try_recv().is_err());
    }

    #[gpui::test]
    fn sidebar_builds_only_rows_near_the_viewport(cx: &mut gpui::TestAppContext) {
        use super::*;

        struct SidebarHost(Entity<Shell>);
        impl Render for SidebarHost {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.0.update(cx, |shell, cx| {
                    shell.sidebar_windowing = true;
                    div()
                        .w(px(280.0))
                        .h(px(300.0))
                        .child(shell.render_chat_sidebar(&Theme::default(), cx))
                })
            }
        }
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
        });
        let dir = tempfile::tempdir().unwrap();
        let (host, cx) = cx.add_window_view(|_, cx| {
            SidebarHost(cx.new(|cx| {
                let state = cx.new(|_| AppState::new());
                let mut shell = Shell::new(
                    state,
                    EngineBootConfig {
                        data_dir: dir.path().into(),
                        ipc_port: 0,
                        edge_url: "http://127.0.0.1:1".into(),
                        edge_token: None,
                        org_id: None,
                        workos_client_id: None,
                        default_harness: zeron_proto::HarnessId::Mock,
                    },
                    cx,
                );
                shell.settings.sidebar_organization = SidebarOrganization::InOneList;
                let now = Utc::now();
                shell.state.update(cx, |state, _| {
                    state.workspace_scope = Some(WorkspaceScope::Local);
                    state.local_device_id = Some("local".into());
                    state.chats = (0..60)
                        .map(|ix| {
                            let mut chat = pin_test_chat(&format!("c{ix:02}"));
                            chat.last_message_at = Some(now - chrono::Duration::minutes(ix));
                            chat
                        })
                        .collect();
                });
                shell
            }))
        });
        let shell = host.read_with(cx, |host, _| host.0.clone());
        let draw = |cx: &mut gpui::VisualTestContext| {
            for _ in 0..2 {
                cx.update(|window, cx| {
                    window.refresh();
                    window.draw(cx).clear();
                });
            }
        };
        draw(cx);
        assert!(cx.debug_bounds("chat-c00").is_some());
        assert!(cx.debug_bounds("chat-c59").is_none());
        assert!(cx.debug_bounds("session-slot-c59").is_some());

        shell.update(cx, |shell, cx| {
            shell.sidebar_scroll.scroll_to_bottom();
            cx.notify();
        });
        draw(cx);
        assert!(cx.debug_bounds("chat-c59").is_some());
        assert!(cx.debug_bounds("chat-c00").is_none());

        // A far row resorted to the top builds on its first frame, not after its stale top heals.
        shell.update(cx, |shell, cx| {
            shell
                .sidebar_scroll
                .set_offset(gpui::point(px(0.0), px(0.0)));
            cx.notify();
        });
        draw(cx);
        assert!(cx.debug_bounds("chat-c58").is_none());
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                let chat = state
                    .chats
                    .iter_mut()
                    .find(|chat| chat.id == "c58")
                    .unwrap();
                chat.last_message_at = Some(Utc::now() + chrono::Duration::minutes(1));
            });
            cx.notify();
        });
        assert!(cx.debug_bounds("chat-c58").is_some());
    }

    #[gpui::test]
    fn active_first_glides_finished_threads_above_idle_ones(cx: &mut gpui::TestAppContext) {
        use super::*;

        struct SidebarHost(Entity<Shell>);
        impl Render for SidebarHost {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.0.update(cx, |shell, cx| {
                    div()
                        .w(px(280.0))
                        .h(px(800.0))
                        .child(shell.render_chat_sidebar(&Theme::default(), cx))
                })
            }
        }
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
        });
        let dir = tempfile::tempdir().unwrap();
        let (host, cx) = cx.add_window_view(|_, cx| {
            SidebarHost(cx.new(|cx| {
                let state = cx.new(|_| AppState::new());
                let mut shell = Shell::new(
                    state,
                    EngineBootConfig {
                        data_dir: dir.path().into(),
                        ipc_port: 0,
                        edge_url: "http://127.0.0.1:1".into(),
                        edge_token: None,
                        org_id: None,
                        workos_client_id: None,
                        default_harness: zeron_proto::HarnessId::Mock,
                    },
                    cx,
                );
                shell.settings.sidebar_organization = SidebarOrganization::InOneList;
                let now = Utc::now();
                shell.state.update(cx, |state, _| {
                    state.workspace_scope = Some(WorkspaceScope::Local);
                    state.local_device_id = Some("local".into());
                    let mut done = pin_test_chat("done");
                    done.last_message_at = Some(now - chrono::Duration::hours(2));
                    let mut idle = pin_test_chat("idle");
                    idle.last_message_at = Some(now - chrono::Duration::minutes(1));
                    idle.last_seen_at = Some(now);
                    state.chats = vec![done, idle];
                });
                shell
            }))
        });
        let shell = host.read_with(cx, |host, _| host.0.clone());
        let epoch = shell.update(cx, |shell, cx| {
            assert_eq!(shell.sidebar_visible_order(cx), ids(&["idle", "done"]));
            shell.activate_sidebar_view_row(SidebarViewRow::ActiveFirst, cx);
            shell.resort_epoch
        });
        shell.update(cx, |shell, cx| {
            assert!(shell.settings.sidebar_active_first);
            assert_eq!(shell.sidebar_visible_order(cx), ids(&["done", "idle"]));
            assert!(shell.resort_epoch > epoch);
            assert!(!shell.sidebar_resort.is_empty());
            shell.state.update(cx, |state, _| {
                let done = state
                    .chats
                    .iter_mut()
                    .find(|chat| chat.id == "done")
                    .unwrap();
                done.last_seen_at = Some(Utc::now());
            });
        });
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.sidebar_visible_order(cx), ids(&["idle", "done"]));
        });
    }

    #[gpui::test]
    fn closing_an_unstarted_sidechat_tab_archives_its_row(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let (engine, mut requests, _replies) = pin_test_engine();
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        let sidechat = |id: &str, last_message_at: Option<chrono::DateTime<chrono::Utc>>| {
            serde_json::from_value::<zeron_proto::Chat>(serde_json::json!({
                "id": id, "title": id, "deviceId": "local", "archived": false,
                "createdAt": chrono::Utc::now(), "lastMessageAt": last_message_at,
                "sidechatSourceChatId": "src",
            }))
            .unwrap()
        };
        let unstarted = sidechat("empty", None);
        let started = sidechat("used", Some(chrono::Utc::now()));
        window
            .update(cx, |shell, window, cx| {
                shell.state.update(cx, |state, _| {
                    state.set_test_engine(engine);
                    state.apply_chats(vec![
                        pin_test_chat("src"),
                        unstarted.clone(),
                        started.clone(),
                    ]);
                });
                let empty_tab = shell.add_sidechat_surface(&unstarted, cx);
                let used_tab = shell.add_sidechat_surface(&started, cx);
                shell.close_right_surface(super::RightSurface::Sidechat(used_tab), window, cx);
                shell.close_right_surface(super::RightSurface::Sidechat(empty_tab), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let mut archived = Vec::new();
        while let Ok(raw) = requests.try_recv() {
            let request: serde_json::Value = serde_json::from_str(&raw).unwrap();
            if request["params"]["op"] == "setChatArchived" {
                archived.push((
                    request["params"]["chatId"].as_str().unwrap().to_string(),
                    request["params"]["archived"].as_bool().unwrap(),
                ));
            }
        }
        assert_eq!(archived, [("empty".to_string(), true)]);
    }

    #[gpui::test]
    fn sidebar_unconfirmed_write_stops_the_queue_without_overwriting_observed_pins(
        cx: &mut gpui::TestAppContext,
    ) {
        // RPC completion (including channel closure during teardown) must wake
        // GPUI on the deterministic test scheduler's owning thread.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let (engine, _requests, _replies) = pin_test_engine();
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, _| {
                    remote_pin_state(state, true, true);
                    state.set_test_engine(engine);
                    state.sidebar_preferences = pin_snapshot(2, &["confirmed"]);
                });
                let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
                shell.apply_sidebar_pin_change(key.clone(), pin_change("first"), cx);
                shell.apply_sidebar_pin_change(key, pin_change("second"), cx);
                let id = shell.sidebar_pin_write.as_ref().unwrap().id;
                shell.mark_pin_write_unconfirmed(id, cx);
                assert!(shell.sidebar_pin_write.as_ref().unwrap().unconfirmed);
                assert_eq!(shell.active_sidebar_pins(cx), ids(&["confirmed"]));
                let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
                assert!(!shell.apply_sidebar_pin_change(key, pin_change("third"), cx));
                assert_eq!(
                    shell.finish_sidebar_pin_write(id, Err("late error".into()), cx),
                    None
                );
                assert!(shell.sidebar_pin_write.is_none());
                shell.state.update(cx, |state, _| {
                    state.apply_sidebar_preferences(pin_snapshot(3, &["first"]));
                });
                assert_eq!(
                    shell.active_sidebar_pins(cx),
                    ids(&["first"]),
                    "a timed-out request may still commit and must be reconciled via its watch"
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn sidebar_optimistic_writes_preserve_newer_edits_and_watch_state_on_failure(
        cx: &mut gpui::TestAppContext,
    ) {
        // RPC completion (including channel closure during teardown) must wake
        // GPUI on the deterministic test scheduler's owning thread.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let (engine, mut requests, _replies) = pin_test_engine();
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        let write_id = window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, _| {
                    remote_pin_state(state, false, true);
                    state.sidebar_preferences = pin_snapshot(1, &["original"]);
                    state.set_test_engine(engine);
                });
                let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
                assert!(shell.apply_sidebar_pin_change(key.clone(), pin_change("first"), cx));
                assert!(shell.apply_sidebar_pin_change(key, pin_change("second"), cx));
                assert_eq!(
                    shell.active_sidebar_pins(cx),
                    ids(&["original", "first", "second"])
                );
                assert_eq!(
                    shell.state.read(cx).sidebar_preferences.pinned_session_ids,
                    ids(&["original"])
                );
                assert!(
                    shell.mutate_task.is_none(),
                    "pin writes cannot be cancelled by generic sidebar mutations"
                );
                shell.sidebar_pin_write.as_ref().unwrap().id
            })
            .unwrap();
        cx.run_until_parked();
        let request: serde_json::Value =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(
            request["params"]["change"],
            serde_json::to_value(pin_change("first")).unwrap()
        );
        assert!(
            requests.try_recv().is_err(),
            "only one write may be in flight"
        );
        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, _| {
                    state.apply_sidebar_preferences(pin_snapshot(7, &["remote"]));
                });
                assert_eq!(
                    shell.finish_sidebar_pin_write(write_id, Err("rejected".into()), cx),
                    Some(pin_change("second"))
                );
                assert_eq!(
                    shell.active_sidebar_pins(cx),
                    ids(&["remote", "second"]),
                    "an older failure must not roll back a newer drop"
                );
                assert_eq!(
                    shell.finish_sidebar_pin_write(write_id, Err("rejected".into()), cx),
                    None
                );
                assert_eq!(
                    shell.active_sidebar_pins(cx),
                    ids(&["remote"]),
                    "failure restores latest observed state, not the old backup"
                );
                assert!(
                    shell
                        .sidebar_notice
                        .as_deref()
                        .unwrap()
                        .contains("Couldn't save sidebar changes")
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn sidebar_write_acknowledgements_ignore_older_watches_and_previous_operations(
        cx: &mut gpui::TestAppContext,
    ) {
        // RPC completion (including channel closure during teardown) must wake
        // GPUI on the deterministic test scheduler's owning thread.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let (engine, _requests, _replies) = pin_test_engine();
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, _| {
                    remote_pin_state(state, true, true);
                    state.set_test_engine(engine);
                });
                let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
                shell.apply_sidebar_pin_change(key.clone(), pin_change("saved"), cx);
                let first = shell.sidebar_pin_write.as_ref().unwrap().id;
                shell.finish_sidebar_pin_write(first, Ok(pin_snapshot(5, &["saved"])), cx);
                shell.state.update(cx, |state, _| {
                    assert!(!state.apply_sidebar_preferences(pin_snapshot(4, &["stale"])));
                });
                assert_eq!(shell.active_sidebar_pins(cx), ids(&["saved"]));
                shell.apply_sidebar_pin_change(key, pin_change("new-drop"), cx);
                let second = shell.sidebar_pin_write.as_ref().unwrap().id;
                shell.finish_sidebar_pin_write(first, Err("late failure".into()), cx);
                assert_eq!(shell.active_sidebar_pins(cx), ids(&["saved", "new-drop"]));
                shell.state.update(cx, |state, _| {
                    state.apply_sidebar_preferences(pin_snapshot(8, &["newer-remote"]));
                });
                shell.sidebar_notice = Some("Unrelated archive error".into());
                shell.finish_sidebar_pin_write(second, Ok(pin_snapshot(6, &["new-drop"])), cx);
                assert_eq!(shell.active_sidebar_pins(cx), ids(&["newer-remote"]));
                assert_eq!(
                    shell.sidebar_notice.as_deref(),
                    Some("Unrelated archive error")
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn sidebar_write_replies_cannot_cross_profile_or_engine_boundaries(
        cx: &mut gpui::TestAppContext,
    ) {
        // RPC completion (including channel closure during teardown) must wake
        // GPUI on the deterministic test scheduler's owning thread.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        for change_profile in [true, false] {
            let (engine, _requests, _replies) = pin_test_engine();
            let (replacement, _other_requests, _other_replies) = pin_test_engine();
            window
                .update(cx, |shell, _, cx| {
                    shell.state.update(cx, |state, _| {
                        remote_pin_state(state, true, true);
                        state.set_test_engine(engine);
                    });
                    let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
                    shell.apply_sidebar_pin_change(key, pin_change("pending"), cx);
                    let id = shell.sidebar_pin_write.as_ref().unwrap().id;
                    shell.state.update(cx, |state, _| {
                        if change_profile {
                            state.workspace_scope = Some(zeron_proto::WorkspaceScope::Local);
                        } else {
                            state.set_test_engine(replacement);
                        }
                        state.sidebar_preferences = pin_snapshot(0, &["new-runtime"]);
                    });
                    shell.finish_sidebar_pin_write(id, Ok(pin_snapshot(99, &["old-runtime"])), cx);
                    assert_eq!(
                        shell.state.read(cx).sidebar_preferences.pinned_session_ids,
                        ids(&["new-runtime"])
                    );
                    assert!(shell.sidebar_pin_write.is_none());
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn sidebar_drop_without_engine_returns_without_an_optimistic_pin(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        window
            .update(cx, |shell, window, cx| {
                shell.state.update(cx, |state, _| {
                    remote_pin_state(state, false, true);
                    state.chats = vec![pin_test_chat("normal")];
                });
                shell.settings.space_filter = None;
                let payload = SidebarSessionDrag {
                    chat_id: "normal".into(),
                    visible_ids: std::sync::Arc::new(vec![]),
                    filter: None,
                    profile_key: shell.active_sidebar_pin_profile_key(cx).unwrap(),
                };
                shell.begin_sidebar_session_transfer(
                    &payload,
                    gpui::point(px(0.0), px(0.0)),
                    window,
                    cx,
                );
                shell.finish_sidebar_session_transfer(&payload, SidebarSessionDrop::Pinned(0), cx);
                assert!(shell.active_sidebar_pins(cx).is_empty());
                assert!(shell.sidebar_session_return.is_some());
                assert!(shell.sidebar_pin_write.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn sidebar_menu_and_drop_both_reject_unknown_remote_preferences(cx: &mut gpui::TestAppContext) {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        window
            .update(cx, |shell, window, cx| {
                shell.state.update(cx, |state, _| {
                    remote_pin_state(state, false, false);
                    state.chats = vec![pin_test_chat("normal")];
                });
                shell.settings.space_filter = None;
                shell.set_chat_pinned("normal".into(), true, cx);
                assert_eq!(
                    shell.sidebar_notice.as_deref(),
                    Some("Pins are still syncing")
                );
                let payload = SidebarSessionDrag {
                    chat_id: "normal".into(),
                    visible_ids: std::sync::Arc::new(vec![]),
                    filter: None,
                    profile_key: shell.active_sidebar_pin_profile_key(cx).unwrap(),
                };
                shell.begin_sidebar_session_transfer(
                    &payload,
                    gpui::point(px(0.0), px(0.0)),
                    window,
                    cx,
                );
                shell.finish_sidebar_session_transfer(&payload, SidebarSessionDrop::Pinned(0), cx);
                assert!(shell.active_sidebar_pins(cx).is_empty());
                assert!(!shell.state.read(cx).sidebar_preferences.initialized);
                assert_eq!(
                    shell.sidebar_notice.as_deref(),
                    Some("Pins are still syncing")
                );
                assert!(
                    shell.sidebar_session_return.is_some(),
                    "rejected drop returns to its origin"
                );
                assert!(shell.mutate_task.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn sidebar_full_capacity_drop_returns_without_mutating_local_or_remote_pins(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        window
            .update(cx, |shell, window, cx| {
                let saved: Vec<String> = (0..zeron_proto::MAX_SIDEBAR_PINS)
                    .map(|n| format!("hidden-{n}"))
                    .collect();
                for remote in [false, true] {
                    shell.state.update(cx, |state, _| {
                        if remote {
                            remote_pin_state(state, true, true);
                            state.sidebar_preferences.pinned_session_ids = saved.clone();
                        } else {
                            state.workspace_scope = Some(WorkspaceScope::Local);
                        }
                        state.chats = vec![pin_test_chat("normal")];
                    });
                    let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
                    if !remote {
                        shell
                            .settings
                            .sidebar_pinned_session_ids_by_profile
                            .insert(key.clone(), saved.clone());
                    }
                    shell.settings.space_filter = None;
                    let payload = SidebarSessionDrag {
                        chat_id: "normal".into(),
                        visible_ids: std::sync::Arc::new(vec![]),
                        filter: None,
                        profile_key: key,
                    };
                    shell.begin_sidebar_session_transfer(
                        &payload,
                        gpui::point(px(0.0), px(0.0)),
                        window,
                        cx,
                    );
                    shell.finish_sidebar_session_transfer(
                        &payload,
                        SidebarSessionDrop::Pinned(0),
                        cx,
                    );
                    assert_eq!(shell.active_sidebar_pins(cx), saved);
                    assert_eq!(
                        shell.sidebar_notice.as_deref(),
                        Some("You can pin up to 200 sessions")
                    );
                    assert!(shell.sidebar_session_return.is_some());
                }
            })
            .unwrap();
    }

    #[gpui::test]
    fn sidebar_validation_preserves_offline_edits_and_rejects_stale_profiles(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        window
            .update(cx, |shell, _, cx| {
                shell
                    .state
                    .update(cx, |state, _| remote_pin_state(state, false, true));
                let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
                assert!(shell.validate_sidebar_pin_change(&key, &ids(&["cached"]), cx));
                assert!(!shell.validate_sidebar_pin_change(
                    "another-profile",
                    &ids(&["cached"]),
                    cx
                ));
                assert!(!shell.validate_sidebar_pin_change(
                    &key,
                    &ids(&["duplicate", "duplicate"]),
                    cx
                ));
                assert!(!shell.validate_sidebar_pin_change(&key, &ids(&[""]), cx));
                let saved: Vec<_> = (0..zeron_proto::MAX_SIDEBAR_PINS)
                    .map(|n| format!("pin-{n}"))
                    .collect();
                let reordered = super::sidebar_session_drop_pins(
                    &saved,
                    &saved,
                    &saved[0],
                    super::SidebarSessionDrop::Pinned(199),
                );
                assert!(shell.validate_sidebar_pin_change(&key, &reordered, cx));
                let unpinned = super::sidebar_session_drop_pins(
                    &saved,
                    &saved,
                    &saved[0],
                    super::SidebarSessionDrop::Regular,
                );
                assert!(shell.validate_sidebar_pin_change(&key, &unpinned, cx));
            })
            .unwrap();
    }

    #[gpui::test]
    fn sidebar_preferences_arriving_before_chats_never_prune_live_pins(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, _| {
                    state.apply_chats(vec![]);
                    remote_pin_state(state, true, true);
                    state.sidebar_preferences.pinned_session_ids = ids(&["live-remote"]);
                });
                shell.on_state_changed(&shell.state.clone(), cx);
                assert_eq!(shell.active_sidebar_pins(cx), ids(&["live-remote"]));
                assert!(shell.mutate_task.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn sidebar_project_groups_preserve_pin_order_and_keyboard_order(cx: &mut gpui::TestAppContext) {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        window
            .update(cx, |shell, _, cx| {
                shell.settings.sidebar_organization = SidebarOrganization::ByProject;
                shell
                    .settings
                    .sidebar_pins_mut("local".into())
                    .push("pin".into());
                shell.state.update(cx, |state, _| {
                    state.workspace_scope = Some(WorkspaceScope::Local);
                    state.spaces = ["a", "b"].into_iter().map(|id| serde_json::from_value(serde_json::json!({
                        "id": id, "deviceId": "local", "path": format!("/project/{id}"), "createdAt": Utc::now()
                    })).unwrap()).collect();
                    state.chats = [("pin", "a"), ("b-new", "b"), ("a-new", "a"), ("b-old", "b")]
                        .into_iter()
                        .enumerate()
                        .map(|(ix, (id, project))| {
                            let mut chat = pin_test_chat(id);
                            chat.space_id = Some(project.into());
                            chat.created_at = Utc::now() - chrono::Duration::minutes(ix as i64);
                            chat
                        })
                        .collect();
                });
                assert_eq!(
                    shell.sidebar_visible_order(cx),
                    ids(&["pin", "b-new", "b-old", "a-new"])
                );
                shell.settings.space_filter = Some("a".into());
                assert_eq!(shell.sidebar_visible_order(cx), ids(&["pin", "a-new"]));
            })
            .unwrap();
        cx.run_until_parked();
    }

    #[gpui::test]
    fn sidebar_projects_sharing_a_repository_group_only_by_project(cx: &mut gpui::TestAppContext) {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        window
            .update(cx, |shell, _, cx| {
                shell.settings.sidebar_organization = SidebarOrganization::ByProject;
                shell.state.update(cx, |state, _| {
                    state.workspace_scope = Some(WorkspaceScope::Local);
                    state.local_device_id = Some("local".into());
                    // The same repository on this device and on another,
                    // plus an unrelated local project.
                    state.spaces = [
                        ("remote-clone", "remote", Some("github.com/o/r"), 0),
                        ("local-clone", "local", Some("github.com/o/r"), 1),
                        ("other", "local", None, 2),
                    ]
                    .into_iter()
                    .map(|(id, device, repository, minutes)| {
                        serde_json::from_value(serde_json::json!({
                            "id": id, "deviceId": device, "path": format!("/{device}/{id}"),
                            "name": id, "repositoryId": repository,
                            "createdAt": Utc::now() - chrono::Duration::minutes(10 - minutes),
                        }))
                        .unwrap()
                    })
                    .collect();
                    state.chats = [
                        ("remote-new", "remote-clone", "remote"),
                        ("other-mid", "other", "local"),
                        ("local-old", "local-clone", "local"),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(ix, (id, space, device))| {
                        let mut chat = pin_test_chat(id);
                        chat.space_id = Some(space.into());
                        chat.device_id = device.into();
                        chat.created_at = Utc::now() - chrono::Duration::minutes(ix as i64);
                        chat
                    })
                    .collect();
                });
                assert_eq!(
                    shell.sidebar_visible_order(cx),
                    ids(&["remote-new", "local-old", "other-mid"])
                );
                // The oldest clone names the group on every device; this
                // device's checkout leads the project's members.
                let state = shell.state.read(cx);
                let local_chat = state.chats.iter().find(|c| c.id == "local-old").unwrap();
                assert_eq!(
                    sidebar_project_group(state, local_chat),
                    ("repo:github.com/o/r".into(), "remote-clone".into())
                );
                let local_space = state.space_row("local-clone").unwrap();
                assert_eq!(state.representative_space(local_space).id, "remote-clone");
                let members: Vec<&str> = state
                    .project_members(local_space)
                    .into_iter()
                    .map(|space| space.id.as_str())
                    .collect();
                assert_eq!(members, ["local-clone", "remote-clone"]);

                shell.settings.sidebar_organization = SidebarOrganization::ByDevice;
                assert_eq!(
                    shell.sidebar_visible_order(cx),
                    ids(&["other-mid", "local-old", "remote-new"])
                );
            })
            .unwrap();
        cx.run_until_parked();
    }

    #[gpui::test]
    fn sidebar_remote_pins_ignore_old_local_preferences(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let window = pin_test_shell(cx, dir.path());
        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, _| {
                    state.apply_chats(vec![]);
                    remote_pin_state(state, true, false);
                });
                let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
                shell
                    .settings
                    .sidebar_pinned_session_ids_by_profile
                    .insert(key.clone(), ids(&["live-remote"]));
                shell.on_state_changed(&shell.state.clone(), cx);
                assert_eq!(shell.settings.sidebar_pins(&key), ids(&["live-remote"]));
                assert!(!shell.state.read(cx).sidebar_preferences.initialized);
            })
            .unwrap();
    }

    #[test]
    fn pins_lead_without_changing_unpinned_recency() {
        let recency = ids(&["newest", "p2", "middle", "p1", "oldest"]);
        assert_eq!(
            project_pinned_first(&recency, &ids(&["p1", "p2"])),
            ids(&["p1", "p2", "newest", "middle", "oldest"])
        );
    }

    #[test]
    fn inline_session_slide_retargets_and_returns_without_a_ghost() {
        use super::*;
        let mut slide = SidebarSessionSlide {
            from: 0.0,
            to: SIDEBAR_SESSION_SLOT,
            epoch: 1,
            started: std::time::Instant::now() - TAB_SLIDE.total(),
        };
        slide.retarget(2.0 * SIDEBAR_SESSION_SLOT);
        assert_eq!(slide.from, SIDEBAR_SESSION_SLOT);
        assert_eq!(slide.to, 2.0 * SIDEBAR_SESSION_SLOT);
        assert_eq!(slide.epoch, 2);
        slide.retarget(2.0 * SIDEBAR_SESSION_SLOT);
        assert_eq!(slide.epoch, 2);
        slide.started -= TAB_SLIDE.total();
        slide.retarget(0.0);
        assert_eq!(slide.from, 2.0 * SIDEBAR_SESSION_SLOT);
        assert_eq!(slide.to, 0.0);
    }

    #[test]
    fn session_layout_reverses_from_its_current_height() {
        use super::*;
        let mut slide = SidebarSessionSlide {
            from: 0.0,
            to: SIDEBAR_SESSION_SLOT,
            epoch: 0,
            started: std::time::Instant::now() - TAB_SLIDE.total() / 2,
        };
        let halfway = slide.current();
        assert!(halfway > 0.0 && halfway < SIDEBAR_SESSION_SLOT);
        slide.retarget(0.0);
        assert!((slide.from - halfway).abs() < 0.5);
        assert_eq!(slide.to, 0.0);
        slide.started -= TAB_SLIDE.total();
        assert_eq!(slide.current(), 0.0);
    }

    #[test]
    fn transfer_gaps_shift_neighbors_without_reordering_data() {
        use super::sidebar_gap_offset;
        // Entering from another section opens a full slot at the destination.
        assert_eq!(sidebar_gap_offset(0, None, 1, 63.0), 0.0);
        assert_eq!(sidebar_gap_offset(1, None, 1, 63.0), 63.0);
        assert_eq!(sidebar_gap_offset(2, None, 1, 63.0), 63.0);
        // Within a normal group, its original vacant slot is reused.
        assert_eq!(sidebar_gap_offset(0, Some(2), 0, 63.0), 63.0);
        assert_eq!(sidebar_gap_offset(1, Some(2), 0, 63.0), 63.0);
        assert_eq!(sidebar_gap_offset(2, Some(2), 0, 63.0), 0.0);
        assert_eq!(sidebar_gap_offset(1, Some(0), 3, 63.0), -63.0);
        assert_eq!(sidebar_gap_offset(3, Some(0), 3, 63.0), 0.0);
    }

    #[gpui::test]
    fn pinned_disclosure_click_collapses_and_restores_rows(cx: &mut gpui::TestAppContext) {
        use super::*;

        struct PinnedHost(Entity<Shell>);
        impl Render for PinnedHost {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.0.update(cx, |shell, cx| {
                    let items = (0..2)
                        .map(|_| div().h(px(61.0)).into_any_element())
                        .collect();
                    div()
                        .w(px(280.0))
                        .flex()
                        .flex_col()
                        .child(shell.render_pinned_section(items, 128.0, &Theme::default(), cx))
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let (host, cx) = cx.add_window_view(|_, cx| {
            PinnedHost(cx.new(|cx| {
                let state = cx.new(|_| AppState::new());
                let mut shell = Shell::new(
                    state,
                    EngineBootConfig {
                        data_dir: dir.path().into(),
                        ipc_port: 0,
                        edge_url: "http://127.0.0.1:1".into(),
                        edge_token: None,
                        org_id: None,
                        workos_client_id: None,
                        default_harness: zeron_proto::HarnessId::Mock,
                    },
                    cx,
                );
                shell.state.update(cx, |state, _| {
                    state.workspace_scope = Some(WorkspaceScope::Development);
                    state.sidebar_preferences.pinned_session_ids = vec!["pin".into()];
                    state.chats = ["pin", "regular"]
                        .into_iter()
                        .map(|id| {
                            serde_json::from_value(serde_json::json!({
                                "id": id, "deviceId": "local", "archived": false,
                                "createdAt": Utc::now(),
                            }))
                            .unwrap()
                        })
                        .collect();
                });
                shell
            }))
        });
        let shell = host.read_with(cx, |host, _| host.0.clone());
        for open in [true, false, true] {
            if !open || !shell.read_with(cx, |shell, _| shell.pinned_open) {
                let toggle = cx.debug_bounds("pinned-toggle").unwrap().center();
                cx.simulate_mouse_down(toggle, MouseButton::Left, gpui::Modifiers::default());
                cx.simulate_mouse_up(toggle, MouseButton::Left, gpui::Modifiers::default());
            }
            shell.update(cx, |shell, cx| {
                assert_eq!(shell.pinned_open, open);
                let expected = if open {
                    vec!["pin", "regular"]
                } else {
                    vec!["regular"]
                };
                assert_eq!(shell.sidebar_visible_order(cx), expected);
                assert_eq!(shell.active_sidebar_pins(cx), ["pin"]);
                // Finish the tween to assert settled layout without wall-clock sleeps.
                shell.sidebar_disclosure_motion.clear();
                cx.notify();
            });
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear();
            });
            let bounds = cx.debug_bounds("sidebar-pinned-section").unwrap();
            assert_eq!(
                f32::from(bounds.size.height),
                if open { 160.0 } else { 32.0 }
            );
            assert!(cx.debug_bounds("sidebar-pinned-divider").is_none());
        }
    }

    #[gpui::test]
    fn session_section_drops_preserve_live_activity_order(cx: &mut gpui::TestAppContext) {
        exercise_session_section_drops(cx, false, true);
    }

    #[gpui::test]
    fn compact_sidebar_section_drops_follow_pointer(cx: &mut gpui::TestAppContext) {
        exercise_session_section_drops(cx, true, false);
    }

    #[gpui::test]
    fn hidden_label_sidebar_section_drops_follow_pointer(cx: &mut gpui::TestAppContext) {
        exercise_session_section_drops(cx, false, false);
    }

    fn exercise_session_section_drops(
        cx: &mut gpui::TestAppContext,
        compact: bool,
        show_label: bool,
    ) {
        use super::*;

        struct SidebarHost(Entity<Shell>);
        impl Render for SidebarHost {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.0.update(cx, |shell, cx| {
                    div()
                        .w(px(280.0))
                        .h(px(800.0))
                        .on_drag_move::<SidebarSessionDrag>(
                            cx.listener(Shell::contain_pinned_session_drag),
                        )
                        .on_drop::<SidebarSessionDrag>(
                            cx.listener(|shell, _, _, cx| {
                                shell.cancel_sidebar_session_transfer(cx)
                            }),
                        )
                        .child(shell.render_chat_sidebar(&Theme::default(), cx))
                })
            }
        }
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
        });
        let (host, cx) = cx.add_window_view(|_, cx| {
            SidebarHost(cx.new(|cx| {
                let state = cx.new(|_| AppState::new());
                let mut shell = Shell::new(
                    state,
                    EngineBootConfig {
                        data_dir: dir.path().into(),
                        ipc_port: 0,
                        edge_url: "http://127.0.0.1:1".into(),
                        edge_token: None,
                        org_id: None,
                        workos_client_id: None,
                        default_harness: zeron_proto::HarnessId::Mock,
                    },
                    cx,
                );
                shell.settings.sidebar_organization = SidebarOrganization::InOneList;
                shell.settings.sidebar_show_branch = true;
                shell.settings.sidebar_compact = compact;
                shell.settings.sidebar_show_project_label = show_label;
                shell.state.update(cx, |state, _| {
                    state.workspace_scope = Some(WorkspaceScope::Local);
                    state.local_device_id = Some("local".into());
                    state.chats = ["older", "newer"]
                        .into_iter()
                        .enumerate()
                        .map(|(ix, id)| {
                            serde_json::from_value(serde_json::json!({
                                "id": id, "title": id, "deviceId": "local", "archived": false,
                                "sourceContext": {
                                    "checkoutId": "checkout", "repoRoot": "/project", "cwd": "/project",
                                    "branch": "feature/sidebar-drag", "observedAt": Utc::now(),
                                },
                                "createdAt": Utc::now() - chrono::Duration::minutes(10 - ix as i64),
                            }))
                            .unwrap()
                        })
                        .collect();
                    let mut archived = state.chats[0].clone();
                    archived.id = "archived".into();
                    archived.archived = true;
                    state.chats.push(archived);
                });
                shell
            }))
        });
        let shell = host.read_with(cx, |host, _| host.0.clone());

        // Archived rows share geometry and metadata in every sidebar layout.
        let active = cx.debug_bounds("chat-older").unwrap();
        let archived = cx.debug_bounds("chat-archived").unwrap();
        assert_eq!(active.size, archived.size);
        assert_eq!(cx.debug_bounds("chat-branch-archived").is_some(), !compact);
        assert_eq!(
            cx.debug_bounds("chat-device-archived").is_some(),
            !compact && show_label
        );
        if compact {
            assert!(cx.debug_bounds("chat-status-archived").is_some());
            let time = cx.debug_bounds("chat-time-archived").unwrap();
            cx.simulate_mouse_move(archived.center(), None, gpui::Modifiers::default());
            assert_eq!(cx.debug_bounds("chat-time-archived").unwrap(), time);
        }

        // Hiding Archived removes the whole shelf without changing its chats or disclosure state.
        for visible in [false, true, false] {
            shell.update(cx, |shell, cx| {
                let open = shell.archived_open;
                shell.activate_sidebar_view_row(SidebarViewRow::ShowArchived, cx);
                assert_eq!(shell.settings.sidebar_show_archived, visible);
                assert_eq!(shell.archived_open, open);
                assert_eq!(shell.archived_sidebar_chats(cx).len(), 1);
            });
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear();
            });
            assert_eq!(cx.debug_bounds("archived-toggle").is_some(), visible);
            assert_eq!(cx.debug_bounds("chat-archived").is_some(), visible);
        }

        shell.update(cx, |shell, cx| {
            assert!(shell.reveal_sidebar_chat("archived", cx));
            assert!(shell.settings.sidebar_show_archived);
        });
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        assert!(cx.debug_bounds("archived-toggle").is_some());
        assert!(cx.debug_bounds("chat-archived").is_some());

        // Sessions owns all unpinned rows, including their keyboard traversal.
        for open in [false, true] {
            let toggle = cx.debug_bounds("sessions-toggle").unwrap().center();
            cx.simulate_mouse_down(toggle, MouseButton::Left, gpui::Modifiers::default());
            cx.simulate_mouse_up(toggle, MouseButton::Left, gpui::Modifiers::default());
            shell.update(cx, |shell, cx| {
                assert_eq!(shell.sessions_open, open);
                assert_eq!(
                    shell.sidebar_visible_order(cx),
                    if open {
                        ids(&["newer", "older"])
                    } else {
                        Vec::new()
                    }
                );
                shell.sidebar_disclosure_motion.clear();
                cx.notify();
            });
        }
        if compact {
            let status = cx.debug_bounds("chat-status-older").unwrap();
            let time = cx.debug_bounds("chat-time-older").unwrap();
            assert!(status.right() < time.left());
            let row = cx.debug_bounds("chat-older").unwrap();
            let title = cx.debug_bounds("chat-title-older").unwrap();
            cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::default());
            // The globe/archive slot is always reserved, so hovering swaps its
            // glyph in place: the title keeps its width and the status and time
            // columns never move.
            assert_eq!(cx.debug_bounds("chat-title-older").unwrap().size.width, title.size.width);
            assert_eq!(cx.debug_bounds("chat-status-older").unwrap(), status);
            assert_eq!(cx.debug_bounds("chat-time-older").unwrap(), time);
        }

        // Dragging a regular session over another regular session is a no-op.
        let source_bounds = cx.debug_bounds("chat-older").unwrap();
        let from = source_bounds.center();
        cx.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            from + gpui::point(px(8.0), px(0.0)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        let dragged_card = cx.debug_bounds("chat-older").unwrap();
        assert!(cx.debug_bounds("drag-chat-older").is_none());
        assert_eq!(dragged_card.size, source_bounds.size);
        assert_eq!(dragged_card.origin.x, source_bounds.origin.x);
        assert_eq!(
            f32::from(dragged_card.size.height),
            super::super::sidebar_row_height(compact, false, show_label)
        );
        // Even a sub-row move follows the pointer immediately, before a drop slot changes.
        let pointer = from + gpui::point(px(8.0), px(7.0));
        cx.simulate_mouse_move(pointer, Some(MouseButton::Left), gpui::Modifiers::default());
        let moving = cx.debug_bounds("chat-older").unwrap();
        assert!((f32::from(moving.origin.y - source_bounds.origin.y) - 7.0).abs() < 1.0);
        let slot = cx.debug_bounds("session-slot-newer").unwrap();
        let target = gpui::point(slot.center().x, slot.top() + px(3.0));
        cx.simulate_mouse_move(target, Some(MouseButton::Left), gpui::Modifiers::default());
        shell.read_with(cx, |shell, cx| {
            let drag = shell.sidebar_session_transfer.as_ref().unwrap();
            assert_eq!(drag.preview.as_ref().unwrap().group, "regular");
            assert!(drag.siblings["newer"].to > 0.0);
            assert!(shell.active_sidebar_pins(cx).is_empty());
        });
        cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::default());
        let returning_card = cx.debug_bounds("chat-older").unwrap();
        assert_eq!(returning_card.size, source_bounds.size);
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.active_sidebar_pins(cx), Vec::<String>::new());
            assert!(shell.sidebar_session_return.is_some());
            assert_eq!(shell.sidebar_visible_order(cx), ids(&["newer", "older"]));
            shell.sidebar_session_return = None;
            cx.notify();
        });

        // The transient empty Pinned header accepts the first pin.
        let from = cx.debug_bounds("chat-older").unwrap().center();
        cx.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            from + gpui::point(px(8.0), px(0.0)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        let target = cx.debug_bounds("pinned-toggle").unwrap().center();
        cx.simulate_mouse_move(target, Some(MouseButton::Left), gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            let drag = shell.sidebar_session_transfer.as_mut().unwrap();
            assert_eq!(drag.source_collapse.to, drag.row_height + SIDEBAR_LIST_GAP);
            drag.source_collapse.started -= TAB_SLIDE.total();
            for gap in drag.section_gaps.values_mut() {
                gap.started -= TAB_SLIDE.total();
            }
            cx.notify();
        });
        assert_eq!(
            cx.debug_bounds("session-slot-older").unwrap().size.height,
            px(0.0)
        );
        // Only the surviving row (and minimum list spacing) remains, not a
        // second card-sized vacancy at the source.
        assert!(
            cx.debug_bounds("sidebar-regular-sessions")
                .unwrap()
                .size
                .height
                <= cx.debug_bounds("session-slot-newer").unwrap().size.height
                    + px(SIDEBAR_LIST_GAP)
        );
        cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.active_sidebar_pins(cx), ids(&["older"]));
            assert_eq!(shell.sidebar_visible_order(cx), ids(&["older", "newer"]));
            assert!(
                shell.sidebar_resort.is_empty(),
                "successful pin must not replay the movement"
            );
            assert!(shell.sidebar_new_keys.is_empty());
            cx.notify();
        });

        // A single pin can leave Pinned and returns to its activity position.
        let from = cx.debug_bounds("chat-older").unwrap().center();
        cx.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            from + gpui::point(px(8.0), px(0.0)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        let slot = cx.debug_bounds("session-slot-newer").unwrap();
        let target = gpui::point(slot.center().x, slot.top() + px(3.0));
        cx.simulate_mouse_move(target, Some(MouseButton::Left), gpui::Modifiers::default());
        shell.read_with(cx, |shell, cx| {
            assert!(shell.sidebar_transfer_extra_gap("regular") > 0.0);
            assert!(shell.sidebar_session_transfer.as_ref().unwrap().siblings["newer"].to > 0.0);
            assert_eq!(shell.active_sidebar_pins(cx), ids(&["older"]));
        });
        shell.update(cx, |shell, cx| {
            let drag = shell.sidebar_session_transfer.as_mut().unwrap();
            assert_eq!(drag.source_collapse.to, drag.row_height);
            drag.source_collapse.started -= TAB_SLIDE.total();
            for gap in drag.section_gaps.values_mut() {
                gap.started -= TAB_SLIDE.total();
            }
            cx.notify();
        });
        assert_eq!(
            cx.debug_bounds("session-slot-older").unwrap().size.height,
            px(0.0)
        );
        cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            assert!(shell.active_sidebar_pins(cx).is_empty());
            assert_eq!(shell.sidebar_visible_order(cx), ids(&["newer", "older"]));
            assert!(
                shell.sidebar_resort.is_empty(),
                "successful unpin must not replay the movement"
            );
            assert!(shell.sidebar_new_keys.is_empty());
            cx.notify();
        });

        // A live activity update still wins while the normal session is dragged.
        let from = cx.debug_bounds("chat-older").unwrap().center();
        cx.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            from + gpui::point(px(8.0), px(0.0)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state
                    .chats
                    .iter_mut()
                    .find(|chat| chat.id == "older")
                    .unwrap()
                    .last_message_at = Some(Utc::now());
            });
            cx.notify();
        });
        let target = cx.debug_bounds("chat-newer").unwrap().center();
        cx.simulate_mouse_move(target, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            assert!(shell.active_sidebar_pins(cx).is_empty());
            assert_eq!(shell.sidebar_visible_order(cx), ids(&["older", "newer"]));
            assert!(shell.sidebar_session_return.is_some());
        });

        // A closed Pinned header remains a valid target and opens on success.
        shell.update(cx, |shell, cx| {
            shell.pinned_open = false;
            cx.notify();
        });
        let from = cx.debug_bounds("chat-newer").unwrap().center();
        cx.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            from + gpui::point(px(8.0), px(0.0)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        let target = cx.debug_bounds("pinned-toggle").unwrap().center();
        cx.simulate_mouse_move(target, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            assert!(shell.pinned_open);
            assert_eq!(shell.active_sidebar_pins(cx), ids(&["newer"]));
            assert!(shell.sidebar_resort.is_empty());
            assert!(shell.sidebar_new_keys.is_empty());
            cx.notify();
        });

        // Dropping outside the sections cancels; it must not unpin the source.
        let from = cx.debug_bounds("chat-newer").unwrap().center();
        cx.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            from + gpui::point(px(8.0), px(0.0)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        let regular = cx.debug_bounds("session-slot-older").unwrap().center();
        cx.simulate_mouse_move(regular, Some(MouseButton::Left), gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            let drag = shell.sidebar_session_transfer.as_mut().unwrap();
            assert!(drag.source_collapse.to > 0.0);
            drag.source_collapse.started -= TAB_SLIDE.total() / 2;
            for gap in drag.section_gaps.values_mut() {
                gap.started -= TAB_SLIDE.total() / 2;
            }
            cx.notify();
        });
        let outside = gpui::point(px(275.0), px(790.0));
        cx.simulate_mouse_move(outside, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(outside, MouseButton::Left, gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.active_sidebar_pins(cx), ids(&["newer"]));
            assert!(shell.sidebar_session_transfer.is_none());
            let returning = shell.sidebar_session_return.as_ref().unwrap();
            assert!(returning.transfer.source_collapse.from > 0.0);
            assert_eq!(returning.transfer.source_collapse.to, 0.0);
            assert!(
                returning
                    .transfer
                    .section_gaps
                    .values()
                    .all(|gap| gap.to == 0.0)
            );
        });

        // Remote pin changes invalidate an in-flight snapshot instead of being overwritten.
        let from = cx.debug_bounds("chat-older").unwrap().center();
        cx.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            from + gpui::point(px(8.0), px(0.0)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        shell.update(cx, |shell, cx| {
            let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
            shell
                .settings
                .sidebar_pinned_session_ids_by_profile
                .insert(key, ids(&["older", "newer"]));
            cx.notify();
        });
        let target = cx.debug_bounds("pinned-toggle").unwrap().center();
        cx.simulate_mouse_move(target, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.active_sidebar_pins(cx), ids(&["older", "newer"]));
            shell.sidebar_resort.clear();
            shell.sidebar_new_keys.clear();
            shell.reduced_motion = true;
            cx.notify();
        });

        // Even with no regular rows, a temporary destination can unpin one.
        let from = cx.debug_bounds("chat-newer").unwrap().center();
        cx.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            from + gpui::point(px(8.0), px(0.0)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        let target = cx
            .debug_bounds("sidebar-regular-sessions")
            .unwrap()
            .center();
        cx.simulate_mouse_move(target, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.active_sidebar_pins(cx), ids(&["older"]));
            assert_eq!(shell.sidebar_visible_order(cx), ids(&["older", "newer"]));
            assert!(shell.sidebar_resort.is_empty());
            assert!(shell.sidebar_new_keys.is_empty());
            cx.notify();
        });

        // Dropping in the lower half of a pinned row inserts after that row.
        let from = cx.debug_bounds("chat-newer").unwrap().center();
        cx.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            from + gpui::point(px(8.0), px(0.0)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        let row = cx.debug_bounds("chat-older").unwrap();
        let target = gpui::point(row.center().x, row.bottom() - px(3.0));
        cx.simulate_mouse_move(target, Some(MouseButton::Left), gpui::Modifiers::default());
        shell.read_with(cx, |shell, cx| {
            assert!(shell.sidebar_transfer_extra_gap("pinned") > 0.0);
            assert_eq!(shell.active_sidebar_pins(cx), ids(&["older"]));
        });
        cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.active_sidebar_pins(cx), ids(&["older", "newer"]));
            assert!(shell.sidebar_resort.is_empty());
            assert!(shell.sidebar_new_keys.is_empty());
            cx.notify();
        });

        // Existing pin-to-pin reordering still works with the shared payload.
        shell.update(cx, |shell, cx| {
            shell.reduced_motion = false;
            cx.notify();
        });
        let from = cx.debug_bounds("chat-newer").unwrap().center();
        cx.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            from + gpui::point(px(8.0), px(0.0)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        let row = cx.debug_bounds("chat-older").unwrap();
        let target = gpui::point(row.center().x, row.top() + px(3.0));
        cx.simulate_mouse_move(target, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.active_sidebar_pins(cx), ids(&["newer", "older"]));
            assert!(
                shell.sidebar_resort.is_empty(),
                "successful reorder must not replay the movement"
            );
            assert!(shell.sidebar_new_keys.is_empty());
            assert!(shell.sidebar_session_transfer.is_none());
            assert!(shell.sidebar_session_return.is_none());
        });

        // Suppressing a completed drag must not disable later activity-driven glides.
        shell.update(cx, |shell, cx| {
            let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
            for id in shell.active_sidebar_pins(cx) {
                shell.apply_sidebar_pin_change(
                    key.clone(),
                    zeron_proto::SidebarPinChange::Unpin { session_id: id },
                    cx,
                );
            }
            cx.notify();
        });
        let epoch = shell.update(cx, |shell, cx| {
            assert_eq!(shell.sidebar_visible_order(cx), ids(&["older", "newer"]));
            shell.resort_epoch
        });
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state
                    .chats
                    .iter_mut()
                    .find(|chat| chat.id == "newer")
                    .unwrap()
                    .last_message_at = Some(Utc::now() + chrono::Duration::seconds(1));
            });
            cx.notify();
        });
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.sidebar_visible_order(cx), ids(&["newer", "older"]));
            assert!(shell.resort_epoch > epoch);
            assert!(!shell.sidebar_resort.is_empty());
        });
        // A collapsed Sessions header remains an unpin target and opens on drop.
        shell.update(cx, |shell, cx| {
            let key = shell.active_sidebar_pin_profile_key(cx).unwrap();
            *shell.settings.sidebar_pins_mut(key) = ids(&["older"]);
            shell.sidebar_session_return = None;
            shell.sidebar_resort.clear();
            shell.sidebar_disclosure_motion.clear();
            cx.notify();
        });
        let toggle = cx.debug_bounds("sessions-toggle").unwrap().center();
        cx.simulate_mouse_down(toggle, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(toggle, MouseButton::Left, gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            assert!(!shell.sessions_open);
            shell.sidebar_disclosure_motion.clear();
            cx.notify();
        });
        let from = cx.debug_bounds("chat-older").unwrap().center();
        cx.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            from + gpui::point(px(8.0), px(0.0)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        let target = cx.debug_bounds("sessions-toggle").unwrap().center();
        cx.simulate_mouse_move(target, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::default());
        shell.read_with(cx, |shell, cx| {
            assert!(shell.sessions_open);
            assert!(shell.active_sidebar_pins(cx).is_empty());
        });
    }

    #[gpui::test]
    fn sidebar_hover_card_opens_after_the_delay_beside_the_row(cx: &mut gpui::TestAppContext) {
        use super::*;

        struct SidebarHost(Entity<Shell>);
        impl Render for SidebarHost {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.0.update(cx, |shell, cx| {
                    div()
                        .w(px(280.0))
                        .h(px(800.0))
                        .child(shell.render_chat_sidebar(&Theme::default(), cx))
                })
            }
        }
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let (_host, cx) = cx.add_window_view(|_, cx| {
            SidebarHost(cx.new(|cx| {
                let state = cx.new(|_| AppState::new());
                let mut shell = Shell::new(
                    state,
                    EngineBootConfig {
                        data_dir: dir.path().into(),
                        ipc_port: 0,
                        edge_url: "http://127.0.0.1:1".into(),
                        edge_token: None,
                        org_id: None,
                        workos_client_id: None,
                        default_harness: zeron_proto::HarnessId::Mock,
                    },
                    cx,
                );
                shell.settings.sidebar_organization = SidebarOrganization::InOneList;
                shell.reduced_motion = true;
                shell.state.update(cx, |state, _| {
                    state.workspace_scope = Some(WorkspaceScope::Local);
                    state.local_device_id = Some("local".into());
                    state.chats = ["a", "b"]
                        .into_iter()
                        .enumerate()
                        .map(|(ix, id)| {
                            serde_json::from_value(serde_json::json!({
                                "id": id, "title": id, "deviceId": "local", "archived": false,
                                "createdAt": Utc::now() - chrono::Duration::minutes(ix as i64),
                            }))
                            .unwrap()
                        })
                        .collect();
                });
                shell
            }))
        });
        let row = cx.debug_bounds("chat-a").unwrap();
        cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("sidebar-hover-card").is_none());
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(200));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        let card = cx.debug_bounds("sidebar-hover-card").unwrap();
        assert!(card.left() >= row.right());
        assert!((card.top() - row.top()).abs() < px(8.0));

        let below = cx.debug_bounds("chat-b").unwrap();
        cx.simulate_mouse_move(
            gpui::point(row.center().x, below.bottom() + px(200.0)),
            None,
            gpui::Modifiers::default(),
        );
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("sidebar-hover-card").is_none());
    }

    #[gpui::test]
    fn archive_pill_follows_a_still_pointer_down_the_list(cx: &mut gpui::TestAppContext) {
        use super::*;

        struct SidebarHost(Entity<Shell>);
        impl Render for SidebarHost {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.0.update(cx, |shell, cx| {
                    div()
                        .w(px(280.0))
                        .h(px(800.0))
                        .child(shell.render_chat_sidebar(&Theme::default(), cx))
                })
            }
        }
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
        });
        let (host, cx) = cx.add_window_view(|_, cx| {
            SidebarHost(cx.new(|cx| {
                let state = cx.new(|_| AppState::new());
                let mut shell = Shell::new(
                    state,
                    EngineBootConfig {
                        data_dir: dir.path().into(),
                        ipc_port: 0,
                        edge_url: "http://127.0.0.1:1".into(),
                        edge_token: None,
                        org_id: None,
                        workos_client_id: None,
                        default_harness: zeron_proto::HarnessId::Mock,
                    },
                    cx,
                );
                shell.settings.sidebar_organization = SidebarOrganization::InOneList;
                shell.reduced_motion = true;
                shell.state.update(cx, |state, _| {
                    state.workspace_scope = Some(WorkspaceScope::Local);
                    state.local_device_id = Some("local".into());
                    state.chats = ["a", "b", "c"]
                        .into_iter()
                        .enumerate()
                        .map(|(ix, id)| {
                            serde_json::from_value(serde_json::json!({
                                "id": id, "title": id, "deviceId": "local", "archived": false,
                                "createdAt": Utc::now() - chrono::Duration::minutes(ix as i64),
                            }))
                            .unwrap()
                        })
                        .collect();
                });
                shell
            }))
        });
        let shell = host.read_with(cx, |host, _| host.0.clone());
        let order = shell.update(cx, |shell, cx| shell.sidebar_visible_order(cx));
        assert_eq!(order, ids(&["a", "b", "c"]));
        let state = shell.read_with(cx, |shell, _| shell.state.clone());

        let row = cx.debug_bounds("chat-a").unwrap();
        cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::default());
        let pointer = cx.debug_bounds("chat-a-corner").unwrap().center();
        cx.simulate_mouse_move(pointer, None, gpui::Modifiers::default());
        for (archived, next) in [
            ("a", Some(("b", "chat-b-corner"))),
            ("b", Some(("c", "chat-c-corner"))),
            ("c", None),
        ] {
            cx.simulate_click(pointer, gpui::Modifiers::default());
            shell.read_with(cx, |shell, _| {
                assert!(shell.chat_hover_resync);
                assert_eq!(
                    shell.chat_status_hover.as_deref(),
                    Some(format!("chat-{archived}").as_str())
                );
            });
            // The engine round-trip lands; the next row slides up under the
            // unmoved pointer.
            state.update(cx, |state, cx| {
                state
                    .chats
                    .iter_mut()
                    .find(|chat| chat.id == archived)
                    .unwrap()
                    .archived = true;
                cx.notify();
            });
            cx.run_until_parked();
            shell.update(cx, |shell, cx| {
                shell.sidebar_resort.clear();
                shell.sidebar_new_keys.clear();
                cx.notify();
            });
            cx.run_until_parked();
            // Its Archived-section copy shares the row key and must not stay lit.
            assert_eq!(
                crate::motion::hover_t(&format!("chat-{archived}-hover")),
                0.0
            );
            let expected = next.map(|(next, _)| format!("chat-{next}"));
            shell.read_with(cx, |shell, _| {
                assert_eq!(shell.chat_status_hover, expected);
            });
            if let Some((_, corner)) = next {
                assert!(cx.debug_bounds(corner).unwrap().contains(&pointer));
            }
        }

        // Real pointer movement hands hover back to gpui's own tracking.
        let row = cx.debug_bounds("chat-c").unwrap();
        cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::default());
        shell.read_with(cx, |shell, _| assert!(!shell.chat_hover_resync));
    }

    #[test]
    fn missing_duplicate_and_archived_pins_do_not_disturb_regular_rows() {
        let recency = ids(&["b", "a", "c"]);
        assert_eq!(
            project_pinned_first(&recency, &ids(&["archived", "a", "a"])),
            ids(&["a", "b", "c"])
        );
    }

    #[test]
    fn filtered_pin_reorder_preserves_every_other_pins_relative_order() {
        let saved = ids(&["a1", "b1", "a2", "archived", "b2"]);
        let visible = ids(&["a1", "a2"]);
        assert_eq!(
            reorder_visible_pins(&saved, &visible, 0, 1),
            ids(&["b1", "a2", "a1", "archived", "b2"])
        );
    }

    #[test]
    fn pin_reorder_rejects_invalid_or_noop_moves() {
        let saved = ids(&["a", "b"]);
        assert_eq!(reorder_visible_pins(&saved, &saved, 0, 0), saved);
        assert_eq!(reorder_visible_pins(&saved, &saved, 8, 0), saved);
    }

    #[test]
    fn pin_cleanup_retains_archived_and_prunes_deleted() {
        let mut saved = ids(&["active", "archived", "deleted", "active"]);
        let known = HashSet::from(["active".to_string(), "archived".to_string()]);
        assert!(retain_known_pins(&mut saved, &known));
        assert_eq!(saved, ids(&["active", "archived"]));
        assert!(!retain_known_pins(&mut saved, &known));
    }

    #[test]
    fn pin_cleanup_for_one_profile_leaves_other_profiles_untouched() {
        let mut settings = crate::settings::UiSettings::default();
        settings
            .sidebar_pins_mut("local".to_string())
            .extend(ids(&["local-active", "local-deleted"]));
        settings
            .sidebar_pins_mut("synced:org-1:user-1".to_string())
            .push("synced-pin".to_string());

        let known = HashSet::from(["local-active".to_string()]);
        assert!(retain_known_pins(
            settings.sidebar_pins_mut("local".to_string()),
            &known,
        ));
        assert_eq!(settings.sidebar_pins("local"), ["local-active"]);
        assert_eq!(settings.sidebar_pins("synced:org-1:user-1"), ["synced-pin"]);
    }

    #[test]
    fn sidebar_hit_testing_uses_compact_and_mixed_row_heights() {
        assert_eq!(super::row_drop_index(31.0, &[29.0, 29.0], false), Some(1));
        assert_eq!(super::row_drop_index(61.0, &[29.0, 29.0], false), None);
        assert_eq!(super::row_drop_index(48.0, &[45.0, 63.0], false), Some(1));
        assert_eq!(super::row_drop_index(-1.0, &[29.0], false), None);
        assert_eq!(super::row_drop_index(-1.0, &[29.0], true), Some(0));
        assert_eq!(super::row_drop_index(500.0, &[29.0, 45.0], true), Some(1));
    }

    #[test]
    fn pinned_drop_index_quantizes_clamps_and_rejects_outside() {
        assert_eq!(pinned_session_drop_index(-1.0, 3), None);
        assert_eq!(pinned_session_drop_index(0.0, 3), Some(0));
        assert_eq!(
            pinned_session_drop_index(super::super::SIDEBAR_SESSION_SLOT, 3),
            Some(1)
        );
        assert_eq!(pinned_session_drop_index(500.0, 3), None);
        assert_eq!(pinned_session_drop_index(0.0, 0), None);
    }

    #[test]
    fn pinned_drag_accounts_for_disclosure_header_and_scroll() {
        // The first row starts 40px below the scroll viewport: 4px list
        // padding, 32px header, and 4px body inset.
        let viewport_top = 100.0;
        let row_top = 140.0;
        assert_eq!(
            super::pinned_session_pointer_y(row_top, viewport_top, 0.0),
            0.0
        );
        assert_eq!(
            pinned_session_drop_index(super::pinned_session_pointer_y(125.0, viewport_top, 0.0), 3),
            None,
        );
        assert_eq!(
            pinned_session_drop_index(
                super::pinned_session_pointer_y(row_top, viewport_top, 83.0),
                3
            ),
            Some(1),
        );
    }

    #[test]
    fn sidebar_wide_pin_drag_clamps_to_the_nearest_pinned_slot() {
        assert_eq!(pinned_session_clamped_index(-50.0, 3), Some(0));
        assert_eq!(
            pinned_session_clamped_index(super::super::SIDEBAR_SESSION_SLOT, 3),
            Some(1)
        );
        assert_eq!(pinned_session_clamped_index(500.0, 3), Some(2));
        assert_eq!(pinned_session_clamped_index(0.0, 0), None);
    }

    #[test]
    fn session_transfers_only_change_pin_membership_and_order() {
        use super::{SidebarSessionDrop, sidebar_session_drop_pins};
        let saved = ids(&["hidden", "a", "b", "hidden-tail"]);
        let visible = ids(&["a", "b"]);
        assert_eq!(
            sidebar_session_drop_pins(&saved, &visible, "normal", SidebarSessionDrop::Regular),
            saved
        );
        assert_eq!(
            sidebar_session_drop_pins(&saved, &visible, "normal", SidebarSessionDrop::Pinned(1)),
            ids(&["hidden", "a", "normal", "b", "hidden-tail"])
        );
        assert_eq!(
            sidebar_session_drop_pins(&saved, &visible, "normal", SidebarSessionDrop::Pinned(2)),
            ids(&["hidden", "a", "b", "normal", "hidden-tail"])
        );
        assert_eq!(
            sidebar_session_drop_pins(&saved, &visible, "a", SidebarSessionDrop::Regular),
            ids(&["hidden", "b", "hidden-tail"])
        );
        assert_eq!(
            sidebar_session_drop_pins(&saved, &visible, "a", SidebarSessionDrop::Pinned(1)),
            ids(&["hidden", "b", "a", "hidden-tail"])
        );
        assert_eq!(
            sidebar_session_drop_pins(&[], &[], "first", SidebarSessionDrop::Pinned(0)),
            ids(&["first"])
        );
        assert!(
            sidebar_session_drop_pins(
                &ids(&["only"]),
                &ids(&["only"]),
                "only",
                SidebarSessionDrop::Regular
            )
            .is_empty()
        );
    }

    #[test]
    fn pinned_edge_scroll_is_proportional_and_lifecycle_bound() {
        let top = 100.0;
        let bottom = 300.0;
        assert_eq!(pinned_drag_scroll_delta(200.0, top, bottom), 0.0);
        assert_eq!(pinned_drag_scroll_delta(124.0, top, bottom), -6.0);
        assert_eq!(pinned_drag_scroll_delta(276.0, top, bottom), 6.0);
        assert_eq!(
            pinned_drag_scroll_step(true, 4, 4, 20.0, 100.0, 6.0),
            Some(26.0)
        );
        assert_eq!(pinned_drag_scroll_step(false, 4, 4, 20.0, 100.0, 6.0), None);
        assert_eq!(pinned_drag_scroll_step(true, 3, 4, 20.0, 100.0, 6.0), None);
    }

    #[test]
    fn pinned_drag_snapshot_requires_the_same_remote_order() {
        let snapshot = ids(&["a", "b"]);
        assert!(pinned_drag_snapshot_is_valid("a", &snapshot, &snapshot));
        assert!(!pinned_drag_snapshot_is_valid(
            "a",
            &snapshot,
            &ids(&["b", "a"])
        ));
        assert!(!pinned_drag_snapshot_is_valid("a", &snapshot, &ids(&["a"])));
    }
}

pub(super) fn pinned_drag_scroll_step(
    drag_active: bool,
    loop_generation: u64,
    drag_generation: u64,
    current: f32,
    max: f32,
    delta: f32,
) -> Option<f32> {
    if !drag_active || loop_generation != drag_generation || delta == 0.0 {
        return None;
    }
    let next = (current + delta).clamp(0.0, max.max(0.0));
    (next != current).then_some(next)
}

pub(super) fn pinned_drag_snapshot_is_valid(
    dragged_id: &str,
    snapshot_ids: &[String],
    current_ids: &[String],
) -> bool {
    snapshot_ids.iter().any(|id| id == dragged_id) && snapshot_ids == current_ids
}

struct ActiveChatRow {
    status: ChatIndicator,
    chat: zeron_proto::Chat,
    folder: String,
    branch: Option<String>,
    change_request: Option<zeron_proto::ChangeRequestSummary>,
    group: Option<(String, String)>,
    created_by: Option<String>,
}

impl ActiveChatRow {
    fn height(&self, compact: bool, slim: bool, show_label: bool) -> f32 {
        sidebar_row_height(compact, slim, show_label)
    }
}

pub(super) fn compare_sidebar_chats(
    sort: SidebarSort,
    left: &zeron_proto::Chat,
    right: &zeron_proto::Chat,
) -> std::cmp::Ordering {
    let primary = match sort {
        SidebarSort::Created => right.created_at.cmp(&left.created_at),
        SidebarSort::LastUpdated => right
            .last_message_at
            .unwrap_or(right.created_at)
            .cmp(&left.last_message_at.unwrap_or(left.created_at)),
    };
    primary.then_with(|| left.id.cmp(&right.id))
}

pub(super) fn compare_sidebar_rows(
    sort: SidebarSort,
    active_first: bool,
    left: (ChatIndicator, &zeron_proto::Chat),
    right: (ChatIndicator, &zeron_proto::Chat),
) -> std::cmp::Ordering {
    let active = |status: ChatIndicator| active_first && status != ChatIndicator::Idle;
    active(right.0)
        .cmp(&active(left.0))
        .then_with(|| compare_sidebar_chats(sort, left.1, right.1))
}

#[derive(Clone, Copy)]
pub(super) struct InboxMark {
    working: bool,
    since: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum InboxShelf {
    Inbox,
    Working,
    Settled,
}

impl InboxShelf {
    fn of(status: ChatIndicator, settled_at: Option<DateTime<Utc>>, working_shelf: bool) -> Self {
        if settled_at.is_some() {
            Self::Settled
        } else if working_shelf && status == ChatIndicator::Working {
            Self::Working
        } else {
            Self::Inbox
        }
    }

    fn group(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Inbox => Some(("inbox", "Inbox")),
            Self::Working => Some(("working", "Working")),
            Self::Settled => Some(("settled", "Settled")),
        }
    }
}

pub(super) fn inbox_settled_at(
    status: ChatIndicator,
    chat: &zeron_proto::Chat,
    after_days: u32,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if matches!(
        status,
        ChatIndicator::Working | ChatIndicator::AwaitingInput
    ) {
        return None;
    }
    chat.settled_at(after_days, now)
}

pub(super) fn sidebar_settled_at(
    state: &AppState,
    status: ChatIndicator,
    chat: &zeron_proto::Chat,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    inbox_settled_at(status, chat, state.sidebar_settings.settle_after_days, now)
}

pub(super) fn inbox_rank(
    status: ChatIndicator,
    chat: &zeron_proto::Chat,
    started_at: Option<DateTime<Utc>>,
    mark: Option<InboxMark>,
    settled_at: Option<DateTime<Utc>>,
    working_shelf: bool,
) -> (InboxShelf, std::cmp::Reverse<DateTime<Utc>>) {
    let shelf = InboxShelf::of(status, settled_at, working_shelf);
    if let Some(at) = settled_at {
        return (shelf, std::cmp::Reverse(at));
    }
    let anchor = chat
        .unsettled_at
        .map_or(chat.created_at, |at| at.max(chat.created_at));
    if !working_shelf {
        return (shelf, std::cmp::Reverse(anchor));
    }
    let working = shelf == InboxShelf::Working;
    let since = mark
        .filter(|mark| mark.working == working)
        .and_then(|mark| mark.since);
    let base = chat.last_message_at.map_or(anchor, |at| at.max(anchor));
    let key = if working {
        started_at.or(since).unwrap_or(base)
    } else {
        since.map_or(base, |at| at.max(base))
    };
    (shelf, std::cmp::Reverse(key))
}

/// The space-filter dropdown, `Some` while open. The same searchable-menu
/// recipe as the composer's ref picker: filter input on top
/// (`PaletteSearch` context so ↑↓/⏎ bubble to the card), ranked substring
/// rows, keyboard highlight.
pub(super) struct SpacesMenu {
    search: Entity<ComposerInput>,
    /// Keyboard highlight — an index into [`Shell::spaces_menu_rows`], or
    /// that list's length when the pinned "New project" footer holds it.
    active: usize,
    /// Tracked on the card — puts it on the keyboard dispatch path while the
    /// search input holds focus (the structure every working picker uses).
    focus: FocusHandle,
    list_scroll: gpui::ScrollHandle,
    _search_events: Subscription,
}

pub(super) struct SidebarViewMenu {
    submenu: Option<usize>,
    submenu_active: Option<usize>,
    submenu_bounds: Option<gpui::Bounds<gpui::Pixels>>,
    submenu_on_left: bool,
    hover_intent: popover::HoverIntent<usize>,
    /// Keyboard cursor. Mouse-opened menus start without one so the persisted
    /// radio/check state is the only selection signal until an arrow key is
    /// pressed.
    active: Option<usize>,
    focus: FocusHandle,
}

struct SidebarViewOptionsTooltip;

impl Render for SidebarViewOptionsTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.surface_raised)
            .shadow_md()
            .text_size(crate::typography::ui_rems(11.0))
            .text_color(theme.text)
            .child("Sidebar view options")
    }
}

#[derive(Clone, Copy)]
enum SidebarViewRow {
    ByProject,
    Compact,
    ShowProjectIcon,
    ShowProjectLabel,
    ByDevice,
    InOneList,
    Inbox,
    LastUpdated,
    Created,
    ActiveFirst,
    ShowBranch,
    ShowPullRequest,
    ShowHarness,
    ShowProjectFilter,
    ShowWorking,
    ShowArchived,
}

impl SidebarViewRow {
    /// Radio choices dismiss their submenu after selection. Show toggles
    /// stay open for batch changes.
    fn closes_submenu(self) -> bool {
        matches!(
            self,
            Self::ByProject
                | Self::ByDevice
                | Self::InOneList
                | Self::Inbox
                | Self::LastUpdated
                | Self::Created
        )
    }

    fn inert(self, inbox: bool) -> bool {
        match self {
            Self::LastUpdated | Self::Created | Self::ActiveFirst | Self::ShowArchived => inbox,
            Self::ShowWorking => !inbox,
            _ => false,
        }
    }
}

const SIDEBAR_VIEW_GROUPS: [(&str, std::ops::Range<usize>); 3] =
    [("Organize", 0..4), ("Sort", 4..7), ("Show", 7..15)];

const SIDEBAR_VIEW_ROWS: [SidebarViewRow; 15] = [
    SidebarViewRow::ByDevice,
    SidebarViewRow::ByProject,
    SidebarViewRow::InOneList,
    SidebarViewRow::Inbox,
    SidebarViewRow::LastUpdated,
    SidebarViewRow::Created,
    SidebarViewRow::ActiveFirst,
    SidebarViewRow::ShowBranch,
    SidebarViewRow::ShowPullRequest,
    SidebarViewRow::ShowHarness,
    SidebarViewRow::ShowProjectIcon,
    SidebarViewRow::ShowProjectLabel,
    SidebarViewRow::ShowProjectFilter,
    SidebarViewRow::ShowWorking,
    SidebarViewRow::ShowArchived,
];

// list items stay tightly related at 2px, while section boundaries use 12px
// (well over 2x the intra-list gap). Disclosure content gets a small 4px
// handoff from its header without leaving dead space while collapsed.
pub(super) const SIDEBAR_SECTION_GAP: f32 = 12.0;
pub(super) const SIDEBAR_DISCLOSURE_HEADER_HEIGHT: f32 = 32.0;
pub(super) const SIDEBAR_DISCLOSURE_BODY_INSET: f32 = 4.0;
const SIDEBAR_DISCLOSURE_SECTION_HEIGHT: f32 =
    SIDEBAR_SECTION_GAP + SIDEBAR_DISCLOSURE_HEADER_HEIGHT;
/// The Archived and Settled shelves show this many rows, then "Show more"
/// pages by [`SHELF_PAGE_ROWS`].
const SHELF_INITIAL_ROWS: usize = 10;
const SHELF_PAGE_ROWS: usize = 25;

fn sidebar_show_more_row(
    id: &'static str,
    remaining: usize,
    height: f32,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(height))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(Theme::SPACE_SM))
        .rounded(px(6.0))
        .text_size(crate::typography::ui_rems(13.0))
        .text_color(theme.text_muted.opacity(0.55))
        .cursor_pointer()
        .hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
        .child(
            crate::icons::icon(crate::icons::PLUS)
                .size(px(14.0))
                .text_color(theme.text_muted.opacity(0.55)),
        )
        .child(SharedString::from(format!("Show {remaining} more")))
}

const CHATS_GROUP_KEY: &str = "chats";
const NEW_CHAT_DIMMED_OPACITY: f32 = 0.45;

/// Put one group first (this machine's device, or the projectless Chats)
/// without disturbing the recency-based order of the others. A targeted
/// promotion is more truthful than a full name sort: that context leads, then
/// the user's chosen chat sort wins.
fn promote_group<T>(groups: &mut Vec<(Option<(String, String)>, Vec<T>)>, key: Option<&str>) {
    let Some(key) = key else {
        return;
    };
    let Some(index) = groups.iter().position(|(group, _)| {
        group
            .as_ref()
            .is_some_and(|(group_key, _)| group_key == key)
    }) else {
        return;
    };
    if index > 0 {
        let local = groups.remove(index);
        groups.insert(0, local);
    }
}

/// The chat ids in the order the sidebar draws them: the sorted input
/// regrouped exactly as `render_active_rows` groups it (project or device
/// sections in first-seen order, local device promoted), or unchanged when the
/// organization draws one flat list.
fn grouped_visible_order(
    chats: Vec<zeron_proto::Chat>,
    organization: SidebarOrganization,
    filtered: bool,
    local_device_id: Option<&str>,
    project_group: impl Fn(&zeron_proto::Chat) -> String,
) -> Vec<String> {
    let key = |chat: &zeron_proto::Chat| -> Option<String> {
        match organization {
            SidebarOrganization::ByProject if !filtered => Some(project_group(chat)),
            SidebarOrganization::ByDevice => Some(chat.device_id.clone()),
            SidebarOrganization::ByProject
            | SidebarOrganization::InOneList
            | SidebarOrganization::Inbox => None,
        }
    };
    let mut groups: Vec<(Option<(String, String)>, Vec<zeron_proto::Chat>)> = Vec::new();
    for chat in chats {
        let key = key(&chat).map(|k| (k, String::new()));
        if let Some((_, existing)) = groups.iter_mut().find(|(group, _)| group == &key) {
            existing.push(chat);
        } else {
            groups.push((key, vec![chat]));
        }
    }
    match organization {
        SidebarOrganization::ByDevice => promote_group(&mut groups, local_device_id),
        SidebarOrganization::ByProject => promote_group(&mut groups, Some(CHATS_GROUP_KEY)),
        SidebarOrganization::InOneList | SidebarOrganization::Inbox => {}
    }
    groups
        .into_iter()
        .flat_map(|(_, rows)| rows)
        .map(|chat| chat.id)
        .collect()
}

/// Project-mode group of a chat as (key, label). Projects sharing a
/// repository identity fold into one group named for their representative
/// space; project-less sessions on every device share one "Chats" group.
fn sidebar_project_group(state: &AppState, chat: &zeron_proto::Chat) -> (String, String) {
    match state.space_for_chat(chat) {
        Some(space) => (
            zeron_proto::view::project_key(space),
            state.representative_space(space).display_name().to_string(),
        ),
        None => match chat.space_id.clone() {
            Some(space_id) => (space_id, "?".to_string()),
            None => (CHATS_GROUP_KEY.to_string(), "Chats".to_string()),
        },
    }
}

/// Shared quiet rule for sidebar groups and palette sections.
pub(super) fn sidebar_separator(theme: &Theme) -> gpui::Div {
    div().h(px(1.0)).bg(theme.border.opacity(0.6))
}

/// `icon` leads the label — a project group's icon, which its rows then omit.
/// Every sidebar section's header (Pinned, Sessions, Archived, project
/// groups and custom sections): a muted label, `count` as a muted badge
/// beside it while the section is collapsed, then any action and the
/// disclosure chevron.
pub(super) fn sidebar_disclosure_header(
    theme: &Theme,
    icon: Option<AnyElement>,
    label: SharedString,
    count: Option<usize>,
    action: Option<AnyElement>,
    chevron: AnyElement,
) -> gpui::Div {
    let label: SharedString = match count {
        Some(count) => format!("{label} ({count})").into(),
        None => label,
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(SIDEBAR_DISCLOSURE_HEADER_HEIGHT))
        .mx(px(2.0))
        .px(px(8.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .children(icon)
        .child(
            div()
                .flex_none()
                .max_w(px(180.0))
                .child(super::sidebar_faded_label(
                    "sidebar-disclosure-label".into(),
                    false,
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .line_height(px(16.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(sidebar_header_glyph(theme))
                        .child(label),
                )),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(8.0))
                .h(px(1.0))
                .bg(theme.border.opacity(0.6)),
        )
        .children(action)
        .child(chevron)
}

pub(super) fn sidebar_header_glyph(theme: &Theme) -> gpui::Hsla {
    theme.text_muted.opacity(0.6)
}

/// One activatable row of the open dropdown, in nav order. `AddSpace` names
/// the card's pinned "New project" footer, not a list row — keyboard nav
/// maps the list-length index to it.
#[derive(Clone, PartialEq)]
pub(super) enum SpacesMenuRow {
    All,
    Space(String),
    AddSpace,
}

/// New project navigates devices, locations, then folders on a command-palette surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProjectStep {
    Devices,
    Locations,
    Folders,
}

pub(super) struct AddSpaceFlow {
    step: ProjectStep,
    location: Option<(String, Option<String>)>,
    /// The selected device.
    device: Option<Device>,
    /// Filter input; Enter descends into the highlighted folder. Carries the
    /// tab-completion ghost (the faint suffix ⇥ accepts), and a trailing `/`
    /// on a folder-naming query descends immediately.
    search: Entity<ComposerInput>,
    browser: Loadable<FolderListing>,
    /// The selected device's mounted drives/volumes.
    /// Best-effort: an error just leaves the section at Home only.
    drives: Loadable<Vec<DriveEntry>>,
    /// Requested browser path (`None` = the device's default, i.e. home).
    browser_path: Option<String>,
    /// The device's home (the path a `None` browse resolved to) — breadcrumbs
    /// fold everything up to here into the Home crumb.
    home: Option<String>,
    /// Best-effort git seed for the CURRENT browser path (known when we
    /// descended through an entry whose `is_repo` we saw; the owning device's
    /// SpacesSync re-verifies either way).
    browser_repo: bool,
    /// Keyboard highlight within the current step’s filtered rows.
    active: usize,
    submit_busy: bool,
    error: Option<SharedString>,
    /// Tracked on the card (`track_focus`) — puts the card on the keyboard
    /// dispatch path so ↑↓/⌫/esc reach `add_space_key` while the search input
    /// holds focus (the structure every working picker uses).
    focus: FocusHandle,
    /// Folder-list scroll — keyboard navigation keeps the highlighted row in
    /// view (`scroll_to_item`).
    list_scroll: gpui::ScrollHandle,
    /// Horizontal breadcrumb strip; `crumb_key` is the path it last revealed.
    crumb_scroll: gpui::ScrollHandle,
    crumb_key: String,
    focus_pending: bool,
    load_task: Option<Task<()>>,
    drives_task: Option<Task<()>>,
    submit_task: Option<Task<()>>,
    /// Folders matching the query anywhere under the device's home (Home
    /// location only).
    home_search: HomeFolderSearch,
    _search_events: Subscription,
}

/// Shorter queries match nearly every folder under home; they are not sent.
const HOME_FOLDER_MIN_QUERY_CHARS: usize = 2;
const HOME_FOLDER_RESULTS: u16 = 20;
const HOME_FOLDER_INDEX_RETRY: Duration = Duration::from_millis(500);
const HOME_FOLDER_INDEX_RETRY_MAX: Duration = Duration::from_secs(2);

/// The new-project picker's search over the device's home, answered by the
/// host's home search index (the one projectless chats use). Entering Home
/// starts the index; leaving Home or closing the picker releases it unless a
/// projectless chat warmed it.
#[derive(Default)]
struct HomeFolderSearch {
    /// Bumped per request; an answer to an older one is dropped.
    generation: u64,
    loading: bool,
    /// The host's index was still scanning at the last answer.
    indexing: bool,
    results: Vec<HomeFolderMatch>,
    /// A request reached the host, so it may hold an index to release.
    started: bool,
    /// The host predates `SearchHomeFolders`; the section stays hidden.
    unsupported: bool,
    task: Option<Task<()>>,
    /// The request that started the host's index.
    warm_task: Option<Task<()>>,
}

/// Refresh `SearchHomeFolders` until indexing finishes, backing off between
/// replies. `generation` identifies the query; `None` is a one-shot warm-up
/// whose answer only tells whether the host supports the method.
fn request_home_folders(
    engine: crate::state::EngineHandle,
    device_id: Option<String>,
    local: Option<String>,
    query: String,
    generation: Option<u64>,
    cx: &mut Context<Shell>,
) -> Task<()> {
    let request = SearchHomeFoldersRequest {
        query,
        limit: Some(HOME_FOLDER_RESULTS),
    };
    cx.spawn(async move |this, cx| {
        let mut params = match serde_json::to_value(&request) {
            Ok(serde_json::Value::Object(params)) => params,
            _ => return,
        };
        add_space_target(&mut params, device_id.as_deref(), local.as_deref());
        let params = serde_json::Value::Object(params);
        let mut delay = HOME_FOLDER_INDEX_RETRY;
        loop {
            let result = engine
                .client()
                .call(methods::SEARCH_HOME_FOLDERS, params.clone())
                .await;
            let retry = this
                .update(cx, |shell, cx| {
                    let selected = shell
                        .add_space
                        .as_ref()
                        .and_then(|flow| shell.add_space_folder_rows(cx).get(flow.active).cloned());
                    let Some(search) = shell.add_space.as_mut().map(|flow| &mut flow.home_search)
                    else {
                        return false;
                    };
                    if generation.is_some() && generation != Some(search.generation) {
                        return false;
                    }
                    if let Err(zeron_rpc::RpcError::UnknownMethod(_)) = &result {
                        search.unsupported = true;
                        search.started = false;
                        search.loading = false;
                        search.indexing = false;
                        cx.notify();
                        return false;
                    }
                    if generation.is_none() || search.unsupported {
                        return false;
                    }
                    search.loading = false;
                    search.indexing = false;
                    match result.map(serde_json::from_value::<HomeFolderSearchResult>) {
                        Ok(Ok(found)) => {
                            search.indexing = found.indexing;
                            search.results = found.matches;
                        }
                        Ok(Err(error)) => {
                            tracing::debug!(%error, "home folder search answer did not decode");
                        }
                        Err(error) => tracing::debug!(%error, "home folder search failed"),
                    }
                    let retry = search.indexing;
                    // New partial results may reorder the rows under the cursor.
                    if let Some(selected) = selected
                        && let Some(active) = shell
                            .add_space_folder_rows(cx)
                            .iter()
                            .position(|row| row == &selected)
                        && let Some(flow) = shell.add_space.as_mut()
                    {
                        flow.active = active;
                    }
                    cx.notify();
                    retry
                })
                .unwrap_or(false);
            if !retry {
                break;
            }
            // The stored task is dropped on edits, navigation or close. Wait
            // after the reply so slow hosts never accumulate parallel calls.
            cx.background_executor().timer(delay).await;
            delay = (delay * 2).min(HOME_FOLDER_INDEX_RETRY_MAX);
            let current = this
                .update(cx, |shell, _| {
                    shell.add_space_in_home()
                        && shell.add_space.as_ref().is_some_and(|flow| {
                            generation == Some(flow.home_search.generation)
                                && !flow.home_search.unsupported
                        })
                })
                .unwrap_or(false);
            if !current {
                break;
            }
        }
    })
}

/// One row of the Folders step, in navigation order.
#[derive(Clone, Debug, PartialEq)]
enum FolderRow {
    /// A subfolder of the open folder, matched by name.
    Local(zeron_proto::FolderEntry),
    /// A folder anywhere under home.
    Home(HomeFolderMatch),
}

/// `targetDeviceId` for a remote device; local calls skip the relay.
fn add_space_target(
    params: &mut serde_json::Map<String, serde_json::Value>,
    device_id: Option<&str>,
    local: Option<&str>,
) {
    if let Some(target) = device_id
        && local != Some(target)
    {
        params.insert(
            "targetDeviceId".into(),
            serde_json::Value::String(target.to_owned()),
        );
    }
}

/// The title over the picker's matches from under home, with the host's
/// search status trailing.
fn home_folders_header(theme: &Theme, status: Option<&'static str>) -> gpui::Div {
    div()
        .px(px(8.0))
        .pt(px(2.0))
        .pb(px(4.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.text_muted)
        .child("In Home")
        .when_some(status, |header, status| {
            header.child(div().opacity(0.7).child(status))
        })
}

/// A folder from under home: its name (query highlighted), then where it
/// lives (`~/…`), marked when it is a repository.
fn home_folder_row(
    row: gpui::Stateful<gpui::Div>,
    found: &HomeFolderMatch,
    query: &str,
    theme: &Theme,
    cx: &Context<Shell>,
) -> gpui::Stateful<gpui::Div> {
    let (name, parent) = match found.relative.rsplit_once('/') {
        Some((parent, name)) => (name, format!("~/{parent}")),
        None => (found.relative.as_str(), "~".to_owned()),
    };
    let (path, is_repo) = (found.path.clone(), found.is_repo);
    row.aria_label(SharedString::from(format!("{name} in {parent}")))
        .child(
            icon(icons::FOLDER)
                .size(px(16.0))
                .flex_none()
                .text_color(theme.text_muted),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.0))
                .overflow_hidden()
                .child(div().flex_none().child(popover::search_highlight(
                    SharedString::from(name.to_owned()),
                    Some(query),
                    theme,
                )))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.text_muted)
                        .child(SharedString::from(parent)),
                ),
        )
        .when(is_repo, |el| {
            el.child(
                icon(icons::GIT_BRANCH)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(theme.text_muted),
            )
        })
        .on_click(
            cx.listener(move |this, _, _, cx| this.add_space_descend(path.clone(), is_repo, cx)),
        )
}

/// Folder crumbs shown before the middle folds into `…`, and how many of the
/// deepest stay visible once it does.
const CRUMB_FOLDERS_MAX: usize = 3;
const CRUMB_FOLDERS_TAIL: usize = 2;

/// Fold the middle of a deep trail: returns the folders hidden behind `…`,
/// leaving the deepest [`CRUMB_FOLDERS_TAIL`] in `folders`.
fn fold_crumb_folders<T>(folders: &mut Vec<T>) -> Vec<T> {
    if folders.len() <= CRUMB_FOLDERS_MAX {
        return Vec::new();
    }
    folders
        .drain(..folders.len() - CRUMB_FOLDERS_TAIL)
        .collect()
}

struct Crumb {
    name: SharedString,
    glyph: Option<&'static str>,
    current: bool,
    target: CrumbTarget,
}

enum CrumbTarget {
    Devices,
    Locations,
    Location(String, Option<String>),
    Folder(String),
    /// The `…` fold; opens [`Shell::render_project_crumb_menu`].
    More,
}

fn device_glyph(platform: &str) -> &'static str {
    match platform {
        "macos" | "darwin" => icons::LAPTOP,
        "web" => icons::GLOBAL,
        "ios" | "android" => icons::SMARTPHONE,
        _ => icons::MONITOR,
    }
}

/// Segment-aware "is `path` at or under `base`" (`/media/a` is not under
/// `/media/ab`); a root base covers everything. Either separator counts, so
/// Windows drive paths (`D:\` under `D:\`) work too.
fn path_under(path: &str, base: &str) -> bool {
    let path = folder_path_key(path);
    let base = folder_path_key(base);
    let base = base.trim_end_matches(['/', '\\']);
    base.is_empty()
        || path
            .strip_prefix(base)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '\\']))
}

/// The space-row Rename dialog (same shape as [`RenameChatDialog`]).
pub(super) struct RenameSpaceDialog {
    pub space_id: String,
    pub input: Entity<ComposerInput>,
    pub focus_pending: bool,
    pub _events: Subscription,
}

/// Dot color for a chat's display status (tab dots + Sessions rows).
pub(crate) fn status_dot_color(status: ChatIndicator, theme: &Theme) -> gpui::Hsla {
    match status {
        // Preset activity tone, not warning amber: running is routine.
        // Non-done statuses sit well below full
        // strength: at full alpha the colored words shouted across the
        // whole sidebar (user request) — only Done keeps its pop.
        ChatIndicator::Working => theme.busy.opacity(0.55),
        // Blue: "asking you a question" must read differently from "busy
        // working" at a glance.
        ChatIndicator::AwaitingInput => theme.accent.opacity(0.6),
        ChatIndicator::Errored => theme.danger.opacity(0.65),
        // Green: finished-but-unseen reads as "ready for you".
        ChatIndicator::Completed => {
            theme.success.opacity(0.9) // emerald-400
        }
        ChatIndicator::Idle => crate::theme::ink(0.14),
    }
}

// Handle-based rail host for the spaces dropdown: its list is a plain
// tracked scroller, so the trait's default metrics/press/drag (off the live
// ScrollHandle) apply unchanged.
impl popover::ScrollRailHost for Shell {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        &mut self.spaces_menu_bar
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.spaces_menu.get().map(|menu| menu.list_scroll.clone())
    }
}

impl Shell {
    pub(super) fn begin_sidebar_disclosure_motion(
        &mut self,
        key: &str,
        resting_height: f32,
        target_height: f32,
    ) {
        let previous = self.sidebar_disclosure_motion.get(key).copied();
        let from = previous
            .filter(|motion| motion.animating())
            .map(SidebarDisclosureMotion::current)
            .unwrap_or(resting_height);
        let epoch = previous.map_or(1, |motion| motion.epoch + 1);
        self.sidebar_disclosure_motion.insert(
            key.to_owned(),
            SidebarDisclosureMotion::new(epoch, from, target_height),
        );
    }

    pub(super) fn render_sidebar_disclosure_body(
        &self,
        key: &str,
        open: bool,
        full_height: f32,
        content: AnyElement,
    ) -> AnyElement {
        let target = if open { full_height } else { 0.0 };
        let frame = div().w_full().flex_none().overflow_hidden().child(content);
        let Some(tween) = self
            .sidebar_disclosure_motion
            .get(key)
            .copied()
            .filter(|motion| motion.animating())
        else {
            return frame.h(px(target)).into_any_element();
        };
        let denominator = full_height.max(1.0);
        frame
            .with_animation(
                SharedString::from(format!("sidebar-disclosure-{key}-{}", tween.epoch)),
                motion::COLLAPSE.animation(),
                move |el, t| {
                    let height = motion::lerp(tween.from, tween.to, t);
                    let reveal = (height / denominator).clamp(0.0, 1.0);
                    el.h(px(height))
                        .opacity(0.35 + 0.65 * reveal)
                        .relative()
                        .top(px(-3.0 * (1.0 - reveal)))
                },
            )
            .into_any_element()
    }

    pub(super) fn sidebar_disclosure_chevron(
        &self,
        key: &str,
        open: bool,
        theme: &Theme,
    ) -> AnyElement {
        let resting_reveal = if open { 1.0 } else { 0.0 };
        let chevron = icon(icons::LUCIDE_CHEVRON_DOWN)
            .size(px(12.0))
            .text_color(sidebar_header_glyph(theme));
        if let Some(tween) = self
            .sidebar_disclosure_motion
            .get(key)
            .copied()
            .filter(|motion| motion.animating())
        {
            let denominator = tween.from.max(tween.to).max(1.0);
            let from = (tween.from / denominator).clamp(0.0, 1.0);
            let to = (tween.to / denominator).clamp(0.0, 1.0);
            div()
                .flex_none()
                .size(px(12.0))
                .child(chevron.with_animation(
                    SharedString::from(format!("sidebar-chevron-{key}-{}", tween.epoch)),
                    motion::COLLAPSE.animation(),
                    move |el, t| {
                        let reveal = motion::lerp(from, to, t);
                        el.with_transformation(gpui::Transformation::rotate(gpui::percentage(
                            (1.0 - reveal) * 0.5,
                        )))
                    },
                ))
                .into_any_element()
        } else {
            div()
                .flex_none()
                .size(px(12.0))
                .child(
                    chevron.with_transformation(gpui::Transformation::rotate(gpui::percentage(
                        (1.0 - resting_reveal) * 0.5,
                    ))),
                )
                .into_any_element()
        }
    }
    // ---- space filter ----

    /// Set the sidebar's session filter (`None` = All spaces). On the
    /// new-session canvas the space context follows the filter — the canvas
    /// default is "the space you're looking at".
    pub(super) fn set_space_filter(&mut self, filter: Option<String>, cx: &mut Context<Self>) {
        if self.settings.space_filter != filter {
            self.cancel_pinned_session_drag(cx);
        }
        self.settings.space_filter = filter.clone();
        if let Some(space_id) = filter
            && self.state.read(cx).selected_chat.is_none()
        {
            self.state
                .update(cx, |s, cx| s.select_space(Some(space_id), cx));
        }
        self.close_spaces_menu(cx);
        self.schedule_save(cx);
        if let Some(page) = self.pull_requests_page.as_ref() {
            page.update(cx, |page, cx| page.on_project_scope_changed(cx));
        }
        cx.notify();
    }

    /// Close the space-filter dropdown through the exit animation (no-op when
    /// it isn't open). Every close path funnels here so the menu always
    /// animates out instead of vanishing.
    pub(super) fn close_spaces_menu(&mut self, cx: &mut Context<Self>) {
        if self.spaces_menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.spaces_menu);
            cx.notify();
        }
    }

    fn on_spaces_menu_list_hover(
        &mut self,
        hovered: &bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.spaces_menu_bar.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    /// Open the new-session canvas in a just-added space, preserving the
    /// sidebar's current project filter.
    pub(super) fn land_in_space(&mut self, space_id: String, cx: &mut Context<Self>) {
        self.cancel_pinned_session_drag(cx);
        self.set_route(Route::Chat, cx);
        self.focus_composer(cx);
        // "All" stays as-is; an explicit project filter follows the new
        // project so the first send lands in a visible session.
        if self.settings.space_filter.is_some() {
            self.settings.space_filter = Some(space_id.clone());
        }
        self.settings.last_space_id = Some(space_id.clone());
        self.state.update(cx, |s, cx| {
            // Draft first, so the space records onto the canvas draft and
            // never onto an invested one being left (see open_new_session).
            s.select_chat(None, cx);
            s.new_draft(cx);
            s.select_space(Some(space_id), cx);
        });
        let entry = self.focus_nav_entry(cx);
        self.record_nav_focus(entry, cx);
        self.freeze_draft_row(cx);
        self.schedule_save(cx);
        cx.notify();
    }

    // ---- sidebar sections ----

    pub(super) fn sidebar_transfer_extra_gap(&self, group: &str) -> f32 {
        self.sidebar_session_transfer
            .as_ref()
            .or_else(|| {
                self.sidebar_session_return
                    .as_ref()
                    .map(|state| &state.transfer)
            })
            .and_then(|drag| drag.section_gaps.get(group))
            .map_or(0.0, |gap| {
                if self.reduced_motion {
                    gap.to
                } else {
                    gap.current()
                }
            })
    }

    fn render_sidebar_gap_row(
        &mut self,
        row: AnyElement,
        id: &str,
        group: &str,
        index: usize,
    ) -> AnyElement {
        let returning = self.sidebar_session_transfer.is_none();
        let transfer = if let Some(drag) = self.sidebar_session_transfer.as_mut() {
            Some(drag)
        } else {
            self.sidebar_session_return
                .as_mut()
                .map(|returning| &mut returning.transfer)
        };
        let Some(drag) = transfer else {
            return row;
        };
        // Pin-to-pin keeps the existing sibling-slide implementation.
        let target = if returning || (group == "pinned" && drag.source_group == "pinned") {
            0.0
        } else {
            drag.preview
                .as_ref()
                .filter(|gap| gap.group == group)
                .map_or(0.0, |gap| {
                    sidebar_gap_offset(
                        index,
                        (drag.source_group == group).then_some(drag.source_index),
                        gap.index,
                        drag.row_height + SIDEBAR_LIST_GAP,
                    )
                })
        };
        let slide = drag
            .siblings
            .entry(id.to_owned())
            .or_insert_with(|| SidebarSessionSlide {
                from: 0.0,
                to: 0.0,
                epoch: drag.slide.epoch,
                started: std::time::Instant::now(),
            });
        slide.retarget(target);
        let (from, to, epoch) = (slide.from, slide.to, slide.epoch);
        let frame = div().relative().child(row);
        if !returning || self.reduced_motion {
            frame.top(px(to)).into_any_element()
        } else {
            frame
                .with_animation(
                    SharedString::from(format!("session-gap-{id}-{epoch}")),
                    TAB_SLIDE.animation(),
                    move |el, t| el.top(px(motion::lerp(from, to, t))),
                )
                .into_any_element()
        }
    }

    pub(super) fn begin_sidebar_session_transfer(
        &mut self,
        payload: &SidebarSessionDrag,
        cursor_offset: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.lift_command_palette() && !self.sidebar_session_transfer_is_valid(payload, cx) {
            cx.notify();
            return;
        }
        self.chat_status_hover = None;
        self.cancel_pinned_session_drag(cx);
        self.sidebar_session_return = None;
        self.pinned_session_drag_generation = self.pinned_session_drag_generation.wrapping_add(1);
        let lifted = window.mouse_position() - cursor_offset;
        self.sidebar_session_transfer = Some(SidebarSessionTransfer {
            payload: payload.clone(),
            origin: std::rc::Rc::new(std::cell::Cell::new(
                window.mouse_position() - cursor_offset,
            )),
            cursor_offset,
            pointer: window.mouse_position(),
            viewport: None,
            preview: None,
            source_group: String::new(),
            source_index: 0,
            row_height: 0.0,
            source_collapse: SidebarSessionSlide {
                from: 0.0,
                to: 0.0,
                epoch: 0,
                started: std::time::Instant::now(),
            },
            collapsed_height: 0.0,
            section_gaps: Default::default(),
            siblings: Default::default(),
            slide: SidebarSessionSlide {
                from: f32::from(lifted.y),
                to: f32::from(lifted.y),
                epoch: self.pinned_session_drag_generation << 32,
                started: std::time::Instant::now(),
            },
            slide_x: SidebarSessionSlide {
                from: f32::from(lifted.x),
                to: f32::from(lifted.x),
                epoch: self.pinned_session_drag_generation << 32,
                started: std::time::Instant::now(),
            },
        });
        {
            let origin = self
                .sidebar_session_transfer
                .as_ref()
                .unwrap()
                .origin
                .clone();
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(
                            SIDEBAR_DRAG_SCROLL_FRAME_MS,
                        ))
                        .await;
                    let keep_running = this
                        .update(cx, |shell, cx| {
                            let Some(transfer) = shell.sidebar_session_transfer.as_ref() else {
                                return false;
                            };
                            if !cx.has_active_drag() {
                                // A drop outside the sidebar never reaches its handlers.
                                cx.notify();
                                return false;
                            }
                            if !std::rc::Rc::ptr_eq(&origin, &transfer.origin) {
                                return false;
                            }
                            // Layout also animates while the pointer is stationary.
                            cx.notify();
                            // Pinned drags already have their own edge-scroll loop.
                            if transfer
                                .payload
                                .visible_ids
                                .contains(&transfer.payload.chat_id)
                            {
                                return true;
                            }
                            let Some(viewport) = transfer.viewport else {
                                return true;
                            };
                            if !viewport.contains(&transfer.pointer) {
                                return true;
                            }
                            let delta = pinned_drag_scroll_delta(
                                f32::from(transfer.pointer.y),
                                f32::from(viewport.top()),
                                f32::from(viewport.bottom()),
                            );
                            let offset = shell.sidebar_scroll.offset();
                            let scroll_top = -f32::from(offset.y);
                            let max_scroll = f32::from(shell.sidebar_scroll.max_offset().y);
                            let next = (scroll_top + delta).clamp(0.0, max_scroll);
                            if next != scroll_top {
                                shell
                                    .sidebar_scroll
                                    .set_offset(gpui::point(offset.x, px(-next)));
                                cx.notify();
                            }
                            true
                        })
                        .unwrap_or(false);
                    if !keep_running {
                        break;
                    }
                }
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn sidebar_session_transfer_is_valid(
        &self,
        payload: &SidebarSessionDrag,
        cx: &App,
    ) -> bool {
        if payload.filter != self.settings.space_filter
            || self.active_sidebar_pin_profile_key(cx).as_deref() != Some(&payload.profile_key)
        {
            return false;
        }
        let visible = self.sidebar_visible_chat_ids(payload.filter.as_deref(), cx);
        if !visible.contains(&payload.chat_id) {
            return false;
        }
        self.visible_sidebar_pins(&payload.profile_key, &visible, cx) == *payload.visible_ids
    }

    fn sidebar_visible_chat_ids(&self, filter: Option<&str>, cx: &App) -> HashSet<String> {
        self.state
            .read(cx)
            .sidebar_chats(Utc::now(), filter)
            .into_iter()
            .map(|(_, chat)| chat.id.clone())
            .collect()
    }

    fn visible_sidebar_pins(
        &self,
        profile_key: &str,
        visible: &HashSet<String>,
        cx: &App,
    ) -> Vec<String> {
        self.sidebar_pins_for_profile(profile_key, cx)
            .into_iter()
            .filter(|id| visible.contains(id))
            .collect()
    }

    pub(super) fn palette_session_drag(&self, chat_id: String, cx: &App) -> SidebarSessionDrag {
        let filter = self.settings.space_filter.clone();
        let profile_key = self.active_sidebar_pin_profile_key(cx).unwrap_or_default();
        let visible = self.sidebar_visible_chat_ids(filter.as_deref(), cx);
        SidebarSessionDrag {
            chat_id,
            visible_ids: std::sync::Arc::new(self.visible_sidebar_pins(&profile_key, &visible, cx)),
            filter,
            profile_key,
        }
    }

    pub(super) fn cancel_sidebar_session_transfer(&mut self, cx: &mut Context<Self>) {
        self.chat_status_hover = None;
        self.cancel_pinned_session_drag(cx);
        if let Some(mut transfer) = self.sidebar_session_transfer.take() {
            transfer.preview = None;
            if !self.reduced_motion && self.sidebar_session_transfer_is_valid(&transfer.payload, cx)
            {
                self.sidebar_session_return = Some(SidebarSessionReturn {
                    transfer,
                    epoch: self.pinned_session_drag_generation,
                    started: std::time::Instant::now(),
                });
                let epoch = self.pinned_session_drag_generation;
                cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(
                                SIDEBAR_DRAG_SCROLL_FRAME_MS,
                            ))
                            .await;
                        let keep_running = this
                            .update(cx, |shell, cx| {
                                let Some(returning) = shell.sidebar_session_return.as_ref() else {
                                    return false;
                                };
                                if returning.epoch != epoch {
                                    return false;
                                }
                                if returning.started.elapsed() >= TAB_SLIDE.total()
                                    && returning.transfer.slide.started.elapsed()
                                        >= TAB_SLIDE.total()
                                    && returning.transfer.source_collapse.started.elapsed()
                                        >= TAB_SLIDE.total()
                                    && returning
                                        .transfer
                                        .section_gaps
                                        .values()
                                        .all(|gap| gap.started.elapsed() >= TAB_SLIDE.total())
                                {
                                    shell.sidebar_session_return = None;
                                    cx.notify();
                                    return false;
                                }
                                cx.notify();
                                true
                            })
                            .unwrap_or(false);
                        if !keep_running {
                            break;
                        }
                    }
                })
                .detach();
                self.pinned_session_drag_generation =
                    self.pinned_session_drag_generation.wrapping_add(1);
            }
            cx.notify();
        }
    }

    pub(super) fn render_moving_sidebar_session(
        &mut self,
        row: AnyElement,
        height: f32,
        theme: &Theme,
    ) -> AnyElement {
        // Window coordinates: the row is drawn deferred and anchored, above
        // everything and outside the sidebar's clip, so it can be carried out
        // of the sidebar (onto a composer, to mention it). Elements live for
        // one frame, so it is built here, never stashed for another view.
        let list = self.sidebar_scroll.bounds();
        let width = (f32::from(list.size.width) - 2.0 * Theme::SPACE_SM).max(0.0);
        let returning = self.sidebar_session_transfer.is_none();
        let sidebar_width = self.settings.sidebar_width;
        let transfer = if let Some(transfer) = self.sidebar_session_transfer.as_mut() {
            transfer
        } else {
            &mut self.sidebar_session_return.as_mut().unwrap().transfer
        };
        let outside = !returning && f32::from(transfer.pointer.x) > sidebar_width;
        let origin = transfer.origin.get();
        let (target_x, target_y) = if returning {
            (f32::from(origin.x), f32::from(origin.y))
        } else {
            let lifted = transfer.pointer - transfer.cursor_offset;
            // Over the sidebar it keeps its column for reordering; anywhere
            // else it follows the pointer.
            let x = if transfer.pointer.x <= list.right() {
                origin.x
            } else {
                lifted.x
            };
            (f32::from(x), f32::from(lifted.y))
        };
        for (slide, target) in [
            (&mut transfer.slide, target_y),
            (&mut transfer.slide_x, target_x),
        ] {
            if returning {
                slide.retarget(target);
            } else {
                slide.from = target;
                slide.to = target;
                slide.started = std::time::Instant::now();
            }
        }
        let (from, to) = (transfer.slide.from, transfer.slide.to);
        let (from_x, to_x) = (transfer.slide_x.from, transfer.slide_x.to);
        let epoch = transfer.slide.epoch ^ transfer.slide_x.epoch.rotate_left(16);
        // Lifted, the row is frosted glass a level above the sidebar (the
        // queue's dragged row): what slides beneath reads as blurred, not
        // through its text. Returning to its slot it settles back unlifted.
        let row = if returning {
            row
        } else {
            crate::frost::frosted(
                8.0,
                crate::frost::MENU_BLUR,
                div()
                    .size_full()
                    .rounded(px(8.0))
                    .bg(crate::popover::surface_bg(theme))
                    .shadow_md()
                    .child(
                        div()
                            .size_full()
                            .rounded(px(8.0))
                            .bg(crate::theme::wash(0.06))
                            .child(row),
                    ),
            )
            .into_any_element()
        };
        let frame = div()
            .debug_selector(|| "sidebar-lifted-session".into())
            .relative()
            .w(px(width))
            .h(px(height))
            .rounded(px(8.0))
            .when(outside, |el| el.invisible())
            .child(row);
        // Anchored at the destination; a slide home eases its offset to zero.
        let frame = if self.reduced_motion {
            frame.into_any_element()
        } else {
            frame
                .with_animation(
                    ("sidebar-session-slide", epoch),
                    TAB_SLIDE.animation(),
                    move |el, t| {
                        el.left(px(motion::lerp(from_x - to_x, 0.0, t)))
                            .top(px(motion::lerp(from - to, 0.0, t)))
                    },
                )
                .into_any_element()
        };
        gpui::deferred(
            gpui::anchored()
                .position(gpui::point(px(to_x), px(to)))
                .child(frame),
        )
        .with_priority(1)
        .into_any_element()
    }

    pub(super) fn finish_sidebar_session_transfer(
        &mut self,
        payload: &SidebarSessionDrag,
        target: SidebarSessionDrop,
        cx: &mut Context<Self>,
    ) {
        self.chat_status_hover = None;
        let matches_drag = self.sidebar_session_transfer.as_ref().is_some_and(|drag| {
            drag.payload.chat_id == payload.chat_id
                && drag.payload.profile_key == payload.profile_key
                && drag.payload.filter == payload.filter
                && drag.payload.visible_ids == payload.visible_ids
        });
        if !matches_drag || !self.sidebar_session_transfer_is_valid(payload, cx) {
            self.cancel_sidebar_session_transfer(cx);
            return;
        }
        if let SidebarSessionDrop::Section(id) = &target {
            if !self
                .active_sidebar_sections(cx)
                .iter()
                .any(|section| &section.id == id)
            {
                self.cancel_sidebar_session_transfer(cx);
                return;
            }
        }
        let saved = self.raw_sidebar_pins(cx);
        let next = sidebar_session_drop_pins(
            &saved,
            &payload.visible_ids,
            &payload.chat_id,
            target.clone(),
        );
        let sections = self.active_sidebar_sections(cx);
        let source_section = sections
            .iter()
            .find(|section| section.session_ids.contains(&payload.chat_id))
            .map(|section| section.id.as_str());
        let target_section = match &target {
            SidebarSessionDrop::Section(id) => Some(id.as_str()),
            _ => None,
        };
        // Sections keep a placed order, as Pinned does: where the drop lands
        // among the section's members.
        let (after, before, reordered) = match target_section {
            Some(id) => self.sidebar_section_drop_anchors(id, &payload.chat_id, cx),
            None => (None, None, false),
        };
        if saved == next && source_section == target_section && !reordered {
            // A no-op drop still animates home; ordinary groups retain recency order.
            self.cancel_sidebar_session_transfer(cx);
            return;
        }
        // Validate and accept before ending the preview. Rejected drops use
        // the same animated return path as dropping outside a destination.
        let change = if let Some(index) = next.iter().position(|id| id == &payload.chat_id) {
            let after = index.checked_sub(1).and_then(|i| next.get(i)).cloned();
            let before = next.get(index + 1).cloned();
            if saved.contains(&payload.chat_id) {
                zeron_proto::SidebarPinChange::Move {
                    session_id: payload.chat_id.clone(),
                    after,
                    before,
                }
            } else {
                zeron_proto::SidebarPinChange::Pin {
                    session_id: payload.chat_id.clone(),
                    after,
                    before,
                }
            }
        } else {
            zeron_proto::SidebarPinChange::Unpin {
                session_id: payload.chat_id.clone(),
            }
        };
        if self.state.read(cx).workspace_scope != Some(WorkspaceScope::Local)
            && !matches!(target, SidebarSessionDrop::Pinned(_))
        {
            if !self.change_sidebar_section(
                zeron_proto::SidebarSectionChange::Assign {
                    session_id: payload.chat_id.clone(),
                    section_id: target_section.map(str::to_owned),
                    after: after.clone(),
                    before: before.clone(),
                },
                cx,
            ) {
                self.cancel_sidebar_session_transfer(cx);
                return;
            }
        } else {
            if saved != next
                && !self.apply_sidebar_pin_change(payload.profile_key.clone(), change, cx)
            {
                self.cancel_sidebar_session_transfer(cx);
                return;
            }
            if !matches!(target, SidebarSessionDrop::Pinned(_))
                || self.state.read(cx).workspace_scope == Some(WorkspaceScope::Local)
                || saved == next
            {
                self.change_sidebar_section(
                    zeron_proto::SidebarSectionChange::Assign {
                        session_id: payload.chat_id.clone(),
                        section_id: target_section.map(str::to_owned),
                        after,
                        before,
                    },
                    cx,
                );
            }
        }
        self.sidebar_session_transfer = None;
        self.accept_command_palette_drop();
        self.cancel_pinned_session_drag(cx);
        if matches!(target, SidebarSessionDrop::Pinned(_)) {
            self.pinned_open = true;
        } else {
            self.sessions_open = true;
        }
        // Membership changes preserve each group's live activity sort.
        // Drag previews already animated this move. Establish a fresh layout
        // baseline so the automatic resort glide does not replay it on release.
        self.sidebar_prev_order.clear();
        self.sidebar_resort.clear();
        self.sidebar_new_keys.clear();
        cx.notify();
    }

    /// Where a drop into section `id` places `chat`: its left and right
    /// neighbours among the section's members, and whether that moves it.
    /// The preview gap names the slot among the rows on screen; a collapsed
    /// section, or a drop without a gap, appends.
    fn sidebar_section_drop_anchors(
        &self,
        id: &str,
        chat: &str,
        cx: &Context<Self>,
    ) -> (Option<String>, Option<String>, bool) {
        let Some(section) = self
            .active_sidebar_sections(cx)
            .into_iter()
            .find(|section| section.id == id)
        else {
            return (None, None, false);
        };
        let others: Vec<String> = section
            .session_ids
            .iter()
            .filter(|member| *member != chat)
            .cloned()
            .collect();
        let group = format!("regular:section:{id}");
        let gap = self
            .sidebar_session_transfer
            .as_ref()
            .and_then(|drag| drag.preview.as_ref())
            .filter(|gap| gap.group == group && !section.collapsed);
        let (after, before) = match gap {
            Some(gap) => {
                let visible: HashSet<String> = self.sidebar_visible_order(cx).into_iter().collect();
                let shown: Vec<&String> = section
                    .session_ids
                    .iter()
                    .filter(|member| visible.contains(*member))
                    .collect();
                let mut index = gap.index;
                if let Some(source) = shown.iter().position(|member| *member == chat)
                    && index > source
                {
                    index -= 1;
                }
                let shown: Vec<&String> = shown.into_iter().filter(|m| *m != chat).collect();
                let after = index.checked_sub(1).and_then(|i| shown.get(i)).map(|m| (*m).clone());
                let before = shown.get(index).map(|m| (*m).clone());
                if after.is_none() && before.is_none() {
                    (others.last().cloned(), None)
                } else {
                    (after, before)
                }
            }
            None => (others.last().cloned(), None),
        };
        let mut placed = others;
        let index = zeron_proto::section_insert_index(&placed, &after, &before);
        placed.insert(index, chat.to_owned());
        (after, before, placed != section.session_ids)
    }

    pub(super) fn update_pinned_session_drag(
        &mut self,
        payload: &SidebarSessionDrag,
        over: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.pinned_open
            || self.active_sidebar_pin_profile_key(cx).as_deref() != Some(&payload.profile_key)
        {
            self.cancel_pinned_session_drag(cx);
            return;
        }
        let Some(from) = payload
            .visible_ids
            .iter()
            .position(|id| id == &payload.chat_id)
        else {
            self.cancel_pinned_session_drag(cx);
            return;
        };
        if over >= payload.visible_ids.len() {
            self.cancel_pinned_session_drag(cx);
            return;
        }
        match &mut self.pinned_session_drag {
            Some(drag)
                if drag.chat_id == payload.chat_id
                    && drag.filter == payload.filter
                    && drag.profile_key == payload.profile_key
                    && drag.visible_ids.as_ref() == payload.visible_ids.as_ref()
                    && drag.over != over =>
            {
                drag.prev_over = drag.over;
                drag.over = over;
                drag.epoch = drag.epoch.wrapping_add(1);
                cx.notify();
            }
            Some(drag)
                if drag.chat_id == payload.chat_id
                    && drag.filter == payload.filter
                    && drag.profile_key == payload.profile_key
                    && drag.visible_ids.as_ref() == payload.visible_ids.as_ref() => {}
            _ => {
                self.pinned_session_drag_generation =
                    self.pinned_session_drag_generation.wrapping_add(1);
                self.pinned_session_drag = Some(PinnedSessionDragState {
                    chat_id: payload.chat_id.clone(),
                    visible_ids: payload.visible_ids.clone(),
                    from,
                    over,
                    prev_over: from,
                    epoch: 0,
                    filter: payload.filter.clone(),
                    profile_key: payload.profile_key.clone(),
                    pointer_y: None,
                    viewport_top: 0.0,
                    viewport_bottom: 0.0,
                    generation: self.pinned_session_drag_generation,
                    autoscroll_active: false,
                });
                cx.notify();
            }
        }
    }

    pub(super) fn track_pinned_session_drag_pointer(
        &mut self,
        payload: SidebarSessionDrag,
        pointer_y: f32,
        viewport_top: f32,
        viewport_bottom: f32,
        cx: &mut Context<Self>,
    ) {
        if !payload.visible_ids.contains(&payload.chat_id) {
            return;
        }
        let scroll_top = -f32::from(self.sidebar_scroll.offset().y);
        let rel_y = pinned_session_pointer_y(pointer_y, viewport_top, scroll_top);
        let inside_pinned_section =
            row_drop_index(rel_y, &self.sidebar_pinned_heights, false).is_some();
        let Some(over) = row_drop_index(rel_y, &self.sidebar_pinned_heights, true) else {
            if let Some(drag) = self.pinned_session_drag.as_mut() {
                drag.pointer_y = None;
                drag.autoscroll_active = false;
            }
            return;
        };

        self.update_pinned_session_drag(&payload, over, cx);
        if !inside_pinned_section {
            if let Some(drag) = self.pinned_session_drag.as_mut() {
                drag.pointer_y = None;
                drag.autoscroll_active = false;
            }
            return;
        }
        let delta = pinned_drag_scroll_delta(pointer_y, viewport_top, viewport_bottom);
        let Some(drag) = self.pinned_session_drag.as_mut() else {
            return;
        };
        drag.pointer_y = Some(pointer_y);
        drag.viewport_top = viewport_top;
        drag.viewport_bottom = viewport_bottom;
        let should_start = delta != 0.0 && !drag.autoscroll_active;
        let generation = drag.generation;
        if should_start {
            drag.autoscroll_active = true;
            self.start_pinned_session_autoscroll(generation, cx);
        }
    }

    fn start_pinned_session_autoscroll(&mut self, generation: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(
                        super::SIDEBAR_DRAG_SCROLL_FRAME_MS,
                    ))
                    .await;
                let keep_running = this
                    .update(cx, |shell, cx| {
                        shell.pinned_session_autoscroll_tick(generation, cx)
                    })
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        })
        .detach();
    }

    fn pinned_session_autoscroll_tick(&mut self, generation: u64, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.pinned_session_drag.as_ref() else {
            return false;
        };
        if drag.generation != generation || !cx.has_active_drag() {
            if let Some(drag) = self.pinned_session_drag.as_mut() {
                drag.autoscroll_active = false;
            }
            return false;
        }
        let Some(pointer_y) = drag.pointer_y else {
            if let Some(drag) = self.pinned_session_drag.as_mut() {
                drag.autoscroll_active = false;
            }
            return false;
        };
        let viewport_top = drag.viewport_top;
        let viewport_bottom = drag.viewport_bottom;
        let delta = pinned_drag_scroll_delta(pointer_y, viewport_top, viewport_bottom);
        let scroll_top = -f32::from(self.sidebar_scroll.offset().y);
        let max_scroll = f32::from(self.sidebar_scroll.max_offset().y);
        let Some(next_scroll) = pinned_drag_scroll_step(
            true,
            generation,
            drag.generation,
            scroll_top,
            max_scroll,
            delta,
        ) else {
            if let Some(drag) = self.pinned_session_drag.as_mut() {
                drag.autoscroll_active = false;
            }
            return false;
        };

        let rel_y = pinned_session_pointer_y(pointer_y, viewport_top, next_scroll);
        let Some(over) = row_drop_index(rel_y, &self.sidebar_pinned_heights, false) else {
            if let Some(drag) = self.pinned_session_drag.as_mut() {
                drag.autoscroll_active = false;
            }
            return false;
        };

        let offset = self.sidebar_scroll.offset();
        self.sidebar_scroll
            .set_offset(gpui::point(offset.x, px(-next_scroll)));
        if let Some(drag) = self.pinned_session_drag.as_mut()
            && drag.over != over
        {
            drag.prev_over = drag.over;
            drag.over = over;
            drag.epoch = drag.epoch.wrapping_add(1);
        }
        cx.notify();
        true
    }

    pub(super) fn pinned_session_drag_is_valid(&self, cx: &App) -> bool {
        let Some(drag) = self.pinned_session_drag.as_ref() else {
            return true;
        };
        if !self.pinned_open
            || drag.filter != self.settings.space_filter
            || self.active_sidebar_pin_profile_key(cx).as_deref() != Some(&drag.profile_key)
        {
            return false;
        }
        let current_pins = self.sidebar_pins_for_profile(&drag.profile_key, cx);
        let state = self.state.read(cx);
        let in_filter = state.project_filter(drag.filter.as_deref());
        let visible_ids: HashSet<String> = state
            .overview_chats(Utc::now())
            .into_iter()
            .filter(|(_, chat)| in_filter(chat))
            .map(|(_, chat)| chat.id.clone())
            .collect();
        let current_ids = current_pins
            .iter()
            .filter(|id| visible_ids.contains(id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        pinned_drag_snapshot_is_valid(&drag.chat_id, drag.visible_ids.as_ref(), &current_ids)
    }

    pub(super) fn cancel_pinned_session_drag(&mut self, cx: &mut Context<Self>) {
        if self.pinned_session_drag.take().is_some() {
            self.pinned_session_drag_generation =
                self.pinned_session_drag_generation.wrapping_add(1);
            cx.notify();
        }
    }

    /// The filter's scrollable rows: "All projects", then projects matching
    /// the search (ranked — `popover::filter_indices`) — one row per
    /// repository across devices, carrying its local-first checkout's id. "All"
    /// only shows on an empty query (searching means hunting a project). The
    /// "New project" action is not a row here — the card renders it as a
    /// pinned footer.
    fn spaces_menu_rows(&self, cx: &App) -> Vec<SpacesMenuRow> {
        let query = self
            .spaces_menu
            .get()
            .map(|menu| menu.search.read(cx).text().to_string())
            .unwrap_or_default();
        let state = self.state.read(cx);
        let projects = state.projects();
        let names: Vec<String> = projects
            .iter()
            .map(|members| state.representative_space(members[0]).display_name().to_string())
            .collect();
        let mut rows: Vec<SpacesMenuRow> = Vec::new();
        if query.trim().is_empty() {
            rows.push(SpacesMenuRow::All);
        }
        rows.extend(
            popover::filter_indices(&query, &names)
                .into_iter()
                .map(|ix| SpacesMenuRow::Space(projects[ix][0].id.clone())),
        );
        rows
    }

    /// The repository the sidebar filter names, if any — any checkout's row
    /// selects (and reads as) the whole project.
    fn space_filter_key(&self, cx: &App) -> Option<String> {
        let state = self.state.read(cx);
        let id = self.settings.space_filter.as_deref()?;
        Some(
            state
                .space_row(id)
                .map_or_else(|| id.to_owned(), zeron_proto::view::project_key),
        )
    }

    /// The project key of a menu row's space id.
    fn space_row_key(&self, id: &str, cx: &App) -> String {
        self.state
            .read(cx)
            .space_row(id)
            .map_or_else(|| id.to_owned(), zeron_proto::view::project_key)
    }

    fn open_spaces_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_sidebar_view_menu(cx);
        // "PaletteSearch" context: ↑↓/⏎ stay unbound in the input and bubble
        // to the card's key handler.
        let search =
            cx.new(|cx| ComposerInput::with_context("Search projects…", "PaletteSearch", cx));
        let search_events = cx.subscribe(&search, |this: &mut Shell, _, event, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                if let Some(menu) = this.spaces_menu.open_mut() {
                    menu.active = 0;
                }
                cx.notify();
            }
        });
        // The highlight starts ON the current filter row.
        let current = self.space_filter_key(cx);
        let handle = search.read(cx).focus_handle(cx);
        self.spaces_menu.open(SpacesMenu {
            search,
            active: 0,
            focus: cx.focus_handle(),
            list_scroll: gpui::ScrollHandle::new(),
            _search_events: search_events,
        });
        // Fresh handle at the top — don't let the stale rail baseline read
        // the reopen as scrolling.
        self.spaces_menu_bar.clear_scroll_baseline();
        let rows = self.spaces_menu_rows(cx);
        let start = match &current {
            None => 0,
            Some(key) => rows
                .iter()
                .position(|row| matches!(row, SpacesMenuRow::Space(s) if self.space_row_key(s, cx) == *key))
                .unwrap_or(0),
        };
        if let Some(menu) = self.spaces_menu.open_mut() {
            menu.active = start;
        }
        // Focusable before first paint (the add-space palette's proven order).
        window.focus(&handle, cx);
        cx.notify();
    }

    fn activate_spaces_menu_row(&mut self, row: SpacesMenuRow, cx: &mut Context<Self>) {
        match row {
            SpacesMenuRow::All => self.set_space_filter(None, cx),
            SpacesMenuRow::Space(id) => self.set_space_filter(Some(id), cx),
            SpacesMenuRow::AddSpace => {
                self.close_spaces_menu(cx);
                self.open_add_space(cx);
            }
        }
    }

    /// Dropdown keys (bubbling from the focused search input): ↑↓ navigate,
    /// ⏎ activates the highlighted row, esc closes.
    fn spaces_menu_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        // The card stays mounted (and focused) through the exit animation —
        // keys must not drive a dying menu.
        if !self.spaces_menu.is_open() {
            return;
        }
        let key = popover::classify_key(
            event.keystroke.key.as_str(),
            event.keystroke.modifiers.platform,
            event.keystroke.modifiers.control,
        );
        match key {
            popover::MenuKey::Escape => {
                self.close_spaces_menu(cx);
                cx.stop_propagation();
            }
            popover::MenuKey::Up | popover::MenuKey::Down => {
                let rows = self.spaces_menu_rows(cx);
                // +1: the pinned footer stays in the nav order, exactly as
                // when it was the list's last row.
                let count = rows.len() + 1;
                let delta = if key == popover::MenuKey::Up { -1 } else { 1 };
                if let Some(menu) = self.spaces_menu.open_mut() {
                    menu.active = popover::menu_step(Some(menu.active), count, delta).unwrap_or(0);
                    // The footer renders below the scroller — only in-list
                    // rows can be scrolled to (the footer index would leave
                    // a request pending against a row that never exists).
                    if menu.active < rows.len() {
                        menu.list_scroll.scroll_to_item(menu.active);
                    }
                    cx.notify();
                }
            }
            popover::MenuKey::Enter | popover::MenuKey::ModEnter => {
                let active = self.spaces_menu.get().map(|m| m.active).unwrap_or(0);
                let rows = self.spaces_menu_rows(cx);
                // One past the scrollable rows is the pinned footer.
                let row = if active < rows.len() {
                    rows[active].clone()
                } else {
                    SpacesMenuRow::AddSpace
                };
                self.activate_spaces_menu_row(row, cx);
            }
            popover::MenuKey::Backspace | popover::MenuKey::Other => {}
        }
    }

    pub(super) fn close_sidebar_view_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = self.sidebar_view_menu.open_mut() {
            menu.hover_intent.reset();
        }
        if self.sidebar_view_menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.sidebar_view_menu);
            cx.notify();
        }
    }

    fn open_sidebar_view_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_spaces_menu(cx);
        let focus = cx.focus_handle();
        self.sidebar_view_menu.open(SidebarViewMenu {
            submenu: None,
            submenu_active: None,
            submenu_bounds: None,
            submenu_on_left: false,
            hover_intent: popover::HoverIntent::default(),
            active: None,
            focus: focus.clone(),
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    fn activate_sidebar_view_row(&mut self, row: SidebarViewRow, cx: &mut Context<Self>) {
        if let Some(menu) = self.sidebar_view_menu.open_mut() {
            menu.hover_intent.cancel();
        }
        if row.inert(self.settings.sidebar_organization == SidebarOrganization::Inbox) {
            return;
        }
        self.cancel_sidebar_session_transfer(cx);
        self.cancel_pinned_session_drag(cx);
        match row {
            SidebarViewRow::ByProject => {
                self.settings.sidebar_organization = SidebarOrganization::ByProject
            }
            SidebarViewRow::ShowProjectLabel => {
                self.settings.sidebar_show_project_label = !self.settings.sidebar_show_project_label
            }
            SidebarViewRow::Compact => {
                self.settings.sidebar_compact = !self.settings.sidebar_compact
            }
            SidebarViewRow::ShowProjectIcon => {
                self.settings.sidebar_show_project_icon = !self.settings.sidebar_show_project_icon
            }
            SidebarViewRow::ByDevice => {
                self.settings.sidebar_organization = SidebarOrganization::ByDevice
            }
            SidebarViewRow::InOneList => {
                self.settings.sidebar_organization = SidebarOrganization::InOneList
            }
            SidebarViewRow::Inbox => {
                self.settings.sidebar_organization = SidebarOrganization::Inbox
            }
            SidebarViewRow::LastUpdated => self.settings.sidebar_sort = SidebarSort::LastUpdated,
            SidebarViewRow::Created => self.settings.sidebar_sort = SidebarSort::Created,
            SidebarViewRow::ActiveFirst => {
                self.settings.sidebar_active_first = !self.settings.sidebar_active_first
            }
            SidebarViewRow::ShowBranch => {
                self.settings.sidebar_show_branch = !self.settings.sidebar_show_branch
            }
            SidebarViewRow::ShowPullRequest => {
                self.settings.sidebar_show_pull_request = !self.settings.sidebar_show_pull_request;
                let visible = self.settings.sidebar_show_pull_request;
                self.state.update(cx, |state, cx| {
                    state.set_change_requests_visible(visible, cx)
                });
            }
            SidebarViewRow::ShowHarness => {
                self.settings.sidebar_show_harness = !self.settings.sidebar_show_harness
            }
            SidebarViewRow::ShowWorking => {
                self.settings.sidebar_working_shelf = !self.settings.sidebar_working_shelf
            }
            SidebarViewRow::ShowArchived => {
                self.settings.sidebar_show_archived = !self.settings.sidebar_show_archived
            }
            SidebarViewRow::ShowProjectFilter => {
                self.settings.sidebar_show_project_filter =
                    !self.settings.sidebar_show_project_filter;
                // A hidden filter must not keep narrowing the list unseen.
                if !self.settings.sidebar_show_project_filter {
                    self.set_space_filter(None, cx);
                }
            }
        }
        self.schedule_save(cx);
        if row.closes_submenu() {
            if let Some(menu) = self.sidebar_view_menu.open_mut() {
                menu.submenu = None;
                menu.submenu_active = None;
                menu.submenu_bounds = None;
                menu.hover_intent.cancel();
            }
        }
        cx.notify();
    }

    fn open_sidebar_view_submenu(&mut self, group: usize, keyboard: bool, cx: &mut Context<Self>) {
        if let Some(menu) = self.sidebar_view_menu.open_mut() {
            menu.hover_intent.reset();
            menu.active = Some(group);
            menu.submenu = (group < SIDEBAR_VIEW_GROUPS.len()).then_some(group);
            menu.submenu_active = keyboard.then_some(0);
            menu.submenu_bounds = None;
            cx.notify();
        }
    }

    fn hover_sidebar_view_group(
        &mut self,
        group: usize,
        pointer: gpui::Point<gpui::Pixels>,
        moved: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.sidebar_view_menu.open_mut() else {
            return;
        };
        let action = if moved {
            menu.hover_intent.moved(
                menu.submenu.as_ref(),
                &group,
                pointer,
                menu.submenu_bounds,
                menu.submenu_on_left,
            )
        } else {
            menu.hover_intent.enter(
                menu.submenu.as_ref(),
                &group,
                pointer,
                menu.submenu_bounds,
                menu.submenu_on_left,
            )
        };
        match action {
            popover::HoverAction::None => {}
            popover::HoverAction::Open => {
                self.open_sidebar_view_submenu(group, false, cx);
                if let Some(menu) = self.sidebar_view_menu.open_mut() {
                    menu.hover_intent.record_origin(pointer);
                }
            }
            popover::HoverAction::Defer => {
                let source = menu.submenu;
                menu.hover_intent.defer(cx, move |this, cx| {
                    if this.sidebar_view_menu.open_mut().is_some_and(|menu| {
                        menu.submenu == source && menu.hover_intent.pending() == Some(&group)
                    }) {
                        this.open_sidebar_view_submenu(group, false, cx);
                        if let Some(menu) = this.sidebar_view_menu.open_mut() {
                            menu.hover_intent.record_origin(pointer);
                        }
                    }
                });
            }
        }
    }

    fn sidebar_view_menu_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let Some(menu) = self.sidebar_view_menu.open_mut() else {
            return;
        };
        menu.hover_intent.cancel();
        let key = match popover::classify_key(
            event.keystroke.key.as_str(),
            event.keystroke.modifiers.platform,
            event.keystroke.modifiers.control,
        ) {
            popover::MenuKey::Up => "up",
            popover::MenuKey::Down => "down",
            _ => event.keystroke.key.as_str(),
        };
        match key {
            "escape" | "left" => {
                if menu.submenu.take().is_some() {
                    menu.submenu_active = None;
                    menu.submenu_bounds = None;
                    cx.notify();
                } else if key == "escape" {
                    self.close_sidebar_view_menu(cx);
                }
            }
            "up" | "down" => {
                let delta = if key == "up" { -1 } else { 1 };
                if let Some(group) = menu.submenu {
                    menu.submenu_active = popover::menu_step(
                        menu.submenu_active,
                        SIDEBAR_VIEW_GROUPS[group].1.len(),
                        delta,
                    );
                } else {
                    menu.active =
                        popover::menu_step(menu.active, SIDEBAR_VIEW_GROUPS.len() + 2, delta);
                }
                cx.notify();
            }
            "right" | "enter" | "space" => {
                if let Some(group) = menu.submenu {
                    if key == "enter" {
                        if let Some(choice) = menu.submenu_active {
                            let row =
                                SIDEBAR_VIEW_ROWS[SIDEBAR_VIEW_GROUPS[group].1.start + choice];
                            self.activate_sidebar_view_row(row, cx);
                        }
                    }
                } else {
                    let group = menu.active.unwrap_or(0);
                    if group == SIDEBAR_VIEW_GROUPS.len() + 1 {
                        if key != "right" && !event.is_held {
                            self.open_section_dialog(None, cx);
                        }
                    } else if group == SIDEBAR_VIEW_GROUPS.len() {
                        if key != "right" && !event.is_held {
                            self.activate_sidebar_view_row(SidebarViewRow::Compact, cx);
                        }
                    } else if key != "space" {
                        self.open_sidebar_view_submenu(group, true, cx);
                    }
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_sidebar_view_menu(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let theme = &theme.for_popup();
        let Some(menu_state) = self.sidebar_view_menu.get() else {
            return div().into_any_element();
        };
        let active = menu_state.active;
        let submenu = menu_state.submenu;
        let submenu_active = menu_state.submenu_active;
        let on_left = menu_state.submenu_on_left;
        let focus = menu_state.focus.clone();
        let organization = self.settings.sidebar_organization;
        let sort = self.settings.sidebar_sort;
        let show_harness = self.settings.sidebar_show_harness;
        let show_branch = self.settings.sidebar_show_branch;
        let show_pr = self.settings.sidebar_show_pull_request;

        let labels: [&str; SIDEBAR_VIEW_ROWS.len()] = [
            "By device",
            "By project",
            "None",
            "Inbox",
            "Last updated",
            "Created",
            "Active first",
            "Branch",
            "Pull request",
            "Harness",
            "Project icon",
            "Location",
            "Project filter",
            "Working",
            "Archived",
        ];
        let icons: [&str; SIDEBAR_VIEW_ROWS.len()] = [
            icons::LAPTOP,
            icons::FOLDER,
            icons::LIST,
            icons::BELL,
            icons::CLOCK_CIRCLE,
            icons::CALENDAR,
            icons::BOLT,
            icons::GIT_BRANCH,
            icons::PULL_REQUEST,
            icons::BOT,
            icons::PROJECT_DEFAULT,
            // Location names the host: the remote glyph sidebar rows wear.
            icons::REMOTE_SERVER,
            icons::FOLDER,
            icons::PLAY,
            icons::ARCHIVE_MINIMALISTIC,
        ];
        let selected: [bool; SIDEBAR_VIEW_ROWS.len()] = [
            organization == SidebarOrganization::ByDevice,
            organization == SidebarOrganization::ByProject,
            organization == SidebarOrganization::InOneList,
            organization == SidebarOrganization::Inbox,
            sort == SidebarSort::LastUpdated,
            sort == SidebarSort::Created,
            self.settings.sidebar_active_first,
            show_branch,
            show_pr,
            show_harness,
            self.settings.sidebar_show_project_icon,
            self.settings.sidebar_show_project_label,
            self.settings.sidebar_show_project_filter,
            self.settings.sidebar_working_shelf,
            self.settings.sidebar_show_archived,
        ];
        let inbox = organization == SidebarOrganization::Inbox;
        let values = [
            labels[selected[..4].iter().position(|v| *v).unwrap_or(0)].to_string(),
            if inbox {
                labels[3].to_string()
            } else {
                labels[4 + selected[4..6].iter().position(|v| *v).unwrap_or(0)].to_string()
            },
        ];
        let mut groups: Vec<AnyElement> = Vec::new();
        for (group, (label, range)) in SIDEBAR_VIEW_GROUPS.iter().enumerate() {
            if group == 2 {
                groups.push(popover::menu_separator().into_any_element());
            }
            let open = submenu == Some(group);
            let entity = cx.entity().downgrade();
            let exit_entity = cx.entity().downgrade();
            let mut trigger = popover::menu_row_nav(
                theme,
                open,
                active == Some(group),
                format!("sidebar-view-group-{group}"),
            )
            .id(("sidebar-view-group", group))
            .relative()
            .h(px(30.0))
            .py(px(0.0))
            .on_click(cx.listener(move |this, _, _, cx| {
                // Match model settings: hover opens; clicking dismisses, including
                // a sibling crossed during hover grace, without delayed reopening.
                if let Some(menu) = this.sidebar_view_menu.open_mut() {
                    menu.active = Some(group);
                    menu.submenu = None;
                    menu.submenu_active = None;
                    menu.submenu_bounds = None;
                    menu.hover_intent.reset();
                }
                cx.notify();
                cx.stop_propagation();
            }))
            .child(div().flex_1().child(*label))
            .when(group < values.len(), |el| {
                el.child(
                    div()
                        .text_color(theme.text_muted)
                        .child(values[group].clone()),
                )
            })
            .child(
                icon(icons::ALT_ARROW_RIGHT)
                    .size(px(12.0))
                    .text_color(theme.text_muted),
            )
            .child(
                gpui::canvas(
                    move |bounds, window, cx| {
                        let left = bounds.right() + px(244.0) > window.viewport_size().width;
                        let _ = entity.update(cx, |this, cx| {
                            if let Some(menu) = this.sidebar_view_menu.open_mut() {
                                if menu.submenu_on_left != left {
                                    menu.submenu_on_left = left;
                                    cx.notify();
                                }
                            }
                        });
                    },
                    move |trigger, _, window, _| {
                        if !open {
                            return;
                        }
                        window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                            if phase != gpui::DispatchPhase::Bubble {
                                return;
                            }
                            let _ = exit_entity.update(cx, |this, cx| {
                                let Some(menu) = this.sidebar_view_menu.open_mut() else {
                                    return;
                                };
                                if menu.submenu != Some(group)
                                    || menu.hover_intent.contains_pointer(
                                        trigger,
                                        menu.submenu_bounds,
                                        event.position,
                                        menu.submenu_on_left,
                                    )
                                {
                                    return;
                                }
                                menu.hover_intent.reset();
                                menu.submenu = None;
                                menu.submenu_bounds = None;
                                menu.submenu_active = None;
                                menu.active = None;
                                cx.notify();
                            });
                        });
                    },
                )
                .absolute()
                .inset_0(),
            );
            if open {
                let mut choices = Vec::new();
                for (choice, ix) in range.clone().enumerate() {
                    let row = SIDEBAR_VIEW_ROWS[ix];
                    choices.push(
                        popover::menu_row_nav(
                            theme,
                            selected[ix],
                            submenu_active == Some(choice),
                            format!("sidebar-view-row-{ix}"),
                        )
                        .id(("sidebar-view-row", ix))
                        .when(row.inert(inbox), |el| el.opacity(0.45))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.activate_sidebar_view_row(row, cx);
                            cx.stop_propagation();
                        }))
                        .child(
                            icon(icons[ix])
                                .size(px(15.0))
                                .flex_none()
                                .text_color(theme.text_muted),
                        )
                        .child(div().flex_1().child(labels[ix]))
                        .child(div().w(px(14.0)).flex_none().when(selected[ix], |el| {
                            el.child(
                                icon(icons::CHECK)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted),
                            )
                        })),
                    );
                }
                let entity = cx.entity().downgrade();
                let child = popover::popover_card(theme)
                    .w(px(232.0))
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(popover::menu_heading(theme, label))
                    .children(choices)
                    .child(
                        gpui::canvas(
                            move |bounds, _, cx| {
                                let _ = entity.update(cx, |this, _| {
                                    if let Some(menu) = this.sidebar_view_menu.open_mut() {
                                        menu.submenu_bounds = Some(bounds);
                                    }
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    );
                trigger = trigger.child(popover::nested_menu(
                    format!("sidebar-view-child-{group}"),
                    child.into_any_element(),
                    on_left,
                ));
            }
            groups.push(
                div()
                    .id(("sidebar-view-hover", group))
                    .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                        if *hovered {
                            this.hover_sidebar_view_group(
                                group,
                                window.mouse_position(),
                                false,
                                cx,
                            );
                        } else if let Some(menu) = this.sidebar_view_menu.open_mut() {
                            menu.hover_intent.leave(&group);
                        }
                    }))
                    .on_mouse_move(
                        cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                            this.hover_sidebar_view_group(group, event.position, true, cx);
                        }),
                    )
                    .child(trigger)
                    .into_any_element(),
            );
        }
        groups.push(popover::menu_separator().into_any_element());
        groups.push(
            div()
                .id("sidebar-view-compact-hover")
                .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                    if *hovered {
                        this.hover_sidebar_view_group(
                            SIDEBAR_VIEW_GROUPS.len(),
                            window.mouse_position(),
                            false,
                            cx,
                        );
                    } else if let Some(menu) = this.sidebar_view_menu.open_mut() {
                        menu.hover_intent.leave(&SIDEBAR_VIEW_GROUPS.len());
                    }
                }))
                .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                    this.hover_sidebar_view_group(
                        SIDEBAR_VIEW_GROUPS.len(),
                        event.position,
                        true,
                        cx,
                    );
                }))
                .child(
                    popover::menu_row_nav(
                        theme,
                        false,
                        active == Some(SIDEBAR_VIEW_GROUPS.len()),
                        "sidebar-view-compact",
                    )
                    .id("sidebar-view-compact")
                    .h(px(30.0))
                    .py(px(0.0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_sidebar_view_submenu(SIDEBAR_VIEW_GROUPS.len(), false, cx);
                        this.activate_sidebar_view_row(SidebarViewRow::Compact, cx);
                        cx.stop_propagation();
                    }))
                    .child(div().flex_1().child("Compact"))
                    .child(crate::settings::widgets::toggle_switch(
                        theme,
                        self.settings.sidebar_compact,
                        "sidebar-view-compact",
                    )),
                )
                .into_any_element(),
        );
        groups.push(popover::menu_separator().into_any_element());
        groups.push(
            div()
                .id("sidebar-create-section-hover")
                .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                    let target = SIDEBAR_VIEW_GROUPS.len() + 1;
                    if *hovered {
                        this.hover_sidebar_view_group(target, window.mouse_position(), false, cx);
                    } else if let Some(menu) = this.sidebar_view_menu.open_mut() {
                        menu.hover_intent.leave(&target);
                    }
                }))
                .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                    this.hover_sidebar_view_group(
                        SIDEBAR_VIEW_GROUPS.len() + 1,
                        event.position,
                        true,
                        cx,
                    );
                }))
                .child(
                    popover::menu_row_nav(
                        theme,
                        false,
                        active == Some(SIDEBAR_VIEW_GROUPS.len() + 1),
                        "sidebar-create-section",
                    )
                    .id("sidebar-create-section")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_section_dialog(None, cx);
                        cx.stop_propagation();
                    }))
                    .child(
                        icon(icons::PLUS)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    )
                    .child("Create Section"),
                )
                .into_any_element(),
        );
        popover::popover_card(theme)
            .w(px(self.sidebar_content_width() - 2.0 * Theme::SPACE_SM))
            .track_focus(&focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                this.sidebar_view_menu_key(event, cx)
            }))
            .on_mouse_down_out(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                if !this
                    .sidebar_view_menu
                    .get()
                    .and_then(|m| m.submenu_bounds)
                    .is_some_and(|bounds| bounds.contains(&event.position))
                {
                    this.close_sidebar_view_menu(cx);
                }
            }))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(groups)
            .into_any_element()
    }

    /// The sidebar's space-filter row: current filter ("All projects" or the
    /// space's name), the dropdown floating beneath while open.
    /// Sits OUTSIDE the sidebar's scroll region so the float never clips.
    pub(super) fn render_spaces_filter(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let filter = self.settings.space_filter.clone();
        let (label, space): (SharedString, Option<zeron_proto::Space>) = {
            let state = self.state.read(cx);
            match filter.as_deref().and_then(|id| state.space_row(id)) {
                Some(space) => (
                    state
                        .representative_space(space)
                        .display_name()
                        .to_string()
                        .into(),
                    Some(space.clone()),
                ),
                None => (SharedString::from("All projects"), None),
            }
        };
        let open = self.spaces_menu.is_open();
        let filtered = space.is_some();
        let header_button = |id: &'static str, glyph: &'static str, active: bool| {
            let group = SharedString::from(format!("{id}-group"));
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .role(gpui::Role::Button)
                .group(group.clone())
                .size(px(28.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.0))
                .cursor_pointer()
                .when(active, |el| el.bg(theme.glass_hover()))
                .hover(|s| s.bg(theme.glass_hover()))
                .child(
                    icon(glyph)
                        .size(px(16.0))
                        .text_color(if filtered && id == "spaces-filter" {
                            theme.accent
                        } else {
                            theme.text_muted.opacity(0.6)
                        })
                        .group_hover(group, |s| s.text_color(theme.text)),
                )
        };
        let search = div()
            .id("sidebar-search")
            .debug_selector(|| "sidebar-search".into())
            .role(gpui::Role::Button)
            .aria_label(if filtered {
                SharedString::from(format!("Search {label}"))
            } else {
                SharedString::from("Search")
            })
            .group("sidebar-search")
            .flex_1()
            .min_w_0()
            .h(px(32.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(8.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .hover(|s| s.bg(theme.glass_hover()))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_command_palette(window, cx)))
            .child(
                icon(icons::LUCIDE_SEARCH)
                    .size(px(16.0))
                    .flex_none()
                    .text_color(theme.text_muted.opacity(0.6)),
            )
            .child(super::sidebar_faded_label(
                "sidebar-search-label".into(),
                true,
                div()
                    .text_size(crate::typography::ui_rems(14.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .group_hover("sidebar-search", |s| s.text_color(theme.text))
                    .child(if filtered {
                        SharedString::from(format!("Search {label}"))
                    } else {
                        SharedString::from("Search")
                    }),
            ));
        let filter_button = self.settings.sidebar_show_project_filter.then(|| {
            header_button("spaces-filter", icons::LUCIDE_LIST_FILTER, open)
                .aria_label(if filtered {
                    SharedString::from(format!("Filter: {label}"))
                } else {
                    SharedString::from("Filter by project")
                })
                .aria_expanded(open)
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, _| this.spaces_menu.note_trigger_press()),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.spaces_menu.take_press_was_open() {
                        this.close_spaces_menu(cx);
                    } else {
                        this.open_spaces_menu(window, cx);
                    }
                }))
                .tooltip(crate::settings::widgets::text_tooltip(if filtered {
                    SharedString::from(format!("Filter: {label}"))
                } else {
                    SharedString::from("Filter by project")
                }))
        });
        let add_project = header_button("sidebar-add-project", icons::LUCIDE_FOLDER_PLUS, false)
            .aria_label(ShortcutId::NewProject.label())
            .on_click(cx.listener(|this, _, _, cx| this.open_add_space(cx)))
            .tooltip(crate::settings::widgets::text_tooltip(
                ShortcutId::NewProject.label(),
            ));
        let new_chat = header_button("sidebar-new-chat", icons::LUCIDE_SQUARE_PEN, false)
            .aria_label(ShortcutId::NewSession.label())
            .on_click(cx.listener(|this, _, _, cx| this.open_new_session(None, cx)))
            .tooltip(crate::settings::widgets::text_tooltip(
                ShortcutId::NewSession.label(),
            ));
        let row = div()
            .flex_none()
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .p(px(8.0))
            .child(search)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .children(filter_button)
                    .child(add_project)
                    .child(new_chat),
            );
        if self.spaces_menu.get().is_some() {
            let closing = self.spaces_menu.closing_since();
            let menu = self.render_spaces_menu(theme, cx);
            row.child(popover::anchored_menu_below(
                "spaces-filter-menu",
                menu,
                closing,
            ))
            .into_any_element()
        } else {
            row.into_any_element()
        }
    }

    /// The sidebar's view options ("…"): a titlebar control at the sidebar's
    /// trailing edge, opening the Organize / Sort / Show menu.
    pub(super) fn render_sidebar_view_trigger(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view_open = self.sidebar_view_menu.is_open();
        let view_focus = self.sidebar_view_trigger_focus.clone();
        let view_trigger = div()
            .id("sidebar-view-options")
            .role(gpui::Role::Button)
            .aria_label("Sidebar view options")
            .aria_expanded(view_open)
            .track_focus(&view_focus)
            .size(px(24.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.border.opacity(0.0))
            .focus_visible(|el| el.border_color(theme.border_strong))
            .cursor_pointer()
            .text_color(theme.text_muted)
            .bg(if view_open {
                theme.glass_hover()
            } else {
                theme.glass_hover().opacity(0.0)
            })
            .hover(|el| el.bg(theme.glass_hover()))
            // Over the titlebar's drag strip: block it, as the other titlebar
            // controls do, so a click never starts a window drag.
            .occlude()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, _| {
                    window.prevent_default();
                    this.sidebar_view_menu.note_trigger_press();
                }),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                if this.sidebar_view_menu.take_press_was_open() {
                    this.close_sidebar_view_menu(cx);
                } else {
                    this.open_sidebar_view_menu(window, cx);
                }
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if matches!(
                    event.keystroke.key.to_ascii_lowercase().as_str(),
                    "enter" | "space" | "arrowdown"
                ) {
                    cx.stop_propagation();
                    if this.sidebar_view_menu.is_open()
                        && !event.keystroke.key.eq_ignore_ascii_case("arrowdown")
                    {
                        this.close_sidebar_view_menu(cx);
                    } else if !this.sidebar_view_menu.is_open() {
                        this.open_sidebar_view_menu(window, cx);
                    }
                }
            }))
            .tooltip(|_, cx| cx.new(|_| SidebarViewOptionsTooltip).into())
            .tooltip_show_delay(std::time::Duration::from_millis(350))
            .child(
                icon(icons::MORE_HORIZONTAL)
                    .size(px(16.0))
                    .text_color(theme.text_muted),
            );
        let view_trigger = if self.sidebar_view_menu.get().is_some() {
            let closing = self.sidebar_view_menu.closing_since();
            let menu = self.render_sidebar_view_menu(theme, cx);
            view_trigger.relative().child(popover::anchored_menu_right(
                "sidebar-view-options-menu",
                menu,
                closing,
            ))
        } else {
            view_trigger
        };
        view_trigger.into_any_element()
    }

    /// The dropdown card: search on top, "All projects" + space rows (check on
    /// the active filter; right-click for rename/remove) + "New project".
    fn render_spaces_menu(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let theme = &theme.for_popup();
        let (search, active, focus, list_scroll) = {
            let Some(menu) = self.spaces_menu.get() else {
                return div().into_any_element();
            };
            (
                menu.search.clone(),
                menu.active,
                menu.focus.clone(),
                menu.list_scroll.clone(),
            )
        };
        let rows = self.spaces_menu_rows(cx);
        let scrollbar = popover::rail(self, "spaces-menu-scrollbar", theme, cx);
        let filter = self.settings.space_filter.clone();
        let filter_key = self.space_filter_key(cx);
        // Each row is a whole repository: its artwork and name, plus the
        // offline glyph when no device holding it is reachable. Consume
        // `rows` to avoid cloning the list children per frame.
        let details: Vec<(
            SpacesMenuRow,
            SharedString,
            Option<zeron_proto::Space>,
            bool,
            bool,
        )> = {
            let state = self.state.read(cx);
            rows.into_iter()
                .map(|row| match row {
                    SpacesMenuRow::All => (
                        SpacesMenuRow::All,
                        SharedString::from("All projects"),
                        None,
                        false,
                        filter.is_none(),
                    ),
                    SpacesMenuRow::Space(id) => match state.space_row(&id) {
                        Some(space) => {
                            let key = zeron_proto::view::project_key(space);
                            let selected = filter_key.as_deref() == Some(key.as_str());
                            let members = state.project_members(space);
                            let (_, offline) = state.project_device_tag(&members, Utc::now());
                            (
                                SpacesMenuRow::Space(id),
                                state
                                    .representative_space(space)
                                    .display_name()
                                    .to_string()
                                    .into(),
                                Some(space.clone()),
                                offline,
                                selected,
                            )
                        }
                        None => {
                            let selected = filter.as_deref() == Some(id.as_str());
                            (
                                SpacesMenuRow::Space(id),
                                SharedString::from("?"),
                                None,
                                false,
                                selected,
                            )
                        }
                    },
                    // spaces_menu_rows never yields this variant — the
                    // footer is rendered by the card, not the list.
                    SpacesMenuRow::AddSpace => unreachable!(),
                })
                .collect()
        };
        // The pinned footer's keyboard-nav index: one past the last
        // scrollable row, its permanent place at the end of the nav order.
        let add_index = details.len();
        let profile = self.active_sidebar_pin_profile_key(cx);
        let icons: Vec<AnyElement> = details
            .iter()
            .map(|(_, _, space, _, selected)| match space {
                Some(space) => self.project_icons.render(
                    &self.state,
                    Some(space),
                    profile.clone(),
                    *selected,
                    cx,
                ),
                None => icon(icons::FOLDER)
                    .size(px(14.0))
                    .text_color(theme.text_muted)
                    .into_any_element(),
            })
            .collect();

        let list = popover::menu_scroll_host("spaces-menu-list-host")
            .on_hover(cx.listener(Self::on_spaces_menu_list_hover))
            .child(popover::faded_menu_list(
                &list_scroll,
                popover::menu_scroll_list("spaces-menu-list", &list_scroll)
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    // Same scroll budget as the composer project menu.
                    .max_h(px(224.0))
                    .children(details.into_iter().zip(icons).enumerate().map(
                        |(ix, ((row, label, _, offline, selected), art))| {
                            let menu_space = match &row {
                                SpacesMenuRow::Space(id) => Some(id.clone()),
                                _ => None,
                            };
                            let activate = row;
                            popover::picker_row(
                                theme,
                                selected,
                                ix == active,
                                format!("spaces-menu-row-{ix}"),
                            )
                            .id(("spaces-menu-row", ix))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.activate_spaces_menu_row(activate.clone(), cx);
                            }))
                            .when_some(menu_space, |el, space_id| {
                                el.on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                        this.space_menu.open((space_id.clone(), event.position));
                                        cx.notify();
                                    }),
                                )
                            })
                            .child(popover::picker_icon_slot(art))
                            .child(div().flex_1().min_w_0().truncate().child(label))
                            // Disconnected glyph, not the word (user request).
                            .when(offline, |el| {
                                el.child(
                                    icon(icons::WIFI_OFF)
                                        .size(px(12.0))
                                        .flex_none()
                                        .text_color(theme.warning.opacity(0.8)),
                                )
                            })
                            // No check glyph — the selected row's glass plate
                            // is the selection signal.
                        },
                    )),
            ))
            .children(scrollbar);

        popover::popover_card(theme)
            // Match the trigger row as the sidebar is resized. Both live
            // inside the same SPACE_SM horizontal gutters.
            .w(px(self.sidebar_content_width() - 2.0 * Theme::SPACE_SM))
            .track_focus(&focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                this.spaces_menu_key(event, cx)
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.close_spaces_menu(cx);
            }))
            .flex()
            .flex_col()
            .child(popover::search_input_frame(
                theme,
                search.into_any_element(),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    // Same 2px rhythm as the composer project menu.
                    .gap(px(2.0))
                    .child(list)
                    // "New project" is a pinned action row under the list
                    // (the chat composer's project menu treatment) —
                    // scrolling must never carry it away, and its nav index
                    // (`add_index`) keeps it LAST.
                    .child(popover::picker_divider())
                    .child(
                        popover::picker_row(
                            theme,
                            false,
                            active == add_index,
                            "spaces-menu-add".to_string(),
                        )
                        .id("spaces-menu-add")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.activate_spaces_menu_row(SpacesMenuRow::AddSpace, cx);
                        }))
                        .child(popover::picker_icon_slot(
                            icon(icons::PLUS)
                                .size(px(12.0))
                                .text_color(theme.text_muted)
                                .into_any_element(),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(SharedString::from("New project")),
                        ),
                    ),
            )
            .into_any_element()
    }

    /// Flat top-to-bottom chat ids exactly as [`Self::render_active_rows`]
    /// draws them: pins first, then the user's sort, device grouping,
    /// and local-device promotion. Jump shortcuts and session cycling read
    /// this projection so keyboard order never drifts from the screen.
    pub(super) fn sidebar_visible_order(&self, cx: &Context<Self>) -> Vec<String> {
        let filter = self.settings.space_filter.clone();
        let profile_key = self.active_sidebar_pin_profile_key(cx);
        let saved_pins = self.active_sidebar_pins(cx);
        let frozen_pinned = self
            .pinned_session_drag
            .as_ref()
            .filter(|drag| {
                drag.filter == filter && profile_key.as_deref() == Some(&drag.profile_key)
            })
            .map(|drag| drag.visible_ids.clone());
        let pinned_order = frozen_pinned
            .as_ref()
            .map_or(saved_pins.as_slice(), |ids| ids.as_slice());
        let state = self.state.read(cx);
        let chats = self.sorted_sidebar_chats(state, Utc::now(), filter.as_deref());
        let chats: Vec<zeron_proto::Chat> =
            chats.into_iter().map(|(_, chat)| chat.clone()).collect();
        let (pinned_chats, chats): (Vec<_>, Vec<_>) = chats
            .into_iter()
            .partition(|chat| pinned_order.contains(&chat.id));
        let custom_sections = self.active_sidebar_sections(cx);
        let mut custom_order = Vec::new();
        let listed: HashSet<&str> = chats.iter().map(|chat| chat.id.as_str()).collect();
        for section in &custom_sections {
            if !section.collapsed {
                custom_order.extend(
                    section
                        .session_ids
                        .iter()
                        .filter(|id| listed.contains(id.as_str()))
                        .cloned(),
                );
            }
        }
        let chats: Vec<_> = chats
            .into_iter()
            .filter(|chat| {
                !custom_sections
                    .iter()
                    .any(|section| section.session_ids.contains(&chat.id))
            })
            .collect();
        let ordered = pinned_chats
            .into_iter()
            .map(|chat| chat.id)
            .chain(custom_order.iter().cloned())
            .chain(grouped_visible_order(
                chats,
                self.settings.sidebar_organization,
                filter.is_some(),
                state.local_device_id.as_deref(),
                |chat| sidebar_project_group(state, chat).0,
            ))
            .collect::<Vec<_>>();
        let mut visible = project_pinned_first(&ordered, pinned_order);
        if !self.sessions_open
            && self.settings.sidebar_organization == SidebarOrganization::InOneList
        {
            visible.retain(|id| pinned_order.contains(id) || custom_order.contains(id));
        }
        if !self.pinned_open {
            let pins: HashSet<&str> = pinned_order.iter().map(String::as_str).collect();
            visible.retain(|id| !pins.contains(id.as_str()));
        }
        if let Some(pair) = self.sidebar_split_card(pinned_order) {
            pair.adjoin(&mut visible);
        }
        visible
    }

    /// The Drafts disclosure (t3code SidebarDraftRow rows): every unsent draft
    /// the user has invested content in, space-filtered like sessions, two
    /// lines — target, then the typed prompt — with a discard ✕ on hover. The
    /// open draft renders its frozen snapshot (or nothing, if it never had
    /// one). `None` when no draft has a row.
    pub(super) fn render_drafts_section(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<(String, f32, AnyElement)> {
        let filter = self.settings.space_filter.clone();
        let rows: Vec<(String, String, String)> = {
            let state = self.state.read(cx);
            let focused = state.focused_id();
            state
                .draft_sessions
                .iter()
                .filter(|d| match filter.as_deref() {
                    Some(space) => d.space_id.as_deref() == Some(space) && !d.no_project,
                    None => true,
                })
                .filter_map(|d| {
                    let prompt = if focused.as_deref() == Some(d.id.as_str()) {
                        self.frozen_draft_rows.get(&d.id).cloned()?
                    } else {
                        crate::draft_store::entry_prompt(&d.id, cx)?
                    };
                    Some((
                        d.id.clone(),
                        super::switcher::draft_target_label(d, state),
                        transcript::single_line(&prompt),
                    ))
                })
                .collect()
        };
        let focused = self
            .state
            .read(cx)
            .focused_id()
            .filter(|_| !matches!(self.route, Route::PullRequests));
        if rows.is_empty() {
            return None;
        }
        let row_height = super::chat_row_height(true);
        let row_count = rows.len();
        let rows: Vec<AnyElement> = rows
            .into_iter()
            .map(|(id, target, prompt)| {
                let selected = focused.as_deref() == Some(id.as_str());
                self.render_draft_row(id, target, prompt, selected, theme, cx)
            })
            .collect();
        let key = "drafts";
        let motion_key = format!("group:{key}");
        let collapsed = self.sidebar_collapsed_groups.contains(key);
        let body_height = SIDEBAR_DISCLOSURE_BODY_INSET
            + row_height * row_count as f32
            + SIDEBAR_LIST_GAP * row_count.saturating_sub(1) as f32;
        self.begin_queued_sidebar_reveal(&motion_key, !collapsed, body_height);
        let chevron = self.sidebar_disclosure_chevron(&motion_key, !collapsed, theme);
        let toggle_motion_key = motion_key.clone();
        let header = sidebar_disclosure_header(
            theme,
            None,
            "Drafts".into(),
            collapsed.then_some(row_count),
            None,
            chevron,
        )
        .id("sidebar-group-drafts")
        .role(gpui::Role::Button)
        .aria_label("Drafts")
        .aria_expanded(!collapsed)
        .on_click(cx.listener(move |this, _, _, cx| {
            let was_open = !this.sidebar_collapsed_groups.contains(key);
            this.begin_sidebar_disclosure_motion(
                &toggle_motion_key,
                if was_open { body_height } else { 0.0 },
                if was_open { 0.0 } else { body_height },
            );
            if was_open {
                this.sidebar_collapsed_groups.insert(key.to_owned());
            } else {
                this.sidebar_collapsed_groups.remove(key);
            }
            cx.notify();
        }));
        let body = div()
            .w_full()
            .flex()
            .flex_col()
            .pt(px(SIDEBAR_DISCLOSURE_BODY_INSET))
            .gap(px(SIDEBAR_LIST_GAP))
            .children(rows);
        let body = self.render_sidebar_disclosure_body(
            &motion_key,
            !collapsed,
            body_height,
            body.into_any_element(),
        );
        let height = SIDEBAR_DISCLOSURE_SECTION_HEIGHT + if collapsed { 0.0 } else { body_height };
        let element = div()
            .w_full()
            .flex()
            .flex_col()
            .pt(px(SIDEBAR_SECTION_GAP))
            .child(header)
            .child(body)
            .into_any_element();
        Some((format!("g:{key}"), height, element))
    }

    fn render_draft_row(
        &self,
        id: String,
        target: String,
        prompt: String,
        selected: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row_id = format!("draft-{id}");
        let hovered = self.chat_status_hover.as_deref() == Some(row_id.as_str());
        let bg = if selected {
            crate::theme::glass_selected_bg()
        } else {
            theme.warning.opacity(if hovered { 0.08 } else { 0.04 })
        };
        let open_id = id.clone();
        let discard_id = id.clone();
        let hover_id = row_id.clone();
        let discard_group = SharedString::from(format!("draft-discard-{id}"));
        let discard = div()
            .id(discard_group.clone())
            .role(gpui::Role::Button)
            .aria_label("Discard draft")
            .group(discard_group.clone())
            .ml_auto()
            .h(px(20.0))
            .px(px(4.0))
            .flex_none()
            .flex()
            .items_center()
            .rounded(px(8.0))
            .cursor_pointer()
            .opacity(if hovered { 1.0 } else { 0.0 })
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.discard_draft(discard_id.clone(), cx);
            }))
            .child(
                icon(icons::LUCIDE_X)
                    .size(px(12.0))
                    .text_color(theme.text_muted)
                    .group_hover(discard_group, |s| s.text_color(theme.text)),
            );
        div()
            .id(SharedString::from(row_id.clone()))
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(crate::transcript::single_line(&prompt)))
            .aria_description(SharedString::from(format!("Draft in {target}")))
            .aria_selected(selected)
            .h(px(super::chat_row_height(true)))
            .py(px(super::SIDEBAR_CARD_INSET))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    if this.chat_status_hover.as_deref() != Some(hover_id.as_str()) {
                        this.chat_status_hover = Some(hover_id.clone());
                        cx.notify();
                    }
                } else if this.chat_status_hover.as_deref() == Some(hover_id.as_str()) {
                    this.chat_status_hover = None;
                    cx.notify();
                }
            }))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_draft(open_id.clone(), true, cx);
            }))
            .child(
                div()
                    .size_full()
                    .rounded(px(8.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .flex()
                    .flex_col()
                    .bg(bg)
                    .child(
                        div()
                            .h(px(20.0))
                            .min_w_0()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                icon(icons::LUCIDE_SQUARE_PEN)
                                    .size(px(12.0))
                                    .flex_none()
                                    .text_color(theme.warning),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .line_height(px(16.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(target)),
                            )
                            .child(discard),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .w_full()
                            .min_w_0()
                            .truncate()
                            .text_size(crate::typography::ui_rems(14.0))
                            .line_height(px(20.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text.opacity(0.9))
                            .child(SharedString::from(prompt)),
                    ),
            )
            .into_any_element()
    }

    /// The group a chat's row sits under when the sidebar groups by device or
    /// by project; `None` in one list.
    fn sidebar_group_key(&self, state: &AppState, chat: &zeron_proto::Chat) -> Option<String> {
        match self.settings.sidebar_organization {
            SidebarOrganization::ByDevice => Some(chat.device_id.clone()),
            // Under a space filter every row is that one project, so ByProject
            // draws no group header there.
            SidebarOrganization::ByProject if self.settings.space_filter.is_none() => {
                Some(sidebar_project_group(state, chat).0)
            }
            SidebarOrganization::Inbox => {
                let now = Utc::now();
                self.inbox_shelf(state, state.display_status_for(chat, now), chat, now)
                    .group()
                    .map(|(key, _)| key.to_string())
            }
            SidebarOrganization::ByProject | SidebarOrganization::InOneList => None,
        }
    }

    fn inbox_shelf(
        &self,
        state: &AppState,
        status: ChatIndicator,
        chat: &zeron_proto::Chat,
        now: DateTime<Utc>,
    ) -> InboxShelf {
        InboxShelf::of(
            status,
            sidebar_settled_at(state, status, chat, now),
            self.settings.sidebar_working_shelf,
        )
    }

    fn sorted_sidebar_chats<'a>(
        &self,
        state: &'a AppState,
        now: DateTime<Utc>,
        filter: Option<&str>,
    ) -> Vec<(ChatIndicator, &'a zeron_proto::Chat)> {
        let mut chats = state.sidebar_chats(now, filter);
        if self.settings.sidebar_organization == SidebarOrganization::Inbox {
            chats.sort_by_cached_key(|(status, chat)| {
                inbox_rank(
                    *status,
                    chat,
                    state
                        .session_for(&chat.id)
                        .and_then(|session| session.started_at),
                    self.sidebar_inbox_marks.get(&chat.id).copied(),
                    sidebar_settled_at(state, *status, chat, now),
                    self.settings.sidebar_working_shelf,
                )
            });
        } else {
            chats.sort_by(|left, right| {
                compare_sidebar_rows(
                    self.settings.sidebar_sort,
                    self.settings.sidebar_active_first,
                    *left,
                    *right,
                )
            });
        }
        chats
    }

    fn note_inbox_marks(&mut self, now: DateTime<Utc>, cx: &mut Context<Self>) {
        if self.settings.sidebar_organization != SidebarOrganization::Inbox {
            return;
        }
        let state = self.state.read(cx);
        for (status, chat) in state.overview_chats(now) {
            let working = status == ChatIndicator::Working;
            let mark = self
                .sidebar_inbox_marks
                .entry(chat.id.clone())
                .or_insert(InboxMark {
                    working,
                    since: None,
                });
            if mark.working != working {
                mark.working = working;
                mark.since = Some(if working {
                    now
                } else {
                    state
                        .session_for(&chat.id)
                        .map_or(now, |session| session.updated_at.min(now))
                });
            }
        }
    }

    /// A device/project group's key in `sidebar_collapsed_groups`.
    fn sidebar_group_collapse_key(&self, group: &str) -> String {
        let organization = match self.settings.sidebar_organization {
            SidebarOrganization::ByDevice => "device",
            SidebarOrganization::ByProject => "project",
            SidebarOrganization::InOneList => "list",
            SidebarOrganization::Inbox => "inbox",
        };
        format!("{organization}:{group}")
    }

    /// Open whatever hides `chat_id`'s sidebar row — the sidebar itself, the
    /// Pinned / Sessions / Archived disclosure, its custom section or group,
    /// the Archived page — so an inline rename has a row to draw on. Each
    /// disclosure opens with its usual motion. False when the row cannot be
    /// shown: the project filter hides it (the filter is the user's to
    /// clear), or a synced section could not be expanded. Either way the
    /// sidebar notice says why.
    pub(super) fn reveal_sidebar_chat(&mut self, chat_id: &str, cx: &mut Context<Self>) -> bool {
        let (active, archived) = {
            let state = self.state.read(cx);
            let active = state
                .sidebar_chats(Utc::now(), self.settings.space_filter.as_deref())
                .into_iter()
                .find(|(_, chat)| chat.id == chat_id)
                .map(|(_, chat)| chat.clone());
            let archived = self
                .archived_sidebar_chats(cx)
                .iter()
                .position(|chat| chat.id == chat_id);
            (active, archived)
        };
        if active.is_none() && archived.is_none() {
            self.show_notice(crate::toast::ToastKind::Error, "Clear the project filter to rename this session", cx);
            return false;
        }
        if let Some(chat) = &active {
            if self.active_sidebar_pins(cx).contains(&chat.id) {
                if !self.pinned_open {
                    self.pinned_open = true;
                    self.queue_sidebar_reveal("pinned");
                }
            } else if let Some(section) = self
                .active_sidebar_sections(cx)
                .into_iter()
                .find(|section| section.session_ids.contains(&chat.id))
            {
                if section.collapsed {
                    if !self.change_sidebar_section(
                        zeron_proto::SidebarSectionChange::Collapse {
                            id: section.id.clone(),
                            collapsed: false,
                        },
                        cx,
                    ) {
                        return false;
                    }
                    self.queue_sidebar_reveal(&format!("custom:{}", section.id));
                }
            } else if let Some(group) = self.sidebar_group_key(self.state.read(cx), chat) {
                let collapse_key = self.sidebar_group_collapse_key(&group);
                if self.sidebar_collapsed_groups.remove(&collapse_key) {
                    self.queue_sidebar_reveal(&format!("group:{collapse_key}"));
                }
            } else if !self.sessions_open {
                self.sessions_open = true;
                self.queue_sidebar_reveal("sessions");
            }
        } else if let Some(index) = archived {
            if self.settings.sidebar_organization == SidebarOrganization::Inbox {
                self.show_notice(
                    crate::toast::ToastKind::Error,
                    "Inbox has no Archived section; rename this session from another view",
                    cx,
                );
                return false;
            }
            if !self.settings.sidebar_show_archived {
                self.settings.sidebar_show_archived = true;
                self.schedule_save(cx);
            }
            if !self.archived_open {
                self.archived_open = true;
                self.archived_shown = SHELF_INITIAL_ROWS;
                self.queue_sidebar_reveal("archived");
            }
            // Page the shelf just far enough to include the row, in the
            // same steps "Show more" takes.
            let shown = self.archived_shown.max(SHELF_INITIAL_ROWS);
            if index >= shown {
                self.archived_shown = SHELF_INITIAL_ROWS
                    + (index + 1 - SHELF_INITIAL_ROWS).div_ceil(SHELF_PAGE_ROWS) * SHELF_PAGE_ROWS;
            }
        }
        if self.settings.sidebar_collapsed {
            self.toggle_sidebar(cx);
        }
        cx.notify();
        true
    }

    /// Mark a disclosure opened for a reveal: its motion starts on the next
    /// render, where the body height is known. Like a header click, it
    /// replaces the resort glide for this movement.
    fn queue_sidebar_reveal(&mut self, key: &str) {
        self.sidebar_reveal_motions.insert(key.to_owned());
        self.sidebar_prev_order.clear();
        self.sidebar_resort.clear();
        self.sidebar_new_keys.clear();
    }

    /// Start a revealed disclosure's opening motion now that its height is
    /// known.
    pub(super) fn begin_queued_sidebar_reveal(&mut self, key: &str, open: bool, body_height: f32) {
        if open && self.sidebar_reveal_motions.remove(key) {
            self.begin_sidebar_disclosure_motion(key, 0.0, body_height);
        }
    }

    /// Shared metadata and visibility settings for active and archived sessions.
    fn sidebar_chat_data(
        &self,
        status: ChatIndicator,
        chat: zeron_proto::Chat,
        state: &AppState,
        now: DateTime<Utc>,
    ) -> ActiveChatRow {
        let space = state.space_for_chat(&chat);
        let project = match (space, chat.space_id.as_deref()) {
            (Some(space), _) => Some(space.display_name().to_string()),
            (None, None) => None,
            (None, Some(_)) => Some("?".to_string()),
        };
        let device = state
            .device_name(&chat.device_id)
            .unwrap_or("Unknown device")
            .to_string();
        let folder = match (project, state.device_name(&chat.device_id)) {
            (Some(project), _) => project,
            (None, Some(_)) => device.clone(),
            (None, None) => String::new(),
        };
        // The branch shows whenever the engine has stamped one —
        // main-checkout sessions included, not just worktrees.
        let branch = crate::change_requests::conversation_branch(&chat, &state.spaces)
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .map(str::to_string)
            .filter(|_| self.settings.sidebar_show_branch);
        let change_request = state
            .change_request_for_chat(&chat)
            .cloned()
            .filter(|_| self.settings.sidebar_show_pull_request);
        let group = match self.settings.sidebar_organization {
            SidebarOrganization::ByDevice => Some((chat.device_id.clone(), device)),
            SidebarOrganization::ByProject if self.settings.space_filter.is_none() => {
                Some(sidebar_project_group(state, &chat))
            }
            SidebarOrganization::Inbox => self
                .inbox_shelf(state, status, &chat, now)
                .group()
                .map(|(key, label)| (key.to_string(), label.to_string())),
            SidebarOrganization::ByProject | SidebarOrganization::InOneList => None,
        };
        let created_by = chat.created_by_chat_id.as_deref().and_then(|creator| {
            let creator = state.chats.iter().find(|row| row.id == creator)?;
            Some(transcript::single_line(
                creator.title.as_deref().unwrap_or("New session"),
            ))
        });
        ActiveChatRow {
            status,
            chat: chat.clone(),
            folder,
            branch,
            change_request,
            group,
            created_by,
        }
    }

    fn sidebar_row_band(&self) -> Option<std::ops::Range<f32>> {
        if !self.sidebar_windowing
            || self.pinned_session_drag.is_some()
            || self.sidebar_session_transfer.is_some()
            || self.sidebar_session_return.is_some()
        {
            return None;
        }
        let height = f32::from(self.sidebar_scroll.bounds().size.height);
        let top = -f32::from(self.sidebar_scroll.offset().y);
        (height > 0.0).then(|| top - height..top + 2.0 * height)
    }

    /// The sidebar's Sessions list: every session (idle included) of the
    /// filter space — or all spaces under "All" — attention-sorted. Rows are
    /// keyed for the FLIP resort glide.
    pub(super) fn render_active_rows(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> SidebarSessionRows {
        let now = Utc::now();
        self.note_inbox_marks(now, cx);
        let filter = self.settings.space_filter.clone();
        let profile_key = self.active_sidebar_pin_profile_key(cx);
        let saved_pins = self.active_sidebar_pins(cx);
        let frozen_pinned = self
            .pinned_session_drag
            .as_ref()
            .filter(|drag| {
                drag.filter == filter && profile_key.as_deref() == Some(&drag.profile_key)
            })
            .map(|drag| drag.visible_ids.clone());
        let mut rows: Vec<ActiveChatRow> = {
            let state = self.state.read(cx);
            self.sorted_sidebar_chats(state, now, filter.as_deref())
                .into_iter()
                .map(|(status, chat)| self.sidebar_chat_data(status, chat.clone(), state, now))
                .collect()
        };
        let pinned_order = frozen_pinned
            .as_ref()
            .map_or(saved_pins.as_slice(), |ids| ids.as_slice());
        let base_ids: Vec<String> = rows.iter().map(|row| row.chat.id.clone()).collect();
        let ordered_ids = project_pinned_first(&base_ids, pinned_order);
        let rank: std::collections::HashMap<&str, usize> = ordered_ids
            .iter()
            .enumerate()
            .map(|(ix, id)| (id.as_str(), ix))
            .collect();
        rows.sort_by_key(|row| {
            rank.get(row.chat.id.as_str())
                .copied()
                .unwrap_or(usize::MAX)
        });
        let active: HashSet<&str> = base_ids.iter().map(String::as_str).collect();
        let pinned_count = pinned_order
            .iter()
            .filter(|id| active.contains(id.as_str()))
            .collect::<HashSet<_>>()
            .len();
        let regular_rows = rows.split_off(pinned_count);
        let pinned_rows = rows;
        self.sidebar_pinned_heights = pinned_rows
            .iter()
            .map(|row| {
                row.height(
                    self.settings.sidebar_compact,
                    false,
                    self.settings.sidebar_show_project_label,
                )
            })
            .collect();
        let visible_pinned_ids = std::sync::Arc::new(
            pinned_rows
                .iter()
                .map(|row| row.chat.id.clone())
                .collect::<Vec<_>>(),
        );

        let custom_sections = self.active_sidebar_sections(cx);
        let custom_count = custom_sections.len();
        let mut custom_groups: Vec<_> = custom_sections
            .iter()
            .map(|section| {
                (
                    Some((format!("section:{}", section.id), section.name.clone())),
                    Vec::new(),
                )
            })
            .collect();
        let mut regular_groups: Vec<(Option<(String, String)>, Vec<ActiveChatRow>)> = Vec::new();
        for row in regular_rows {
            if let Some(index) = custom_sections
                .iter()
                .position(|section| section.session_ids.contains(&row.chat.id))
            {
                custom_groups[index].1.push(row);
                continue;
            }
            if let Some((_, existing)) = regular_groups
                .iter_mut()
                .find(|(group, _)| group == &row.group)
            {
                existing.push(row);
            } else {
                regular_groups.push((row.group.clone(), vec![row]));
            }
        }
        // Sections keep their placed order, as Pinned does, not activity order.
        for ((_, rows), section) in custom_groups.iter_mut().zip(&custom_sections) {
            rows.sort_by_key(|row| {
                section
                    .session_ids
                    .iter()
                    .position(|id| id == &row.chat.id)
            });
        }
        match self.settings.sidebar_organization {
            SidebarOrganization::ByDevice => {
                let local_device_id = self.state.read(cx).local_device_id.clone();
                promote_group(&mut regular_groups, local_device_id.as_deref());
            }
            SidebarOrganization::ByProject => {
                promote_group(&mut regular_groups, Some(CHATS_GROUP_KEY));
            }
            SidebarOrganization::InOneList | SidebarOrganization::Inbox => {}
        }
        let split_card = self.sidebar_split_card(pinned_order);
        if let Some(pair) = &split_card {
            pair.adjoin_grouped(&mut regular_groups, |row| row.chat.id.as_str());
        }
        let mut sections = Vec::with_capacity(regular_groups.len() + usize::from(pinned_count > 0));
        if !pinned_rows.is_empty() {
            sections.push((None, pinned_rows));
        }
        let custom_sessions = custom_groups
            .iter()
            .map(|(_, rows)| rows.len())
            .sum::<usize>();
        sections.extend(custom_groups);
        sections.extend(regular_groups);

        let returning = self.sidebar_session_transfer.is_none();
        let transfer = self.sidebar_session_transfer.as_mut().or_else(|| {
            self.sidebar_session_return
                .as_mut()
                .map(|state| &mut state.transfer)
        });
        if let Some(drag) = transfer {
            for (section_index, (group, rows)) in sections.iter().enumerate() {
                let Some(index) = rows
                    .iter()
                    .position(|row| row.chat.id == drag.payload.chat_id)
                else {
                    continue;
                };
                drag.source_group = if section_index == 0 && pinned_count > 0 {
                    "pinned".into()
                } else {
                    group
                        .as_ref()
                        .map_or_else(|| "regular".into(), |(key, _)| format!("regular:{key}"))
                };
                drag.source_index = index;
                let project_group = self.settings.sidebar_organization
                    == SidebarOrganization::ByProject
                    && group
                        .as_ref()
                        .is_some_and(|(key, _)| !key.starts_with("section:"));
                drag.row_height = rows[index].height(
                    self.settings.sidebar_compact,
                    false,
                    self.settings.sidebar_show_project_label && !project_group,
                );
                // Move the vacant slot to the destination instead of keeping two
                // holes. Sample layout and paint with the same reversible easing.
                let collapse = !returning
                    && drag
                        .preview
                        .as_ref()
                        .is_some_and(|gap| gap.group != drag.source_group);
                let target = if collapse {
                    drag.row_height
                        + if rows.len() > 1 {
                            SIDEBAR_LIST_GAP
                        } else {
                            0.0
                        }
                } else {
                    0.0
                };
                drag.source_collapse.retarget(target);
                let removed = if self.reduced_motion {
                    target
                } else {
                    drag.source_collapse.current()
                };
                if let Some(gap) = drag.preview.as_mut() {
                    let origin = f32::from(
                        drag.origin.get().y
                            - self.sidebar_scroll.bounds().top()
                            - self.sidebar_scroll.offset().y,
                    );
                    if gap.group != drag.source_group && gap.top > origin {
                        gap.top -= removed - drag.collapsed_height;
                    }
                }
                drag.collapsed_height = removed;
                let destination = drag
                    .preview
                    .as_ref()
                    .filter(|gap| !returning && gap.group != drag.source_group)
                    .map(|gap| gap.group.clone());
                if let Some(group) = &destination {
                    drag.section_gaps
                        .entry(group.clone())
                        .or_insert_with(|| SidebarSessionSlide {
                            from: 0.0,
                            to: 0.0,
                            epoch: 0,
                            started: std::time::Instant::now(),
                        });
                }
                for (group, gap) in &mut drag.section_gaps {
                    gap.retarget(if destination.as_ref() == Some(group) {
                        drag.row_height
                            + if group == "pinned" && pinned_count == 0 {
                                0.0
                            } else {
                                SIDEBAR_LIST_GAP
                            }
                    } else {
                        0.0
                    });
                }
                break;
            }
        }

        let selected = self
            .state
            .read(cx)
            .selected_chat
            .clone()
            .filter(|_| !matches!(self.route, Route::PullRequests));
        let canvas_chat_tab = {
            let state = self.state.read(cx);
            (matches!(self.route, Route::Chat) && state.selected_chat.is_none())
                .then_some(state.no_project)
        };
        let dimmed = |chat: &zeron_proto::Chat| canvas_chat_tab == Some(chat.space_id.is_some());
        let split_pair = self.split_pair.clone();
        // Re-checked at render so the chips drop the FRAME a popover opens,
        // not on the next modifier event — the jumps are suppressed under it.
        let jump_hints = self.jump_hints && !self.overlay_owns_keyboard(cx);
        let keymap = self.settings.keymap.clone();
        // Flat top-to-bottom slot across groups: the same order
        // `sidebar_visible_order` hands the jump shortcuts and cycling, so a
        // chip always names the key that opens its row.
        let visible_slots: std::collections::HashMap<_, _> = self
            .sidebar_visible_order(cx)
            .into_iter()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect();
        let mut slot = 0usize;
        let mut rendered = Vec::new();
        let mut moving_row = None;
        let band = self.sidebar_row_band();
        let row_tops = self.sidebar_row_tops.clone();
        row_tops
            .borrow_mut()
            .retain(|id, _| active.contains(id.as_str()));
        let renaming = self
            .chat_rename
            .as_ref()
            .map(|rename| rename.chat_id.clone());
        for (group, rows) in sections {
            let pinned_group = slot < pinned_count;
            let drag_group = if pinned_group {
                "pinned".to_owned()
            } else {
                group
                    .as_ref()
                    .map_or_else(|| "regular".to_owned(), |(key, _)| format!("regular:{key}"))
            };
            // A project group wears the project icon on its header instead
            // of repeating it on every row.
            let project_group = self.settings.sidebar_organization
                == SidebarOrganization::ByProject
                && group
                    .as_ref()
                    .is_some_and(|(key, _)| !key.starts_with("section:"));
            let header_icon_chat = (project_group && self.settings.sidebar_show_project_icon)
                .then(|| rows.first().map(|row| row.chat.id.clone()))
                .flatten();
            let shelf = [InboxShelf::Inbox, InboxShelf::Working, InboxShelf::Settled]
                .into_iter()
                .find(|shelf| {
                    self.settings.sidebar_organization == SidebarOrganization::Inbox
                        && group.as_ref().map(|(key, _)| key.as_str())
                            == shelf.group().map(|(key, _)| key)
                });
            let slim = shelf == Some(InboxShelf::Settled);
            let header_dimmed =
                project_group && !rows.is_empty() && rows.iter().all(|row| dimmed(&row.chat));
            let mut rendered_rows = Vec::with_capacity(rows.len());
            for (group_index, row) in rows.into_iter().enumerate() {
                let height = row.height(
                    self.settings.sidebar_compact,
                    slim,
                    self.settings.sidebar_show_project_label && !project_group,
                );
                let ActiveChatRow {
                    status,
                    chat,
                    folder,
                    branch,
                    change_request,
                    group: _,
                    created_by,
                } = row;
                let row_dimmed = dimmed(&chat);
                let time_ago: SharedString =
                    format_time_ago(chat.last_message_at.unwrap_or(chat.created_at), now).into();
                let is_selected = selected.as_deref() == Some(chat.id.as_str());
                let split_side = split_pair.as_ref().and_then(|pair| pair.side_of(&chat.id));
                let harness = self
                    .settings
                    .sidebar_show_harness
                    .then(|| crate::pickers::chat_marks(&chat, cx))
                    .unwrap_or_default();
                // Only rows a jump slot can reach wear a chip; row 10 onward
                // keeps its time-ago.
                let jump_slot = visible_slots.get(&chat.id).copied();
                let jump_label: Option<SharedString> = if jump_hints && let Some(slot) = jump_slot {
                    let combo = keymap.get(ShortcutId::JumpSession(slot));
                    (slot < JUMP_SLOTS && !combo.is_empty()).then(|| badge_combo(combo).into())
                } else {
                    None
                };
                // Sidebar targets reject an empty profile; the split zones still take it.
                let drag = (self.pinned_open || slot >= pinned_count).then(|| SidebarSessionDrag {
                    chat_id: chat.id.clone(),
                    visible_ids: visible_pinned_ids.clone(),
                    filter: filter.clone(),
                    profile_key: profile_key.clone().unwrap_or_default(),
                });
                slot += 1;
                let origin = self
                    .sidebar_session_transfer
                    .as_ref()
                    .filter(|drag| drag.payload.chat_id == chat.id)
                    .map(|drag| drag.origin.clone())
                    .or_else(|| {
                        self.sidebar_session_return
                            .as_ref()
                            .filter(|returning| returning.transfer.payload.chat_id == chat.id)
                            .map(|returning| returning.transfer.origin.clone())
                    });
                let is_moving = origin.is_some();
                let removed = self
                    .sidebar_session_transfer
                    .as_ref()
                    .or_else(|| {
                        self.sidebar_session_return
                            .as_ref()
                            .map(|state| &state.transfer)
                    })
                    .filter(|drag| drag.payload.chat_id == chat.id)
                    .map_or(0.0, |drag| drag.collapsed_height);
                let slot_height = height - removed;
                let windowed = band.as_ref().is_some_and(|band| {
                    !is_selected
                        && split_side.is_none()
                        && renaming.as_deref() != Some(chat.id.as_str())
                        && row_tops.borrow().get(&chat.id).is_some_and(|(at, top)| {
                            *at == slot && (top + slot_height < band.start || *top > band.end)
                        })
                });
                let element = if windowed {
                    div().into_any_element()
                } else {
                    let element = self.render_chat_row(
                        chat.id.clone(),
                        transcript::single_line(
                            &chat.title.clone().unwrap_or_else(|| "New session".into()),
                        )
                        .into(),
                        time_ago,
                        folder.into(),
                        branch.map(SharedString::from),
                        change_request,
                        harness,
                        created_by.map(SharedString::from),
                        status,
                        is_selected,
                        split_side,
                        false,
                        is_moving,
                        if is_moving { None } else { drag },
                        jump_label,
                        self.settings.sidebar_show_project_icon && !project_group,
                        project_group,
                        slim,
                        None,
                        theme,
                        cx,
                    );
                    // The source slot shrinks when its vacancy moves across sections.
                    // Its zero-height anchor still tracks the live activity position.
                    if let Some(origin) = origin {
                        moving_row = Some((element, height));
                        div()
                            .child(div().h(px(slot_height.max(0.0))))
                            .on_children_prepainted(move |bounds, _, _| {
                                if let Some(bounds) = bounds.first() {
                                    origin.set(bounds.origin);
                                }
                            })
                            .into_any_element()
                    } else {
                        self.render_sidebar_gap_row(element, &chat.id, &drag_group, group_index)
                    }
                };
                let row_top = self.sidebar_windowing.then(|| {
                    let tops = row_tops.clone();
                    let scroll = self.sidebar_scroll.clone();
                    let id = chat.id.clone();
                    let at = slot;
                    gpui::canvas(
                        move |bounds, window, _| {
                            let viewport = scroll.bounds();
                            tops.borrow_mut().insert(
                                id,
                                (
                                    at,
                                    f32::from(bounds.top() - viewport.top() - scroll.offset().y),
                                ),
                            );
                            let margin = viewport.size.height / 2.0;
                            // An emptied slot that drifted near the viewport builds next frame.
                            if windowed
                                && bounds.bottom() > viewport.top() - margin
                                && bounds.top() < viewport.bottom() + margin
                            {
                                window.request_animation_frame();
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0()
                });
                // The hit region stays at the natural slot while its content slides;
                // preview movement must not move its own insertion thresholds.
                let target_group = drag_group.clone();
                let element = div()
                    .id(SharedString::from(format!("session-slot-{}", chat.id)))
                    .debug_selector({
                        let id = chat.id.clone();
                        move || format!("session-slot-{id}")
                    })
                    .h(px(slot_height.max(0.0)))
                    .mb(px(slot_height.min(0.0)))
                    .flex_none()
                    .when(row_dimmed, |el| el.opacity(NEW_CHAT_DIMMED_OPACITY))
                    .child(element)
                    .children(row_top)
                    .on_drag_move::<SidebarSessionDrag>(cx.listener(
                        move |this, event: &gpui::DragMoveEvent<SidebarSessionDrag>, _, cx| {
                            if !event.bounds.contains(&event.event.position)
                                || !this.sidebar_scroll.bounds().contains(&event.event.position)
                            {
                                return;
                            }
                            let scroll_top = -f32::from(this.sidebar_scroll.offset().y);
                            let viewport_top = this.sidebar_scroll.bounds().top();
                            let Some(drag) = this.sidebar_session_transfer.as_mut() else {
                                return;
                            };
                            let after = event.event.position.y >= event.bounds.center().y;
                            let index = group_index + usize::from(after);
                            let mut top = f32::from(
                                if after {
                                    event.bounds.bottom() + px(SIDEBAR_LIST_GAP)
                                } else {
                                    event.bounds.top()
                                } - viewport_top,
                            ) + scroll_top;
                            if drag.source_group == target_group && index > drag.source_index {
                                top -= drag.row_height + SIDEBAR_LIST_GAP - drag.collapsed_height;
                            }
                            drag.preview = Some(SidebarSessionGap {
                                group: target_group.clone(),
                                index,
                                pinned: pinned_group,
                                top,
                            });
                            cx.notify();
                        },
                    ))
                    .into_any_element();
                rendered_rows.push((format!("c:{}", chat.id), slot_height, element));
            }
            if let Some(pair) = &split_card {
                self.join_split_rows(&mut rendered_rows, pair, theme, cx);
            }

            let Some((key, label)) = group else {
                rendered.extend(rendered_rows);
                if !pinned_group {
                    let extra = self.sidebar_transfer_extra_gap(&drag_group);
                    if extra > 0.0 {
                        rendered.push((
                            format!("gap:{drag_group}"),
                            extra - SIDEBAR_LIST_GAP,
                            div()
                                .flex_none()
                                .h(px((extra - SIDEBAR_LIST_GAP).max(0.0)))
                                .mb(px((extra - SIDEBAR_LIST_GAP).min(0.0)))
                                .into_any_element(),
                        ));
                    }
                }
                continue;
            };
            if let Some(id) = key.strip_prefix("section:") {
                if let Some(section) = custom_sections.iter().find(|section| section.id == id) {
                    rendered.push(self.render_custom_sidebar_section(
                        section.clone(),
                        rendered_rows,
                        drag_group,
                        theme,
                        cx,
                    ));
                    continue;
                }
            }
            let collapse_key = self.sidebar_group_collapse_key(&key);
            let motion_key = format!("group:{collapse_key}");
            let collapsed = self.sidebar_collapsed_groups.contains(&collapse_key);
            let row_count = rendered_rows.len();
            let more_height = sidebar_row_height(self.settings.sidebar_compact, true, true);
            let hidden = if slim {
                let shown = self.sidebar_settled_shown.max(SHELF_INITIAL_ROWS);
                let mut tail = rendered_rows.split_off(shown.min(row_count));
                let pulled = [
                    selected.as_deref(),
                    self.chat_rename
                        .as_ref()
                        .map(|rename| rename.chat_id.as_str()),
                ];
                for id in pulled.into_iter().flatten() {
                    let key = format!("c:{id}");
                    if let Some(index) = tail.iter().position(|(row, _, _)| *row == key) {
                        rendered_rows.push(tail.remove(index));
                    }
                }
                tail.len()
            } else {
                0
            };
            let extra_gap = self.sidebar_transfer_extra_gap(&drag_group);
            let body_height = SIDEBAR_DISCLOSURE_BODY_INSET
                + extra_gap
                + rendered_rows
                    .iter()
                    .map(|(_, height, _)| *height)
                    .sum::<f32>()
                + SIDEBAR_LIST_GAP * rendered_rows.len().saturating_sub(1) as f32
                + if hidden > 0 {
                    more_height + SIDEBAR_LIST_GAP
                } else {
                    0.0
                };
            self.begin_queued_sidebar_reveal(&motion_key, !collapsed, body_height);
            let kept_row = (collapsed && shelf.is_some())
                .then(|| {
                    let selected = format!("c:{}", selected.as_deref()?);
                    let index = rendered_rows
                        .iter()
                        .position(|(key, _, _)| *key == selected)?;
                    Some(rendered_rows.remove(index))
                })
                .flatten();
            let body = div()
                .w_full()
                .flex()
                .flex_col()
                .pt(px(SIDEBAR_DISCLOSURE_BODY_INSET))
                .gap(px(SIDEBAR_LIST_GAP))
                .children(rendered_rows.into_iter().map(|(_, _, row)| row))
                .when(hidden > 0, |el| {
                    el.child(
                        sidebar_show_more_row(
                            "settled-more",
                            hidden.min(SHELF_PAGE_ROWS),
                            more_height,
                            theme,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sidebar_settled_shown =
                                this.sidebar_settled_shown.max(SHELF_INITIAL_ROWS)
                                    + SHELF_PAGE_ROWS;
                            cx.notify();
                        })),
                    )
                })
                .when(extra_gap > 0.0, |el| {
                    el.child(
                        div()
                            .flex_none()
                            .h(px((extra_gap - SIDEBAR_LIST_GAP).max(0.0)))
                            .mb(px((extra_gap - SIDEBAR_LIST_GAP).min(0.0))),
                    )
                });
            let visible_label: SharedString = label.into();
            let chevron = self.sidebar_disclosure_chevron(&motion_key, !collapsed, theme);
            let toggle_key = collapse_key.clone();
            let toggle_motion_key = motion_key.clone();
            let group_icon = header_icon_chat.map(|chat_id| {
                self.render_project_group_icon(&chat_id, 16.0, cx)
            });
            // A real project's group gets a hover-revealed `+` that opens a new
            // chat homed on it — on this device's checkout when it has one
            // (the project picker's local-first order), never silently on a
            // possibly offline machine just because it cloned first. The
            // device chip still switches among the repository's checkouts.
            let group_name = SharedString::from(format!("sidebar-group-hover-{collapse_key}"));
            let group_space = (self.settings.sidebar_organization
                == SidebarOrganization::ByProject)
                .then(|| {
                    let state = self.state.read(cx);
                    state
                        .spaces
                        .iter()
                        .find(|space| zeron_proto::view::project_key(space) == key)
                        .and_then(|space| state.project_members(space).first().map(|m| m.id.clone()))
                })
                .flatten();
            let new_chat_button = group_space.map(|project| {
                div()
                    .id(SharedString::from(format!(
                        "sidebar-group-new-chat-{collapse_key}"
                    )))
                    .size(px(20.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .role(gpui::Role::Button)
                    .aria_label("New chat in project")
                    .opacity(0.0)
                    .group_hover(group_name.clone(), |style| style.opacity(1.0))
                    .hover(|el| el.bg(theme.glass_hover()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_new_session(Some(project.clone()), cx);
                    }))
                    .tooltip(crate::settings::widgets::text_tooltip_above(
                        "New chat in project",
                    ))
                    .child(
                        icon(icons::PLUS)
                            .size(px(14.0))
                            .text_color(sidebar_header_glyph(theme)),
                    )
                    .into_any_element()
            });
            let header =
                sidebar_disclosure_header(
                    theme,
                    group_icon,
                    visible_label.clone(),
                    collapsed.then_some(row_count),
                    new_chat_button,
                    chevron,
                )
                    .group(group_name)
                .when(header_dimmed, |el| el.opacity(NEW_CHAT_DIMMED_OPACITY))
                .id(SharedString::from(format!("sidebar-group-{collapse_key}")))
                .role(gpui::Role::Button)
                .aria_label(visible_label)
                .aria_expanded(!collapsed)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let was_open = !this.sidebar_collapsed_groups.contains(&toggle_key);
                    this.begin_sidebar_disclosure_motion(
                        &toggle_motion_key,
                        if was_open { body_height } else { 0.0 },
                        if was_open { 0.0 } else { body_height },
                    );
                    if was_open {
                        this.sidebar_collapsed_groups.insert(toggle_key.clone());
                    } else {
                        this.sidebar_collapsed_groups.remove(&toggle_key);
                    }
                    cx.notify();
                }));
            let body = self.render_sidebar_disclosure_body(
                &motion_key,
                !collapsed,
                body_height,
                body.into_any_element(),
            );
            let height = SIDEBAR_DISCLOSURE_SECTION_HEIGHT
                + kept_row
                    .as_ref()
                    .map_or(0.0, |(_, height, _)| SIDEBAR_DISCLOSURE_BODY_INSET + height)
                + if collapsed { 0.0 } else { body_height };
            let element = div()
                .w_full()
                .flex()
                .flex_col()
                .pt(px(SIDEBAR_SECTION_GAP))
                .child(header)
                .when_some(kept_row, |el, (_, _, row)| {
                    el.child(div().pt(px(SIDEBAR_DISCLOSURE_BODY_INSET)).child(row))
                })
                .child(body)
                .into_any_element();
            rendered.push((format!("g:{collapse_key}"), height, element));
        }
        SidebarSessionRows {
            custom_count,
            regular_count: base_ids
                .len()
                .saturating_sub(pinned_count + custom_sessions),
            rows: rendered,
            pinned_count,
            moving_row,
        }
    }

    pub(super) fn render_pinned_section(
        &mut self,
        items: Vec<AnyElement>,
        body_height: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.pinned_open;
        self.begin_queued_sidebar_reveal("pinned", open, body_height);
        let count = (!open).then_some(items.len());
        let chevron = self.sidebar_disclosure_chevron("pinned", open, theme);
        let header = sidebar_disclosure_header(theme, None, "Pinned".into(), count, None, chevron)
            .id("pinned-toggle")
            .role(gpui::Role::Button)
            .aria_label("Pinned")
            .aria_expanded(open)
            .debug_selector(|| "pinned-toggle".into())
            .on_drag_move::<SidebarSessionDrag>(cx.listener(
                |this, event: &gpui::DragMoveEvent<SidebarSessionDrag>, _, _| {
                    if !event.bounds.contains(&event.event.position)
                        || !this.sidebar_scroll.bounds().contains(&event.event.position)
                    {
                        return;
                    }
                    let top = f32::from(event.bounds.bottom() - this.sidebar_scroll.bounds().top())
                        + SIDEBAR_DISCLOSURE_BODY_INSET
                        - f32::from(this.sidebar_scroll.offset().y);
                    if let Some(drag) = this.sidebar_session_transfer.as_mut() {
                        drag.preview = Some(SidebarSessionGap {
                            group: "pinned".into(),
                            index: 0,
                            pinned: true,
                            top,
                        });
                    }
                },
            ))
            .on_drop::<SidebarSessionDrag>(cx.listener(|this, payload, _, cx| {
                this.finish_sidebar_session_transfer(payload, SidebarSessionDrop::Pinned(0), cx);
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                let was_open = this.pinned_open;
                this.cancel_pinned_session_drag(cx);
                this.begin_sidebar_disclosure_motion(
                    "pinned",
                    if was_open { body_height } else { 0.0 },
                    if was_open { 0.0 } else { body_height },
                );
                this.pinned_open = !was_open;
                // The disclosure owns this movement; avoid a second FLIP
                // animation on the regular sessions below it.
                this.sidebar_prev_order.clear();
                this.sidebar_resort.clear();
                this.sidebar_new_keys.clear();
                cx.notify();
            }));
        let content = div()
            .pt(px(SIDEBAR_DISCLOSURE_BODY_INSET))
            .child(Self::render_pinned_session_group(
                items,
                self.sidebar_transfer_extra_gap("pinned"),
                cx,
            ))
            .into_any_element();
        let body = self.render_sidebar_disclosure_body("pinned", open, body_height, content);
        div()
            .id("sidebar-pinned-section")
            .debug_selector(|| "sidebar-pinned-section".into())
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .into_any_element()
    }

    pub(super) fn render_sessions_section(
        &mut self,
        content: AnyElement,
        body_height: f32,
        count: usize,
        follows_pinned: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.settings.sidebar_organization != SidebarOrganization::InOneList {
            return content;
        }
        let open = self.sessions_open;
        self.begin_queued_sidebar_reveal("sessions", open, body_height);
        let chevron = self.sidebar_disclosure_chevron("sessions", open, theme);
        let header = sidebar_disclosure_header(
            theme,
            None,
            "Sessions".into(),
            (!open).then_some(count),
            None,
            chevron,
        )
            .id("sessions-toggle")
            .debug_selector(|| "sessions-toggle".into())
            .role(gpui::Role::Button)
            .aria_label("Sessions")
            .aria_expanded(open)
            .on_drag_move::<SidebarSessionDrag>(cx.listener(
                |this, event: &gpui::DragMoveEvent<SidebarSessionDrag>, _, _| {
                    if event.bounds.contains(&event.event.position) {
                        let top = f32::from(
                            event.bounds.bottom()
                                - this.sidebar_scroll.bounds().top()
                                - this.sidebar_scroll.offset().y,
                        ) + SIDEBAR_DISCLOSURE_BODY_INSET;
                        if let Some(drag) = this.sidebar_session_transfer.as_mut() {
                            drag.preview = Some(SidebarSessionGap {
                                group: "regular".into(),
                                index: 0,
                                pinned: false,
                                top,
                            });
                        }
                    }
                },
            ))
            .on_drop::<SidebarSessionDrag>(cx.listener(|this, payload, _, cx| {
                this.finish_sidebar_session_transfer(payload, SidebarSessionDrop::Regular, cx);
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.cancel_sidebar_session_transfer(cx);
                let was_open = this.sessions_open;
                this.begin_sidebar_disclosure_motion(
                    "sessions",
                    if was_open { body_height } else { 0.0 },
                    if was_open { 0.0 } else { body_height },
                );
                this.sessions_open = !was_open;
                this.sidebar_prev_order.clear();
                this.sidebar_resort.clear();
                this.sidebar_new_keys.clear();
                cx.notify();
            }));
        let content = div()
            .pt(px(SIDEBAR_DISCLOSURE_BODY_INSET))
            .child(content)
            .into_any_element();
        let body = self.render_sidebar_disclosure_body("sessions", open, body_height, content);
        div()
            .id("sidebar-sessions-section")
            .debug_selector(|| "sidebar-sessions-section".into())
            .flex()
            .flex_col()
            .pt(px(if follows_pinned {
                SIDEBAR_SECTION_GAP
            } else {
                0.0
            }))
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// The Archived shelf's chats under the project filter, in sidebar order.
    fn archived_sidebar_chats(&self, cx: &App) -> Vec<zeron_proto::Chat> {
        let state = self.state.read(cx);
        let in_filter = state.project_filter(self.settings.space_filter.as_deref());
        let mut rows: Vec<zeron_proto::Chat> = state
            .chats
            .iter()
            .filter(|c| c.archived && !zeron_proto::voice::is_orchestrator_chat(&c.id))
            .filter(|chat| in_filter(chat))
            .cloned()
            .collect();
        rows.sort_by(|left, right| compare_sidebar_chats(self.settings.sidebar_sort, left, right));
        rows
    }

    /// Archived sessions share active-row data and layout, with a restore action.
    /// The shelf starts with ten sessions and pages by 25.
    pub(super) fn render_archived_section(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.settings.sidebar_show_archived
            || self.settings.sidebar_organization == SidebarOrganization::Inbox
        {
            return None;
        }
        const INITIAL: usize = SHELF_INITIAL_ROWS;
        const PAGE: usize = SHELF_PAGE_ROWS;
        let now = Utc::now();
        let rows = self.archived_sidebar_chats(cx);
        if rows.is_empty() {
            return None;
        }
        let rows: Vec<_> = {
            let state = self.state.read(cx);
            rows.into_iter()
                .map(|chat| {
                    self.sidebar_chat_data(state.display_status_for(&chat, now), chat, state, now)
                })
                .collect()
        };
        let total = rows.len();
        let open = self.archived_open;
        let shown = self.archived_shown.max(INITIAL);
        let visible_count = total.min(shown);
        let has_more = total > shown;
        // "Show more" matches the row slot: compact rows are 29px, so the
        // button shrinks with them instead of towering over the list.
        let more_height = sidebar_row_height(self.settings.sidebar_compact, true, true);
        let body_height = SIDEBAR_DISCLOSURE_BODY_INSET
            + rows
                .iter()
                .take(shown)
                .map(|_| {
                    sidebar_row_height(
                        self.settings.sidebar_compact,
                        false,
                        self.settings.sidebar_show_project_label,
                    )
                })
                .sum::<f32>()
            + visible_count.saturating_sub(1) as f32 * SIDEBAR_LIST_GAP
            + if has_more {
                more_height + SIDEBAR_LIST_GAP
            } else {
                0.0
            };
        self.begin_queued_sidebar_reveal("archived", open, body_height);
        // Match Pinned: a muted label with a right-aligned disclosure chevron.
        // The count only shows while collapsed.
        let chevron = self.sidebar_disclosure_chevron("archived", open, theme);
        let header = sidebar_disclosure_header(
            theme,
            None,
            "Archived".into(),
            (!open).then_some(total),
            None,
            chevron,
        )
            .id("archived-toggle")
            .debug_selector(|| "archived-toggle".into())
            .role(gpui::Role::Button)
            .aria_label("Archived")
            .aria_expanded(open)
            .on_click(cx.listener(move |this, _, _, cx| {
                let was_open = this.archived_open;
                this.begin_sidebar_disclosure_motion(
                    "archived",
                    if was_open { body_height } else { 0.0 },
                    if was_open { 0.0 } else { body_height },
                );
                this.archived_open = !was_open;
                this.archived_shown = INITIAL;
                cx.notify();
            }));
        let section = div().flex().flex_col().child(header);
        let body = {
            let selected = self
                .state
                .read(cx)
                .selected_chat
                .clone()
                .filter(|_| !matches!(self.route, Route::PullRequests));
            let mut list = div()
                .flex()
                .flex_col()
                .pt(px(SIDEBAR_DISCLOSURE_BODY_INSET))
                .gap(px(SIDEBAR_LIST_GAP));
            for row in rows.into_iter().take(shown) {
                let chat = row.chat;
                let is_selected = selected.as_deref() == Some(chat.id.as_str());
                let harness = self
                    .settings
                    .sidebar_show_harness
                    .then(|| crate::pickers::chat_marks(&chat, cx))
                    .unwrap_or_default();
                list = list.child(
                    self.render_chat_row(
                        chat.id.clone(),
                        transcript::single_line(
                            &chat.title.clone().unwrap_or_else(|| "New session".into()),
                        )
                        .into(),
                        format_time_ago(chat.last_message_at.unwrap_or(chat.created_at), now)
                            .into(),
                        row.folder.into(),
                        row.branch.map(SharedString::from),
                        row.change_request,
                        harness,
                        None,
                        row.status,
                        is_selected,
                        None,
                        true,
                        false,
                        None,
                        None,
                        self.settings.sidebar_show_project_icon,
                        false,
                        false,
                        None,
                        theme,
                        cx,
                    ),
                );
            }
            let mut body = div().w_full().flex().flex_col().child(list);
            if has_more {
                let remaining = (total - shown).min(PAGE);
                body = body.child(
                    sidebar_show_more_row("archived-more", remaining, more_height, theme)
                        // Sits outside the rows' gapped column — match the
                        // list's 2px row gap or it fuses with the last row.
                        .mt(px(2.0))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.archived_shown = this.archived_shown.max(INITIAL) + PAGE;
                            cx.notify();
                        })),
                );
            }
            body.into_any_element()
        };
        let body = self.render_sidebar_disclosure_body("archived", open, body_height, body);
        // The active list already provides the spacing above Archived.
        let section = section.child(body);
        Some(section.into_any_element())
    }

    pub(super) fn render_chat_work_toggle(
        &mut self,
        leaving: bool,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = std::time::Instant::now();
        let slot = usize::from(!self.state.read(cx).no_project);
        if !leaving && self.chat_work_slide.0 != slot {
            self.chat_work_slide
                .1
                .retarget(slot as f32, self.reduced_motion, now);
            self.chat_work_slide.0 = slot;
        }
        if self.chat_work_slide.1.animating_at(now) {
            window.request_animation_frame();
        }
        let segments = [
            crate::pull_request_detail::DockSegment {
                id: "new-chat-mode-chat",
                label: "Chat",
                glyph: icons::CHAT_ROUND_LINE,
                count: None,
            },
            crate::pull_request_detail::DockSegment {
                id: "new-chat-mode-work",
                label: "Work",
                glyph: icons::FOLDER,
                count: None,
            },
        ];
        crate::pull_request_detail::dock_pill(
            "new-chat-mode",
            "Chat or Work",
            &segments,
            self.chat_work_slide.0,
            self.chat_work_slide.1.value_at(now),
            &self.chat_work_focus,
            theme,
            cx,
            Self::select_chat_work,
        )
    }

    fn select_chat_work(&mut self, slot: usize, _: &mut Window, cx: &mut Context<Self>) {
        let pickers = self.composer.read(cx).pickers().clone();
        if slot == 0 {
            pickers.update(cx, |pickers, cx| pickers.pick_no_project(cx));
        } else if !pickers.update(cx, |pickers, cx| pickers.pick_work(cx)) {
            self.open_add_space(cx);
        }
    }

    // ---- add-space flow ----

    pub(super) fn open_add_space_on_device(&mut self, device_id: &str, cx: &mut Context<Self>) {
        let device = self
            .state
            .read(cx)
            .devices
            .iter()
            .find(|device| device.id == device_id)
            .cloned();
        self.open_add_space(cx);
        if let Some(device) = device {
            self.add_space_pick_device(device, cx);
        }
    }

    pub(super) fn open_add_space(&mut self, cx: &mut Context<Self>) {
        self.release_home_folder_search(cx);
        self.command_palette = None;
        self.project_crumb_menu = popover::Popup::default();
        // "PaletteSearch" context: navigation keys stay unbound so ↑↓/←/→/⏎
        // bubble to the palette frame (`add_space_key`) instead of moving the
        // text caret — Enter and ⌘Enter are both handled there.
        let search = cx.new(|cx| {
            ComposerInput::with_context("Search devices…", "PaletteSearch", cx)
                .with_accessibility_role(gpui::Role::SearchInput)
        });
        let search_events = cx.subscribe(&search, |this: &mut Shell, _, event, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                // Typing `/` after a query that names a folder descends into
                // it — the query reads as a path segment, so the slash IS the
                // pick (shell-style). Otherwise the slash stays in the query
                // (it matches nothing, which is honest feedback).
                if this.add_space_slash_descend(cx) {
                    return;
                }
                if let Some(flow) = this.add_space.as_mut() {
                    flow.active = 0;
                    flow.list_scroll.set_offset(gpui::Point::default());
                }
                this.search_home_folders(cx);
                cx.notify();
            }
        });
        self.add_space = Some(AddSpaceFlow {
            step: ProjectStep::Devices,
            location: None,
            device: None,
            search,
            browser: Loadable::Idle,
            drives: Loadable::Idle,
            browser_path: None,
            home: None,
            browser_repo: false,
            active: 0,
            submit_busy: false,
            error: None,
            focus: cx.focus_handle(),
            list_scroll: gpui::ScrollHandle::new(),
            crumb_scroll: gpui::ScrollHandle::new(),
            crumb_key: String::new(),
            focus_pending: true,
            load_task: None,
            drives_task: None,
            submit_task: None,
            home_search: HomeFolderSearch::default(),
            _search_events: search_events,
        });
        cx.notify();
    }

    /// Close the new-project picker, releasing the host's home index.
    pub(super) fn close_add_space(&mut self, cx: &mut Context<Self>) {
        self.release_home_folder_search(cx);
        self.add_space = None;
        cx.notify();
    }

    /// Whether the picker browses the Home location, where the query also
    /// searches every folder under home.
    fn add_space_in_home(&self) -> bool {
        self.add_space.as_ref().is_some_and(|flow| {
            flow.step == ProjectStep::Folders
                && flow
                    .location
                    .as_ref()
                    .is_some_and(|(_, path)| path.is_none())
        })
    }

    /// Look the query up under the device's home. The first call in Home
    /// also starts the host's index, so the first real query is fast.
    fn search_home_folders(&mut self, cx: &mut Context<Self>) {
        let in_home = self.add_space_in_home();
        let engine = self.state.read(cx).engine().cloned();
        let local = self.state.read(cx).local_device_id.clone();
        let Some(flow) = self.add_space.as_mut() else {
            return;
        };
        let query = flow.search.read(cx).text().trim().to_owned();
        let device_id = flow.device.as_ref().map(|d| d.id.clone());
        let search = &mut flow.home_search;
        search.generation += 1;
        search.loading = false;
        search.indexing = false;
        search.task = None;
        search.results.clear();
        let Some(engine) = engine.filter(|_| in_home && !search.unsupported) else {
            return;
        };
        if !search.started {
            // Its own task: typing must not cancel it.
            search.started = true;
            search.warm_task = Some(request_home_folders(
                engine.clone(),
                device_id.clone(),
                local.clone(),
                String::new(),
                None,
                cx,
            ));
        }
        if query.chars().count() < HOME_FOLDER_MIN_QUERY_CHARS
            || crate::pickers::is_typed_path(&query)
        {
            return;
        }
        search.loading = true;
        let generation = Some(search.generation);
        search.task = Some(request_home_folders(
            engine, device_id, local, query, generation, cx,
        ));
    }

    /// Tell the host the picker is done with its home index (it stays when a
    /// projectless chat warmed it).
    fn release_home_folder_search(&mut self, cx: &mut Context<Self>) {
        let engine = self.state.read(cx).engine().cloned();
        let local = self.state.read(cx).local_device_id.clone();
        let Some(flow) = self.add_space.as_mut() else {
            return;
        };
        let search = std::mem::take(&mut flow.home_search);
        flow.home_search.unsupported = search.unsupported;
        let (true, Some(engine)) = (search.started, engine) else {
            return;
        };
        let mut params = serde_json::Map::new();
        add_space_target(
            &mut params,
            flow.device.as_ref().map(|d| d.id.as_str()),
            local.as_deref(),
        );
        cx.spawn(async move |_, _| {
            if let Err(error) = engine
                .client()
                .call(
                    methods::RELEASE_HOME_FOLDER_SEARCH,
                    serde_json::Value::Object(params),
                )
                .await
            {
                tracing::debug!(%error, "home folder search release failed");
            }
        })
        .detach();
    }

    /// Selecting a device advances to its locations.
    fn add_space_pick_device(&mut self, device: Device, cx: &mut Context<Self>) {
        let Some(flow) = self.add_space.as_mut() else {
            return;
        };
        flow.focus_pending = true;
        flow.step = ProjectStep::Locations;
        flow.location = None;
        flow.load_task = None;
        flow.drives_task = None;
        flow.list_scroll.set_offset(gpui::Point::default());
        flow.device = Some(device);
        flow.browser = Loadable::Idle;
        flow.drives = Loadable::Idle;
        flow.browser_path = None;
        flow.home = None;
        flow.browser_repo = false;
        flow.active = 0;
        flow.error = None;
        let search = flow.search.clone();
        search.update(cx, |input, cx| {
            input.set_placeholder("Search locations…", cx);
            input.set_text("", cx);
        });
        self.load_space_drives(cx);
        cx.notify();
    }

    fn add_space_goto_location(
        &mut self,
        name: String,
        path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if path.is_some() {
            self.release_home_folder_search(cx);
        }
        let Some(flow) = self.add_space.as_mut() else {
            return;
        };
        flow.focus_pending = true;
        flow.step = ProjectStep::Folders;
        flow.location = Some((name, path.clone()));
        flow.browser_repo = false;
        let search = flow.search.clone();
        search.update(cx, |input, cx| {
            input.set_placeholder("Search folders…", cx);
            input.set_text("", cx);
        });
        self.load_space_folders(path, cx);
        // Entering Home starts the host's index (an empty query only warms).
        self.search_home_folders(cx);
    }

    fn add_space_back_to(&mut self, step: ProjectStep, cx: &mut Context<Self>) {
        self.release_home_folder_search(cx);
        let Some(flow) = self.add_space.as_mut() else {
            return;
        };
        flow.focus_pending = true;
        flow.step = step;
        flow.load_task = None;
        flow.browser = Loadable::Idle;
        flow.browser_path = None;
        flow.location = None;
        flow.browser_repo = false;
        flow.active = 0;
        flow.error = None;
        flow.list_scroll.set_offset(gpui::Point::default());
        if step == ProjectStep::Devices {
            flow.drives_task = None;
            flow.device = None;
            flow.drives = Loadable::Idle;
            flow.home = None;
        }
        let search = flow.search.clone();
        search.update(cx, |input, cx| {
            input.set_placeholder(
                if step == ProjectStep::Devices {
                    "Search devices…"
                } else {
                    "Search locations…"
                },
                cx,
            );
            input.set_text("", cx);
        });
        cx.notify();
    }

    fn add_space_devices(&self, cx: &App) -> Vec<Device> {
        let Some(flow) = &self.add_space else {
            return Vec::new();
        };
        let devices = &self.state.read(cx).devices;
        let names: Vec<_> = devices.iter().map(|d| d.name.as_str()).collect();
        popover::filter_indices(flow.search.read(cx).text(), &names)
            .into_iter()
            .map(|ix| devices[ix].clone())
            .collect()
    }

    fn add_space_locations(&self, cx: &App) -> Vec<(String, Option<String>)> {
        let Some(flow) = &self.add_space else {
            return Vec::new();
        };
        let locations: Vec<_> = std::iter::once(("Home".to_string(), None))
            .chain(
                flow.drives
                    .ready()
                    .into_iter()
                    .flatten()
                    .map(|d| (d.name.clone(), Some(d.path.clone()))),
            )
            .collect();
        let names: Vec<_> = locations.iter().map(|(name, _)| name.as_str()).collect();
        popover::filter_indices(flow.search.read(cx).text(), &names)
            .into_iter()
            .map(|ix| locations[ix].clone())
            .collect()
    }

    /// ListDrives on the flow's device (relay-forwarded when remote).
    /// Failures stay silent — the section just shows Home; the folder
    /// browser's own error row already covers "device didn't respond".
    fn load_space_drives(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let local = self.state.read(cx).local_device_id.clone();
        let Some(flow) = self.add_space.as_mut() else {
            return;
        };
        let device_id = flow.device.as_ref().map(|d| d.id.clone());
        flow.drives = Loadable::Loading;
        flow.drives_task = Some(cx.spawn(async move |this, cx| {
            let mut params = serde_json::Map::new();
            // Only target remote devices — local calls skip the relay.
            if let (Some(target), local) = (&device_id, &local)
                && local.as_deref() != Some(target.as_str())
            {
                params.insert(
                    "targetDeviceId".into(),
                    serde_json::Value::String(target.clone()),
                );
            }
            let result = engine
                .client()
                .call(methods::LIST_DRIVES, serde_json::Value::Object(params))
                .await;
            this.update(cx, |shell, cx| {
                if let Some(flow) = shell.add_space.as_mut() {
                    flow.drives = match result {
                        Ok(value) => match serde_json::from_value::<DriveListing>(value) {
                            Ok(listing) => Loadable::Ready(listing.drives),
                            Err(err) => Loadable::Error(err.to_string()),
                        },
                        Err(err) => Loadable::Error(err.to_string()),
                    };
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The current listing's folder rows filtered by the search query
    /// (prefix matches first — `popover::filter_indices`).
    fn add_space_filtered(&self, cx: &App) -> Vec<zeron_proto::FolderEntry> {
        let Some(flow) = self.add_space.as_ref() else {
            return Vec::new();
        };
        if flow.step != ProjectStep::Folders {
            return Vec::new();
        }
        let Some(listing) = flow.browser.ready() else {
            return Vec::new();
        };
        let dirs = browser_rows(listing);
        let query = flow.search.read(cx).text().to_string();
        let names: Vec<&str> = dirs.iter().map(|e| e.name.as_str()).collect();
        popover::filter_indices(&query, &names)
            .into_iter()
            .map(|ix| dirs[ix].clone())
            .collect()
    }

    /// The Folders step's rows: the open folder's matching subfolders, then
    /// matches from anywhere under home not already listed.
    fn add_space_folder_rows(&self, cx: &App) -> Vec<FolderRow> {
        let local = self.add_space_filtered(cx);
        let Some(flow) = self.add_space.as_ref() else {
            return Vec::new();
        };
        let open = flow.browser.ready().map(|listing| listing.path.as_str());
        let listed: HashSet<String> = open
            .map(|open| {
                local
                    .iter()
                    .map(|entry| crate::pickers::child_path(open, &entry.name))
                    .chain(std::iter::once(open.to_owned()))
                    .map(|path| folder_path_key(&path))
                    .collect()
            })
            .unwrap_or_default();
        let home = if self.add_space_in_home() {
            flow.home_search.results.as_slice()
        } else {
            &[]
        };
        local
            .into_iter()
            .map(FolderRow::Local)
            .chain(
                home.iter()
                    .filter(|found| !listed.contains(&folder_path_key(&found.path)))
                    .cloned()
                    .map(FolderRow::Home),
            )
            .collect()
    }

    /// Descend into the highlighted (filtered) folder; clears the query.
    /// A path-shaped query with no matching rows browses the typed path
    /// instead — `/disk2⏎` must work, not sit on "No folders match" (an
    /// absolute query can never match a folder name anyway).
    fn add_space_open_active(&mut self, cx: &mut Context<Self>) {
        let Some(flow) = self.add_space.as_ref() else {
            return;
        };
        match flow.step {
            ProjectStep::Devices => {
                if let Some(device) = self.add_space_devices(cx).get(flow.active).cloned() {
                    self.add_space_pick_device(device, cx);
                }
                return;
            }
            ProjectStep::Locations => {
                if let Some((name, path)) = self.add_space_locations(cx).get(flow.active).cloned() {
                    self.add_space_goto_location(name, path, cx);
                }
                return;
            }
            ProjectStep::Folders => {}
        }
        let rows = self.add_space_folder_rows(cx);
        let Some(flow) = self.add_space.as_ref() else {
            return;
        };
        if rows.is_empty() {
            let text = flow.search.read(cx).text().to_string();
            if let Some(target) = crate::pickers::typed_path_target(&text, flow.home.as_deref()) {
                self.add_space_descend(target, false, cx);
            }
            return;
        }
        let entry = match rows.get(flow.active) {
            Some(FolderRow::Local(entry)) => entry,
            Some(FolderRow::Home(found)) => {
                let (path, is_repo) = (found.path.clone(), found.is_repo);
                self.add_space_descend(path, is_repo, cx);
                return;
            }
            None => return,
        };
        let Some(listing) = flow.browser.ready() else {
            return;
        };
        let full = crate::pickers::child_path(&listing.path, &entry.name);
        let is_repo = entry.is_repo;
        let search = flow.search.clone();
        if let Some(flow) = self.add_space.as_mut() {
            flow.browser_repo = is_repo;
        }
        search.update(cx, |input, cx| input.set_text("", cx));
        self.load_space_folders(Some(full), cx);
    }

    /// Slash-descend: when the query ends in `/` and the part before it names
    /// a folder of the current listing (exact name — matching casing wins
    /// over a case-colliding sibling — else a unique prefix), descend into it
    /// as though it were picked. Returns whether it fired —
    /// descending clears the query, so the caller must not keep acting on the
    /// old text.
    fn add_space_slash_descend(&mut self, cx: &mut Context<Self>) -> bool {
        if self
            .add_space
            .as_ref()
            .is_none_or(|f| f.step != ProjectStep::Folders)
        {
            return false;
        }
        // A typed PATH jump: an absolute (`/disk2/`), drive-rooted (`D:\x\`)
        // or home-relative (`~/x/`) query browses that path directly — mounts
        // at unconventional roots (and anywhere else) are reachable without a
        // Locations row. Same trailing-separator trigger as the folder-name
        // descend below.
        {
            let Some(flow) = self.add_space.as_ref() else {
                return false;
            };
            let text = flow.search.read(cx).text().to_string();
            if crate::pickers::is_typed_path(&text) && text.ends_with(['/', '\\']) {
                let target = crate::pickers::typed_path_target(&text, flow.home.as_deref());
                let Some(target) = target else {
                    // Path-shaped but unresolvable (`~/…` before home is
                    // known) — leave the query alone.
                    return false;
                };
                self.add_space_descend(target, false, cx);
                return true;
            }
        }
        let target = {
            let Some(flow) = self.add_space.as_ref() else {
                return false;
            };
            let text = flow.search.read(cx).text().to_string();
            let Some(query) = text.strip_suffix('/') else {
                return false;
            };
            if query.is_empty() || query.contains('/') {
                return false;
            }
            let Some(listing) = flow.browser.ready() else {
                return false;
            };
            let dirs = browser_rows(listing);
            let names: Vec<&str> = dirs.iter().map(|e| e.name.as_str()).collect();
            crate::pickers::segment_target(&names, query).map(|ix| {
                (
                    crate::pickers::child_path(&listing.path, &dirs[ix].name),
                    dirs[ix].is_repo,
                )
            })
        };
        let Some((full, is_repo)) = target else {
            return false;
        };
        self.add_space_descend(full, is_repo, cx);
        true
    }

    /// The tab-completion target: the highlighted row when the query prefixes
    /// its name, else the first prefix match (filtering ranks those first).
    /// `(full name, remaining suffix)`; `None` on an empty query or when the
    /// match is already complete.
    fn add_space_completion(&self, cx: &App) -> Option<(String, String)> {
        let flow = self.add_space.as_ref()?;
        let query = flow.search.read(cx).text().to_string();
        if query.is_empty() {
            return None;
        }
        let rows = self.add_space_filtered(cx);
        let entry = rows
            .get(flow.active)
            .filter(|e| completion_prefix_len(&e.name, &query).is_some())
            .or_else(|| {
                rows.iter()
                    .find(|e| completion_prefix_len(&e.name, &query).is_some())
            })?;
        let len = completion_prefix_len(&entry.name, &query)?;
        if len >= entry.name.len() {
            return None;
        }
        Some((entry.name.clone(), entry.name[len..].to_string()))
    }

    /// ⇥: accept the completion — the query becomes the full folder name
    /// (the ghost the input was previewing). Descending stays on `/`/⏎.
    fn add_space_accept_completion(&mut self, cx: &mut Context<Self>) {
        let Some((name, _)) = self.add_space_completion(cx) else {
            return;
        };
        if let Some(flow) = self.add_space.as_ref() {
            let search = flow.search.clone();
            search.update(cx, |input, cx| input.set_text(name, cx));
        }
    }

    /// Descend into a specific folder row (mouse path); clears the query.
    fn add_space_descend(&mut self, full: String, is_repo: bool, cx: &mut Context<Self>) {
        let Some(flow) = self.add_space.as_mut() else {
            return;
        };
        flow.browser_repo = is_repo;
        let search = flow.search.clone();
        search.update(cx, |input, cx| input.set_text("", cx));
        self.load_space_folders(Some(full), cx);
    }

    /// ListFolders on the flow's device (relay-forwarded when remote).
    pub(super) fn load_space_folders(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        let engine = self.state.read(cx).engine().cloned();
        let local = self.state.read(cx).local_device_id.clone();
        let Some(flow) = self.add_space.as_mut() else {
            return;
        };
        flow.focus_pending = true;
        let device_id = flow.device.as_ref().map(|d| d.id.clone());
        let went_home = path.is_none();
        flow.browser_path = path.clone();
        flow.browser = Loadable::Loading;
        flow.active = 0;
        flow.list_scroll.set_offset(gpui::Point::default());
        let Some(engine) = engine else {
            flow.browser = Loadable::Error("Device is not connected".into());
            cx.notify();
            return;
        };
        flow.load_task = Some(cx.spawn(async move |this, cx| {
            let mut params = serde_json::Map::new();
            if let Some(p) = &path {
                params.insert("path".into(), serde_json::Value::String(p.clone()));
            }
            // Only target remote devices — local calls skip the relay.
            if let (Some(target), local) = (&device_id, &local)
                && local.as_deref() != Some(target.as_str())
            {
                params.insert(
                    "targetDeviceId".into(),
                    serde_json::Value::String(target.clone()),
                );
            }
            let result = engine
                .client()
                .call(methods::LIST_FOLDERS, serde_json::Value::Object(params))
                .await;
            this.update(cx, |shell, cx| {
                if let Some(flow) = shell.add_space.as_mut() {
                    flow.browser = match result {
                        Ok(value) => match serde_json::from_value::<FolderListing>(value) {
                            Ok(listing) => {
                                // A pathless browse resolved home — remember it
                                // so the breadcrumbs can fold it into the
                                // device crumb.
                                if went_home {
                                    flow.home = Some(listing.path.clone());
                                }
                                Loadable::Ready(listing)
                            }
                            Err(err) => Loadable::Error(err.to_string()),
                        },
                        Err(err) => Loadable::Error(err.to_string()),
                    };
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Create the space for the browser's current folder.
    fn submit_add_space(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(flow) = self.add_space.as_ref() else {
            return;
        };
        if flow.submit_busy || flow.step != ProjectStep::Folders {
            return;
        }
        let Some(device) = flow.device.clone() else {
            return;
        };
        let Some(listing) = flow.browser.ready() else {
            return;
        };
        let path = listing.path.clone();
        let git_detected = flow.browser_repo;
        // Same (device, folder) already has a space → just switch to it. The
        // engine dedupes this case too (a createSpace for a duplicate pair
        // no-ops), so creating would leave the minted id dangling.
        if let Some(existing) = self
            .state
            .read(cx)
            .spaces
            .iter()
            .find(|s| s.device_id == device.id && same_folder_path(&s.path, &path))
            .map(|s| s.id.clone())
        {
            self.close_add_space(cx);
            self.land_in_space(existing, cx);
            return;
        }
        let Some(flow) = self.add_space.as_mut() else {
            return;
        };
        flow.submit_busy = true;
        flow.error = None;
        let space_id = uuid::Uuid::new_v4().to_string();
        // Optimistic echo: the watch frame carrying the real row replaces it
        // by id (apply_spaces re-sorts; same-id upsert is idempotent).
        let space = Space {
            id: space_id.clone(),
            device_id: device.id.clone(),
            path: path.clone(),
            name: None,
            git_detected,
            git_checked_at: None,
            checkout_id: None,
            repository_id: None,
            created_at: Utc::now(),
        };
        self.state.update(cx, |s, cx| {
            if !s.spaces.iter().any(|existing| existing.id == space.id) {
                s.spaces.push(space);
            }
            cx.notify();
        });
        let params = serde_json::json!({
            "op": "createSpace",
            "spaceId": space_id,
            "deviceId": device.id,
            "path": path,
            "gitDetected": git_detected,
        });
        let submit_id = space_id.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = engine.client().call(methods::MUTATE, params).await;
            this.update(cx, |shell, cx| {
                match result {
                    Ok(_) => {
                        shell.close_add_space(cx);
                        shell.land_in_space(submit_id.clone(), cx);
                    }
                    Err(err) => {
                        // Roll the optimistic row back; surface the error inline.
                        shell.state.update(cx, |s, cx| {
                            s.spaces.retain(|space| space.id != submit_id);
                            cx.notify();
                        });
                        if let Some(flow) = shell.add_space.as_mut() {
                            flow.submit_busy = false;
                            flow.error = Some(format!("{err}").into());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        });
        if let Some(flow) = self.add_space.as_mut() {
            flow.submit_task = Some(task);
        }
        cx.notify();
    }

    /// Back traverses folders, then locations, then devices.
    fn add_space_go_up(&mut self, cx: &mut Context<Self>) {
        let Some(flow) = &self.add_space else {
            return;
        };
        match flow.step {
            ProjectStep::Devices => return,
            ProjectStep::Locations => self.add_space_back_to(ProjectStep::Devices, cx),
            ProjectStep::Folders => {
                let listing = flow.browser.ready();
                let root = flow
                    .location
                    .as_ref()
                    .and_then(|(_, path)| path.as_deref())
                    .or(flow.home.as_deref());
                let parent = listing
                    .filter(|l| !root.is_some_and(|root| same_folder_path(&l.path, root)))
                    .and_then(|l| parent_path(&l.path));
                if let Some(parent) = parent {
                    self.add_space_descend(parent, false, cx);
                } else {
                    self.add_space_back_to(ProjectStep::Locations, cx);
                }
            }
        }
    }

    /// Palette keys (bubbling from the focused search input) — every legend
    /// maps to a REAL key: ↑↓ (or ctrl-n/p) navigate, →/⏎ open the
    /// highlighted folder, ← up a level, ⇥ completes the query to the
    /// previewed folder name, ⌘⏎ add the OPEN folder, ⌫ (empty query) also
    /// goes up, esc closes. (Typing `/` also descends — see the Edited
    /// subscription.)
    fn add_space_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        // ←/→ act on the FOLDERS, not the text cursor — the palette is a
        // navigator first; queries are short and edited with ⌫.
        match event.keystroke.key.as_str() {
            "right" => {
                self.add_space_open_active(cx);
                return;
            }
            "left" => {
                self.add_space_go_up(cx);
                return;
            }
            // Unbound in "PaletteSearch" (like enter), so it bubbles here
            // instead of editing text or moving focus.
            "tab" => {
                self.add_space_accept_completion(cx);
                return;
            }
            _ => {}
        }
        let key = popover::classify_key(
            event.keystroke.key.as_str(),
            event.keystroke.modifiers.platform,
            event.keystroke.modifiers.control,
        );
        match key {
            popover::MenuKey::Escape => {
                self.close_add_space(cx);
                cx.stop_propagation();
            }
            popover::MenuKey::Up | popover::MenuKey::Down => {
                let count = match self.add_space.as_ref().map(|f| f.step) {
                    Some(ProjectStep::Devices) => self.add_space_devices(cx).len(),
                    Some(ProjectStep::Locations) => self.add_space_locations(cx).len(),
                    _ => self.add_space_folder_rows(cx).len(),
                };
                let delta = if key == popover::MenuKey::Up { -1 } else { 1 };
                if let Some(flow) = self.add_space.as_mut() {
                    flow.active = popover::menu_step(Some(flow.active), count, delta).unwrap_or(0);
                    // Keep the highlighted row in view as the cursor walks
                    // past the viewport (user-reported: the list didn't
                    // follow the keyboard).
                    flow.list_scroll.scroll_to_item(flow.active);
                    cx.notify();
                }
            }
            // ⏎ opens the highlighted folder (an alias for →); the space is
            // added with ⌘⏎ — and the chord acts on the folder OPEN in the
            // breadcrumbs, not the highlight. The highlight auto-rests on the
            // first row, so a chord that took it would add arbitrary
            // subfolders; the usual target (a repo root full of subfolders)
            // is only ever "the folder you're standing in".
            popover::MenuKey::Enter => self.add_space_open_active(cx),
            popover::MenuKey::ModEnter => self.submit_add_space(cx),
            popover::MenuKey::Backspace => {
                let empty = self
                    .add_space
                    .as_ref()
                    .is_some_and(|f| f.search.read(cx).is_empty());
                if empty {
                    self.add_space_go_up(cx);
                }
            }
            popover::MenuKey::Other => {}
        }
    }

    pub(super) fn close_project_crumb_menu(&mut self, cx: &mut Context<Self>) {
        if self.project_crumb_menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.project_crumb_menu);
            cx.notify();
        }
    }

    /// The folders folded into the `…` crumb, in path order.
    fn render_project_crumb_menu(
        &self,
        hidden: &[(String, String)],
        closing: Option<std::time::Instant>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = hidden.iter().enumerate().map(|(ix, (name, full))| {
            let full = full.clone();
            popover::menu_row(theme, false, format!("project-crumb-menu-{ix}"))
                .id(("project-crumb-menu", ix))
                .child(
                    icon(icons::FOLDER)
                        .size(px(16.0))
                        .flex_none()
                        .text_color(theme.text_muted),
                )
                .child(div().min_w_0().truncate().child(name.clone()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.project_crumb_menu = popover::Popup::default();
                    this.add_space_descend(full.clone(), false, cx);
                }))
        });
        let card = popover::popover_card(theme)
            .id("project-crumb-menu")
            .min_w(px(180.0))
            .max_w(px(280.0))
            .max_h(px(280.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_project_crumb_menu(cx)))
            .children(rows);
        popover::anchored_menu_below_layer(
            "project-crumb-menu",
            card.into_any_element(),
            closing,
            6.0,
            3,
        )
    }

    /// The same glass, header, row rhythm, scroll gutters and footer as Cmd+K.
    pub(super) fn render_add_space_overlay(
        &mut self,
        viewport: gpui::Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::of(cx).for_popup();
        let flow = self.add_space.as_mut()?;
        if std::mem::take(&mut flow.focus_pending) {
            window.focus(&flow.search.focus_handle(cx), cx);
        }
        let step = flow.step;
        let search = flow.search.clone();
        let focus = flow.focus.clone();
        let scroll = flow.list_scroll.clone();
        let device = flow.device.clone();
        let listing = flow.browser.ready().cloned();
        let location = flow.location.clone();
        let home = flow.home.clone();
        let browser_path = flow.browser_path.clone();
        let load_error = flow.browser.error().map(str::to_string);
        let error = flow.error.clone();
        let busy = flow.submit_busy;
        let active = flow.active;
        let loading = matches!(flow.browser, Loadable::Idle | Loadable::Loading);
        let drives_loading = matches!(flow.drives, Loadable::Loading);
        let home_loading = flow.home_search.loading;
        let home_status = if flow.home_search.indexing {
            Some("Indexing…")
        } else if home_loading {
            Some("Searching…")
        } else {
            None
        };
        let ghost = self
            .add_space_completion(cx)
            .map(|(_, suffix)| SharedString::from(suffix));
        search.update(cx, |input, cx| {
            input.set_ghost(ghost, cx);
        });
        let query = search.read(cx).text().to_string();
        // Cmd+K's row rhythm: 30px rows, 16px muted glyphs, 8px list gutters.
        let row = |ix: usize| {
            popover::menu_row(&theme, ix == active, format!("project-result-{ix}"))
                .id(("project-result", ix))
                .role(gpui::Role::ListBoxOption)
                .aria_selected(ix == active)
                .rounded(px(popover::PALETTE_ITEM_RADIUS))
                .min_h(px(30.0))
                .py(px(4.0))
                // Pointer motion moves the highlight, so hover and keyboard
                // never light two rows. Motion only: rows scrolling under a
                // resting pointer must not steal the keyboard's place.
                .on_mouse_move(cx.listener(move |this, _: &gpui::MouseMoveEvent, _, cx| {
                    if let Some(flow) = this.add_space.as_mut()
                        && flow.active != ix
                    {
                        flow.active = ix;
                        cx.notify();
                    }
                }))
        };
        let glyph_el = |glyph: &'static str| {
            icon(glyph)
                .size(px(16.0))
                .flex_none()
                .text_color(theme.text_muted)
        };
        let label_el = |label: String| {
            div().flex_1().min_w_0().child(popover::search_highlight(
                label.into(),
                Some(&query),
                &theme,
            ))
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        match step {
            ProjectStep::Devices => {
                for (ix, device) in self.add_space_devices(cx).into_iter().enumerate() {
                    let online = self.state.read(cx).device_online(&device.id, Utc::now());
                    let name = device.name.clone();
                    rows.push(
                        row(ix)
                            .aria_label(SharedString::from(name.clone()))
                            .child(glyph_el(device_glyph(&device.platform)))
                            .child(label_el(name))
                            .child(
                                div()
                                    .size(px(5.0))
                                    .flex_none()
                                    .rounded_full()
                                    .bg(if online {
                                        theme.success
                                    } else {
                                        theme.text_faint
                                    }),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.add_space_pick_device(device.clone(), cx)
                            }))
                            .into_any_element(),
                    );
                }
            }
            ProjectStep::Locations => {
                for (ix, (name, path)) in self.add_space_locations(cx).into_iter().enumerate() {
                    let glyph = if path.is_none() {
                        icons::HOME
                    } else {
                        icons::HARD_DRIVE
                    };
                    let label = name.clone();
                    rows.push(
                        row(ix)
                            .aria_label(SharedString::from(label.clone()))
                            .child(glyph_el(glyph))
                            .child(label_el(label))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.add_space_goto_location(name.clone(), path.clone(), cx)
                            }))
                            .into_any_element(),
                    );
                }
            }
            ProjectStep::Folders => {
                if !loading && load_error.is_none() {
                    let base = listing.as_ref().map(|l| l.path.as_str()).unwrap_or("");
                    let mut first_home = true;
                    for (ix, folder) in self.add_space_folder_rows(cx).into_iter().enumerate() {
                        let entry = match folder {
                            FolderRow::Local(entry) => entry,
                            FolderRow::Home(found) => {
                                let el = home_folder_row(row(ix), &found, &query, &theme, cx);
                                rows.push(if std::mem::take(&mut first_home) {
                                    div()
                                        .flex()
                                        .flex_col()
                                        .when(ix > 0, |el| el.pt(px(6.0)))
                                        .child(home_folders_header(&theme, home_status))
                                        .child(el)
                                        .into_any_element()
                                } else {
                                    el.into_any_element()
                                });
                                continue;
                            }
                        };
                        let full = crate::pickers::child_path(base, &entry.name);
                        let is_repo = entry.is_repo;
                        rows.push(
                            row(ix)
                                .aria_label(SharedString::from(entry.name.clone()))
                                .child(glyph_el(icons::FOLDER))
                                .child(label_el(entry.name))
                                .when(is_repo, |el| {
                                    el.child(
                                        icon(icons::GIT_BRANCH)
                                            .size(px(14.0))
                                            .flex_none()
                                            .text_color(theme.text_muted),
                                    )
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.add_space_descend(full.clone(), is_repo, cx)
                                }))
                                .into_any_element(),
                        );
                    }
                }
            }
        }
        if let Some(flow) = self.add_space.as_mut() {
            flow.active = flow.active.min(rows.len().saturating_sub(1));
        }
        let count = rows.len();
        // End spacing belongs to the content, so it scrolls out of the fade
        // instead of leaving a permanent gutter beside the chrome (Cmd+K).
        let rows = rows.into_iter().enumerate().map(|(ix, content)| {
            div()
                .flex_none()
                .px(px(8.0))
                .when(ix == 0, |row| row.pt(px(8.0)))
                .when(ix + 1 == count, |row| row.pb(px(8.0)))
                .child(content)
        });
        let mut results = div()
            .id("project-results")
            .role(gpui::Role::ListBox)
            .aria_label("Results")
            .min_h_0()
            .max_h(px(command_palette::palette_results_height(viewport)))
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .flex()
            .flex_col()
            .gap(px(SIDEBAR_LIST_GAP))
            .children(rows);
        if step == ProjectStep::Folders && loading {
            results = results.child(div().p(px(8.0)).child(popover::skeleton_rows(
                "project-loading",
                &theme,
                5,
                cx.entity_id(),
                cx,
            )));
        } else if let Some(message) = load_error.filter(|_| step == ProjectStep::Folders) {
            results = results.child(
                popover::error_row(&theme, &message).p(px(16.0)).child(
                    popover::btn_ghost(&theme, "Retry", "project-retry")
                        .id("project-retry")
                        .on_click(cx.listener(|this, _, _, cx| {
                            let path = this.add_space.as_ref().and_then(|f| f.browser_path.clone());
                            this.load_space_folders(path, cx);
                        })),
                ),
            );
        } else if count == 0 && !(step == ProjectStep::Locations && drives_loading) {
            let (title, hint) = match step {
                ProjectStep::Devices => ("No devices found", "Try another device name.".into()),
                ProjectStep::Locations => {
                    ("No locations found", "Try Home or a drive name.".into())
                }
                ProjectStep::Folders if query.is_empty() => (
                    "No folders here",
                    format!(
                        "Add this folder with {}, or go back with ←.",
                        crate::settings::badge_combo("mod-enter")
                    ),
                ),
                ProjectStep::Folders if home_loading => (
                    "Searching Home…",
                    "Looking for matching folders anywhere under Home.".to_string(),
                ),
                ProjectStep::Folders if home_status == Some("Indexing…") => (
                    "No folders match yet",
                    "Home is still being indexed — keep typing or try again.".to_string(),
                ),
                ProjectStep::Folders => (
                    "No folders match",
                    "Type a path like ~/code or /mnt to jump there.".to_string(),
                ),
            };
            results = results.child(command_palette::palette_empty(&theme, title, hint));
        }
        if step == ProjectStep::Locations && drives_loading {
            results = results.child(
                div()
                    .px(px(16.0))
                    .pb(px(8.0))
                    .text_color(theme.text_muted)
                    .text_size(crate::typography::ui_rems(11.0))
                    .child("Loading locations…"),
            );
        }
        let results = command_palette::palette_results_fade(results, &scroll);

        // Breadcrumbs: one line that scrolls sideways under edge fades rather
        // than wrapping, and follows the open folder as the path grows. Deep
        // paths fold their middle folders into a `…` menu.
        let mut specs: Vec<Crumb> = vec![Crumb {
            name: "New project".into(),
            glyph: None,
            current: step == ProjectStep::Devices,
            target: CrumbTarget::Devices,
        }];
        let mut crumb_key = String::new();
        if let Some(device) = device {
            crumb_key.push_str(&device.id);
            specs.push(Crumb {
                name: device.name.into(),
                glyph: Some(device_glyph(&device.platform)),
                current: step == ProjectStep::Locations,
                target: CrumbTarget::Locations,
            });
        }
        let mut hidden: Vec<(String, String)> = Vec::new();
        if let Some((name, path)) = location {
            let root = path.clone().or(home.clone());
            // While a descend loads, keep showing the REQUESTED path so the
            // trail doesn't collapse to the location and pop back.
            let open_path = listing
                .as_ref()
                .map(|l| l.path.clone())
                .or(browser_path)
                .or(root.clone());
            let at_root = open_path.is_none()
                || open_path
                    .as_deref()
                    .zip(root.as_deref())
                    .is_some_and(|(open, root)| same_folder_path(open, root));
            crumb_key.push_str(&name);
            specs.push(Crumb {
                name: name.clone().into(),
                glyph: Some(if path.is_none() {
                    icons::HOME
                } else {
                    icons::HARD_DRIVE
                }),
                current: at_root,
                target: CrumbTarget::Location(name, path),
            });
            if let Some(open_path) = open_path {
                crumb_key.push_str(&open_path);
                let mut folders: Vec<(String, String)> = breadcrumbs(&open_path)
                    .into_iter()
                    .filter(|(_, full)| !root.as_deref().is_some_and(|root| path_under(root, full)))
                    .collect();
                hidden = fold_crumb_folders(&mut folders);
                if !hidden.is_empty() {
                    specs.push(Crumb {
                        name: "…".into(),
                        glyph: None,
                        current: false,
                        target: CrumbTarget::More,
                    });
                }
                for (name, full) in folders {
                    specs.push(Crumb {
                        name: name.into(),
                        glyph: None,
                        current: same_folder_path(&full, &open_path),
                        target: CrumbTarget::Folder(full),
                    });
                }
            }
        }
        if hidden.is_empty() && self.project_crumb_menu.get().is_some() {
            self.project_crumb_menu = popover::Popup::default();
        }
        let menu_open = self.project_crumb_menu.is_open();
        let menu_closing = self.project_crumb_menu.closing_since();
        let menu_mounted = self.project_crumb_menu.get().is_some();
        let mut trail: Vec<AnyElement> = Vec::new();
        for (ix, spec) in specs.into_iter().enumerate() {
            if ix > 0 {
                trail.push(
                    icon(icons::ALT_ARROW_RIGHT)
                        .size(px(12.0))
                        .flex_none()
                        .text_color(theme.text_faint)
                        .into_any_element(),
                );
            }
            let group: SharedString = format!("project-crumb-{ix}").into();
            let more = matches!(spec.target, CrumbTarget::More);
            let color = if spec.current || (more && menu_open) {
                theme.text
            } else {
                theme.text_muted
            };
            let long = spec.name.chars().count() > 26;
            let mut el = div()
                .id(group.clone())
                .group(group.clone())
                .relative()
                .flex_none()
                .h(px(24.0))
                .px(px(6.0))
                .rounded(px(6.0))
                .flex()
                .items_center()
                .gap(px(5.0))
                .text_color(color)
                .when(more, |el| el.min_w(px(24.0)).justify_center())
                .when(more && menu_open, |el| el.bg(theme.element_hover))
                .when_some(spec.glyph, |el, glyph| {
                    el.child(
                        icon(glyph)
                            .size(px(14.0))
                            .flex_none()
                            .text_color(color)
                            .group_hover(group.clone(), |s| s.text_color(theme.text)),
                    )
                })
                .child(div().max_w(px(180.0)).truncate().child(spec.name.clone()))
                .when(long, |el| {
                    el.tooltip(crate::settings::widgets::text_tooltip(spec.name.clone()))
                });
            // The open crumb is where you already are: no hover, no click.
            if !spec.current {
                el = el
                    .cursor_pointer()
                    .role(gpui::Role::Button)
                    .aria_label(if more {
                        SharedString::from("Show hidden folders")
                    } else {
                        spec.name.clone()
                    })
                    .hover(|s| s.bg(theme.element_hover).text_color(theme.text));
                el = match spec.target {
                    CrumbTarget::Devices => el.on_click(cx.listener(|this, _, _, cx| {
                        this.add_space_back_to(ProjectStep::Devices, cx)
                    })),
                    CrumbTarget::Locations => el.on_click(cx.listener(|this, _, _, cx| {
                        this.add_space_back_to(ProjectStep::Locations, cx)
                    })),
                    CrumbTarget::Location(name, path) => {
                        el.on_click(cx.listener(move |this, _, _, cx| {
                            this.add_space_goto_location(name.clone(), path.clone(), cx)
                        }))
                    }
                    CrumbTarget::Folder(full) => el.on_click(cx.listener(move |this, _, _, cx| {
                        this.add_space_descend(full.clone(), false, cx)
                    })),
                    CrumbTarget::More => el
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, _| {
                                this.project_crumb_menu.note_trigger_press()
                            }),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            // A press that found the menu open closes it.
                            if this.project_crumb_menu.take_press_was_open() {
                                this.close_project_crumb_menu(cx);
                            } else {
                                this.project_crumb_menu.open(());
                            }
                            cx.notify();
                        })),
                };
            }
            if more && menu_mounted {
                el = el.child(self.render_project_crumb_menu(&hidden, menu_closing, &theme, cx));
            }
            trail.push(el.into_any_element());
        }
        let crumb_scroll = self.add_space.as_mut().map(|flow| {
            // Reveal the open folder whenever the path changes; otherwise
            // leave the strip where the user scrolled it.
            if flow.crumb_key != crumb_key {
                flow.crumb_key = crumb_key;
                flow.crumb_scroll
                    .scroll_to_item(trail.len().saturating_sub(1));
            }
            flow.crumb_scroll.clone()
        })?;
        let trail = div()
            .id("project-crumbs")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .gap(px(2.0))
            .overflow_x_scroll()
            .track_scroll(&crumb_scroll)
            .children(trail);
        // ← mirrors the Left key: up one level, and back to Cmd+K from the
        // first step.
        let back_label = if step == ProjectStep::Devices {
            "Back to commands"
        } else {
            "Back"
        };
        let crumbs = div()
            .h(px(36.0))
            .flex_none()
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .border_b_1()
            .border_color(crate::theme::hairline(0.06))
            .text_size(crate::typography::ui_rems(12.0))
            .child(
                div()
                    .id("project-crumb-back")
                    .group("project-crumb-back")
                    .size(px(24.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .role(gpui::Role::Button)
                    .aria_label(back_label)
                    .tooltip(crate::settings::widgets::text_tooltip(back_label))
                    .hover(|s| s.bg(theme.element_hover))
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.add_space.as_ref().map(|f| f.step) == Some(ProjectStep::Devices) {
                            this.close_add_space(cx);
                            this.toggle_command_palette(window, cx);
                        } else {
                            this.add_space_go_up(cx);
                        }
                    }))
                    .child(
                        icon(icons::ARROW_LEFT)
                            .size(px(16.0))
                            .text_color(theme.text_muted)
                            .group_hover("project-crumb-back", |s| s.text_color(theme.text)),
                    ),
            )
            .child(
                crate::edge_fade::edge_faded(18.0, false, false, trail)
                    .fade_left(true)
                    .fade_right(true)
                    .fade_overflow_x(&crumb_scroll),
            );

        let shortcut = {
            let id = ShortcutId::NewProject;
            let combo = self.settings.keymap.get(id);
            let valid = Keystroke::parse(&platform_combo(combo)).is_ok();
            crate::settings::badge_combo(if valid { combo } else { id.default_combo() })
        };
        let can_add = !busy && listing.is_some();
        let footer = command_palette::palette_footer()
            .child(command_palette::command_key_hint(&theme, "↑ ↓", "Navigate"))
            .child(command_palette::command_key_hint(
                &theme,
                "↵",
                if step == ProjectStep::Folders {
                    "Open"
                } else {
                    "Select"
                },
            ))
            .when(step != ProjectStep::Devices, |el| {
                el.child(command_palette::command_key_hint(&theme, "←", "Back"))
            })
            .child(command_palette::command_key_hint(&theme, "Esc", "Close"))
            .when(step == ProjectStep::Folders, |el| {
                el.child(div().flex_1()).child(
                    div()
                        .id("project-add")
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        // Even 3px around the key chip, concentric corners
                        // (chip 5px + 3px); negative margins keep the footer
                        // height and the label on the footer's right inset.
                        .pl(px(3.0))
                        .pr(px(8.0))
                        .py(px(3.0))
                        .my(px(-3.0))
                        .mr(px(-8.0))
                        .rounded(px(8.0))
                        .role(gpui::Role::Button)
                        .aria_label("Add project")
                        .when(can_add, |el| {
                            el.cursor_pointer()
                                .hover(|s| s.bg(crate::theme::card_selected_bg()))
                                .active(|s| s.opacity(0.8))
                                .on_click(cx.listener(|this, _, _, cx| this.submit_add_space(cx)))
                        })
                        .when(!can_add, |el| el.opacity(0.5))
                        .child(popover::kbd_hint(
                            &theme,
                            &crate::settings::badge_combo("mod-enter"),
                        ))
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(10.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(if busy { "Adding…" } else { "Add project" }),
                        ),
                )
            });
        let card =
            command_palette::palette_card("add-space-palette", &focus, viewport, &theme)
                .role(gpui::Role::Dialog)
                .aria_label("Add project")
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                    this.add_space_key(event, cx)
                }))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    // The crumb menu floats outside the card; its own
                    // mouse-down-out dismisses it without closing the palette.
                    if this.project_crumb_menu.get().is_some() {
                        return;
                    }
                    this.close_add_space(cx);
                }))
                .child(command_palette::palette_header(
                    &theme,
                    search.into_any_element(),
                    popover::kbd_hint(&theme, &shortcut),
                ))
                .child(crumbs)
                .child(results)
                .when_some(error, |el, error| {
                    el.child(
                        div()
                            .px(px(16.0))
                            .pb(px(8.0))
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(theme.danger)
                            .child(error),
                    )
                })
                .child(footer);
        Some(command_palette::palette_overlay(viewport, card))
    }

    // ---- space context menu / rename / delete overlays ----

    fn close_space_menu(&mut self, cx: &mut Context<Self>) {
        if self.space_menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.space_menu);
            cx.notify();
        }
    }

    pub(super) fn open_rename_space(&mut self, space_id: String, cx: &mut Context<Self>) {
        self.close_space_menu(cx);
        let current = self
            .state
            .read(cx)
            .space_row(&space_id)
            .map(|s| s.display_name().to_string())
            .unwrap_or_default();
        let input = cx.new(|cx| ComposerInput::new("Project name", cx));
        input.update(cx, |input, cx| input.set_text(current, cx));
        let events = cx.subscribe(&input, |this: &mut Shell, _, event, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                this.submit_rename_space(cx);
            }
        });
        self.rename_space_dialog = Some(RenameSpaceDialog {
            space_id,
            input,
            focus_pending: true,
            _events: events,
        });
        cx.notify();
    }

    pub(super) fn submit_rename_space(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.rename_space_dialog.take() else {
            return;
        };
        let name = dialog.input.read(cx).text().trim().to_string();
        if !name.is_empty() {
            self.mutate(
                serde_json::json!({ "op": "renameSpace", "spaceId": dialog.space_id, "name": name }),
                cx,
            );
        }
        cx.notify();
    }

    pub(super) fn delete_space(&mut self, space_id: String, cx: &mut Context<Self>) {
        self.delete_space_confirm = None;
        self.mutate(
            serde_json::json!({ "op": "deleteSpace", "spaceId": space_id }),
            cx,
        );
        cx.notify();
    }

    /// Space context menu + rename dialog + delete confirm (appended to the
    /// shell's overlay list).
    pub(super) fn render_space_overlays(
        &mut self,
        viewport: gpui::Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = Theme::of(cx).for_popup();
        let mut overlays: Vec<AnyElement> = Vec::new();

        if let Some((space_id, position)) = self.space_menu.get().cloned() {
            let closing = self.space_menu.closing_since();
            let rename_id = space_id.clone();
            let delete_id = space_id.clone();
            let menu = popover::popover_card(&theme)
                .w(px(170.0))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.close_space_menu(cx);
                }))
                .flex()
                .flex_col()
                .child(
                    popover::menu_row(&theme, false, format!("space-menu-rename-{space_id}"))
                        .id("space-menu-rename")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_rename_space(rename_id.clone(), cx)
                        }))
                        .child(icon(icons::PEN).size(px(16.0)).text_color(theme.text_muted))
                        .child(SharedString::from("Rename…")),
                )
                .child(popover::menu_separator())
                .child(
                    popover::menu_row(&theme, false, format!("space-menu-delete-{space_id}"))
                        .id("space-menu-delete")
                        .text_color(theme.danger)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.close_space_menu(cx);
                            this.delete_space_confirm = Some(delete_id.clone());
                            cx.notify();
                        }))
                        .child(
                            icon(icons::TRASH_BIN_MINIMALISTIC)
                                .size(px(16.0))
                                .text_color(theme.danger),
                        )
                        .child(SharedString::from("Remove…")),
                )
                .into_any_element();
            overlays.push(popover::menu_at(
                "space-context-menu",
                position,
                menu,
                closing,
            ));
        }

        if let Some(dialog) = &mut self.rename_space_dialog {
            if std::mem::take(&mut dialog.focus_pending) {
                window.focus(&dialog.input.focus_handle(cx), cx);
            }
            let input = dialog.input.clone();
            let card = popover::dialog_card(&theme)
                .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _, cx| {
                    if ev.keystroke.key == "escape" {
                        this.rename_space_dialog = None;
                        cx.notify();
                        cx.stop_propagation();
                    }
                }))
                .child(popover::dialog_title(&theme, "Rename project"))
                .child(
                    div()
                        .mt(px(12.0))
                        .child(popover::dialog_field(input.into_any_element())),
                )
                .child(
                    div()
                        .mt(px(16.0))
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            popover::btn_ghost(&theme, "Cancel", "rename-space-cancel")
                                .id("rename-space-cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.rename_space_dialog = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            popover::btn_primary(&theme, "Rename")
                                .id("rename-space-save")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.submit_rename_space(cx)),
                                ),
                        ),
                )
                .into_any_element();
            overlays.push(popover::modal("rename-space-dialog", viewport, card));
        }

        if let Some(space_id) = self.delete_space_confirm.clone() {
            let (name, device, count) = {
                let state = self.state.read(cx);
                let space = state.space_row(&space_id);
                (
                    space
                        .map(|s| s.display_name().to_string())
                        .unwrap_or_else(|| "this project".into()),
                    space
                        .and_then(|s| state.device_name(&s.device_id))
                        .unwrap_or("its device")
                        .to_string(),
                    state.chats_removed_with_space(&space_id),
                )
            };
            let copy = if count == 1 {
                format!(
                    "Removing “{name}” permanently deletes its 1 session on {device}. This can’t be undone."
                )
            } else {
                format!(
                    "Removing “{name}” permanently deletes its {count} sessions on {device}. This can’t be undone."
                )
            };
            let card = popover::dialog_card(&theme)
                .child(popover::dialog_title(&theme, "Remove project?"))
                .child(div().mt(px(6.0)).child(popover::dialog_body(&theme, copy)))
                .child(
                    div()
                        .mt(px(16.0))
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            popover::btn_ghost(&theme, "Cancel", "delete-space-cancel")
                                .id("delete-space-cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.delete_space_confirm = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            popover::btn_danger(&theme, "Remove")
                                .id("delete-space-confirm")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.delete_space(space_id.clone(), cx)
                                })),
                        ),
                )
                .into_any_element();
            overlays.push(popover::modal("delete-space-dialog", viewport, card));
        }

        overlays
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone as _, Utc};

    use super::{
        CHATS_GROUP_KEY, ChatIndicator, InboxMark, compare_sidebar_chats, compare_sidebar_rows,
        grouped_visible_order, inbox_rank, inbox_settled_at, promote_group,
    };
    use crate::settings::{SidebarOrganization, SidebarSort};

    fn group(device: &str, value: u8) -> (Option<(String, String)>, Vec<u8>) {
        (Some((device.into(), device.into())), vec![value])
    }

    fn space_key(chat: &zeron_proto::Chat) -> String {
        chat.space_id
            .clone()
            .unwrap_or_else(|| CHATS_GROUP_KEY.to_string())
    }

    fn chat(id: &str) -> zeron_proto::Chat {
        zeron_proto::Chat {
            id: id.into(),
            device_id: "device".into(),
            title: None,
            archived: false,
            cwd: None,
            branch: None,
            checkout_id: None,
            source_context: None,
            config: None,
            last_message_preview: None,
            last_message_at: Some(Utc.timestamp_opt(10, 0).unwrap()),
            created_at: Utc.timestamp_opt(5, 0).unwrap(),
            harness_session_id: None,
            harness_session_cwd: None,
            space_id: None,
            last_seen_at: None,
            room_gen: None,
            sidechat_source_chat_id: None,
            created_by_chat_id: None,
            previous_harnesses: Vec::new(),
            delegation: None,
            settle: None,
            unsettled_at: None,
        }
    }

    #[test]
    fn equal_sidebar_timestamps_sort_by_stable_chat_id() {
        let alpha = chat("alpha");
        let beta = chat("beta");
        assert!(compare_sidebar_chats(SidebarSort::Created, &alpha, &beta).is_lt());
        assert!(compare_sidebar_chats(SidebarSort::LastUpdated, &alpha, &beta).is_lt());
    }

    #[test]
    fn active_first_lifts_every_live_or_unseen_status_above_idle() {
        let stale = zeron_proto::Chat {
            last_message_at: Some(Utc.timestamp_opt(1, 0).unwrap()),
            ..chat("stale")
        };
        let fresh = chat("fresh");
        let sort = SidebarSort::LastUpdated;
        for status in [
            ChatIndicator::Working,
            ChatIndicator::AwaitingInput,
            ChatIndicator::Errored,
            ChatIndicator::Completed,
        ] {
            assert!(
                compare_sidebar_rows(sort, true, (status, &stale), (ChatIndicator::Idle, &fresh))
                    .is_lt()
            );
            assert!(
                compare_sidebar_rows(sort, false, (status, &stale), (ChatIndicator::Idle, &fresh))
                    .is_gt()
            );
        }
        assert!(
            compare_sidebar_rows(
                sort,
                true,
                (ChatIndicator::Completed, &fresh),
                (ChatIndicator::Working, &stale),
            )
            .is_lt()
        );
    }

    #[test]
    fn current_device_is_promoted_without_resorting_remote_groups() {
        let mut groups = vec![
            group("recent-remote", 1),
            group("local", 2),
            group("older-remote", 3),
        ];

        promote_group(&mut groups, Some("local"));

        let order: Vec<_> = groups
            .iter()
            .map(|(group, _)| group.as_ref().unwrap().0.as_str())
            .collect();
        assert_eq!(order, ["local", "recent-remote", "older-remote"]);
    }

    fn chat_in(id: &str, space_id: Option<&str>) -> zeron_proto::Chat {
        let mut chat = chat(id);
        chat.space_id = space_id.map(Into::into);
        chat
    }

    #[test]
    fn by_project_visible_order_regroups_interleaved_projects() {
        let chats = vec![
            chat_in("a1", Some("a")),
            chat_in("b1", Some("b")),
            chat_in("home", None),
            chat_in("a2", Some("a")),
            chat_in("b2", Some("b")),
        ];

        let order = grouped_visible_order(
            chats.clone(),
            SidebarOrganization::ByProject,
            false,
            None,
            space_key,
        );
        assert_eq!(order, ["home", "a1", "a2", "b1", "b2"]);

        let filtered = grouped_visible_order(
            chats.clone(),
            SidebarOrganization::ByProject,
            true,
            None,
            space_key,
        );
        assert_eq!(filtered, ["a1", "b1", "home", "a2", "b2"]);

        let flat = grouped_visible_order(
            chats,
            SidebarOrganization::InOneList,
            false,
            None,
            space_key,
        );
        assert_eq!(flat, ["a1", "b1", "home", "a2", "b2"]);
    }

    #[test]
    fn linked_chats_keep_their_own_sort_position() {
        let created_by = |id: &str, creator: &str| {
            let mut chat = chat(id);
            chat.created_by_chat_id = Some(creator.into());
            chat
        };
        let sidechat_of = |id: &str, source: &str| {
            let mut chat = chat(id);
            chat.sidechat_source_chat_id = Some(source.into());
            chat
        };
        let chats = vec![
            created_by("child", "parent"),
            chat("x"),
            sidechat_of("side", "x"),
            chat("parent"),
        ];
        assert_eq!(
            grouped_visible_order(
                chats,
                SidebarOrganization::InOneList,
                false,
                None,
                space_key
            ),
            ["child", "x", "side", "parent"]
        );
    }

    type InboxRow = (
        ChatIndicator,
        zeron_proto::Chat,
        Option<chrono::DateTime<Utc>>,
        Option<InboxMark>,
    );

    const DAY: i64 = 86_400;

    fn inbox_at(secs: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(10 * DAY + secs, 0).unwrap()
    }

    fn inbox_ids(rows: &mut [InboxRow], working_shelf: bool) -> Vec<String> {
        let now = inbox_at(1_000);
        rows.sort_by_key(|(status, chat, started, mark)| {
            let settled = inbox_settled_at(*status, chat, 3, now);
            inbox_rank(*status, chat, *started, *mark, settled, working_shelf)
        });
        rows.iter().map(|row| row.1.id.clone()).collect()
    }

    #[test]
    fn inbox_orders_returns_then_working_then_settled() {
        let at = inbox_at;
        let ids = |rows: &mut [InboxRow]| inbox_ids(rows, true);
        let replied = |id: &str, secs| {
            let mut chat = chat(id);
            chat.last_message_at = Some(at(secs));
            chat
        };
        let settle = |mut chat: zeron_proto::Chat, settled, secs| {
            chat.settle = Some(zeron_proto::SidebarSettle {
                settled,
                at: at(secs),
            });
            if !settled {
                chat.unsettled_at = Some(at(secs));
            }
            chat
        };
        let mut silent = chat("f");
        silent.last_message_at = None;
        let returned = InboxMark {
            working: false,
            since: Some(at(450)),
        };
        use ChatIndicator::{Completed, Errored, Idle, Working};
        let mut rows: Vec<InboxRow> = vec![
            (Working, replied("a", 500), Some(at(100)), None),
            (Working, replied("b", 300), Some(at(200)), None),
            (Idle, replied("c", 400), None, None),
            (Errored, replied("d", 50), None, Some(returned)),
            (Idle, replied("e", -4 * DAY), None, None),
            (Idle, silent, None, None),
            (Idle, settle(replied("g", 600), true, 700), None, None),
            (Idle, settle(replied("h", -5 * DAY), false, 800), None, None),
            (Completed, replied("i", -6 * DAY), None, None),
            (
                Working,
                settle(replied("j", -6 * DAY), true, 900),
                Some(at(150)),
                None,
            ),
        ];
        assert_eq!(
            ids(&mut rows),
            ["h", "d", "c", "f", "b", "j", "a", "g", "e", "i"]
        );
        for (id, secs) in [("a", 900), ("g", 950), ("h", 960)] {
            rows.iter_mut()
                .find(|row| row.1.id == id)
                .unwrap()
                .1
                .last_message_at = Some(at(secs));
        }
        assert_eq!(
            ids(&mut rows),
            ["h", "g", "d", "c", "f", "b", "j", "a", "e", "i"]
        );
    }

    #[test]
    fn inbox_without_working_shelf_keeps_creation_order() {
        let at = inbox_at;
        let row = |id: &str, status, created, last, unsettled: Option<i64>| -> InboxRow {
            let mut chat = chat(id);
            chat.created_at = at(created);
            chat.last_message_at = Some(at(last));
            chat.unsettled_at = unsettled.map(at);
            (status, chat, None, None)
        };
        use ChatIndicator::{Completed, Idle, Working};
        let mut rows = vec![
            row("old", Working, -2 * DAY, 900, None),
            row("new", Idle, 100, 200, None),
            row("finished", Completed, -DAY, 950, None),
            row("woke", Idle, -10 * DAY, 980, Some(970)),
            row("stale", Idle, -8 * DAY, -5 * DAY, None),
        ];
        assert_eq!(
            inbox_ids(&mut rows, false),
            ["woke", "new", "finished", "old", "stale"]
        );
        assert_eq!(
            inbox_ids(&mut rows, true),
            ["woke", "finished", "new", "old", "stale"]
        );
    }

    #[gpui::test]
    fn by_project_draws_one_section_per_project(cx: &mut gpui::TestAppContext) {
        use crate::settings::{self, UiSettings};
        use crate::state::{AppState, EngineBootConfig};
        use gpui::AppContext as _;

        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            settings::init(UiSettings::default(), dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            gpui_base::init(cx);
            cx.set_global(crate::theme::Theme::default());
            crate::app_menus::init(cx);
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            super::Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: zeron_proto::HarnessId::Mock,
                },
                cx,
            )
        });
        window
            .update(cx, |shell, _window, cx| {
                shell.state.update(cx, |state, _| {
                    state.spaces = ["a", "b"]
                        .into_iter()
                        .map(|id| {
                            serde_json::from_value(serde_json::json!({
                                "id": id, "deviceId": "local", "path": format!("/tmp/{id}"),
                                "gitDetected": false, "createdAt": Utc::now(),
                            }))
                            .unwrap()
                        })
                        .collect();
                    state.chats = [("a1", "a", 3), ("b1", "b", 2), ("a2", "a", 1)]
                        .into_iter()
                        .map(|(id, space, secs)| {
                            serde_json::from_value(serde_json::json!({
                                "id": id, "deviceId": "local", "spaceId": space,
                                "archived": false,
                                "createdAt": Utc.timestamp_opt(secs, 0).unwrap(),
                            }))
                            .unwrap()
                        })
                        .collect();
                });
                let theme = crate::theme::Theme::default();
                let keys = |shell: &mut super::Shell, cx: &mut gpui::Context<super::Shell>| {
                    shell
                        .render_active_rows(&theme, cx)
                        .rows
                        .into_iter()
                        .map(|(key, _, _)| key)
                        .collect::<Vec<_>>()
                };

                shell.settings.space_filter = None;
                shell.settings.sidebar_organization = SidebarOrganization::InOneList;
                assert_eq!(keys(shell, cx), ["c:a1", "c:b1", "c:a2"]);

                shell.settings.sidebar_organization = SidebarOrganization::ByProject;
                assert_eq!(keys(shell, cx), ["g:project:a", "g:project:b"]);
                assert_eq!(shell.sidebar_visible_order(cx), ["a1", "a2", "b1"]);

                shell.settings.space_filter = Some("a".into());
                assert_eq!(keys(shell, cx), ["c:a1", "c:a2"]);

                // An open split pulls the pair's later row up beside the
                // earlier one, across groups, in both the drawn rows (one
                // card at a1's slot) and the jump/cycle order.
                shell.settings.space_filter = None;
                shell
                    .state
                    .update(cx, |state, cx| state.select_chat(Some("a1".into()), cx));
                shell.drop_session_on_side("b1".into(), super::split::SplitSide::Right, cx);
                assert!(shell.split.is_some());
                assert_eq!(shell.sidebar_visible_order(cx), ["a1", "b1", "a2"]);
                shell.settings.sidebar_organization = SidebarOrganization::InOneList;
                assert_eq!(keys(shell, cx), ["c:a1", "c:a2"]);
                assert_eq!(shell.sidebar_visible_order(cx), ["a1", "b1", "a2"]);

                // Selecting outside the pair parks it; its card stays.
                shell
                    .state
                    .update(cx, |state, cx| state.select_chat(Some("a2".into()), cx));
                shell.reconcile_split_with_selection(Some("a2"), cx);
                assert!(shell.split.is_none());
                assert_eq!(keys(shell, cx), ["c:a1", "c:a2"]);
                assert_eq!(shell.sidebar_visible_order(cx), ["a1", "b1", "a2"]);
            })
            .unwrap();
    }

    #[test]
    fn missing_current_device_leaves_group_order_untouched() {
        let mut groups = vec![group("first", 1), group("second", 2)];
        let before = groups.clone();

        promote_group(&mut groups, Some("not-present"));

        assert_eq!(groups, before);
    }
}

/// Synthetic responses for the isolated native screenshot fixture only.
#[cfg(feature = "project-palette-fixture")]
impl Shell {
    pub fn fixture_project_responses(&mut self, cx: &mut Context<Self>) {
        if std::env::var_os("ZERON_FIXTURE_BACKGROUND").is_some() {
            self.composer
                .read(cx)
                .pickers()
                .clone()
                .update(cx, |pickers, cx| pickers.fixture_model_catalog(cx));
        }
        let Some(flow) = self.add_space.as_mut() else {
            return;
        };
        if flow.device.is_some() && !matches!(flow.drives, Loadable::Ready(_)) {
            flow.drives = Loadable::Ready(
                serde_json::from_value(serde_json::json!([
                    {"name":"Projects", "path":"/projects"},
                    {"name":"System", "path":"/"}
                ]))
                .unwrap(),
            );
            cx.notify();
        }
        if flow.step == ProjectStep::Folders && !matches!(flow.browser, Loadable::Ready(_)) {
            let path = flow
                .browser_path
                .clone()
                .unwrap_or_else(|| "/home/alex".into());
            if flow.browser_path.is_none() {
                flow.home = Some(path.clone());
            }
            let names = match path.as_str() {
                "/home/alex" => vec![
                    "Desktop",
                    "Documents",
                    "Downloads",
                    "Movies",
                    "Music",
                    "Pictures",
                    "Projects",
                    "Public",
                    "dotfiles",
                    "notes",
                    "sandbox",
                    "scratch",
                ],
                "/projects" | "/home/alex/Projects" => vec!["fieldnotes", "mobile-app", "website"],
                _ => vec!["assets", "docs", "src", "tests"],
            };
            flow.browser = Loadable::Ready(
                serde_json::from_value(serde_json::json!({
                    "path":path,
                    "entries":names.into_iter().map(|name| serde_json::json!({
                        "name":name,"isDir":true,"isRepo":name=="fieldnotes"
                    })).collect::<Vec<_>>()
                }))
                .unwrap(),
            );
            cx.notify();
        }
    }
}

#[cfg(test)]
mod project_flow_tests {
    use super::*;

    #[test]
    fn deep_crumb_trails_fold_all_but_the_deepest_folders() {
        let mut short = vec!["a", "b", "c"];
        assert!(fold_crumb_folders(&mut short).is_empty());
        assert_eq!(short, ["a", "b", "c"]);
        let mut deep = vec!["a", "b", "c", "d", "e"];
        assert_eq!(fold_crumb_folders(&mut deep), ["a", "b", "c"]);
        assert_eq!(deep, ["d", "e"]);
    }

    #[test]
    fn path_under_handles_posix_and_windows_drive_paths() {
        assert!(path_under("/media/a", "/"));
        assert!(path_under("/media/a", "/media"));
        assert!(!path_under("/media/ab", "/media/a"));
        // A drive-root crumb hides itself, not a sibling drive.
        assert!(path_under(r"D:\", r"D:\"));
        assert!(path_under(r"D:\Random", r"D:\"));
        assert!(!path_under(r"D:\Random2", r"D:\Random"));
        assert!(!path_under(r"C:\Random", r"D:\"));
        assert!(path_under(r"c:\Random\repo", r"C:\Random"));
        assert!(!path_under(r"C:\random\repo", r"C:\Random"));
        assert!(path_under(r"\\?\D:\Random", r"D:\"));
        assert!(path_under(r"D:\Random", r"\\?\D:\"));
        assert!(path_under(r"\\?\UNC\server\share\repo", r"\\server\share\"));
        assert!(!path_under(r"\\server\share2\repo", r"\\server\share\"));
        assert!(!path_under(r"\\other\share\repo", r"\\server\share\"));
    }

    #[gpui::test]
    fn devices_locations_folders_and_back_clear_stale_state(cx: &mut gpui::TestAppContext) {
        let data = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let shell = cx.new(|cx| {
            let state = cx.new(|_| {
                let mut state = AppState::new();
                state.devices = serde_json::from_value(serde_json::json!([
                    {"id":"local","name":"Studio","platform":"macos","lastSeenAt":null},
                    {"id":"remote","name":"Server","platform":"linux","lastSeenAt":null}
                ]))
                .unwrap();
                state
            });
            Shell::new(
                state,
                EngineBootConfig {
                    data_dir: data.path().into(),
                    ipc_port: 0,
                    edge_url: String::new(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: zeron_proto::HarnessId::Mock,
                },
                cx,
            )
        });
        shell.update(cx, |shell, cx| {
            shell.open_add_space(cx);
            assert_eq!(shell.add_space.as_ref().unwrap().step, ProjectStep::Devices);
            assert!(shell.add_space.as_ref().unwrap().device.is_none());
            let search = shell.add_space.as_ref().unwrap().search.clone();
            search.update(cx, |input, cx| input.set_text("server", cx));
            assert_eq!(shell.add_space_devices(cx).len(), 1);
            shell.add_space_open_active(cx);
            let flow = shell.add_space.as_mut().unwrap();
            assert_eq!(flow.step, ProjectStep::Locations);
            assert_eq!(flow.device.as_ref().unwrap().id, "remote");
            assert!(flow.search.read(cx).is_empty());
            flow.drives = Loadable::Ready(vec![DriveEntry {
                name: "Projects".into(),
                path: "/projects".into(),
            }]);
            search.update(cx, |input, cx| input.set_text("projects", cx));
            shell.add_space_open_active(cx);
            let flow = shell.add_space.as_mut().unwrap();
            assert_eq!(flow.step, ProjectStep::Folders);
            assert_eq!(flow.browser_path.as_deref(), Some("/projects"));
            flow.browser = Loadable::Ready(FolderListing {
                path: "/projects".into(),
                entries: Vec::new(),
                truncated: false,
            });
            shell.add_space_go_up(cx);
            assert_eq!(
                shell.add_space.as_ref().unwrap().step,
                ProjectStep::Locations
            );
            assert!(shell.add_space.as_ref().unwrap().browser.ready().is_none());
            shell.add_space_go_up(cx);
            let flow = shell.add_space.as_ref().unwrap();
            assert_eq!(flow.step, ProjectStep::Devices);
            assert!(flow.device.is_none());
            assert!(flow.drives.ready().is_none());
            assert!(flow.search.read(cx).is_empty());
            // Slash navigation only applies to folders, never device search.
            search.update(cx, |input, cx| input.set_text("/projects/", cx));
            assert!(!shell.add_space_slash_descend(cx));
        });
    }
}

#[cfg(test)]
mod home_folder_search_tests {
    use super::*;
    use gpui::TestAppContext;

    /// The engine side of a shell under test. ListFolders answers with
    /// `listing`; methods not being awaited answer `{}`.
    struct FakeEngine {
        runtime: tokio::runtime::Runtime,
        requests: tokio::sync::mpsc::Receiver<String>,
        replies: tokio::sync::mpsc::Sender<String>,
        listing: serde_json::Value,
    }

    impl FakeEngine {
        fn take(&mut self, method: &str, cx: &mut TestAppContext) -> Vec<serde_json::Value> {
            let mut found = Vec::new();
            loop {
                cx.run_until_parked();
                let Ok(request) = self.requests.try_recv() else {
                    return found;
                };
                let request: serde_json::Value = serde_json::from_str(&request).unwrap();
                if request["method"] == method {
                    found.push(request);
                } else if request["method"] == methods::LIST_FOLDERS {
                    let mut listing = self.listing.clone();
                    if let Some(path) = request["params"]["path"].as_str() {
                        listing["path"] = path.into();
                        listing["entries"] = serde_json::json!([]);
                    }
                    self.reply(&request, listing);
                } else {
                    self.reply(&request, serde_json::json!({}));
                }
            }
        }

        fn reply(&self, request: &serde_json::Value, ok: serde_json::Value) {
            self.send(serde_json::json!({ "id": request["id"], "ok": ok }));
        }

        fn fail(&self, request: &serde_json::Value, err: &str) {
            self.send(serde_json::json!({ "id": request["id"], "err": err }));
        }

        fn send(&self, reply: serde_json::Value) {
            // The client's reader task runs on this runtime: let it drain.
            self.runtime.block_on(async {
                self.replies.send(reply.to_string()).await.unwrap();
                while self.replies.capacity() < self.replies.max_capacity() {
                    tokio::task::yield_now().await;
                }
            });
        }
    }

    fn device(id: &str) -> Device {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "platform": "linux", "lastSeenAt": null,
        }))
        .unwrap()
    }

    /// A shell with the new-project picker open on `device` (local: "local").
    fn picker(
        cx: &mut TestAppContext,
        device_id: &str,
    ) -> (gpui::WindowHandle<Shell>, tempfile::TempDir, FakeEngine) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (out, requests) = tokio::sync::mpsc::channel(64);
        let (replies, inbound) = tokio::sync::mpsc::channel(64);
        let engine = {
            let _guard = runtime.enter();
            crate::state::EngineHandle::from_test_client(zeron_rpc::RpcClient::new(out, inbound))
        };
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let data_dir = dir.path().to_path_buf();
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| crate::state::AppState::new());
            Shell::new(
                state,
                crate::EngineBootConfig {
                    data_dir,
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: zeron_proto::HarnessId::Mock,
                },
                cx,
            )
        });
        let picked = device(device_id);
        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, _| {
                    state.local_device_id = Some("local".into());
                    state.devices = vec![picked.clone()];
                    state.set_test_engine(engine);
                });
                shell.open_add_space(cx);
                shell.add_space_pick_device(picked, cx);
            })
            .unwrap();
        let fake = FakeEngine {
            runtime,
            requests,
            replies,
            listing: serde_json::json!({
                "path": "/home/me",
                "entries": [{ "name": "code", "isDir": true, "isRepo": false }],
            }),
        };
        (window, dir, fake)
    }

    fn go_home(window: gpui::WindowHandle<Shell>, cx: &mut TestAppContext) {
        window
            .update(cx, |shell, _, cx| {
                shell.add_space_goto_location("Home".into(), None, cx)
            })
            .unwrap();
    }

    fn type_query(window: gpui::WindowHandle<Shell>, query: &str, cx: &mut TestAppContext) {
        window
            .update(cx, |shell, _, cx| {
                let search = shell.add_space.as_ref().unwrap().search.clone();
                search.update(cx, |input, cx| input.set_text(query, cx));
            })
            .unwrap();
    }

    fn rows(window: gpui::WindowHandle<Shell>, cx: &mut TestAppContext) -> Vec<FolderRow> {
        window
            .read_with(cx, |shell, cx| shell.add_space_folder_rows(cx))
            .unwrap()
    }

    fn found(path: &str, relative: &str, is_repo: bool) -> serde_json::Value {
        serde_json::json!({ "path": path, "relative": relative, "isRepo": is_repo })
    }

    fn begin_home_search(
        cx: &mut TestAppContext,
    ) -> (
        gpui::WindowHandle<Shell>,
        tempfile::TempDir,
        FakeEngine,
        serde_json::Value,
    ) {
        let (window, dir, mut engine) = picker(cx, "remote");
        go_home(window, cx);
        let warm = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
        assert_eq!(warm.len(), 1);
        engine.reply(
            &warm[0],
            serde_json::json!({ "matches": [], "indexing": true }),
        );
        cx.run_until_parked();
        // Merely warming Home must not start polling an empty query.
        cx.executor().advance_clock(Duration::from_secs(10));
        assert!(engine.take(methods::SEARCH_HOME_FOLDERS, cx).is_empty());
        type_query(window, "comet", cx);
        let request = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
        assert_eq!(request.len(), 1);
        (window, dir, engine, request.into_iter().next().unwrap())
    }

    #[gpui::test]
    fn home_search_refreshes_with_capped_backoff_and_no_overlapping_requests(
        cx: &mut TestAppContext,
    ) {
        let (window, _dir, mut engine, first) = begin_home_search(cx);
        engine.reply(
            &first,
            serde_json::json!({ "matches": [], "indexing": true }),
        );
        cx.run_until_parked();
        assert!(rows(window, cx).is_empty());
        let comet = found("/home/me/dev/comet", "dev/comet", true);
        for (round, millis) in [500, 1000, 2000, 2000].into_iter().enumerate() {
            cx.executor()
                .advance_clock(Duration::from_millis(millis - 1));
            assert!(engine.take(methods::SEARCH_HOME_FOLDERS, cx).is_empty());
            cx.executor().advance_clock(Duration::from_millis(1));
            let requests = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0]["params"], first["params"]);
            assert_eq!(requests[0]["params"]["targetDeviceId"], "remote");
            // A slow response does not queue more requests or erase rows.
            let partial = rows(window, cx);
            cx.executor().advance_clock(Duration::from_secs(10));
            assert!(engine.take(methods::SEARCH_HOME_FOLDERS, cx).is_empty());
            assert_eq!(rows(window, cx), partial);
            let matches = if round == 0 {
                vec![comet.clone()]
            } else {
                vec![
                    found("/home/me/new/comet", "new/comet", true),
                    comet.clone(),
                ]
            };
            engine.reply(
                &requests[0],
                serde_json::json!({
                    "matches": matches, "indexing": round < 3,
                }),
            );
            cx.run_until_parked();
            window
                .read_with(cx, |shell, cx| {
                    let flow = shell.add_space.as_ref().unwrap();
                    assert_eq!(flow.home_search.indexing, round < 3);
                    assert!(!flow.home_search.loading);
                    // The original selected folder survives result reordering.
                    assert!(matches!(
                        &shell.add_space_folder_rows(cx)[flow.active],
                        FolderRow::Home(found) if found.relative == "dev/comet"
                    ));
                })
                .unwrap();
        }
        assert_eq!(rows(window, cx).len(), 2);
        cx.executor().advance_clock(Duration::from_secs(10));
        assert!(engine.take(methods::SEARCH_HOME_FOLDERS, cx).is_empty());
    }

    #[gpui::test]
    fn home_search_cancels_retries_on_edits_navigation_and_close(cx: &mut TestAppContext) {
        for action in ["clear", "short", "path", "drive", "device", "close"] {
            let (window, _dir, mut engine, first) = begin_home_search(cx);
            engine.reply(
                &first,
                serde_json::json!({ "matches": [], "indexing": true }),
            );
            cx.run_until_parked();
            match action {
                "clear" => type_query(window, "", cx),
                "short" => type_query(window, "c", cx),
                "path" => type_query(window, "/mnt/data", cx),
                _ => window
                    .update(cx, |shell, _, cx| match action {
                        "drive" => shell.add_space_goto_location(
                            "Data".into(),
                            Some("/mnt/data".into()),
                            cx,
                        ),
                        "device" => {
                            shell.add_space_back_to(ProjectStep::Devices, cx);
                            shell.add_space_pick_device(device("local"), cx);
                        }
                        "close" => shell.close_add_space(cx),
                        _ => unreachable!(),
                    })
                    .unwrap(),
            }
            cx.executor().advance_clock(Duration::from_secs(10));
            assert!(
                engine.take(methods::SEARCH_HOME_FOLDERS, cx).is_empty(),
                "{action}"
            );
            window
                .read_with(cx, |shell, _| {
                    assert!(
                        !shell
                            .add_space
                            .as_ref()
                            .is_some_and(|flow| flow.home_search.indexing)
                    );
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn new_home_query_resets_backoff_and_ignores_in_flight_answers(cx: &mut TestAppContext) {
        let (window, _dir, mut engine, first) = begin_home_search(cx);
        engine.reply(
            &first,
            serde_json::json!({ "matches": [], "indexing": true }),
        );
        cx.run_until_parked();
        cx.executor().advance_clock(HOME_FOLDER_INDEX_RETRY);
        let stale = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
        assert_eq!(stale.len(), 1);
        type_query(window, "updated", cx);
        let updated = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0]["params"]["query"], "updated");
        engine.reply(
            &updated[0],
            serde_json::json!({ "matches": [], "indexing": true }),
        );
        engine.reply(
            &stale[0],
            serde_json::json!({
                "matches": [found("/home/me/dev/comet", "dev/comet", true)], "indexing": true,
            }),
        );
        cx.run_until_parked();
        assert!(rows(window, cx).is_empty());
        cx.executor()
            .advance_clock(HOME_FOLDER_INDEX_RETRY - Duration::from_millis(1));
        assert!(engine.take(methods::SEARCH_HOME_FOLDERS, cx).is_empty());
        cx.executor().advance_clock(Duration::from_millis(1));
        let retry = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0]["params"], updated[0]["params"]);
        engine.reply(
            &retry[0],
            serde_json::json!({ "matches": [], "indexing": false }),
        );
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(10));
        assert!(engine.take(methods::SEARCH_HOME_FOLDERS, cx).is_empty());
    }

    #[gpui::test]
    fn home_search_stops_retrying_after_errors(cx: &mut TestAppContext) {
        for error in ["failed", "unsupported", "decode"] {
            let (window, _dir, mut engine, first) = begin_home_search(cx);
            engine.reply(
                &first,
                serde_json::json!({ "matches": [], "indexing": true }),
            );
            cx.run_until_parked();
            cx.executor().advance_clock(HOME_FOLDER_INDEX_RETRY);
            let retry = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
            assert_eq!(retry.len(), 1);
            match error {
                "failed" => engine.fail(&retry[0], "search failed"),
                "unsupported" => engine.fail(&retry[0], "unknown method: SearchHomeFolders"),
                "decode" => engine.reply(&retry[0], serde_json::json!({})),
                _ => unreachable!(),
            }
            cx.run_until_parked();
            window
                .read_with(cx, |shell, _| {
                    let search = &shell.add_space.as_ref().unwrap().home_search;
                    assert!(!search.indexing);
                    assert!(!search.loading);
                })
                .unwrap();
            cx.executor().advance_clock(Duration::from_secs(10));
            assert!(
                engine.take(methods::SEARCH_HOME_FOLDERS, cx).is_empty(),
                "{error}"
            );
        }
    }

    #[gpui::test]
    fn home_warms_on_entry_and_lists_matches_below_the_open_folder(cx: &mut TestAppContext) {
        let (window, _dir, mut engine) = picker(cx, "local");
        go_home(window, cx);
        let warm = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
        assert_eq!(warm.len(), 1);
        assert_eq!(warm[0]["params"]["query"], "");
        assert!(warm[0]["params"].get("targetDeviceId").is_none());
        engine.reply(
            &warm[0],
            serde_json::json!({ "matches": [], "indexing": true }),
        );

        type_query(window, "c", cx);
        assert!(engine.take(methods::SEARCH_HOME_FOLDERS, cx).is_empty());
        type_query(window, "co", cx);
        let search = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
        assert_eq!(search[0]["params"]["query"], "co");
        engine.reply(
            &search[0],
            serde_json::json!({
                "matches": [
                    found("/home/me/code", "code", false),
                    found("/home/me/dev/comet", "dev/comet", true),
                ],
                "indexing": false,
            }),
        );
        cx.run_until_parked();
        // The open folder's own `code` is listed once, as a local row.
        let rows = rows(window, cx);
        assert_eq!(rows.len(), 2);
        assert!(matches!(&rows[0], FolderRow::Local(entry) if entry.name == "code"));
        assert!(matches!(&rows[1], FolderRow::Home(m) if m.relative == "dev/comet" && m.is_repo));
    }

    #[gpui::test]
    fn remote_windows_home_matches_navigate_and_reuse_existing_spaces(cx: &mut TestAppContext) {
        for (home, extended_home) in [
            (r"C:\Users\Ana", r"\\?\C:\Users\Ana"),
            (r"\\server\share\Ana", r"\\?\UNC\server\share\Ana"),
        ] {
            let (window, _dir, mut engine) = picker(cx, "remote");
            cx.update(|cx| {
                crate::history::init(
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    cx,
                );
            });
            engine.listing["path"] = home.into();
            go_home(window, cx);
            let warm = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
            engine.reply(
                &warm[0],
                serde_json::json!({ "matches": [], "indexing": false }),
            );
            type_query(window, "co", cx);
            let search = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
            let code = format!("{extended_home}\\code");
            let comet = format!("{code}\\comet");
            engine.reply(
                &search[0],
                serde_json::json!({
                    "matches": [found(&code, "code", false), found(&comet, "code/comet", true)],
                    "indexing": false,
                }),
            );
            cx.run_until_parked();
            // Plain ListFolders and extended SearchHomeFolders paths refer
            // to the same local row; only the nested match is added.
            let listed = rows(window, cx);
            assert_eq!(listed.len(), 2);
            assert!(matches!(&listed[0], FolderRow::Local(entry) if entry.name == "code"));
            window
                .update(cx, |shell, _, cx| {
                    shell.add_space.as_mut().unwrap().active = 1;
                    shell.add_space_open_active(cx);
                })
                .unwrap();
            let browse = engine.take(methods::LIST_FOLDERS, cx);
            assert_eq!(browse[0]["params"]["path"], comet);
            assert_eq!(browse[0]["params"]["targetDeviceId"], "remote");
            engine.reply(
                &browse[0],
                serde_json::json!({
                    "path": comet,
                    "entries": [{ "name": "src", "isDir": true, "isRepo": false }],
                }),
            );
            cx.run_until_parked();
            window
                .update(cx, |shell, _, cx| shell.add_space_open_active(cx))
                .unwrap();
            let child = engine.take(methods::LIST_FOLDERS, cx);
            assert_eq!(child[0]["params"]["path"], format!("{comet}\\src"));
            engine.reply(
                &child[0],
                serde_json::json!({ "path": format!("{comet}\\src"), "entries": [] }),
            );
            cx.run_until_parked();
            window
                .update(cx, |shell, _, cx| shell.add_space_go_up(cx))
                .unwrap();
            let parent = engine.take(methods::LIST_FOLDERS, cx);
            assert_eq!(parent[0]["params"]["path"], comet);
            engine.reply(
                &parent[0],
                serde_json::json!({ "path": comet, "entries": [] }),
            );
            cx.run_until_parked();
            window.update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, _| {
                    state.spaces.push(serde_json::from_value(serde_json::json!({
                        "id": "existing", "deviceId": "remote", "path": format!("{home}\\code\\comet"),
                        "gitDetected": true, "createdAt": "2026-09-20T00:00:00Z",
                    })).unwrap());
                });
                shell.submit_add_space(cx);
                assert!(shell.add_space.is_none());
                assert_eq!(shell.state.read(cx).selected_space.as_deref(), Some("existing"));
                assert_eq!(shell.state.read(cx).spaces.len(), 1);
            }).unwrap();
            assert!(engine.take(methods::MUTATE, cx).is_empty());
        }
    }

    #[gpui::test]
    fn opening_a_home_match_browses_it(cx: &mut TestAppContext) {
        let (window, _dir, mut engine) = picker(cx, "local");
        go_home(window, cx);
        engine.take(methods::SEARCH_HOME_FOLDERS, cx);
        type_query(window, "comet", cx);
        let search = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
        engine.reply(
            &search[0],
            serde_json::json!({
                "matches": [found("/home/me/dev/comet", "dev/comet", true)],
                "indexing": false,
            }),
        );
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| shell.add_space_open_active(cx))
            .unwrap();
        let browse = engine.take(methods::LIST_FOLDERS, cx);
        assert_eq!(browse[0]["params"]["path"], "/home/me/dev/comet");
        window
            .read_with(cx, |shell, _| {
                let flow = shell.add_space.as_ref().unwrap();
                assert!(flow.browser_repo);
            })
            .unwrap();
    }

    #[gpui::test]
    fn closing_or_leaving_home_releases_the_index(cx: &mut TestAppContext) {
        let (window, _dir, mut engine) = picker(cx, "remote");
        go_home(window, cx);
        let warm = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
        assert_eq!(warm[0]["params"]["targetDeviceId"], "remote");
        // A drive location neither searches nor keeps the home index.
        window
            .update(cx, |shell, _, cx| {
                shell.add_space_goto_location("Data".into(), Some("/mnt/data".into()), cx)
            })
            .unwrap();
        let released = engine.take(methods::RELEASE_HOME_FOLDER_SEARCH, cx);
        assert_eq!(released.len(), 1);
        assert_eq!(released[0]["params"]["targetDeviceId"], "remote");
        type_query(window, "comet", cx);
        assert!(engine.take(methods::SEARCH_HOME_FOLDERS, cx).is_empty());

        go_home(window, cx);
        engine.take(methods::SEARCH_HOME_FOLDERS, cx);
        window
            .update(cx, |shell, _, cx| shell.close_add_space(cx))
            .unwrap();
        assert_eq!(
            engine.take(methods::RELEASE_HOME_FOLDER_SEARCH, cx).len(),
            1
        );
        // Nothing left to release.
        window
            .update(cx, |shell, _, cx| shell.close_add_space(cx))
            .unwrap();
        assert!(
            engine
                .take(methods::RELEASE_HOME_FOLDER_SEARCH, cx)
                .is_empty()
        );
    }

    #[gpui::test]
    fn an_older_host_is_not_asked_again(cx: &mut TestAppContext) {
        let (window, _dir, mut engine) = picker(cx, "local");
        go_home(window, cx);
        let warm = engine.take(methods::SEARCH_HOME_FOLDERS, cx);
        engine.fail(&warm[0], "unknown method: SearchHomeFolders");
        cx.run_until_parked();
        type_query(window, "comet", cx);
        assert!(engine.take(methods::SEARCH_HOME_FOLDERS, cx).is_empty());
        window
            .update(cx, |shell, _, cx| shell.close_add_space(cx))
            .unwrap();
        assert!(
            engine
                .take(methods::RELEASE_HOME_FOLDER_SEARCH, cx)
                .is_empty()
        );
    }
}
