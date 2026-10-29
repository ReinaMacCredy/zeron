use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSRunningApplication, NSScreen, NSView, NSWindow, NSWorkspace,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub(crate) fn frontmost_pid() -> Option<i32> {
    Some(
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()?
            .processIdentifier(),
    )
}

pub(crate) fn activate(pid: i32) {
    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        app.activateWithOptions(NSApplicationActivationOptions::empty());
    }
}

pub(crate) fn active_display() -> Option<u64> {
    let screen = NSScreen::mainScreen(MainThreadMarker::new()?)?;
    let number: Option<Retained<AnyObject>> = unsafe {
        objc2::msg_send![
            &*screen.deviceDescription(),
            objectForKey: &*NSString::from_str("NSScreenNumber")
        ]
    };
    let id: u32 = unsafe { objc2::msg_send![&*number?, unsignedIntValue] };
    Some(id.into())
}

fn ns_window(window: &gpui::Window) -> Option<Retained<NSWindow>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    view.window()
}

pub(crate) fn style_panel(window: &gpui::Window, radius: f64) {
    let Some(panel) = ns_window(window) else {
        return;
    };
    // NSPanel hides on deactivation by default, taking the toast with it.
    unsafe {
        let _: () = objc2::msg_send![&*panel, setHidesOnDeactivate: false];
    }
    panel.setHasShadow(true);
    if let Some(content) = panel.contentView() {
        content.setWantsLayer(true);
        let layer: Option<Retained<AnyObject>> = unsafe { objc2::msg_send![&*content, layer] };
        if let Some(layer) = layer {
            unsafe {
                let _: () = objc2::msg_send![&*layer, setCornerRadius: radius];
                let _: () = objc2::msg_send![&*layer, setMasksToBounds: true];
            }
        }
    }
    panel.invalidateShadow();
}

pub(crate) fn resize_panel(window: &gpui::Window) -> Option<impl FnOnce(f64, f64) + 'static> {
    let panel = ns_window(window)?;
    Some(move |width: f64, height: f64| {
        let frame = panel.frame();
        let top = frame.origin.y + frame.size.height;
        let center = frame.origin.x + frame.size.width / 2.0;
        let next = NSRect::new(
            NSPoint::new(center - width / 2.0, top - height),
            NSSize::new(width, height),
        );
        panel.setFrame_display(next, true);
        panel.invalidateShadow();
    })
}
