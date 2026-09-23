use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use block2::RcBlock;
use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation::base::{CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::dictionary::{CFDictionaryGetValue, CFDictionaryRef};
use core_foundation::number::{CFNumber, CFNumberRef};
use core_graphics::access::ScreenCaptureAccess;
use core_graphics::color_space::kCGColorSpaceSRGB;
use dispatch2::{DispatchQueue, DispatchRetained};
use image::ExtendedColorType;
use image::codecs::jpeg::JpegEncoder;
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_core_video::{
    CVBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol, NSRect};

use super::frame_tap::{FrameSource, StreamEvents, StreamFailure, StreamSignal};
use super::window_info;

pub const PREVIEW_MAX_DIMENSION: f64 = 960.0;
pub const PREVIEW_FPS: i32 = 15;
const SCALE_FACTOR: f64 = 2.0;
const JPEG_QUALITY: u8 = 70;
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(6);
const WINDOW_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const PIXEL_FORMAT_BGRA: u32 = 0x4247_5241;
const OUTPUT_TYPE_SCREEN: isize = 0;
const FRAME_STATUS_COMPLETE: i64 = 0;

pub fn stream_size(width: f64, height: f64) -> (isize, isize) {
    let longest = width.max(height).max(1.0);
    let ratio = SCALE_FACTOR.min(PREVIEW_MAX_DIMENSION / longest);
    (
        ((width * ratio).round() as isize).max(2),
        ((height * ratio).round() as isize).max(2),
    )
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

unsafe impl Encode for CMTime {
    const ENCODING: Encoding = Encoding::Struct(
        "?",
        &[i64::ENCODING, i32::ENCODING, u32::ENCODING, i64::ENCODING],
    );
}

pub fn frame_interval(fps: i32) -> CMTime {
    CMTime {
        value: 1,
        timescale: fps.max(1),
        flags: 1,
        epoch: 0,
    }
}

#[derive(Default)]
pub struct EncodeGate(AtomicBool);

impl EncodeGate {
    pub fn try_begin(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }

    pub fn finish(&self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowCheck {
    Gone,
    Resized(f64, f64),
    Unchanged,
}

pub fn check_window(
    entry: Option<(i32, f64, f64)>,
    owner_pid: u32,
    current: (f64, f64),
) -> WindowCheck {
    match entry {
        Some((pid, width, height))
            if u32::try_from(pid).is_ok_and(|pid| pid == owner_pid)
                && width >= 2.0
                && height >= 2.0 =>
        {
            if (width, height) == current {
                WindowCheck::Unchanged
            } else {
                WindowCheck::Resized(width, height)
            }
        }
        _ => WindowCheck::Gone,
    }
}

pub fn encode_bgra_jpeg(
    width: usize,
    height: usize,
    bytes_per_row: usize,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    let row = width.checked_mul(4).ok_or("frame too wide")?;
    let needed = bytes_per_row.checked_mul(height).ok_or("frame too large")?;
    if width == 0 || height == 0 || bytes_per_row < row || data.len() < needed {
        return Err("The captured frame has an unexpected layout.".into());
    }
    let mut rgb = Vec::with_capacity(width * height * 3);
    for line in data.chunks(bytes_per_row).take(height) {
        for pixel in line[..row].chunks_exact(4) {
            rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
    }
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode(&rgb, width as u32, height as u32, ExtendedColorType::Rgb8)
        .map_err(|error| error.to_string())?;
    Ok(jpeg)
}

#[repr(C)]
struct OpaqueCFString {
    _private: [u8; 0],
}

unsafe impl RefEncode for OpaqueCFString {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CFString", &[]));
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMSampleBufferGetImageBuffer(sample: *const c_void) -> *mut CVBuffer;
    fn CMSampleBufferGetSampleAttachmentsArray(sample: *const c_void, create: u8) -> CFArrayRef;
}

fn load_screen_capture_kit() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| unsafe {
        !libc::dlopen(
            c"/System/Library/Frameworks/ScreenCaptureKit.framework/ScreenCaptureKit".as_ptr(),
            libc::RTLD_LAZY,
        )
        .is_null()
    })
}

fn frame_status_key() -> Option<CFTypeRef> {
    static KEY: OnceLock<usize> = OnceLock::new();
    let key = *KEY.get_or_init(|| unsafe {
        let symbol = libc::dlsym(libc::RTLD_DEFAULT, c"SCStreamFrameInfoStatus".as_ptr());
        if symbol.is_null() {
            0
        } else {
            *(symbol as *const usize)
        }
    });
    (key != 0).then_some(key as CFTypeRef)
}

fn frame_complete(sample: *const c_void) -> bool {
    let Some(key) = frame_status_key() else {
        return true;
    };
    unsafe {
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, 0);
        if attachments.is_null() || CFArrayGetCount(attachments) < 1 {
            return false;
        }
        let first = CFArrayGetValueAtIndex(attachments, 0) as CFDictionaryRef;
        if first.is_null() {
            return false;
        }
        let status = CFDictionaryGetValue(first, key);
        if status.is_null() {
            return false;
        }
        CFNumber::wrap_under_get_rule(status as CFNumberRef).to_i64() == Some(FRAME_STATUS_COMPLETE)
    }
}

struct Pixels(NonNull<CVBuffer>);

unsafe impl Send for Pixels {}

impl Pixels {
    fn retain(pixels: NonNull<CVBuffer>) -> Self {
        unsafe { CFRetain(pixels.as_ptr() as CFTypeRef) };
        Self(pixels)
    }

    fn encode(&self) -> Result<Vec<u8>, String> {
        let pixels = unsafe { self.0.as_ref() };
        if CVPixelBufferGetPixelFormatType(pixels) != PIXEL_FORMAT_BGRA {
            return Err("The captured frame is not BGRA.".into());
        }
        if unsafe { CVPixelBufferLockBaseAddress(pixels, CVPixelBufferLockFlags::ReadOnly) } != 0 {
            return Err("The captured frame could not be read.".into());
        }
        let base = CVPixelBufferGetBaseAddress(pixels);
        let width = CVPixelBufferGetWidth(pixels);
        let height = CVPixelBufferGetHeight(pixels);
        let bytes_per_row = CVPixelBufferGetBytesPerRow(pixels);
        let result = if base.is_null() {
            Err("The captured frame has no pixels.".into())
        } else {
            let data =
                unsafe { std::slice::from_raw_parts(base as *const u8, bytes_per_row * height) };
            encode_bgra_jpeg(width, height, bytes_per_row, data)
        };
        unsafe { CVPixelBufferUnlockBaseAddress(pixels, CVPixelBufferLockFlags::ReadOnly) };
        result
    }
}

impl Drop for Pixels {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0.as_ptr() as CFTypeRef) };
    }
}

