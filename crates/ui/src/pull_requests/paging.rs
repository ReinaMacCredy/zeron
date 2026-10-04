use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{AnyElement, Context, Window, div, prelude::*, px};
use zeron_proto::{ChangeRequestDetail, ChangeRequestListing};
use zeron_rpc::methods;

use super::{PullRequestKey, PullRequestsPage};
use crate::popover::Loadable;
use crate::theme::Theme;

pub(super) const PAGE_SIZE: u32 = 99;
pub(super) const MAX_PAGE_SIZE: u32 = 500;
const LIST_FILE: &str = "pull-requests-list.json";
const DETAILS_FILE: &str = "pull-requests-details.json";
const DETAIL_SNAPSHOTS: usize = 30;
const FOCUS_MIN_INTERVAL: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const IDLE_AFTER: Duration = Duration::from_secs(6 * 60);
const POLL_CHECK: Duration = Duration::from_secs(30);

pub(super) fn next_page(limit: u32) -> u32 {
    (limit + PAGE_SIZE).min(MAX_PAGE_SIZE)
}

pub(super) fn refresh_on_focus(last_read: Instant, now: Instant) -> bool {
    now.duration_since(last_read) >= FOCUS_MIN_INTERVAL
}

pub(super) fn refresh_on_poll(last_read: Instant, last_interaction: Instant, now: Instant) -> bool {
    now.duration_since(last_read) >= POLL_INTERVAL
        && now.duration_since(last_interaction) < IDLE_AFTER
}

pub(super) fn save_list(dir: &Path, listing: &ChangeRequestListing) {
    let mut snapshot = listing.clone();
    snapshot.errors.clear();
    snapshot.truncated = false;
    if let Ok(bytes) = serde_json::to_vec(&snapshot) {
        let _ = std::fs::write(dir.join(LIST_FILE), bytes);
    }
}

pub(super) fn load_list(dir: &Path) -> Option<ChangeRequestListing> {
    serde_json::from_slice(&std::fs::read(dir.join(LIST_FILE)).ok()?).ok()
}

fn detail_key(key: &PullRequestKey) -> String {
    format!("{} {}#{}", key.host, key.repository, key.number)
}

