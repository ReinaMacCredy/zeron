use sha2::{Digest, Sha256};
use zeron_proto::computer::{ComputerPoint, ComputerRect, ComputerScreenshot, ComputerScrollDelta};

use super::ui_tree_targeting::{ComputerTargetError, ComputerTargetErrorCode};

pub const SCREENSHOT_FRAMES_PER_THREAD: usize = 8;
const SCREENSHOT_FRAME_THREADS: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub struct ScreenshotFrame {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub region: ComputerRect,
    pub scale: f64,
    pub window_id: Option<String>,
    image_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenshotFrameSource<'a> {
    pub width: u32,
    pub height: u32,
    pub region: Option<ComputerRect>,
    pub scale: Option<f64>,
    pub bytes_base64: Option<&'a str>,
}

impl<'a> From<&'a ComputerScreenshot> for ScreenshotFrameSource<'a> {
    fn from(screenshot: &'a ComputerScreenshot) -> Self {
        Self {
            width: screenshot.width,
            height: screenshot.height,
            region: screenshot.region,
            scale: screenshot.scale,
            bytes_base64: Some(&screenshot.bytes_base64),
        }
    }
}

fn image_hash(bytes_base64: &str) -> String {
    Sha256::digest(bytes_base64.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn frame_scale(width: u32, region: &ComputerRect, scale: Option<f64>) -> f64 {
    scale.unwrap_or(f64::from(width) / region.width)
}

#[derive(Debug, Default)]
pub struct ScreenshotFrameRegistry {
    threads: Vec<(String, Vec<ScreenshotFrame>)>,
    sequence: u64,
}

impl ScreenshotFrameRegistry {
    fn frames(&self, thread_id: &str) -> &[ScreenshotFrame] {
        self.threads
            .iter()
            .find(|(id, _)| id == thread_id)
            .map_or(&[], |(_, frames)| frames.as_slice())
    }

    fn take_thread(&mut self, thread_id: &str) -> Vec<ScreenshotFrame> {
        self.threads
            .iter()
            .position(|(id, _)| id == thread_id)
            .map(|index| self.threads.remove(index).1)
            .unwrap_or_default()
    }

    pub fn record(
        &mut self,
        thread_id: &str,
        screenshot: ScreenshotFrameSource<'_>,
        window_id: Option<&str>,
    ) -> Option<ScreenshotFrame> {
        let Some(region) = screenshot
            .region
            .filter(|region| region.width > 0.0 && region.height > 0.0)
        else {
            self.take_thread(thread_id);
            return None;
        };
        let scale = frame_scale(screenshot.width, &region, screenshot.scale);
        if !scale.is_finite() || scale <= 0.0 {
            self.take_thread(thread_id);
            return None;
        }
        self.sequence += 1;
        let frame = ScreenshotFrame {
            id: format!("shot-{}", self.sequence),
            width: screenshot.width,
            height: screenshot.height,
            region,
            scale,
            window_id: window_id.map(str::to_owned),
            image_hash: screenshot.bytes_base64.map(image_hash),
        };
        let mut frames = self.take_thread(thread_id);
        frames.push(frame.clone());
        if frames.len() > SCREENSHOT_FRAMES_PER_THREAD {
            frames.drain(..frames.len() - SCREENSHOT_FRAMES_PER_THREAD);
        }
        self.threads.push((thread_id.to_owned(), frames));
        if self.threads.len() > SCREENSHOT_FRAME_THREADS {
            self.threads
                .drain(..self.threads.len() - SCREENSHOT_FRAME_THREADS);
        }
        Some(frame)
    }

    pub fn latest(&self, thread_id: &str) -> Option<&ScreenshotFrame> {
        self.frames(thread_id).last()
    }

    pub fn match_latest(
        &self,
        thread_id: &str,
        screenshot: &ComputerScreenshot,
        window_id: Option<&str>,
    ) -> Option<&ScreenshotFrame> {
        let frame = self.latest(thread_id)?;
        let region = screenshot.region.as_ref()?;
        let same = frame.window_id.as_deref() == window_id
            && frame.width == screenshot.width
            && frame.height == screenshot.height
            && frame.scale == frame_scale(screenshot.width, region, screenshot.scale)
            && frame.region == *region
            && frame.image_hash.as_deref() == Some(image_hash(&screenshot.bytes_base64).as_str());
        same.then_some(frame)
    }

    pub fn resolve(
        &self,
        thread_id: &str,
        screenshot_id: Option<&str>,
    ) -> Result<&ScreenshotFrame, ComputerTargetError> {
        let frames = self.frames(thread_id);
        if let Some(screenshot_id) = screenshot_id {
            if let Some(frame) = frames.iter().find(|frame| frame.id == screenshot_id) {
                return Ok(frame);
            }
            let available = if frames.is_empty() {
                ". Take a screenshot first and use its screenshotId.".to_owned()
            } else {
                let ids: Vec<&str> = frames.iter().map(|frame| frame.id.as_str()).collect();
                format!(
                    "; the screenshots still available are {} (newest last).",
                    ids.join(", ")
                )
            };
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::NotFound,
                format!(
                    "No screenshot {} is available to point into{available}",
                    serde_json::to_string(screenshot_id).unwrap_or_default()
                ),
                Vec::new(),
            ));
        }
        frames.last().ok_or_else(|| {
            ComputerTargetError::new(
                ComputerTargetErrorCode::Invalid,
                "No screenshot to point into: this conversation has not received one yet. \
                 Take one with computer_screenshot or computer_get_state, then give x/y as pixel \
                 coordinates in that image.",
                Vec::new(),
            )
        })
    }
}

