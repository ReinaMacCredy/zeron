use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;

use crate::sessions::SessionsEngine;

const TAIL_BYTES: u64 = 32 * 1024;
const TAIL_LINES: usize = 200;
const POLL: Duration = Duration::from_millis(300);

pub fn watch(
    sessions: SessionsEngine,
    chat_id: String,
    task_id: String,
) -> anyhow::Result<BoxStream<'static, serde_json::Value>> {
    let path = sessions
        .session_status(&chat_id)
        .and_then(|s| s.background_tasks.into_iter().find(|t| t.id == task_id))
        .and_then(|t| t.output_path)
        .ok_or_else(|| anyhow::anyhow!("no output file for background task {task_id}"))?;
    let path = PathBuf::from(path);
    let named = path.file_name().and_then(|n| n.to_str()) == Some(&format!("{task_id}.output"));
    if !named || !path.is_absolute() {
        anyhow::bail!("refusing output path {}", path.display());
    }
    let state = Watch {
        sessions,
        chat_id,
        task_id,
        path,
        last: None,
        first: true,
        done: false,
    };
    Ok(futures::stream::unfold(state, |mut st| async move {
        loop {
            if st.done {
                return None;
            }
            if !st.first {
                tokio::time::sleep(POLL).await;
            }
            st.first = false;
            let running = st
                .sessions
                .session_status(&st.chat_id)
                .is_some_and(|s| s.background_tasks.iter().any(|t| t.id == st.task_id));
            let path = st.path.clone();
            let lines = tokio::task::spawn_blocking(move || read_tail(&path))
                .await
                .unwrap_or_default();
            st.done = !running;
            if !running || st.last.as_ref() != Some(&lines) {
                st.last = Some(lines.clone());
                let value = serde_json::json!({ "lines": lines, "running": running });
                return Some((value, st));
            }
        }
    })
    .boxed())
}

struct Watch {
    sessions: SessionsEngine,
    chat_id: String,
    task_id: String,
    path: PathBuf,
    last: Option<Vec<String>>,
    first: bool,
    done: bool,
}

fn read_tail(path: &Path) -> Vec<String> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    tail_lines(&String::from_utf8_lossy(&bytes), start > 0)
}

fn tail_lines(text: &str, cut: bool) -> Vec<String> {
    let text = strip_ansi(text);
    let mut lines: Vec<String> = text
        .split('\n')
        .skip(usize::from(cut))
        .map(|line| {
            let line = line.trim_end_matches('\r');
            line.rsplit('\r').next().unwrap_or(line).to_owned()
        })
        .collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let drop = lines.len().saturating_sub(TAIL_LINES);
    lines.drain(..drop);
    lines
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            if c == '\t' || c == '\n' || c == '\r' || !c.is_control() {
                out.push(c);
            }
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_titles_are_stripped() {
        assert_eq!(
            strip_ansi("\x1b[1;32mok\x1b[0m \x1b]0;title\x07done\x1b]8;;u\x1b\\x"),
            "ok donex"
        );
    }

    #[test]
    fn a_progress_bar_keeps_its_last_redraw() {
        assert_eq!(
            tail_lines("start\n 10%\r 50%\r100%\ndone\n\n", false),
            ["start", "100%", "done"]
        );
    }

    #[test]
    fn a_cut_tail_drops_its_partial_first_line_and_keeps_the_last_lines() {
        let text = (0..300).map(|i| format!("line {i}\n")).collect::<String>();
        let lines = tail_lines(&format!("tial{text}"), true);
        assert_eq!(lines.len(), TAIL_LINES);
        assert_eq!(lines.first().map(String::as_str), Some("line 100"));
        assert_eq!(lines.last().map(String::as_str), Some("line 299"));
    }
}
