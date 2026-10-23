//! Canonical attachment chips shared by the editor, transcript and harness
//! delivery.
//!
//! A composer attachment is referenced from the prompt by a strict local
//! Markdown link carrying only its per-draft number, never a path. Images read
//! `[Image N]`; other files carry their file name. The editor projects either
//! to a chip; providers receive the plain label, which pairs with the upload of
//! the same name.
use std::ops::Range;

pub const IMAGE_MENTION_SCHEME: &str = "zeron-image:";
pub const ATTACHMENT_MENTION_SCHEME: &str = "zeron-attachment:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentMention {
    pub range: Range<usize>,
    pub index: u32,
    /// `Image N` for images, the file name for other attachments.
    pub label: String,
    pub is_image: bool,
}

impl AttachmentMention {
    /// Whether this chip names the attachment at `path`. Images match by
    /// draft number (`Image 2` ↔ `ab12cd34-Image_2.png`), files by name as
    /// the engine sanitizes it.
    pub fn names_attachment(&self, path: &str) -> bool {
        if self.is_image {
            return image_index_from_name(path) == Some(self.index);
        }
        let sanitized = |name: &str| -> String {
            name.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        };
        let name = attachment_display_name(path.rsplit(['/', '\\']).next().unwrap_or(path));
        sanitized(&self.label) == sanitized(name)
    }
}

/// The user-visible name of the `index`th image of a draft.
pub fn image_label(index: u32) -> String {
    format!("Image {index}")
}

/// The chip link for attachment `index`: an image when `file_name` is `None`.
pub fn attachment_mention_link(index: u32, file_name: Option<&str>) -> String {
    match file_name {
        None => format!("[{}]({IMAGE_MENTION_SCHEME}{index})", image_label(index)),
        Some(name) => format!(
            "[{}]({ATTACHMENT_MENTION_SCHEME}{index})",
            crate::file_mentions::escape_mention_label(name)
        ),
    }
}

/// Recover the draft number from an upload or staged image name such as
/// `Image 2.png` / `ab12cd34-Image_2.png` (uploads sanitize spaces).
pub fn image_index_from_name(name: &str) -> Option<u32> {
    let name = name.rsplit(['/', '\\']).next()?;
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let digits_at = stem.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (head, digits) = stem.split_at(digits_at);
    let head = head.strip_suffix([' ', '_'])?;
    if !(head == "Image" || head.ends_with("-Image")) {
        return None;
    }
    digits.parse().ok().filter(|index| *index > 0)
}

pub fn pasted_text_label(index: u32) -> String {
    format!("Pasted text {index}.txt")
}

pub fn pasted_text_index_from_name(name: &str) -> Option<u32> {
    let name = attachment_display_name(name.rsplit(['/', '\\']).next()?);
    let digits = name
        .strip_suffix(".txt")?
        .strip_prefix("Pasted")?
        .strip_prefix([' ', '_'])?
        .strip_prefix("text")?
        .strip_prefix([' ', '_'])?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|index| *index > 0)
}

pub const PASTED_TEXT_MIN_CHARS: usize = 5_000;

/// A leading skill command's arguments stay inline, however long.
pub fn pastes_as_file(pasted: &str, draft: &str) -> bool {
    pasted.chars().count() >= PASTED_TEXT_MIN_CHARS
        && attachment_mentions(pasted).is_empty()
        && !crate::invocation::leading_command(&crate::invocation::invocation_prompt(draft))
            .is_some_and(|(name, _)| crate::invocation::valid_skill_command_name(name))
}

pub fn next_pasted_text_name<'a>(staged: impl IntoIterator<Item = &'a str>) -> String {
    let last = staged
        .into_iter()
        .filter_map(pasted_text_index_from_name)
        .max()
        .unwrap_or(0);
    pasted_text_label(last + 1)
}

pub fn pasted_text_pill_label(index: u32, lines: Option<usize>) -> String {
    match lines {
        Some(lines) => format!("Pasted text {index} · {}", pasted_text_lines_label(lines)),
        None => format!("Pasted text {index}"),
    }
}

pub fn pasted_text_lines_label(lines: usize) -> String {
    if lines == 1 {
        "1 line".to_owned()
    } else {
        format!("{lines} lines")
    }
}

/// The body used for image-only sends.
pub const ATTACHMENT_ONLY_TEXT: &str = "See the attached image(s).";
/// The body used when any attachment is not an image.
pub const FILE_ATTACHMENT_ONLY_TEXT: &str = "See the attached file(s).";

/// Whether `text` is the placeholder body of an attachments-only send.
pub fn is_attachment_only_text(text: &str) -> bool {
    text == ATTACHMENT_ONLY_TEXT || text == FILE_ATTACHMENT_ONLY_TEXT
}

/// The placeholder body for an attachments-only send of `paths`.
pub fn attachment_only_text(paths: &[String]) -> &'static str {
    if paths.iter().all(|path| is_image_path(path)) {
        ATTACHMENT_ONLY_TEXT
    } else {
        FILE_ATTACHMENT_ONLY_TEXT
    }
}

