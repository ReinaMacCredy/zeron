use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use cursor_overlay::codex_cursor::{fog_from_wallpaper, WALLPAPER_SAMPLE_RADIUS};

use super::display::DisplayFrame;

const CELL_POINTS: f64 = 8.0;
const THUMBNAIL_PIXELS: i32 = 384;
const RECHECK: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq)]
pub struct Backdrop {
    cols: usize,
    rows: usize,
    rgb: Vec<[f32; 3]>,
}

impl Backdrop {
    pub fn from_rgba(width: usize, height: usize, rgba: &[u8], screen: (f64, f64)) -> Option<Self> {
        if width == 0 || height == 0 || rgba.len() < width * height * 4 {
            return None;
        }
        if !(screen.0 > 0.0 && screen.1 > 0.0) {
            return None;
        }
        let scale = (screen.0 / width as f64).max(screen.1 / height as f64);
        let offset = (
            (width as f64 * scale - screen.0) / 2.0,
            (height as f64 * scale - screen.1) / 2.0,
        );
        let cols = (screen.0 / CELL_POINTS).ceil().max(1.0) as usize;
        let rows = (screen.1 / CELL_POINTS).ceil().max(1.0) as usize;
        let texel = |x: usize, y: usize| {
            let i = (y.min(height - 1) * width + x.min(width - 1)) * 4;
            [
                f32::from(rgba[i]),
                f32::from(rgba[i + 1]),
                f32::from(rgba[i + 2]),
            ]
        };
        let mut rgb = Vec::with_capacity(cols * rows);
        for row in 0..rows {
            for col in 0..cols {
                let sx = ((col as f64 + 0.5) * CELL_POINTS + offset.0) / scale - 0.5;
                let sy = ((row as f64 + 0.5) * CELL_POINTS + offset.1) / scale - 0.5;
                rgb.push(bilinear(sx, sy, texel));
            }
        }
        let radius = (WALLPAPER_SAMPLE_RADIUS / CELL_POINTS / 2.0)
            .round()
            .max(1.0) as usize;
        for _ in 0..2 {
            box_blur(&mut rgb, cols, rows, radius);
        }
        Some(Self { cols, rows, rgb })
    }

    pub fn sample(&self, x: f64, y: f64) -> [u8; 3] {
        let (cols, rows) = (self.cols, self.rows);
        let at = |c: usize, r: usize| self.rgb[r.min(rows - 1) * cols + c.min(cols - 1)];
        bilinear(x / CELL_POINTS - 0.5, y / CELL_POINTS - 0.5, at)
            .map(|v| v.round().clamp(0.0, 255.0) as u8)
    }
}

fn bilinear(x: f64, y: f64, at: impl Fn(usize, usize) -> [f32; 3]) -> [f32; 3] {
    let x = x.max(0.0);
    let y = y.max(0.0);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let (a, b, c, d) = (
        at(x0, y0),
        at(x0 + 1, y0),
        at(x0, y0 + 1),
        at(x0 + 1, y0 + 1),
    );
    let mut out = [0.0; 3];
    for i in 0..3 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bottom = c[i] + (d[i] - c[i]) * fx;
        out[i] = top + (bottom - top) * fy;
    }
    out
}

fn box_blur(rgb: &mut [[f32; 3]], cols: usize, rows: usize, radius: usize) {
    let mut scratch = rgb.to_vec();
    let window = |count: usize, center: usize| {
        (
            center.saturating_sub(radius),
            (center + radius).min(count - 1),
        )
    };
    for r in 0..rows {
        for c in 0..cols {
            let (lo, hi) = window(cols, c);
            let mut sum = [0.0; 3];
            for k in lo..=hi {
                let v = rgb[r * cols + k];
                for i in 0..3 {
                    sum[i] += v[i];
                }
            }
            let n = (hi - lo + 1) as f32;
            scratch[r * cols + c] = sum.map(|s| s / n);
        }
    }
    for c in 0..cols {
        for r in 0..rows {
            let (lo, hi) = window(rows, r);
            let mut sum = [0.0; 3];
            for k in lo..=hi {
                let v = scratch[k * cols + c];
                for i in 0..3 {
                    sum[i] += v[i];
                }
            }
            let n = (hi - lo + 1) as f32;
            rgb[r * cols + c] = sum.map(|s| s / n);
        }
    }
}

type Shared = Arc<Mutex<HashMap<u32, Arc<Backdrop>>>>;

pub struct WallpaperTint {
    shared: Shared,
    refresh: mpsc::Sender<(u32, (f64, f64))>,
    last_request: HashMap<u32, Instant>,
}

