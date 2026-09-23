use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFStringRef;
use core_graphics::geometry::{CGPoint, CGRect};
use core_graphics::window::{
    CGWindowID, CGWindowListOption, copy_window_info, kCGNullWindowID, kCGWindowBounds,
    kCGWindowLayer, kCGWindowListExcludeDesktopElements, kCGWindowListOptionIncludingWindow,
    kCGWindowListOptionOnScreenOnly, kCGWindowNumber, kCGWindowOwnerPID,
};

#[derive(Debug, Clone, Copy)]
pub struct WindowEntry {
    pub id: u32,
    pub pid: i32,
    pub layer: i32,
    pub bounds: CGRect,
}

fn number(dict: &CFDictionary, key: CFStringRef) -> Option<CFNumber> {
    let value = dict.find(key.cast::<std::ffi::c_void>())?;
    unsafe { CFType::wrap_under_get_rule(*value as CFTypeRef) }.downcast::<CFNumber>()
}

fn rect(dict: &CFDictionary, key: CFStringRef) -> Option<CGRect> {
    let value = dict.find(key.cast::<std::ffi::c_void>())?;
    unsafe { CFType::wrap_under_get_rule(*value as CFTypeRef) }
        .downcast::<CFDictionary>()
        .and_then(|bounds| CGRect::from_dict_representation(&bounds))
}

fn entries(option: CGWindowListOption, relative: CGWindowID) -> Vec<WindowEntry> {
    let Some(list) = copy_window_info(option, relative) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            let dict = unsafe { CFType::wrap_under_get_rule(*item as CFTypeRef) }
                .downcast::<CFDictionary>()?;
            Some(WindowEntry {
                id: number(&dict, unsafe { kCGWindowNumber })?
                    .to_i64()
                    .and_then(|id| u32::try_from(id).ok())?,
                pid: number(&dict, unsafe { kCGWindowOwnerPID })?.to_i32()?,
                layer: number(&dict, unsafe { kCGWindowLayer })?.to_i32()?,
                bounds: rect(&dict, unsafe { kCGWindowBounds })?,
            })
        })
        .collect()
}

pub fn on_screen_windows() -> Vec<WindowEntry> {
    entries(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    )
}

pub fn window(id: u32) -> Option<WindowEntry> {
    entries(kCGWindowListOptionIncludingWindow, id)
        .into_iter()
        .find(|entry| entry.id == id)
}

pub fn window_at(windows: &[WindowEntry], point: CGPoint) -> Option<(u32, u32)> {
    windows
        .iter()
        .find(|window| window.layer == 0 && window.pid > 0 && window.bounds.contains(&point))
        .map(|window| (window.pid as u32, window.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_graphics::geometry::CGSize;

    fn entry(id: u32, pid: i32, layer: i32, x: f64) -> WindowEntry {
        WindowEntry {
            id,
            pid,
            layer,
            bounds: CGRect::new(&CGPoint::new(x, 0.0), &CGSize::new(100.0, 100.0)),
        }
    }

    #[test]
    fn pointer_attribution_skips_overlays_and_unknown_owners() {
        let windows = [
            entry(1, 10, 25, 0.0),
            entry(2, 0, 0, 0.0),
            entry(3, 30, 0, 200.0),
            entry(4, 40, 0, 0.0),
            entry(5, 50, 0, 0.0),
        ];
        assert_eq!(window_at(&windows, CGPoint::new(50.0, 50.0)), Some((40, 4)));
        assert_eq!(
            window_at(&windows, CGPoint::new(250.0, 50.0)),
            Some((30, 3))
        );
        assert_eq!(window_at(&windows, CGPoint::new(500.0, 50.0)), None);
    }
}
