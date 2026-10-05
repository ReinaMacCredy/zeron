use zeron_proto::device_hub::{AVCC_DELTA, AVCC_KEYFRAME};

pub const MAX_PENDING_FRAMES: usize = 8;
pub const MAX_DECODE_ERRORS: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterSets {
    pub sps: Vec<Vec<u8>>,
    pub pps: Vec<Vec<u8>>,
    pub nal_length: usize,
}

pub fn parse_avcc(record: &[u8]) -> Option<ParameterSets> {
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
    (!sps.is_empty() && !pps.is_empty()).then(|| ParameterSets {
        sps,
        pps,
        nal_length: usize::from(record[4] & 3) + 1,
    })
}

fn read_parameter_sets(record: &[u8], at: &mut usize, count: usize) -> Option<Vec<Vec<u8>>> {
    let mut sets = Vec::with_capacity(count);
    for _ in 0..count {
        let length = usize::from(u16::from_be_bytes([
            *record.get(*at)?,
            *record.get(*at + 1)?,
        ]));
        let set = record.get(*at + 2..*at + 2 + length)?;
        if set.is_empty() {
            return None;
        }
        sets.push(set.to_vec());
        *at += 2 + length;
    }
    Some(sets)
}

pub fn codec_string(record: &[u8]) -> Option<String> {
    (record.len() >= 4 && record[0] == 1)
        .then(|| format!("avc1.{:02x}{:02x}{:02x}", record[1], record[2], record[3]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    DecoderUnavailable,
    NoDescription,
    DecodeErrors,
    Backlog,
    RotationFailed,
}

impl FallbackReason {
    pub fn describe(self) -> &'static str {
        match self {
            FallbackReason::DecoderUnavailable => "the H.264 decoder could not be created",
            FallbackReason::NoDescription => "video arrived without an H.264 description",
            FallbackReason::DecodeErrors => "H.264 decoding failed repeatedly",
            FallbackReason::Backlog => "the H.264 decoder fell behind",
            FallbackReason::RotationFailed => "the H.264 frame could not be rotated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeEvent {
    DecoderFailed,
    VideoWithoutDescription,
    DecodeFailed,
    Decoded,
    Pending(usize),
}

#[derive(Debug, Default)]
pub struct Health {
    errors: u32,
    backlogs: u32,
}

impl Health {
    pub fn record(&mut self, event: DecodeEvent) -> Option<FallbackReason> {
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
            DecodeEvent::Pending(frames) if frames > MAX_PENDING_FRAMES => {
                self.backlogs += 1;
                (self.backlogs >= 2).then_some(FallbackReason::Backlog)
            }
            DecodeEvent::Pending(_) => {
                self.backlogs = 0;
                None
            }
        }
    }
}

#[derive(Debug)]
pub struct KeyframeGate {
    awaiting: bool,
}

impl Default for KeyframeGate {
    fn default() -> Self {
        Self { awaiting: true }
    }
}

impl KeyframeGate {
    pub fn reset(&mut self) {
        self.awaiting = true;
    }

    pub fn admit(&mut self, tag: u8) -> bool {
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

#[cfg(target_os = "macos")]
pub use decoder::{Batch, Decoder, Pipeline, Surface};

#[cfg(target_os = "macos")]
mod decoder {
    use std::ffi::{c_int, c_void};
    use std::ptr::{NonNull, null, null_mut};

    use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
    use objc2_core_media::{
        CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMTime,
        CMVideoFormatDescriptionCreateFromH264ParameterSets, kCMBlockBufferAssureMemoryNowFlag,
    };
    use objc2_core_video::{
        CVImageBuffer, CVPixelBuffer, CVPixelBufferGetHeight, CVPixelBufferGetWidth,
        CVPixelBufferPool, kCVPixelBufferHeightKey, kCVPixelBufferIOSurfacePropertiesKey,
        kCVPixelBufferMetalCompatibilityKey, kCVPixelBufferPixelFormatTypeKey,
        kCVPixelBufferWidthKey, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
    };
    use objc2_video_toolbox::{
        VTDecodeFrameFlags, VTDecodeInfoFlags, VTDecompressionOutputCallbackRecord,
        VTDecompressionSession, VTPixelRotationSession, VTSessionSetProperty,
        kVTPixelRotationPropertyKey_Rotation, kVTRotation_0, kVTRotation_180, kVTRotation_CCW90,
        kVTRotation_CW90,
    };
    use zeron_proto::device_hub::{
        AVCC_DELTA, AVCC_DESCRIPTION, AVCC_KEYFRAME, AVCC_SEED, AvccEnvelope, DeviceOrientation,
    };

    use super::{DecodeEvent, FallbackReason, Health, KeyframeGate, parse_avcc};

    #[derive(Clone)]
    pub struct Surface(CFRetained<CVPixelBuffer>);

    // SAFETY: a decoded CVPixelBuffer is never written after decode, and CF retain/release are thread-safe.
    unsafe impl Send for Surface {}

    impl Surface {
        pub fn width(&self) -> u32 {
            CVPixelBufferGetWidth(&self.0) as u32
        }

        pub fn height(&self) -> u32 {
            CVPixelBufferGetHeight(&self.0) as u32
        }

        pub fn buffer(&self) -> &CVPixelBuffer {
            &self.0
        }

        pub fn to_gpui(&self) -> core_video::pixel_buffer::CVPixelBuffer {
            use core_foundation::base::TCFType as _;
            let raw = CFRetained::as_ptr(&self.0).as_ptr();
            // SAFETY: both crates name the same CVPixelBufferRef; get-rule adds the retain the wrapper releases.
            unsafe { core_video::pixel_buffer::CVPixelBuffer::wrap_under_get_rule(raw.cast()) }
        }
    }

    fn output_attributes(size: Option<(u32, u32)>) -> CFRetained<CFDictionary<CFString, CFType>> {
        let format = CFNumber::new_i32(kCVPixelFormatType_420YpCbCr8BiPlanarFullRange as i32);
        let io_surface = CFDictionary::<CFString, CFType>::empty();
        let dimensions =
            size.map(|(width, height)| [width, height].map(|side| CFNumber::new_i32(side as i32)));
        // SAFETY: the keys are immutable CoreVideo constants.
        let mut keys = unsafe {
            vec![
                kCVPixelBufferPixelFormatTypeKey,
                kCVPixelBufferIOSurfacePropertiesKey,
                kCVPixelBufferMetalCompatibilityKey,
            ]
        };
        let mut values: Vec<&CFType> = vec![
            format.as_ref(),
            io_surface.as_ref(),
            CFBoolean::new(true).as_ref(),
        ];
        if let Some([width, height]) = &dimensions {
            // SAFETY: the keys are immutable CoreVideo constants.
            keys.extend(unsafe { [kCVPixelBufferWidthKey, kCVPixelBufferHeightKey] });
            values.extend::<[&CFType; 2]>([width.as_ref(), height.as_ref()]);
        }
        CFDictionary::from_slices(&keys, &values)
    }

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

    pub struct Decoder {
        description: Vec<u8>,
        format: CFRetained<CMFormatDescription>,
        session: CFRetained<VTDecompressionSession>,
    }

    impl Decoder {
        pub fn new(description: &[u8]) -> Result<Self, String> {
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
            let callback = VTDecompressionOutputCallbackRecord {
                decompressionOutputCallback: Some(decoded),
                decompressionOutputRefCon: null_mut(),
            };
            let mut session: *mut VTDecompressionSession = null_mut();
            let attributes = output_attributes(None);
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
            Ok(Self {
                description: description.to_vec(),
                format,
                // SAFETY: a successful Create returns a +1 reference.
                session: unsafe { CFRetained::from_raw(session) },
            })
        }

        pub fn description(&self) -> &[u8] {
            &self.description
        }

        pub fn decode(&self, access_unit: &[u8]) -> Result<Option<Surface>, i32> {
            let length = access_unit.len();
            let source =
                NonNull::new(access_unit.as_ptr().cast_mut().cast::<c_void>()).ok_or(-1)?;
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
                let status = self.session.decode_frame(
                    &sample,
                    VTDecodeFrameFlags(0),
                    (&raw mut slot).cast(),
                    null_mut(),
                );
                self.session.wait_for_asynchronous_frames();
                status
            };
            if status != 0 {
                return Err(status);
            }
            match slot {
                Some(Ok(buffer)) => Ok(buffer.map(Surface)),
                Some(Err(status)) => Err(status),
                None => Ok(None),
            }
        }
    }

    impl Drop for Decoder {
        fn drop(&mut self) {
            // SAFETY: the session is valid and no decode is in flight on this thread.
            unsafe { self.session.invalidate() };
        }
    }

    struct Rotator {
        session: CFRetained<VTPixelRotationSession>,
        angle: Option<DeviceOrientation>,
        pool: Option<((u32, u32), CFRetained<CVPixelBufferPool>)>,
    }

    impl Rotator {
        fn new() -> Option<Self> {
            let mut session: *mut VTPixelRotationSession = null_mut();
            // SAFETY: the out pointer is valid for the call.
            let status =
                unsafe { VTPixelRotationSession::create(None, NonNull::from(&mut session)) };
            let session = NonNull::new(session).filter(|_| status == 0)?;
            Some(Self {
                // SAFETY: a successful Create returns a +1 reference.
                session: unsafe { CFRetained::from_raw(session) },
                angle: None,
                pool: None,
            })
        }

        fn rotate(&mut self, source: &Surface, rotation: DeviceOrientation) -> Option<Surface> {
            if self.angle != Some(rotation) {
                // SAFETY: the angle names are immutable VideoToolbox constants.
                let angle = unsafe {
                    match rotation {
                        DeviceOrientation::Portrait => kVTRotation_0,
                        DeviceOrientation::LandscapeLeft => kVTRotation_CW90,
                        DeviceOrientation::PortraitUpsideDown => kVTRotation_180,
                        DeviceOrientation::LandscapeRight => kVTRotation_CCW90,
                    }
                };
                // SAFETY: the rotation session accepts a CFString angle for this key.
                let status = unsafe {
                    VTSessionSetProperty(
                        &self.session,
                        kVTPixelRotationPropertyKey_Rotation,
                        Some(angle.as_ref()),
                    )
                };
                if status != 0 {
                    return None;
                }
                self.angle = Some(rotation);
            }
            let (width, height) = match rotation {
                DeviceOrientation::LandscapeLeft | DeviceOrientation::LandscapeRight => {
                    (source.height(), source.width())
                }
                _ => (source.width(), source.height()),
            };
            if self
                .pool
                .as_ref()
                .is_none_or(|(size, _)| *size != (width, height))
            {
                let mut pool: *mut CVPixelBufferPool = null_mut();
                let attributes = output_attributes(Some((width, height)));
                // SAFETY: the out pointer and attribute dictionary outlive the call.
                let status = unsafe {
                    CVPixelBufferPool::create(
                        None,
                        None,
                        Some(attributes.as_opaque()),
                        NonNull::from(&mut pool),
                    )
                };
                let pool = NonNull::new(pool).filter(|_| status == 0)?;
                // SAFETY: a successful Create returns a +1 reference.
                self.pool = Some(((width, height), unsafe { CFRetained::from_raw(pool) }));
            }
            let (_, pool) = self.pool.as_ref()?;
            let mut target: *mut CVPixelBuffer = null_mut();
            // SAFETY: the pool is valid and the out pointer outlives the call.
            let status = unsafe {
                CVPixelBufferPool::create_pixel_buffer(None, pool, NonNull::from(&mut target))
            };
            let target = NonNull::new(target).filter(|_| status == 0)?;
            // SAFETY: a successful Create returns a +1 reference.
            let target = unsafe { CFRetained::from_raw(target) };
            // SAFETY: both buffers are valid NV12 buffers with matching rotated dimensions.
            let status = unsafe { self.session.rotate_image(source.buffer(), &target) };
            (status == 0).then_some(Surface(target))
        }
    }

    impl Drop for Rotator {
        fn drop(&mut self) {
            // SAFETY: the session is valid and used only from this thread.
            unsafe { self.session.invalidate() };
        }
    }

    #[derive(Default)]
    pub struct Batch {
        pub seed: Option<Vec<u8>>,
        pub picture: Option<Surface>,
        pub fallback: Option<FallbackReason>,
    }

    #[derive(Default)]
    pub struct Pipeline {
        decoder: Option<Decoder>,
        gate: KeyframeGate,
        health: Health,
        rotator: Option<Rotator>,
    }

    // SAFETY: the pipeline owns its CoreMedia and VideoToolbox objects outright and is only ever
    // driven by one task at a time; those objects carry no thread affinity.
    unsafe impl Send for Pipeline {}

    impl Pipeline {
        pub fn pending(&mut self, frames: usize) -> Option<FallbackReason> {
            self.health.record(DecodeEvent::Pending(frames))
        }

        pub fn feed(&mut self, envelopes: Vec<AvccEnvelope>, rotation: DeviceOrientation) -> Batch {
            let mut batch = Batch::default();
            for AvccEnvelope { tag, payload } in envelopes {
                match tag {
                    AVCC_SEED => {
                        batch.seed = Some(payload);
                        batch.picture = None;
                    }
                    AVCC_DESCRIPTION => {
                        self.gate.reset();
                        if self
                            .decoder
                            .as_ref()
                            .is_some_and(|decoder| decoder.description() == payload)
                        {
                            continue;
                        }
                        self.decoder = None;
                        match Decoder::new(&payload) {
                            Ok(decoder) => self.decoder = Some(decoder),
                            Err(error) => {
                                tracing::warn!(%error, "device panel H.264 decoder");
                                batch.fallback = self.health.record(DecodeEvent::DecoderFailed);
                                return batch;
                            }
                        }
                    }
                    AVCC_KEYFRAME | AVCC_DELTA => {
                        let Some(decoder) = &self.decoder else {
                            batch.fallback =
                                self.health.record(DecodeEvent::VideoWithoutDescription);
                            return batch;
                        };
                        if !self.gate.admit(tag) {
                            continue;
                        }
                        match decoder.decode(&payload) {
                            Ok(picture) => {
                                self.health.record(DecodeEvent::Decoded);
                                if picture.is_some() {
                                    batch.picture = picture;
                                    batch.seed = None;
                                }
                            }
                            Err(status) => {
                                tracing::debug!(status, "device panel H.264 decode error");
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
            if rotation != DeviceOrientation::Portrait
                && let Some(picture) = batch.picture.take()
            {
                if self.rotator.is_none() {
                    self.rotator = Rotator::new();
                }
                batch.picture = self
                    .rotator
                    .as_mut()
                    .and_then(|rotator| rotator.rotate(&picture, rotation));
                if batch.picture.is_none() {
                    batch.fallback = Some(FallbackReason::RotationFailed);
                }
            }
            batch
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::device_hub::{AVCC_DESCRIPTION, AVCC_SEED, AvccDemuxer};

    const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/device-ios-588x1280.avcc");

    fn fixture() -> Vec<zeron_proto::device_hub::AvccEnvelope> {
        AvccDemuxer::default().push(FIXTURE).unwrap()
    }

    #[test]
    fn the_avcc_record_yields_its_parameter_sets_and_codec_string() {
        let record = [
            0x01, 0x64, 0x00, 0x20, 0xff, 0xe1, 0x00, 0x03, 0x67, 0x64, 0x00, 0x01, 0x00, 0x02,
            0x68, 0xee,
        ];
        assert_eq!(
            parse_avcc(&record),
            Some(ParameterSets {
                sps: vec![vec![0x67, 0x64, 0x00]],
                pps: vec![vec![0x68, 0xee]],
                nal_length: 4,
            })
        );
        assert_eq!(codec_string(&record).as_deref(), Some("avc1.640020"));
        assert_eq!(parse_avcc(&record[..15]), None, "truncated PPS");
        assert_eq!(
            parse_avcc(&[0x02, 0x64, 0, 0x20, 0xff, 0xe1]),
            None,
            "version"
        );
        let mut no_pps = record[..11].to_vec();
        no_pps.push(0);
        assert_eq!(parse_avcc(&no_pps), None, "no PPS");
        assert_eq!(codec_string(&[1, 2]), None);
    }

    #[test]
    fn the_captured_description_is_high_profile_with_one_sps_and_pps() {
        let description = fixture()
            .into_iter()
            .find(|envelope| envelope.tag == AVCC_DESCRIPTION)
            .unwrap()
            .payload;
        assert_eq!(codec_string(&description).as_deref(), Some("avc1.640020"));
        let sets = parse_avcc(&description).unwrap();
        assert_eq!((sets.sps.len(), sets.pps.len(), sets.nal_length), (1, 1, 4));
        assert_eq!(sets.sps[0][0] & 0x1f, 7);
        assert_eq!(sets.pps[0][0] & 0x1f, 8);
    }

    #[test]
    fn fallback_decision_table() {
        let cases: &[(&[DecodeEvent], Option<FallbackReason>)] = &[
            (
                &[DecodeEvent::DecoderFailed],
                Some(FallbackReason::DecoderUnavailable),
            ),
            (
                &[DecodeEvent::VideoWithoutDescription],
                Some(FallbackReason::NoDescription),
            ),
            (
                &[DecodeEvent::DecodeFailed, DecodeEvent::DecodeFailed],
                None,
            ),
            (
                &[
                    DecodeEvent::DecodeFailed,
                    DecodeEvent::DecodeFailed,
                    DecodeEvent::DecodeFailed,
                ],
                Some(FallbackReason::DecodeErrors),
            ),
            (
                &[
                    DecodeEvent::DecodeFailed,
                    DecodeEvent::DecodeFailed,
                    DecodeEvent::Decoded,
                    DecodeEvent::DecodeFailed,
                    DecodeEvent::DecodeFailed,
                ],
                None,
            ),
            (&[DecodeEvent::Pending(MAX_PENDING_FRAMES)], None),
            (&[DecodeEvent::Pending(MAX_PENDING_FRAMES + 1)], None),
            (
                &[
                    DecodeEvent::Pending(MAX_PENDING_FRAMES + 1),
                    DecodeEvent::Pending(MAX_PENDING_FRAMES + 1),
                ],
                Some(FallbackReason::Backlog),
            ),
            (
                &[
                    DecodeEvent::Pending(MAX_PENDING_FRAMES + 1),
                    DecodeEvent::Pending(0),
                    DecodeEvent::Pending(MAX_PENDING_FRAMES + 1),
                ],
                None,
            ),
            (&[DecodeEvent::Decoded, DecodeEvent::Pending(0)], None),
        ];
        for (events, expected) in cases {
            let mut health = Health::default();
            let last = events
                .iter()
                .map(|event| health.record(*event))
                .last()
                .unwrap();
            assert_eq!(last, *expected, "{events:?}");
        }
    }

    #[test]
    fn deltas_wait_for_a_keyframe_after_every_reset() {
        let mut gate = KeyframeGate::default();
        assert!(!gate.admit(AVCC_DELTA));
        assert!(gate.admit(AVCC_KEYFRAME));
        assert!(gate.admit(AVCC_DELTA));
        assert!(!gate.admit(AVCC_SEED));
        gate.reset();
        assert!(!gate.admit(AVCC_DELTA));
        assert!(!gate.admit(AVCC_DELTA));
        assert!(gate.admit(AVCC_KEYFRAME));
        assert!(gate.admit(AVCC_DELTA));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_captured_stream_decodes_to_an_iosurface_backed_nv12_buffer() {
        use core_foundation::base::TCFType as _;
        use core_video::pixel_buffer_io_surface::CVPixelBufferGetIOSurface;
        use objc2_core_video::{
            CVPixelBufferGetPixelFormatType, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
        };
        let backed = |surface: &Surface| {
            // SAFETY: the buffer is alive for the call; the getter does not retain.
            !unsafe { CVPixelBufferGetIOSurface(surface.to_gpui().as_concrete_TypeRef()) }.is_null()
        };
        use zeron_proto::device_hub::DeviceOrientation;

        let envelopes = fixture();
        assert_eq!(
            envelopes
                .iter()
                .map(|envelope| envelope.tag)
                .collect::<Vec<_>>(),
            [1, 2, 3, 3, 3, 3]
        );
        let started = std::time::Instant::now();
        drop(Decoder::new(&envelopes[0].payload).unwrap());
        let first_session = started.elapsed();
        let started = std::time::Instant::now();
        let decoder = Decoder::new(&envelopes[0].payload).unwrap();
        eprintln!(
            "decoder session create: first {first_session:?}, again {:?}",
            started.elapsed()
        );
        let mut timings = Vec::new();
        let mut last = None;
        for envelope in &envelopes[1..] {
            let started = std::time::Instant::now();
            last = decoder.decode(&envelope.payload).unwrap();
            timings.push(started.elapsed());
            assert!(last.is_some());
        }
        eprintln!("H.264 decode per frame: {timings:?}");
        let surface = last.unwrap();
        assert_eq!((surface.width(), surface.height()), (588, 1280));
        assert_eq!(
            CVPixelBufferGetPixelFormatType(surface.buffer()),
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
        );
        assert!(backed(&surface));
        let gpui_buffer = surface.to_gpui();
        assert_eq!(
            (gpui_buffer.get_width(), gpui_buffer.get_height()),
            (588, 1280)
        );

        let mut pipeline = Pipeline::default();
        let batch = pipeline.feed(envelopes[2..].to_vec(), DeviceOrientation::Portrait);
        assert_eq!(batch.fallback, Some(FallbackReason::NoDescription));
        let mut pipeline = Pipeline::default();
        let batch = pipeline.feed(
            vec![envelopes[0].clone(), envelopes[2].clone()],
            DeviceOrientation::LandscapeLeft,
        );
        assert!(batch.picture.is_none() && batch.fallback.is_none());
        let batch = pipeline.feed(envelopes[1..].to_vec(), DeviceOrientation::LandscapeLeft);
        assert!(batch.fallback.is_none());
        let rotated = batch.picture.unwrap();
        assert_eq!((rotated.width(), rotated.height()), (1280, 588));
        assert!(backed(&rotated));
        let batch = pipeline.feed(
            vec![envelopes[0].clone(), envelopes[1].clone()],
            DeviceOrientation::Portrait,
        );
        assert_eq!(
            batch
                .picture
                .map(|picture| (picture.width(), picture.height())),
            Some((588, 1280))
        );
    }
}
