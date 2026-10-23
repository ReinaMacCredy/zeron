use gpui::motion_clock;
use serde_json::{Value, json};

pub(super) fn json() -> Value {
    let paused = motion_clock::paused();
    json!({
        "paused": paused.is_some(),
        "virtualMs": paused.map_or(0, |advanced| advanced.as_millis() as u64),
    })
}

#[cfg(debug_assertions)]
pub(super) use drive::{Clock, run};

#[cfg(debug_assertions)]
mod drive {
    use super::super::{fresh_frame, target_window};
    use gpui::{AsyncApp, motion_clock};
    use serde::Deserialize;
    use serde_json::{Value, json};
    use std::time::Duration;
    use zeron_proto::ui_driver::MAX_WAIT_MS;

    #[derive(Debug, PartialEq, Deserialize)]
    #[serde(tag = "do", rename_all = "camelCase")]
    pub(in super::super) enum Clock {
        Pause,
        Resume,
        Advance { ms: u64 },
    }

    impl Clock {
        pub(in super::super) fn check(&self) -> Result<(), String> {
            match self {
                Clock::Advance { ms } if *ms > MAX_WAIT_MS => {
                    Err(format!("`ms` must be at most {MAX_WAIT_MS} ms, not {ms}"))
                }
                _ => Ok(()),
            }
        }
    }

    pub(in super::super) async fn run(
        window: Option<u64>,
        clock: Clock,
        cx: &mut AsyncApp,
    ) -> Result<Value, String> {
        let handle = cx.update(|cx| target_window(window, cx))?;
        match clock {
            Clock::Pause => motion_clock::pause(),
            Clock::Resume => motion_clock::resume(),
            Clock::Advance { ms } => {
                motion_clock::advance(Duration::from_millis(ms))
                    .ok_or("the clock is running; send `clock` with `do: pause` first")?;
            }
        }
        // A paused animation stops requesting frames, so every window must redraw to pick up the new instant.
        cx.update(|cx| {
            for window in cx.windows() {
                let _ = window.update(cx, |_, window, _| window.refresh());
            }
        });
        let drawn = fresh_frame(handle, Duration::from_millis(500), cx).await?;
        let mut reply = super::json();
        reply["frame"] = json!(super::super::frame_count());
        if drawn.is_none() {
            reply["stale"] = json!(true);
        }
        Ok(reply)
    }
}