/// Whether an attachment ref names an image, as opposed to any other file.
/// The same extensions the desktop decodes (`format_by_extension`), so every
/// client shows a given attachment the same way.
pub fn is_image_path(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff"
        )
    })
}

/// The file name to show for an attachment ref: uploads are stored as
/// `{id8}-{name}`, and the prefix is not part of the name.
pub fn attachment_display_name(name: &str) -> &str {
    match name.split_once('-') {
        Some((id, rest)) if id.len() == 8 && id.bytes().all(|b| b.is_ascii_hexdigit()) => rest,
        _ => name,
    }
}

/// Undo [`crate::file_mentions::escape_mention_label`], rejecting any label
/// that is not exactly what escaping produces.
fn unescape_label(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next()? {
                escaped @ ('\\' | '[' | ']' | '`') => out.push(escaped),
                _ => return None,
            },
            '[' | ']' | '`' => return None,
            c if c.is_control() => return None,
            c => out.push(c),
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Only canonical chip links are decoded, never escaped text or code examples.
pub fn attachment_mentions(text: &str) -> Vec<AttachmentMention> {
    if !text.contains(IMAGE_MENTION_SCHEME) && !text.contains(ATTACHMENT_MENTION_SCHEME) {
        return Vec::new();
    }
    let mut image_depth = 0;
    pulldown_cmark::Parser::new(text)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            match &event {
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { .. }) => {
                    image_depth += 1;
                }
                pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Image) => {
                    image_depth -= 1;
                }
                _ => {}
            }
            if image_depth > 0 {
                return None;
            }
            let pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { dest_url, .. }) = event
            else {
                return None;
            };
            let (scheme, is_image) = if dest_url.starts_with(IMAGE_MENTION_SCHEME) {
                (IMAGE_MENTION_SCHEME, true)
            } else {
                (ATTACHMENT_MENTION_SCHEME, false)
            };
            let digits = dest_url.strip_prefix(scheme)?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let index: u32 = digits.parse().ok().filter(|index| *index > 0)?;
            let source = &text[range.clone()];
            let label = if is_image {
                (source == attachment_mention_link(index, None)).then(|| image_label(index))?
            } else {
                let tail = format!("]({scheme}{index})");
                unescape_label(source.strip_prefix('[')?.strip_suffix(&tail)?)?
            };
            Some(AttachmentMention {
                range,
                index,
                label,
                is_image,
            })
        })
        .collect()
}

/// The attachment numbers a draft mentions, in order of appearance, without
/// repeats.
pub fn attachment_mention_indices(text: &str) -> Vec<u32> {
    let mut seen = Vec::new();
    for mention in attachment_mentions(text) {
        if !seen.contains(&mention.index) {
            seen.push(mention.index);
        }
    }
    seen
}

/// Replace chips with their plain label. Providers read prose, and the label
/// pairs with the attached upload.
pub fn attachment_mention_prompt(text: &str) -> String {
    replace_mentions(text, |_| true)
}

/// Replace chips whose attachment is not attached by plain text, so a dangling
/// reference never leaves the composer as a link.
pub fn demote_unattached_mentions(text: &str, attached: &[u32]) -> String {
    replace_mentions(text, |index| !attached.contains(&index))
}