fn describe_frame(frame: &ScreenshotFrame) -> String {
    format!(
        "the {}x{} screenshot {}",
        frame.width, frame.height, frame.id
    )
}

fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

fn clamp_to_span(value: f64, start: f64, length: f64) -> f64 {
    let end = start + (length - 1.0).max(0.0);
    end.min(start.max(value))
}

pub fn screenshot_point_to_desktop(
    frame: &ScreenshotFrame,
    x: f64,
    y: f64,
) -> Result<ComputerPoint, ComputerTargetError> {
    let (width, height) = (f64::from(frame.width), f64::from(frame.height));
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x > width || y > height {
        return Err(ComputerTargetError::new(
            ComputerTargetErrorCode::Offscreen,
            format!(
                "Computer target ({x}, {y}) is outside {}; x/y are pixel coordinates in that image.",
                describe_frame(frame)
            ),
            Vec::new(),
        ));
    }
    let region = &frame.region;
    Ok(ComputerPoint {
        x: clamp_to_span(js_round(region.x + x / frame.scale), region.x, region.width),
        y: clamp_to_span(
            js_round(region.y + y / frame.scale),
            region.y,
            region.height,
        ),
    })
}

pub fn screenshot_rect_to_desktop(
    frame: &ScreenshotFrame,
    rect: &ComputerRect,
) -> Result<ComputerRect, ComputerTargetError> {
    let left = rect.x.max(0.0);
    let top = rect.y.max(0.0);
    let right = f64::from(frame.width).min(rect.x + rect.width);
    let bottom = f64::from(frame.height).min(rect.y + rect.height);
    if !(right > left && bottom > top) {
        return Err(ComputerTargetError::new(
            ComputerTargetErrorCode::Offscreen,
            format!(
                "The requested region does not overlap {}; x/y/width/height are pixel coordinates in that image.",
                describe_frame(frame)
            ),
            Vec::new(),
        ));
    }
    let region = &frame.region;
    let x = js_round(region.x + left / frame.scale);
    let y = js_round(region.y + top / frame.scale);
    let far_x = js_round(region.x + right / frame.scale);
    let far_y = js_round(region.y + bottom / frame.scale);
    Ok(ComputerRect {
        x,
        y,
        width: (far_x - x).max(1.0),
        height: (far_y - y).max(1.0),
    })
}

