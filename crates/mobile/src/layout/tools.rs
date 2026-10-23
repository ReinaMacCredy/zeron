//! Tool groups, laid out like the desktop transcript (`ui/src/transcript.rs`):
//! a 26pt header (chevron on the rail's trunk, summary title), then one 32pt
//! row per call — rail elbow, tool icon, verb, detail or file badge — each
//! with an inline detail (invocation, output / diff / stats, thought text).
//! Subagent spawns render as their own card group. Desktop metrics scale by
//! [`T`] so 12pt desktop text lands on the phone's 13.5pt small size.

use std::borrow::Cow;
use std::sync::Arc;

use zeron_doc::parts::{MessagePart, SubagentStatus};
use zeron_markdown::parser::{Block, BlockTree, IncrementalParser, InlineRun, InlineStyle};
use zeron_proto::computer::presentation::{
    computer_action_label, computer_call, computer_call_app, computer_call_js_actions, computer_call_rows, computer_group_segment,
    computer_js_action_label, computer_output_hidden,
};
use zeron_proto::device_tools::device_call;
use zeron_proto::{ChildRoute, DelegationStatus, HarnessId, ToolCall};
use zeron_text::WhiteSpace;

use super::display::{ColorRole, Decoration, DisplayBuilder, FadeEdge, WidgetKind};
use super::file_icons::{basename, file_icon_asset};
use super::markdown::{Ctx, PText, Px, SpanPaint, place_text, prepare_plain};
use super::rows::{Blob, Content, RowBuilder, RowCore, RowKind, next_version, place_text_lines, quick_hash, row_key};
use super::seams::{ChatRef, Chats, brand_icon, model_label, route_badge, route_detail, single_line};
use super::strip::{blob_link, chat_link, limit_link, linked, skill_link};
use super::style::{Family, Weight, baseline};

/// Desktop → phone scale (12pt tool text → 13.5pt).
const T: f32 = 1.125;
const HEADER_H: f32 = 26.0;
const TOP_PAD: f32 = 2.0;
const ROW_H: f32 = 32.0;
const TRUNK_X: f32 = 12.5;
const BEND: f32 = 6.0;
const BRANCH_END: f32 = 28.0;
const ICON_LEFT: f32 = 32.0;
const ICON: f32 = 16.0;
const TEXT_X: f32 = 56.0;
const TITLE_X: f32 = 28.0;
const OUT_LH: f32 = 18.0;
const BODY_PAD: f32 = 6.0;
const SEPARATOR: f32 = 1.0;
const MAX_LINES: usize = 24;
const FULL_OUTPUT_MAX_LINES: usize = 400;
const DIFF_MAX_LINES: usize = 600;
const CALL_WRAP_COLS: usize = 80;
const AGENT_ROW: f32 = 38.0;
const AGENT_CARD: f32 = 30.0;
const AGENT_SUB: f32 = 16.0;

fn d(px: Px, v: f32) -> f32 {
    px.v(v * T)
}

pub(crate) struct ToolLine {
    pub icon: String,
    pub label: PText,
    pub detail: Option<PText>,
    /// File calls show a badge: (file-icon asset, basename).
    pub badge: Option<(String, PText)>,
    /// A file change's `+N −M` after the badge.
    pub stats: Option<(PText, PText)>,
    pub failed: bool,
    pub running: bool,
    pub key: u64,
    pub open: bool,
    pub body: Vec<DetailBlock>,
    pub agent: Option<AgentExtra>,
    pub link: Option<String>,
}

pub(crate) struct AgentExtra {
    pub trailing: Option<PText>,
    pub meta: Option<PText>,
    pub route: Option<RouteBadge>,
    pub open: Option<String>,
}

pub(crate) struct RouteBadge {
    pub label: PText,
    pub wash: ColorRole,
    pub title: String,
    pub detail: String,
}

pub(crate) struct ToolGroup {
    pub summary: PText,
    pub lines: Vec<ToolLine>,
    pub expanded: bool,
    /// Auto-open live tail (drives the title shimmer).
    pub live: bool,
    /// Subagent spawns: bordered cards, no header, no rail.
    pub agents: bool,
}

pub(crate) enum DetailBlock {
    /// Mono lines (invocation / output), single-line each, scrolling sideways.
    Lines { lines: Vec<PText>, more: Option<PText> },
    /// Thought markdown flattened to styled lines, wrapped at width.
    Thought(Arc<ThoughtBody>),
    Stats(Vec<StatRow>),
    Diff { rows: Vec<DiffRowP>, notice: Option<PText>, digits: usize },
    Fetch(PText),
}

pub(crate) struct StatRow {
    pub icon: String,
    pub path: PText,
    pub add: PText,
    pub del: PText,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffKind {
    Hunk,
    Add,
    Del,
    Ctx,
}

pub(crate) struct DiffRowP {
    pub kind: DiffKind,
    pub old: Option<PText>,
    pub new: Option<PText>,
    pub marker: Option<PText>,
    pub text: PText,
}

// MARK: - Summary / invocation / detail (desktop ports)

/// Desktop summary: thought segment first, then the shared tool summary.
fn group_summary(thoughts: usize, calls: &[(ToolCall, bool)], computer_apps: &[&str], advisor_out: bool) -> String {
    let mut segments: Vec<String> = Vec::new();
    match thoughts {
        0 => {}
        1 => segments.push("thought process".into()),
        n => segments.push(format!("thought {n} times")),
    }
    let computer_actions = calls.iter().filter(|(call, _)| computer_call(call).is_some()).count();
    if computer_actions > 0 {
        segments.push(computer_group_segment(computer_apps, computer_actions));
    }
    let calls: Vec<(ToolCall, bool)> = calls.iter().filter(|(call, _)| computer_call(call).is_none()).cloned().collect();
    let devices = calls.iter().filter(|(call, _)| device_call(call).is_some()).count();
    if devices > 0 {
        segments.push(zeron_proto::device_tools::device_group_label(devices));
    }
    let calls: Vec<(ToolCall, bool)> = calls.iter().filter(|(call, _)| device_call(call).is_none()).cloned().collect();
    if !calls.is_empty() {
        let mut s = zeron_proto::view::tool_group_summary(&calls);
        if advisor_out {
            s = zeron_proto::transcript::advisor_out(&s);
        }
        let mut c = s.chars();
        let lowered = c.next().map(|f| f.to_lowercase().collect::<String>() + c.as_str()).unwrap_or_default();
        segments.push(lowered);
    }
    let mut out = segments.join(" · ");
    if let Some(first) = out.get(0..1) {
        let up = first.to_uppercase();
        out.replace_range(0..1, &up);
    }
    out
}

fn computer_app_of(part: &MessagePart) -> Option<String> {
    match part {
        MessagePart::Tool { call, output, .. } => computer_call(call).and_then(|c| computer_call_app(c.tool, c.args, output.as_deref())),
        _ => None,
    }
}

fn computer_apps<'a>(parts: impl Iterator<Item = &'a MessagePart>) -> Vec<String> {
    let mut apps: Vec<String> = Vec::new();
    for app in parts.filter_map(computer_app_of) {
        if !apps.contains(&app) {
            apps.push(app);
        }
    }
    apps
}

fn tool_icon(call: &ToolCall) -> &'static str {
    match call {
        ToolCall::Exec { .. } => "tool-terminal",
        ToolCall::ReadFile { .. } | ToolCall::ApplyPatch { .. } => "tool-document",
        ToolCall::WriteFile { .. } => "tool-document-add",
        ToolCall::EditFile { .. } => "tool-pen",
        ToolCall::Search { .. } => "tool-magnifer",
        ToolCall::Glob { .. } => "tool-folder-with-files",
        ToolCall::WebFetch { .. } | ToolCall::WebSearch { .. } => "tool-global",
        ToolCall::Todo { .. } => "tool-checklist",
        ToolCall::Unknown { name, .. } if name == "Agent" || name.starts_with("Agent: ") || name.eq_ignore_ascii_case("wait for agents") => "tool-bot",
        ToolCall::Mcp { .. } | ToolCall::Unknown { .. } => "tool-widget",
    }
}

fn spawn_tag(harness: Option<HarnessId>, call: &ToolCall) -> Option<String> {
    if let Some(model) = call.subagent_model() {
        return Some(model_label(harness, model));
    }
    let (ToolCall::Unknown { input, .. } | ToolCall::Mcp { input, .. }) = call else {
        return None;
    };
    input
        .as_ref()?
        .get("subagent_type")?
        .as_str()
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .map(str::to_owned)
}

fn file_path(call: &ToolCall) -> Option<&str> {
    match call {
        ToolCall::ReadFile { path } | ToolCall::WriteFile { path, .. } | ToolCall::EditFile { path, .. } => Some(path),
        ToolCall::ApplyPatch { path } => path.as_deref(),
        _ => None,
    }
}

fn wrap_cols(line: &str, cols: usize) -> Vec<String> {
    if line.chars().count() <= cols {
        return vec![line.to_owned()];
    }
    line.chars().collect::<Vec<_>>().chunks(cols).map(|c| c.iter().collect()).collect()
}

