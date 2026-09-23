#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LumaImage {
    pub width: usize,
    pub height: usize,
    pub luma: Vec<u8>,
}

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

fn channels_for_color_type(color_type: u8) -> Option<usize> {
    match color_type {
        0 => Some(1),
        2 => Some(3),
        6 => Some(4),
        _ => None,
    }
}

// CPU-bound on multi-megapixel captures: async callers run it under `spawn_blocking`.
pub fn decode_png_luma(bytes: &[u8]) -> Option<LumaImage> {
    if bytes.len() < PNG_SIGNATURE.len() || bytes[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return None;
    }
    let read_u32 = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    let mut offset = PNG_SIGNATURE.len();
    let mut header: Option<(usize, usize, usize)> = None;
    let mut compressed = Vec::new();
    while offset + 8 <= bytes.len() {
        let length = read_u32(offset) as usize;
        let kind = &bytes[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start.checked_add(length)?;
        if data_end.checked_add(4)? > bytes.len() {
            return None;
        }
        match kind {
            b"IHDR" => {
                if length != 13 {
                    return None;
                }
                let width = read_u32(data_start) as usize;
                let height = read_u32(data_start + 4) as usize;
                let bit_depth = bytes[data_start + 8];
                let channels = channels_for_color_type(bytes[data_start + 9]);
                let interlace = bytes[data_start + 12];
                let channels = channels.filter(|_| bit_depth == 8 && interlace == 0)?;
                if width == 0 || height == 0 {
                    return None;
                }
                header = Some((width, height, channels));
            }
            b"IDAT" => compressed.extend_from_slice(&bytes[data_start..data_end]),
            b"IEND" => break,
            _ => {}
        }
        offset = data_end + 4;
    }
    let (width, height, channels) = header?;
    if compressed.is_empty() {
        return None;
    }
    let stride = width.checked_mul(channels)?;
    let needed = stride.checked_add(1)?.checked_mul(height)?;
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&compressed, needed).ok()?;
    unfilter_to_luma(&raw, width, height, channels)
}