struct Capture {
    stream: Retained<AnyObject>,
    _output: Retained<FrameOutput>,
}

unsafe impl Send for Capture {}

struct StreamShared {
    window_id: u32,
    owner_pid: u32,
    events: StreamEvents,
    stopped: AtomicBool,
    received: AtomicBool,
    gate: EncodeGate,
    encoder: Mutex<Option<SyncSender<Pixels>>>,
    capture: Mutex<Option<Capture>>,
    source_size: Mutex<(f64, f64)>,
    sample_queue: DispatchRetained<DispatchQueue>,
}

impl StreamShared {
    fn stop(&self) -> bool {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.encoder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let capture = self
            .capture
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(capture) = capture {
            let done = RcBlock::new(|_: *mut NSError| {});
            unsafe {
                let _: () = msg_send![&*capture.stream, stopCaptureWithCompletionHandler: &*done];
            }
        }
        true
    }

    fn fail(&self, code: &'static str, message: impl Into<String>) {
        if self.stop() {
            (self.events)(StreamSignal::Failed(StreamFailure::new(code, message)));
        }
    }

    fn receive(&self, sample: *mut c_void) {
        if self.stopped.load(Ordering::Acquire) || !frame_complete(sample) {
            return;
        }
        let Some(pixels) = NonNull::new(unsafe { CMSampleBufferGetImageBuffer(sample) }) else {
            return;
        };
        self.received.store(true, Ordering::Release);
        if !self.gate.try_begin() {
            return;
        }
        let sent = self
            .encoder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|encoder| encoder.try_send(Pixels::retain(pixels)));
        if !matches!(sent, Some(Ok(()))) {
            self.gate.finish();
            if let Some(Err(TrySendError::Disconnected(_))) = sent {
                self.fail(
                    "frame_encode_failed",
                    "Could not encode a captured frame as JPEG.",
                );
            }
        }
    }
}

