use gpui::{AnyWindowHandle, App};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub(super) struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[cfg(not(target_os = "macos"))]
pub(super) fn snapshot(
    _: AnyWindowHandle,
    _: Option<Rect>,
    _: Option<PathBuf>,
    _: &mut App,
) -> Result<Value, String> {
    Err("`snapshot` is only implemented on macOS".into())
}

#[cfg(target_os = "macos")]
pub(super) fn snapshot(
    handle: AnyWindowHandle,
    rect: Option<Rect>,
    path: Option<PathBuf>,
    cx: &mut App,
) -> Result<Value, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let path = match path {
        Some(path) => {
            check_path(&path)?;
            path
        }
        None => zeron_proto::ui_driver::run_dir()
            .join("shots")
            .join(format!(
                "{}-{}.png",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed) + 1
            )),
    };
    let number = handle
        .update(cx, |_, window, _| window_number(window))
        .map_err(|error| format!("window is gone: {error}"))?
        .ok_or("this window has no window-server id")?;
    let frame = crate::shell::switcher::capture_window_number(number)
        .ok_or_else(|| format!("the window server returned no image for window {number}"))?;
    let (x, y, w, h) = crop(frame.width, frame.height, rect)?;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * frame.width + x) * 4) as usize;
        for px in frame.bgra[start..start + (w * 4) as usize].chunks_exact(4) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    let image = image::RgbaImage::from_raw(w, h, rgba).ok_or("captured pixels do not fit")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    }
    image
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(serde_json::json!({ "path": path, "width": w, "height": h }))
}

#[cfg(target_os = "macos")]
fn check_path(path: &std::path::Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("`path` must be absolute: {}", path.display()));
    }
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
    {
        return Err(format!("`path` must end in .png: {}", path.display()));
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() => Err(format!(
            "`path` exists and is not a regular file (a symlink, folder or device): {}",
            path.display()
        )),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{}: {error}", path.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(target_os = "macos")]
fn window_number(window: &gpui::Window) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // SAFETY: gpui's AppKit handle points at the window's live content view; we are on the main thread.
    let view = unsafe { handle.ns_view.cast::<objc2_app_kit::NSView>().as_ref() };
    u32::try_from(view.window()?.windowNumber()).ok()
}

#[cfg(target_os = "macos")]
fn crop(width: u32, height: u32, rect: Option<Rect>) -> Result<(u32, u32, u32, u32), String> {
    let Some(rect) = rect else {
        return Ok((0, 0, width, height));
    };
    let clamp = |value: f32, max: u32| (value.round().max(0.0) as u32).min(max);
    let (x0, y0) = (clamp(rect.x, width), clamp(rect.y, height));
    let (x1, y1) = (
        clamp(rect.x + rect.w, width),
        clamp(rect.y + rect.h, height),
    );
    if x1 <= x0 || y1 <= y0 {
        return Err(format!(
            "`rect` {rect:?} is outside the {width}x{height} window"
        ));
    }
    Ok((x0, y0, x1 - x0, y1 - y0))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn snapshot_path_is_an_absolute_png_and_never_a_link_or_folder() {
        let dir = tempfile::tempdir().unwrap();
        let shot = dir.path().join("shot.png");
        assert_eq!(check_path(&shot), Ok(()));
        std::fs::write(&shot, b"old").unwrap();
        assert_eq!(check_path(&shot), Ok(()));
        assert_eq!(check_path(&dir.path().join("UPPER.PNG")), Ok(()));
        assert!(
            check_path(std::path::Path::new("rel/shot.png"))
                .unwrap_err()
                .contains("absolute")
        );
        for name in ["shot.txt", "shot", "shot.png.txt"] {
            let error = check_path(&dir.path().join(name)).unwrap_err();
            assert!(error.contains("must end in .png"), "{name}: {error}");
        }
        let link = dir.path().join("link.png");
        std::os::unix::fs::symlink(&shot, &link).unwrap();
        assert!(
            check_path(&link)
                .unwrap_err()
                .contains("not a regular file")
        );
        let folder = dir.path().join("folder.png");
        std::fs::create_dir(&folder).unwrap();
        assert!(
            check_path(&folder)
                .unwrap_err()
                .contains("not a regular file")
        );
    }
}
