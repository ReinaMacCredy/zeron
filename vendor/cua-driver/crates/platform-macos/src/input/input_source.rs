use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};

// HIToolbox aborts the process when two threads enter TIS/TSM at once, and
// tool calls run on concurrent blocking threads.
static TIS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> CFTypeRef;
    fn TISGetInputSourceProperty(source: CFTypeRef, key: CFStringRef) -> CFTypeRef;
    static kTISPropertyInputSourceType: CFStringRef;
    static kTISPropertyInputSourceID: CFStringRef;
}

pub(crate) fn is_plain_keyboard_layout(source_type: &str) -> bool {
    source_type == "TISTypeKeyboardLayout"
}

unsafe fn string_property(source: CFTypeRef, key: CFStringRef) -> Option<String> {
    let value = TISGetInputSourceProperty(source, key);
    (!value.is_null()).then(|| CFString::wrap_under_get_rule(value as CFStringRef).to_string())
}

pub(crate) fn active_input_method() -> Option<String> {
    let _tis = TIS_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    unsafe {
        let source = TISCopyCurrentKeyboardInputSource();
        if source.is_null() {
            return None;
        }
        let source_type = string_property(source, kTISPropertyInputSourceType);
        let id = string_property(source, kTISPropertyInputSourceID);
        CFRelease(source);
        match source_type {
            Some(source_type) if !is_plain_keyboard_layout(&source_type) => {
                Some(id.unwrap_or(source_type))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_plain_keyboard_layout_passes_key_event_text_through_unchanged() {
        assert!(is_plain_keyboard_layout("TISTypeKeyboardLayout"));
        for input_method in [
            "TISTypeKeyboardInputMode",
            "TISTypeKeyboardInputMethodModeEnabled",
            "TISTypeKeyboardInputMethodWithoutModes",
        ] {
            assert!(!is_plain_keyboard_layout(input_method), "{input_method}");
        }
    }

    #[test]
    fn concurrent_input_source_reads_do_not_abort_the_process() {
        let readers: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..50 {
                        let _ = active_input_method();
                    }
                })
            })
            .collect();
        for reader in readers {
            reader.join().unwrap();
        }
    }
}