fn unfilter_to_luma(raw: &[u8], width: usize, height: usize, channels: usize) -> Option<LumaImage> {
    let stride = width * channels;
    if raw.len() < (stride + 1) * height {
        return None;
    }
    let mut luma = vec![0; width * height];
    let mut previous = vec![0_u8; stride];
    let mut current = vec![0_u8; stride];
    let mut at = 0;
    for row in 0..height {
        let filter = raw[at];
        at += 1;
        current.copy_from_slice(&raw[at..at + stride]);
        at += stride;
        for index in 0..stride {
            let left = if index >= channels {
                current[index - channels]
            } else {
                0
            };
            let up = previous[index];
            let up_left = if index >= channels {
                previous[index - channels]
            } else {
                0
            };
            let value = current[index];
            current[index] = match filter {
                0 => value,
                1 => value.wrapping_add(left),
                2 => value.wrapping_add(up),
                3 => value.wrapping_add(((u16::from(left) + u16::from(up)) >> 1) as u8),
                4 => value.wrapping_add(paeth(left, up, up_left)),
                _ => return None,
            };
        }
        let row_start = row * width;
        for column in 0..width {
            let sample = column * channels;
            luma[row_start + column] = if channels == 1 {
                current[sample]
            } else {
                ((u32::from(current[sample]) * 299
                    + u32::from(current[sample + 1]) * 587
                    + u32::from(current[sample + 2]) * 114)
                    / 1000) as u8
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }
    Some(LumaImage {
        width,
        height,
        luma,
    })
}

fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let (a, b, c) = (i16::from(left), i16::from(up), i16::from(up_left));
    let estimate = a + b - c;
    let to_left = (estimate - a).abs();
    let to_up = (estimate - b).abs();
    let to_up_left = (estimate - c).abs();
    if to_left <= to_up && to_left <= to_up_left {
        left
    } else if to_up <= to_up_left {
        up
    } else {
        up_left
    }
}

const PROFILE_COLUMN_BAND: f64 = 0.5;
const POINT_COLUMN_BAND: f64 = 0.25;
const PROFILE_ROW_BAND: f64 = 0.8;
const MIN_PROFILE_OVERLAP_ROWS: i64 = 32;
const MIN_PROFILE_DEVIATION: f64 = 2.0;
const MAX_WINNING_SCORE_RATIO: f64 = 0.5;
const MAX_WINNING_SCORE: f64 = 6.0;

pub fn estimate_vertical_travel(
    before: &LumaImage,
    after: &LumaImage,
    max_shift: Option<f64>,
    column_center: Option<f64>,
) -> Option<i64> {
    if before.width != after.width || before.height != after.height {
        return None;
    }
    let (width, height) = (before.width as f64, before.height as f64);
    let (column_start, column_count) = match column_center {
        None => (
            (width * ((1.0 - PROFILE_COLUMN_BAND) / 2.0)).floor() as usize,
            ((width * PROFILE_COLUMN_BAND).floor() as usize).max(1),
        ),
        Some(center) => {
            let count = ((width * POINT_COLUMN_BAND).floor() as usize).clamp(1, before.width);
            let start = (center.clamp(0.0, 1.0) * width - count as f64 / 2.0)
                .floor()
                .max(0.0) as usize;
            (start.min(before.width - count), count)
        }
    };
    let row_start = (height * ((1.0 - PROFILE_ROW_BAND) / 2.0)).floor() as usize;
    let row_count = ((height * PROFILE_ROW_BAND).floor() as usize).max(1);
    let rows = row_count as i64;
    if rows < MIN_PROFILE_OVERLAP_ROWS {
        return None;
    }
    let before_profile = row_profile(before, row_start, row_count, column_start, column_count);
    let after_profile = row_profile(after, row_start, row_count, column_start, column_count);
    if mean_absolute_deviation(&before_profile) < MIN_PROFILE_DEVIATION {
        return None;
    }
    let max_shift = max_shift
        .unwrap_or((row_count as f64 * 0.9).floor())
        .floor()
        .max(0.0) as i64;
    let minimum_overlap = MIN_PROFILE_OVERLAP_ROWS.max((row_count as f64 * 0.4).ceil() as i64);
    let mut alignments: Vec<(i64, f64)> = Vec::new();
    let mut best: Option<(i64, f64)> = None;
    for shift in -max_shift..=max_shift {
        let from = 0.max(-shift);
        let to = rows.min(rows - shift);
        if to - from < minimum_overlap {
            continue;
        }
        let total: f64 = (from..to)
            .map(|index| {
                (after_profile[index as usize] - before_profile[(index + shift) as usize]).abs()
            })
            .sum();
        let score = total / (to - from) as f64;
        alignments.push((shift, score));
        if best.is_none_or(|(_, best_score)| score < best_score) {
            best = Some((shift, score));
        }
    }
    let (winner_shift, winner_score) = best?;
    if winner_score > MAX_WINNING_SCORE {
        return None;
    }
    if alignments
        .iter()
        .any(|(shift, score)| (shift - winner_shift).abs() > 3 && *score <= winner_score + 0.5)
    {
        return None;
    }
    let mut scores: Vec<f64> = alignments.iter().map(|(_, score)| *score).collect();
    scores.sort_by(f64::total_cmp);
    let median = scores[scores.len() >> 1];
    (winner_score < median * MAX_WINNING_SCORE_RATIO).then_some(winner_shift)
}

fn row_profile(
    image: &LumaImage,
    row_start: usize,
    row_count: usize,
    column_start: usize,
    column_count: usize,
) -> Vec<f64> {
    (0..row_count)
        .map(|row| {
            let base = (row_start + row) * image.width + column_start;
            let total: f64 = image.luma[base..base + column_count]
                .iter()
                .map(|value| f64::from(*value))
                .sum();
            total / column_count as f64
        })
        .collect()
}

fn mean_absolute_deviation(profile: &[f64]) -> f64 {
    if profile.is_empty() {
        return 0.0;
    }
    let mean = profile.iter().sum::<f64>() / profile.len() as f64;
    profile
        .iter()
        .map(|value| (value - mean).abs())
        .sum::<f64>()
        / profile.len() as f64
}

const MAX_GEARING_KEYS: usize = 64;
pub const MIN_SCROLL_GEARING: f64 = 0.05;
pub const MAX_SCROLL_GEARING: f64 = 50.0;
pub const MIN_LEARNABLE_SCROLL_INJECTION: f64 = 30.0;
pub const SCROLL_GEARING_SMOOTHING: f64 = 0.5;

pub(crate) fn js_sign(value: f64) -> f64 {
    if value == 0.0 { value } else { value.signum() }
}

pub(crate) fn observed_gearing(injected: f64, traveled: f64) -> Option<f64> {
    if !injected.is_finite() || !traveled.is_finite() {
        return None;
    }
    if traveled == 0.0 || injected.abs() < MIN_LEARNABLE_SCROLL_INJECTION {
        return None;
    }
    if traveled.signum() != injected.signum() {
        return None;
    }
    Some(traveled / injected)
        .filter(|observed| (MIN_SCROLL_GEARING..=MAX_SCROLL_GEARING).contains(observed))
}

pub(crate) fn smoothed_gearing(previous: Option<f64>, observed: f64) -> f64 {
    previous
        .map_or(observed, |previous| {
            previous * (1.0 - SCROLL_GEARING_SMOOTHING) + observed * SCROLL_GEARING_SMOOTHING
        })
        .clamp(MIN_SCROLL_GEARING, MAX_SCROLL_GEARING)
}

#[derive(Debug, Clone, Default)]
pub struct ScrollGearingStore {
    gearings: Vec<(String, f64)>,
}

impl ScrollGearingStore {
    fn get(&self, key: Option<&str>) -> Option<f64> {
        let key = key?;
        self.gearings
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, gearing)| *gearing)
    }

    pub fn gearing(&self, key: Option<&str>) -> f64 {
        self.get(key).unwrap_or(1.0)
    }

    pub fn has(&self, key: Option<&str>) -> bool {
        self.get(key).is_some()
    }

    pub fn plan(&self, key: Option<&str>, requested: f64, fallback: Option<f64>) -> f64 {
        if !requested.is_finite() || requested == 0.0 {
            return 0.0;
        }
        let scaled = requested / self.get(key).or(fallback).unwrap_or(1.0);
        if scaled.abs() < 1.0 {
            return js_sign(scaled);
        }
        scaled
    }

    pub fn learn(&mut self, key: Option<&str>, injected: f64, traveled: f64) -> bool {
        let Some(key) = key else {
            return false;
        };
        let Some(observed) = observed_gearing(injected, traveled) else {
            return false;
        };
        let next = smoothed_gearing(self.get(Some(key)), observed);
        match self
            .gearings
            .iter_mut()
            .find(|(candidate, _)| candidate == key)
        {
            Some((_, gearing)) => *gearing = next,
            None => {
                if self.gearings.len() >= MAX_GEARING_KEYS {
                    self.gearings.remove(0);
                }
                self.gearings.push((key.to_owned(), next));
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffff_u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    0xedb8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
        }
        crc ^ 0xffff_ffff
    }

    fn chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    }

    fn ihdr(width: u32, height: u32, bit_depth: u8, color_type: u8, interlace: u8) -> Vec<u8> {
        let mut data = width.to_be_bytes().to_vec();
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&[bit_depth, color_type, 0, 0, interlace]);
        chunk(b"IHDR", &data)
    }

    fn png(header: Vec<u8>, idat: &[u8]) -> Vec<u8> {
        let mut out = PNG_SIGNATURE.to_vec();
        out.extend(header);
        out.extend(chunk(b"IDAT", idat));
        out.extend(chunk(b"IEND", &[]));
        out
    }

    fn deflate(raw: &[u8]) -> Vec<u8> {
        miniz_oxide::deflate::compress_to_vec_zlib(raw, 6)
    }

    fn encode_png_with(
        width: u32,
        height: u32,
        color_type: u8,
        samples: &[u8],
        bit_depth: u8,
        interlace: u8,
    ) -> Vec<u8> {
        let channels = if color_type == 0 { 1 } else { 3 };
        let stride = width as usize * channels;
        let mut raw = Vec::with_capacity((stride + 1) * height as usize);
        for row in 0..height as usize {
            raw.push(0);
            raw.extend_from_slice(&samples[row * stride..(row + 1) * stride]);
        }
        png(
            ihdr(width, height, bit_depth, color_type, interlace),
            &deflate(&raw),
        )
    }

    fn gray_png(width: u32, height: u32, luma: &[u8]) -> Vec<u8> {
        encode_png_with(width, height, 0, luma, 8, 0)
    }

    fn noise(width: usize, height: usize, seed: u32) -> Vec<u8> {
        let mut state = seed;
        (0..width * height)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 16) & 0xff) as u8
            })
            .collect()
    }

    fn page_window(page: &[u8], width: usize, height: usize, top_row: usize) -> LumaImage {
        let bytes = gray_png(
            width as u32,
            height as u32,
            &page[top_row * width..(top_row + height) * width],
        );
        decode_png_luma(&bytes).unwrap()
    }

    #[test]
    fn round_trips_an_8_bit_grayscale_image() {
        let (width, height) = (8, 4);
        let luma: Vec<u8> = (0..width * height).map(|i| ((i * 7) % 256) as u8).collect();
        let decoded = decode_png_luma(&gray_png(width as u32, height as u32, &luma)).unwrap();
        assert_eq!((decoded.width, decoded.height), (width, height));
        assert_eq!(decoded.luma, luma);
    }

    #[test]
    fn converts_rgb_samples_to_luma() {
        let samples = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let decoded = decode_png_luma(&encode_png_with(4, 1, 2, &samples, 8, 0)).unwrap();
        assert_eq!(decoded.luma, [76, 149, 29, 255]);
    }

    #[test]
    fn survives_every_scanline_filter() {
        let (width, height) = (6, 5);
        let stride = width * 3;
        let samples = noise(stride, height, 99);
        let mut raw = vec![0_u8; (stride + 1) * height];
        let mut previous = vec![0_u8; stride];
        for row in 0..height {
            let filter = row as u8;
            raw[row * (stride + 1)] = filter;
            for index in 0..stride {
                let value = samples[row * stride + index];
                let left = if index >= 3 {
                    samples[row * stride + index - 3]
                } else {
                    0
                };
                let up = previous[index];
                let up_left = if index >= 3 { previous[index - 3] } else { 0 };
                let predicted = match filter {
                    1 => left,
                    2 => up,
                    3 => ((u16::from(left) + u16::from(up)) >> 1) as u8,
                    4 => paeth(left, up, up_left),
                    _ => 0,
                };
                raw[row * (stride + 1) + 1 + index] = value.wrapping_sub(predicted);
            }
            previous.copy_from_slice(&samples[row * stride..(row + 1) * stride]);
        }
        let decoded = decode_png_luma(&png(
            ihdr(width as u32, height as u32, 8, 2, 0),
            &deflate(&raw),
        ))
        .unwrap();
        assert_eq!(decoded.width, width);
        let expected = decode_png_luma(&encode_png_with(
            width as u32,
            height as u32,
            2,
            &samples,
            8,
            0,
        ))
        .unwrap();
        assert_eq!(decoded.luma, expected.luma);
    }

    #[test]
    fn declines_what_it_does_not_decode_instead_of_throwing() {
        let luma = noise(4, 4, 7);
        assert!(decode_png_luma(&encode_png_with(4, 4, 0, &luma, 8, 1)).is_none());
        assert!(decode_png_luma(&encode_png_with(4, 4, 0, &luma, 16, 0)).is_none());
        let palette = png(ihdr(4, 4, 8, 3, 0), &deflate(&[0; 20]));
        assert!(decode_png_luma(&palette).is_none());
    }

    #[test]
    fn declines_malformed_bytes_instead_of_throwing() {
        assert!(decode_png_luma(&[]).is_none());
        assert!(decode_png_luma(&[1, 2, 3, 4, 5, 6, 7, 8, 9]).is_none());
        let bad_idat = png(ihdr(4, 4, 8, 0, 0), b"not compressed");
        assert!(decode_png_luma(&bad_idat).is_none());
        assert!(decode_png_luma(&bad_idat[..20]).is_none());
    }

    #[test]
    fn declines_a_stream_that_inflates_past_the_declared_image() {
        let oversized = png(ihdr(4, 4, 8, 0, 0), &deflate(&[0; 4096]));
        assert!(decode_png_luma(&oversized).is_none());
    }

    #[test]
    fn refuses_repeating_rows_with_multiple_equally_plausible_offsets() {
        let (width, height) = (120, 400);
        let frame = |offset: usize| LumaImage {
            width,
            height,
            luma: (0..width * height)
                .map(|index| {
                    if (index / width + offset) % 40 < 5 {
                        20
                    } else {
                        240
                    }
                })
                .collect(),
        };
        assert_eq!(
            estimate_vertical_travel(&frame(0), &frame(17), None, None),
            None
        );
    }

    const WIDTH: usize = 64;
    const HEIGHT: usize = 400;

    fn page() -> Vec<u8> {
        noise(WIDTH, HEIGHT + 300, 20_260_822)
    }

    #[test]
    fn recovers_a_downward_scroll_as_positive_travel() {
        let page = page();
        let before = page_window(&page, WIDTH, HEIGHT, 150);
        let after = page_window(&page, WIDTH, HEIGHT, 187);
        assert_eq!(
            estimate_vertical_travel(&before, &after, None, None),
            Some(37)
        );
    }

    #[test]
    fn recovers_an_upward_scroll_as_negative_travel() {
        let page = page();
        let before = page_window(&page, WIDTH, HEIGHT, 150);
        let after = page_window(&page, WIDTH, HEIGHT, 67);
        assert_eq!(
            estimate_vertical_travel(&before, &after, None, None),
            Some(-83)
        );
    }

    #[test]
    fn requires_substantial_shared_content_before_learning_a_large_scroll() {
        let page = page();
        let before = page_window(&page, WIDTH, HEIGHT, 0);
        let enough = page_window(&page, WIDTH, HEIGHT, 192);
        let too_little = page_window(&page, WIDTH, HEIGHT, 193);
        assert_eq!(
            estimate_vertical_travel(&before, &enough, None, None),
            Some(192)
        );
        assert_eq!(
            estimate_vertical_travel(&before, &too_little, None, None),
            None
        );
    }

    #[test]
    fn reports_zero_for_a_window_that_did_not_move() {
        let before = page_window(&page(), WIDTH, HEIGHT, 150);
        assert_eq!(
            estimate_vertical_travel(&before, &before, None, None),
            Some(0)
        );
    }

    #[test]
    fn measures_a_side_scroller_at_the_scroll_point_instead_of_the_window_center() {
        let (width, height, strip) = (800, 400, 200);
        let page = noise(width, height + 40, 7_331);
        let frame = |strip_top: usize| LumaImage {
            width,
            height,
            luma: (0..width * height)
                .map(|index| {
                    let (row, column) = (index / width, index % width);
                    let top = if column < strip { strip_top } else { 0 };
                    page[(row + top) * width + column]
                })
                .collect(),
        };
        let (before, after) = (frame(0), frame(40));
        assert_eq!(
            estimate_vertical_travel(&before, &after, None, None),
            Some(0)
        );
        assert_eq!(
            estimate_vertical_travel(&before, &after, None, Some(0.125)),
            Some(40)
        );
        assert_eq!(
            estimate_vertical_travel(&before, &after, None, Some(0.75)),
            Some(0)
        );
    }

    #[test]
    fn refuses_a_shifted_pair_whose_best_alignment_is_still_a_poor_match() {
        let before = page_window(&page(), WIDTH, HEIGHT, 150);
        let after = LumaImage {
            luma: before
                .luma
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    ((u64::from(*value) + (index as u64 * 2_654_435_761) % 41) & 0xff) as u8
                })
                .collect(),
            ..before.clone()
        };
        assert_eq!(estimate_vertical_travel(&before, &after, None, None), None);
    }

    #[test]
    fn refuses_a_blank_window_where_any_shift_matches() {
        let blank = LumaImage {
            width: WIDTH,
            height: HEIGHT,
            luma: vec![128; WIDTH * HEIGHT],
        };
        assert_eq!(estimate_vertical_travel(&blank, &blank, None, None), None);
    }

    #[test]
    fn refuses_captures_of_different_sizes() {
        let page = page();
        let before = page_window(&page, WIDTH, HEIGHT, 150);
        let after = page_window(&page, WIDTH, HEIGHT - 40, 150);
        assert_eq!(estimate_vertical_travel(&before, &after, None, None), None);
    }

    #[test]
    fn refuses_a_capture_too_short_to_correlate() {
        let short = decode_png_luma(&gray_png(WIDTH as u32, 40, &noise(WIDTH, 40, 3))).unwrap();
        assert_eq!(estimate_vertical_travel(&short, &short, None, None), None);
    }

    #[test]
    fn honors_a_caller_supplied_shift_ceiling() {
        let page = page();
        let before = page_window(&page, WIDTH, HEIGHT, 150);
        let after = page_window(&page, WIDTH, HEIGHT, 187);
        assert_eq!(
            estimate_vertical_travel(&before, &after, Some(10.0), None),
            None
        );
    }

    #[test]
    fn assumes_pixel_true_until_it_has_measured_otherwise() {
        let mut store = ScrollGearingStore::default();
        assert_eq!(store.gearing(Some("w1")), 1.0);
        assert_eq!(store.plan(Some("w1"), 400.0, None), 400.0);
        assert_eq!(store.plan(None, -250.0, None), -250.0);
        assert_eq!(store.plan(Some("w1"), 0.0, None), 0.0);
        assert!(!store.has(Some("w1")));
        assert!(!store.has(None));
        store.learn(Some("w1"), 100.0, 100.0);
        assert!(store.has(Some("w1")));
        assert_eq!(store.gearing(Some("w1")), 1.0);
    }

    #[test]
    fn converges_on_a_7x_browser_in_one_measurement() {
        let mut store = ScrollGearingStore::default();
        let device = 7.0;
        let first = store.plan(Some("browser"), 400.0, None);
        assert_eq!(first, 400.0);
        store.learn(Some("browser"), first, first * device);
        assert_eq!(store.gearing(Some("browser")), 7.0);
        let second = store.plan(Some("browser"), 400.0, None);
        assert!((second - 400.0 / 7.0).abs() < 1e-6);
        assert!((second * device - 400.0).abs() < 1e-6);
        store.learn(Some("browser"), second, second * device);
        assert!((store.gearing(Some("browser")) - 7.0).abs() < 1e-6);
    }

    #[test]
    fn smooths_later_observations_rather_than_chasing_one_of_them() {
        let mut store = ScrollGearingStore::default();
        store.learn(Some("w1"), 400.0, 1_600.0);
        assert_eq!(store.gearing(Some("w1")), 4.0);
        store.learn(Some("w1"), 400.0, 3_200.0);
        assert_eq!(store.gearing(Some("w1")), 6.0);
    }

    #[test]
    fn drops_samples_that_cannot_mean_anything() {
        let mut store = ScrollGearingStore::default();
        store.learn(Some("w1"), 400.0, 0.0);
        store.learn(Some("w1"), 400.0, -2_800.0);
        store.learn(Some("w1"), 20.0, 140.0);
        store.learn(Some("w1"), 400.0, 400_000.0);
        store.learn(Some("w1"), 400.0, 4.0);
        store.learn(None, 400.0, 2_800.0);
        assert_eq!(store.gearing(Some("w1")), 1.0);
        assert_eq!(store.plan(Some("w1"), 400.0, None), 400.0);
    }

    #[test]
    fn never_plans_a_nonzero_request_down_to_nothing() {
        let mut store = ScrollGearingStore::default();
        store.learn(Some("w1"), 400.0, 400.0 * 50.0);
        assert_eq!(store.gearing(Some("w1")), 50.0);
        assert_eq!(store.plan(Some("w1"), 10.0, None), 1.0);
        assert_eq!(store.plan(Some("w1"), -10.0, None), -1.0);
    }

    #[test]
    fn plans_an_underflowed_request_to_zero_like_math_sign() {
        let mut store = ScrollGearingStore::default();
        store.learn(Some("w1"), 400.0, 400.0 * 50.0);
        let tiny = f64::from_bits(1);
        assert_eq!(tiny / 50.0, 0.0);
        assert_eq!(store.plan(Some("w1"), tiny, None), 0.0);
        assert_eq!(store.plan(Some("w1"), -tiny, None), 0.0);
        assert_eq!(js_sign(0.0), 0.0);
        assert!(js_sign(-0.0).is_sign_negative() && js_sign(-0.0) == 0.0);
        assert_eq!(js_sign(-3.0), -1.0);
    }

    #[test]
    fn forgets_the_oldest_window_rather_than_growing_without_bound() {
        let mut store = ScrollGearingStore::default();
        for index in 0..64 {
            store.learn(Some(&format!("w{index}")), 400.0, 2_800.0);
        }
        assert_eq!(store.gearing(Some("w0")), 7.0);
        store.learn(Some("w64"), 400.0, 2_800.0);
        assert_eq!(store.gearing(Some("w0")), 1.0);
        assert_eq!(store.gearing(Some("w1")), 7.0);
        assert_eq!(store.gearing(Some("w64")), 7.0);
        store.learn(Some("w1"), 400.0, 2_800.0);
        store.learn(Some("w65"), 400.0, 2_800.0);
        assert_eq!(store.gearing(Some("w1")), 1.0);
        assert_eq!(store.gearing(Some("w2")), 7.0);
    }

    #[test]
    fn uses_the_fallback_only_for_an_unmeasured_window() {
        let mut store = ScrollGearingStore::default();
        assert_eq!(store.plan(Some("w1"), 400.0, Some(4.0)), 100.0);
        store.learn(Some("w1"), 400.0, 800.0);
        assert_eq!(store.plan(Some("w1"), 400.0, Some(4.0)), 200.0);
    }
}
