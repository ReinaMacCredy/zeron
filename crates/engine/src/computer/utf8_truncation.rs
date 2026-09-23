const CONTINUATION_MASK: u8 = 0xc0;
const CONTINUATION_MARKER: u8 = 0x80;
const MAX_CONTINUATION_BYTES: usize = 3;

pub const TRUNCATION_MARKER: &str = "…";

fn sequence_length(lead: u8) -> usize {
    if lead < 0x80 {
        1
    } else if lead & 0xe0 == 0xc0 {
        2
    } else if lead & 0xf0 == 0xe0 {
        3
    } else if lead & 0xf8 == 0xf0 {
        4
    } else {
        1
    }
}

pub fn utf8_boundary_before(bytes: &[u8], max_bytes: usize) -> usize {
    let cut = max_bytes.min(bytes.len());
    if cut == 0 {
        return 0;
    }
    let mut lead = cut - 1;
    let mut stepped = 0;
    while lead > 0
        && stepped < MAX_CONTINUATION_BYTES
        && bytes[lead] & CONTINUATION_MASK == CONTINUATION_MARKER
    {
        lead -= 1;
        stepped += 1;
    }
    if lead + sequence_length(bytes[lead]) <= cut {
        cut
    } else {
        lead
    }
}

pub fn decode_utf8_clamped(bytes: &[u8], max_bytes: usize) -> String {
    String::from_utf8_lossy(&bytes[..utf8_boundary_before(bytes, max_bytes)]).into_owned()
}

pub fn clamp_utf8_bytes(text: &str, max_bytes: usize) -> &str {
    &text[..utf8_boundary_before(text.as_bytes(), max_bytes)]
}

pub(crate) fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

// Lengths are UTF-16 units, as proto's bounds (and JS `length`) count them.
pub fn clamp_text_to_length(text: &str, max_length: usize) -> String {
    if utf16_len(text) <= max_length {
        return text.to_owned();
    }
    let budget = max_length.saturating_sub(utf16_len(TRUNCATION_MARKER));
    let mut units = 0;
    let mut end = 0;
    for (index, character) in text.char_indices() {
        units += character.len_utf16();
        if units > budget {
            break;
        }
        end = index + character.len_utf8();
    }
    format!("{}{TRUNCATION_MARKER}", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLACEMENT: char = '\u{fffd}';
    const ACCENT_2: &str = "é";
    const EURO_3: &str = "€";
    const EMOJI_4: &str = "\u{1f600}";

    #[test]
    fn clamp_leaves_text_under_the_cap_untouched() {
        assert_eq!(clamp_utf8_bytes("hello", 64), "hello");
        let wide = format!("{EMOJI_4}{EURO_3}");
        assert_eq!(clamp_utf8_bytes(&wide, 64), wide);
    }

    #[test]
    fn clamp_leaves_text_sitting_exactly_on_the_cap_untouched() {
        assert_eq!(clamp_utf8_bytes("hello", 5), "hello");
        let wide = format!("{EMOJI_4}{EURO_3}");
        assert_eq!(clamp_utf8_bytes(&wide, 7), wide);
    }

    #[test]
    fn clamp_cuts_ascii_at_the_cap() {
        assert_eq!(clamp_utf8_bytes("abcdef", 3), "abc");
    }

    #[test]
    fn clamp_drops_a_multi_byte_character_that_straddles_the_cap() {
        for character in [ACCENT_2, EURO_3, EMOJI_4] {
            for kept in 1..character.len() {
                let text = format!("ab{character}cd");
                let clamped = clamp_utf8_bytes(&text, 2 + kept);
                assert_eq!(clamped, "ab");
                assert!(!clamped.contains(REPLACEMENT));
            }
        }
    }

    #[test]
    fn clamp_keeps_a_multi_byte_character_that_ends_exactly_on_the_cap() {
        assert_eq!(
            clamp_utf8_bytes(&format!("ab{EMOJI_4}cd"), 6),
            format!("ab{EMOJI_4}")
        );
        assert_eq!(
            clamp_utf8_bytes(&format!("ab{EURO_3}cd"), 5),
            format!("ab{EURO_3}")
        );
        assert_eq!(
            clamp_utf8_bytes(&format!("ab{ACCENT_2}cd"), 4),
            format!("ab{ACCENT_2}")
        );
    }

    #[test]
    fn clamp_never_emits_a_replacement_character_for_a_run_of_wide_characters() {
        let text = EMOJI_4.repeat(64);
        for cap in 0..=text.len() {
            let clamped = clamp_utf8_bytes(&text, cap);
            assert!(!clamped.contains(REPLACEMENT));
            assert!(clamped.len() <= cap);
            assert!(text.starts_with(clamped));
        }
    }

    #[test]
    fn clamp_returns_empty_when_the_first_character_is_wider_than_the_cap() {
        assert_eq!(clamp_utf8_bytes(EMOJI_4, 3), "");
        assert_eq!(clamp_utf8_bytes(EMOJI_4, 0), "");
    }

    #[test]
    fn decode_leaves_a_buffer_that_fits_unchanged() {
        let text = format!("hi{EMOJI_4}");
        assert_eq!(decode_utf8_clamped(text.as_bytes(), 64), text);
    }

    #[test]
    fn decode_trims_a_buffer_already_cut_mid_character_upstream() {
        let limit = 4;
        let full = format!("abc{EURO_3}def");
        let truncated = &full.as_bytes()[..limit];
        assert!(String::from_utf8_lossy(truncated).contains(REPLACEMENT));
        assert_eq!(decode_utf8_clamped(truncated, limit), "abc");
    }

    #[test]
    fn boundary_reports_the_buffer_length_when_nothing_needs_dropping() {
        let bytes = EMOJI_4.as_bytes();
        assert_eq!(utf8_boundary_before(bytes, 4), 4);
        assert_eq!(utf8_boundary_before(bytes, 99), 4);
    }

    #[test]
    fn boundary_cuts_at_the_cap_for_malformed_bytes() {
        let bytes = [0x80; 6];
        assert_eq!(utf8_boundary_before(&bytes, 4), 4);
    }

    #[test]
    fn text_clamp_counts_utf16_units_and_keeps_surrogate_pairs_whole() {
        assert_eq!(clamp_text_to_length("short", 10), "short");
        assert_eq!(clamp_text_to_length("abcdef", 4), "abc…");
        let text = format!("{}{EMOJI_4}bbb", "a".repeat(3));
        let clamped = clamp_text_to_length(&text, 5);
        assert_eq!(clamped, "aaa…");
        assert!(utf16_len(&clamped) <= 5);
        assert_eq!(clamp_text_to_length("abc", 0), "…");
    }
}
