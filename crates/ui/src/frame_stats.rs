use std::time::Duration;

use gpui::App;

/// `ZERON_FRAME_STATS=1` turns on the smoothness measurement logs at `warn`
/// level: whole-window draw cost (layout, prepaint and paint) every second,
/// plus the transcript's live-row render cost and view cadence. Off by
/// default; zero cost when off.
pub(crate) fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var("ZERON_FRAME_STATS").is_ok_and(|v| !v.is_empty() && v != "0"))
}

pub(crate) fn init(cx: &mut App) {
    if !enabled() {
        return;
    }
    gpui::set_frame_trace_enabled(true);
    cx.spawn(async move |cx| {
        let mut collector = gpui::FrameTimingCollector::new();
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            report(&collector.collect_unseen());
        }
    })
    .detach();
}

fn report(frames: &[gpui::FrameTiming]) {
    let micros = |duration: Duration| duration.as_micros() as u64;
    let mut draw: Vec<u64> = frames
        .iter()
        .map(|frame| micros(frame.draw_duration()))
        .collect();
    if draw.is_empty() {
        return;
    }
    let mut latency: Vec<u64> = frames
        .iter()
        .filter_map(|frame| frame.dirty_to_draw_duration().map(micros))
        .collect();
    draw.sort_unstable();
    latency.sort_unstable();
    tracing::warn!(
        frames = draw.len(),
        draw_p50_us = percentile(&draw, 50),
        draw_p95_us = percentile(&draw, 95),
        draw_max_us = percentile(&draw, 100),
        dirty_to_draw_p95_us = percentile(&latency, 95),
        "window frame cost"
    );
}

fn percentile(sorted: &[u64], p: usize) -> u64 {
    sorted
        .get(sorted.len().saturating_sub(1) * p / 100)
        .copied()
        .unwrap_or(0)
}
