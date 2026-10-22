//! Transcript text the user selected and added to the chat, staged on the
//! composer and folded into the next prompt as a quoted block.
//!
//! [`with_quotes`] appends them; [`extract`] reads the same block back out
//! for the transcript. There is no second data model.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    pub id: String,
    pub text: String,
    pub terminal: bool,
}

impl Quote {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            text: text.into(),
            terminal: false,
        }
    }

    pub fn terminal(text: impl Into<String>) -> Self {
        Self {
            id: TERMINAL_QUOTE_ID.to_string(),
            text: text.into(),
            terminal: true,
        }
    }

    pub fn location(&self) -> &'static str {
        if self.terminal {
            TERMINAL_LOCATION
        } else {
            DETAIL_LOCATION
        }
    }
}

pub const TERMINAL_QUOTE_ID: &str = "terminal-selection";

pub const QUOTE_ONLY_TEXT: &str = "Refer to the selected text below.";

pub const QUOTE_BLOCK_HEADER: &str = "Selected text from this conversation (quoted):";

pub const DETAIL_LOCATION: &str = "Selected text";

pub const TERMINAL_BLOCK_HEADER: &str = "Selected text from this chat's terminal (quoted):";

pub const TERMINAL_LOCATION: &str = "Terminal selection";

fn quote_block(text: &str) -> String {
    text.trim()
        .lines()
        .map(|line| {
            if line.is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn with_quotes(text: &str, quotes: &[Quote]) -> String {
    if quotes.is_empty() {
        return text.to_string();
    }
    let mut out = if text.is_empty() {
        QUOTE_ONLY_TEXT
    } else {
        text
    }
    .to_string();
    for (terminal, header) in [(false, QUOTE_BLOCK_HEADER), (true, TERMINAL_BLOCK_HEADER)] {
        let blocks: Vec<String> = quotes
            .iter()
            .filter(|q| q.terminal == terminal)
            .map(|q| quote_block(&q.text))
            .collect();
        if !blocks.is_empty() {
            out = format!("{out}\n\n{header}\n{}", blocks.join("\n\n"));
        }
    }
    out
}

pub fn quoted_chunks<'a>(text: &'a str, header: &str) -> Option<(&'a str, Vec<Vec<&'a str>>)> {
    let marker = format!("\n\n{header}\n");
    let at = text.rfind(&marker)?;
    let block = &text[at + marker.len()..];
    if block.is_empty()
        || !block
            .lines()
            .all(|line| line.is_empty() || line.starts_with('>'))
    {
        return None;
    }
    let chunks: Vec<Vec<&str>> = block
        .split("\n\n")
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| {
            chunk
                .lines()
                .map(|line| {
                    line.strip_prefix("> ")
                        .or_else(|| line.strip_prefix('>'))
                        .unwrap_or(line)
                })
                .collect()
        })
        .collect();
    if chunks.is_empty() {
        return None;
    }
    Some((&text[..at], chunks))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotedBlock {
    pub location: &'static str,
    pub label: String,
    pub bodies: Vec<String>,
}

pub fn extract(text: &str) -> Option<(String, QuotedBlock)> {
    extract_block(text, QUOTE_BLOCK_HEADER, DETAIL_LOCATION)
}

pub fn extract_terminal(text: &str) -> Option<(String, QuotedBlock)> {
    extract_block(text, TERMINAL_BLOCK_HEADER, TERMINAL_LOCATION)
}

fn extract_block(
    text: &str,
    header: &str,
    location: &'static str,
) -> Option<(String, QuotedBlock)> {
    let (rest, chunks) = quoted_chunks(text, header)?;
    let bodies: Vec<String> = chunks.into_iter().map(|lines| lines.join("\n")).collect();
    Some((
        rest.to_string(),
        QuotedBlock {
            location,
            label: chip_label(bodies.len()),
            bodies,
        },
    ))
}

pub fn chip_label(count: usize) -> String {
    if count == 1 {
        "1 selection".to_string()
    } else {
        format!("{count} selections")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_leaves_the_prompt_untouched() {
        assert_eq!(with_quotes("hello", &[]), "hello");
    }

    #[test]
    fn quotes_append_as_quoted_blocks() {
        let quotes = vec![Quote::new("first line\nsecond line"), Quote::new("other")];
        assert_eq!(
            with_quotes("explain this", &quotes),
            format!("explain this\n\n{QUOTE_BLOCK_HEADER}\n> first line\n> second line\n\n> other")
        );
    }

    #[test]
    fn quote_only_send_gets_a_body() {
        let out = with_quotes("", &[Quote::new("x")]);
        assert!(out.starts_with(QUOTE_ONLY_TEXT));
    }

    #[test]
    fn blank_lines_inside_a_quote_stay_inside_it() {
        let sent = with_quotes("q", &[Quote::new("a\n\nb")]);
        assert!(sent.ends_with("> a\n>\n> b"));
        let (rest, block) = extract(&sent).expect("block");
        assert_eq!(rest, "q");
        assert_eq!(block.bodies, ["a\n\nb"]);
    }

    #[test]
    fn the_block_round_trips() {
        let quotes = vec![Quote::new("first line\nsecond line"), Quote::new("other")];
        let sent = with_quotes("explain this", &quotes);
        let (rest, block) = extract(&sent).expect("block");
        assert_eq!(rest, "explain this");
        assert_eq!(block.label, "2 selections");
        assert_eq!(block.location, DETAIL_LOCATION);
        assert_eq!(block.bodies, ["first line\nsecond line", "other"]);
    }

    #[test]
    fn a_prompt_without_the_block_is_left_alone() {
        assert!(extract("plain prompt\n\n> a quote the user typed").is_none());
    }

    #[test]
    fn chip_label_pluralizes() {
        assert_eq!(chip_label(1), "1 selection");
        assert_eq!(chip_label(3), "3 selections");
    }
}