/// The full invocation the header truncates (desktop `call_block`).
fn call_text(call: &ToolCall) -> String {
    match call {
        ToolCall::Exec { command } => command.clone(),
        ToolCall::ReadFile { path } => path.clone(),
        ToolCall::WriteFile { path, content } => match content {
            Some(c) => format!("{path}\n{c}"),
            None => path.clone(),
        },
        ToolCall::EditFile { path, .. } => path.clone(),
        ToolCall::ApplyPatch { path } => path.clone().unwrap_or_else(|| "workspace".into()),
        ToolCall::Search { pattern, path } => match path {
            Some(p) => format!("{pattern} in {p}"),
            None => pattern.clone(),
        },
        ToolCall::Glob { pattern } => pattern.clone(),
        ToolCall::WebFetch { url, prompt } => match prompt {
            Some(p) => format!("{url}\n{p}"),
            None => url.clone(),
        },
        ToolCall::WebSearch { query } => query.clone(),
        ToolCall::Todo { items } => items
            .iter()
            .map(|i| format!("{} {}", if i.done { "[x]" } else { "[ ]" }, i.text))
            .collect::<Vec<_>>()
            .join("\n"),
        ToolCall::Mcp { server, tool, input } => match input {
            Some(i) => format!("{server} · {tool}\n{}", serde_json::to_string_pretty(i).unwrap_or_default()),
            None => format!("{server} · {tool}"),
        },
        ToolCall::Unknown { name, input } => match input {
            Some(i) => format!("{name}\n{}", serde_json::to_string_pretty(i).unwrap_or_default()),
            None => name.clone(),
        },
    }
}

struct Styles {
    label: super::style::Resolved,
    mono: super::style::Resolved,
    mono_small: super::style::Resolved,
    /// Detail type size in points (pre-text-scale), for thought run faces.
    size: f32,
    lh: f32,
    out_lh: f32,
}

fn lines_block(ctx: &mut Ctx, st: &Styles, text: &str, wrap: Option<usize>) -> Option<DetailBlock> {
    capped_lines(ctx, st, text, wrap, MAX_LINES)
}

fn capped_lines(ctx: &mut Ctx, st: &Styles, text: &str, wrap: Option<usize>, max: usize) -> Option<DetailBlock> {
    let mut lines: Vec<String> = text
        .lines()
        .flat_map(|l| match wrap {
            Some(cols) => wrap_cols(l, cols),
            None => vec![l.to_owned()],
        })
        .collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        return None;
    }
    let extra = lines.len().saturating_sub(max);
    lines.truncate(max);
    let lines = lines
        .iter()
        .map(|l| prepare_plain(ctx, l, st.mono, st.out_lh, ColorRole::TextFaint, WhiteSpace::Pre))
        .collect();
    let more = (extra > 0).then(|| prepare_plain(ctx, &format!("… {extra} more lines"), st.label, st.out_lh, ColorRole::TextFaint, WhiteSpace::Pre));
    Some(DetailBlock::Lines { lines, more })
}

fn diff_block(ctx: &mut Ctx, st: &Styles, diff: &zeron_proto::ToolDiff) -> Option<DetailBlock> {
    let old = diff.old_text.as_deref().unwrap_or("");
    // Bounded: a huge rewrite falls back to a coarser diff instead of
    // stalling the layout worker (the transcript would stop updating).
    let text_diff = similar::TextDiff::configure()
        .timeout(std::time::Duration::from_millis(100))
        .diff_lines(old, &diff.new_text);
    let mut raw: Vec<(DiffKind, Option<u32>, Option<u32>, String)> = Vec::new();
    for group in text_diff.grouped_ops(3) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else { continue };
        let o = first.old_range().start..last.old_range().end;
        let n = first.new_range().start..last.new_range().end;
        raw.push((DiffKind::Hunk, None, None, format!("@@ -{},{} +{},{} @@", o.start + 1, o.len(), n.start + 1, n.len())));
        for op in &group {
            for change in text_diff.iter_changes(op) {
                let kind = match change.tag() {
                    similar::ChangeTag::Delete => DiffKind::Del,
                    similar::ChangeTag::Insert => DiffKind::Add,
                    similar::ChangeTag::Equal => DiffKind::Ctx,
                };
                raw.push((
                    kind,
                    change.old_index().map(|i| i as u32 + 1),
                    change.new_index().map(|i| i as u32 + 1),
                    change.value().trim_end_matches('\n').to_owned(),
                ));
            }
        }
    }
    if raw.is_empty() {
        return None;
    }
    let total = raw.iter().filter(|r| r.0 != DiffKind::Hunk).count();
    let mut notice = if diff.old_text.is_none() { Some("New file".to_owned()) } else { None };
    if total > DIFF_MAX_LINES {
        notice = Some(format!("Diff truncated — showing first {DIFF_MAX_LINES} of {total} lines"));
        let mut kept = 0;
        raw.retain(|r| {
            if r.0 != DiffKind::Hunk {
                kept += 1;
            }
            kept <= DIFF_MAX_LINES
        });
    }
    let max_no = raw.iter().map(|r| r.1.unwrap_or(0).max(r.2.unwrap_or(0))).max().unwrap_or(0);
    let digits = max_no.max(1).to_string().len();
    let rows = raw
        .into_iter()
        .map(|(kind, o, n, text)| {
            let num = |ctx: &mut Ctx, v: Option<u32>, color| v.map(|v| prepare_plain(ctx, &v.to_string(), st.mono_small, st.out_lh, color, WhiteSpace::Pre));
            let (gutter_color, marker) = match kind {
                DiffKind::Add => (ColorRole::Success, Some(("+", ColorRole::Success))),
                DiffKind::Del => (ColorRole::Danger, Some(("−", ColorRole::Danger))),
                DiffKind::Ctx => (ColorRole::TextFaint, Some(("·", ColorRole::TextFaint))),
                DiffKind::Hunk => (ColorRole::TextFaint, None),
            };
            DiffRowP {
                kind,
                old: num(ctx, o, gutter_color),
                new: num(ctx, n, gutter_color),
                marker: marker.map(|(m, c)| prepare_plain(ctx, m, st.mono, st.out_lh, c, WhiteSpace::Pre)),
                text: if kind == DiffKind::Hunk {
                    prepare_plain(ctx, &text, st.mono_small, st.out_lh, ColorRole::TextFaint, WhiteSpace::Pre)
                } else {
                    prepare_plain(ctx, &text, st.mono, st.out_lh, ColorRole::Text, WhiteSpace::Pre)
                },
            }
        })
        .collect();
    let notice = notice.map(|n| prepare_plain(ctx, &n, st.label, st.out_lh, ColorRole::TextFaint, WhiteSpace::Pre));
    Some(DetailBlock::Diff { rows, notice, digits })
}

/// Added and deleted line totals of a file change (desktop `ToolDetail::line_stats`).
fn line_stats(part: &MessagePart) -> Option<(u64, u64)> {
    let MessagePart::Tool { diff, diff_stats, .. } = part else { return None };
    if let Some(stat) = diff.as_ref().map(zeron_doc::parts::diff_stat).filter(|s| s.additions + s.deletions > 0) {
        return Some((stat.additions, stat.deletions));
    }
    let stats = diff_stats.as_ref().filter(|s| !s.is_empty())?;
    Some(stats.iter().fold((0, 0), |(add, del), stat| (add + stat.additions, del + stat.deletions)))
}

/// Result detail (desktop `tool_detail`): a diff wins, then stats, then output.
fn result_block(ctx: &mut Ctx, st: &Styles, part: &MessagePart) -> Option<DetailBlock> {
    let MessagePart::Tool { output, diff, diff_stats, .. } = part else { return None };
    if let Some(diff) = diff {
        if let Some(b) = diff_block(ctx, st, diff) {
            return Some(b);
        }
    }
    if let Some(stats) = diff_stats.as_ref().filter(|s| !s.is_empty()) {
        return Some(DetailBlock::Stats(
            stats
                .iter()
                .map(|s| StatRow {
                    icon: file_icon_asset(&s.path),
                    path: prepare_plain(ctx, &s.path, st.label, st.out_lh, ColorRole::TextFaint, WhiteSpace::Pre),
                    add: prepare_plain(ctx, &format!("+{}", s.additions), st.label, st.out_lh, ColorRole::Success, WhiteSpace::Pre),
                    del: prepare_plain(ctx, &format!("−{}", s.deletions), st.label, st.out_lh, ColorRole::Danger, WhiteSpace::Pre),
                })
                .collect(),
        ));
    }
    lines_block(ctx, st, output.as_deref()?, None)
}

pub(crate) fn blob_refs(own: Option<&str>, part: &MessagePart) -> [Option<String>; 2] {
    let MessagePart::Tool { diff, diff_ref, diff_stats, output_ref, .. } = part else { return [None, None] };
    let stats_only = diff.is_none() && diff_stats.as_ref().is_some_and(|s| !s.is_empty());
    let diff_ref = diff_ref.clone().or_else(|| own.filter(|_| stats_only).map(|chat| format!("{chat}/{}.diff", part.id())));
    [diff_ref, output_ref.clone()]
}

