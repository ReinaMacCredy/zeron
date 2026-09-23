use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::future::BoxFuture;
use tokio::task::JoinHandle;

const DEBOUNCE: Duration = Duration::from_millis(80);
const THINKING: &str = "Thinking";
const WAITING: [&str; 2] = ["Waiting for you", "Needs approval"];

pub fn cursor_tool_activity(tool: &str) -> &'static str {
    match tool {
        "computer_click" => "Clicking",
        "computer_scroll" => "Scrolling",
        "computer_type_text" => "Typing",
        "computer_set_value" => "Setting field",
        "computer_select_text" => "Selecting text",
        "computer_paste" => "Pasting",
        "computer_press_key" => "Pressing key",
        "computer_drag" => "Dragging",
        "computer_move_cursor" => "Moving cursor",
        "computer_wait" => "Waiting for screen",
        "computer_screenshot" => "Capturing screen",
        "computer_get_state" => "Reading screen",
        "computer_get_screen_size" => "Measuring screen",
        "computer_list_windows" => "Finding window",
        "computer_list_apps" => "Listing apps",
        "computer_set_window_frame" => "Moving window",
        "computer_invoke_menu" => "Opening menu item",
        "computer_verify_state" => "Checking state",
        "computer_zoom" => "Zooming in",
        "computer_get_accessibility_tree" => "Reading desktop inventory",
        "computer_get_cursor_position" => "Reading cursor position",
        "computer_kill_app" => "Force-quitting app",
        "computer_set_window_minimized" => "Changing window visibility",
        "computer_set_app_visibility" => "Changing app visibility",
        "computer_perform_action" => "Activating control",
        "computer_launch_app" => "Opening app",
        "computer_activate_window" => "Activating window",
        "computer_read_clipboard" => "Reading clipboard",
        "computer_write_clipboard" => "Writing clipboard",
        "computer_run" => "Running sequence",
        _ => "Working",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorRuntimeEvent<'a> {
    UserInputRequested,
    RequestOpened,
    UserInputResolved,
    RequestResolved,
    TurnStarted,
    ItemStarted,
    ItemCompleted,
    ContentDelta { stream_kind: &'a str },
    Other,
}

pub fn cursor_runtime_activity(event: CursorRuntimeEvent<'_>) -> Option<&'static str> {
    use CursorRuntimeEvent as Event;
    match event {
        Event::UserInputRequested => Some("Waiting for you"),
        Event::RequestOpened => Some("Needs approval"),
        Event::UserInputResolved | Event::RequestResolved | Event::TurnStarted => Some(THINKING),
        Event::ItemCompleted => Some(THINKING),
        Event::ItemStarted => Some("Working"),
        Event::ContentDelta {
            stream_kind: "assistant_text",
        } => Some("Responding"),
        Event::ContentDelta {
            stream_kind: "reasoning_text" | "reasoning_summary_text" | "plan_text",
        } => Some(THINKING),
        Event::ContentDelta { .. } | Event::Other => None,
    }
}

type Publish = Arc<dyn Fn(Option<String>) -> BoxFuture<'static, ()> + Send + Sync>;

struct Inner {
    owner: Option<String>,
    base: String,
    pending: Vec<(u64, String, String)>,
    next_token: u64,
    timer: Option<JoinHandle<()>>,
    last: Option<Option<String>>,
    disposed: bool,
    #[cfg(test)]
    started: Vec<String>,
}

#[derive(Clone)]
pub struct CursorActivity {
    inner: Arc<Mutex<Inner>>,
    publish: Publish,
}

struct PendingGuard {
    activity: CursorActivity,
    token: u64,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.activity
            .inner()
            .pending
            .retain(|(token, _, _)| *token != self.token);
        self.activity.schedule();
    }
}

impl CursorActivity {
    pub fn new(
        publish: impl Fn(Option<String>) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                owner: None,
                base: THINKING.to_owned(),
                pending: Vec::new(),
                next_token: 0,
                timer: None,
                last: None,
                disposed: false,
                #[cfg(test)]
                started: Vec::new(),
            })),
            publish: Arc::new(publish),
        }
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(crate) fn started(&self) -> Vec<String> {
        self.inner().started.clone()
    }

    pub fn set_owner(&self, owner: Option<&str>) {
        {
            let mut inner = self.inner();
            if inner.owner.as_deref() == owner {
                return;
            }
            inner.owner = owner.map(str::to_owned);
            inner.base = THINKING.to_owned();
        }
        self.schedule();
    }

    pub fn set_runtime(&self, thread: &str, text: &str, resumed: bool) {
        {
            let mut inner = self.inner();
            if inner.owner.as_deref() != Some(thread) || inner.base == text {
                return;
            }
            if !resumed && WAITING.contains(&inner.base.as_str()) {
                return;
            }
            inner.base = text.to_owned();
        }
        self.schedule();
    }

    pub fn during<A, E>(
        &self,
        thread: &str,
        text: &str,
        action: impl Future<Output = Result<A, E>>,
    ) -> impl Future<Output = Result<A, E>> {
        let token = {
            let mut inner = self.inner();
            let token = inner.next_token;
            inner.next_token += 1;
            inner
                .pending
                .push((token, thread.to_owned(), text.to_owned()));
            #[cfg(test)]
            inner.started.push(text.to_owned());
            token
        };
        self.schedule();
        self.set_runtime(thread, THINKING, false);
        let guard = PendingGuard {
            activity: self.clone(),
            token,
        };
        let thread = thread.to_owned();
        async move {
            let result = action.await;
            if result.is_err() {
                guard
                    .activity
                    .set_runtime(&thread, "Needs attention", false);
            }
            drop(guard);
            result
        }
    }

    fn schedule(&self) {
        let mut inner = self.inner();
        if inner.disposed || inner.timer.is_some() {
            return;
        }
        let activity = self.clone();
        inner.timer = Some(tokio::spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;
            activity.fire();
        }));
    }

    fn fire(&self) {
        let text = {
            let mut inner = self.inner();
            inner.timer = None;
            let active = inner
                .pending
                .iter()
                .rev()
                .find(|(_, thread, _)| inner.owner.as_deref() == Some(thread.as_str()))
                .map(|(_, _, text)| text.clone());
            let text = match &inner.owner {
                None => None,
                Some(_) if WAITING.contains(&inner.base.as_str()) => Some(inner.base.clone()),
                Some(_) => Some(active.unwrap_or_else(|| inner.base.clone())),
            };
            if inner.last.as_ref() == Some(&text) || inner.disposed {
                return;
            }
            inner.last = Some(text.clone());
            text
        };
        tokio::spawn((self.publish)(text));
    }

    pub fn dispose(&self) {
        let mut inner = self.inner();
        inner.disposed = true;
        if let Some(timer) = inner.timer.take() {
            timer.abort();
        }
        inner.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use futures::FutureExt;
    use tokio::sync::oneshot;

    use super::*;

    type Labels = Arc<Mutex<Vec<Option<String>>>>;

    fn recording(hang: bool) -> (Labels, CursorActivity) {
        let labels: Labels = Arc::default();
        let sink = labels.clone();
        let activity = CursorActivity::new(move |text| {
            sink.lock().unwrap().push(text);
            if hang {
                futures::future::pending().boxed()
            } else {
                async {}.boxed()
            }
        });
        (labels, activity)
    }

    fn labels(labels: &Labels) -> Vec<Option<String>> {
        labels.lock().unwrap().clone()
    }

    fn some(text: &str) -> Option<String> {
        Some(text.to_owned())
    }

    async fn tick() {
        tokio::time::sleep(DEBOUNCE + Duration::from_millis(1)).await;
    }

    fn gated() -> (oneshot::Sender<()>, impl Future<Output = Result<(), ()>>) {
        let (finish, finished) = oneshot::channel::<()>();
        (finish, async move {
            let _ = finished.await;
            Ok(())
        })
    }

    #[tokio::test(start_paused = true)]
    async fn shows_live_work_then_thinking_without_waiting_on_the_badge_backend() {
        let (published, activity) = recording(true);
        activity.set_owner(Some("owner"));
        let (finish, action) = gated();
        let work = activity.during("owner", "Scrolling", action);
        tick().await;
        assert_eq!(labels(&published), [some("Scrolling")]);
        finish.send(()).unwrap();
        work.await.unwrap();
        tick().await;
        assert_eq!(labels(&published), [some("Scrolling"), some("Thinking")]);
        activity.set_owner(None);
        tick().await;
        assert_eq!(labels(&published).last(), Some(&None));
        activity.dispose();
    }

    #[tokio::test(start_paused = true)]
    async fn coalesces_brief_calls_and_repeated_token_events() {
        let (published, activity) = recording(false);
        activity.set_owner(Some("owner"));
        activity
            .during("owner", "Clicking", async { Ok::<_, ()>(()) })
            .await
            .unwrap();
        for _ in 0..100 {
            activity.set_runtime("owner", "Thinking", false);
        }
        tick().await;
        assert_eq!(labels(&published), [some("Thinking")]);
        activity.dispose();
    }

    #[tokio::test(start_paused = true)]
    async fn ignores_other_threads_and_keeps_pending_work_from_overwriting_a_new_owner() {
        let (published, activity) = recording(false);
        activity.set_owner(Some("a"));
        let (finish, action) = gated();
        let work = activity.during("a", "Typing", action);
        activity.set_owner(Some("b"));
        activity.set_runtime("a", "Waiting for you", false);
        activity.set_runtime("b", "Needs approval", false);
        tick().await;
        finish.send(()).unwrap();
        work.await.unwrap();
        tick().await;
        assert_eq!(labels(&published), [some("Needs approval")]);
        activity.dispose();
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_a_pending_question_visible_until_its_response_arrives() {
        let (published, activity) = recording(false);
        activity.set_owner(Some("owner"));
        activity.set_runtime("owner", "Waiting for you", false);
        activity.set_runtime("owner", "Responding", false);
        activity.set_runtime("owner", "Thinking", false);
        tick().await;
        assert_eq!(labels(&published), [some("Waiting for you")]);
        activity.set_runtime("owner", "Thinking", true);
        tick().await;
        assert_eq!(labels(&published).last(), Some(&some("Thinking")));
        activity.dispose();
    }

    #[tokio::test(start_paused = true)]
    async fn handles_errors_and_disposal_without_failing_input_or_publishing_late_labels() {
        let (published, activity) = recording(false);
        activity.set_owner(Some("a"));
        let failed = activity
            .during("a", "Typing", async { Err::<(), _>("input failed") })
            .await;
        assert_eq!(failed, Err("input failed"));
        tick().await;
        assert_eq!(labels(&published), [some("Needs attention")]);
        activity.set_runtime("a", "Thinking", false);
        activity.dispose();
        tick().await;
        assert_eq!(labels(&published).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancelled_action_releases_its_pending_label() {
        let (published, activity) = recording(false);
        activity.set_owner(Some("owner"));
        let work = activity.during(
            "owner",
            "Dragging",
            futures::future::pending::<Result<(), ()>>(),
        );
        drop(work);
        tick().await;
        assert_eq!(labels(&published), [some("Thinking")]);
        activity.dispose();
    }

    #[test]
    fn uses_concise_labels_for_actual_events_without_showing_private_content() {
        assert!(cursor_tool_activity("computer_scroll").len() <= 20);
        assert_eq!(
            cursor_runtime_activity(CursorRuntimeEvent::UserInputRequested),
            Some("Waiting for you")
        );
        assert_eq!(
            cursor_runtime_activity(CursorRuntimeEvent::RequestOpened),
            Some("Needs approval")
        );
        assert_eq!(
            cursor_runtime_activity(CursorRuntimeEvent::ContentDelta {
                stream_kind: "reasoning_text"
            }),
            Some("Thinking")
        );
        assert_eq!(cursor_runtime_activity(CursorRuntimeEvent::Other), None);
    }

    #[test]
    fn labels_the_registered_cursor_tool_and_never_echoes_unknown_tool_names() {
        for (tool, label) in [
            ("computer_screenshot", "Capturing screen"),
            ("computer_get_state", "Reading screen"),
            ("computer_get_screen_size", "Measuring screen"),
            ("computer_list_windows", "Finding window"),
            ("computer_click", "Clicking"),
            ("computer_move_cursor", "Moving cursor"),
            ("computer_drag", "Dragging"),
            ("computer_scroll", "Scrolling"),
            ("computer_type_text", "Typing"),
            ("computer_press_key", "Pressing key"),
            ("computer_set_value", "Setting field"),
            ("computer_select_text", "Selecting text"),
            ("computer_perform_action", "Activating control"),
            ("computer_launch_app", "Opening app"),
            ("computer_activate_window", "Activating window"),
            ("computer_wait", "Waiting for screen"),
            ("computer_read_clipboard", "Reading clipboard"),
            ("computer_write_clipboard", "Writing clipboard"),
            ("computer_paste", "Pasting"),
            ("computer_run", "Running sequence"),
        ] {
            assert_eq!(cursor_tool_activity(tool), label);
            assert!(label.len() <= 20);
        }
        assert_eq!(cursor_tool_activity("computer_move"), "Working");
        assert_eq!(
            cursor_tool_activity("unknown tool containing private text"),
            "Working"
        );
    }
}
