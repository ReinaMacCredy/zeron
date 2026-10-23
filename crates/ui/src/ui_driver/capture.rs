use gpui::{AnyWindowHandle, App};
use image::RgbaImage;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub(super) struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

pub(super) fn snapshot(
    handle: AnyWindowHandle,
    rect: Option<Rect>,
    path: Option<PathBuf>,
    cx: &mut App,
) -> Result<Value, String> {
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
    let frame = handle
        .update(cx, |_, window, _| grab(window))
        .map_err(|error| format!("window is gone: {error}"))??;
    let (x, y, w, h) = crop(frame.width(), frame.height(), rect)?;
    let image = image::imageops::crop_imm(&frame, x, y, w, h).to_image();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    }
    image
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(serde_json::json!({ "path": path, "width": w, "height": h }))
}

#[cfg(target_os = "macos")]
fn grab(window: &mut gpui::Window) -> Result<RgbaImage, String> {
    use crate::window_capture;
    let number =
        window_capture::window_number(window).ok_or("this window has no window-server id")?;
    let frame = window_capture::capture_window_number(number)
        .ok_or_else(|| format!("the window server returned no image for window {number}"))?;
    let rgba = frame
        .bgra
        .chunks_exact(4)
        .flat_map(|px| [px[2], px[1], px[0], px[3]])
        .collect();
    RgbaImage::from_raw(frame.width, frame.height, rgba)
        .ok_or_else(|| "captured pixels do not fit".into())
}

#[cfg(windows)]
fn grab(window: &mut gpui::Window) -> Result<RgbaImage, String> {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetDC, ReleaseDC, SelectObject,
    };
    use windows_sys::Win32::Storage::Xps::{PW_CLIENTONLY, PrintWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, PW_RENDERFULLCONTENT};

    let RawWindowHandle::Win32(handle) = window
        .window_handle()
        .map_err(|error| format!("this window has no handle: {error}"))?
        .as_raw()
    else {
        return Err("this window has no Win32 handle".into());
    };
    let hwnd = handle.hwnd.get() as HWND;
    let mut client = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: hwnd is gpui's live window, read on the thread that owns it.
    if unsafe { GetClientRect(hwnd, &mut client) } == 0 {
        return Err(format!(
            "GetClientRect failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let (width, height) = (client.right - client.left, client.bottom - client.top);
    if width <= 0 || height <= 0 {
        return Err(format!("the window's client area is {width}x{height}"));
    }
    // SAFETY: BITMAPINFO is plain data; all-zero is a valid empty header filled in below.
    let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..info.bmiHeader
    };
    let mut bits = std::ptr::null_mut();
    // SAFETY: every GDI object made here is selected out and freed before the block ends, and
    // the DIB's bits are copied out while the bitmap is still alive.
    let bgra = unsafe {
        let screen = GetDC(hwnd);
        let dc = CreateCompatibleDC(screen);
        let bitmap = CreateDIBSection(
            dc,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        let previous = SelectObject(dc, bitmap);
        // PW_RENDERFULLCONTENT is what makes PrintWindow read DirectComposition content; BitBlt
        // of the window DC returns black for it.
        let printed = !bitmap.is_null()
            && !bits.is_null()
            && PrintWindow(hwnd, dc, PW_CLIENTONLY | PW_RENDERFULLCONTENT) != 0;
        GdiFlush();
        let bgra = printed.then(|| {
            std::slice::from_raw_parts(bits.cast::<u8>(), (width * height * 4) as usize).to_vec()
        });
        SelectObject(dc, previous);
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        DeleteDC(dc);
        ReleaseDC(hwnd, screen);
        bgra
    }
    .ok_or("PrintWindow returned no image for this window")?;
    let rgba = bgra
        .chunks_exact(4)
        .flat_map(|px| [px[2], px[1], px[0], 255])
        .collect();
    let image = RgbaImage::from_raw(width as u32, height as u32, rgba)
        .ok_or("captured pixels do not fit")?;
    Ok(to_points(image, window.scale_factor()))
}

#[cfg(target_os = "linux")]
fn grab(window: &mut gpui::Window) -> Result<RgbaImage, String> {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    let xid = match window
        .window_handle()
        .map_err(|error| format!("this window has no handle: {error}"))?
        .as_raw()
    {
        RawWindowHandle::Xcb(handle) => handle.window.get(),
        RawWindowHandle::Xlib(handle) => {
            u32::try_from(handle.window).map_err(|_| "the X11 window id does not fit 32 bits")?
        }
        RawWindowHandle::Wayland(_) => {
            return Err("`snapshot` cannot read window pixels under Wayland: gpui has no renderer readback there and Wayland gives a client no way to read its own surface. Run Zeron under X11 or Xwayland (unset WAYLAND_DISPLAY) to take snapshots".into());
        }
        other => return Err(format!("`snapshot` cannot read a {other:?} window")),
    };
    let image = x11_window(xid)?;
    Ok(to_points(image, window.scale_factor()))
}

#[cfg(target_os = "linux")]
fn x11_window(xid: u32) -> Result<RgbaImage, String> {
    use x11rb::connection::Connection as _;
    use x11rb::image::{Image, PixelLayout};
    use x11rb::protocol::xproto::ConnectionExt as _;
    fn failed(error: impl std::fmt::Display) -> String {
        format!("X11 capture failed: {error}")
    }
    let (connection, _) = x11rb::connect(None).map_err(failed)?;
    let geometry = connection
        .get_geometry(xid)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let (image, visual_id) =
        Image::get(&connection, xid, 0, 0, geometry.width, geometry.height).map_err(failed)?;
    let visual = connection
        .setup()
        .roots
        .iter()
        .flat_map(|screen| &screen.allowed_depths)
        .flat_map(|depth| &depth.visuals)
        .find(|visual| visual.visual_id == visual_id)
        .copied()
        .ok_or("X11 returned an unknown visual")?;
    let layout = PixelLayout::from_visual_type(visual).map_err(failed)?;
    let mut rgba =
        Vec::with_capacity(usize::from(geometry.width) * usize::from(geometry.height) * 4);
    for y in 0..geometry.height {
        for x in 0..geometry.width {
            let (red, green, blue) = layout.decode(image.get_pixel(x, y));
            rgba.extend_from_slice(&[(red >> 8) as u8, (green >> 8) as u8, (blue >> 8) as u8, 255]);
        }
    }
    RgbaImage::from_raw(geometry.width.into(), geometry.height.into(), rgba)
        .ok_or_else(|| "captured pixels do not fit".into())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn grab(_: &mut gpui::Window) -> Result<RgbaImage, String> {
    Err("`snapshot` is only implemented on macOS, Windows and Linux".into())
}

#[cfg(any(windows, target_os = "linux"))]
fn to_points(image: RgbaImage, scale: f32) -> RgbaImage {
    if !scale.is_finite() || scale <= 0.0 || (scale - 1.0).abs() < 0.01 {
        return image;
    }
    let size = |pixels: u32| ((pixels as f32 / scale).round() as u32).max(1);
    let (width, height) = (size(image.width()), size(image.height()));
    image::imageops::resize(
        &image,
        width,
        height,
        image::imageops::FilterType::Triangle,
    )
}

fn check_path(path: &Path) -> Result<(), String> {
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

#[cfg(all(test, unix))]
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