// MARK: - Thoughts (desktop `thought_lines`, engine-width wrapping)

/// Flatten a parsed thought into styled logical lines — the desktop's
/// `thought_lines`: inline markers become real styling, blocks flatten
/// structurally (headings bold, list markers, quote bars, verbatim code
/// lines, tables as `·`-joined rows). Desktop wraps at a char budget to keep
/// its detail height analytic; here the text engine wraps each line at the
/// painted width beside its gutter, so heights stay exact anyway.
///
/// Every logical line fills at least one visual line, so only the first `cap`
/// can ever show: flattening stops there. Those lines are also clipped to
/// [`THOUGHT_BYTES`], so one huge paragraph streaming in doesn't re-shape its
/// whole text per delta either: a long thought costs the same per streamed
/// delta as a short one. `true` = more content follows what's kept.
fn thought_lines(tree: &BlockTree, cap: Option<usize>) -> (Vec<Vec<InlineRun>>, bool) {
    let mut out: Vec<Vec<InlineRun>> = Vec::new();
    for top in &tree.blocks {
        if !out.is_empty() {
            // One blank separator line between top-level blocks.
            out.push(Vec::new());
        }
        thought_block_lines(&top.block, 0, &mut out);
        if cap.is_some_and(|cap| out.len() > cap) {
            break;
        }
    }
    let mut more = false;
    if let Some(cap) = cap {
        more = out.len() > cap;
        out.truncate(cap);
        more |= clip_bytes(&mut out, THOUGHT_BYTES);
    }
    while out.last().is_some_and(|l| l.iter().all(|r| r.text.trim().is_empty())) {
        out.pop();
    }
    (out, more)
}

/// Text budget for a thought's visible lines: [`MAX_LINES`] wrapped lines of
/// real text hold well under this even at the widest iPad column, so the clip
/// only drops text that's past the fade anyway.
const THOUGHT_BYTES: usize = MAX_LINES * 512;

/// Keep at most `budget` bytes of run text (cut at a char boundary); `true`
/// when anything was dropped.
fn clip_bytes(lines: &mut Vec<Vec<InlineRun>>, budget: usize) -> bool {
    let mut left = budget;
    for li in 0..lines.len() {
        for ri in 0..lines[li].len() {
            let run = &mut lines[li][ri];
            if run.text.len() <= left {
                left -= run.text.len();
                continue;
            }
            let mut cut = left;
            while !run.text.is_char_boundary(cut) {
                cut -= 1;
            }
            run.text.truncate(cut);
            lines[li].truncate(ri + 1);
            lines.truncate(li + 1);
            return true;
        }
    }
    false
}

/// The indent run every line opens with; list/quote handlers rewrite it to
/// plant markers/bars, so it exists even at zero indent.
fn indent_run(indent: usize) -> Vec<InlineRun> {
    vec![InlineRun { text: " ".repeat(indent), style: InlineStyle::default() }]
}

/// Append text to a line, merging into the tail run when styles match.
fn push_styled(line: &mut Vec<InlineRun>, text: &str, style: &InlineStyle) {
    if text.is_empty() {
        return;
    }
    match line.last_mut() {
        Some(last) if last.style == *style => last.text.push_str(text),
        _ => line.push(InlineRun { text: text.to_owned(), style: style.clone() }),
    }
}

/// Close a line: the indent run in front (see [`indent_run`]).
fn finish_line(indent: usize, mut line: Vec<InlineRun>) -> Vec<InlineRun> {
    let mut full = indent_run(indent);
    full.append(&mut line);
    full
}

/// One segment (between hard `\n`s) into an output line; whitespace-only
/// segments are dropped, exactly as desktop's token wrap drops them.
fn flush_segment(indent: usize, line: &mut Vec<InlineRun>, out: &mut Vec<Vec<InlineRun>>) {
    if line.iter().any(|r| !r.text.trim().is_empty()) {
        out.push(finish_line(indent, std::mem::take(line)));
    } else {
        line.clear();
    }
}

/// Runs into logical lines: hard `\n`s break, wrapping is the engine's.
fn push_runs(runs: &[InlineRun], indent: usize, out: &mut Vec<Vec<InlineRun>>) {
    let mut line: Vec<InlineRun> = Vec::new();
    for run in runs {
        for (ix, piece) in run.text.split('\n').enumerate() {
            if ix > 0 {
                flush_segment(indent, &mut line, out);
            }
            if !piece.is_empty() {
                push_styled(&mut line, piece, &run.style);
            }
        }
    }
    flush_segment(indent, &mut line, out);
}

/// One markdown block into thought detail lines, `indent` spaces deep.
fn thought_block_lines(block: &Block, indent: usize, out: &mut Vec<Vec<InlineRun>>) {
    match block {
        Block::Paragraph { runs } => push_runs(runs, indent, out),
        Block::Heading { runs, .. } => {
            // Headings keep the detail's single type size — bold is the cue.
            let bold: Vec<InlineRun> = runs
                .iter()
                .map(|r| {
                    let mut r = r.clone();
                    r.style.bold = true;
                    r
                })
                .collect();
            push_runs(&bold, indent, out);
        }
        // Thoughts stay source text: display math reads like a code block.
        Block::CodeBlock { code, .. } | Block::Math { tex: code } => {
            let style = InlineStyle { code: true, ..InlineStyle::default() };
            for line in code.lines() {
                let mut row = indent_run(indent);
                if !line.is_empty() {
                    row.push(InlineRun { text: line.to_owned(), style: style.clone() });
                }
                out.push(row);
            }
        }
        Block::List { ordered_start, items } => {
            // Tight rendering: no blank lines inside a list.
            for (ix, item) in items.iter().enumerate() {
                let marker = match ordered_start {
                    Some(start) => format!("{}. ", start + ix as u64),
                    None => "• ".to_string(),
                };
                let inner = indent + marker.chars().count();
                let mark = out.len();
                for child in item {
                    thought_block_lines(child, inner, out);
                }
                if out.len() == mark {
                    // An empty item still shows its marker.
                    out.push(indent_run(inner));
                }
                // The item's first line trades its indent spaces for the marker.
                if let Some(first) = out[mark].first_mut() {
                    first.text = format!("{}{marker}", " ".repeat(indent));
                }
            }
        }
        Block::BlockQuote { children } => {
            let mark = out.len();
            for (ix, child) in children.iter().enumerate() {
                if ix > 0 {
                    out.push(Vec::new());
                }
                thought_block_lines(child, indent + 2, out);
            }
            // Trade the two quote-indent spaces for the bar on every quoted
            // line — nested list markers sit after their own deeper indent.
            for line in &mut out[mark..] {
                if let Some(first) = line.first_mut()
                    && first.text.len() >= indent + 2
                {
                    first.text.replace_range(indent..indent + 2, "│ ");
                }
            }
        }
        Block::Table { header, rows, .. } => {
            // A thought is a record, not a layout surface: cells joined with
            // a dot separator, header bold — no column machinery.
            let join = |cells: &[Vec<InlineRun>], bold: bool| -> Vec<InlineRun> {
                let mut line: Vec<InlineRun> = Vec::new();
                for (ix, cell) in cells.iter().enumerate() {
                    if ix > 0 {
                        push_styled(&mut line, " · ", &InlineStyle::default());
                    }
                    for r in cell {
                        let mut r = r.clone();
                        r.style.bold |= bold;
                        push_styled(&mut line, &r.text, &r.style);
                    }
                }
                line
            };
            push_runs(&join(header, true), indent, out);
            for row in rows {
                push_runs(&join(row, false), indent, out);
            }
        }
        Block::Rule => {
            let mut row = indent_run(indent);
            row.push(InlineRun { text: "———".into(), style: InlineStyle::default() });
            out.push(row);
        }
    }
}

/// One flattened thought line, split at its slot-0 run: the gutter (indent,
/// list marker, quote bars) and the body the text engine wraps beside it. A
/// wrapped body hangs under its own first word and keeps its quote bars —
/// what desktop's re-indented char wrap draws.
pub(crate) struct ThoughtLine {
    /// Gutter width: the body's x offset on every visual line.
    indent: f32,
    /// The gutter on the first visual line (None when it's only spaces).
    gutter: Option<PText>,
    /// Quote bars repeated on wrapped continuation lines.
    bars: Option<PText>,
    /// None for a blank line (still one line box).
    body: Option<PText>,
}

/// A thought's prepared detail: at most [`MAX_LINES`] logical lines (only
/// those can show); `more` = content past them.
pub(crate) struct ThoughtBody {
    lines: Vec<ThoughtLine>,
    more: bool,
    cap: Option<usize>,
    lh: f32,
}

/// One reasoning part's parse and prepared detail. The parse is incremental
/// like text parts' (the mended display tree while the part is the streaming
/// tail, the canonical tree once settled); the prepared body is reused while
/// its visible lines are unchanged.
#[derive(Default)]
pub(crate) struct ThoughtState {
    parser: IncrementalParser,
    /// (len, hash, live) of the last source fed to the parser.
    source: Option<(usize, u64, bool)>,
    lines: Vec<Vec<InlineRun>>,
    body: Option<Arc<ThoughtBody>>,
}

