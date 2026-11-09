//! A Device tab's frames turned into pictures, off the window's thread, as
//! gpui's device_panel (stream_view.rs and h264.rs): H.264 on macOS through
//! VideoToolbox, MJPEG elsewhere and as the fallback, each picture turned to
//! the device's orientation (gpui's `rotate_for_display`).

use zeron_proto::device_hub::{AVCC_DELTA, AVCC_KEYFRAME, DeviceOrientation};

pub(crate) const MAX_PENDING_FRAMES: usize = 8;
pub(crate) const MAX_DECODE_ERRORS: u32 = 3;
pub(crate) const BACKLOG_WINDOW: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParameterSets {
    pub sps: Vec<Vec<u8>>,
    pub pps: Vec<Vec<u8>>,
    pub nal_length: usize,
}

pub(crate) fn parse_avcc(record: &[u8]) -> Option<ParameterSets> {
    if record.len() < 6 || record[0] != 1 {
        return None;
    }
    let mut at = 5;
    let sps_count = usize::from(record[at] & 0x1f);
    at += 1;
    let sps = read_parameter_sets(record, &mut at, sps_count)?;
    let pps_count = usize::from(*record.get(at)?);
    at += 1;
    let pps = read_parameter_sets(record, &mut at, pps_count)?;
    (!sps.is_empty() && !pps.is_empty()).then(|| ParameterSets { sps, pps, nal_length: usize::from(record[4] & 3) + 1 })
}

fn read_parameter_sets(record: &[u8], at: &mut usize, count: usize) -> Option<Vec<Vec<u8>>> {
    let mut sets = Vec::with_capacity(count);
    for _ in 0..count {
        let length = usize::from(u16::from_be_bytes([*record.get(*at)?, *record.get(*at + 1)?]));
        let set = record.get(*at + 2..*at + 2 + length)?;
        if set.is_empty() {
            return None;
        }
        sets.push(set.to_vec());
        *at += 2 + length;
    }
    Some(sets)
}

/// Why a stream stops being decoded as H.264 (gpui's words).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FallbackReason {
    DecoderUnavailable,
    NoDescription,
    DecodeErrors,
    Backlog,
}

impl FallbackReason {
    pub(crate) fn describe(self) -> &'static str {
        match self {
            FallbackReason::DecoderUnavailable => "the H.264 decoder could not be created",
            FallbackReason::NoDescription => "video arrived without an H.264 description",
            FallbackReason::DecodeErrors => "H.264 decoding failed repeatedly",
            FallbackReason::Backlog => "the H.264 decoder fell behind",
        }
    }
}