fn error_text(error: Option<&NSError>, fallback: &str) -> String {
    error
        .map(|error| error.localizedDescription().to_string())
        .unwrap_or_else(|| fallback.into())
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ZeronComputerFrameOutput"]
    #[ivars = Arc<StreamShared>]
    struct FrameOutput;

    unsafe impl NSObjectProtocol for FrameOutput {}

    impl FrameOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _stream: &AnyObject, sample: *mut c_void, kind: isize) {
            if kind == OUTPUT_TYPE_SCREEN {
                self.ivars().receive(sample);
            }
        }

        #[unsafe(method(stream:didStopWithError:))]
        fn did_stop(&self, _stream: &AnyObject, error: Option<&NSError>) {
            let message = error_text(error, "The preview stream stopped.");
            self.ivars().fail("capture_stopped", message);
        }
    }
);

impl FrameOutput {
    fn new(shared: Arc<StreamShared>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(shared);
        unsafe { msg_send![super(this), init] }
    }
}

fn class(name: &str) -> Result<&'static AnyClass, StreamFailure> {
    let name = std::ffi::CString::new(name).map_err(|_| setup_failed("bad class name"))?;
    AnyClass::get(&name).ok_or_else(|| setup_failed("ScreenCaptureKit is unavailable on this Mac."))
}

fn setup_failed(message: &str) -> StreamFailure {
    StreamFailure::new("capture_setup_failed", message)
}

pub fn configuration(width: f64, height: f64) -> Result<Retained<AnyObject>, StreamFailure> {
    if !load_screen_capture_kit() {
        return Err(setup_failed("ScreenCaptureKit is unavailable on this Mac."));
    }
    let class = class("SCStreamConfiguration")?;
    let configuration: Retained<AnyObject> = unsafe { msg_send![class, new] };
    let (width, height) = stream_size(width, height);
    let color_space = unsafe { kCGColorSpaceSRGB } as *const OpaqueCFString;
    unsafe {
        let _: () = msg_send![&*configuration, setWidth: width];
        let _: () = msg_send![&*configuration, setHeight: height];
        let _: () =
            msg_send![&*configuration, setMinimumFrameInterval: frame_interval(PREVIEW_FPS)];
        let _: () = msg_send![&*configuration, setPixelFormat: PIXEL_FORMAT_BGRA];
        let _: () = msg_send![&*configuration, setQueueDepth: 2isize];
        let _: () = msg_send![&*configuration, setScalesToFit: Bool::YES];
        let _: () = msg_send![&*configuration, setShowsCursor: Bool::NO];
        let _: () = msg_send![&*configuration, setColorSpaceName: color_space];
    }
    Ok(configuration)
}

fn find_window(
    content: &AnyObject,
    window_id: u32,
    owner_pid: u32,
) -> Option<(Retained<AnyObject>, NSRect)> {
    let windows: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![content, windows] };
    windows?.iter().find_map(|window| {
        let id: u32 = unsafe { msg_send![&*window, windowID] };
        if id != window_id {
            return None;
        }
        let owner: Option<Retained<AnyObject>> = unsafe { msg_send![&*window, owningApplication] };
        let owner = owner?;
        let pid: i32 = unsafe { msg_send![&*owner, processID] };
        if u32::try_from(pid).ok()? != owner_pid {
            return None;
        }
        let frame: NSRect = unsafe { msg_send![&*window, frame] };
        Some((window, frame))
    })
}