/// Flattened thought lines at the detail's type size — desktop's
/// `thought_line_text`: faint prose, semibold bold, mono code, underlined
/// links (NOT clickable — a thought is a record, not a surface).
fn prepare_thought(ctx: &mut Ctx, st: &Styles, lines: &[Vec<InlineRun>], more: bool, cap: Option<usize>) -> ThoughtBody {
    let size = st.size;
    let semi = ctx.typo.style(Family::Sans, Weight::Semibold, false, size);
    let italic = ctx.typo.style(Family::Sans, Weight::Regular, true, size);
    let semi_italic = ctx.typo.style(Family::Sans, Weight::Semibold, true, size);
    let mono_italic = ctx.typo.style(Family::Mono, Weight::Regular, true, size);
    let mut prepared = Vec::with_capacity(lines.len());
    for line in lines {
        let Some((head, runs)) = line.split_first() else {
            prepared.push(ThoughtLine { indent: 0.0, gutter: None, bars: None, body: None });
            continue;
        };
        // `Pre`: the gutter's trailing spaces are content, so they count.
        let gutter = (!head.text.is_empty()).then(|| prepare_plain(ctx, &head.text, st.label, st.out_lh, ColorRole::TextFaint, WhiteSpace::Pre));
        let indent = gutter.as_ref().map_or(0.0, |g| g.p.max_content_width());
        let gutter = gutter.filter(|_| !head.text.trim().is_empty());
        let bars = head.text.contains('│').then(|| {
            let bars: String = head.text.chars().map(|c| if c == '│' { c } else { ' ' }).collect();
            prepare_plain(ctx, &bars, st.label, st.out_lh, ColorRole::TextFaint, WhiteSpace::Pre)
        });
        if runs.iter().all(|r| r.text.trim().is_empty()) {
            prepared.push(ThoughtLine { indent, gutter, bars, body: None });
            continue;
        }
        let mut text = String::new();
        let mut spans: Vec<zeron_text::Span> = Vec::with_capacity(runs.len());
        let mut paints = Vec::with_capacity(runs.len());
        for run in runs {
            if run.text.is_empty() {
                continue;
            }
            let s = &run.style;
            // Inline math, too, shows its source: `$…$` in the code face.
            let style = if s.code || s.math {
                if s.italic { mono_italic } else { st.mono }
            } else if s.bold {
                if s.italic { semi_italic } else { semi }
            } else if s.italic {
                italic
            } else {
                st.label
            };
            let start = text.len();
            if s.math {
                text.push('$');
                text.push_str(&run.text);
                text.push('$');
            } else {
                text.push_str(&run.text);
            }
            spans.push(zeron_text::Span {
                range: start..text.len(),
                style: style.id,
                pad_start: 0.0,
                pad_end: 0.0,
                atomic: false,
                inline_box: None,
            });
            paints.push(SpanPaint {
                color: ColorRole::TextFaint,
                decoration: if s.strikethrough {
                    Decoration::Strikethrough
                } else if s.link.is_some() {
                    Decoration::Underline
                } else {
                    Decoration::None
                },
                link: None,
                chip: false,
            });
        }
        // Code lines are verbatim (indentation, aligned spaces); prose
        // collapses whitespace like the transcript's markdown does.
        let verbatim = runs.iter().all(|r| r.style.code);
        let p = zeron_text::prepare(
            &ctx.typo.book,
            ctx.cache,
            &text,
            &spans,
            &zeron_text::PrepareOptions {
                white_space: if verbatim { WhiteSpace::PreWrap } else { WhiteSpace::PreLine },
                overflow_wrap: zeron_text::OverflowWrap::Anywhere,
                ..Default::default()
            },
        );
        let body = PText {
            p,
            lh: st.out_lh,
            base: baseline(st.out_lh, st.label),
            paints,
            links: Vec::new(),
            chip: (0.0, 0.0),
            badges: Vec::new(),
            math: Vec::new(),
        };
        prepared.push(ThoughtLine { indent, gutter, bars, body: Some(body) });
    }
    ThoughtBody { lines: prepared, more, cap, lh: st.out_lh }
}

/// Visual lines a thought fills at body width `bw`, capped while it streams;
/// `true` when content is cut (bottom fade).
fn thought_extent(t: &ThoughtBody, bw: f32) -> (usize, bool) {
    let mut n = 0usize;
    for line in &t.lines {
        n += line.visual_lines(bw);
        if let Some(cap) = t.cap.filter(|cap| n > *cap) {
            return (cap, true);
        }
    }
    (n, t.more)
}

impl ThoughtLine {
    fn visual_lines(&self, bw: f32) -> usize {
        self.body.as_ref().map_or(1, |b| b.p.line_count((bw - self.indent).max(1.0)).max(1))
    }
}

/// Paint a thought's lines from `by`: gutters, wrapped bodies beside them,
/// quote bars down wrapped lines; a streaming one cut with a bottom fade.
fn place_thought(t: &ThoughtBody, bx: f32, by: f32, bw: f32, o: &mut DisplayBuilder) {
    let (shown, cut) = thought_extent(t, bw);
    let runs_before = o.runs.len();
    let mut row = 0usize;
    for line in &t.lines {
        if row >= shown {
            break;
        }
        let y = by + row as f32 * t.lh;
        let n = line.visual_lines(bw);
        if let Some(g) = &line.gutter {
            place_text(g, bx, y, g.p.max_content_width() + 1.0, Some(o));
        }
        if let Some(bars) = &line.bars {
            for k in 1..n.min(shown - row) {
                place_text(bars, bx, y + k as f32 * t.lh, bars.p.max_content_width() + 1.0, Some(o));
            }
        }
        if let Some(body) = &line.body {
            place_text(body, bx + line.indent, y, (bw - line.indent).max(1.0), Some(o));
        }
        row += n;
    }
    if cut {
        // Drop the overflow of a wrapped line straddling the cap.
        let limit = by + shown as f32 * t.lh;
        let kept: Vec<_> = o.runs.drain(runs_before..).filter(|r| r.baseline < limit).collect();
        o.runs.extend(kept);
        o.fade(bx, limit - t.lh, bw, t.lh, FadeEdge::Bottom);
    }
}

// MARK: - Building

impl RowBuilder {
    fn full_result(&self, ctx: &mut Ctx, st: &Styles, part: &MessagePart) -> (Option<DetailBlock>, Option<DetailBlock>) {
        let [diff_ref, output_ref] = blob_refs(self.chat_id.as_deref(), part);
        let bytes = match part {
            MessagePart::Tool { output_bytes, .. } => *output_bytes,
            _ => None,
        };
        let candidates = [(diff_ref, "diff", None), (output_ref, "output", bytes)];
        let shown = candidates
            .iter()
            .filter_map(|(r, ..)| r.as_ref())
            .filter_map(|r| match self.blobs.get(r) {
                Some((Blob::Ready(text), order)) => Some((r, text.clone(), *order)),
                _ => None,
            })
            .max_by_key(|(.., order)| *order);
        let full = shown.as_ref().and_then(|(r, text, _)| {
            if r.ends_with(".diff") {
                serde_json::from_str::<zeron_proto::ToolDiff>(text).ok().and_then(|diff| diff_block(ctx, st, &diff))
            } else {
                capped_lines(ctx, st, text, None, FULL_OUTPUT_MAX_LINES)
            }
        });
        let offer = candidates.iter().find_map(|(r, what, bytes)| {
            let r = r.as_ref()?;
            let label = match self.blobs.get(r).map(|(blob, _)| blob) {
                Some(Blob::Ready(_)) if shown.as_ref().is_some_and(|(s, ..)| *s == r) => return None,
                Some(Blob::Ready(_)) => format!("Show full {what}"),
                Some(Blob::Loading) => format!("Loading full {what}…"),
                Some(Blob::Failed) => format!("Couldn't load full {what} — tap to retry"),
                None => match bytes {
                    Some(b) => format!("Show full {what} ({})", zeron_proto::transcript::format_kb(*b)),
                    None => format!("Show full {what}"),
                },
            };
            let text = prepare_plain(ctx, &label, st.label, st.out_lh, ColorRole::Link, WhiteSpace::Pre);
            Some(DetailBlock::Fetch(linked(text, blob_link(r))))
        });
        (full, offer)
    }

    /// A reasoning part's prepared detail. The parse advances incrementally
    /// per streamed delta; flattening stops at the visible cap; preparing
    /// (shaping) reruns only when the visible lines change.
    fn thought_body(&mut self, ctx: &mut Ctx, st: &Styles, key: &str, text: &str, live: bool) -> Arc<ThoughtBody> {
        let state = self.thoughts.entry(key.to_owned()).or_default();
        let source = (text.len(), quick_hash(text), live);
        if let (Some(body), Some(fed)) = (&state.body, state.source)
            && fed == source
        {
            return body.clone();
        }
        state.parser.set_text(text);
        state.source = Some(source);
        let tree = if live { state.parser.display_tree() } else { state.parser.tree().clone() };
        let cap = live.then_some(MAX_LINES);
        let (lines, more) = thought_lines(&tree, cap);
        if let Some(body) = &state.body
            && body.more == more
            && body.cap == cap
            && state.lines == lines
        {
            return body.clone();
        }
        let body = Arc::new(prepare_thought(ctx, st, &lines, more, cap));
        state.lines = lines;
        state.body = Some(body.clone());
        body
    }