/// gpui's `undecodable`: an Android stream has no MJPEG to fall back to.
pub(crate) fn undecodable(reason: &str) -> String {
    format!("This Mac cannot decode the Android stream: {reason}.")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecodeEvent {
    DecoderFailed,
    VideoWithoutDescription,
    DecodeFailed,
    Decoded,
    Pending(usize, std::time::Instant),
}

#[derive(Debug, Default)]
pub(crate) struct Health {
    errors: u32,
    backlog_since: Option<std::time::Instant>,
}

impl Health {
    pub(crate) fn record(&mut self, event: DecodeEvent) -> Option<FallbackReason> {
        match event {
            DecodeEvent::DecoderFailed => Some(FallbackReason::DecoderUnavailable),
            DecodeEvent::VideoWithoutDescription => Some(FallbackReason::NoDescription),
            DecodeEvent::DecodeFailed => {
                self.errors += 1;
                (self.errors >= MAX_DECODE_ERRORS).then_some(FallbackReason::DecodeErrors)
            }
            DecodeEvent::Decoded => {
                self.errors = 0;
                None
            }
            DecodeEvent::Pending(frames, now) if frames > MAX_PENDING_FRAMES => {
                let since = *self.backlog_since.get_or_insert(now);
                (now.duration_since(since) >= BACKLOG_WINDOW).then_some(FallbackReason::Backlog)
            }
            DecodeEvent::Pending(..) => {
                self.backlog_since = None;
                None
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct KeyframeGate {
    awaiting: bool,
}

impl Default for KeyframeGate {
    fn default() -> Self {
        Self { awaiting: true }
    }
}

impl KeyframeGate {
    pub(crate) fn reset(&mut self) {
        self.awaiting = true;
    }

    pub(crate) fn admit(&mut self, tag: u8) -> bool {
        match tag {
            AVCC_KEYFRAME => {
                self.awaiting = false;
                true
            }
            AVCC_DELTA => !self.awaiting,
            _ => false,
        }
    }
}

/// Straight RGBA8 pixels and their size.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The picture turned as the device is held: LandscapeLeft a quarter turn
/// clockwise, as gpui's `rotate_for_display` (image's `rotate90`).
pub(crate) fn rotate(picture: &Rgba, orientation: DeviceOrientation) -> Rgba {
    let (w, h) = (picture.width as usize, picture.height as usize);
    if orientation == DeviceOrientation::Portrait {
        return picture.clone();
    }
    let turned = matches!(orientation, DeviceOrientation::LandscapeLeft | DeviceOrientation::LandscapeRight);
    let (out_w, out_h) = if turned { (h, w) } else { (w, h) };
    let mut out = vec![0u8; picture.pixels.len()];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match orientation {
                DeviceOrientation::LandscapeLeft => (h - 1 - y, x),
                DeviceOrientation::LandscapeRight => (y, w - 1 - x),
                _ => (w - 1 - x, h - 1 - y),
            };
            let from = (y * w + x) * 4;
            let to = (ny * out_w + nx) * 4;
            out[to..to + 4].copy_from_slice(&picture.pixels[from..from + 4]);
        }
    }
    Rgba { width: out_w as u32, height: out_h as u32, pixels: out }
}

/// A JPEG's (or PNG's) pixels.
pub(crate) fn decode_still(bytes: &[u8]) -> Option<Rgba> {
    let image = zgx_gpu::assets::decode_image(bytes).ok()?;
    Some(Rgba { width: image.width(), height: image.height(), pixels: image.pixels().to_vec() })
}

#[cfg(target_os = "macos")]
pub(crate) use decoder::Pipeline;

#[cfg(target_os = "macos")]
mod decoder {
    use std::ffi::{c_int, c_void};
    use std::ptr::{NonNull, null, null_mut};

    use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};
    use objc2_core_media::{
        CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMTime, CMVideoFormatDescriptionCreateFromH264ParameterSets,
        kCMBlockBufferAssureMemoryNowFlag,
    };
    use objc2_core_video::{
        CVImageBuffer, CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
        CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
        kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA,
    };
    use objc2_video_toolbox::{
        VTDecodeFrameFlags, VTDecodeInfoFlags, VTDecompressionOutputCallbackRecord, VTDecompressionSession,
    };
    use zeron_proto::device_hub::{AVCC_DELTA, AVCC_DESCRIPTION, AVCC_KEYFRAME, AVCC_SEED, AvccEnvelope};

    use super::{DecodeEvent, FallbackReason, Health, KeyframeGate, Rgba, parse_avcc};

    type DecodeSlot = Option<Result<Option<CFRetained<CVPixelBuffer>>, i32>>;

    unsafe extern "C-unwind" fn decoded(
        _session: *mut c_void,
        frame: *mut c_void,
        status: i32,
        _flags: VTDecodeInfoFlags,
        image: *mut CVImageBuffer,
        _pts: CMTime,
        _duration: CMTime,
    ) {
        // SAFETY: `frame` is the DecodeSlot `Decoder::decode` keeps alive until the session has drained.
        let slot = unsafe { &mut *frame.cast::<DecodeSlot>() };
        *slot = Some(match (status, NonNull::new(image)) {
            // SAFETY: VT owns `image` for the callback; retaining keeps it past the return.
            (0, Some(image)) => Ok(Some(unsafe { CFRetained::retain(image) })),
            (0, None) => Ok(None),
            (status, _) => Err(status),
        });
    }

    /// A decoded buffer's BGRA rows as straight RGBA.
    fn rgba_of(buffer: &CVPixelBuffer) -> Option<Rgba> {
        let (width, height) = (CVPixelBufferGetWidth(buffer), CVPixelBufferGetHeight(buffer));
        // SAFETY: the buffer is locked read-only while its base address is read.
        unsafe {
            if CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly) != 0 {
                return None;
            }
            let base = CVPixelBufferGetBaseAddress(buffer).cast::<u8>();
            let row = CVPixelBufferGetBytesPerRow(buffer);
            let mut pixels = Vec::with_capacity(width * height * 4);
            if !base.is_null() {
                for y in 0..height {
                    let line = std::slice::from_raw_parts(base.add(y * row), width * 4);
                    for bgra in line.chunks_exact(4) {
                        pixels.extend_from_slice(&[bgra[2], bgra[1], bgra[0], 255]);
                    }
                }
            }
            CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly);
            (pixels.len() == width * height * 4 && width > 0 && height > 0).then(|| Rgba {
                width: width as u32,
                height: height as u32,
                pixels,
            })
        }
    }

    pub(crate) struct Decoder {
        description: Vec<u8>,
        format: CFRetained<CMFormatDescription>,
        session: CFRetained<VTDecompressionSession>,
    }

    impl Decoder {
        pub(crate) fn new(description: &[u8]) -> Result<Self, String> {
            let sets = parse_avcc(description).ok_or("the avcC record is malformed")?;
            let all: Vec<&Vec<u8>> = sets.sps.iter().chain(&sets.pps).collect();
            let pointers: Vec<NonNull<u8>> = all.iter().map(|set| NonNull::from(&set[0])).collect();
            let sizes: Vec<usize> = all.iter().map(|set| set.len()).collect();
            let mut format: *const CMFormatDescription = null();
            // SAFETY: pointers and sizes describe live, non-empty parameter sets of equal count.
            let status = unsafe {
                CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    None,
                    pointers.len(),
                    NonNull::new(pointers.as_ptr().cast_mut()).ok_or("no parameter sets")?,
                    NonNull::new(sizes.as_ptr().cast_mut()).ok_or("no parameter sets")?,
                    sets.nal_length as c_int,
                    NonNull::from(&mut format),
                )
            };
            let format = NonNull::new(format.cast_mut())
                .filter(|_| status == 0)
                .ok_or_else(|| format!("CMVideoFormatDescription failed ({status})"))?;
            // SAFETY: a successful Create returns a +1 reference.
            let format = unsafe { CFRetained::from_raw(format) };
            let callback =
                VTDecompressionOutputCallbackRecord { decompressionOutputCallback: Some(decoded), decompressionOutputRefCon: null_mut() };
            let bgra = CFNumber::new_i32(kCVPixelFormatType_32BGRA as i32);
            // SAFETY: the key is an immutable CoreVideo constant.
            let attributes = CFDictionary::<CFString, CFType>::from_slices(
                &[unsafe { kCVPixelBufferPixelFormatTypeKey }],
                &[bgra.as_ref() as &CFType],
            );
            let mut session: *mut VTDecompressionSession = null_mut();
            // SAFETY: every pointer outlives the call; the attribute dictionary holds CF types.
            let status = unsafe {
                VTDecompressionSession::create(
                    None,
                    &format,
                    None,
                    Some(attributes.as_opaque()),
                    &callback,
                    NonNull::from(&mut session),
                )
            };
            let session = NonNull::new(session)
                .filter(|_| status == 0)
                .ok_or_else(|| format!("VTDecompressionSession failed ({status})"))?;
            // SAFETY: a successful Create returns a +1 reference.
            Ok(Self { description: description.to_vec(), format, session: unsafe { CFRetained::from_raw(session) } })
        }

        pub(crate) fn decode(&self, access_unit: &[u8]) -> Result<Option<Rgba>, i32> {
            let length = access_unit.len();
            let source = NonNull::new(access_unit.as_ptr().cast_mut().cast::<c_void>()).ok_or(-1)?;
            let mut block: *mut CMBlockBuffer = null_mut();
            // SAFETY: CoreMedia allocates and owns `length` bytes; the copy below fills them.
            let status = unsafe {
                CMBlockBuffer::create_with_memory_block(
                    None,
                    null_mut(),
                    length,
                    None,
                    null(),
                    0,
                    length,
                    kCMBlockBufferAssureMemoryNowFlag,
                    NonNull::from(&mut block),
                )
            };
            let block = NonNull::new(block).filter(|_| status == 0).ok_or(status)?;
            // SAFETY: a successful Create returns a +1 reference.
            let block = unsafe { CFRetained::from_raw(block) };
            // SAFETY: `source` points at `length` readable bytes and the block holds `length` bytes.
            let status = unsafe { CMBlockBuffer::replace_data_bytes(source, &block, 0, length) };
            if status != 0 {
                return Err(status);
            }
            let sizes = [length];
            let mut sample: *mut CMSampleBuffer = null_mut();
            // SAFETY: one sample, one size entry, no timing; all pointers outlive the call.
            let status = unsafe {
                CMSampleBuffer::create_ready(
                    None,
                    Some(&block),
                    Some(&self.format),
                    1,
                    0,
                    null(),
                    1,
                    sizes.as_ptr(),
                    NonNull::from(&mut sample),
                )
            };
            let sample = NonNull::new(sample).filter(|_| status == 0).ok_or(status)?;
            // SAFETY: a successful Create returns a +1 reference.
            let sample = unsafe { CFRetained::from_raw(sample) };
            let mut slot: DecodeSlot = None;
            // SAFETY: flags 0 decode synchronously; the wait below drains any callback before `slot` drops.
            let status = unsafe {
                let status = self.session.decode_frame(&sample, VTDecodeFrameFlags(0), (&raw mut slot).cast(), null_mut());
                self.session.wait_for_asynchronous_frames();
                status
            };
            if status != 0 {
                return Err(status);
            }
            match slot {
                Some(Ok(Some(buffer))) => Ok(rgba_of(&buffer)),
                Some(Ok(None)) | None => Ok(None),
                Some(Err(status)) => Err(status),
            }
        }
    }

    impl Drop for Decoder {
        fn drop(&mut self) {
            // SAFETY: the session is valid and no decode is in flight on this thread.
            unsafe { self.session.invalidate() };
        }
    }

    /// What a batch of envelopes made: the newest picture, a seed (a JPEG the
    /// hub sends before the first keyframe), or why to stop.
    #[derive(Default)]
    pub(crate) struct Batch {
        pub seed: Option<Vec<u8>>,
        pub picture: Option<Rgba>,
        pub fallback: Option<FallbackReason>,
    }

    #[derive(Default)]
    pub(crate) struct Pipeline {
        decoder: Option<Decoder>,
        gate: KeyframeGate,
        health: Health,
        decoded: bool,
    }

    impl Pipeline {
        pub(crate) fn pending(&mut self, frames: usize) -> Option<FallbackReason> {
            self.health.record(DecodeEvent::Pending(frames, std::time::Instant::now()))
        }

        pub(crate) fn feed(&mut self, envelopes: Vec<AvccEnvelope>) -> Batch {
            let mut batch = Batch::default();
            for AvccEnvelope { tag, payload } in envelopes {
                match tag {
                    AVCC_SEED if self.decoded => {}
                    AVCC_SEED => {
                        batch.seed = Some(payload);
                        batch.picture = None;
                    }
                    AVCC_DESCRIPTION => {
                        self.gate.reset();
                        if self.decoder.as_ref().is_some_and(|d| d.description == payload) {
                            continue;
                        }
                        self.decoder = None;
                        match Decoder::new(&payload) {
                            Ok(decoder) => self.decoder = Some(decoder),
                            Err(_) => {
                                batch.fallback = self.health.record(DecodeEvent::DecoderFailed);
                                return batch;
                            }
                        }
                    }
                    AVCC_KEYFRAME | AVCC_DELTA => {
                        let Some(decoder) = &self.decoder else {
                            batch.fallback = self.health.record(DecodeEvent::VideoWithoutDescription);
                            return batch;
                        };
                        if !self.gate.admit(tag) {
                            continue;
                        }
                        match decoder.decode(&payload) {
                            Ok(picture) => {
                                self.health.record(DecodeEvent::Decoded);
                                if picture.is_some() {
                                    self.decoded = true;
                                    batch.picture = picture;
                                    batch.seed = None;
                                }
                            }
                            Err(_) => {
                                self.gate.reset();
                                batch.fallback = self.health.record(DecodeEvent::DecodeFailed);
                                if batch.fallback.is_some() {
                                    return batch;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            batch
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use zeron_proto::device_hub::{AVCC_DESCRIPTION, AvccDemuxer};

    const FIXTURE: &[u8] = include_bytes!("../../../crates/ui/tests/fixtures/device-ios-588x1280.avcc");

    #[test]
    fn the_captured_description_parses_and_a_bad_record_does_not() {
        let envelopes = AvccDemuxer::default().push(FIXTURE).unwrap();
        let description = &envelopes.iter().find(|e| e.tag == AVCC_DESCRIPTION).unwrap().payload;
        let sets = parse_avcc(description).unwrap();
        assert_eq!((sets.sps.len(), sets.pps.len(), sets.nal_length), (1, 1, 4));
        assert_eq!(parse_avcc(&[0x02, 0x64, 0, 0x20, 0xff, 0xe1]), None);
        assert_eq!(parse_avcc(&description[..description.len() - 1]), None);
    }

    #[test]
    fn falls_back_after_three_failures_in_a_row_or_three_seconds_behind() {
        let mut health = Health::default();
        assert_eq!(health.record(DecodeEvent::DecodeFailed), None);
        assert_eq!(health.record(DecodeEvent::DecodeFailed), None);
        assert_eq!(health.record(DecodeEvent::Decoded), None);
        assert_eq!(health.record(DecodeEvent::DecodeFailed), None);
        assert_eq!(health.record(DecodeEvent::DecodeFailed), None);
        assert_eq!(health.record(DecodeEvent::DecodeFailed), Some(FallbackReason::DecodeErrors));
        let start = Instant::now();
        let mut health = Health::default();
        let behind = MAX_PENDING_FRAMES + 1;
        assert_eq!(health.record(DecodeEvent::Pending(behind, start)), None);
        assert_eq!(health.record(DecodeEvent::Pending(0, start + Duration::from_secs(1))), None);
        assert_eq!(health.record(DecodeEvent::Pending(behind, start + Duration::from_secs(2))), None);
        assert_eq!(health.record(DecodeEvent::Pending(behind, start + Duration::from_secs(5))), Some(FallbackReason::Backlog));
        let mut gate = KeyframeGate::default();
        assert!(!gate.admit(AVCC_DELTA));
        assert!(gate.admit(AVCC_KEYFRAME) && gate.admit(AVCC_DELTA));
        gate.reset();
        assert!(!gate.admit(AVCC_DELTA));
    }

    #[test]
    fn a_picture_turns_as_the_device_is_held() {
        // 2×1: red, green.
        let p = Rgba { width: 2, height: 1, pixels: vec![255, 0, 0, 255, 0, 255, 0, 255] };
        let left = rotate(&p, DeviceOrientation::LandscapeLeft);
        assert_eq!((left.width, left.height), (1, 2));
        assert_eq!(left.pixels, vec![255, 0, 0, 255, 0, 255, 0, 255], "clockwise: red on top");
        let right = rotate(&p, DeviceOrientation::LandscapeRight);
        assert_eq!(right.pixels, vec![0, 255, 0, 255, 255, 0, 0, 255], "counter-clockwise: green on top");
        let upside = rotate(&p, DeviceOrientation::PortraitUpsideDown);
        assert_eq!((upside.width, upside.pixels[0]), (2, 0));
        assert_eq!(rotate(&p, DeviceOrientation::Portrait), p);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_captured_stream_decodes_to_rgba_and_a_stream_without_description_falls_back() {
        let envelopes = AvccDemuxer::default().push(FIXTURE).unwrap();
        let mut pipeline = Pipeline::default();
        let batch = pipeline.feed(envelopes[1..].to_vec());
        assert_eq!(batch.fallback, Some(FallbackReason::NoDescription));
        let mut pipeline = Pipeline::default();
        let batch = pipeline.feed(envelopes.clone());
        assert!(batch.fallback.is_none());
        let picture = batch.picture.expect("a picture");
        assert_eq!((picture.width, picture.height), (588, 1280));
        assert_eq!(picture.pixels.len(), 588 * 1280 * 4);
        assert!(picture.pixels.chunks_exact(4).all(|p| p[3] == 255));
        assert!(picture.pixels.chunks_exact(4).any(|p| p[0] != p[2]), "colour, not one grey plane");
        let seed = zeron_proto::device_hub::AvccEnvelope { tag: zeron_proto::device_hub::AVCC_SEED, payload: b"stale".to_vec() };
        assert!(pipeline.feed(vec![seed]).seed.is_none(), "a seed never replaces a decoded picture");
    }
}