fn open_stream(shared: &Arc<StreamShared>, content: &AnyObject) -> Result<(), StreamFailure> {
    let (window, frame) =
        find_window(content, shared.window_id, shared.owner_pid).ok_or_else(|| {
            StreamFailure::new(
                "target_unavailable",
                "The selected window is no longer available.",
            )
        })?;
    if frame.size.width < 2.0 || frame.size.height < 2.0 {
        return Err(StreamFailure::new(
            "invalid_window_dimensions",
            "The selected window has no capturable area.",
        ));
    }
    *shared
        .source_size
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = (frame.size.width, frame.size.height);
    let configuration = configuration(frame.size.width, frame.size.height)?;
    let filter: Allocated<AnyObject> = unsafe { msg_send![class("SCContentFilter")?, alloc] };
    let filter: Retained<AnyObject> =
        unsafe { msg_send![filter, initWithDesktopIndependentWindow: &*window] };
    let output = FrameOutput::new(shared.clone());
    let stream: Allocated<AnyObject> = unsafe { msg_send![class("SCStream")?, alloc] };
    let stream: Retained<AnyObject> = unsafe {
        msg_send![
            stream,
            initWithFilter: &*filter,
            configuration: &*configuration,
            delegate: &*output
        ]
    };
    let added: Result<(), Retained<NSError>> = unsafe {
        msg_send![
            &*stream,
            addStreamOutput: &*output,
            type: OUTPUT_TYPE_SCREEN,
            sampleHandlerQueue: &*shared.sample_queue,
            error: _
        ]
    };
    added.map_err(|error| {
        StreamFailure::new(
            "capture_setup_failed",
            error.localizedDescription().to_string(),
        )
    })?;
    let weak = Arc::downgrade(shared);
    let started = RcBlock::new(move |error: *mut NSError| {
        if let (Some(shared), Some(error)) = (weak.upgrade(), unsafe { error.as_ref() }) {
            shared.fail(
                "capture_start_failed",
                error_text(Some(error), "capture failed"),
            );
        }
    });
    unsafe {
        let _: () = msg_send![&*stream, startCaptureWithCompletionHandler: &*started];
    }
    let mut capture = shared
        .capture
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if shared.stopped.load(Ordering::Acquire) {
        let done = RcBlock::new(|_: *mut NSError| {});
        unsafe {
            let _: () = msg_send![&*stream, stopCaptureWithCompletionHandler: &*done];
        }
        return Ok(());
    }
    *capture = Some(Capture {
        stream,
        _output: output,
    });
    Ok(())
}

fn update_size(shared: &Arc<StreamShared>, width: f64, height: f64) {
    *shared
        .source_size
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = (width, height);
    let Ok(configuration) = configuration(width, height) else {
        return;
    };
    let stream = shared
        .capture
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .map(|capture| capture.stream.clone());
    let Some(stream) = stream else {
        return;
    };
    let weak = Arc::downgrade(shared);
    let updated = RcBlock::new(move |error: *mut NSError| {
        if let (Some(shared), Some(error)) = (weak.upgrade(), unsafe { error.as_ref() }) {
            shared.fail(
                "capture_configuration_failed",
                error_text(Some(error), "configuration failed"),
            );
        }
    });
    unsafe {
        let _: () = msg_send![
            &*stream,
            updateConfiguration: &*configuration,
            completionHandler: &*updated
        ];
    }
}

fn watch_window(shared: Weak<StreamShared>) {
    let started = Instant::now();
    loop {
        std::thread::sleep(WINDOW_CHECK_INTERVAL);
        let Some(shared) = shared.upgrade() else {
            return;
        };
        if shared.stopped.load(Ordering::Acquire) {
            return;
        }
        if !shared.received.load(Ordering::Acquire) && started.elapsed() >= FIRST_FRAME_TIMEOUT {
            shared.fail(
                "capture_timed_out",
                "The selected window did not deliver a frame.",
            );
            return;
        }
        let entry = window_info::window(shared.window_id)
            .map(|entry| (entry.pid, entry.bounds.size.width, entry.bounds.size.height));
        let current = *shared
            .source_size
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match check_window(entry, shared.owner_pid, current) {
            WindowCheck::Gone => {
                shared.fail(
                    "target_unavailable",
                    "The selected window was closed. Waiting for the next target.",
                );
                return;
            }
            WindowCheck::Resized(width, height) => {
                objc2::rc::autoreleasepool(|_| update_size(&shared, width, height));
            }
            WindowCheck::Unchanged => {}
        }
    }
}

