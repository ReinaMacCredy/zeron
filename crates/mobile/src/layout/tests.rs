use std::sync::Arc;
use std::time::Instant;

use super::*;

struct Quiet;
impl LayoutListener for Quiet {
    fn frame_ready(&self, _revision: u64) {}
}

/// Fixed-advance fallback so tests never depend on a platform engine.
struct FixedFallback;
impl PlatformMeasurer for FixedFallback {
    fn measure(&self, _face: FaceRole, size: f32, _ligatures: bool, text: String) -> f32 {
        text.chars().count() as f32 * size * 1.1
    }
    fn measure_run(&self, _face: FaceRole, size: f32, _ligatures: bool, text: String) -> Vec<f32> {
        text.chars().map(|_| size * 1.1).collect()
    }
}

fn font(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/../ui/assets/fonts/{name}.ttf", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

pub(crate) fn text_system() -> Arc<TextSystem> {
    let faces = [
        (FaceRole::Sans, "Geist"),
        (FaceRole::SansMedium, "Geist-Medium"),
        (FaceRole::SansSemibold, "Geist-SemiBold"),
        (FaceRole::SansBold, "Geist-Bold"),
        (FaceRole::SansItalic, "Geist-Italic"),
        (FaceRole::SansSemiboldItalic, "Geist-SemiBoldItalic"),
        (FaceRole::Mono, "GeistMono"),
    ]
    .into_iter()
    .map(|(role, name)| FaceData { role, bytes: font(name) })
    .collect();
    TextSystem::new(faces, Some(Arc::new(FixedFallback)))
}

fn worker(width: f32) -> Worker {
    let ts = text_system();
    let mut w = Worker::new(&ts, Arc::new(Shared { frame: Mutex::new(Arc::new(LayoutFrame::empty())) }), Arc::new(Quiet), None);
    w.width = width;
    w
}

pub(crate) const RICH: &str = FIXTURE;

fn transcript(turns: usize) -> TranscriptInput {
    let mut entries = Vec::new();
    for i in 0..turns {
        entries.push(DebugEntry {
            id: format!("u{i}"),
            user: true,
            text: format!("Question {i}: can you explain how the layout engine handles **wrapping** of long lines like /Users/wing/project/src/some/deeply/nested/module.rs?"),
            streaming: false,
        });
        entries.push(DebugEntry {
            id: format!("a{i}"),
            user: false,
            text: RICH.to_owned(),
            streaming: false,
        });
    }
    debug_input(entries, false)
}

#[test]
fn paint_matches_measure_at_many_widths() {
    for width in [280.0, 320.0, 375.0, 393.0, 430.0, 744.0, 1024.0] {
        let mut w = worker(width);
        w.input = transcript(2);
        let frame = w.pass();
        assert!(frame.row_count() > 10, "rows: {}", frame.row_count());
        let mut y = 0.0;
        for i in 0..frame.row_count() {
            let p = frame.placement(i).unwrap();
            assert!((p.y - y).abs() < 0.01, "offsets are a prefix sum");
            y += p.height;
            let d = frame.display(i).unwrap();
            assert!((d.height - p.height).abs() < 0.01);
            let units = d.text.encode_utf16().count() as u32;
            for run in &d.runs {
                assert!(run.start + run.len <= units, "run slices the row text");
                if run.scroller.is_none() {
                    assert!(run.x >= 0.0 && run.x + run.width <= width + 0.5, "run inside width {width}: {run:?}");
                }
                assert!(run.baseline > 0.0 && run.baseline <= d.height + 0.5 || run.scroller.is_some());
            }
        }
    }
}

#[test]
fn streaming_converges_to_full_parse() {
    let mut w = worker(390.0);
    let mut last = None;
    let chars: Vec<char> = RICH.chars().collect();
    let mut shown = String::new();
    for chunk in chars.chunks(7) {
        shown.extend(chunk);
        w.input = debug_input(
            vec![DebugEntry { id: "a".into(), user: false, text: shown.clone(), streaming: true }],
            true,
        );
        last = Some(w.pass());
    }
    let _ = last;
    w.input = debug_input(vec![DebugEntry { id: "a".into(), user: false, text: shown, streaming: false }], false);
    let streamed = w.pass();
    let mut fresh = worker(390.0);
    fresh.input = transcript_one(RICH);
    let full = fresh.pass();
    assert_eq!(streamed.row_count(), full.row_count());
    for i in 0..full.row_count() {
        let (a, b) = (streamed.placement(i).unwrap(), full.placement(i).unwrap());
        assert_eq!(a.key, b.key);
        assert!((a.height - b.height).abs() < 0.01, "row {i}: {} vs {}", a.height, b.height);
    }
}

#[test]
fn agent_messages_show_their_sender_over_the_bubble() {
    let mut w = worker(390.0);
    let id = "cd27c5c2-0000-4000-8000-000000000000";
    let text = zeron_proto::agent_messages::agent_message("Windows lane (cd27c5c2)", id, "Windows lane: done.");
    w.input = debug_input(vec![DebugEntry { id: "u".into(), user: true, text, streaming: false }], false);
    let frame = w.pass();
    let d = frame.display(0).unwrap();
    assert!(d.text.starts_with("Message from \u{2060} Windows lane"), "{}", d.text);
    assert!(d.text.contains("Windows lane: done."), "{}", d.text);
    assert!(!d.text.contains("send_message") && !d.text.contains("cd27c5c2"), "{}", d.text);
    assert_eq!(frame.message_text(0).as_deref(), Some("Windows lane: done."));
    // The sender's mark (a chat glyph: the workspace doesn't list it), and
    // no link to a chat that doesn't open.
    assert!(d.widgets.iter().any(|w| matches!(&w.kind, display::WidgetKind::Icon { name, .. } if name == "bubble.left")));
    assert!(d.links.is_empty());
    // The caption sits over the bubble: its runs end above the bubble's fill.
    let bubble = d.boxes.iter().find(|b| b.color == display::ColorRole::UserBubble).unwrap();
    let caption = d.runs.first().unwrap();
    assert!(caption.baseline < bubble.y, "{caption:?} {bubble:?}");
    // No wider than the bubble: nothing of it starts left of the bubble.
    assert!(d.runs.iter().filter(|r| r.baseline < bubble.y).all(|r| r.x >= bubble.x - 0.5), "{:?} {bubble:?}", d.runs);
}

#[test]
fn messages_the_agent_sent_show_as_one_line_naming_the_recipient() {
    let tool = |id: &str, name: &str, input: serde_json::Value| MessagePart::Tool {
        id: id.into(),
        call: zeron_proto::ToolCall::Mcp { server: "zeron".into(), tool: name.into(), input: Some(input) },
        is_error: false,
        resolved: true,
        output: None,
        diff: None,
        output_ref: None,
        output_bytes: None,
        diff_ref: None,
        diff_stats: None,
        subagent_ref: None,
        subagent_status: None,
        subagent_tail: None,
        delegate_route: None,
        html_render: None,
    };
    let mut w = worker(390.0);
    w.input = TranscriptInput {
        entries: vec![Arc::new(SessionMessageEntry {
            id: "a".into(),
            role: MessageRole::Assistant,
            parts: vec![
                tool("k1", "list_chats", serde_json::json!({})),
                tool("k2", "send_message", serde_json::json!({ "chat": "Windows lane", "text": "Run the matrix." })),
                tool("k3", "send_messages", serde_json::json!({ "requests": [{ "chat": "a", "text": "one" }, { "chat": "b", "text": "two" }] })),
            ],
            created_at: 0,
            device_id: String::new(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            duration_ms: None,
            answers_request_id: None,
            fork_import: false,
            from_chat_id: None,
        })],
        ..Default::default()
    };
    // A settled turn folds under its work header (fork); open it.
    w.builder.expanded.insert(rows::row_key("a#work"));
    let frame = w.pass();
    // The ordinary tool keeps its group; each message is its own row.
    assert_eq!(frame.row_count(), 5);
    assert_eq!(frame.placement(2).unwrap().kind, RowKind::Markdown);
    let d = frame.display(2).unwrap();
    // Just the recipient: no bubble, no message text.
    assert_eq!(d.text, "Message to \u{2060} Windows lane");
    assert!(d.boxes.iter().all(|b| b.color != display::ColorRole::UserBubble));
    // Left-aligned at the column's edge.
    assert!(d.runs.first().unwrap().x < 40.0);
    assert!(frame.display(4).unwrap().text.ends_with(" b"));
}

fn transcript_one(text: &str) -> TranscriptInput {
    debug_input(vec![DebugEntry { id: "a".into(), user: false, text: text.into(), streaming: false }], false)
}

/// The question chip says what was answered, one `label → answer` line per
/// question; a chip from before answers were recorded keeps its old text.
#[test]
fn question_chip_reads_what_was_answered() {
    let question = |id: &str, header: &str| zeron_proto::UserInputQuestion {
        id: id.into(),
        header: header.into(),
        question: format!("{header}?"),
        options: vec!["Deep dive".into(), "Table".into()],
        multi_select: false,
        prefill: None,
        multiline: false,
        approval: false,
        kind: None,
    };
    let chip = |questions: Vec<zeron_proto::UserInputQuestion>, resolved: bool, answers: Option<Vec<zeron_proto::UserInputAnswer>>| {
        let mut w = worker(390.0);
        w.input = TranscriptInput {
            entries: vec![Arc::new(SessionMessageEntry {
                id: "a".into(),
                role: MessageRole::Assistant,
                parts: vec![MessagePart::Input { id: "r1".into(), request_id: "r1".into(), questions, resolved, answers, answered_at_ms: None, asked_at_ms: None }],
                created_at: 0,
                device_id: String::new(),
                status: Some(MessageStatus::Complete),
                continuation_of: None,
                duration_ms: None,
                answers_request_id: None,
                fork_import: false,
                from_chat_id: None,
            })],
            ..Default::default()
        };
        let frame = w.pass();
        let row = frame.display(0).unwrap();
        (row.copy_text, row.height)
    };
    let picked = |id: &str, label: &str| zeron_proto::UserInputAnswer { question_id: id.into(), labels: vec![label.into()] };
    assert_eq!(chip(vec![question("q", "Recherche")], true, None).0, "Answered a question");
    let (closed, one_line) = chip(vec![question("q", "Recherche")], true, Some(vec![]));
    assert_eq!(closed, "Recherche → Not answered");
    let (answered, two_lines) = chip(vec![question("a", "Recherche"), question("b", "Format")], true, Some(vec![picked("a", "Deep dive"), picked("b", "Charts")]));
    assert_eq!(answered, "Recherche → Deep dive\nFormat → “Charts”");
    // Short enough to share a line at this width: only a kept break makes
    // the second answer its own line.
    assert!(two_lines > one_line, "{two_lines} vs {one_line}");
}

#[test]
fn stable_prefix_rows_are_reused_while_streaming() {
    let mut w = worker(390.0);
    let base = "# Title\n\nFirst paragraph.\n\nSecond paragraph that keeps growing";
    w.input = debug_input(vec![DebugEntry { id: "a".into(), user: false, text: base.into(), streaming: true }], true);
    let f1 = w.pass();
    w.input = debug_input(vec![DebugEntry { id: "a".into(), user: false, text: format!("{base} with more words"), streaming: true }], true);
    let f2 = w.pass();
    // Heading + first paragraph keep their versions; the tail changes.
    for i in 0..2 {
        assert_eq!(f1.placement(i).unwrap().version, f2.placement(i).unwrap().version);
    }
    assert_ne!(f1.placement(2).unwrap().version, f2.placement(2).unwrap().version);
}

#[test]
fn toggles_expand_tool_groups_and_long_user_messages() {
    let mut w = worker(390.0);
    let long = (0..40).map(|i| format!("line {i} of a long pasted prompt")).collect::<Vec<_>>().join("\n");
    w.input = debug_input(vec![DebugEntry { id: "u".into(), user: true, text: long, streaming: false }], false);
    let folded = w.pass();
    let key = folded.placement(0).unwrap().key;
    w.builder.expanded.insert(key);
    w.builder.invalidate(key);
    let open = w.pass();
    assert!(open.placement(0).unwrap().height > folded.placement(0).unwrap().height * 3.0);
}

/// Thinking renders desktop-style: markdown flattened to styled detail lines
/// (bold/lists/code/quotes, underlined non-clickable links), not literal
/// markers (desktop PR #220; the mobile port dropped it).
#[test]
fn thinking_renders_styled_markdown_not_markers() {
    use zeron_doc::parts::{MessagePart, MessageStatus};
    use zeron_doc::schema::{MessageRole, SessionMessageEntry};
    let reasoning = concat!(
        "**Planning** the `fix`\n\n",
        "- point *one*\n",
        "- point two with a [link](https://example.com)\n\n",
        "```rust\n",
        "let x = 1;\n",
        "```\n\n",
        "> quoted text",
    );
    let mut w = worker(390.0);
    w.input = TranscriptInput {
        entries: vec![Arc::new(SessionMessageEntry {
            id: "a".into(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::Reasoning { id: "r0".into(), text: reasoning.into() }],
            created_at: 0,
            device_id: String::new(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            duration_ms: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
        })],
        ..Default::default()
    };
    w.builder.expanded.insert(rows::row_key("a#work"));
    let folded = w.pass();
    let key = folded.placement(1).unwrap().key;
    w.builder.expanded.insert(key);
    w.builder.detail_open.insert(rows::row_key("a#g0/r0"), true);
    w.builder.invalidate(key);
    let frame = w.pass();
    let d = frame.display(1).unwrap();
    assert!(!d.text.contains("**"), "{}", d.text);
    assert!(!d.text.contains("`fix`"), "{}", d.text);
    assert!(!d.text.contains("```"), "{}", d.text);
    assert!(d.text.contains("Planning"), "{}", d.text);
    assert!(d.text.contains("• point one"), "{}", d.text);
    assert!(d.text.contains("let x = 1;"), "{}", d.text);
    assert!(d.text.contains("│ quoted text"), "{}", d.text);
    assert!(d.runs.iter().any(|r| r.decoration == display::Decoration::Underline), "{:?}", d.runs);
    assert!(d.links.is_empty(), "thought links must not be clickable");
    assert!(d.runs.iter().any(|r| r.color == display::ColorRole::TextFaint));
    // Bold is a distinct face: "Planning" isn't painted in the regular style.
    let slice = |r: &display::TextRun| -> String {
        d.text.encode_utf16().skip(r.start as usize).take(r.len as usize).filter_map(|u| char::from_u32(u as u32)).collect()
    };
    let planning = d.runs.iter().find(|r| slice(r) == "Planning").expect("a Planning run");
    let regular = d.runs.iter().find(|r| slice(r) == " the ").expect("a regular run");
    assert_ne!(planning.style, regular.style, "bold uses the semibold face");
    // Paint/measure hold across widths (display() debug-asserts equality).
    for width in [280.0, 320.0, 430.0, 744.0, 1024.0] {
        let mut w = worker(width);
        w.input = TranscriptInput {
            entries: vec![Arc::new(SessionMessageEntry {
                id: "a".into(),
                role: MessageRole::Assistant,
                parts: vec![MessagePart::Reasoning { id: "r0".into(), text: reasoning.into() }],
                created_at: 0,
                device_id: String::new(),
                status: Some(MessageStatus::Complete),
                continuation_of: None,
                duration_ms: None,
                from_chat_id: None,
                fork_import: false,
                answers_request_id: None,
            })],
            ..Default::default()
        };
        w.builder.expanded.insert(rows::row_key("a#work"));
        let key = w.pass().placement(1).unwrap().key;
        w.builder.expanded.insert(key);
        w.builder.detail_open.insert(rows::row_key("a#g0/r0"), true);
        w.builder.invalidate(key);
        let frame = w.pass();
        let d = frame.display(1).unwrap();
        assert!(d.text.contains("• point one"), "width {width}: {}", d.text);
        for run in &d.runs {
            assert!(run.x >= -0.5 && run.x + run.width <= width + 0.5, "width {width}: {run:?}");
        }
    }
}

/// A thought streamed through the incremental parser (live display mend)
/// settles to exactly the frame a fresh full parse lays out.
#[test]
fn streaming_thought_markdown_settles_to_the_fresh_parse() {
    use zeron_doc::parts::{MessagePart, MessageStatus};
    use zeron_doc::schema::{MessageRole, SessionMessageEntry};
    let text = "**Checking** the `parser`\n\n1. first step\n2. second with [docs](https://example.com)\n\n> note\n\n```rust\nfn main() {}\n```";
    let entry = |status: MessageStatus, text: &str| {
        Arc::new(SessionMessageEntry {
            id: "a".into(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::Reasoning { id: "r0".into(), text: text.to_owned() }],
            created_at: 0,
            device_id: String::new(),
            status: Some(status),
            continuation_of: None,
            duration_ms: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
        })
    };
    let mut live = worker(390.0);
    let mut shown = String::new();
    let mut key = 0;
    for chunk in text.chars().collect::<Vec<_>>().chunks(5) {
        shown.extend(chunk);
        live.input = TranscriptInput { entries: vec![entry(MessageStatus::Streaming, &shown)], ..Default::default() };
        let frame = live.pass();
        key = frame.placement(0).unwrap().key;
        assert!(!frame.display(0).unwrap().text.contains("**"), "mend holds while streaming");
    }
    live.builder.expanded.insert(key);
    live.builder.expanded.insert(rows::row_key("a#work"));
    live.builder.detail_open.insert(rows::row_key("a#g0/r0"), true);
    live.builder.invalidate(key);
    live.input = TranscriptInput { entries: vec![entry(MessageStatus::Complete, text)], ..Default::default() };
    let streamed = live.pass();

    let mut fresh = worker(390.0);
    fresh.input = TranscriptInput { entries: vec![entry(MessageStatus::Complete, text)], ..Default::default() };
    fresh.builder.expanded.insert(rows::row_key("a#work"));
    let folded = fresh.pass();
    let fresh_key = folded.placement(1).unwrap().key;
    assert_eq!(key, fresh_key);
    fresh.builder.expanded.insert(fresh_key);
    fresh.builder.detail_open.insert(rows::row_key("a#g0/r0"), true);
    fresh.builder.invalidate(fresh_key);
    let settled = fresh.pass();

    assert_eq!(streamed.row_count(), settled.row_count());
    for i in 0..settled.row_count() {
        let a = streamed.display(i).unwrap();
        let b = settled.display(i).unwrap();
        assert_eq!(a.text, b.text, "row {i} text");
        assert!((a.height - b.height).abs() < 0.01, "row {i} height");
    }
}

fn thought_input(text: &str, streaming: bool) -> TranscriptInput {
    use zeron_doc::parts::{MessagePart, MessageStatus};
    use zeron_doc::schema::{MessageRole, SessionMessageEntry};
    TranscriptInput {
        entries: vec![Arc::new(SessionMessageEntry {
            id: "a".into(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::Reasoning { id: "r0".into(), text: text.to_owned() }],
            created_at: 0,
            device_id: String::new(),
            status: Some(if streaming {
                MessageStatus::Streaming
            } else {
                MessageStatus::Complete
            }),
            continuation_of: None,
            duration_ms: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
        })],
        ..Default::default()
    }
}

/// A settled one-thought reply with its group and thought detail opened.
fn open_thought(width: f32, text: &str) -> RowDisplay {
    let mut w = worker(width);
    w.input = thought_input(text, false);
    w.builder.expanded.insert(rows::row_key("a#work"));
    let key = w.pass().placement(1).unwrap().key;
    w.builder.expanded.insert(key);
    w.builder.detail_open.insert(rows::row_key("a#g0/r0"), true);
    w.builder.invalidate(key);
    w.pass().display(1).unwrap()
}

fn run_text(d: &RowDisplay, r: &display::TextRun) -> String {
    String::from_utf16_lossy(&d.text.encode_utf16().skip(r.start as usize).take(r.len as usize).collect::<Vec<_>>())
}

/// Code keeps its indentation; nested items step right; a wrapped list item
/// hangs under its first word and a wrapped quote keeps its bar on every
/// line (desktop's re-indented wrap), at every width.
#[test]
fn thought_code_indents_and_wrapped_lines_hang() {
    let alpha = "alpha ".repeat(20);
    let omega = "omega ".repeat(20);
    let text = format!("```\nfn f() {{\n    let x = 1;\n}}\n```\n\n- top\n  - nested\n- {alpha}\n\n> {omega}");
    for width in [280.0, 320.0, 390.0] {
        let d = open_thought(width, &text);
        assert!(d.text.contains("    let x = 1;"), "code indentation survives: {}", d.text);
        let x_of = |needle: &str| d.runs.iter().find(|r| run_text(&d, r) == needle).unwrap_or_else(|| panic!("{needle}: {:?}", d.runs)).x;
        let top = x_of("top");
        assert!(x_of("nested") > top + 1.0, "nested item indents");
        let lines = |word: &str| {
            let runs: Vec<_> = d.runs.iter().filter(|r| run_text(&d, r).contains(word)).collect();
            let mut baselines: Vec<f32> = runs.iter().map(|r| r.baseline).collect();
            baselines.dedup_by(|a, b| (*a - *b).abs() < 0.5);
            (runs, baselines.len())
        };
        let (alpha_runs, alpha_lines) = lines("alpha");
        assert!(alpha_lines > 1, "width {width}: the long item wraps");
        for r in &alpha_runs {
            assert!((r.x - top).abs() < 0.5, "width {width}: wrapped item hangs under its text: {r:?}");
        }
        let (omega_runs, omega_lines) = lines("omega");
        assert!(omega_lines > 1, "width {width}: the long quote wraps");
        let bar_x = omega_runs[0].x;
        for r in &omega_runs {
            assert!((r.x - bar_x).abs() < 0.5, "width {width}: wrapped quote hangs: {r:?}");
        }
        let bars = d.runs.iter().filter(|r| run_text(&d, r).contains('│')).count();
        assert_eq!(bars, omega_lines, "width {width}: one bar per quoted line");
        for run in &d.runs {
            assert!(run.x >= -0.5 && run.x + run.width <= width + 0.5, "width {width}: {run:?}");
        }
    }
}

/// A long thought streaming past the visible cap stops growing, fades its
/// last line, and stops re-preparing: deltas below the fold reuse the body.
#[test]
fn long_streaming_thought_is_capped_and_reuses_its_body() {
    let mut w = worker(390.0);
    let mut text = String::new();
    let mut heights = Vec::new();
    let mut bodies = Vec::new();
    for i in 0..80 {
        text.push_str(&format!("Step {i}: **check** the `thing` and keep going.\n\n"));
        // Streaming: the group auto-expands and the tail thought auto-opens.
        w.input = thought_input(&text, true);
        let frame = w.pass();
        let d = frame.display(0).unwrap();
        assert!(!d.text.contains("**"));
        heights.push(d.height);
        bodies.push(w.builder.thought_body_for_test("a#r0").expect("thought prepared"));
        if i == 79 {
            assert!(d.fades.iter().any(|f| f.edge == display::FadeEdge::Bottom), "cut thought fades");
        }
    }
    let settled = heights[heights.len() - 1];
    assert!(heights[..5].windows(2).all(|p| p[1] > p[0]), "grows while short: {heights:?}");
    assert!(heights[40..].iter().all(|h| (h - settled).abs() < 0.01), "capped: {heights:?}");
    assert!(bodies[40..].windows(2).all(|p| Arc::ptr_eq(&p[0], &p[1])), "past the cap, deltas reuse the prepared body");
}

/// One huge paragraph (no line breaks for the line cap to act on) streaming
/// in: the visible text is clipped, so the body stops re-preparing too.
#[test]
fn huge_single_paragraph_thought_stops_reshaping() {
    let mut w = worker(390.0);
    let mut text = String::new();
    let mut heights = Vec::new();
    let mut bodies = Vec::new();
    for _ in 0..120 {
        text.push_str(&"streaming thought words ".repeat(10));
        w.input = thought_input(&text, true);
        let frame = w.pass();
        heights.push(frame.display(0).unwrap().height);
        bodies.push(w.builder.thought_body_for_test("a#r0").expect("thought prepared"));
    }
    assert!(text.len() > 25_000);
    let settled = heights[heights.len() - 1];
    assert!(heights[80..].iter().all(|h| (h - settled).abs() < 0.01), "capped: {heights:?}");
    assert!(bodies[80..].windows(2).all(|p| Arc::ptr_eq(&p[0], &p[1])), "past the budget, deltas reuse the prepared body");
}

/// Release-mode timings (run with `cargo test --release -p zeron-mobile -- --ignored --nocapture`).
#[test]
#[ignore]
fn bench_layout_passes() {
    let mut w = worker(393.0);
    w.input = transcript(300);
    let t = Instant::now();
    let frame = w.pass();
    let cold = t.elapsed();
    let t = Instant::now();
    w.width = 430.0;
    w.pass();
    let resize = t.elapsed();
    let t = Instant::now();
    for i in 0..frame.row_count().min(2000) {
        frame.display(i);
    }
    let display = t.elapsed() / frame.row_count().min(2000);
    // Streaming: append ~6 chars per update to a live tail after 600 entries.
    let mut entries = transcript(300).entries;
    let mut text = String::new();
    let mut total = std::time::Duration::ZERO;
    let updates = 400;
    for i in 0..updates {
        text.push_str(["word ", "more ", "text\n\n", "`code` ", "**bold** "][i % 5]);
        let mut e = entries.clone();
        e.push(Arc::new(SessionMessageEntry {
            id: "live".into(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::Text {
                id: "t0".into(),
                text: text.clone(),
            }],
            created_at: 0,
            device_id: String::new(),
            status: Some(MessageStatus::Streaming),
            continuation_of: None,
            duration_ms: None, from_chat_id: None, fork_import: false, answers_request_id: None, 
        }));
        w.input = TranscriptInput { entries: e, pending: vec![], working: true, working_since_ms: None, streaming: true, ..Default::default() };
        let t = Instant::now();
        w.pass();
        total += t.elapsed();
    }
    entries.clear();
    println!(
        "rows={} heap={:.1}MB cold={:?} resize={:?} display/row={:?} stream/update={:?}",
        frame.row_count(),
        frame.prepared_heap_bytes() as f64 / 1_048_576.0,
        cold,
        resize,
        display,
        total / updates as u32
    );
}

#[test]
fn user_mentions_render_as_chips() {
    // The desktop's chips: a soft pill with the file theme's icon on a well,
    // the label in the body color — for files, folders, skills and commands.
    let mut w = worker(390.0);
    let skill = zeron_proto::invocation::Invocation::Skill { name: "review".into(), path: "/repo/SKILL.md".into(), command: None };
    let text = format!(
        "Look at [mod.rs](zeron-file:crates/mobile/src/layout/mod.rs) in [layout](zeron-file:crates/mobile/src/layout/) with {}",
        skill.link()
    );
    w.input = debug_input(vec![DebugEntry { id: "u".into(), user: true, text, streaming: false }], false);
    let frame = w.pass();
    let d = frame.display(0).unwrap();
    assert!(d.text.contains("mod.rs") && !d.text.contains("@mod.rs"), "{}", d.text);
    assert!(!d.text.contains("zeron-"), "{}", d.text);
    assert!(d.runs.iter().all(|r| r.color != display::ColorRole::Link));
    let icons: Vec<&str> = d
        .widgets
        .iter()
        .filter_map(|w| match &w.kind {
            display::WidgetKind::Icon { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(icons, ["fileicon-files-rust", "fileicon-folders-folder", "wand.and.stars"]);
    assert_eq!(d.boxes.iter().filter(|b| b.color == display::ColorRole::ToolBadge).count(), 3);
}

#[test]
fn user_attachments_render_as_images_not_trailer_text() {
    let mut w = worker(390.0);
    let text = "Fix the header spacing\n\nAttached images (local files — open them to view):\n- /tmp/uploads/a/shot.png".to_owned();
    w.input = debug_input(vec![DebugEntry { id: "u".into(), user: true, text, streaming: false }], false);
    let frame = w.pass();
    let d = frame.display(0).unwrap();
    assert!(!d.text.contains("Attached images"), "{}", d.text);
    assert!(d.text.contains("Fix the header spacing"));
    assert!(d.widgets.iter().any(|w| matches!(&w.kind, display::WidgetKind::Image { reference } if reference.ends_with("shot.png"))));
}

#[test]
fn bundled_doc_images_lay_out_as_sized_blocks_and_others_stay_links() {
    let mut w = worker(390.0);
    let text = "Pick a model.\n\n![Compact picker](models-compact-picker.jpg)\n\n![Elsewhere](not-bundled.png)".to_owned();
    w.input = debug_input(vec![DebugEntry { id: "a".into(), user: false, text, streaming: false }], false);
    let frame = w.pass();
    let rows: Vec<_> = (0..frame.row_count()).filter_map(|i| frame.display(i)).collect();
    let images: Vec<_> = rows
        .iter()
        .flat_map(|d| &d.widgets)
        .filter(|w| matches!(&w.kind, display::WidgetKind::Image { .. }))
        .collect();
    assert_eq!(images.len(), 1);
    assert!(matches!(&images[0].kind, display::WidgetKind::Image { reference } if reference == "docs:models-compact-picker.jpg"));
    let (iw, ih) = (images[0].w, images[0].h);
    assert!(iw <= 380.0 && (ih - iw * 256.0 / 760.0).abs() < 0.5, "{iw}x{ih}");
    assert!(rows.iter().any(|d| d.text.contains("Elsewhere")));
}

#[test]
fn quoted_selections_render_as_a_pill_not_the_block() {
    let mut w = worker(390.0);
    let text = zeron_proto::quotes::with_quotes("why this?", &[zeron_proto::quotes::Quote::new("the retry budget")]);
    w.input = debug_input(vec![DebugEntry { id: "u".into(), user: true, text, streaming: false }], false);
    let frame = w.pass();
    let d = frame.display(0).unwrap();
    assert!(d.text.contains("why this?") && d.text.contains("1 selection"), "{}", d.text);
    assert!(!d.text.contains("Selected text") && !d.text.contains("retry budget"), "{}", d.text);
    assert!(d.widgets.iter().any(|w| matches!(&w.kind, display::WidgetKind::Icon { name, .. } if name == "text.quote")));
}

#[test]
fn synced_file_attachments_render_as_chips_and_name_pills() {
    // A desktop send mixing an image and ZIPs (one still a queued-upload ref),
    // with composer chips in the prompt. Chipped attachments show as their
    // chip only; the image chip opens its upload; the unchipped ZIP keeps a
    // name pill and never reaches image loading.
    let mut w = worker(390.0);
    let text = "Compare [Image 1](zeron-image:1) with [notes.zip](zeron-attachment:2)\n\nAttached images (local files — open them to view):\n- /tmp/uploads/ab12cd34-Image_1.png\n- /tmp/uploads/ab12cd34-notes.zip\n- pending://up-3/logs.zip".to_owned();
    w.input = debug_input(vec![DebugEntry { id: "u".into(), user: true, text, streaming: false }], false);
    let frame = w.pass();
    let d = frame.display(0).unwrap();
    assert!(!d.widgets.iter().any(|w| matches!(&w.kind, display::WidgetKind::Image { .. })));
    let icons: Vec<&str> = d
        .widgets
        .iter()
        .filter_map(|w| match &w.kind {
            display::WidgetKind::Icon { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    // The unchipped ZIP's pill, then the bubble's two chips.
    assert_eq!(icons, ["fileicon-files-compressed", "photo", "fileicon-files-compressed"]);
    assert!(d.text.contains("Compare Image 1 with notes.zip"), "{}", d.text);
    assert!(d.text.contains("logs.zip") && !d.text.contains("ab12cd34-"), "{}", d.text);
    assert!(!d.text.contains("zeron-image") && !d.text.contains("zeron-attachment"), "{}", d.text);
    let links: Vec<&str> = d.links.iter().map(|l| l.url.as_str()).collect();
    assert_eq!(links, ["zeron-preview://image?ref=%2Ftmp%2Fuploads%2Fab12cd34-Image_1.png"]);
}

#[test]
fn pasted_text_reads_as_a_tappable_pill_chipped_or_not() {
    let mut w = worker(390.0);
    let text = "Read [Pasted text 1.txt](zeron-attachment:1)\n\nAttached images (local files — open them to view):\n- /tmp/uploads/ab12cd34-Pasted_text_1.txt\n- pending://up-2/Pasted_text_2.txt".to_owned();
    w.input = debug_input(vec![DebugEntry { id: "u".into(), user: true, text, streaming: false }], false);
    let frame = w.pass();
    let d = frame.display(0).unwrap();
    assert!(d.text.contains("Read Pasted text 1") && d.text.contains("Pasted text 2"), "{}", d.text);
    assert!(!d.text.contains(".txt"), "{}", d.text);
    let mut links: Vec<&str> = d.links.iter().map(|l| l.url.as_str()).collect();
    links.sort();
    assert_eq!(
        links,
        [
            "zeron-preview://text?ref=%2Ftmp%2Fuploads%2Fab12cd34-Pasted_text_1.txt",
            "zeron-preview://text?ref=pending%3A%2F%2Fup-2%2FPasted_text_2.txt",
        ]
    );
}

#[test]
fn folded_user_message_fades_its_last_line() {
    let mut w = worker(390.0);
    let long = (0..40).map(|i| format!("line {i} of a long pasted prompt")).collect::<Vec<_>>().join("\n");
    w.input = debug_input(vec![DebugEntry { id: "u".into(), user: true, text: long, streaming: false }], false);
    let frame = w.pass();
    let d = frame.display(0).unwrap();
    let fade = d.fades.iter().find(|f| f.edge == display::FadeEdge::Bottom).expect("bottom fade");
    // Exactly one shown line sits in the fade band (the "Show more" label is below it).
    let in_band: Vec<f32> = d.runs.iter().map(|r| r.baseline).filter(|b| *b > fade.y && *b <= fade.y + fade.h).collect();
    assert!(!in_band.is_empty(), "fade covers the last shown line");
    assert!(in_band.iter().all(|b| (b - in_band[0]).abs() < 0.5));
}

#[test]
fn running_subagent_shows_a_spinner_after_its_spawn_resolves() {
    use zeron_doc::parts::{MessagePart, MessageStatus, SubagentStatus};
    use zeron_doc::schema::{MessageRole, SessionMessageEntry};
    let spawn_with = |status: SubagentStatus, input: Option<serde_json::Value>| MessagePart::Tool {
        id: "k1".into(),
        call: zeron_proto::ToolCall::Unknown { name: "Agent: scan the repo".into(), input },
        is_error: false,
        // Eager-done: the spawn call resolved while the subagent still runs.
        resolved: true,
        output: None,
        diff: None,
        output_ref: None,
        output_bytes: None,
        diff_ref: None,
        diff_stats: None,
        subagent_ref: Some("sub-1".into()),
        subagent_status: Some(status),
        subagent_tail: None,
        html_render: None,
        delegate_route: None,
    };
    let spawn = |status: SubagentStatus| spawn_with(status, None);
    let frame_with = |part: MessagePart, harness: Option<zeron_proto::HarnessId>| {
        let mut w = worker(390.0);
        w.input = TranscriptInput {
            entries: vec![Arc::new(SessionMessageEntry {
                id: "a".into(),
                role: MessageRole::Assistant,
                parts: vec![part],
                created_at: 0,
                device_id: String::new(),
                status: Some(MessageStatus::Complete),
                continuation_of: None,
                duration_ms: None,
                from_chat_id: None,
                fork_import: false,
                answers_request_id: None,
            })],
            harness,
            ..Default::default()
        };
        w.builder.expanded.insert(rows::row_key("a#work"));
        w.pass()
    };
    let frame_for = |status: SubagentStatus| frame_with(spawn(status), None);
    let spinners = |f: &LayoutFrame| f.display(1).unwrap().widgets.iter().filter(|w| matches!(w.kind, display::WidgetKind::Spinner)).count();
    assert_eq!(spinners(&frame_for(SubagentStatus::Running)), 1, "running subagent spins");
    assert_eq!(spinners(&frame_for(SubagentStatus::Done)), 0, "finished subagent is quiet");
    let failed = frame_for(SubagentStatus::Failed);
    assert!(failed.display(1).unwrap().runs.iter().any(|r| r.color == display::ColorRole::Danger), "failed subagent is tinted danger");

    let icon = |f: &LayoutFrame| {
        f.display(1).unwrap().widgets.iter().find_map(|w| match &w.kind {
            display::WidgetKind::Icon { name, .. } => Some(name.clone()),
            _ => None,
        })
    };
    let codex = Some(zeron_proto::HarnessId::Codex);
    let haiku = Some(serde_json::json!({ "model": "claude-haiku-5-5" }));
    assert_eq!(icon(&frame_with(spawn_with(SubagentStatus::Done, haiku), codex)).as_deref(), Some("mark-claude-code"), "the model's maker wins");
    assert_eq!(icon(&frame_with(spawn(SubagentStatus::Done), codex)).as_deref(), Some("mark-codex"), "no model: the chat's harness");
    assert_eq!(icon(&frame_for(SubagentStatus::Done)).as_deref(), Some("tool-bot"), "neither: the bot");
    // The whole card opens the child's transcript, by its doc.
    let done = frame_for(SubagentStatus::Done);
    let display = done.display(1).unwrap();
    let card = display.widgets.iter().find_map(|w| match &w.kind {
        display::WidgetKind::Subagent { doc_id, title, running, .. } => Some((doc_id.clone(), title.clone(), *running, w.w)),
        _ => None,
    });
    let (doc, title, running, width) = card.expect("the card is a tap target");
    assert_eq!(doc.as_deref(), Some("sub-1"));
    assert_eq!(title, "scan the repo");
    assert!(!running);
    assert!(width > 300.0, "it spans the card");
    // Finished and openable: a chevron where the spinner was.
    assert!(display.widgets.iter().any(|w| matches!(&w.kind, display::WidgetKind::Icon { name, .. } if name == "chevron.right")));
}

#[test]
fn links_get_hit_regions() {
    for md in [
        "See [the docs](https://example.com/docs) for details.",
        "1. first\n2. second\n3. third [link](https://ja.wikipedia.org/wiki/x)",
        "- [UIScrollView docs](https://developer.apple.com/documentation/uikit/uiscrollview)",
        "> quoted [link](https://example.com/q)",
    ] {
        let mut w = worker(390.0);
        w.input = transcript_one(md);
        w.pass();
        let frame = w.shared.frame.lock().unwrap().clone();
        let links: Vec<_> = (0..frame.row_count()).flat_map(|i| frame.display(i).unwrap().links).collect();
        assert!(!links.is_empty(), "no link hits for {md:?}");
    }
}

fn message(id: &str, role: zeron_doc::schema::MessageRole, parts: Vec<MessagePart>) -> Arc<SessionMessageEntry> {
    Arc::new(SessionMessageEntry {
        id: id.into(),
        role,
        parts,
        created_at: 0,
        device_id: "mac".into(),
        status: Some(MessageStatus::Complete),
        continuation_of: None,
        duration_ms: None,
        from_chat_id: None,
        fork_import: false,
        answers_request_id: None,
    })
}

fn text_part(id: &str, text: &str) -> MessagePart {
    MessagePart::Text { id: id.into(), text: text.into() }
}

fn computer_part(id: &str, tool: &str, input: Option<serde_json::Value>, is_error: bool, output: Option<&str>) -> MessagePart {
    MessagePart::Tool {
        id: id.into(),
        call: zeron_proto::ToolCall::Mcp { server: "zeron".into(), tool: tool.into(), input },
        is_error,
        resolved: true,
        output: output.map(str::to_owned),
        diff: None,
        output_ref: None,
        output_bytes: None,
        diff_ref: None,
        diff_stats: None,
        subagent_ref: None,
        subagent_status: None,
        subagent_tail: None,
        html_render: None,
        delegate_route: None,
    }
}

fn consent_part(request_id: &str) -> MessagePart {
    MessagePart::Input {
        id: request_id.into(),
        request_id: request_id.into(),
        questions: vec![zeron_proto::UserInputQuestion {
            approval: false,
            id: "consent".into(),
            header: "Computer".into(),
            question: "Allow Computer for this task? The agent wants to run computer_click and will drive this Mac's apps until the turn ends.".into(),
            options: vec!["Allow".into(), "Deny".into()],
            multi_select: false,
            kind: Some(zeron_proto::UserInputKind::Consent { scope: zeron_proto::computer::ComputerConsentScope::Task, app: None, tool: None, params: None }), prefill: None, multiline: false, 
        }],
        resolved: false,
        answers: None,
        asked_at_ms: None,
        answered_at_ms: None,
    }
}

fn frame_of(entries: Vec<Arc<SessionMessageEntry>>, width: f32) -> Arc<LayoutFrame> {
    let mut w = worker(width);
    w.input = TranscriptInput { entries, ..Default::default() };
    w.pass()
}

fn kinds(frame: &LayoutFrame) -> Vec<RowKind> {
    (0..frame.row_count()).map(|i| frame.placement(i).unwrap().kind).collect()
}

fn actions(frame: &LayoutFrame, row: u32) -> Vec<display::CardAction> {
    frame
        .display(row)
        .unwrap()
        .widgets
        .into_iter()
        .filter_map(|w| match w.kind {
            display::WidgetKind::CardButton { action } => Some(action),
            _ => None,
        })
        .collect()
}

#[test]
fn command_cards_tick_while_running_and_offer_retry_once_the_answer_never_came() {
    use zeron_doc::schema::MessageRole::System;
    use zeron_proto::claude_cards::{STALE_PENDING_MS, pending_card, side_headline};
    let now = chrono::Utc::now().timestamp_millis();
    let note = |id: &str, payloads: Vec<String>| {
        let mut parts = vec![text_part("t0", &side_headline("context"))];
        parts.extend(payloads.iter().enumerate().map(|(ix, p)| text_part(&format!("h{ix}"), p)));
        message(id, System, parts)
    };
    let json = |card: zeron_proto::claude_config::CommandCard| serde_json::to_string(&card).unwrap();
    let running = note("live", vec![json(pending_card("context", now))]);
    let stale = note("stale", vec![json(pending_card("context", now - STALE_PENDING_MS - 1))]);
    let retried = note("again", vec![json(pending_card("context", now - STALE_PENDING_MS - 1)), json(pending_card("context", now))]);
    let frame = frame_of(vec![running, stale, retried], 390.0);
    let working = |row: u32| frame.display(row).unwrap().widgets.iter().any(|w| matches!(w.kind, display::WidgetKind::Working { .. }));
    assert!(working(0) && working(2), "the newest part is the card shown");
    assert!(!working(1));
    assert_eq!(
        actions(&frame, 1).first(),
        Some(&display::CardAction::RetryCard { message_id: "stale".into(), command: "/context".into() })
    );

    let notice = |name: &str| {
        let card = zeron_proto::claude_config::CommandCard { command: format!("/{name}"), ..Default::default() };
        message(name, System, vec![text_part("t0", &side_headline(name)), text_part("h0", &json(card))])
    };
    let frame = frame_of(vec![notice("hooks"), notice("export")], 390.0);
    assert_eq!(actions(&frame, 0), [display::CardAction::OpenClaudeHooks]);
    assert_eq!(actions(&frame, 1), [display::CardAction::ExportChat]);
}

#[test]
fn computer_notes_take_the_desktop_shapes_and_other_system_text_is_a_divider() {
    use zeron_doc::schema::MessageRole::System;
    use zeron_proto::computer::cards::{
        COMPUTER_CONTROL_ON_DISCLOSURE, COMPUTER_DESKTOP_REVOKED_NOTE, COMPUTER_ESCAPE_STOP_NOTICE, COMPUTER_SETUP_NOTE,
        COMPUTER_UNAVAILABLE_NOTE,
    };
    let rows = |note: &str, hint: Option<&str>| {
        let mut parts = vec![text_part("t0", note)];
        parts.extend(hint.map(|h| text_part("t1", h)));
        frame_of(vec![message("n", System, parts)], 390.0)
    };
    assert!(kinds(&rows(COMPUTER_CONTROL_ON_DISCLOSURE, None)).is_empty(), "the model-facing disclosure is not shown");
    for (note, hint) in [
        (COMPUTER_SETUP_NOTE, "Allow Accessibility for Zeron in System Settings, then continue."),
        (COMPUTER_UNAVAILABLE_NOTE, "OpenCode does not receive Zeron's Computer tools."),
    ] {
        let frame = rows(note, Some(hint));
        assert_eq!(kinds(&frame), [RowKind::Card], "{note}");
        let d = frame.display(0).unwrap();
        assert!(d.text.starts_with(note) && d.text.contains(hint), "{}", d.text);
        assert!(actions(&frame, 0).is_empty());
    }
    for (note, hint, shown) in [
        (COMPUTER_ESCAPE_STOP_NOTICE, None, "Desktop access revoked by you (Esc)"),
        (COMPUTER_DESKTOP_REVOKED_NOTE, Some("Stop"), "Desktop access revoked by you"),
        ("Paused while you used Calculator", None, "Paused while you used Calculator"),
        ("Left 2 Calculator windows open", None, "Left 2 Calculator windows open"),
    ] {
        let frame = rows(note, hint);
        assert_eq!(kinds(&frame), [RowKind::Chip], "{note}");
        assert_eq!(frame.display(0).unwrap().text, shown);
    }
    let other = frame_of(vec![message("n", System, vec![text_part("t0", "Context compacted")])], 390.0);
    assert_eq!(kinds(&other), [RowKind::Chip]);
    assert_eq!(other.display(0).unwrap().text, "Context compacted");
}

#[test]
fn thread_sent_rows_route_by_sender() {
    use zeron_doc::schema::MessageRole::User;
    let from = |id: &str, chat: &str, text: &str| {
        let mut entry = (*message(id, User, vec![text_part("t0", text)])).clone();
        entry.from_chat_id = Some(chat.into());
        Arc::new(entry)
    };
    let child = seams::ChatRef { title: Some("Fix it".into()), delegated_by: Some("own".into()), ..Default::default() };
    let peer = seams::ChatRef { title: Some("Peer".into()), ..Default::default() };
    let mut w = worker(390.0);
    w.input = TranscriptInput {
        entries: vec![
            from("a", "child", "Delegated task child (\"Fix it\") completed."),
            from("b", "peer", "hello"),
            from("c", "own", "keep going"),
            message("d", User, vec![text_part("t0", zeron_proto::RESUME_PROMPT)]),
        ],
        chat_id: Some("own".into()),
        chats: Arc::new([("child".into(), child), ("peer".into(), peer)].into_iter().collect()),
        ..Default::default()
    };
    let frame = w.pass();
    assert_eq!(kinds(&frame), [RowKind::Chip, RowKind::Chip, RowKind::Markdown, RowKind::Chip, RowKind::User, RowKind::Chip]);
    let text = |i| frame.display(i).unwrap().text;
    assert!(text(0).contains("Fix it"), "{}", text(0));
    assert!(text(1).contains("Peer"), "{}", text(1));
    assert_eq!(text(5), "Resumed");
}

#[test]
fn device_calls_group_apart_with_their_own_wording() {
    use zeron_doc::schema::MessageRole::Assistant;
    let call = |id: &str, tool: &str| {
        let mut part = computer_part(id, "x", None, false, None);
        if let MessagePart::Tool { call, .. } = &mut part {
            *call = zeron_proto::ToolCall::Mcp { server: "zeron".into(), tool: tool.into(), input: None };
        }
        part
    };
    let mut w = worker(390.0);
    w.input = TranscriptInput {
        entries: vec![Arc::new(SessionMessageEntry {
            status: Some(MessageStatus::Streaming),
            ..(*message("a", Assistant, vec![computer_part("t0", "x", None, false, None), call("t1", "device_open"), call("t2", "device_close")])).clone()
        })],
        working: true,
        streaming: true,
        ..Default::default()
    };
    let frame = w.pass();
    assert_eq!(kinds(&frame)[..2], [RowKind::Tools, RowKind::Tools]);
    let devices = frame.display(1).unwrap().text;
    assert!(devices.contains("Used device controls 2 times"), "{devices}");
    assert!(devices.contains("Opened a device in the Device panel"), "{devices}");
}

#[test]
fn a_file_change_line_carries_its_added_and_deleted_counts() {
    use zeron_doc::schema::MessageRole::Assistant;
    let mut part = computer_part("t1", "x", None, false, None);
    if let MessagePart::Tool { call, diff_stats, .. } = &mut part {
        *call = zeron_proto::ToolCall::ApplyPatch { path: None };
        *diff_stats = Some(vec![
            zeron_doc::parts::ToolDiffStat { path: "a.rs".into(), additions: 2, deletions: 1 },
            zeron_doc::parts::ToolDiffStat { path: "b.rs".into(), additions: 3, deletions: 2 },
        ]);
    }
    let mut w = worker(390.0);
    w.input = TranscriptInput {
        entries: vec![Arc::new(SessionMessageEntry {
            status: Some(MessageStatus::Streaming),
            ..(*message("a", Assistant, vec![part])).clone()
        })],
        working: true,
        streaming: true,
        ..Default::default()
    };
    let text = w.pass().display(0).unwrap().text;
    assert!(text.contains("+5") && text.contains("−3"), "{text}");
}

#[test]
fn a_stats_only_change_offers_its_full_diff_and_shows_it_once_fetched() {
    use zeron_doc::schema::MessageRole::Assistant;
    let mut part = computer_part("t1", "x", None, false, None);
    if let MessagePart::Tool { call, diff_stats, .. } = &mut part {
        *call = zeron_proto::ToolCall::ApplyPatch { path: None };
        *diff_stats = Some(vec![zeron_doc::parts::ToolDiffStat { path: "a.rs".into(), additions: 1, deletions: 1 }]);
    }
    let mut w = worker(390.0);
    w.input = TranscriptInput {
        entries: vec![Arc::new(SessionMessageEntry {
            status: Some(MessageStatus::Streaming),
            ..(*message("a", Assistant, vec![part])).clone()
        })],
        working: true,
        streaming: true,
        chat_id: Some("c".into()),
        ..Default::default()
    };
    let key = w.pass().placement(0).unwrap().key;
    w.builder.expanded.insert(key);
    w.builder.detail_open.insert(rows::row_key("a#g0/t1"), true);
    w.builder.invalidate(key);
    let d = w.pass().display(0).unwrap();
    assert!(d.text.contains("Show full diff"), "{}", d.text);
    assert!(d.links.iter().any(|l| l.url.starts_with("zeron-blob://fetch?ref=c") && l.url.ends_with("t1.diff")), "{:?}", d.links);
    let diff = zeron_proto::ToolDiff { path: "a.rs".into(), old_text: Some("old line\n".into()), new_text: "new line\n".into(), line_offset: None };
    w.builder.blobs.insert("c/t1.diff".into(), (rows::Blob::Ready(serde_json::to_string(&diff).unwrap().into()), 1));
    let d = w.pass().display(0).unwrap();
    assert!(d.text.contains("new line") && !d.text.contains("Show full diff"), "{}", d.text);
}

#[test]
fn an_advisor_call_names_the_wait_until_it_answers() {
    use zeron_doc::schema::MessageRole::Assistant;
    let frame = |resolved: bool| {
        let mut entry = (*message("a", Assistant, vec![computer_part("t1", "x", None, false, None)])).clone();
        entry.status = Some(MessageStatus::Streaming);
        let MessagePart::Tool { call, resolved: done, .. } = &mut entry.parts[0] else { unreachable!() };
        *call = zeron_proto::ToolCall::Unknown { name: zeron_proto::ADVISOR_TOOL_NAME.into(), input: None };
        *done = resolved;
        let mut w = worker(390.0);
        w.input = TranscriptInput { entries: vec![Arc::new(entry)], working: true, streaming: true, ..Default::default() };
        w.pass()
    };
    let word = |f: &LayoutFrame| {
        (0..f.row_count())
            .find_map(|i| f.display(i).unwrap().widgets.iter().find_map(|w| match &w.kind {
                display::WidgetKind::Working { word, .. } => Some(word.clone()),
                _ => None,
            }))
            .unwrap()
    };
    let out = frame(false);
    assert_eq!(word(&out), "Consulting advisor");
    assert!(out.display(0).unwrap().text.contains("onsulting advisor"), "{}", out.display(0).unwrap().text);
    let answered = frame(true);
    assert_eq!(word(&answered), "Writing");
    assert!(answered.display(0).unwrap().text.contains("onsulted advisor"), "{}", answered.display(0).unwrap().text);
}

#[test]
fn the_working_row_follows_the_token_rate() {
    use zeron_doc::schema::MessageRole::Assistant;
    let mut w = worker(390.0);
    let rate = |w: &mut Worker, token_rate: Option<u32>| {
        w.input = TranscriptInput {
            entries: vec![message("a", Assistant, vec![text_part("t0", "hi")])],
            working: true,
            token_rate,
            ..Default::default()
        };
        let f = w.pass();
        (0..f.row_count())
            .find_map(|i| f.display(i).unwrap().widgets.iter().find_map(|w| match &w.kind {
                display::WidgetKind::Working { rate, .. } => Some(rate.clone()),
                _ => None,
            }))
            .unwrap()
    };
    assert_eq!(rate(&mut w, None), None);
    assert_eq!(rate(&mut w, Some(40)).as_deref(), Some("· 40 tok/s"));
    assert_eq!(rate(&mut w, Some(58)).as_deref(), Some("· 58 tok/s"));
}

#[test]
fn an_idle_chat_waiting_on_compaction_shows_the_wait_row() {
    use zeron_doc::schema::MessageRole::Assistant;
    use zeron_proto::transcript::CompactionWait;
    let mut w = worker(390.0);
    let shown = |w: &mut Worker, compaction: Option<CompactionWait>| {
        w.input = TranscriptInput {
            entries: vec![message("a", Assistant, vec![text_part("t0", "hi")])],
            token_rate: Some(40),
            compaction: compaction.map(|wait| (wait, 1_000)),
            ..Default::default()
        };
        let f = w.pass();
        (0..f.row_count()).find_map(|i| {
            f.display(i)
                .unwrap()
                .widgets
                .iter()
                .find_map(|w| match &w.kind {
                    display::WidgetKind::Working {
                        word,
                        since_ms,
                        rate,
                        divider,
                    } => Some((word.clone(), *since_ms, rate.clone(), *divider)),
                    _ => None,
                })
        })
    };
    assert_eq!(shown(&mut w, None), None);
    assert_eq!(
        shown(
            &mut w,
            Some(CompactionWait::Compacting(Some(zeron_proto::transcript::Boundary {
                tokens: 120_000,
                carry: true,
                cause: zeron_doc::DecidedBy::Streak.compaction_cause(),
            })))
        ),
        Some((
            "Compacting 120.0k after three same-task verdicts, carrying the last turns over"
                .to_owned(),
            Some(1_000),
            None,
            true
        ))
    );
    assert_eq!(
        shown(&mut w, Some(CompactionWait::Queued))
            .map(|(word, since, _, divider)| (word, since, divider)),
        Some((
            "Compaction queued · starts when the turn settles".to_owned(),
            None,
            true
        ))
    );
}

#[test]
fn structured_user_text_is_markdown_and_prose_stays_plain() {
    use zeron_doc::schema::MessageRole::User;
    let mut w = worker(390.0);
    w.input = TranscriptInput {
        entries: vec![
            message("a", User, vec![text_part("t0", "Two things:\n\n- one\n- two")]),
            message("b", User, vec![text_part("t0", "plain prose")]),
        ],
        ..Default::default()
    };
    let frame = w.pass();
    let text = |i| frame.display(i).unwrap().text;
    assert!(text(0).contains("one") && !text(0).contains("- one"), "{}", text(0));
    assert_eq!(text(1), "plain prose");
}

#[test]
fn a_context_block_becomes_a_pill_that_opens_its_card() {
    use zeron_doc::schema::MessageRole::User;
    let sent = format!(
        "Review this\n\n{}\n> Pull request #7 in acme/zeron: Fix 7\n> Body line\n\n> Checks: 1 of 3 passed",
        zeron_proto::badges::PR_CONTEXT_HEADER
    );
    let mut w = worker(390.0);
    w.input = TranscriptInput { entries: vec![message("a", User, vec![text_part("t0", &sent)])], ..Default::default() };
    let frame = w.pass();
    let row = frame.display(0).unwrap();
    assert!(row.text.contains("PR context") && row.text.contains("Review this"), "{}", row.text);
    assert!(!row.text.contains("Pull request #7"), "{}", row.text);
    let pill = row.links.iter().find(|l| l.url == "zeron-badge://open?i=0").expect("pill hit");
    assert!(pill.y < row.height / 2.0);
    let card = frame.badge(row.key, 0).unwrap();
    assert_eq!(card.title, "PR context");
    let locations: Vec<&str> = card.details.iter().map(|d| d.location.as_str()).collect();
    assert_eq!(locations, ["Pull request #7 in acme/zeron: Fix 7", "Checks: 1 of 3 passed"]);
    assert_eq!(card.details[0].body, "Body line");
}

#[test]
fn consent_is_a_card_with_its_choices_until_an_answer_settles_it() {
    use zeron_doc::schema::MessageRole::Assistant;
    use zeron_proto::computer::cards::{ConsentChoice, consent_answers};
    let entry = message("m", Assistant, vec![consent_part("req-1")]);
    let mut w = worker(390.0);
    w.input = TranscriptInput { entries: vec![entry.clone()], ..Default::default() };
    let open = w.pass();
    assert_eq!(kinds(&open), [RowKind::Card]);
    let answers: Vec<(String, String)> = actions(&open, 0)
        .into_iter()
        .map(|a| match a {
            display::CardAction::Consent { request_id, question_id, answer, identifier } => {
                assert_eq!((request_id.as_str(), question_id.as_str()), ("req-1", "consent"));
                (answer, identifier)
            }
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        answers,
        [("Deny".to_owned(), "computer-consent-deny".to_owned()), ("Allow".to_owned(), "computer-consent-allow".to_owned())]
    );
    let text = open.display(0).unwrap().text;
    assert!(text.starts_with("Allow Zeron to use this Mac?"), "{text}");
    assert!(text.contains("The agent wants to run computer_click"), "{text}");
    assert!(text.contains("Allow this conversation"), "{text}");
    assert!(!text.contains("Elevated Risk"), "{text}");

    // The same entry Arc: only the in-flight answer changed.
    let sent = consent_answers("consent", &ConsentChoice::Allow);
    w.input = TranscriptInput {
        entries: vec![entry],
        input_answers: Arc::new([("req-1".to_owned(), sent)].into_iter().collect()),
        ..Default::default()
    };
    let settled = w.pass();
    assert_eq!(kinds(&settled), [RowKind::Chip]);
    assert!(actions(&settled, 0).is_empty(), "a settled consent has no buttons");
    assert_eq!(settled.display(0).unwrap().text, "Allowed Computer for this task");
}

#[test]
fn an_app_consent_names_the_app_and_warns_of_its_risk() {
    use zeron_doc::schema::MessageRole::Assistant;
    let mut part = consent_part("req-1");
    let MessagePart::Input { questions, .. } = &mut part else { unreachable!() };
    questions[0].kind = Some(zeron_proto::UserInputKind::Consent {
        scope: zeron_proto::computer::ComputerConsentScope::Task,
        app: Some(zeron_proto::computer::ComputerConsentApp { name: "Terminal".into(), bundle_id: Some("com.apple.Terminal".into()), pid: None }),
        tool: None,
        params: None,
    });
    let frame = frame_of(vec![message("m", Assistant, vec![part])], 390.0);
    let text = frame.display(0).unwrap().text;
    assert!(text.contains("Allow Zeron to use Terminal?"), "{text}");
    assert!(text.contains("Elevated Risk") && text.contains("run commands that change files"), "{text}");
    let labels: Vec<String> = actions(&frame, 0)
        .into_iter()
        .filter_map(|a| match a {
            display::CardAction::Consent { identifier, .. } => Some(identifier),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["computer-consent-deny", "computer-consent-always", "computer-consent-allow"]);
}

#[test]
fn a_foreground_request_is_a_card_until_answered_then_its_outcome() {
    use zeron_doc::schema::MessageRole::Assistant;
    let part = |answers: Option<Vec<zeron_proto::UserInputAnswer>>, resolved: bool| MessagePart::Input {
        id: "fg".into(),
        request_id: "fg".into(),
        questions: vec![zeron_proto::UserInputQuestion {
            approval: false,
            id: "foreground".into(),
            header: "Computer".into(),
            question: "Bring Calculator to the front?".into(),
            options: vec!["No".into(), "Allow for this turn".into()],
            multi_select: false,
            kind: Some(zeron_proto::UserInputKind::Foreground {
                app: Some(zeron_proto::computer::ComputerConsentApp { name: "Calculator".into(), bundle_id: None, pid: None }),
            }),
            prefill: None,
            multiline: false,
        }],
        resolved,
        answers,
        asked_at_ms: None,
        answered_at_ms: None,
    };
    let open = frame_of(vec![message("m", Assistant, vec![part(None, false)])], 390.0);
    assert_eq!(kinds(&open), [RowKind::Card]);
    assert!(open.display(0).unwrap().text.starts_with("Bring Calculator to the front?"));
    let ids: Vec<String> = actions(&open, 0)
        .into_iter()
        .filter_map(|a| match a {
            display::CardAction::Consent { identifier, .. } => Some(identifier),
            _ => None,
        })
        .collect();
    assert_eq!(ids, ["computer-foreground-decline", "computer-foreground-allow"]);
    let allowed = vec![zeron_proto::UserInputAnswer { question_id: "foreground".into(), labels: vec!["Allow for this turn".into()] }];
    let settled = frame_of(vec![message("m", Assistant, vec![part(Some(allowed), true)])], 390.0);
    assert_eq!(kinds(&settled), [RowKind::Chip]);
    assert_eq!(settled.display(0).unwrap().text, "You allowed Calculator to come to the front");
    assert!(kinds(&frame_of(vec![message("m", Assistant, vec![part(None, true)])], 390.0)).is_empty());
}

#[test]
fn capability_denial_adds_one_card_per_tool_after_its_chip() {
    use zeron_doc::schema::MessageRole::Assistant;
    use zeron_proto::computer::cards::{COMPUTER_DENIED_HINT, COMPUTER_DENIED_TITLE};
    let denied = r#"{"error":{"code":"capability_denied","message":"Computer control is off for this turn; no input was sent."}}"#;
    let parts = vec![
        computer_part("c1", "computer_click", None, true, Some(denied)),
        computer_part("c2", "computer_click", None, true, Some(denied)),
        computer_part("c3", "computer_type_text", None, true, Some(denied)),
        computer_part("c4", "computer_click", None, true, Some("approval_denied")),
    ];
    let mut w = worker(390.0);
    w.builder.expanded.insert(rows::row_key("m#work"));
    w.input = TranscriptInput { entries: vec![message("m", Assistant, parts)], ..Default::default() };
    let frame = w.pass();
    assert_eq!(kinds(&frame), [RowKind::Tools, RowKind::Tools, RowKind::Card, RowKind::Tools, RowKind::Card, RowKind::Tools]);
    let card = frame.display(2).unwrap();
    assert!(card.text.starts_with(COMPUTER_DENIED_TITLE), "{}", card.text);
    assert!(card.text.contains(COMPUTER_DENIED_HINT));
    assert_eq!(actions(&frame, 2), [display::CardAction::EnableComputer]);

    w.builder.expanded.clear();
    let frame = w.pass();
    let cards: Vec<u32> = (0u32..).zip(kinds(&frame)).filter(|(_, k)| *k == RowKind::Card).map(|(i, _)| i).collect();
    assert_eq!(cards.len(), 2, "the denied cards stay outside a closed fold, as on the desktop");
    assert_eq!(actions(&frame, cards[0]), [display::CardAction::EnableComputer]);
}

#[test]
fn computer_tools_take_the_desktop_label_and_never_repeat_arguments_in_detail() {
    use zeron_doc::schema::MessageRole::Assistant;
    let secret = "hunter2-typed-secret";
    let parts = vec![computer_part("c1", "computer_type_text", Some(serde_json::json!({ "text": secret })), false, Some(secret))];
    let mut w = worker(390.0);
    w.builder.expanded.insert(rows::row_key("m#work"));
    w.builder.expanded.insert(rows::row_key("m#g0"));
    w.builder.detail_open.insert(rows::row_key("m#g0/c1"), true);
    w.input = TranscriptInput { entries: vec![message("m", Assistant, parts)], ..Default::default() };
    let frame = w.pass();
    let d = frame.display(1).unwrap();
    assert!(d.text.starts_with("Used the desktop · 1 action"), "{}", d.text);
    assert!(d.text.contains(&format!("Typed text \u{201c}{secret}\u{201d} in App")), "{}", d.text);
    assert_eq!(d.text.matches(secret).count(), 1, "the detail rows never repeat it: {}", d.text);
}

#[test]
fn computer_cards_paint_what_they_measure_at_narrow_widths() {
    use zeron_doc::schema::MessageRole::Assistant;
    for width in [240.0, 320.0, 390.0, 744.0] {
        let frame = frame_of(vec![message("m", Assistant, vec![consent_part("req-1")])], width);
        let p = frame.placement(0).unwrap();
        let d = frame.display(0).unwrap();
        assert!((d.height - p.height).abs() < 0.01);
        for w in &d.widgets {
            assert!(w.x >= 0.0 && w.x + w.w <= width + 0.5 && w.y + w.h <= d.height + 0.5, "{w:?} at {width}");
        }
        for run in &d.runs {
            assert!(run.baseline <= d.height + 0.5, "{run:?} at {width}");
        }
    }
}

#[test]
fn visual_rows_take_the_height_their_worker_measures() {
    struct Ping(Mutex<mpsc::Sender<u64>>);
    impl LayoutListener for Ping {
        fn frame_ready(&self, revision: u64) {
            let _ = self.0.lock().unwrap().send(revision);
        }
    }
    let (tx, frames) = mpsc::channel();
    let view = TranscriptView::new(text_system(), Arc::new(Ping(Mutex::new(tx))));
    let script = r#"return ui.column { gap = 10, ui.text "Hello", ui.canvas { height = 200, draw = function(g) end } }"#;
    view.visuals.set_fetch(Arc::new(move |path: &str| (path == "up/v1-visual.luau").then(|| script.as_bytes().to_vec())), 2.0);
    view.set_viewport(390.0, 1.0);
    view.set_input(TranscriptInput {
        entries: vec![Arc::new(SessionMessageEntry {
            id: "a1".into(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::Visual { id: "v1".into(), path: "up/v1-visual.luau".into(), title: "Chart".into() }],
            created_at: 0,
            device_id: String::new(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            duration_ms: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
        })],
        ..Default::default()
    });
    // The loading line first, then the laid-out visual (text, gap, canvas).
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    let (frame, row) = loop {
        let _ = frames.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("a frame");
        let frame = view.frame();
        if let Some(row) = frame.placement(0).filter(|p| p.kind == RowKind::Visual && p.height > 200.0) {
            break (frame, row);
        }
    };
    let display = frame.display(row.index).unwrap();
    let widget = display.widgets.iter().find(|w| matches!(&w.kind, display::WidgetKind::Visual { id } if id == "v1")).expect("a visual widget");
    assert!((widget.y + widget.h - row.height).abs() < 0.5, "the widget fills the row below its gap");
    assert!(view.visual("v1".into()).is_some());
    view.close();
}

/// Every row of a one-message transcript at `width`.
fn rows_of(md: &str, width: f32) -> Vec<RowDisplay> {
    let mut w = worker(width);
    w.input = transcript_one(md);
    let frame = w.pass();
    (0..frame.row_count()).map(|i| frame.display(i).unwrap()).collect()
}

/// Inline math paints as glyphs on the text's baseline, never as TeX, and
/// a tall formula grows its line.
#[test]
fn inline_math_paints_glyphs_and_grows_its_line() {
    let plain = rows_of("Plain text on one line.", 390.0).remove(0);
    let d = rows_of(r"A fraction $\frac{a^2}{b_k}$ on one line.", 390.0).remove(0);
    assert!(!d.glyphs.is_empty(), "formula glyphs");
    assert!(!d.text.contains(r"\frac") && !d.text.contains('$'), "{}", d.text);
    assert!(d.runs.iter().all(|r| !run_text(&d, r).contains('\u{FFFC}')), "the placeholder is never text");
    assert!(d.boxes.iter().any(|b| b.color == display::ColorRole::Text && b.radius == 0.0), "the fraction bar is a fill");
    assert!(d.height > plain.height + 1.0, "{} vs {}", d.height, plain.height);
    // Text and formula share one baseline.
    let baseline = d.runs[0].baseline;
    assert!(d.runs.iter().all(|r| (r.baseline - baseline).abs() < 0.01));
    let text_end = d.runs.iter().find(|r| run_text(&d, r).starts_with("A fraction")).map(|r| r.x + r.width).unwrap();
    let tail = d.runs.iter().find(|r| run_text(&d, r).contains("on one line")).unwrap();
    for g in &d.glyphs {
        assert!(g.x > text_end - 0.5 && g.x < tail.x, "glyph between the words: {g:?}");
        assert!(g.y > 0.0 && g.y <= d.height + 0.01 && g.scroller.is_none(), "{g:?} in {}", d.height);
        assert_eq!(g.color, display::ColorRole::Text);
    }
    // `start` points at the placeholder, for the streaming veil.
    let units: Vec<u16> = d.text.encode_utf16().collect();
    assert_eq!(units[d.glyphs[0].start as usize], 0xFFFC);
    assert_eq!(d.copy_text, r"A fraction $\frac{a^2}{b_k}$ on one line.");
}

/// Wrapped paragraphs with formulas measure exactly what they paint, and
/// every glyph stays inside its row — in every face at 100%, and in a sans and
/// a serif at 150%.
#[test]
fn math_paint_matches_measure_at_many_widths() {
    let md = concat!(
        r"Euler: $e^{i\pi} + 1 = 0$, sums $\sum_{k=1}^{n} k = \frac{n(n+1)}{2}$, roots $\sqrt[3]{x^3 + y^3}$ and ",
        r"matrices $\begin{pmatrix} a & b \\ c & d \end{pmatrix}$ all wrap with the prose around them, and ",
        r"[a link with $x^2$ inside](https://example.com) keeps its hit region.",
        "\n\n",
        r"- item $\frac{1}{2}$ first",
        "\n\n",
        "| a | b |\n|---|---|\n| $\\frac{x}{y}$ | $z_i^2$ |\n\n",
        r"$$\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}$$",
        "\n\n",
        r"Too wide: $a_1 + a_2 + a_3 + a_4 + a_5 + a_6 + a_7 + a_8 + a_9 + a_{10} + a_{11} + a_{12} + a_{13} + a_{14}$ shrinks.",
        "\n\n",
        "| f | next |\n|---|---|\n| $a_1 + a_2 + a_3 + a_4 + a_5 + a_6 + a_7 + a_8 + a_9 + a_{10} + a_{11} + a_{12} + a_{13} + a_{14} + a_{15}$ | n |\n\n",
        "- ## Head $\\frac{a^2}{b^2}$ tall\n\n",
        r"~~struck $\frac{x}{y}$ math~~",
    );
    let settings = zatex::bundled::Face::ALL.map(|f| (f, 100)).into_iter().chain([(zatex::bundled::Face::Fira, 150), (zatex::bundled::Face::ComputerModern, 150)]);
    for ((face, size), width) in settings.flat_map(|f| [120.0, 200.0, 280.0, 320.0, 390.0, 744.0, 1024.0].map(|w| (f, w))) {
        let mut w = worker(width);
        w.math = MathSettings::new(true, face, size);
        w.input = transcript_one(md);
        let frame = w.pass();
        let mut glyphs = 0;
        for i in 0..frame.row_count() {
            let p = frame.placement(i).unwrap();
            let d = frame.display(i).unwrap();
            assert!((d.height - p.height).abs() < 0.01);
            for g in &d.glyphs {
                assert!(g.y > 0.0 && g.y < d.height + 0.5, "{face:?} width {width}: {g:?} in {}", d.height);
                match g.scroller {
                    Some(s) => {
                        let s = &d.scrollers[s as usize];
                        assert!(g.x >= -0.5 && g.x <= s.content_width + 0.5, "{face:?} width {width}: {g:?}");
                    }
                    None => assert!(g.x >= 0.0 && g.x <= width, "{face:?} width {width}: {g:?}"),
                }
            }
            glyphs += d.glyphs.len();
        }
        assert!(glyphs > 50, "{face:?} width {width}: {glyphs} glyphs");
    }
}

/// `$$…$$` is a centered block padded above and below; one wider than the
/// column scrolls.
#[test]
fn display_math_centers_and_scrolls_when_wide() {
    let tex = r"\int_0^1 x^2\,dx = \frac{1}{3}";
    let d = rows_of(&format!("$${tex}$$"), 390.0).remove(0);
    let f = zatex::bundled::layout(tex, zatex::bundled::Face::Fira, &zatex::Options::display(style::TYPE.body.0)).unwrap();
    let gap = rows::Gap::First.px(Px(1.0));
    let pad = markdown::geom::MATH_PAD_Y;
    assert!((d.height - (gap + f.height() + pad * 2.0)).abs() < 0.01, "{} vs {}", d.height, f.height());
    assert!(d.scrollers.is_empty() && d.runs.is_empty());
    let left = d.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
    let column = rows::column_width(Px(1.0), 390.0);
    let x0 = (390.0 - column) / 2.0;
    assert!((left - (x0 + (column - f.width) / 2.0)).abs() < 1.0, "centered: {left}");
    assert_eq!(d.copy_text, format!("$${tex}$$"));

    let wide = format!("$${}$$", r"a_1 + a_2 + a_3 + a_4 + a_5 + a_6 + a_7 + a_8 + a_9 + a_{10} + a_{11} + a_{12} + a_{13}");
    let d = rows_of(&wide, 320.0).remove(0);
    assert_eq!(d.scrollers.len(), 1);
    assert!(d.scrollers[0].content_width > d.scrollers[0].w);
    assert!(d.glyphs.iter().all(|g| g.scroller == Some(0)));
}

/// TeX zatex can't lay out keeps its source: inline as a code chip with its
/// dollars, display as a `math` code block.
#[test]
fn invalid_math_falls_back_to_source() {
    let d = rows_of(r"Unknown $\foo{x}$ here.", 390.0).remove(0);
    assert!(d.glyphs.is_empty());
    assert!(d.text.contains(r"$\foo{x}$"), "{}", d.text);
    let chip = d.runs.iter().find(|r| run_text(&d, r).contains("foo")).unwrap();
    assert_eq!(chip.color, display::ColorRole::InlineCodeText);
    assert!(d.boxes.iter().any(|b| b.color == display::ColorRole::InlineCodeBackground));

    // (`$$\frac{a$$` never reaches layout: the parser keeps unbalanced
    // braces as text.)
    let d = rows_of(r"$$\frac{a}$$", 390.0).remove(0);
    assert!(d.glyphs.is_empty());
    assert!(d.text.contains(r"\frac{a}"), "{}", d.text);
    assert!(d.text.contains("math"), "labelled a math block: {}", d.text);
    assert!(d.boxes.iter().any(|b| b.color == display::ColorRole::CodeBackground));
}

/// A runaway formula (the reviewer's `\\`×8000: thousands of empty lines)
/// is rejected by its extent and shows as source — no taller than that
/// source as inline code (~10,000pt at 390pt), not a ~198,000pt canvas; as
/// display math it's a one-line `math` code block.
#[test]
fn runaway_math_falls_back_to_a_bounded_row() {
    let tex = r"\\".repeat(8000);
    let height = |md: &str| {
        let mut w = worker(390.0);
        w.input = transcript_one(md);
        let frame = w.pass();
        let d = frame.display(0).unwrap();
        assert!(d.glyphs.is_empty(), "rendered as source");
        assert!(d.text.contains(&tex[..64]), "source shown");
        assert!((frame.total - d.height) < 100.0, "one row");
        (d.height, w.math_failures.len())
    };
    let (inline, failures) = height(&format!("Hi ${tex}$"));
    assert_eq!(failures, 1);
    let (code, _) = height(&format!("Hi `{tex}`"));
    assert!(inline <= code + 1.0 && inline < 12_000.0, "inline {inline} vs code {code}");
    // Long display source wraps rather than becoming one huge scrolling line.
    let (display, failures) = height(&format!("$${tex}$$"));
    assert_eq!(failures, 1);
    assert!(display < 12_000.0, "display {display}");
    let d = rows_of(&format!("$${tex}$$"), 390.0).remove(0);
    assert!(d.scrollers.iter().all(|s| s.content_width < 5_000.0), "{:?}", d.scrollers.iter().map(|s| s.content_width).collect::<Vec<_>>());
    // Short failures keep the `math` code block (its label and bare source).
    let d = rows_of(r"$$\foo{x}$$", 390.0).remove(0);
    assert!(d.text.contains("math") && d.text.contains(r"\foo{x}") && !d.text.contains("$$"), "{}", d.text);
}

/// A formula zatex rejected is remembered: preparing its row again skips
/// layout; the memory stays bounded at `CAP` keys, oldest out first.
#[test]
fn failed_formulas_are_cached_and_bounded() {
    let md = r"Bad $\foo{x}$ and again $\foo{x}$, display: ";
    let mut w = worker(390.0);
    w.input = transcript_one(&format!("{md}\n\n$$\\foo{{x}}$$"));
    let first = w.pass().display(0).unwrap();
    // Inline and display are separate keys; the repeat is a hit.
    assert_eq!(w.math_failures.len(), 2);
    assert_eq!(w.math_failures.hits, 1);
    w.rebuild();
    let again = w.pass().display(0).unwrap();
    assert_eq!(w.math_failures.len(), 2);
    assert_eq!(w.math_failures.hits, 4, "all three formulas skipped layout");
    assert_eq!(first.text, again.text, "same fallback");
    // Another face or size is another key.
    assert!(w.set_math(MathSettings::new(true, zatex::bundled::Face::StixTwo, 100)));
    w.pass();
    assert_eq!(w.math_failures.len(), 4);
    // Bounded: past CAP the oldest are evicted.
    let many: String = (0..MathFailures::CAP + 50).map(|i| format!("$\\foo{{{i}}}$ ")).collect();
    w.input = transcript_one(&many);
    w.pass();
    assert_eq!(w.math_failures.len(), MathFailures::CAP);
    // Good formulas are never recorded.
    let mut ok = worker(390.0);
    ok.input = transcript_one(r"Fine $x^2$ here.");
    ok.pass();
    assert_eq!(ok.math_failures.len(), 0);
}

/// Half-streamed formulas fall back to their source and settle into the same
/// rows a fresh parse lays out.
#[test]
fn streaming_math_converges_to_full_parse() {
    let md = concat!(
        r"Inline $\frac{a}{b} + \sqrt{x}$ and $y^2$ in prose.",
        "\n\n",
        r"$$\sum_{k=1}^{n} k = \frac{n(n+1)}{2}$$",
        "\n\nAfter.",
    );
    let mut w = worker(390.0);
    let mut shown = String::new();
    for c in md.chars() {
        shown.push(c);
        w.input = debug_input(vec![DebugEntry { id: "a".into(), user: false, text: shown.clone(), streaming: true }], true);
        let frame = w.pass();
        for i in 0..frame.row_count() {
            frame.display(i).unwrap();
        }
    }
    w.input = transcript_one(md);
    let streamed = w.pass();
    let mut fresh = worker(390.0);
    fresh.input = transcript_one(md);
    let full = fresh.pass();
    assert_eq!(streamed.row_count(), full.row_count());
    for i in 0..full.row_count() {
        let (a, b) = (streamed.display(i).unwrap(), full.display(i).unwrap());
        assert_eq!(a.key, b.key);
        assert_eq!(a.text, b.text, "row {i}");
        assert_eq!(a.glyphs, b.glyphs, "row {i}");
        assert!((a.height - b.height).abs() < 0.01, "row {i}: {} vs {}", a.height, b.height);
    }
    assert!((0..full.row_count()).all(|i| !full.display(i).unwrap().glyphs.is_empty() || i == full.row_count() - 1));
}

#[test]
fn math_fonts_cross_ffi() {
    let fonts: Vec<Vec<u8>> = (0..6).map(display::math_font_data).collect();
    for (i, data) in fonts.iter().enumerate() {
        assert!(!data.is_empty(), "font {i}");
        assert!(fonts[..i].iter().all(|other| other != data), "font {i} is its own file");
    }
    assert!(display::math_font_data(6).is_empty());
    assert!(display::math_font_data(u8::MAX).is_empty());
}

/// Every FFI face lays formulas out in its own font (its table index), with
/// STIX Two (1) only for what it lacks; the order matches `math_font_data`.
#[test]
fn every_math_face_lays_out() {
    use display::MathFace;
    let faces = [MathFace::Fira, MathFace::StixTwo, MathFace::ComputerModern, MathFace::Libertinus, MathFace::Pagella, MathFace::Euler];
    let md = r"Sum $\sum_{k=1}^{n} \frac{x_k^2}{\sqrt{y}} + \mathcal{L} = \alpha$ here.";
    for (i, face) in faces.into_iter().enumerate() {
        let zface = zatex::bundled::Face::from(face);
        assert_eq!(zface.index() as usize, i, "{face:?}");
        let row = rows_with_math(md, MathSettings { face: zface, ..MathSettings::default() }).remove(0);
        assert!(row.glyphs.len() > 8, "{face:?}: {:?}", row.glyphs);
        assert!(row.glyphs.iter().all(|g| g.font == i as u8 || g.font == 1), "{face:?}: {:?}", row.glyphs);
        assert!(row.glyphs.iter().any(|g| g.font == i as u8), "{face:?} paints from its own font");
    }
}

/// Every row of a one-message transcript at 390pt, with `math` settings.
fn rows_with_math(md: &str, math: MathSettings) -> Vec<RowDisplay> {
    let mut w = worker(390.0);
    w.math = math;
    w.input = transcript_one(md);
    let frame = w.pass();
    (0..frame.row_count()).map(|i| frame.display(i).unwrap()).collect()
}

/// With math rendering off, formulas show their source: inline as a code
/// chip with its dollars, display as a `math` code block.
#[test]
fn math_off_shows_source() {
    let off = MathSettings { enabled: false, ..MathSettings::default() };
    let d = rows_with_math(r"A fraction $\frac{a}{b}$ here.", off).remove(0);
    assert!(d.glyphs.is_empty());
    assert!(d.text.contains(r"$\frac{a}{b}$"), "{}", d.text);
    let chip = d.runs.iter().find(|r| run_text(&d, r).contains(r"\frac")).unwrap();
    assert_eq!(chip.color, display::ColorRole::InlineCodeText);
    assert!(d.boxes.iter().any(|b| b.color == display::ColorRole::InlineCodeBackground));

    let d = rows_with_math(r"$$\int_0^1 x\,dx$$", off).remove(0);
    assert!(d.glyphs.is_empty());
    assert!(d.text.contains(r"\int_0^1 x\,dx"), "{}", d.text);
    assert!(d.text.contains("math"), "labelled a math block: {}", d.text);
    assert!(d.boxes.iter().any(|b| b.color == display::ColorRole::CodeBackground));
}

/// STIX Two paints from font 1 only; Fira paints Latin from font 0 and
/// falls back to STIX Two (font 1) for script letters it lacks.
#[test]
fn math_face_picks_glyph_fonts() {
    use zatex::bundled::Face;
    let md = r"Loss $\mathcal{L} = x^2 + \alpha$ here.";
    let stix = rows_with_math(md, MathSettings { face: Face::StixTwo, ..MathSettings::default() }).remove(0);
    assert!(!stix.glyphs.is_empty());
    assert!(stix.glyphs.iter().all(|g| g.font == 1), "{:?}", stix.glyphs);

    let fira = rows_with_math(r"Sum $x^2 + \alpha$ here.", MathSettings::default()).remove(0);
    assert!(!fira.glyphs.is_empty());
    assert!(fira.glyphs.iter().all(|g| g.font == 0), "{:?}", fira.glyphs);
    let cal = rows_with_math(r"$\mathcal{L}$", MathSettings::default()).remove(0);
    assert!(!cal.glyphs.is_empty());
    assert!(cal.glyphs.iter().all(|g| g.font == 1), "{:?}", cal.glyphs);
}

/// Changing the math settings rebuilds every row (new versions, so platform
/// caches miss) and keeps what the user unfolded.
#[test]
fn math_setting_change_relayouts_and_keeps_folds() {
    let mut w = worker(390.0);
    let long = (0..40).map(|i| format!("line {i} of a long pasted prompt")).collect::<Vec<_>>().join("\n");
    w.input = debug_input(
        vec![
            DebugEntry { id: "u".into(), user: true, text: long, streaming: false },
            DebugEntry { id: "a".into(), user: false, text: r"Inline $x^2$ math.".into(), streaming: false },
        ],
        false,
    );
    let folded = w.pass();
    let key = folded.placement(0).unwrap().key;
    w.builder.expanded.insert(key);
    w.builder.invalidate(key);
    let open = w.pass();
    assert!(!w.set_math(MathSettings::default()), "unchanged settings are a no-op");
    assert!(w.set_math(MathSettings { enabled: false, ..MathSettings::default() }));
    let off = w.pass();
    assert_eq!(off.row_count(), open.row_count());
    for i in 0..off.row_count() {
        assert_ne!(off.placement(i).unwrap().version, open.placement(i).unwrap().version, "row {i}");
    }
    assert!((off.placement(0).unwrap().height - open.placement(0).unwrap().height).abs() < 0.01, "still unfolded");
    let math = (0..off.row_count()).find(|&i| !open.display(i).unwrap().glyphs.is_empty()).expect("a formula row");
    let d = off.display(math).unwrap();
    assert!(d.glyphs.is_empty() && d.text.contains("$x^2$"), "{}", d.text);
}

/// The math size scales formulas against the text around them: glyphs, the
/// inline box (so the words after it move) and display blocks alike, and a
/// taller formula grows its line further.
#[test]
fn math_size_scales_formulas() {
    let at = |percent| MathSettings::new(true, zatex::bundled::Face::Fira, percent);
    let plain = rows_with_math("A fraction x on one line.", at(100)).remove(0);
    let md = r"A fraction $\frac{a^2}{b_k}$ on one line.";
    let (normal, big) = (rows_with_math(md, at(100)).remove(0), rows_with_math(md, at(135)).remove(0));
    assert_eq!(normal.glyphs.len(), big.glyphs.len());
    for (a, b) in normal.glyphs.iter().zip(&big.glyphs) {
        assert!((b.size - a.size * 1.35).abs() < 0.01, "{a:?} vs {b:?}");
    }
    // The inline box: from the end of the words before it to the start of
    // the words after it, less the fixed thin space either side.
    let box_w = |d: &RowDisplay| {
        let before = d.runs.iter().find(|r| run_text(d, r).starts_with("A fraction")).map(|r| r.x + r.width).unwrap();
        let after = d.runs.iter().find(|r| run_text(d, r).contains("on one line")).unwrap().x;
        after - before - 3.0
    };
    assert!((box_w(&big) - box_w(&normal) * 1.35).abs() < 0.05, "{} vs {}", box_w(&big), box_w(&normal));
    let (grow, grow_big) = (normal.height - plain.height, big.height - plain.height);
    assert!(grow > 1.0 && grow_big > grow + 1.0, "{grow} then {grow_big}");

    let tex = r"\int_0^1 x^2\,dx = \frac{1}{3}";
    let block = |percent| {
        let d = rows_with_math(&format!("$${tex}$$"), at(percent)).remove(0);
        d.height - rows::Gap::First.px(Px(1.0)) - markdown::geom::MATH_PAD_Y * 2.0
    };
    let f = zatex::bundled::layout(tex, zatex::bundled::Face::Fira, &zatex::Options::display(style::TYPE.body.0)).unwrap();
    assert!((block(100) - f.height()).abs() < 0.01);
    assert!((block(135) - f.height() * 1.35).abs() < 0.05, "{} vs {}", block(135), f.height() * 1.35);

    // Out of range sizes clamp.
    assert_eq!(at(10).size_percent, 75);
    assert_eq!(at(1000).size_percent, 200);
}

/// Changing only the math size rebuilds every row and keeps folds too.
#[test]
fn math_size_change_relayouts_and_keeps_folds() {
    let mut w = worker(390.0);
    let long = (0..40).map(|i| format!("line {i} of a long pasted prompt")).collect::<Vec<_>>().join("\n");
    w.input = debug_input(
        vec![
            DebugEntry { id: "u".into(), user: true, text: long, streaming: false },
            DebugEntry { id: "a".into(), user: false, text: r"Inline $\frac{x^2}{y}$ math.".into(), streaming: false },
        ],
        false,
    );
    let folded = w.pass();
    let key = folded.placement(0).unwrap().key;
    w.builder.expanded.insert(key);
    w.builder.invalidate(key);
    let open = w.pass();
    assert!(w.set_math(MathSettings { size_percent: 135, ..MathSettings::default() }));
    let big = w.pass();
    assert_eq!(big.row_count(), open.row_count());
    for i in 0..big.row_count() {
        assert_ne!(big.placement(i).unwrap().version, open.placement(i).unwrap().version, "row {i}");
    }
    assert!((big.placement(0).unwrap().height - open.placement(0).unwrap().height).abs() < 0.01, "still unfolded");
    let math = (0..big.row_count()).find(|&i| !open.display(i).unwrap().glyphs.is_empty()).expect("a formula row");
    let (a, b) = (open.display(math).unwrap(), big.display(math).unwrap());
    assert!((b.glyphs[0].size - a.glyphs[0].size * 1.35).abs() < 0.01);
    assert!(b.height > a.height);
}

/// Leftmost x of the content column in a row `width` wide.
fn column_x(width: f32) -> (f32, f32) {
    let column = rows::column_width(Px(1.0), width);
    ((width - column) / 2.0, column)
}

/// An inline formula wider than the column shrinks to fit it (glyphs,
/// offsets and rules alike) instead of painting off the edge, and its line
/// keeps the unscaled formula's height.
#[test]
fn overly_wide_inline_formula_shrinks_to_the_column() {
    let tex = r"a_1 + a_2 + a_3 + a_4 + a_5 + a_6 + a_7 + a_8 + a_9 + a_{10} + a_{11} + a_{12} + \frac{a_{13}}{a_{14}}";
    let f = zatex::bundled::layout(tex, zatex::bundled::Face::Fira, &zatex::Options::inline(style::TYPE.body.0)).unwrap();
    for width in [200.0, 320.0] {
        let (x0, column) = column_x(width);
        assert!(f.width > column, "the formula must overflow at {width}");
        let d = rows_of(&format!("${tex}$"), width).remove(0);
        assert!(!d.glyphs.is_empty());
        let right = x0 + column + 0.5;
        assert!(d.glyphs.iter().all(|g| g.x >= x0 && g.x <= right), "width {width}");
        assert!(d.glyphs.iter().all(|g| g.size < style::TYPE.body.0), "scaled glyphs");
        let rules: Vec<_> = d.boxes.iter().filter(|b| b.start.is_some()).collect();
        assert!(!rules.is_empty(), "the fraction bar");
        assert!(rules.iter().all(|b| b.x >= x0 && b.x + b.w <= right), "width {width}: {rules:?}");
        // The last glyph's origin lands near the column's right edge.
        let last = d.glyphs.iter().map(|g| g.x).fold(0f32, f32::max);
        assert!(last > x0 + column * 0.8, "fills the column: {last}");
    }
}

/// A table column grows to its widest formula (the table scrolls), so the
/// formula never paints over the next column.
#[test]
fn table_formula_cell_does_not_overlap_the_next_column() {
    let tex = r"a_1 + a_2 + a_3 + a_4 + a_5 + a_6 + a_7 + a_8 + a_9 + a_{10} + a_{11} + a_{12} + a_{13} + a_{14} + a_{15}";
    let d = rows_of(&format!("| f | next |\n|---|---|\n| ${tex}$ | after |\n"), 390.0).remove(0);
    assert_eq!(d.scrollers.len(), 1, "the widened table scrolls");
    let next = d.runs.iter().find(|r| run_text(&d, r) == "after").expect("next cell");
    let f = zatex::bundled::layout(tex, zatex::bundled::Face::Fira, &zatex::Options::inline(style::TYPE.body.0)).unwrap();
    let left = d.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
    assert!(left + f.width <= next.x, "{left} + {} vs {}", f.width, next.x);
    assert!(d.glyphs.iter().all(|g| (g.size - style::TYPE.body.0).abs() < 0.01 || g.size < style::TYPE.body.0), "unscaled");
    assert!(d.glyphs.iter().any(|g| (g.size - style::TYPE.body.0).abs() < 0.01), "not shrunk");
}

/// A list marker sits on its item's first line even when that line is a
/// heading grown by a tall formula.
#[test]
fn list_marker_follows_a_heading_grown_by_a_formula() {
    let offset = |md: &str| {
        let d = rows_of(md, 390.0).remove(0);
        let marker = d.runs.iter().find(|r| run_text(&d, r) == "•").expect("marker").baseline;
        let head = d.runs.iter().find(|r| run_text(&d, r).starts_with("Head")).expect("heading").baseline;
        head - marker
    };
    let plain = offset("- ## Head tall\n");
    let grown = offset("- ## Head $\\frac{a^2}{b^2}$ tall\n");
    assert!((plain - grown).abs() < 0.01, "{plain} vs {grown}");
}

/// Strikethrough crosses a formula too: a 1pt fill in the run's color at the
/// strike offset, veiled with the formula.
#[test]
fn strikethrough_crosses_a_formula() {
    let d = rows_of(r"~~$x^2 + y^2$~~", 390.0).remove(0);
    assert!(!d.glyphs.is_empty());
    let start = d.glyphs[0].start;
    let baseline = d.glyphs[0].y;
    let strike = d.boxes.iter().find(|b| b.start == Some(start) && (b.y - (baseline - 5.0)).abs() < 0.01).expect("strike line");
    assert_eq!(strike.h, 1.0);
    assert_eq!(strike.color, display::ColorRole::Text);
    assert!(strike.w > 10.0);
}

/// Formula rules carry their placeholder offset so the streaming veil fades
/// them with the glyphs; chrome boxes don't.
#[test]
fn formula_rules_veil_with_their_glyphs() {
    let d = rows_of("Code chip `x` and $\\frac{a}{b}$.", 390.0).remove(0);
    let start = d.glyphs[0].start;
    assert!(d.boxes.iter().any(|b| b.start == Some(start)), "fraction bar");
    assert!(d.boxes.iter().filter(|b| b.color == display::ColorRole::InlineCodeBackground).all(|b| b.start.is_none()));
}

/// Scrolling display math pads its content so ink past the advance stays
/// visible at both scroll ends.
#[test]
fn scrolling_display_math_pads_its_sides() {
    let d = rows_of(&format!("$${}$$", r"a_1 + a_2 + a_3 + a_4 + a_5 + a_6 + a_7 + a_8 + a_9 + a_{10} + a_{11} + a_{12} + a_{13}"), 320.0).remove(0);
    let s = &d.scrollers[0];
    let left = d.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
    assert!(left >= 3.0, "{left}");
    assert!(s.content_width > s.w);
}

/// Laid-out formulas are reused: a rebuild (or a streamed token
/// re-preparing its paragraph) doesn't lay any of them out again.
#[test]
fn laid_out_formulas_are_reused() {
    let mut w = worker(390.0);
    w.input = transcript_one(r"Fine $x^2$ and $\frac{a}{b}$ here.");
    let first = w.pass().display(0).unwrap();
    assert_eq!(w.math_failures.laid_hits, 0);
    w.rebuild();
    let again = w.pass().display(0).unwrap();
    assert_eq!(w.math_failures.laid_hits, 2, "both formulas came from the cache");
    assert_eq!(first.glyphs.len(), again.glyphs.len());
}

/// Formulas zatex accepts but this host can't afford show as source: an
/// inline one taller than a line can hold, or display math whose scrolling
/// canvas would be an outsized backing store.
#[test]
fn oversized_formulas_show_as_source() {
    for md in [
        r"Tall $\begin{matrix}x\\[40em]y\end{matrix}$ inline",
        r"$$\hspace{70em}\hspace{70em}\begin{matrix}x\\[95em]y\end{matrix}$$",
    ] {
        let d = rows_of(md, 390.0).remove(0);
        assert!(d.glyphs.is_empty(), "rendered as source: {md}");
        assert!(d.height < 2_000.0, "{md}: {}", d.height);
    }
    // Ordinary display math still renders.
    let d = rows_of(r"$$\sum_{i=1}^n i = \frac{n(n+1)}{2}$$", 390.0).remove(0);
    assert!(!d.glyphs.is_empty());
}

/// A table's rows share one scrolling canvas, so a tall formula in a cell
/// shows as source; the same formula in prose still renders. Long display
/// derivations narrower than the column render (only area is capped).
#[test]
fn table_cells_hold_formulas_short_and_long_derivations_render() {
    let tall = r"\begin{matrix}a\\b\\c\\d\\e\\f\\g\\h\\i\\j\\k\\l\\m\\n\end{matrix}";
    let cell = rows_of(&format!("| t | v |\n|---|---|\n| x | ${tall}$ |"), 390.0).remove(0);
    assert!(cell.glyphs.is_empty(), "tall cell formula shows as source");
    let header = rows_of(&format!("| ${tall}$ |\n|---|\n| x |"), 390.0).remove(0);
    assert!(header.glyphs.is_empty(), "header cells are held short too");
    // Ordinary cell math still renders.
    let vector = rows_of(r"| v |
|---|
| $\begin{pmatrix}1\\2\\3\\4\\5\end{pmatrix}$ |", 390.0).remove(0);
    assert!(!vector.glyphs.is_empty(), "a 5-row vector fits a cell");
    let prose = rows_of(&format!("so ${tall}$ here"), 390.0).remove(0);
    assert!(!prose.glyphs.is_empty(), "in prose it renders");
    let rows: String = (0..40).map(|i| format!("a_{{{i}}} &= \\frac{{b_{i}}}{{c_{i}}} \\\\ ")).collect();
    let d = rows_of(&format!("$$\\begin{{aligned}}{rows}\\end{{aligned}}$$"), 390.0).remove(0);
    assert!(!d.glyphs.is_empty(), "a 40-row derivation renders");
}

/// The formula cache stays within its bounds through many compactions:
/// every live entry keeps a use record, so eviction can always reach it.
#[test]
fn formula_cache_stays_bounded_through_compactions() {
    let mut w = worker(390.0);
    for round in 0..12 {
        let md: String = (0..1500).map(|i| format!("$x_{{{}}}$ ", round * 1500 + i)).collect();
        w.input = transcript_one(&md);
        w.pass();
        // Hit the newest ones again: uses pile up and force compactions.
        w.rebuild();
        w.pass();
    }
    let (live, uses, bytes) = w.math_failures.laid_stats();
    assert!(live <= 4096 && bytes <= 8 << 20, "{live} formulas, {bytes} bytes");
    assert!(uses >= live, "every live formula keeps a use: {uses} < {live}");
}

/// A row's formulas share one canvas budget: past it, the rest show as
/// source instead of each adding a full-size backing store.
#[test]
fn a_rows_formulas_share_one_budget() {
    let wide = r"$$\hspace{60em}\hspace{60em}\begin{matrix}x\\[30em]y\end{matrix}$$";
    let list: String = (0..40).map(|_| format!("- {wide}\n")).collect();
    let one = rows_of(&format!("- {wide}\n"), 390.0).remove(0);
    assert!(!one.glyphs.is_empty(), "one of them alone renders");
    let d = rows_of(&list, 390.0).remove(0);
    let rendered = d.glyphs.len() / one.glyphs.len();
    assert!(rendered < 40, "budget stops the row's formulas: {rendered}");
}

/// A header-only table leaves the cell limit behind it: the paragraph after
/// keeps the prose limit.
#[test]
fn a_header_only_table_leaves_prose_limits_alone() {
    let tall = r"\begin{matrix}a\\b\\c\\d\\e\\f\\g\\h\\i\\j\\k\\l\\m\\n\end{matrix}";
    let alone = rows_of(&format!("so ${tall}$ here"), 390.0).remove(0);
    assert!(!alone.glyphs.is_empty());
    let rows = rows_of(&format!("| h |\n|---|\n\nso ${tall}$ here"), 390.0);
    let after = rows.last().unwrap();
    assert_eq!(after.glyphs.len(), alone.glyphs.len(), "the paragraph after renders as on its own");
}

/// A table's canvas is all its columns by all its rows: formulas that widen
/// columns and raise rows multiply. Past its budget the table sets its
/// formulas as source, as a text table; a small math table still renders.
#[test]
fn math_tables_keep_a_bounded_canvas() {
    let wide = r"$\hspace{35em}x$";
    let tall = r"$\begin{matrix}a\\[7.5em]b\end{matrix}$";
    let header = format!("|{}|\n|{}|\n", [wide; 10].join("|"), ["---"; 10].join("|"));
    let body: String = (0..28).map(|_| format!("|{}|\n", [tall; 10].join("|"))).collect();
    let d = rows_of(&format!("{header}{body}"), 390.0).remove(0);
    let canvas: f32 = d.scrollers.iter().map(|s| s.content_width * s.h).sum::<f32>().max(390.0 * d.height);
    assert!(canvas < 8e6, "table canvas {canvas} pt²");
    let small = rows_of("| a | b |\n|---|---|\n| $x^2$ | $\\frac{1}{2}$ |", 390.0).remove(0);
    assert!(!small.glyphs.is_empty(), "a small math table renders its formulas");
}

#[test]
fn errors_render_as_cards_with_a_log_in_button_when_signing_in_fixes_them() {
    use zeron_doc::{MessagePart, MessageRole, MessageStatus, SessionMessageEntry};
    let error = |id: &str, message: &str| {
        Arc::new(SessionMessageEntry {
            id: id.into(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::Error { id: "e".into(), message: message.into() }],
            created_at: 0,
            device_id: String::new(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            duration_ms: None,
            answers_request_id: Default::default(),
            fork_import: Default::default(),
            from_chat_id: Default::default(),
        })
    };
    let mut w = worker(390.0);
    let mut input = debug_input(Vec::new(), false);
    input.entries = vec![
        error("signed-out", "Claude Code isn't signed in on Cloud. Sign in with Claude, then send again."),
        error("limited", "Claude usage limit reached — try again after the limit resets."),
        error("other", "Claude is overloaded right now — try again shortly."),
    ];
    w.input = input;
    let frame = w.pass();
    for (i, wants_button) in [(0, true), (1, true), (2, false)] {
        let d = frame.display(i).unwrap();
        // The card: a red wash under lighter red text.
        assert!(d.boxes.iter().any(|b| b.color == display::ColorRole::DangerWash), "row {i}");
        assert!(d.runs.iter().all(|r| r.color == display::ColorRole::DangerSoft), "row {i}");
        let actions: Vec<_> = d
            .widgets
            .iter()
            .filter_map(|w| match &w.kind {
                display::WidgetKind::Action { action, label } => Some((action.as_str(), label.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(actions.is_empty(), !wants_button, "row {i}: {actions:?}");
        if wants_button {
            assert_eq!(actions, [("sign-in", "Log in")]);
            // The button trails the text, never under it.
            let button = d.widgets.iter().find(|w| matches!(w.kind, display::WidgetKind::Action { .. })).unwrap();
            let text_right = d.runs.iter().map(|r| r.x + r.width).fold(0.0, f32::max);
            assert!(button.x > text_right, "row {i}: button at {} beside text ending {text_right}", button.x);
        }
    }
}

#[test]
fn page_rows_sit_outside_the_fold_and_take_the_height_their_page_reports() {
    struct Ping(Mutex<mpsc::Sender<u64>>);
    impl LayoutListener for Ping {
        fn frame_ready(&self, revision: u64) {
            let _ = self.0.lock().unwrap().send(revision);
        }
    }
    let (tx, frames) = mpsc::channel();
    let view = TranscriptView::new(text_system(), Arc::new(Ping(Mutex::new(tx))));
    let tool = MessagePart::Tool {
        id: "h1".into(),
        call: zeron_proto::ToolCall::Mcp { server: "zeron".into(), tool: "html_render".into(), input: None },
        is_error: false,
        resolved: true,
        output: None,
        diff: None,
        output_ref: None,
        output_bytes: None,
        diff_ref: None,
        diff_stats: None,
        subagent_ref: None,
        subagent_status: None,
        subagent_tail: None,
        html_render: Some(zeron_proto::html_render::HtmlRenderRef { path: "html/a1/page.html".into(), title: "Mockup".into(), height: 600 }),
        delegate_route: None,
    };
    view.set_viewport(390.0, 1.0);
    view.set_input(TranscriptInput {
        entries: vec![Arc::new(SessionMessageEntry {
            id: "a1".into(),
            role: MessageRole::Assistant,
            parts: vec![tool, MessagePart::Text { id: "t1".into(), text: "Here it is.".into() }],
            created_at: 0,
            device_id: "dev-mac".into(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            duration_ms: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
        })],
        ..Default::default()
    });
    let page_frame = |height: f32| {
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let _ = frames.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("a frame");
            let frame = view.frame();
            let kinds: Vec<RowKind> = (0..frame.row_count()).filter_map(|i| frame.placement(i)).map(|p| p.kind).collect();
            assert_eq!(kinds, [RowKind::Tools, RowKind::Html, RowKind::Markdown]);
            let row = frame.placement(1).unwrap();
            let display = frame.display(1).unwrap();
            let widget = display.widgets.iter().find_map(|w| match &w.kind {
                display::WidgetKind::HtmlRender { id, device_id, path, scrolls } => Some((w.clone(), id.clone(), device_id.clone(), path.clone(), *scrolls)),
                _ => None,
            });
            let (w, id, device, path, scrolls) = widget.expect("a page widget");
            if (w.h - height).abs() < 0.5 {
                assert!((w.y + w.h - row.height).abs() < 0.5, "the page fills the row below its title");
                assert_eq!((device.as_str(), path.as_str()), ("dev-mac", "html/a1/page.html"));
                break (id, scrolls);
            }
        }
    };
    let (id, scrolls) = page_frame(600.0);
    assert!(!scrolls);
    view.set_html_height(id.clone(), 420.0);
    assert_eq!(page_frame(420.0), (id.clone(), false), "a shorter page shrinks its frame");
    view.set_html_height(id.clone(), 900.0);
    assert_eq!(page_frame(600.0), (id.clone(), true), "an overflowing page keeps the agent's height and scrolls");
    view.set_html_height(id.clone(), 300.0);
    assert_eq!(page_frame(600.0), (id, false), "once capped it stays at the agent's height");
    view.close();
}
