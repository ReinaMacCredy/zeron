use super::contract::{
    COMPUTER_FRAME_MAGIC, COMPUTER_FRAME_MAX_COMPUTER_ID_BYTES, COMPUTER_FRAME_VERSION,
    ComputerFrameDecodeErrorReason, ComputerFrameHeader,
};

pub const FRAME_HEADER_FIXED_BYTES: usize = 17;
pub const FRAME_FLAG_KEYFRAME: u8 = 0b0000_0001;
pub const FRAME_FLAG_CODEC_CONFIG: u8 = 0b0000_0010;

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerFrame {
    pub header: ComputerFrameHeader,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerFrameEncodeError(pub String);

impl std::fmt::Display for ComputerFrameEncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ComputerFrameEncodeError {}

pub fn encode_computer_frame(frame: &ComputerFrame) -> Result<Vec<u8>, ComputerFrameEncodeError> {
    let id = frame.header.computer_id.as_bytes();
    if id.is_empty() {
        return Err(ComputerFrameEncodeError(
            "Computer frame header requires a non-empty computerId".into(),
        ));
    }
    if id.len() > COMPUTER_FRAME_MAX_COMPUTER_ID_BYTES {
        return Err(ComputerFrameEncodeError(format!(
            "Computer frame computerId exceeds {COMPUTER_FRAME_MAX_COMPUTER_ID_BYTES} UTF-8 bytes"
        )));
    }
    let mut flags = 0;
    if frame.header.keyframe {
        flags |= FRAME_FLAG_KEYFRAME;
    }
    if frame.header.codec_config {
        flags |= FRAME_FLAG_CODEC_CONFIG;
    }
    let mut bytes = Vec::with_capacity(FRAME_HEADER_FIXED_BYTES + id.len() + frame.payload.len());
    bytes.extend_from_slice(&COMPUTER_FRAME_MAGIC.to_le_bytes());
    bytes.push(COMPUTER_FRAME_VERSION);
    bytes.push(flags);
    bytes.extend_from_slice(&frame.header.sequence.to_le_bytes());
    bytes.extend_from_slice(&frame.header.timestamp_ms.to_le_bytes());
    bytes.push(id.len() as u8);
    bytes.extend_from_slice(id);
    bytes.extend_from_slice(&frame.payload);
    Ok(bytes)
}

pub fn decode_computer_frame(
    bytes: &[u8],
) -> Result<ComputerFrame, ComputerFrameDecodeErrorReason> {
    if bytes.len() < FRAME_HEADER_FIXED_BYTES {
        return Err(ComputerFrameDecodeErrorReason::TooShort);
    }
    if u16::from_le_bytes([bytes[0], bytes[1]]) != COMPUTER_FRAME_MAGIC {
        return Err(ComputerFrameDecodeErrorReason::BadMagic);
    }
    if bytes[2] != COMPUTER_FRAME_VERSION {
        return Err(ComputerFrameDecodeErrorReason::UnsupportedVersion);
    }
    let flags = bytes[3];
    let id_len = bytes[16] as usize;
    let payload_offset = FRAME_HEADER_FIXED_BYTES + id_len;
    if id_len == 0 || bytes.len() < payload_offset {
        return Err(ComputerFrameDecodeErrorReason::TruncatedComputerId);
    }
    let computer_id = std::str::from_utf8(&bytes[FRAME_HEADER_FIXED_BYTES..payload_offset])
        .map_err(|_| ComputerFrameDecodeErrorReason::InvalidComputerId)?
        .to_string();
    let mut sequence = [0; 4];
    sequence.copy_from_slice(&bytes[4..8]);
    let mut timestamp = [0; 8];
    timestamp.copy_from_slice(&bytes[8..16]);
    Ok(ComputerFrame {
        header: ComputerFrameHeader {
            computer_id,
            sequence: u32::from_le_bytes(sequence),
            timestamp_ms: f64::from_le_bytes(timestamp),
            keyframe: flags & FRAME_FLAG_KEYFRAME != 0,
            codec_config: flags & FRAME_FLAG_CODEC_CONFIG != 0,
        },
        payload: bytes[payload_offset..].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYNARA_DEVICE_FRAME_MAGIC: u16 = 0x5346;

    fn header() -> ComputerFrameHeader {
        ComputerFrameHeader {
            computer_id: "desktop".into(),
            sequence: 42,
            timestamp_ms: 1_234.5,
            keyframe: false,
            codec_config: false,
        }
    }

    fn frame() -> ComputerFrame {
        ComputerFrame {
            header: header(),
            payload: vec![0x00, 0x01, 0x02, 0xff],
        }
    }

    #[test]
    fn round_trips_header_fields_and_payload_bytes() {
        let frame = frame();
        assert_eq!(
            decode_computer_frame(&encode_computer_frame(&frame).unwrap()),
            Ok(frame)
        );
    }

    #[test]
    fn round_trips_an_empty_payload_and_independent_flags() {
        let frame = ComputerFrame {
            header: ComputerFrameHeader {
                keyframe: true,
                codec_config: true,
                ..header()
            },
            payload: Vec::new(),
        };
        let decoded = decode_computer_frame(&encode_computer_frame(&frame).unwrap()).unwrap();
        assert!(decoded.header.keyframe);
        assert!(decoded.header.codec_config);
        assert!(decoded.payload.is_empty());
    }

    #[test]
    fn uses_a_distinct_wire_magic_from_device_frames() {
        assert_ne!(COMPUTER_FRAME_MAGIC, SYNARA_DEVICE_FRAME_MAGIC);
        let mut device = encode_computer_frame(&frame()).unwrap();
        device[..2].copy_from_slice(&SYNARA_DEVICE_FRAME_MAGIC.to_le_bytes());
        assert_eq!(
            decode_computer_frame(&device),
            Err(ComputerFrameDecodeErrorReason::BadMagic)
        );
    }

    #[test]
    fn rejects_empty_or_oversized_computer_ids() {
        for computer_id in [String::new(), "x".repeat(256)] {
            let frame = ComputerFrame {
                header: ComputerFrameHeader {
                    computer_id,
                    ..header()
                },
                payload: Vec::new(),
            };
            assert!(encode_computer_frame(&frame).is_err());
        }
    }

    #[test]
    fn rejects_buffers_shorter_than_the_fixed_header() {
        assert_eq!(
            decode_computer_frame(&[0; FRAME_HEADER_FIXED_BYTES - 1]),
            Err(ComputerFrameDecodeErrorReason::TooShort)
        );
    }

    #[test]
    fn rejects_a_wrong_magic_and_unsupported_version() {
        let encoded = encode_computer_frame(&frame()).unwrap();
        let mut wrong_magic = encoded.clone();
        wrong_magic[..2].copy_from_slice(&(COMPUTER_FRAME_MAGIC ^ 0xffff).to_le_bytes());
        assert_eq!(
            decode_computer_frame(&wrong_magic),
            Err(ComputerFrameDecodeErrorReason::BadMagic)
        );
        let mut future = encoded;
        future[2] = COMPUTER_FRAME_VERSION + 1;
        assert_eq!(
            decode_computer_frame(&future),
            Err(ComputerFrameDecodeErrorReason::UnsupportedVersion)
        );
    }

    #[test]
    fn rejects_zero_length_and_invalid_utf8_computer_ids() {
        let encoded = encode_computer_frame(&frame()).unwrap();
        let mut zero_length = encoded.clone();
        zero_length[16] = 0;
        assert_eq!(
            decode_computer_frame(&zero_length),
            Err(ComputerFrameDecodeErrorReason::TruncatedComputerId)
        );
        let mut invalid_utf8 = encoded;
        invalid_utf8[FRAME_HEADER_FIXED_BYTES] = 0xff;
        assert_eq!(
            decode_computer_frame(&invalid_utf8),
            Err(ComputerFrameDecodeErrorReason::InvalidComputerId)
        );
    }
}