    #[cfg(test)]
    pub(crate) fn thought_body_for_test(&self, key: &str) -> Option<Arc<ThoughtBody>> {
        self.thoughts.get(key).and_then(|s| s.body.clone())
    }

    pub(crate) fn work_header(&mut self, ctx: &mut Ctx, entry_id: &str, key: &str, parts: &[MessagePart], worked_secs: Option<i64>, open: bool) -> RowCore {
        let calls: Vec<(ToolCall, bool)> = parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Tool { call, is_error, .. } => Some((call.clone(), *is_error)),
                _ => None,
            })
            .collect();
        let thoughts = parts.iter().filter(|p| matches!(p, MessagePart::Reasoning { .. })).count();
        let text = match worked_secs {
            Some(secs) => format!("Worked for {}", zeron_proto::transcript::format_elapsed(secs)),
            None => {
                let apps = computer_apps(parts.iter());
                group_summary(thoughts, &calls, &apps.iter().map(String::as_str).collect::<Vec<_>>(), false)
            }
        };
        let size = 12.0 * T;
        let label = ctx.typo.style(Family::Sans, Weight::Regular, false, size);
        let lh = ctx.typo.px(18.0 * T);
        let summary = prepare_plain(ctx, &text, label, lh, ColorRole::TextSecondary, WhiteSpace::Pre);
        RowCore {
            key: row_key(key),
            version: next_version(),
            kind: RowKind::Tools,
            entry_id: Arc::from(entry_id),
            content: Content::Tools(ToolGroup { summary, lines: Vec::new(), expanded: open, live: false, agents: false }),
            copy_text: String::new(),
        }
    }

    /// One group of consecutive tool / thought parts (`{msg}#g{n}`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tool_row(
        &mut self,
        ctx: &mut Ctx,
        entry_id: &str,
        id: &str,
        parts: &[&MessagePart],
        live: bool,
        agents: bool,
        chats: &Chats,
        own: Option<&ChatRef>,
    ) -> RowCore {
        let key = row_key(id);
        let mut parts: Vec<&MessagePart> = parts.to_vec();
        if parts.iter().all(|p| matches!(p, MessagePart::Tool { call, .. } if computer_call(call).is_some())) {
            let mut apps: Vec<Option<String>> = Vec::new();
            for part in &parts {
                let app = computer_app_of(part);
                if !apps.contains(&app) {
                    apps.push(app);
                }
            }
            parts.sort_by_key(|part| apps.iter().position(|app| *app == computer_app_of(part)));
        }
        let computer_apps = computer_apps(parts.iter().copied());
        let computer_apps: Vec<&str> = computer_apps.iter().map(String::as_str).collect();
        let calls: Vec<(ToolCall, bool)> = parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Tool { call, is_error, .. } => Some((call.clone(), *is_error)),
                _ => None,
            })
            .collect();
        let thoughts = parts.iter().filter(|p| matches!(p, MessagePart::Reasoning { .. })).count();
        let expanded = agents
            || if self.expanded.contains(&key) {
                true
            } else if self.collapsed.contains(&key) {
                false
            } else {
                live // Auto-open while it's the live tail, like desktop.
            };
        let size = 12.0 * T;
        let st = Styles {
            label: ctx.typo.style(Family::Sans, Weight::Regular, false, size),
            mono: ctx.typo.style(Family::Mono, Weight::Regular, false, size),
            mono_small: ctx.typo.style(Family::Mono, Weight::Regular, false, 11.0 * T),
            size,
            lh: ctx.typo.px(18.0 * T),
            out_lh: ctx.typo.px(OUT_LH * T),
        };
        let medium = ctx.typo.style(Family::Sans, Weight::Medium, false, size);
        let small = ctx.typo.style(Family::Sans, Weight::Regular, false, 11.0 * T);
        let small_lh = ctx.typo.px(AGENT_SUB * T);
        let advisor_out = parts.iter().any(|p| matches!(p, MessagePart::Tool { call, resolved: false, .. } if call.is_advisor()));
        let summary = prepare_plain(ctx, &group_summary(thoughts, &calls, &computer_apps, advisor_out), st.label, st.lh, ColorRole::TextSecondary, WhiteSpace::Pre);
        let mut lines = Vec::new();
        if expanded {
            let last = parts.len().saturating_sub(1);
            for (i, part) in parts.iter().enumerate() {
                let dkey = row_key(&format!("{id}/{}", part.id()));
                match part {
                    MessagePart::Tool { call, subagent_ref, delegate_route, .. }
                        if agents && call.is_delegate_task() && (subagent_ref.is_some() || delegate_route.is_some()) =>
                    {
                        lines.push(delegate_line(ctx, &st, medium, call, subagent_ref.as_deref(), delegate_route.as_ref(), dkey, chats, own));
                    }
                    MessagePart::Tool { call, is_error, resolved, output, subagent_ref, subagent_status, .. } => {
                        let computer = computer_call(call);
                        let device = device_call(call);
                        let (label, detail): (Cow<str>, String) = match (&computer, device) {
                            (Some(c), _) => {
                                let app = computer_call_app(c.tool, c.args, output.as_deref());
                                (computer_action_label(c.tool, c.args, app.as_deref(), *resolved).into(), String::new())
                            }
                            (None, Some(device)) => (
                                zeron_proto::device_tools::device_call_label(device, *resolved, *is_error).into(),
                                zeron_proto::device_tools::device_call_detail(device, output.as_deref().unwrap_or("")).unwrap_or_default(),
                            ),
                            (None, None) => {
                                let (label, detail) = zeron_proto::view::tool_chip_content(call);
                                (label.into(), detail)
                            }
                        };
                        // Subagent lifecycle is distinct from `resolved`: under
                        // eager-done the spawn call resolves while the subagent
                        // still runs (desktop transcript.rs `running`/`failed`).
                        let (running, is_error) = if agents {
                            let spawned = subagent_ref.is_some();
                            (
                                spawned && matches!(subagent_status, Some(SubagentStatus::Running)) || !spawned && !*resolved,
                                &(*is_error || spawned && matches!(subagent_status, Some(SubagentStatus::Failed))),
                            )
                        } else {
                            (!*resolved, is_error)
                        };
                        let color = if *is_error { ColorRole::Danger } else { ColorRole::TextSecondary };
                        let badge = if agents {
                            None
                        } else {
                            file_path(call).map(|p| (file_icon_asset(p), prepare_plain(ctx, basename(p), st.label, st.lh, if *is_error { ColorRole::Danger } else { ColorRole::TextSoft }, WhiteSpace::Pre)))
                        };
                        let skill = call.skill_name().filter(|_| !agents && computer.is_none() && device.is_none());
                        let detail_color = if *is_error {
                            color
                        } else if skill.is_some() {
                            ColorRole::Link
                        } else if agents {
                            ColorRole::TextSoft
                        } else {
                            color
                        };
                        let detail = (badge.is_none() && !detail.is_empty()).then(|| prepare_plain(ctx, &detail, st.label, st.lh, detail_color, WhiteSpace::Pre));
                        let stats = (!agents).then(|| line_stats(part)).flatten().map(|(add, del)| {
                            (
                                prepare_plain(ctx, &format!("+{add}"), st.mono_small, st.lh, ColorRole::Success, WhiteSpace::Pre),
                                prepare_plain(ctx, &format!("−{del}"), st.mono_small, st.lh, ColorRole::Danger, WhiteSpace::Pre),
                            )
                        });
                        let open = !agents && skill.is_none() && self.detail_open.get(&dkey).copied().unwrap_or(false);
                        let mut body = Vec::new();
                        if open && let Some(c) = &computer {
                            body.extend(lines_block(ctx, &st, &computer_call_rows(c.tool, c.args).join("\n"), Some(CALL_WRAP_COLS)));
                            let js_actions = computer_call_js_actions(c, output.as_deref());
                            if !js_actions.is_empty() {
                                let labels: Vec<String> = js_actions.iter().map(|action| computer_js_action_label(action, true)).collect();
                                body.extend(lines_block(ctx, &st, &labels.join("\n"), None));
                            } else if !computer_output_hidden(c.tool, c.args) {
                                body.extend(result_block(ctx, &st, part));
                            }
                        }
                        if open && computer.is_none() {
                            body.extend(lines_block(ctx, &st, &call_text(call), Some(CALL_WRAP_COLS)));
                            let (full, offer) = self.full_result(ctx, &st, part);
                            match full {
                                Some(full) => body.push(full),
                                None => body.extend(result_block(ctx, &st, part)),
                            }
                            body.extend(offer);
                        }
                        let agent = (agents && call.is_subagent_spawn()).then(|| AgentExtra {
                            trailing: spawn_tag(self.harness, call).map(|tag| prepare_plain(ctx, &tag, small, small_lh, ColorRole::TextFaint, WhiteSpace::Pre)),
                            meta: None,
                            route: None,
                            open: None,
                        });
                        lines.push(ToolLine {
                            icon: match (&computer, agents) {
                                (Some(_), _) => "cursorarrow".to_owned(),
                                (None, false) if device.is_some() => "tool-smartphone".to_owned(),
                                (None, true) if subagent_ref.is_some() => brand_icon(self.harness, call.subagent_model()),
                                (None, _) => tool_icon(call).to_owned(),
                            },
                            label: prepare_plain(ctx, &label, if agents { medium } else { st.label }, st.lh, color, WhiteSpace::Pre),
                            detail,
                            badge,
                            stats,
                            failed: *is_error,
                            running,
                            key: dkey,
                            open,
                            body,
                            agent,
                            link: skill.map(|name| skill_link(name, call.skill_path())),
                        });
                    }
                    MessagePart::Reasoning { text, .. } => {
                        // A streaming thought opens by default (desktop). Live
                        // only while it is the tail of a streaming reply:
                        // once anything follows, the thought is finished even
                        // though the entry still streams.
                        let tail = live && i == last;
                        let open = self.detail_open.get(&dkey).copied().unwrap_or(tail);
                        let body = if open && !text.trim().is_empty() {
                            // Same parse wiring as text parts: incremental while
                            // streaming, inline markers mended for display, the
                            // canonical tree once settled.
                            vec![DetailBlock::Thought(self.thought_body(ctx, &st, &format!("{entry_id}#{}", part.id()), text, tail))]
                        } else {
                            Vec::new()
                        };
                        lines.push(ToolLine {
                            icon: "tool-chat-round-line".into(),
                            label: prepare_plain(ctx, "Thought process", st.label, st.lh, ColorRole::TextSecondary, WhiteSpace::Pre),
                            detail: None,
                            badge: None,
                            stats: None,
                            failed: false,
                            running: false,
                            key: dkey,
                            open,
                            body,
                            agent: None,
                            link: None,
                        });
                    }
                    _ => {}
                }
            }
        }
        RowCore {
            key,
            version: next_version(),
            kind: RowKind::Tools,
            entry_id: Arc::from(entry_id),
            content: Content::Tools(ToolGroup { summary, lines, expanded, live, agents }),
            copy_text: String::new(),
        }
    }
}

