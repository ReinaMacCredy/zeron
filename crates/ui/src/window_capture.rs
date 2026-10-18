/// One window frame as the window server holds it: tightly packed BGRA at
/// nominal (1px per point) resolution — the byte order `RenderImage` consumes.
pub(crate) struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

#[cfg(target_os = "macos")]
pub(crate) fn capture_window_number(id: u32) -> Option<CapturedFrame> {
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};
    use core_graphics::window::{
        create_image, kCGWindowImageBoundsIgnoreFraming, kCGWindowImageNominalResolution,
        kCGWindowImageShouldBeOpaque, kCGWindowListOptionIncludingWindow,
    };
    // CGRectNull: the image takes the window's own bounds.
    let null_rect = CGRect::new(
        &CGPoint::new(f64::INFINITY, f64::INFINITY),
        &CGSize::new(0.0, 0.0),
    );
    // Opaque: the glass shell's translucent regions would otherwise carry
    // their alpha into the card and show the card's own fill through.
    let image = create_image(
        null_rect,
        kCGWindowListOptionIncludingWindow,
        id,
        kCGWindowImageNominalResolution
            | kCGWindowImageBoundsIgnoreFraming
            | kCGWindowImageShouldBeOpaque,
    )?;
    let (width, height) = (image.width(), image.height());
    let stride = image.bytes_per_row();
    if image.bits_per_pixel() != 32 || width == 0 || height == 0 {
        return None;
    }
    let data = image.data();
    let bytes = data.bytes();
    let mut bgra = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let start = row * stride;
        bgra.extend_from_slice(bytes.get(start..start + width * 4)?);
    }
    Some(CapturedFrame {
        width: width as u32,
        height: height as u32,
        bgra,
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn window_number(window: &gpui::Window) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // SAFETY: gpui's AppKit handle points at the window's live content view; we are on the main thread.
    let view = unsafe { handle.ns_view.cast::<objc2_app_kit::NSView>().as_ref() };
    u32::try_from(view.window()?.windowNumber()).ok()
}
