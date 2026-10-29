//! Terminal queries (DSR, DA) are answered by `replies` or the viewer:
//! this one drops `PtyWrite`, or every cursor report would arrive twice.

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};
use zeron_proto::{TerminalColor, TerminalRun};

const SCROLLBACK_LINES: usize = 3_000;

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Clone, Default)]
struct TitleListener(Arc<Mutex<Option<String>>>);

impl EventListener for TitleListener {
    fn send_event(&self, event: Event) {
        let mut title = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match event {
            Event::Title(next) => *title = Some(next),
            Event::ResetTitle => *title = None,
            _ => {}
        }
    }
}

pub(crate) struct Screen {
    term: Term<TitleListener>,
    parser: Processor,
    title: TitleListener,
}

pub(crate) struct ScreenText {
    pub cols: u16,
    pub rows: u16,
    pub alt_screen: bool,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub lines: Vec<String>,
    pub styled_lines: Vec<Vec<TerminalRun>>,
    pub truncated: bool,
}

impl Screen {
    pub fn new(cols: u16, rows: u16) -> Self {
        let title = TitleListener::default();
        let config = Config {
            scrolling_history: SCROLLBACK_LINES,
            ..Config::default()
        };
        let size = Size {
            cols: cols as usize,
            rows: rows as usize,
        };
        Self {
            term: Term::new(config, &size, title.clone()),
            parser: Processor::new(),
            title,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(Size {
            cols: cols as usize,
            rows: rows as usize,
        });
    }

    pub fn title(&self) -> Option<String> {
        self.title
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .filter(|title| !title.trim().is_empty())
    }

    pub fn app_cursor(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    pub fn text(&self, max_lines: usize, styled: bool) -> ScreenText {
        let grid = self.term.grid();
        let cols = grid.columns();
        let rows = grid.screen_lines();
        let cursor = grid.cursor.point;
        let history = grid.history_size() as i32;
        let row_text = |line: i32, runs: Option<&mut Vec<TerminalRun>>| -> (String, bool) {
            let row = &grid[Line(line)];
            let mut text = String::with_capacity(cols);
            let mut runs = runs;
            for col in 0..cols {
                let cell = &row[Column(col)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                let start = text.len();
                text.push(cell.c);
                if let Some(zerowidth) = cell.zerowidth() {
                    text.extend(zerowidth);
                }
                if let Some(runs) = runs.as_deref_mut() {
                    push_run(runs, cell_run(cell, &text[start..]));
                }
            }
            let wrapped = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
            (text, wrapped)
        };
        let mut last = cursor.line.0;
        for line in (cursor.line.0 + 1..rows as i32).rev() {
            if !row_text(line, None).0.trim().is_empty() {
                last = line;
                break;
            }
        }
        let mut lines = Vec::new();
        let mut styled_lines = Vec::new();
        let mut current = String::new();
        let mut current_runs = Vec::new();
        for line in -history..=last {
            let (text, wrapped) = row_text(line, styled.then_some(&mut current_runs));
            current.push_str(&text);
            if !wrapped || line == last {
                lines.push(current.trim_end().to_string());
                current = String::new();
                if styled {
                    styled_lines.push(trim_runs(std::mem::take(&mut current_runs)));
                }
            }
        }
        let truncated = lines.len() > max_lines;
        if truncated {
            lines.drain(..lines.len() - max_lines);
            if styled {
                styled_lines.drain(..styled_lines.len() - max_lines);
            }
        }
        ScreenText {
            cols: cols as u16,
            rows: rows as u16,
            alt_screen: self.term.mode().contains(TermMode::ALT_SCREEN),
            cursor_row: cursor.line.0.max(0) as usize,
            cursor_col: cursor.column.0,
            lines,
            styled_lines,
            truncated,
        }
    }
}

fn terminal_color(color: Color) -> Option<TerminalColor> {
    match color {
        Color::Spec(rgb) => Some(TerminalColor::Rgb(rgb.r, rgb.g, rgb.b)),
        Color::Indexed(index) => Some(TerminalColor::Indexed(index)),
        Color::Named(named) => {
            let index = named as usize;
            if index < 16 {
                return Some(TerminalColor::Indexed(index as u8));
            }
            match named {
                NamedColor::DimBlack
                | NamedColor::DimRed
                | NamedColor::DimGreen
                | NamedColor::DimYellow
                | NamedColor::DimBlue
                | NamedColor::DimMagenta
                | NamedColor::DimCyan
                | NamedColor::DimWhite => Some(TerminalColor::Indexed(
                    (index - NamedColor::DimBlack as usize) as u8,
                )),
                _ => None,
            }
        }
    }
}

fn cell_run(cell: &Cell, text: &str) -> TerminalRun {
    TerminalRun {
        text: text.to_owned(),
        fg: terminal_color(cell.fg),
        bg: terminal_color(cell.bg),
        bold: cell.flags.contains(Flags::BOLD),
        italic: cell.flags.contains(Flags::ITALIC),
        underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
        dim: cell.flags.contains(Flags::DIM),
        inverse: cell.flags.contains(Flags::INVERSE),
    }
}

fn push_run(runs: &mut Vec<TerminalRun>, run: TerminalRun) {
    match runs.last_mut() {
        Some(last) if last.same_style(&run) => last.text.push_str(&run.text),
        _ => runs.push(run),
    }
}

fn trim_runs(mut runs: Vec<TerminalRun>) -> Vec<TerminalRun> {
    while let Some(last) = runs.last_mut() {
        if last.bg.is_some() || last.inverse {
            break;
        }
        let kept = last.text.trim_end().len();
        last.text.truncate(kept);
        if !last.text.is_empty() {
            break;
        }
        runs.pop();
    }
    runs
}

pub(crate) fn key_bytes(key: &str, app_cursor: bool) -> Option<Vec<u8>> {
    let key = key.trim().to_ascii_lowercase();
    let mut parts: Vec<&str> = key.split('+').collect();
    let name = parts.pop().filter(|name| !name.is_empty())?;
    let (mut ctrl, mut alt, mut shift) = (false, false, false);
    for part in parts {
        match part {
            "ctrl" | "control" => ctrl = true,
            "alt" | "option" | "meta" => alt = true,
            "shift" => shift = true,
            _ => return None,
        }
    }
    let ss3 = |csi: &[u8], ss3: &[u8]| if app_cursor { ss3 } else { csi }.to_vec();
    let mut bytes = if ctrl {
        match name.as_bytes() {
            [c @ b'a'..=b'z'] => vec![c & 0x1f],
            b"space" | b"@" | b"2" => vec![0],
            b"[" | b"3" => vec![0x1b],
            b"\\" | b"4" => vec![0x1c],
            b"]" | b"5" => vec![0x1d],
            b"^" | b"6" => vec![0x1e],
            b"_" | b"-" | b"7" => vec![0x1f],
            _ => return None,
        }
    } else {
        match name {
            "enter" | "return" => b"\r".to_vec(),
            "backspace" => vec![0x7f],
            "tab" if shift => b"\x1b[Z".to_vec(),
            "tab" => b"\t".to_vec(),
            "escape" | "esc" => vec![0x1b],
            "space" => b" ".to_vec(),
            "up" => ss3(b"\x1b[A", b"\x1bOA"),
            "down" => ss3(b"\x1b[B", b"\x1bOB"),
            "right" => ss3(b"\x1b[C", b"\x1bOC"),
            "left" => ss3(b"\x1b[D", b"\x1bOD"),
            "home" => ss3(b"\x1b[H", b"\x1bOH"),
            "end" => ss3(b"\x1b[F", b"\x1bOF"),
            "insert" => b"\x1b[2~".to_vec(),
            "delete" => b"\x1b[3~".to_vec(),
            "pageup" => b"\x1b[5~".to_vec(),
            "pagedown" => b"\x1b[6~".to_vec(),
            _ if name.chars().count() == 1 => name.as_bytes().to_vec(),
            _ => return None,
        }
    };
    if alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_joins_wrapped_rows_and_trims_blank_screen_tail() {
        let mut screen = Screen::new(10, 5);
        screen.feed(b"abcdefghijKLM\r\nnext\r\n");
        let text = screen.text(100, false);
        assert_eq!(text.lines, vec!["abcdefghijKLM", "next", ""]);
        assert_eq!((text.cursor_row, text.cursor_col), (3, 0));
        assert!(!text.truncated);
    }

    #[test]
    fn styled_text_carries_colour_runs_beside_the_plain_lines() {
        let mut screen = Screen::new(30, 4);
        screen
            .feed(b"\x1b[1;32mok\x1b[0m done \x1b[38;5;196mred\x1b[0m   \r\n\x1b[41m  \x1b[0m\r\n");
        let text = screen.text(10, true);
        assert_eq!(text.lines[0], "ok done red");
        let runs = &text.styled_lines[0];
        assert_eq!(runs.len(), 3);
        assert_eq!(
            (runs[0].text.as_str(), runs[0].fg, runs[0].bold),
            ("ok", Some(TerminalColor::Indexed(2)), true)
        );
        assert_eq!((runs[1].text.as_str(), runs[1].fg), (" done ", None));
        assert_eq!(
            (runs[2].text.as_str(), runs[2].fg),
            ("red", Some(TerminalColor::Indexed(196)))
        );
        assert_eq!(text.styled_lines[1][0].bg, Some(TerminalColor::Indexed(1)));
        assert_eq!(text.styled_lines[1][0].text, "  ");
        assert_eq!(text.styled_lines.len(), text.lines.len());
        assert!(screen.text(10, false).styled_lines.is_empty());
    }

    #[test]
    fn text_keeps_scrollback_and_bounds_it() {
        let mut screen = Screen::new(20, 3);
        for i in 0..10 {
            screen.feed(format!("line {i}\r\n").as_bytes());
        }
        let text = screen.text(4, false);
        assert_eq!(text.lines, vec!["line 7", "line 8", "line 9", ""]);
        assert!(text.truncated);
    }

    #[test]
    fn alt_screen_title_and_queries_are_tracked_without_replies() {
        let mut screen = Screen::new(20, 4);
        screen.feed(b"\x1b]0;vim notes\x07\x1b[?1049h\x1b[6n\x1b[?1h");
        assert_eq!(screen.title().as_deref(), Some("vim notes"));
        let text = screen.text(10, false);
        assert!(text.alt_screen);
        assert!(screen.app_cursor());
    }

    #[test]
    fn resize_changes_the_grid() {
        let mut screen = Screen::new(10, 4);
        screen.resize(30, 6);
        let text = screen.text(10, false);
        assert_eq!((text.cols, text.rows), (30, 6));
    }

    #[test]
    fn keys_encode_modifiers_and_cursor_mode() {
        assert_eq!(key_bytes("enter", false), Some(b"\r".to_vec()));
        assert_eq!(key_bytes("Ctrl+C", false), Some(vec![3]));
        assert_eq!(key_bytes("up", false), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes("up", true), Some(b"\x1bOA".to_vec()));
        assert_eq!(key_bytes("alt+b", false), Some(b"\x1bb".to_vec()));
        assert_eq!(key_bytes("shift+tab", false), Some(b"\x1b[Z".to_vec()));
        assert_eq!(key_bytes("hyper+x", false), None);
        assert_eq!(key_bytes("enterr", false), None);
    }
}
