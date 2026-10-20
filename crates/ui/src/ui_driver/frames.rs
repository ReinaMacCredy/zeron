use super::selector::Element;
use super::tree::{self, Bounds};
use super::{Cancel, Drive, Windowed, epoch, frame_count, target_window};
use gpui::{AnyWindowHandle, AsyncApp};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const FRAMES_MS: u64 = 1000;
const FRAMES_MAX_MS: u64 = 10_000;
const TRACK_MS: u64 = 800;
const TRACK_MAX_MS: u64 = 5000;
const JUMP_PX: f64 = 40.0;
const LONG_CAP: usize = 20;
const JUMP_CAP: usize = 20;
const POLL: Duration = Duration::from_millis(2);

pub(super) static LAST_US: AtomicU64 = AtomicU64::new(0);
static STAMPS: Mutex<Stamps> = Mutex::new(Stamps {
    recorders: 0,
    times: Vec::new(),
});
static TRACE: Mutex<Trace> = Mutex::new(Trace {
    users: 0,
    owned: false,
});

struct Stamps {
    recorders: usize,
    times: Vec<u64>,
}

struct Trace {
    users: usize,
    owned: bool,
}

pub(super) fn stamp(at_us: u64) {
    LAST_US.store(at_us, Ordering::Relaxed);
    let mut stamps = STAMPS.lock().unwrap_or_else(|poison| poison.into_inner());
    if stamps.recorders > 0 {
        stamps.times.push(at_us);
    }
}

fn now_us() -> u64 {
    epoch().elapsed().as_micros() as u64
}

struct Recorder {
    start: usize,
}

impl Recorder {
    fn start() -> Self {
        let mut stamps = STAMPS.lock().unwrap_or_else(|poison| poison.into_inner());
        if stamps.recorders == 0 {
            stamps.times.clear();
        }
        stamps.recorders += 1;
        Self {
            start: stamps.times.len(),
        }
    }

    fn times(&self) -> Vec<u64> {
        let stamps = STAMPS.lock().unwrap_or_else(|poison| poison.into_inner());
        stamps.times[self.start.min(stamps.times.len())..].to_vec()
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let mut stamps = STAMPS.lock().unwrap_or_else(|poison| poison.into_inner());
        stamps.recorders -= 1;
    }
}

struct DrawTimes {
    collector: gpui::FrameTimingCollector,
}

impl DrawTimes {
    fn start() -> Self {
        let mut trace = TRACE.lock().unwrap_or_else(|poison| poison.into_inner());
        if trace.users == 0 {
            // Only turn tracing off later if we turned it on: disabling clears ZERON_FRAME_STATS' ring.
            trace.owned = gpui::set_frame_trace_enabled(true);
        }
        trace.users += 1;
        Self {
            collector: gpui::FrameTimingCollector::new(),
        }
    }

    fn take(&mut self, window: gpui::WindowId) -> Vec<f64> {
        self.collector
            .collect_unseen()
            .iter()
            .filter(|timing| timing.window_id == window)
            .map(|timing| timing.draw_duration().as_secs_f64() * 1000.0)
            .collect()
    }
}