impl WallpaperTint {
    pub fn spawn() -> Self {
        let shared: Shared = Arc::default();
        let (refresh, requests) = mpsc::channel::<(u32, (f64, f64))>();
        let worker = Arc::clone(&shared);
        let _ = std::thread::Builder::new()
            .name("cursor-wallpaper".into())
            .spawn(move || {
                let mut loaded: HashMap<u32, (PathBuf, Option<SystemTime>, (f64, f64))> =
                    HashMap::new();
                while let Ok(first) = requests.recv() {
                    let mut pending = vec![first];
                    pending.extend(requests.try_iter());
                    pending.sort_by_key(|(id, _)| *id);
                    pending.dedup_by_key(|(id, _)| *id);
                    for (display, size) in pending {
                        let Some(path) = platform::desktop_image_path(display) else {
                            continue;
                        };
                        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                        let key = (path, modified, size);
                        if loaded.get(&display) == Some(&key) {
                            continue;
                        }
                        let backdrop = platform::decode_thumbnail(&key.0)
                            .and_then(|(w, h, rgba)| Backdrop::from_rgba(w, h, &rgba, size));
                        let mut shared = worker.lock().unwrap_or_else(|p| p.into_inner());
                        match backdrop {
                            Some(backdrop) => shared.insert(display, Arc::new(backdrop)),
                            None => shared.remove(&display),
                        };
                        loaded.insert(display, key);
                    }
                }
            });
        Self {
            shared,
            refresh,
            last_request: HashMap::new(),
        }
    }

    pub fn warm(&mut self, display: &DisplayFrame) {
        let stale = self
            .last_request
            .get(&display.id)
            .is_none_or(|last| last.elapsed() >= RECHECK);
        if stale {
            self.last_request.insert(display.id, Instant::now());
            let _ = self.refresh.send((display.id, display.size()));
        }
    }

    pub fn tint_at(&mut self, display: &DisplayFrame, x: f64, y: f64) -> Option<[u8; 3]> {
        self.warm(display);
        let backdrop = self
            .shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&display.id)
            .cloned()?;
        let (ox, oy) = display.origin();
        Some(fog_from_wallpaper(backdrop.sample(x - ox, y - oy)))
    }
}

