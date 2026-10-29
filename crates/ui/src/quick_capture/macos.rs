use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSRunningApplication, NSScreen, NSView, NSWorkspace,
};
use objc2_foundation::NSString;

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

// NSPanel hides on deactivation by default, taking the toast with it.
pub(crate) fn keep_on_deactivate(window: &gpui::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    if let Some(window) = view.window() {
        let _: () = unsafe { objc2::msg_send![&*window, setHidesOnDeactivate: false] };
    }
}