impl Drop for DrawTimes {
    fn drop(&mut self) {
        let mut trace = TRACE.lock().unwrap_or_else(|poison| poison.into_inner());
        trace.users -= 1;
        if trace.users == 0 && trace.owned {
            gpui::set_frame_trace_enabled(false);
            trace.owned = false;
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Frames {
    #[serde(default = "frames_ms")]
    ms: u64,
    during: Option<Windowed<Drive>>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Track {
    #[serde(flatten)]
    element: Element,
    #[serde(default = "track_ms")]
    ms: u64,
    #[serde(default = "jump_px")]
    jump_px: f64,
    during: Option<Windowed<Drive>>,
}

fn frames_ms() -> u64 {
    FRAMES_MS
}

fn track_ms() -> u64 {
    TRACK_MS
}

fn jump_px() -> f64 {
    JUMP_PX
}

type During = Option<gpui::Task<Result<Value, String>>>;

fn start_during(
    window: Option<u64>,
    during: Option<Windowed<Drive>>,
    cancel: Cancel,
    cx: &mut AsyncApp,
) -> During {
    let Windowed {
        window: own,
        action,
    } = during?;
    let window = own.or(window);
    Some(cx.spawn(async move |cx| super::drive(window, action, cancel, cx).await))
}

async fn finish_during(during: During, reply: &mut Value) -> Result<(), String> {
    if let Some(task) = during {
        reply["during"] = task
            .await
            .map_err(|error| format!("`during` failed: {error}"))?;
    }
    Ok(())
}

fn check_ms(ms: u64, max: u64) -> Result<Duration, String> {
    if ms == 0 || ms > max {
        return Err(format!("`ms` must be 1..={max}, not {ms}"));
    }
    Ok(Duration::from_millis(ms))
}

pub(super) async fn frames(
    window: Option<u64>,
    request: Frames,
    cancel: Cancel,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let span = check_ms(request.ms, FRAMES_MAX_MS)?;
    let handle = cx.update(|cx| target_window(window, cx))?;
    let recorder = Recorder::start();
    let mut draws = DrawTimes::start();
    let t0 = now_us();
    let during = start_during(window, request.during, cancel, cx);
    cx.background_executor().timer(span).await;
    let times = recorder.times();
    let draw = draws.take(handle.window_id());
    drop((recorder, draws));
    let mut reply = frame_report(&times, t0, request.ms, &draw);
    finish_during(during, &mut reply).await?;
    Ok(reply)
}

pub(super) async fn track(
    window: Option<u64>,
    request: Track,
    cancel: Cancel,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let span = check_ms(request.ms, TRACK_MAX_MS)?;
    request.element.selector()?;
    let handle = cx.update(|cx| target_window(window, cx))?;
    let first = tree::read(handle, cx).await?;
    let start = if request.during.is_some() {
        request.element.find_or_absent(&first)?
    } else {
        Some(request.element.find(&first)?)
    };
    let start_ref = start.map(|node| node.id.clone());
    let start_line = start.map(tree::line);
    let (t0, mark, started) = (now_us(), frame_count(), Instant::now());
    let during = start_during(window, request.during, cancel, cx);
    let mut sampler = Sampler::new(start_ref.clone(), first.frame);
    let mut read_ms = Vec::new();
    let mut seen = mark;
    let sampled = loop {
        match next_tree(handle, &mut seen, started + span, cx).await {
            Ok(Some((tree, cost))) => {
                let at_ms = LAST_US.load(Ordering::Relaxed).saturating_sub(t0) as f64 / 1000.0;
                read_ms.push(cost);
                sampler.add(&tree, &request.element, at_ms);
            }
            Ok(None) => break Ok(()),
            Err(error) => break Err(error),
        }
    };
    if let Err(error) = sampled {
        // Dropping the task would cancel a drag between its press and release.
        if let Some(task) = during {
            let _ = task.await;
        }
        return Err(error);
    }
    let mut reply = sampler.report(request.jump_px);
    reply["ref"] = json!(start_ref.or_else(|| sampler.last_ref.clone()));
    reply["node"] = json!(start_line);
    reply["framesDrawn"] = json!(frame_count() - mark);
    read_ms.sort_by(f64::total_cmp);
    reply["readMs"] = stats(&read_ms);
    finish_during(during, &mut reply).await?;
    Ok(reply)
}

pub(super) async fn next_tree(
    handle: AnyWindowHandle,
    seen: &mut u64,
    until: Instant,
    cx: &mut AsyncApp,
) -> Result<Option<(tree::Tree, f64)>, String> {
    loop {
        if Instant::now() >= until {
            return Ok(None);
        }
        if frame_count() != *seen {
            break;
        }
        cx.background_executor().timer(POLL).await;
    }
    *seen = frame_count();
    let read_at = Instant::now();
    let tree = tree::read(handle, cx).await?;
    Ok(Some((tree, read_at.elapsed().as_secs_f64() * 1000.0)))
}

struct Sampler {
    last_frame: u64,
    last_ref: Option<String>,
    reresolved: usize,
    samples: Vec<Sample>,
}

impl Sampler {
    fn new(start_ref: Option<String>, frame: u64) -> Self {
        Self {
            last_frame: frame,
            last_ref: start_ref,
            reresolved: 0,
            samples: Vec::new(),
        }
    }

    fn add(&mut self, tree: &tree::Tree, element: &Element, at_ms: f64) {
        if tree.frame <= self.last_frame {
            return;
        }
        let frames = tree.frame - self.last_frame;
        self.last_frame = tree.frame;
        let bounds = element.find(tree).ok().and_then(|node| {
            if self.last_ref.as_ref() != Some(&node.id) {
                self.reresolved += usize::from(self.last_ref.is_some());
                self.last_ref = Some(node.id.clone());
            }
            node.bounds
        });
        self.samples.push(Sample {
            at_ms,
            frames,
            bounds,
        });
    }

    fn report(&self, jump_px: f64) -> Value {
        json!({
            "samples": collapse(&self.samples),
            "framesSampled": self.samples.len(),
            "missingFrames": self.samples.iter().filter(|s| s.bounds.is_none()).count(),
            "jumps": jumps(&self.samples, jump_px),
            "settledAtMs": settled_at(&self.samples).map(round1),
            "reresolved": self.reresolved,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Sample {
    at_ms: f64,
    frames: u64,
    bounds: Option<Bounds>,
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn percentile(sorted: &[f64], p: usize) -> f64 {
    sorted
        .get(sorted.len().saturating_sub(1) * p / 100)
        .copied()
        .unwrap_or(0.0)
}

fn stats(sorted: &[f64]) -> Value {
    if sorted.is_empty() {
        return Value::Null;
    }
    json!({
        "p50": round1(percentile(sorted, 50)),
        "p95": round1(percentile(sorted, 95)),
        "max": round1(percentile(sorted, 100)),
    })
}

fn frame_report(times_us: &[u64], t0_us: u64, ms: u64, draw_ms: &[f64]) -> Value {
    let at = |us: u64| round1(us.saturating_sub(t0_us) as f64 / 1000.0);
    let gaps: Vec<f64> = times_us
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]) as f64 / 1000.0)
        .collect();
    let mut sorted = gaps.clone();
    sorted.sort_by(f64::total_cmp);
    let median = percentile(&sorted, 50);
    let long: Vec<Value> = if gaps.len() >= 3 {
        gaps.iter()
            .enumerate()
            .filter(|&(_, &gap)| gap > 2.0 * median)
            .take(LONG_CAP)
            .map(|(i, &gap)| json!({ "atMs": at(times_us[i]), "gapMs": round1(gap) }))
            .collect()
    } else {
        Vec::new()
    };
    let mut draw = draw_ms.to_vec();
    draw.sort_by(f64::total_cmp);
    let mut reply = json!({
        "frames": times_us.len(),
        "fps": round1(times_us.len() as f64 * 1000.0 / ms as f64),
        "intervalsMs": stats(&sorted),
        "longFrames": long,
        "firstFrameAtMs": times_us.first().map(|&us| at(us)),
        "lastFrameAtMs": times_us.last().map(|&us| at(us)),
    });
    if !draw.is_empty() {
        reply["drawMs"] = stats(&draw);
    }
    reply
}

fn rounded(bounds: &Bounds) -> [i64; 4] {
    [bounds.x, bounds.y, bounds.w, bounds.h].map(|v| v.round() as i64)
}

fn collapse(samples: &[Sample]) -> Vec<Vec<i64>> {
    let present: Vec<(f64, [i64; 4])> = samples
        .iter()
        .filter_map(|s| s.bounds.map(|b| (s.at_ms, rounded(&b))))
        .collect();
    let row = |(at, b): (f64, [i64; 4])| vec![at.round() as i64, b[0], b[1], b[2], b[3]];
    present
        .iter()
        .enumerate()
        .filter(|&(i, (_, b))| i == 0 || i + 1 == present.len() || present[i - 1].1 != *b)
        .map(|(_, &sample)| row(sample))
        .collect()
}

pub(super) fn delta(from: &Bounds, to: &Bounds) -> f64 {
    [to.x - from.x, to.y - from.y, to.w - from.w, to.h - from.h]
        .into_iter()
        .map(|d| f64::from(d.abs()))
        .fold(0.0, f64::max)
}

fn jumps(samples: &[Sample], jump_px: f64) -> Vec<Value> {
    let steps: Vec<(f64, Bounds, Bounds, f64)> = samples
        .windows(2)
        .filter_map(|pair| {
            let (from, to) = (pair[0].bounds?, pair[1].bounds?);
            let per_frame = delta(&from, &to) / pair[1].frames.max(1) as f64;
            Some((pair[1].at_ms, from, to, per_frame))
        })
        .collect();
    let mut moving: Vec<f64> = steps
        .iter()
        .map(|step| step.3)
        .filter(|&d| d > 0.0)
        .collect();
    moving.sort_by(f64::total_cmp);
    let median = (moving.len() >= 3).then(|| percentile(&moving, 50));
    let as_row = |b: &Bounds| json!(rounded(b));
    let neighbor = |i: usize| {
        let before = i.checked_sub(1).map_or(0.0, |j| steps[j].3);
        let after = steps.get(i + 1).map_or(0.0, |step| step.3);
        before.max(after)
    };
    // A fast ease-out start beats 4x the median; a snap also beats 4x its neighbors.
    steps
        .iter()
        .enumerate()
        .filter(|&(i, (_, _, _, d))| {
            *d > jump_px || median.is_some_and(|median| *d > 4.0 * median && *d > 4.0 * neighbor(i))
        })
        .take(JUMP_CAP)
        .map(|(_, (at, from, to, d))| {
            json!({
                "atMs": round1(*at),
                "from": as_row(from),
                "to": as_row(to),
                "delta": round1(*d),
            })
        })
        .collect()
}

fn settled_at(samples: &[Sample]) -> Option<f64> {
    let state = |s: &Sample| s.bounds.as_ref().map(rounded);
    let last = state(samples.last()?);
    let run = samples
        .iter()
        .rev()
        .take_while(|s| state(s) == last)
        .count();
    Some(samples[samples.len() - run].at_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(at_ms: f64, x: f32) -> Sample {
        Sample {
            at_ms,
            frames: 1,
            bounds: Some(Bounds {
                x,
                y: 100.0,
                w: 200.0,
                h: 30.0,
            }),
        }
    }

    fn eased(n: usize, distance: f32) -> Vec<Sample> {
        (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let ease = 1.0 - (1.0 - t).powi(3);
                at(i as f64 * 8.3, 300.0 - distance * ease)
            })
            .collect()
    }

    #[test]
    fn interval_stats_and_long_frames() {
        let times: Vec<u64> = [0, 8_000, 16_000, 24_000, 60_000, 68_000, 76_000]
            .map(|us| 1_000_000 + us)
            .to_vec();
        let report = frame_report(&times, 1_000_000, 1000, &[]);
        assert_eq!(report["frames"], 7);
        assert_eq!(report["fps"], 7.0);
        assert_eq!(
            report["intervalsMs"],
            json!({ "p50": 8.0, "p95": 8.0, "max": 36.0 })
        );
        assert_eq!(
            report["longFrames"],
            json!([{ "atMs": 24.0, "gapMs": 36.0 }])
        );
        assert_eq!(report["firstFrameAtMs"], 0.0);
        assert_eq!(report["lastFrameAtMs"], 76.0);
        assert!(report.get("drawMs").is_none());
    }

    #[test]
    fn too_few_intervals_flag_no_long_frame_and_draw_stats_appear() {
        let report = frame_report(&[0, 5_000, 50_000], 0, 500, &[3.0, 1.0, 2.0]);
        assert_eq!(report["longFrames"], json!([]));
        assert_eq!(
            report["drawMs"],
            json!({ "p50": 2.0, "p95": 2.0, "max": 3.0 })
        );
        let empty = frame_report(&[], 0, 1000, &[]);
        assert_eq!(empty["frames"], 0);
        assert_eq!(empty["intervalsMs"], Value::Null);
    }

    #[test]
    fn an_eased_slide_has_no_jump_but_a_snap_does() {
        let smooth = eased(24, 240.0);
        assert_eq!(jumps(&smooth, 40.0), Vec::<Value>::new());

        let mut snapped = eased(24, 240.0);
        for sample in &mut snapped[12..] {
            sample.bounds.as_mut().unwrap().x -= 30.0;
        }
        let found = jumps(&snapped, 40.0);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0]["atMs"], round1(12.0 * 8.3));

        let teleport = vec![at(0.0, 0.0), at(8.0, 0.0), at(16.0, 100.0), at(24.0, 100.0)];
        let found = jumps(&teleport, 40.0);
        assert_eq!(
            found,
            vec![json!({
                "atMs": 16.0, "from": [0, 100, 200, 30], "to": [100, 100, 200, 30], "delta": 100.0,
            })]
        );
        assert_eq!(jumps(&teleport, 150.0), Vec::<Value>::new());
    }

    #[test]
    fn a_missed_frame_spreads_the_delta() {
        let mut two_frames = vec![at(0.0, 0.0), at(16.0, 60.0)];
        two_frames[1].frames = 2;
        assert_eq!(jumps(&two_frames, 40.0), Vec::<Value>::new());
        two_frames[1].frames = 1;
        assert_eq!(jumps(&two_frames, 40.0).len(), 1);
    }

    #[test]
    fn settled_at_is_the_start_of_the_final_still_run() {
        let samples = vec![
            at(0.0, 10.0),
            at(8.0, 20.0),
            at(16.0, 30.0),
            at(24.0, 30.2),
            at(32.0, 30.0),
        ];
        assert_eq!(settled_at(&samples), Some(16.0));
        assert_eq!(settled_at(&[at(5.0, 1.0), at(9.0, 1.0)]), Some(5.0));
        let mut vanished = samples.clone();
        vanished.push(Sample {
            at_ms: 40.0,
            frames: 1,
            bounds: None,
        });
        assert_eq!(settled_at(&vanished), Some(40.0));
        assert_eq!(settled_at(&[]), None);
    }

    #[test]
    fn collapse_drops_repeats_but_keeps_first_last_and_skips_missing() {
        let mut samples = vec![
            at(0.0, 10.0),
            at(8.0, 10.2),
            at(16.0, 20.0),
            at(24.0, 20.0),
            at(32.0, 20.0),
        ];
        samples.insert(
            2,
            Sample {
                at_ms: 12.0,
                frames: 1,
                bounds: None,
            },
        );
        assert_eq!(
            collapse(&samples),
            vec![
                vec![0, 10, 100, 200, 30],
                vec![16, 20, 100, 200, 30],
                vec![32, 20, 100, 200, 30],
            ]
        );
    }

    #[test]
    fn the_sampler_keeps_only_new_frames_and_counts_reresolutions() {
        let element = Element::Select {
            select: "Button \"Go\"".into(),
            nth: None,
        };
        let tree_at = |frame: u64, id: &str, x: f32| {
            let mut node = tree::tests::node(id, "Button", Some("Go"));
            node.bounds.as_mut().unwrap().x = x;
            tree::Tree {
                frame,
                nodes: vec![node],
                ..tree::Tree::default()
            }
        };
        let mut sampler = Sampler::new(Some("e1".into()), 4);
        sampler.add(&tree_at(4, "e1", 0.0), &element, 1.0);
        sampler.add(&tree_at(5, "e1", 5.0), &element, 2.0);
        sampler.add(&tree_at(5, "e1", 9.0), &element, 3.0);
        sampler.add(&tree_at(7, "e2", 15.0), &element, 4.0);
        sampler.add(
            &tree::Tree {
                frame: 8,
                ..tree::Tree::default()
            },
            &element,
            5.0,
        );
        assert_eq!(sampler.samples.len(), 3);
        assert_eq!(sampler.samples[1].frames, 2);
        let report = sampler.report(40.0);
        assert_eq!(report["reresolved"], 1);
        assert_eq!(report["missingFrames"], 1);
        assert_eq!(report["framesSampled"], 3);
        assert_eq!(report["settledAtMs"], 5.0);
    }
}
