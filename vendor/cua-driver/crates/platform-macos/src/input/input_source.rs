use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::data::{CFDataGetBytePtr, CFDataRef};
use core_foundation::string::{CFString, CFStringRef};

// HIToolbox aborts the process when two threads enter TIS/TSM at once, and
// tool calls run on concurrent blocking threads.
static TIS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const MAIN_QUEUE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(300);

#[link(name = "System", kind = "framework")]
extern "C" {
    static _dispatch_main_q: u8;
    fn dispatch_async_f(
        queue: *const std::ffi::c_void,
        context: *mut std::ffi::c_void,
        work: unsafe extern "C" fn(*mut std::ffi::c_void),
    );
}

type MainJob = Box<dyn FnOnce() + Send>;

unsafe extern "C" fn run_main_job(context: *mut std::ffi::c_void) {
    let job = unsafe { Box::from_raw(context.cast::<MainJob>()) };
    job();
}

// In a GUI session TSM asserts it runs on the main queue and traps otherwise;
// None means the main queue did not answer in time.
fn on_main_queue<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    if objc2_foundation::MainThreadMarker::new().is_some() {
        return Some(work());
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let job: MainJob = Box::new(move || {
        let _ = tx.send(work());
    });
    unsafe {
        dispatch_async_f(
            &raw const _dispatch_main_q as *const std::ffi::c_void,
            Box::into_raw(Box::new(job)).cast::<std::ffi::c_void>(),
            run_main_job,
        );
    }
    rx.recv_timeout(MAIN_QUEUE_TIMEOUT).ok()
}

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> CFTypeRef;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> CFTypeRef;
    fn TISGetInputSourceProperty(source: CFTypeRef, key: CFStringRef) -> CFTypeRef;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const u8,
        key_code: u16,
        key_action: u16,
        modifier_key_state: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: usize,
        actual_length: *mut usize,
        unicode_string: *mut u16,
    ) -> i32;
    static kTISPropertyInputSourceType: CFStringRef;
    static kTISPropertyInputSourceID: CFStringRef;
    static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
}

const KEY_ACTION_DOWN: u16 = 0;
const NO_DEAD_KEYS: u32 = 1;

pub(crate) fn is_plain_keyboard_layout(source_type: &str) -> bool {
    source_type == "TISTypeKeyboardLayout"
}

unsafe fn string_property(source: CFTypeRef, key: CFStringRef) -> Option<String> {
    let value = TISGetInputSourceProperty(source, key);
    (!value.is_null()).then(|| CFString::wrap_under_get_rule(value as CFStringRef).to_string())
}

pub(crate) fn active_input_method() -> Option<String> {
    on_main_queue(active_input_method_now).flatten()
}

fn active_input_method_now() -> Option<String> {
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

pub(crate) fn key_code_for_char(ch: char, translate: impl Fn(u16) -> Option<char>) -> Option<u16> {
    (0..128).find(|&code| translate(code) == Some(ch))
}

pub(crate) fn current_layout_key_code(ch: char) -> Option<u16> {
    match on_main_queue(move || current_layout_key_code_now(ch)) {
        Some(code) => code,
        None => super::keyboard::key_name_to_code(&ch.to_string()).ok(),
    }
}

fn current_layout_key_code_now(ch: char) -> Option<u16> {
    let _tis = TIS_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    unsafe {
        let source = TISCopyCurrentKeyboardLayoutInputSource();
        if source.is_null() {
            return None;
        }
        let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
        let code = (!data.is_null()).then(|| {
            let layout = CFDataGetBytePtr(data as CFDataRef);
            let keyboard_type = u32::from(LMGetKbdType());
            key_code_for_char(ch, |code| {
                let mut dead_key_state = 0;
                let mut length = 0;
                let mut buffer = [0u16; 4];
                let status = UCKeyTranslate(
                    layout,
                    code,
                    KEY_ACTION_DOWN,
                    0,
                    keyboard_type,
                    NO_DEAD_KEYS,
                    &mut dead_key_state,
                    buffer.len(),
                    &mut length,
                    buffer.as_mut_ptr(),
                );
                if status != 0 {
                    return None;
                }
                let mut decoded = char::decode_utf16(buffer[..length].iter().copied());
                match (decoded.next(), decoded.next()) {
                    (Some(Ok(ch)), None) => Some(ch),
                    _ => None,
                }
            })
        });
        CFRelease(source);
        code.flatten()
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
    fn reverse_lookup_follows_the_layout_not_the_us_table() {
        let azerty = |code: u16| match code {
            0 => Some('q'),
            6 => Some('w'),
            12 => Some('a'),
            13 => Some('z'),
            _ => None,
        };
        assert_eq!(key_code_for_char('a', azerty), Some(12));
        assert_eq!(key_code_for_char('q', azerty), Some(0));
        assert_eq!(key_code_for_char('z', azerty), Some(13));
        assert_eq!(key_code_for_char('w', azerty), Some(6));
        assert_eq!(key_code_for_char('m', azerty), None);
    }

    #[test]
    fn concurrent_input_source_reads_do_not_abort_the_process() {
        let readers: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..50 {
                        let _ = active_input_method_now();
                        assert_eq!(current_layout_key_code_now(' '), Some(49));
                    }
                })
            })
            .collect();
        for reader in readers {
            reader.join().unwrap();
        }
    }
}