fn replace_mentions(text: &str, replace: impl Fn(u32) -> bool) -> String {
    let mut out = String::new();
    let mut at = 0;
    for mention in attachment_mentions(text) {
        if !replace(mention.index) {
            continue;
        }
        out.push_str(&text[at..mention.range.start]);
        out.push_str(&mention.label);
        at = mention.range.end;
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(index: u32) -> String {
        attachment_mention_link(index, None)
    }

    #[test]
    fn links_round_trip_and_become_plain_labels() {
        assert_eq!(image(2), "[Image 2](zeron-image:2)");
        let notes = attachment_mention_link(3, Some("my [notes].md"));
        assert_eq!(notes, "[my \\[notes\\].md](zeron-attachment:3)");
        let text = format!("compare {} with {notes} and {}", image(2), image(10));
        let mentions = attachment_mentions(&text);
        assert_eq!(
            mentions
                .iter()
                .map(|m| (m.index, m.label.as_str(), m.is_image))
                .collect::<Vec<_>>(),
            vec![
                (2, "Image 2", true),
                (3, "my [notes].md", false),
                (10, "Image 10", true)
            ]
        );
        assert_eq!(&text[mentions[1].range.clone()], notes);
        assert_eq!(
            attachment_mention_prompt(&text),
            "compare Image 2 with my [notes].md and Image 10"
        );
        assert_eq!(
            attachment_mention_indices(&format!("{}{}", image(2), image(2))),
            vec![2]
        );
    }

    #[test]
    fn hostile_and_literal_links_stay_ordinary_text() {
        let link = image(1);
        let file = attachment_mention_link(1, Some("a.md"));
        for literal in [
            format!("`{link}`"),
            format!("```\n{link}\n```"),
            format!("\\{link}"),
            format!("![example {link}](example.png)"),
            format!("`{file}`"),
            format!("\\{file}"),
            "[Image 1](zeron-image:0)".to_string(),
            "[Image 1](zeron-image:)".to_string(),
            "[Image 1](zeron-image:1x)".to_string(),
            "[Image 1](zeron-image:-1)".to_string(),
            "[Image 1](zeron-image:../x)".to_string(),
            "[Image 2](zeron-image:1)".to_string(),
            "[Other](zeron-image:1)".to_string(),
            "[Image 1](zeron-image:99999999999)".to_string(),
            "[Image 01](zeron-image:01)".to_string(),
            "[](zeron-attachment:1)".to_string(),
            "[a.md](zeron-attachment:0)".to_string(),
            "[a.md](zeron-attachment:1 \"title\")".to_string(),
            "[a\nb.md](zeron-attachment:1)".to_string(),
            "[a`b.md](zeron-attachment:1)".to_string(),
            "[a\\xb.md](zeron-attachment:1)".to_string(),
        ] {
            assert!(attachment_mentions(&literal).is_empty(), "{literal}");
            assert_eq!(attachment_mention_prompt(&literal), literal);
        }
    }

    #[test]
    fn unattached_mentions_demote_to_labels_only() {
        let text = format!(
            "{} and {}",
            image(1),
            attachment_mention_link(2, Some("a.md"))
        );
        assert_eq!(
            demote_unattached_mentions(&text, &[2]),
            format!("Image 1 and {}", attachment_mention_link(2, Some("a.md")))
        );
    }

    #[test]
    fn image_names_recover_their_number() {
        for (name, index) in [
            ("Image 3.png", Some(3)),
            ("ab12cd34-Image_12.png", Some(12)),
            ("/uploads/ab-Image_2.webp", Some(2)),
            ("Image 0.png", None),
            ("image.png", None),
            ("Imaged 2.png", None),
            ("cat.png", None),
        ] {
            assert_eq!(image_index_from_name(name), index, "{name}");
        }
    }

    #[test]
    fn pasted_text_names_recover_their_number() {
        assert_eq!(pasted_text_label(2), "Pasted text 2.txt");
        for (name, index) in [
            ("Pasted text 2.txt", Some(2)),
            ("ab12cd34-Pasted_text_2.txt", Some(2)),
            ("/uploads/ab12cd34-Pasted_text_14.txt", Some(14)),
            ("Pasted text 0.txt", None),
            ("Pasted text.txt", None),
            ("Pasted text 2.md", None),
            ("my-Pasted_text_2.txt", None),
            ("Image 2.png", None),
            ("notes.txt", None),
        ] {
            assert_eq!(pasted_text_index_from_name(name), index, "{name}");
        }
        assert_eq!(next_pasted_text_name([]), "Pasted text 1.txt");
        assert_eq!(
            next_pasted_text_name(["Pasted text 3.txt", "Image 1.jpg", "Pasted text 1.txt"]),
            "Pasted text 4.txt"
        );
    }

    #[test]
    fn only_long_plain_pastes_outside_a_skill_command_stage_as_files() {
        let long = "x".repeat(PASTED_TEXT_MIN_CHARS);
        assert!(pastes_as_file(&long, ""));
        assert!(pastes_as_file(&long, "Review this:"));
        assert!(!pastes_as_file(&long[1..], ""));
        assert!(!pastes_as_file(
            &format!("{long} {}", attachment_mention_link(1, None)),
            ""
        ));
        assert!(!pastes_as_file(&long, "/review "));
    }

    #[test]
    fn a_files_only_send_says_files_not_images() {
        let image = vec!["/u/ab12cd34-Image_1.jpg".to_owned()];
        let mixed = vec![image[0].clone(), "/u/ab12cd34-Pasted_text_1.txt".to_owned()];
        assert_eq!(attachment_only_text(&image), ATTACHMENT_ONLY_TEXT);
        assert_eq!(attachment_only_text(&mixed), FILE_ATTACHMENT_ONLY_TEXT);
        assert_eq!(pasted_text_pill_label(2, Some(1)), "Pasted text 2 · 1 line");
        assert_eq!(pasted_text_pill_label(2, Some(40)), "Pasted text 2 · 40 lines");
    }

    #[test]
    fn attachment_refs_split_images_from_files() {
        assert!(is_image_path("/uploads/ab12cd34-shot.PNG"));
        assert!(is_image_path("pending://u1/Image_1.jpeg"));
        assert!(!is_image_path("/uploads/ab12cd34-notes.zip"));
        assert!(!is_image_path("/uploads/png"));
        assert_eq!(attachment_display_name("ab12cd34-notes.zip"), "notes.zip");
        assert_eq!(attachment_display_name("my-notes.zip"), "my-notes.zip");
    }
}