fn run_encoder(shared: Weak<StreamShared>, frames: Receiver<Pixels>) {
    for pixels in frames {
        let Some(shared) = shared.upgrade() else {
            return;
        };
        let encoded = pixels.encode();
        drop(pixels);
        shared.gate.finish();
        if shared.stopped.load(Ordering::Acquire) {
            return;
        }
        match encoded {
            Ok(jpeg) => (shared.events)(StreamSignal::Frame(jpeg)),
            Err(error) => {
                tracing::debug!(target: "desktop_cua", "preview frame encode failed: {error}");
                shared.fail(
                    "frame_encode_failed",
                    "Could not encode a captured frame as JPEG.",
                );
                return;
            }
        }
    }
}

pub struct WindowFrameStream {
    shared: Arc<StreamShared>,
}

impl WindowFrameStream {
    pub fn start(
        window_id: u32,
        owner_pid: u32,
        events: StreamEvents,
    ) -> Result<Self, StreamFailure> {
        if !load_screen_capture_kit() {
            return Err(setup_failed("ScreenCaptureKit is unavailable on this Mac."));
        }
        if !ScreenCaptureAccess.preflight() {
            return Err(StreamFailure::new(
                "screen_recording_required",
                "Screen Recording is required for the preview. Enable it in Zeron’s permission setup.",
            ));
        }
        let (encoder, frames) = std::sync::mpsc::sync_channel(1);
        let shared = Arc::new(StreamShared {
            window_id,
            owner_pid,
            events,
            stopped: AtomicBool::new(false),
            received: AtomicBool::new(false),
            gate: EncodeGate::default(),
            encoder: Mutex::new(Some(encoder)),
            capture: Mutex::new(None),
            source_size: Mutex::new((0.0, 0.0)),
            sample_queue: DispatchQueue::new("sh.zeron.computer-frame-tap", None),
        });
        let spawn = |name: &str, work: Box<dyn FnOnce() + Send>| {
            std::thread::Builder::new()
                .name(name.into())
                .spawn(work)
                .map_err(|error| setup_failed(&error.to_string()))
        };
        let weak = Arc::downgrade(&shared);
        spawn(
            "zeron-cua-frame-encode",
            Box::new(move || run_encoder(weak, frames)),
        )?;
        let weak = Arc::downgrade(&shared);
        spawn(
            "zeron-cua-frame-watch",
            Box::new(move || watch_window(weak)),
        )?;
        let content_class = class("SCShareableContent")?;
        let weak = Arc::downgrade(&shared);
        let enumerated = RcBlock::new(move |content: *mut AnyObject, error: *mut NSError| {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            if shared.stopped.load(Ordering::Acquire) {
                return;
            }
            objc2::rc::autoreleasepool(|_| {
                let Some(content) = (unsafe { content.as_ref() }) else {
                    let message = error_text(
                        unsafe { error.as_ref() },
                        "The selected window is no longer available.",
                    );
                    shared.fail("target_unavailable", message);
                    return;
                };
                if let Err(failure) = open_stream(&shared, content) {
                    shared.fail(failure.code, failure.message);
                }
            });
        });
        unsafe {
            let _: () = msg_send![
                content_class,
                getShareableContentExcludingDesktopWindows: Bool::YES,
                onScreenWindowsOnly: Bool::NO,
                completionHandler: &*enumerated
            ];
        }
        Ok(Self { shared })
    }

    pub fn stop(&self) {
        self.shared.stop();
    }
}

impl Drop for WindowFrameStream {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

pub struct ScreenCaptureFrames;

impl FrameSource for ScreenCaptureFrames {
    type Stream = WindowFrameStream;