/// Unbound with a route, the delegate ran as this chat's native sub-agent.
#[allow(clippy::too_many_arguments)]
fn delegate_line(
    ctx: &mut Ctx,
    st: &Styles,
    medium: super::style::Resolved,
    call: &ToolCall,
    child: Option<&str>,
    native_route: Option<&ChildRoute>,
    key: u64,
    chats: &Chats,
    own: Option<&ChatRef>,
) -> ToolLine {
    let chat = child.and_then(|id| chats.get(id));
    let native = child.is_none();
    let runs_on = if native { own } else { chat };
    let harness = runs_on.and_then(|c| c.harness);
    let model = runs_on.and_then(|c| c.model.as_deref());
    let status = chat.and_then(|c| c.status);
    let failed = status == Some(DelegationStatus::Failed);
    let route = if native { native_route.cloned() } else { chat.and_then(|c| c.route.clone()) };
    let name = harness.map_or("Agent", |h| h.display_name());
    let label = if native { format!("{name} native") } else { name.to_owned() };
    let title = if native {
        let input = match call {
            ToolCall::Mcp { input, .. } | ToolCall::Unknown { input, .. } => input.as_ref(),
            _ => None,
        };
        input.and_then(|i| i.get("title")?.as_str()).or(call.delegate_task_text()).map(single_line)
    } else {
        chat.and_then(|c| c.title.as_deref()).map(single_line)
    }
    .filter(|t| !t.is_empty())
    .unwrap_or_else(|| "Delegated task".into());
    let small = ctx.typo.style(Family::Sans, Weight::Regular, false, 11.0 * T);
    let small_lh = ctx.typo.px(AGENT_SUB * T);
    let role = route.as_ref().and_then(|r| r.role.clone());
    let model_text = model.map(|m| model_label(harness, m));
    let meta = match (role, model_text) {
        (Some(role), Some(model)) => Some(format!("{role} · {model}")),
        (Some(role), None) => Some(role),
        (None, model) => model,
    };
    let route_badge = route.as_ref().and_then(|r| {
        let (text, tint) = route_badge(r)?;
        let wash = match tint {
            ColorRole::Accent => ColorRole::AccentWash,
            ColorRole::Warning => ColorRole::WarningWash,
            _ => ColorRole::CyanWash,
        };
        let ran = harness.map(|h| (h, model));
        let parent = own.and_then(|o| Some((o.harness?, o.model.as_deref())));
        let (title, detail) = route_detail(r, ran, parent, native);
        Some(RouteBadge {
            label: prepare_plain(ctx, &text, small, small_lh, tint, WhiteSpace::Pre),
            wash,
            title,
            detail,
        })
    });
    let limited = chat.is_some_and(|c| c.limit.as_ref().is_some_and(|l| l.offer.is_some()));
    let trailing = (!native).then(|| {
        if limited {
            return prepare_plain(ctx, "Usage limit", small, small_lh, ColorRole::Warning, WhiteSpace::Pre);
        }
        let state = zeron_proto::transcript::delegate_state_text(status, chat.and_then(|c| c.elapsed_secs), false);
        prepare_plain(ctx, &state, small, small_lh, if failed { ColorRole::Danger } else { ColorRole::TextFaint }, WhiteSpace::Pre)
    });
    ToolLine {
        icon: brand_icon(harness, model),
        label: prepare_plain(ctx, &label, medium, st.lh, if failed { ColorRole::Danger } else { ColorRole::TextSecondary }, WhiteSpace::Pre),
        detail: Some(prepare_plain(ctx, &title, st.label, st.lh, if failed { ColorRole::Danger } else { ColorRole::TextSoft }, WhiteSpace::Pre)),
        badge: None,
        stats: None,
        failed,
        running: status == Some(DelegationStatus::Running),
        key,
        open: false,
        body: Vec::new(),
        agent: Some(AgentExtra {
            trailing,
            meta: meta.map(|m| prepare_plain(ctx, &m, small, small_lh, ColorRole::TextFaint, WhiteSpace::Pre)),
            route: route_badge,
            open: child.map(|id| if limited { limit_link(id) } else { chat_link(id) }),
        }),
        link: None,
    }
}

// MARK: - Placement

pub(crate) fn place_tools(t: &ToolGroup, px: Px, x: f32, y: f32, cw: f32, mut out: Option<&mut DisplayBuilder>) -> f32 {
    if t.agents {
        return place_agents(t, px, x, y, cw, out);
    }
    let hh = d(px, HEADER_H);
    if let Some(o) = out.as_deref_mut() {
        let cs = d(px, 14.0);
        o.widget(WidgetKind::Chevron { expanded: t.expanded }, (x + d(px, TRUNK_X) - cs / 2.0, y + (hh - cs) / 2.0, cs, cs), None);
        let tx = x + d(px, TITLE_X);
        let tw = (cw - d(px, TITLE_X) - d(px, 4.0)).max(1.0);
        let sw = t.summary.p.max_content_width().min(tw);
        let ty = y + (hh - t.summary.lh) / 2.0;
        place_text_lines(&t.summary, tx, ty, tw, 1, px, o);
        if t.live {
            o.widget(WidgetKind::Shimmer, (tx, ty, sw, t.summary.lh), None);
        }
        o.widget(WidgetKind::Disclosure { expanded: t.expanded }, (x, y, (d(px, TITLE_X) + sw + d(px, 12.0)).min(cw), hh), None);
    }
    if !t.expanded || t.lines.is_empty() {
        return hh;
    }
    let row_h = d(px, ROW_H);
    let mut ry = y + hh + d(px, TOP_PAD);
    let mut tops = Vec::with_capacity(t.lines.len());
    let mut heights = Vec::with_capacity(t.lines.len());
    let bx = x + d(px, TEXT_X);
    let bw = (cw - d(px, TEXT_X)).max(1.0);
    for line in &t.lines {
        if let Some(o) = out.as_deref_mut() {
            place_line_header(line, px, x, ry, cw, o);
        }
        let body = place_body(line, px, bx, ry + row_h, bw, out.as_deref_mut());
        tops.push(ry - y);
        heights.push(row_h + body);
        ry += row_h + body;
    }
    if let Some(o) = out {
        o.widget(
            WidgetKind::ToolRail {
                trunk_x: d(px, TRUNK_X),
                bend: d(px, BEND),
                branch_end: d(px, BRANCH_END),
                row_mid: row_h / 2.0,
                tops,
                heights,
            },
            (x, y, d(px, TEXT_X), ry - y),
            None,
        );
    }
    ry - y
}