fn load_details(dir: &Path) -> Vec<(String, ChangeRequestDetail)> {
    std::fs::read(dir.join(DETAILS_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub(super) fn save_detail(dir: &Path, key: &PullRequestKey, detail: &ChangeRequestDetail) {
    let key = detail_key(key);
    let mut details = load_details(dir);
    details.retain(|(known, _)| *known != key);
    details.push((key, detail.clone()));
    let overflow = details.len().saturating_sub(DETAIL_SNAPSHOTS);
    details.drain(..overflow);
    if let Ok(bytes) = serde_json::to_vec(&details) {
        let _ = std::fs::write(dir.join(DETAILS_FILE), bytes);
    }
}

pub(super) fn load_detail(dir: &Path, key: &PullRequestKey) -> Option<ChangeRequestDetail> {
    let key = detail_key(key);
    load_details(dir)
        .into_iter()
        .find(|(known, _)| *known == key)
        .map(|(_, detail)| detail)
}

pub(super) fn snapshot_dir(cx: &gpui::App) -> Option<PathBuf> {
    crate::settings::data_dir(cx)
}

impl PullRequestsPage {
    pub(super) fn start_live_refresh(&mut self, cx: &mut Context<Self>) {
        self.poll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_CHECK).await;
                let Ok(()) = this.update(cx, |page, cx| {
                    if page.visible
                        && refresh_on_poll(page.last_read, page.last_interaction, Instant::now())
                    {
                        page.live_refresh(cx);
                    }
                }) else {
                    break;
                };
            }
        }));
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Focus and poll reads go through the engine's cache; only the refresh button busts it.
    pub(super) fn live_refresh(&mut self, cx: &mut Context<Self>) {
        if !self.refreshing {
            self.refresh(cx);
        }
        if let Some(key) = self.active.clone() {
            self.load_detail(&key, cx);
        }
    }

    pub(super) fn note_focus(&mut self, window: &Window, cx: &mut Context<Self>) {
        let active = window.is_window_active();
        if active && !self.window_active && refresh_on_focus(self.last_read, Instant::now()) {
            self.live_refresh(cx);
        }
        self.window_active = active;
    }

    pub(super) fn hard_refresh(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.refreshing = true;
        let targets: Vec<Option<String>> = self
            .list_devices(cx)
            .iter()
            .map(|device| device.target())
            .collect();
        cx.spawn(async move |this, cx| {
            futures::future::join_all(targets.into_iter().map(|target| {
                let engine = engine.clone();
                async move {
                    let _ = engine
                        .client()
                        .call(
                            methods::INVALIDATE_CHANGE_REQUESTS,
                            super::routed(serde_json::json!({}), target.as_deref()),
                        )
                        .await;
                }
            }))
            .await;
            this.update(cx, |page, cx| {
                page.refreshing = false;
                page.refresh(cx);
                if let Some(key) = page.active.clone() {
                    page.load_detail(&key, cx);
                    page.load_activity(&key, cx);
                    page.load_permissions(&key, cx);
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn load_more(&mut self, cx: &mut Context<Self>) {
        self.limit = next_page(self.limit);
        self.refresh(cx);
    }

    pub(super) fn render_more(
        &self,
        truncated: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !truncated {
            return None;
        }
        let footer = div()
            .flex()
            .justify_center()
            .py(px(12.0))
            .text_size(px(12.0))
            .text_color(theme.text_muted);
        Some(if self.limit < MAX_PAGE_SIZE {
            footer
                .child(
                    super::actions::button(
                        "pull-requests-load-more",
                        if self.refreshing {
                            "Loading..."
                        } else {
                            "Load more"
                        },
                        super::actions::Look::Outline,
                        theme,
                    )
                    .when(!self.refreshing, |el| {
                        el.on_click(cx.listener(|page, _, _, cx| page.load_more(cx)))
                    }),
                )
                .into_any_element()
        } else {
            footer
                .child("Narrow your search to find more pull requests.")
                .into_any_element()
        })
    }

    pub(super) fn hydrate_detail(&mut self, key: &PullRequestKey, cx: &mut Context<Self>) {
        let Some(dir) = snapshot_dir(cx) else {
            return;
        };
        let Some(detail) = load_detail(&dir, key) else {
            return;
        };
        if let Some(state) = self.details.get_mut(key)
            && matches!(state.detail, Loadable::Idle | Loadable::Loading)
        {
            state.set_detail(detail);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_grow_by_99_to_500() {
        assert_eq!(next_page(PAGE_SIZE), 198);
        assert_eq!(next_page(495), MAX_PAGE_SIZE);
        assert_eq!(next_page(MAX_PAGE_SIZE), MAX_PAGE_SIZE);
    }

    #[test]
    fn live_refresh_waits_ten_seconds_on_focus_and_polls_only_while_used() {
        let start = Instant::now();
        assert!(!refresh_on_focus(start, start + Duration::from_secs(9)));
        assert!(refresh_on_focus(start, start + Duration::from_secs(10)));
        let later = start + Duration::from_secs(5 * 60);
        assert!(refresh_on_poll(
            start,
            later - Duration::from_secs(60),
            later
        ));
        assert!(!refresh_on_poll(
            start,
            start,
            later + Duration::from_secs(60 * 2)
        ));
        assert!(!refresh_on_poll(
            later,
            later,
            later + Duration::from_secs(60)
        ));
    }

    #[test]
    fn snapshots_round_trip_and_keep_the_newest_thirty() {
        let dir = std::env::temp_dir().join(format!("zeron-pr-snapshots-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let listing = ChangeRequestListing {
            truncated: true,
            ..Default::default()
        };
        save_list(&dir, &listing);
        assert_eq!(
            load_list(&dir).map(|listing| listing.truncated),
            Some(false)
        );
        let detail: ChangeRequestDetail = serde_json::from_value(serde_json::json!({
            "item": {
                "provider": "github", "host": "github.com", "repository": "acme/zeron", "number": 1,
                "title": "t", "url": "u", "headRef": "h", "baseRef": "b", "state": "open",
                "isDraft": false, "mergeability": "unknown", "additions": 0, "deletions": 0,
                "createdAt": "2026-10-01T00:00:00Z", "updatedAt": "2026-10-01T00:00:00Z",
                "viewerReviewRequested": false, "labels": []
            },
            "body": "", "changedFiles": 0, "isCrossRepository": false, "reviewers": [],
            "checks": [], "comments": [], "commentCount": 0, "commitCount": 0
        }))
        .unwrap();
        let key = |number| PullRequestKey {
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            number,
            device: None,
        };
        for number in 0..35 {
            save_detail(&dir, &key(number), &detail);
        }
        assert!(load_detail(&dir, &key(4)).is_none());
        assert!(load_detail(&dir, &key(5)).is_some());
        assert!(load_detail(&dir, &key(34)).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }
}