    fn start(
        &self,
        pid: u32,
        window_id: u32,
        events: StreamEvents,
    ) -> Result<WindowFrameStream, StreamFailure> {
        WindowFrameStream::start(window_id, pid, events)
    }

    fn stop(&self, stream: WindowFrameStream) {
        stream.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_fit_the_960_pixel_long_edge_without_upscaling_past_2x() {
        assert_eq!(stream_size(1920.0, 1080.0), (960, 540));
        assert_eq!(stream_size(1080.0, 1920.0), (540, 960));
        assert_eq!(stream_size(300.0, 200.0), (600, 400));
        assert_eq!(stream_size(480.0, 100.0), (960, 200));
        assert_eq!(stream_size(1.0, 1.0), (2, 2));
        assert_eq!(stream_size(10_000.0, 3.0), (960, 2));
    }

    #[test]
    fn the_stream_is_paced_at_fifteen_frames_a_second() {
        assert_eq!(
            frame_interval(PREVIEW_FPS),
            CMTime {
                value: 1,
                timescale: 15,
                flags: 1,
                epoch: 0
            }
        );
        assert_eq!(frame_interval(0).timescale, 1);
    }

    #[test]
    fn one_encode_in_flight_drops_the_frames_behind_it() {
        let gate = EncodeGate::default();
        assert!(gate.try_begin());
        assert!(!gate.try_begin(), "a frame arriving mid-encode is dropped");
        gate.finish();
        assert!(gate.try_begin());
    }

    #[test]
    fn a_closed_or_foreign_window_ends_the_stream() {
        assert_eq!(check_window(None, 42, (100.0, 100.0)), WindowCheck::Gone);
        assert_eq!(
            check_window(Some((7, 100.0, 100.0)), 42, (100.0, 100.0)),
            WindowCheck::Gone
        );
        assert_eq!(
            check_window(Some((42, 1.0, 100.0)), 42, (100.0, 100.0)),
            WindowCheck::Gone
        );
        assert_eq!(
            check_window(Some((42, 100.0, 100.0)), 42, (100.0, 100.0)),
            WindowCheck::Unchanged
        );
        assert_eq!(
            check_window(Some((42, 200.0, 100.0)), 42, (100.0, 100.0)),
            WindowCheck::Resized(200.0, 100.0)
        );
    }

    #[test]
    fn bgra_rows_encode_to_a_jpeg_of_the_same_size() {
        let (width, height, stride) = (5usize, 3usize, 24usize);
        let mut data = vec![0u8; stride * height];
        for row in 0..height {
            for column in 0..width {
                let at = row * stride + column * 4;
                data[at..at + 4].copy_from_slice(&[0, 0, 255, 255]);
            }
        }
        let jpeg = encode_bgra_jpeg(width, height, stride, &data).unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap().to_rgb8();
        assert_eq!((decoded.width(), decoded.height()), (5, 3));
        let pixel = decoded.get_pixel(2, 1);
        assert!(
            pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60,
            "{pixel:?}"
        );
        assert!(
            encode_bgra_jpeg(5, 3, 16, &data).is_err(),
            "stride shorter than a row"
        );
        assert!(encode_bgra_jpeg(5, 3, stride, &data[..10]).is_err());
    }

    #[test]
    fn the_stream_configuration_matches_the_runtime_signatures() {
        let configuration = configuration(1920.0, 1080.0).unwrap();
        let width: isize = unsafe { msg_send![&*configuration, width] };
        let height: isize = unsafe { msg_send![&*configuration, height] };
        let interval: CMTime = unsafe { msg_send![&*configuration, minimumFrameInterval] };
        let pixel_format: u32 = unsafe { msg_send![&*configuration, pixelFormat] };
        assert_eq!((width, height), (960, 540));
        assert_eq!(interval, frame_interval(PREVIEW_FPS));
        assert_eq!(pixel_format, PIXEL_FORMAT_BGRA);
    }

    #[test]
    fn every_screen_capture_kit_selector_matches_its_runtime_signature() {
        use block2::Block;
        use objc2::sel;
        type Done = Block<dyn Fn(*mut NSError)>;
        assert!(load_screen_capture_kit());
        let content = class("SCShareableContent").unwrap().metaclass();
        content
            .verify_sel::<(Bool, Bool, &Block<dyn Fn(*mut AnyObject, *mut NSError)>), ()>(sel!(
                getShareableContentExcludingDesktopWindows:onScreenWindowsOnly:completionHandler:
            ))
            .unwrap();
        let window = class("SCWindow").unwrap();
        window.verify_sel::<(), u32>(sel!(windowID)).unwrap();
        window.verify_sel::<(), NSRect>(sel!(frame)).unwrap();
        window
            .verify_sel::<(), *mut AnyObject>(sel!(owningApplication))
            .unwrap();
        class("SCRunningApplication")
            .unwrap()
            .verify_sel::<(), i32>(sel!(processID))
            .unwrap();
        class("SCContentFilter")
            .unwrap()
            .verify_sel::<(&AnyObject,), *mut AnyObject>(sel!(initWithDesktopIndependentWindow:))
            .unwrap();
        let stream = class("SCStream").unwrap();
        stream
            .verify_sel::<(&AnyObject, &AnyObject, &AnyObject), *mut AnyObject>(sel!(
                initWithFilter:configuration:delegate:
            ))
            .unwrap();
        stream
            .verify_sel::<(&AnyObject, isize, &DispatchQueue, *mut *mut NSError), Bool>(sel!(
                addStreamOutput:type:sampleHandlerQueue:error:
            ))
            .unwrap();
        stream
            .verify_sel::<(&Done,), ()>(sel!(startCaptureWithCompletionHandler:))
            .unwrap();
        stream
            .verify_sel::<(&Done,), ()>(sel!(stopCaptureWithCompletionHandler:))
            .unwrap();
        stream
            .verify_sel::<(&AnyObject, &Done), ()>(sel!(updateConfiguration:completionHandler:))
            .unwrap();
    }

    #[test]
    fn the_frame_output_answers_the_stream_protocol_selectors() {
        use objc2::{ClassType, sel};
        let output = FrameOutput::class();
        output
            .verify_sel::<(&AnyObject, *mut c_void, isize), ()>(sel!(
                stream:didOutputSampleBuffer:ofType:
            ))
            .unwrap();
        output
            .verify_sel::<(&AnyObject, Option<&NSError>), ()>(sel!(stream:didStopWithError:))
            .unwrap();
        assert!(
            frame_status_key().is_some(),
            "SCStreamFrameInfoStatus resolves"
        );
    }

    #[test]
    #[ignore = "needs Screen Recording for the test runner and an on-screen window"]
    fn live_stream_delivers_a_bounded_jpeg_of_one_window() {
        let window = window_info::on_screen_windows()
            .into_iter()
            .find(|window| window.layer == 0 && window.bounds.size.width > 100.0 && window.pid > 0)
            .expect("an on-screen window");
        let (sender, frames) = std::sync::mpsc::channel();
        let sender = Mutex::new(sender);
        let stream = WindowFrameStream::start(
            window.id,
            window.pid as u32,
            Arc::new(move |signal| {
                let _ = sender.lock().unwrap().send(match signal {
                    StreamSignal::Frame(jpeg) => Ok(jpeg),
                    StreamSignal::Failed(failure) => Err(failure),
                });
            }),
        )
        .unwrap();
        let jpeg = frames
            .recv_timeout(Duration::from_secs(8))
            .expect("a frame or failure within 8 s")
            .expect("a frame");
        stream.stop();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        let (width, height) = stream_size(window.bounds.size.width, window.bounds.size.height);
        assert!(
            (decoded.width() as isize).abs_diff(width) <= 2
                && (decoded.height() as isize).abs_diff(height) <= 2,
            "{}x{} for a {width}x{height} request",
            decoded.width(),
            decoded.height()
        );
        assert!(decoded.width().max(decoded.height()) <= PREVIEW_MAX_DIMENSION as u32);
    }
}