fn place_line_header(line: &ToolLine, px: Px, x: f32, ry: f32, cw: f32, o: &mut DisplayBuilder) {
    let row_h = d(px, ROW_H);
    let is = d(px, ICON);
    let icon_color = if line.failed { ColorRole::Danger } else { ColorRole::TextSecondary };
    o.widget(WidgetKind::Icon { name: line.icon.clone(), color: icon_color }, (x + d(px, ICON_LEFT), ry + (row_h - is) / 2.0, is, is), None);
    let tx = x + d(px, TEXT_X);
    let lw = line.label.p.max_content_width();
    place_text(&line.label, tx, ry + (row_h - line.label.lh) / 2.0, lw + 1.0, Some(o));
    let dx = tx + lw + d(px, 8.0);
    let stats_w = line.stats.as_ref().map_or(0.0, |(add, del)| add.p.max_content_width() + d(px, 4.0) + del.p.max_content_width());
    let avail = (x + cw - dx - if stats_w > 0.0 { stats_w + d(px, 8.0) } else { 0.0 }).max(0.0);
    let mut used = 0.0;
    if let Some((asset, name)) = &line.badge {
        let bh = d(px, 22.0);
        let by = ry + (row_h - bh) / 2.0;
        let nw = name.p.max_content_width();
        let bw = (d(px, 1.0 + 20.0 + 6.0 + 6.0) + nw).min(avail);
        if bw > d(px, 34.0) {
            o.fill(dx, by, bw, bh, d(px, 5.0), ColorRole::ToolBadge);
            let well = d(px, 20.0);
            o.fill(dx + d(px, 1.0), by + d(px, 1.0), well, well, d(px, 4.0), ColorRole::ToolWell);
            let fs = d(px, 14.0);
            o.widget(
                WidgetKind::Icon { name: asset.clone(), color: ColorRole::TextSoft },
                (dx + d(px, 1.0) + (well - fs) / 2.0, by + d(px, 1.0) + (well - fs) / 2.0, fs, fs),
                None,
            );
            place_text_lines(name, dx + d(px, 27.0), ry + (row_h - name.lh) / 2.0, (bw - d(px, 33.0)).max(1.0), 1, px, o);
            used = bw;
        }
    } else if let Some(detail) = &line.detail {
        if avail > 1.0 {
            place_text_lines(detail, dx, ry + (row_h - detail.lh) / 2.0, avail, 1, px, o);
            used = detail.p.max_content_width().min(avail);
        }
    }
    if let Some((add, del)) = &line.stats {
        let sx = dx + if used > 0.0 { used + d(px, 8.0) } else { 0.0 };
        let aw = add.p.max_content_width();
        place_text(add, sx, ry + (row_h - add.lh) / 2.0, aw + 1.0, Some(&mut *o));
        place_text(del, sx + aw + d(px, 4.0), ry + (row_h - del.lh) / 2.0, del.p.max_content_width() + 1.0, Some(&mut *o));
    }
    match &line.link {
        Some(url) => o.links.push(super::display::LinkHit { x, y: ry, w: cw, h: row_h, url: url.clone(), scroller: o.scroller }),
        None => {
            o.widget(WidgetKind::ToolToggle { detail: line.key, open: line.open }, (x, ry, cw, row_h), None);
        }
    }
}

/// Inline detail under a row: blocks in the text column, each preceded by a
/// (transparent) 1pt separator.
fn place_body(line: &ToolLine, px: Px, bx: f32, by: f32, bw: f32, mut out: Option<&mut DisplayBuilder>) -> f32 {
    if !line.open || line.body.is_empty() {
        return 0.0;
    }
    let mut y = by;
    for block in &line.body {
        y += d(px, SEPARATOR);
        y += place_block(block, px, bx, y, bw, out.as_deref_mut());
    }
    y - by
}

fn place_block(block: &DetailBlock, px: Px, bx: f32, by: f32, bw: f32, out: Option<&mut DisplayBuilder>) -> f32 {
    let pad = d(px, BODY_PAD);
    let lh = d(px, OUT_LH);
    match block {
        DetailBlock::Lines { lines, more } => {
            let n = lines.len() + more.is_some() as usize;
            let h = pad * 2.0 + n as f32 * lh;
            if let Some(o) = out {
                let content = lines.iter().map(|l| l.p.max_content_width()).fold(0.0f32, f32::max) + d(px, 8.0);
                o.begin_scroller(bx, by + pad, bw, lines.len() as f32 * lh, content.max(bw));
                for (i, l) in lines.iter().enumerate() {
                    place_text(l, 0.0, i as f32 * lh + (lh - l.lh) / 2.0, l.p.max_content_width() + 1.0, Some(o));
                }
                o.end_scroller();
                if let Some(m) = more {
                    place_text(m, bx, by + pad + lines.len() as f32 * lh + (lh - m.lh) / 2.0, m.p.max_content_width() + 1.0, Some(o));
                }
            }
            h
        }
        DetailBlock::Fetch(t) => {
            if let Some(o) = out {
                place_text(t, bx, by + (lh - t.lh) / 2.0, t.p.max_content_width() + 1.0, Some(o));
            }
            lh
        }
        DetailBlock::Thought(t) => {
            let (shown, _) = thought_extent(t, bw);
            if let Some(o) = out {
                place_thought(t, bx, by + pad, bw, o);
            }
            pad * 2.0 + shown as f32 * t.lh
        }
        DetailBlock::Stats(rows) => {
            let h = pad * 2.0 + rows.len() as f32 * lh;
            if let Some(o) = out {
                let fs = d(px, 14.0);
                for (i, r) in rows.iter().enumerate() {
                    let ry = by + pad + i as f32 * lh;
                    o.widget(WidgetKind::Icon { name: r.icon.clone(), color: ColorRole::TextSoft }, (bx, ry + (lh - fs) / 2.0, fs, fs), None);
                    let px0 = bx + fs + d(px, 8.0);
                    let aw = r.add.p.max_content_width();
                    let dw = r.del.p.max_content_width();
                    let tail = aw + dw + d(px, 16.0);
                    let pw = r.path.p.max_content_width().min((bx + bw - px0 - tail).max(1.0));
                    place_text_lines(&r.path, px0, ry + (lh - r.path.lh) / 2.0, pw.max(1.0), 1, px, o);
                    let ax = px0 + pw + d(px, 8.0);
                    place_text(&r.add, ax, ry + (lh - r.add.lh) / 2.0, aw + 1.0, Some(o));
                    place_text(&r.del, ax + aw + d(px, 8.0), ry + (lh - r.del.lh) / 2.0, dw + 1.0, Some(o));
                }
            }
            h
        }
        DetailBlock::Diff { rows, notice, digits } => {
            let notice_h = if notice.is_some() { d(px, 24.0) } else { 0.0 };
            let row_h = |k: DiffKind| if k == DiffKind::Hunk { d(px, 28.0) } else { d(px, 21.0) };
            let body: f32 = rows.iter().map(|r| row_h(r.kind)).sum();
            let h = notice_h + body + d(px, 8.0);
            if let Some(o) = out {
                if let Some(n) = notice {
                    place_text(n, bx, by + (notice_h - n.lh) / 2.0, n.p.max_content_width() + 1.0, Some(o));
                }
                let bar = 3.0;
                let gw = (*digits as f32 * 6.6 + 14.0).max(36.0) * T * px.v(1.0);
                let marker_w = d(px, 28.0);
                let code_x = bar + gw * 2.0 + marker_w + d(px, 12.0);
                let code_w = rows.iter().map(|r| r.text.p.max_content_width()).fold(0.0f32, f32::max);
                let content = (code_x + code_w + d(px, 12.0)).max(bw);
                o.begin_scroller(bx, by + notice_h, bw, body, content);
                let mut ry = 0.0;
                for r in rows {
                    let rh = row_h(r.kind);
                    match r.kind {
                        DiffKind::Hunk => {
                            o.fill(0.0, ry, content, rh, 0.0, ColorRole::DiffHunk);
                            place_text(&r.text, d(px, 16.0), ry + (rh - r.text.lh) / 2.0, r.text.p.max_content_width() + 1.0, Some(o));
                        }
                        kind => {
                            match kind {
                                DiffKind::Add => {
                                    o.fill(0.0, ry, content, rh, 0.0, ColorRole::DiffAddWash);
                                    o.fill(0.0, ry, bar, rh, 0.0, ColorRole::DiffAddBar);
                                }
                                DiffKind::Del => {
                                    o.fill(0.0, ry, content, rh, 0.0, ColorRole::DiffDelWash);
                                    o.fill(0.0, ry, bar, rh, 0.0, ColorRole::DiffDelBar);
                                }
                                _ => {}
                            }
                            for (i, n) in [&r.old, &r.new].into_iter().enumerate() {
                                if let Some(n) = n {
                                    let w = n.p.max_content_width();
                                    place_text(n, bar + gw * (i as f32 + 1.0) - d(px, 8.0) - w, ry + (rh - n.lh) / 2.0, w + 1.0, Some(o));
                                }
                            }
                            if let Some(m) = &r.marker {
                                let w = m.p.max_content_width();
                                place_text(m, bar + gw * 2.0 + (marker_w - w) / 2.0, ry + (rh - m.lh) / 2.0, w + 1.0, Some(o));
                            }
                            place_text(&r.text, code_x, ry + (rh - r.text.lh) / 2.0, r.text.p.max_content_width() + 1.0, Some(o));
                        }
                    }
                    ry += rh;
                }
                o.end_scroller();
            }
            h
        }
    }
}