pub fn screenshot_delta_to_desktop(
    frame: &ScreenshotFrame,
    delta_x: f64,
    delta_y: f64,
) -> ComputerScrollDelta {
    ComputerScrollDelta {
        delta_x: delta_x / frame.scale,
        delta_y: delta_y / frame.scale,
    }
}

#[cfg(test)]
mod tests {
    use zeron_proto::computer::ComputerPngMimeType;

    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> ComputerRect {
        ComputerRect {
            x,
            y,
            width,
            height,
        }
    }

    fn half() -> ScreenshotFrame {
        ScreenshotFrame {
            id: "shot-1".into(),
            width: 200,
            height: 400,
            region: rect(1_050.0, 120.0, 400.0, 800.0),
            scale: 0.5,
            window_id: None,
            image_hash: None,
        }
    }

    fn shot(width: u32, height: u32) -> ScreenshotFrameSource<'static> {
        ScreenshotFrameSource {
            width,
            height,
            region: Some(rect(0.0, 0.0, f64::from(width), f64::from(height))),
            scale: Some(1.0),
            bytes_base64: None,
        }
    }

    const fn point(x: f64, y: f64) -> ComputerPoint {
        ComputerPoint { x, y }
    }

    #[test]
    fn adds_the_region_offset_and_undoes_the_downscale() {
        assert_eq!(
            screenshot_point_to_desktop(&half(), 100.0, 100.0),
            Ok(point(1_250.0, 320.0))
        );
        assert_eq!(
            screenshot_point_to_desktop(&half(), 0.0, 0.0),
            Ok(point(1_050.0, 120.0))
        );
    }

    #[test]
    fn lands_a_far_edge_pixel_on_the_regions_last_desktop_pixel() {
        assert_eq!(
            screenshot_point_to_desktop(&half(), 200.0, 400.0),
            Ok(point(1_449.0, 919.0))
        );
    }

    #[test]
    fn refuses_a_point_the_picture_does_not_show() {
        for (x, y) in [(201.0, 10.0), (10.0, 401.0), (-1.0, 10.0), (f64::NAN, 10.0)] {
            assert!(screenshot_point_to_desktop(&half(), x, y).is_err());
        }
        let error = screenshot_point_to_desktop(&half(), 201.0, 10.0).unwrap_err();
        assert_eq!(error.code, ComputerTargetErrorCode::Offscreen);
        assert!(error.message.contains("200x400 screenshot shot-1"));
    }

    #[test]
    fn maps_and_clips_a_rect_to_what_the_frame_covers() {
        assert_eq!(
            screenshot_rect_to_desktop(&half(), &rect(100.0, 300.0, 200.0, 200.0)),
            Ok(rect(1_250.0, 720.0, 200.0, 200.0))
        );
        assert_eq!(
            screenshot_rect_to_desktop(&half(), &rect(-10.0, -10.0, 20.0, 20.0)),
            Ok(rect(1_050.0, 120.0, 20.0, 20.0))
        );
    }

    #[test]
    fn refuses_a_rect_with_nothing_of_the_picture_in_it() {
        assert!(screenshot_rect_to_desktop(&half(), &rect(200.0, 0.0, 10.0, 10.0)).is_err());
    }

    #[test]
    fn scales_a_distance_the_same_way_as_a_point() {
        assert_eq!(
            screenshot_delta_to_desktop(&half(), 40.0, -10.0),
            ComputerScrollDelta {
                delta_x: 80.0,
                delta_y: -20.0
            }
        );
    }

    #[test]
    fn rounds_negative_half_pixels_up_like_math_round() {
        let frame = ScreenshotFrame {
            region: rect(-100.5, -50.5, 100.0, 100.0),
            scale: 1.0,
            ..half()
        };
        assert_eq!(
            screenshot_point_to_desktop(&frame, 0.0, 0.0),
            Ok(point(-100.0, -50.0))
        );
    }

    #[test]
    fn hands_out_sequential_ids_and_resolves_the_latest_by_default() {
        let mut frames = ScreenshotFrameRegistry::default();
        assert_eq!(
            frames.record("t", shot(100, 100), None).unwrap().id,
            "shot-1"
        );
        assert_eq!(frames.record("t", shot(50, 50), None).unwrap().id, "shot-2");
        assert_eq!(frames.resolve("t", None).unwrap().id, "shot-2");
        assert_eq!(frames.resolve("t", Some("shot-1")).unwrap().width, 100);
    }

    #[test]
    fn refuses_a_thread_that_has_not_seen_a_screenshot_and_an_unknown_id() {
        let mut frames = ScreenshotFrameRegistry::default();
        assert_eq!(
            frames.resolve("t", None).unwrap_err().code,
            ComputerTargetErrorCode::Invalid
        );
        frames.record("t", shot(100, 100), None);
        let unknown = frames.resolve("t", Some("shot-7")).unwrap_err();
        assert_eq!(unknown.code, ComputerTargetErrorCode::NotFound);
        assert!(unknown.message.contains("shot-1"));
        assert!(frames.resolve("u", None).is_err());
    }

    #[test]
    fn keeps_a_bounded_number_of_frames_per_thread_and_forgets_the_oldest() {
        let mut frames = ScreenshotFrameRegistry::default();
        for _ in 0..=SCREENSHOT_FRAMES_PER_THREAD {
            frames.record("t", shot(100, 100), None);
        }
        assert!(frames.resolve("t", Some("shot-1")).is_err());
        assert_eq!(frames.resolve("t", Some("shot-2")).unwrap().id, "shot-2");
    }

    #[test]
    fn records_nothing_for_a_screenshot_that_carries_no_mapping() {
        let mut frames = ScreenshotFrameRegistry::default();
        let unmapped = ScreenshotFrameSource {
            region: None,
            scale: None,
            ..shot(10, 10)
        };
        assert!(frames.record("t", unmapped, None).is_none());
        assert!(frames.resolve("t", None).is_err());
    }

    #[test]
    fn forgets_the_least_recently_pointed_into_thread_first() {
        let mut frames = ScreenshotFrameRegistry::default();
        for index in 0..SCREENSHOT_FRAME_THREADS {
            frames.record(&format!("t{index}"), shot(10, 10), None);
        }
        frames.record("t0", shot(10, 10), None);
        frames.record("extra", shot(10, 10), None);
        assert!(frames.latest("t0").is_some());
        assert!(frames.latest("t1").is_none());
        assert!(frames.latest("extra").is_some());
    }

    fn image() -> ComputerScreenshot {
        ComputerScreenshot {
            mime_type: ComputerPngMimeType::Png,
            width: 200,
            height: 400,
            size_bytes: 3,
            bytes_base64: "YWJj".into(),
            window_id: None,
            region: Some(rect(1050.0, 120.0, 400.0, 800.0)),
            scale: Some(0.5),
            captured_at: "2026-09-05T00:00:00.000Z".into(),
        }
    }

    #[test]
    fn matches_a_shared_capture_by_content_across_copies() {
        let mut frames = ScreenshotFrameRegistry::default();
        let image = image();
        let frame = frames
            .record("thread-a", (&image).into(), Some("window-a"))
            .unwrap();
        frames.record("thread-b", (&image).into(), Some("window-a"));
        for _ in 0..5 {
            assert_eq!(
                frames.match_latest("thread-a", &image, Some("window-a")),
                Some(&frame)
            );
        }
        assert_eq!(
            frames.match_latest("thread-a", &image.clone(), Some("window-a")),
            Some(&frame)
        );
        let other = ComputerScreenshot {
            bytes_base64: "ZGVm".into(),
            ..image
        };
        assert_eq!(
            frames.match_latest("thread-a", &other, Some("window-a")),
            None
        );
    }

    #[test]
    fn requires_identical_pixels_window_geometry_and_scale_in_the_latest_delivered_frame() {
        let mut frames = ScreenshotFrameRegistry::default();
        let image = image();
        let frame = frames
            .record("thread-a", (&image).into(), Some("window-a"))
            .unwrap();
        let later = ComputerScreenshot {
            captured_at: "2026-09-05T00:01:00.000Z".into(),
            ..image.clone()
        };
        assert_eq!(
            frames.match_latest("thread-a", &later, Some("window-a")),
            Some(&frame)
        );
        assert_eq!(
            frames.match_latest("thread-a", &image, Some("window-b")),
            None
        );
        assert_eq!(
            frames.match_latest("thread-b", &image, Some("window-a")),
            None
        );
        for changed in [
            ComputerScreenshot {
                scale: Some(1.0),
                ..image.clone()
            },
            ComputerScreenshot {
                width: 201,
                ..image.clone()
            },
            ComputerScreenshot {
                bytes_base64: "ZGVm".into(),
                ..image.clone()
            },
            ComputerScreenshot {
                region: Some(rect(600.0, 120.0, 400.0, 800.0)),
                ..image.clone()
            },
        ] {
            assert_eq!(
                frames.match_latest("thread-a", &changed, Some("window-a")),
                None
            );
        }
        frames.record("thread-b", (&image).into(), Some("window-a"));
        assert_eq!(
            frames.match_latest("thread-a", &image, Some("window-a")),
            Some(&frame)
        );
        frames.record("thread-a", (&image).into(), Some("window-b"));
        assert_eq!(
            frames.match_latest("thread-a", &image, Some("window-a")),
            None
        );
    }

    #[test]
    fn invalidates_the_active_frame_after_delivering_an_image_without_geometry() {
        let mut frames = ScreenshotFrameRegistry::default();
        let image = image();
        frames.record("thread-a", (&image).into(), Some("window-a"));
        frames.record(
            "thread-a",
            ScreenshotFrameSource {
                width: 1,
                height: 1,
                region: None,
                scale: None,
                bytes_base64: Some("YWJj"),
            },
            None,
        );
        assert_eq!(
            frames.match_latest("thread-a", &image, Some("window-a")),
            None
        );
        assert!(
            frames
                .resolve("thread-a", None)
                .unwrap_err()
                .message
                .contains("No screenshot")
        );
    }

    #[test]
    fn reuses_only_the_latest_delivered_pixels_with_an_identical_coordinate_frame() {
        let screenshot = ComputerScreenshot {
            window_id: Some("fake-terminal".into()),
            region: Some(rect(40.0, 60.0, 800.0, 600.0)),
            scale: Some(1.0),
            width: 800,
            height: 600,
            ..image()
        };
        let mut frames = ScreenshotFrameRegistry::default();
        let delivered = frames
            .record("a", (&screenshot).into(), Some("fake-terminal"))
            .unwrap();
        assert_eq!(
            frames.match_latest("a", &screenshot, Some("fake-terminal")),
            Some(&delivered)
        );
        assert_eq!(
            frames.match_latest("b", &screenshot, Some("fake-terminal")),
            None
        );
        for changed in [
            ComputerScreenshot {
                width: 801,
                ..screenshot.clone()
            },
            ComputerScreenshot {
                height: 601,
                ..screenshot.clone()
            },
            ComputerScreenshot {
                scale: Some(0.25),
                ..screenshot.clone()
            },
            ComputerScreenshot {
                region: Some(rect(41.0, 60.0, 800.0, 600.0)),
                ..screenshot.clone()
            },
        ] {
            assert_eq!(
                frames.match_latest("a", &changed, Some("fake-terminal")),
                None
            );
        }
        frames.record("a", (&screenshot).into(), Some("different-window"));
        assert_eq!(
            frames.match_latest("a", &screenshot, Some("fake-terminal")),
            None
        );
    }
}