mod platform {
    use std::ffi::{c_void, CStr};
    use std::path::{Path, PathBuf};

    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use core_foundation::url::CFURL;
    use core_graphics::base::{kCGBitmapByteOrder32Big, kCGImageAlphaPremultipliedLast};
    use core_graphics::color_space::CGColorSpace;
    use core_graphics::context::CGContext;
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};
    use core_graphics::image::CGImage;
    use foreign_types::ForeignType;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};

    #[link(name = "ImageIO", kind = "framework")]
    extern "C" {
        fn CGImageSourceCreateWithURL(url: *const c_void, options: *const c_void) -> *const c_void;
        fn CGImageSourceCreateThumbnailAtIndex(
            source: *const c_void,
            index: usize,
            options: *const c_void,
        ) -> *mut c_void;
        static kCGImageSourceThumbnailMaxPixelSize: CFStringRef;
        static kCGImageSourceCreateThumbnailFromImageAlways: CFStringRef;
        static kCGImageSourceCreateThumbnailWithTransform: CFStringRef;
    }

    pub(super) fn desktop_image_path(display: u32) -> Option<PathBuf> {
        objc2::rc::autoreleasepool(|_| unsafe {
            let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
            if screens.is_null() {
                return None;
            }
            let key: *mut AnyObject = msg_send![class!(NSString),
                stringWithUTF8String: c"NSScreenNumber".as_ptr()
            ];
            // NSScreen.screens is Swift-bridged: count/index are NSInteger there, so enumerate instead.
            let screens: *mut AnyObject = msg_send![screens, objectEnumerator];
            let mut screen: *mut AnyObject = std::ptr::null_mut();
            loop {
                let candidate: *mut AnyObject = msg_send![screens, nextObject];
                if candidate.is_null() {
                    break;
                }
                let description: *mut AnyObject = msg_send![candidate, deviceDescription];
                let number: *mut AnyObject = msg_send![description, objectForKey: key];
                if number.is_null() {
                    continue;
                }
                let id: u32 = msg_send![number, unsignedIntValue];
                if id == display {
                    screen = candidate;
                    break;
                }
            }
            if screen.is_null() {
                return None;
            }
            let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            let url: *mut AnyObject = msg_send![workspace, desktopImageURLForScreen: screen];
            if url.is_null() {
                return None;
            }
            let path: *mut AnyObject = msg_send![url, path];
            if path.is_null() {
                return None;
            }
            let utf8: *const std::ffi::c_char = msg_send![path, UTF8String];
            (!utf8.is_null())
                .then(|| PathBuf::from(CStr::from_ptr(utf8).to_string_lossy().into_owned()))
        })
    }

    pub(super) fn decode_thumbnail(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
        let url = CFURL::from_path(path, false)?;
        let options = unsafe {
            CFDictionary::from_CFType_pairs(&[
                (
                    CFString::wrap_under_get_rule(kCGImageSourceThumbnailMaxPixelSize).as_CFType(),
                    CFNumber::from(super::THUMBNAIL_PIXELS).as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kCGImageSourceCreateThumbnailFromImageAlways)
                        .as_CFType(),
                    CFBoolean::true_value().as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kCGImageSourceCreateThumbnailWithTransform)
                        .as_CFType(),
                    CFBoolean::true_value().as_CFType(),
                ),
            ])
        };
        let source = unsafe {
            CGImageSourceCreateWithURL(url.as_concrete_TypeRef().cast(), std::ptr::null())
        };
        if source.is_null() {
            return None;
        }
        let source = unsafe { CFType::wrap_under_create_rule(source) };
        let image = unsafe {
            CGImageSourceCreateThumbnailAtIndex(
                source.as_CFTypeRef(),
                0,
                options.as_concrete_TypeRef().cast(),
            )
        };
        if image.is_null() {
            return None;
        }
        let image = unsafe { CGImage::from_ptr(image.cast()) };
        let (w, h) = (image.width(), image.height());
        if w == 0 || h == 0 {
            return None;
        }
        let mut context = CGContext::create_bitmap_context(
            None,
            w,
            h,
            8,
            w * 4,
            &CGColorSpace::create_device_rgb(),
            kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big,
        );
        context.draw_image(
            CGRect::new(&CGPoint::new(0.0, 0.0), &CGSize::new(w as f64, h as f64)),
            &image,
        );
        let stride = context.bytes_per_row();
        let data = context.data();
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            rgba.extend_from_slice(&data[row * stride..row * stride + w * 4]);
        }
        Some((w, h, rgba))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn halves(width: usize, height: usize) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(width * height * 4);
        for _ in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(if x < width / 2 {
                    &[220, 40, 40, 255]
                } else {
                    &[40, 40, 220, 255]
                });
            }
        }
        rgba
    }

    #[test]
    fn samples_follow_the_wallpaper_under_the_point() {
        let backdrop = Backdrop::from_rgba(200, 100, &halves(200, 100), (1000.0, 500.0)).unwrap();
        let left = backdrop.sample(100.0, 250.0);
        let right = backdrop.sample(900.0, 250.0);
        assert!(left[0] > 200 && left[2] < 60, "{left:?}");
        assert!(right[2] > 200 && right[0] < 60, "{right:?}");
        let seam = backdrop.sample(500.0, 250.0);
        assert!(
            seam[0] > 80 && seam[2] > 80,
            "the seam is blurred: {seam:?}"
        );
    }

    #[test]
    fn aspect_fill_crops_the_centre_of_a_wide_wallpaper() {
        let mut rgba = halves(400, 100);
        for y in 0..100 {
            for x in 0..100 {
                let i = (y * 400 + x) * 4;
                rgba[i..i + 4].copy_from_slice(&[0, 200, 0, 255]);
            }
        }
        let backdrop = Backdrop::from_rgba(400, 100, &rgba, (500.0, 500.0)).unwrap();
        let left_edge = backdrop.sample(4.0, 250.0);
        assert!(
            left_edge[1] < 60,
            "the cropped-away green band must not show: {left_edge:?}"
        );
    }

    #[test]
    fn rejects_truncated_pixels() {
        assert!(Backdrop::from_rgba(10, 10, &[0; 12], (100.0, 100.0)).is_none());
    }

    #[test]
    #[ignore = "reads the live desktop wallpaper; run with --ignored on a logged-in Mac"]
    fn live_wallpaper_decodes_into_a_fog_tint() {
        let displays = super::super::display::active_displays();
        let display = displays
            .iter()
            .find(|d| d.primary)
            .expect("primary display");
        let path = std::env::var_os("FOG_PATH")
            .map(PathBuf::from)
            .or_else(|| platform::desktop_image_path(display.id))
            .expect("desktop image path");
        let (w, h, rgba) = platform::decode_thumbnail(&path).expect("decodable wallpaper");
        let size = display.size();
        let backdrop = Backdrop::from_rgba(w, h, &rgba, size).unwrap();
        let point = (
            std::env::var("FOG_X")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(755.0),
            std::env::var("FOG_Y")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(330.0),
        );
        let wall = backdrop.sample(point.0, point.1);
        println!(
            "wallpaper {} {w}x{h} screen {size:?} at {point:?}: wallpaper {wall:?} fog {:?}",
            path.display(),
            fog_from_wallpaper(wall)
        );
    }
}