/// Subagent spawns and delegates: a bordered 30pt card per call, plus a meta
/// line with the route badge for delegates.
fn agent_card_h(line: &ToolLine, px: Px) -> f32 {
    let sub = line.agent.as_ref().is_some_and(|a| a.meta.is_some() || a.route.is_some());
    d(px, AGENT_CARD) + if sub { d(px, AGENT_SUB + 6.0) } else { 0.0 }
}

fn place_agents(t: &ToolGroup, px: Px, x: f32, y: f32, cw: f32, mut out: Option<&mut DisplayBuilder>) -> f32 {
    let margin = d(px, AGENT_ROW - AGENT_CARD);
    let h = t.lines.iter().map(|l| agent_card_h(l, px) + margin).sum();
    let Some(o) = out.as_deref_mut() else { return h };
    let mut ry = y;
    for line in &t.lines {
        let card_h = agent_card_h(line, px);
        let head = d(px, AGENT_CARD);
        let card_y = ry + margin / 2.0;
        ry += card_h + margin;
        o.fill(x, card_y, cw, card_h, d(px, 9.0), ColorRole::AgentCard);
        o.hairline(x, card_y, cw, card_h, d(px, 9.0), ColorRole::AgentCardBorder);
        let tile = d(px, 18.0);
        let tx = x + d(px, 8.0);
        let ty = card_y + (head - tile) / 2.0;
        o.fill(tx, ty, tile, tile, d(px, 5.0), ColorRole::AgentTile);
        let is = d(px, 12.0);
        let icon_color = if line.failed { ColorRole::Danger } else { ColorRole::TextSecondary };
        o.widget(WidgetKind::Icon { name: line.icon.clone(), color: icon_color }, (tx + (tile - is) / 2.0, ty + (tile - is) / 2.0, is, is), None);
        let lx = tx + tile + d(px, 8.0);
        let lw = line.label.p.max_content_width();
        place_text(&line.label, lx, card_y + (head - line.label.lh) / 2.0, lw + 1.0, Some(o));
        let extra = line.agent.as_ref();
        let mut right = x + cw - d(px, 8.0);
        if let Some(url) = extra.and_then(|a| a.open.as_ref()) {
            right -= tile;
            o.fill(right, ty, tile, tile, d(px, 5.0), ColorRole::ToolBadge);
            let s = d(px, 10.0);
            o.widget(
                WidgetKind::Icon {
                    name: "arrow.up.right".into(),
                    color: ColorRole::TextSecondary,
                },
                (right + (tile - s) / 2.0, ty + (tile - s) / 2.0, s, s),
                None,
            );
            o.links.push(super::display::LinkHit {
                x,
                y: card_y,
                w: cw,
                h: head,
                url: url.clone(),
                scroller: o.scroller,
            });
            right -= d(px, 8.0);
        }
        if line.running {
            let s = d(px, 14.0);
            right -= s;
            o.widget(WidgetKind::Spinner, (right, card_y + (head - s) / 2.0, s, s), None);
            right -= d(px, 8.0);
        }
        if let Some(trailing) = extra.and_then(|a| a.trailing.as_ref()) {
            let w = trailing.p.max_content_width().min((right - lx - lw - d(px, 40.0)).max(0.0));
            if w > 1.0 {
                right -= w;
                place_text_lines(trailing, right, card_y + (head - trailing.lh) / 2.0, w, 1, px, o);
                right -= d(px, 8.0);
            }
        }
        if let Some(detail) = &line.detail {
            let dx = lx + lw + d(px, 8.0);
            let avail = right - dx;
            if avail > 1.0 {
                place_text_lines(detail, dx, card_y + (head - detail.lh) / 2.0, avail, 1, px, o);
            }
        }
        let Some(extra) = extra.filter(|a| a.meta.is_some() || a.route.is_some()) else { continue };
        let sy = card_y + head - d(px, 3.0);
        let sub_h = d(px, AGENT_SUB);
        let limit = x + cw - d(px, 8.0);
        let mut sx = lx;
        if let Some(meta) = &extra.meta {
            let w = meta.p.max_content_width().min(limit - sx);
            place_text_lines(meta, sx, sy + (sub_h - meta.lh) / 2.0, w.max(1.0), 1, px, o);
            sx += w + d(px, 6.0);
        }
        if let Some(badge) = &extra.route {
            let pad = d(px, 6.0);
            let w = (badge.label.p.max_content_width() + pad * 2.0).min(limit - sx);
            if w > pad * 3.0 {
                o.fill(sx, sy, w, sub_h, d(px, 4.0), badge.wash);
                place_text_lines(&badge.label, sx + pad, sy + (sub_h - badge.label.lh) / 2.0, w - pad * 2.0, 1, px, o);
                o.widget(
                    WidgetKind::Detail { title: badge.title.clone() },
                    (sx, sy - d(px, 4.0), w, sub_h + d(px, 8.0)),
                    Some(badge.detail.clone()),
                );
            }
        }
    }
    h
}

pub(crate) fn heap_bytes(t: &ToolGroup) -> usize {
    t.summary.p.heap_bytes()
        + t.lines
            .iter()
            .map(|l| {
                l.label.p.heap_bytes()
                    + l.detail.as_ref().map_or(0, |d| d.p.heap_bytes())
                    + l.badge.as_ref().map_or(0, |b| b.1.p.heap_bytes())
                    + l.stats.as_ref().map_or(0, |(a, d)| a.p.heap_bytes() + d.p.heap_bytes())
                    + l.agent.as_ref().map_or(0, |a| {
                        a.trailing.as_ref().map_or(0, |t| t.p.heap_bytes())
                            + a.meta.as_ref().map_or(0, |t| t.p.heap_bytes())
                            + a.route.as_ref().map_or(0, |r| r.label.p.heap_bytes() + r.detail.len())
                    })
                    + l.body
                        .iter()
                        .map(|b| match b {
                            DetailBlock::Lines { lines, .. } => lines.iter().map(|p| p.p.heap_bytes()).sum(),
                            DetailBlock::Thought(t) => t
                                .lines
                                .iter()
                                .flat_map(|l| [&l.gutter, &l.bars, &l.body])
                                .flatten()
                                .map(|p| p.p.heap_bytes())
                                .sum::<usize>(),
                            DetailBlock::Stats(rows) => rows.iter().map(|r| r.path.p.heap_bytes()).sum(),
                            DetailBlock::Diff { rows, .. } => rows.iter().map(|r| r.text.p.heap_bytes()).sum(),
                            DetailBlock::Fetch(t) => t.p.heap_bytes(),
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thought_lines_stop_at_the_visible_cap() {
        use zeron_markdown::parser::parse_full;
        let long: String = (0..100).map(|i| format!("para {i}\n\n")).collect();
        let (lines, more) = thought_lines(&parse_full(&long), Some(MAX_LINES));
        assert!(more && lines.len() <= MAX_LINES && lines.len() >= MAX_LINES - 1, "{}", lines.len());
        let (lines, more) = thought_lines(&parse_full("one\n\n- two\n\n```\n  three\n```"), Some(MAX_LINES));
        let text: Vec<String> = lines.iter().map(|l| l.iter().map(|r| r.text.as_str()).collect()).collect();
        assert_eq!(text, ["one", "", "• two", "", "  three"]);
        assert!(!more);
    }

    #[test]
    fn thought_lines_clip_one_huge_paragraph_to_the_byte_budget() {
        use zeron_markdown::parser::parse_full;
        let bytes = |lines: &[Vec<InlineRun>]| lines.iter().flatten().map(|r| r.text.len()).sum::<usize>();
        let (lines, more) = thought_lines(&parse_full(&"word ".repeat(20_000)), Some(MAX_LINES));
        assert!(more && bytes(&lines) <= THOUGHT_BYTES && bytes(&lines) > THOUGHT_BYTES - 8, "{}", bytes(&lines));
        // The budget lands inside a two-byte char: cut on its boundary.
        let (lines, more) = thought_lines(&parse_full(&format!("a{}", "é".repeat(THOUGHT_BYTES))), Some(MAX_LINES));
        assert!(more && bytes(&lines) == THOUGHT_BYTES - 1, "{}", bytes(&lines));
        // Under budget: untouched.
        let (lines, more) = thought_lines(&parse_full("short thought"), Some(MAX_LINES));
        assert!(!more && bytes(&lines) == "short thought".len());
    }

    #[test]
    fn summary_matches_desktop_wording() {
        let calls = vec![
            (ToolCall::Exec { command: "ls".into() }, false),
            (ToolCall::ReadFile { path: "a.rs".into() }, false),
            (ToolCall::Exec { command: "x".into() }, true),
        ];
        assert_eq!(group_summary(0, &calls, &[], false), "Ran 2 commands · read 1 file · 1 failed");
        assert_eq!(group_summary(1, &calls, &[], false), "Thought process · ran 2 commands · read 1 file · 1 failed");
        assert_eq!(group_summary(3, &[], &[], false), "Thought 3 times");
    }
}
