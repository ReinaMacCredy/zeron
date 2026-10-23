//! Transcript entries → rows. One row per top-level markdown block, user
//! message, tool group, chip or image (desktop's `rows_for_entry` model), with
//! stable keys (`{msg}#{part}.{block}`, `{msg}#g{n}`) so rows survive reparses
//! and optimistic sends swap to host echoes without flicker.
//!
//! Everything here is width-independent: [`RowCore`]s hold prepared text and
//! are rebuilt only when their source changes. Heights at a width are the
//! frame's job.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use zeron_doc::parts::MessagePart;
use zeron_doc::parts::MessageStatus;
use zeron_doc::schema::{MessageRole, SessionMessageEntry};
use zeron_markdown::parser::{IncrementalParser, TopBlock};
use zeron_proto::computer::cards::{computer_notice, consent_question, is_control_denied};
use zeron_proto::computer::presentation::computer_call;
use zeron_proto::transcript::CompactionWait;
use zeron_proto::{HarnessId, UserInputAnswer};
use zeron_text::WhiteSpace;

use super::cards::{Card, consent_row, denied_row, foreground_question, foreground_row, notice_row, place_card};
use super::display::{ColorRole, DisplayBuilder, FadeEdge, LinkHit, TextRun, WidgetKind};
use super::file_icons::file_icon_asset;
use super::markdown::geom::PARA_GAP;
use super::markdown::{Ctx, PBlock, PText, Px, place, place_stack, place_text, prepare_block, prepare_plain};
use super::seams::{self, Chats};
use super::strip::{Strip, place_strip};
use super::style::{Family, TYPE, Weight};
use super::tools::{ThoughtState, ToolGroup, place_tools};

/// Row kinds the painter may style differently (e.g. context menus).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RowKind {
    Markdown,
    User,
    Tools,
    Chip,
    Image,
    Working,
    Card,
    Visual,
}

/// A user message awaiting its host echo (client-minted id = the echo's id).
#[derive(Debug, Clone, PartialEq)]
pub struct PendingUser {
    pub id: String,
    pub text: String,
}

/// What the row builder reads from a session snapshot.
#[derive(Debug, Clone, Default)]
pub struct TranscriptInput {
    pub entries: Vec<Arc<SessionMessageEntry>>,
    pub pending: Vec<PendingUser>,
    /// A turn is running (drives the tail working indicator).
    pub working: bool,
    pub working_since_ms: Option<i64>,
    pub token_rate: Option<u32>,
    pub compaction: Option<(zeron_proto::transcript::CompactionWait, i64)>,
    pub streaming: bool,
    pub input_answers: Arc<HashMap<String, Vec<UserInputAnswer>>>,
    pub harness: Option<HarnessId>,
    pub chat_id: Option<String>,
    pub chats: Arc<Chats>,
}

pub(crate) fn row_key(id: &str) -> u64 {
    // FNV-1a: stable across launches so platform caches may persist keys.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

static VERSION: AtomicU64 = AtomicU64::new(1);
pub(crate) fn next_version() -> u64 {
    VERSION.fetch_add(1, Ordering::Relaxed)
}

pub(crate) struct UserBubble {
    pub text: PText,
    pub md: Vec<PBlock>,
    pub images: Vec<String>,
    /// Non-image attachments (desktop-sent ZIPs, docs): icon asset + name.
    pub files: Vec<(String, PText)>,
    pub pending: bool,
    pub expanded: bool,
    pub more: PText,
    pub badges: Vec<UserBadge>,
}

pub(crate) struct UserBadge {
    pub icon: &'static str,
    pub title: String,
    pub label: PText,
    pub details: Vec<zeron_proto::badges::BadgeDetail>,
}

pub(crate) struct Chip {
    pub icon: &'static str,
    pub color: ColorRole,
    pub text: PText,
}

// Cores are built once and shared behind Arc; boxing variants only adds a hop.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Content {
    Block(PBlock),
    User(UserBubble),
    Tools(ToolGroup),
    Chip(Chip),
    Image { reference: String },
    Working { since_ms: Option<i64>, word: String, rate: Option<u32> },
    Card(Card),
    /// An agent visual: its height comes from its worker.
    Visual(Arc<crate::visual::VisualSlot>),
    Strip(Strip),
}

/// A width-independent row: identity, top gap class and prepared content.
pub(crate) struct RowCore {
    pub key: u64,
    pub version: u64,
    pub kind: RowKind,
    pub entry_id: Arc<str>,
    pub content: Content,
    pub copy_text: String,
}

/// A row in transcript order: shared core + its context-dependent top gap.
#[derive(Clone)]
pub(crate) struct Placed {
    pub core: Arc<RowCore>,
    pub gap: Gap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Gap {
    First,
    Turn,
    Reply,
    Block,
    Heading,
}

pub(crate) mod geom {
    pub const MARGIN_X: f32 = 18.0;
    /// The message column's width cap on wide screens (iPad, landscape).
    pub const READING_WIDTH: f32 = 768.0;
    pub const GAP_FIRST: f32 = 14.0;
    pub const GAP_TURN: f32 = 30.0;
    pub const GAP_REPLY: f32 = 18.0;
    pub const GAP_BLOCK: f32 = 12.0;
    pub const GAP_HEADING: f32 = 22.0;
    pub const BUBBLE_PAD_X: f32 = 15.0;
    pub const BUBBLE_PAD_Y: f32 = 10.0;
    pub const BUBBLE_RADIUS: f32 = 20.0;
    /// Width of a trailing overflow fade.
    pub const FADE: f32 = 28.0;
    pub const BUBBLE_FOLD_LINES: usize = 8;
    pub const BUBBLE_FOLD_SHOW: usize = 6;
    pub const THUMB: f32 = 76.0;
    pub const FILE_PILL: f32 = 32.0;
    pub const BADGE: f32 = 26.0;
    pub const CHIP_LINE: f32 = 32.0;
    pub const IMAGE: f32 = 260.0;
    pub const WORKING: f32 = 36.0;
    pub const VISUAL_LOADING: f32 = 18.0;
}

impl Gap {
    pub fn px(self, px: Px) -> f32 {
        use geom::*;
        px.v(match self {
            Gap::First => GAP_FIRST,
            Gap::Turn => GAP_TURN,
            Gap::Reply => GAP_REPLY,
            Gap::Block => GAP_BLOCK,
            Gap::Heading => GAP_HEADING,
        })
    }
}

struct PartState {
    parser: IncrementalParser,
    /// Last source fed to the parser (ptr-cheap equality short-circuit).
    source_len: usize,
    source_hash: u64,
    streaming: bool,
    blocks: Vec<(Arc<TopBlock>, Arc<RowCore>)>,
}

struct EntryState {
    entry: Arc<SessionMessageEntry>,
    sig: u64,
    rows: Vec<Placed>,
}

/// Builds rows incrementally across snapshots.
#[derive(Default)]
pub(crate) struct RowBuilder {
    parts: HashMap<String, PartState>,
    /// Reasoning parts' parse + prepared detail, keyed `{entry}#{part}`.
    pub(crate) thoughts: HashMap<String, ThoughtState>,
    entries: HashMap<String, EntryState>,
    pending: HashMap<String, (String, Arc<RowCore>)>,
    working: Option<Arc<RowCore>>,
    pub expanded: HashSet<u64>,
    pub collapsed: HashSet<u64>,
    /// Per-tool inline detail overrides (row detail key → open).
    pub detail_open: HashMap<u64, bool>,
    /// Visual rows' links to their workers, by visual id (kept across
    /// rebuilds: the worker holds the same slot).
    pub visual_slots: HashMap<String, Arc<crate::visual::VisualSlot>>,
    pub(crate) harness: Option<HarnessId>,
    in_fold: bool,
    pub(crate) blobs: HashMap<String, (Blob, u64)>,
    pub(crate) blob_clock: u64,
    pub(crate) chat_id: Option<String>,
}

pub(crate) enum Blob {
    Loading,
    Ready(Arc<str>),
    Failed,
}

impl Blob {
    fn tag(&self) -> u64 {
        match self {
            Blob::Loading => 1,
            Blob::Ready(_) => 2,
            Blob::Failed => 3,
        }
    }
}

pub(crate) fn quick_hash(s: &str) -> u64 {
    // Change detector for a part's text: the whole text, so a same-length
    // rewrite is caught too. Only parts of messages that changed get here,
    // and re-parsing them is already linear in their length.
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

impl RowBuilder {
    /// Rows for `input`, reusing every row whose source is unchanged.
    pub fn build(&mut self, ctx: &mut Ctx, input: &TranscriptInput) -> Vec<Placed> {
        if self.harness != input.harness {
            self.harness = input.harness;
            self.entries.clear();
        }
        if self.chat_id != input.chat_id {
            self.chat_id = input.chat_id.clone();
            self.blobs.clear();
        }
        let mut out: Vec<Placed> = Vec::with_capacity(self.entries.len() * 2 + 4);
        let mut live_entries: HashSet<&str> = HashSet::with_capacity(input.entries.len());
        let mut live_parts: HashSet<String> = HashSet::new();
        let mut prev_role: Option<MessageRole> = None;
        for entry in &input.entries {
            live_entries.insert(entry.id.as_str());
            let first = out.is_empty();
            let sig = self.entry_sig(entry, first, prev_role, input);
            let reuse = self
                .entries
                .get(&entry.id)
                .filter(|s| Arc::ptr_eq(&s.entry, entry) && s.sig == sig)
                .map(|s| s.rows.clone());
            let rows = match reuse {
                Some(rows) => {
                    for part in &entry.parts {
                        if matches!(part, MessagePart::Text { .. } | MessagePart::Reasoning { .. }) {
                            live_parts.insert(format!("{}#{}", entry.id, part.id()));
                        }
                    }
                    rows
                }
                None => {
                    let rows = self.entry_rows(ctx, entry, first, prev_role, input, &mut live_parts);
                    self.entries.insert(
                        entry.id.clone(),
                        EntryState {
                            entry: entry.clone(),
                            sig,
                            rows: rows.clone(),
                        },
                    );
                    rows
                }
            };
            if !rows.is_empty() {
                prev_role = Some(entry.role);
            }
            out.extend(rows);
        }
        self.entries.retain(|id, _| live_entries.contains(id.as_str()));
        self.parts.retain(|id, _| live_parts.contains(id));
        self.thoughts.retain(|id, _| live_parts.contains(id));

        // Optimistic sends not yet echoed by the host.
        let mut live_pending = HashSet::new();
        for p in &input.pending {
            if live_entries.contains(p.id.as_str()) {
                continue;
            }
            live_pending.insert(p.id.clone());
            let gap = if out.is_empty() { Gap::First } else { Gap::Turn };
            let cached = self
                .pending
                .get(&p.id)
                .filter(|(text, _)| *text == p.text)
                .map(|(_, row)| row.clone());
            let core = cached.unwrap_or_else(|| {
                let row = Arc::new(self.user_row(ctx, &p.id, &p.text, true));
                self.pending.insert(p.id.clone(), (p.text.clone(), row.clone()));
                row
            });
            out.push(Placed { core, gap });
        }
        self.pending.retain(|id, _| live_pending.contains(id));

        if input.working || input.compaction.is_some() {
            let (word, since_ms, token_rate) = match &input.compaction {
                Some((wait, since)) => {
                    let (lead, tail) = wait.words();
                    let chip = match wait {
                        CompactionWait::Classifying { harness, model, .. } => {
                            Some(format!("· {}", seams::side_detail(*harness, model.as_deref())))
                        }
                        _ => None,
                    };
                    let word = [Some(lead), chip, tail.map(str::to_owned)].into_iter().flatten().collect::<Vec<_>>().join(" ");
                    (word, wait.timed().then_some(*since), None)
                }
                None if input.entries.last().is_some_and(|e| zeron_doc::schema::consulting_advisor(e)) => {
                    ("Consulting advisor".to_owned(), input.working_since_ms, input.token_rate)
                }
                None if input.streaming => ("Writing".to_owned(), input.working_since_ms, input.token_rate),
                None => ("Working".to_owned(), input.working_since_ms, input.token_rate),
            };
            let stale = self
                .working
                .as_ref()
                .is_none_or(|w| !matches!(&w.content, Content::Working { since_ms: shown_since, word: shown, rate } if *shown_since == since_ms && *shown == word && *rate == token_rate));
            if stale {
                self.working = Some(Arc::new(RowCore {
                    key: row_key("#working"),
                    version: next_version(),
                    kind: RowKind::Working,
                    entry_id: Arc::from(""),
                    content: Content::Working { since_ms, word, rate: token_rate },
                    copy_text: String::new(),
                }));
            }
            let core = self.working.clone().expect("set above");
            out.push(Placed { core, gap: Gap::Reply });
        }
        out
    }

    /// Drop cached rows owning `key` so the next build re-prepares them
    /// (disclosure toggles). Only that entry pays.
    pub fn invalidate(&mut self, key: u64) {
        self.entries.retain(|_, s| !s.rows.iter().any(|p| p.core.key == key));
        self.pending.retain(|_, (_, row)| row.key != key);
    }

    /// Context outside the entry that changes its rows: gaps, answers on
    /// their way to the entry's questions, and the chats it names.
    fn entry_sig(&self, entry: &SessionMessageEntry, first: bool, prev: Option<MessageRole>, input: &TranscriptInput) -> u64 {
        let gaps = (first as u64) | ((prev.map_or(0, |r| r as u64 + 1)) << 1);
        let sig = entry
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Input { request_id, .. } => input.input_answers.get(request_id),
                _ => None,
            })
            .fold(gaps, |sig, answers| sig.rotate_left(7) ^ quick_hash(&format!("{answers:?}")));
        let sig = if self.blobs.is_empty() {
            sig
        } else {
            entry
                .parts
                .iter()
                .flat_map(|p| super::tools::blob_refs(self.chat_id.as_deref(), p))
                .flatten()
                .filter_map(|r| self.blobs.get(&r))
                .fold(sig, |sig, (blob, order)| sig.rotate_left(7) ^ blob.tag() ^ order << 2)
        };
        let refs = seams::entry_refs(entry);
        let native = entry.parts.iter().any(|p| matches!(p, MessagePart::Tool { delegate_route: Some(_), .. }));
        if refs.is_empty() && !native {
            return sig;
        }
        let own = input.chat_id.as_deref();
        let chats: Vec<_> = refs.iter().map(|id| input.chats.get(id)).collect();
        let parent = own.and_then(|id| input.chats.get(id));
        sig.rotate_left(7) ^ quick_hash(&format!("{own:?}{chats:?}{parent:?}"))
    }

    fn entry_rows(
        &mut self,
        ctx: &mut Ctx,
        entry: &SessionMessageEntry,
        first: bool,
        prev: Option<MessageRole>,
        input: &TranscriptInput,
        live_parts: &mut HashSet<String>,
    ) -> Vec<Placed> {
        let sent = &*input.input_answers;
        let chats = &*input.chats;
        let own = input.chat_id.as_deref();
        let mut rows: Vec<Placed> = Vec::new();
        let gap_for = |rows: &Vec<Placed>, heading: bool| {
            if rows.is_empty() {
                if first {
                    Gap::First
                } else if prev == Some(MessageRole::User) || prev.is_none() {
                    Gap::Reply
                } else {
                    Gap::Block
                }
            } else if heading {
                Gap::Heading
            } else {
                Gap::Block
            }
        };
        if entry.role == MessageRole::User {
            let text = seams::user_text(entry);
            let gap = if first { Gap::First } else { Gap::Turn };
            if let Some(from) = entry.from_chat_id.as_deref() {
                if Some(from) == own {
                    rows.push(Placed { core: Arc::new(seams::auto_continue(ctx, &entry.id)), gap });
                    rows.push(Placed { core: Arc::new(self.user_row(ctx, &entry.id, &text, false)), gap: Gap::First });
                    return rows;
                }
                let delegate = own.is_some() && chats.get(from).and_then(|c| c.delegated_by.as_deref()) == own;
                if let Some(ids) = zeron_proto::transcript::wake_task_ids(&text).filter(|_| delegate) {
                    rows.push(Placed { core: Arc::new(seams::wake_row(ctx, &entry.id, &ids, chats)), gap });
                    return rows;
                }
                let steps = zeron_proto::transcript::step_handback_threads(&text);
                let ids = if steps.len() > 1 { steps } else { vec![from.to_owned()] };
                let senders: Vec<(String, String)> = ids.into_iter().map(|id| (seams::sender_title(chats, &id), id)).map(|(t, id)| (id, t)).collect();
                rows.push(Placed { core: Arc::new(seams::peer_header(ctx, entry, &senders)), gap });
                let body = zeron_proto::transcript::handback_quotes(&text).unwrap_or_else(|| zeron_proto::transcript::agent_message_display(&text));
                let pkey = format!("{}#peer", entry.id);
                live_parts.insert(pkey.clone());
                for (bi, (_, core)) in self.text_blocks(ctx, &pkey, &body, false).iter().enumerate() {
                    let heading = matches!(core.content, Content::Block(PBlock::Heading(_)));
                    rows.push(Placed { core: core.clone(), gap: if heading && bi > 0 { Gap::Heading } else if bi == 0 { Gap::First } else { Gap::Block } });
                }
                return rows;
            }
            if text.trim() == zeron_proto::RESUME_PROMPT {
                let core = seams::divider(ctx, format!("{}#resumed", entry.id), &entry.id, "Resumed", Some(zeron_proto::RESUME_PROMPT));
                rows.push(Placed { core: Arc::new(core), gap });
                return rows;
            }
            rows.push(Placed { core: Arc::new(self.user_row(ctx, &entry.id, &text, false)), gap });
            return rows;
        }
        if entry.role == MessageRole::System {
            let gap = if first { Gap::First } else { Gap::Turn };
            for part in &entry.parts {
                let core = match part {
                    MessagePart::Classifier { id, record } => {
                        let key = format!("{}#{id}", entry.id);
                        let open = self.expanded.contains(&row_key(&key));
                        seams::classifier(ctx, key, &entry.id, record, open)
                    }
                    MessagePart::Handoff { id, harness, text, route } => {
                        let key = format!("{}#{id}", entry.id);
                        let open = self.expanded.contains(&row_key(&key));
                        seams::handoff(ctx, key, &entry.id, *harness, text, route, open)
                    }
                    _ => continue,
                };
                rows.push(Placed { core: Arc::new(core), gap });
                return rows;
            }
            let mut texts = entry.parts.iter().filter_map(|p| match p {
                MessagePart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            });
            let text = texts.next().unwrap_or_default();
            let hint = texts.next().filter(|hint| !hint.trim().is_empty());
            if let Some(notice) = computer_notice(text) {
                if let Some(core) = notice_row(ctx, &format!("{}#notice", entry.id), &entry.id, notice, text, hint) {
                    rows.push(Placed { core: Arc::new(core), gap });
                }
                return rows;
            }
            if !text.trim().is_empty() {
                rows.push(Placed { core: Arc::new(seams::divider(ctx, format!("{}#divider", entry.id), &entry.id, text.trim(), hint)), gap });
            }
            return rows;
        }
        let streaming = entry.status == Some(MessageStatus::Streaming);
        if !streaming && !self.in_fold {
            self.in_fold = true;
            let folded = self.work_fold(ctx, entry, first, prev, input, live_parts);
            self.in_fold = false;
            if let Some(rows) = folded {
                return rows;
            }
        }
        let mut tools: Vec<&MessagePart> = Vec::new();
        let mut group_ix = 0usize;
        let flush_tools = |this: &mut Self, ctx: &mut Ctx, rows: &mut Vec<Placed>, tools: &mut Vec<&MessagePart>, group_ix: &mut usize, tail: bool| {
            if tools.is_empty() {
                return;
            }
            let id = format!("{}#g{}", entry.id, group_ix);
            *group_ix += 1;
            let gap = gap_for(rows, false);
            let agents = tools.iter().any(|p| matches!(p, MessagePart::Tool { call, .. } if is_agent_call(call)));
            let row = this.tool_row(ctx, &entry.id, &id, tools, streaming && tail, agents, chats, own.and_then(|id| chats.get(id)));
            rows.push(Placed { core: Arc::new(row), gap });
            tools.clear();
        };
        let nparts = entry.parts.len();
        let mut denied_tools: HashSet<&'static str> = HashSet::new();
        for (pi, part) in entry.parts.iter().enumerate() {
            match part {
                // Consecutive tools and thoughts share a group; subagent
                // spawns never share one with ordinary tools (desktop genus).
                MessagePart::Tool { id, call, subagent_ref: Some(thread), .. } if call.is_create_thread() => {
                    flush_tools(self, ctx, &mut rows, &mut tools, &mut group_ix, false);
                    let input = match call {
                        zeron_proto::ToolCall::Mcp { input, .. } | zeron_proto::ToolCall::Unknown { input, .. } => input.as_ref(),
                        _ => None,
                    };
                    let title = chats
                        .get(thread)
                        .and_then(|c| c.title.clone())
                        .or_else(|| input.and_then(|i| i.get("title")?.as_str()).map(str::to_owned))
                        .map(|t| seams::single_line(&t))
                        .filter(|t| !t.is_empty())
                        .unwrap_or_else(|| "thread".into());
                    let gap = gap_for(&rows, false);
                    let core = seams::started_thread(ctx, format!("{}#{id}-thread", entry.id), &entry.id, thread, &title);
                    rows.push(Placed { core: Arc::new(core), gap });
                }
                MessagePart::Tool { call, subagent_ref: None, delegate_route: Some(_), .. } if call.is_delegate_task() => {
                    flush_tools(self, ctx, &mut rows, &mut tools, &mut group_ix, false);
                    tools.push(part);
                    flush_tools(self, ctx, &mut rows, &mut tools, &mut group_ix, false);
                }
                MessagePart::Tool { id, call, is_error, output, .. } => {
                    let genus = Genus::of(call);
                    let genus_flip = tools.iter().any(|p| match p {
                        MessagePart::Tool { call, .. } => Genus::of(call) != genus,
                        _ => genus != Genus::Other,
                    });
                    if genus_flip {
                        flush_tools(self, ctx, &mut rows, &mut tools, &mut group_ix, false);
                    }
                    tools.push(part);
                    if let Some(computer) = computer_call(call)
                        && is_control_denied(*is_error, output.as_deref())
                        && denied_tools.insert(computer.tool)
                    {
                        flush_tools(self, ctx, &mut rows, &mut tools, &mut group_ix, false);
                        let gap = gap_for(&rows, false);
                        let core = denied_row(ctx, &format!("{}#{}-denied", entry.id, id), &entry.id);
                        rows.push(Placed { core: Arc::new(core), gap });
                    }
                }
                MessagePart::Reasoning { text, .. } if !text.trim().is_empty() => {
                    if tools.iter().any(|p| matches!(p, MessagePart::Tool { call, .. } if Genus::of(call) != Genus::Other)) {
                        flush_tools(self, ctx, &mut rows, &mut tools, &mut group_ix, false);
                    }
                    live_parts.insert(format!("{}#{}", entry.id, part.id()));
                    tools.push(part);
                }
                MessagePart::Reasoning { .. } => {}
                _ => {
                    flush_tools(self, ctx, &mut rows, &mut tools, &mut group_ix, false);
                    match part {
                        MessagePart::Text { id, text } => {
                            let pkey = format!("{}#{}", entry.id, id);
                            live_parts.insert(pkey.clone());
                            let part_streaming = streaming && pi + 1 == nparts;
                            let blocks = self.text_blocks(ctx, &pkey, text, part_streaming);
                            for (bi, (_, core)) in blocks.iter().enumerate() {
                                let heading = matches!(core.content, Content::Block(PBlock::Heading(_)));
                                let gap = gap_for(&rows, heading && bi > 0);
                                rows.push(Placed { core: core.clone(), gap });
                            }
                        }
                        MessagePart::Image { id, path, .. } => {
                            let gap = gap_for(&rows, false);
                            let core = Arc::new(RowCore {
                                key: row_key(&format!("{}#{}", entry.id, id)),
                                version: next_version(),
                                kind: RowKind::Image,
                                entry_id: Arc::from(entry.id.as_str()),
                                content: Content::Image { reference: path.clone() },
                                copy_text: String::new(),
                            });
                            rows.push(Placed { core, gap });
                        }
                        MessagePart::Visual { id, path, title } => {
                            let gap = gap_for(&rows, false);
                            let slot = self
                                .visual_slots
                                .entry(id.clone())
                                .or_insert_with(|| crate::visual::VisualSlot::new(id, path, title))
                                .clone();
                            let core = Arc::new(RowCore {
                                key: row_key(&format!("{}#{}", entry.id, id)),
                                version: next_version(),
                                kind: RowKind::Visual,
                                entry_id: Arc::from(entry.id.as_str()),
                                content: Content::Visual(slot),
                                copy_text: title.clone(),
                            });
                            rows.push(Placed { core, gap });
                        }
                        MessagePart::Input { id, request_id, questions, resolved, answers, .. } => {
                            let gap = gap_for(&rows, false);
                            let answers_or_sent = answers.as_deref().or_else(|| sent.get(request_id).map(Vec::as_slice));
                            if let Some(question) = consent_question(questions) {
                                let core = consent_row(ctx, &format!("{}#{}", entry.id, id), &entry.id, id, request_id, question, *resolved, answers_or_sent);
                                rows.push(Placed { core: Arc::new(core), gap });
                                continue;
                            }
                            if foreground_question(questions).is_some() {
                                if let Some(core) = foreground_row(ctx, &format!("{}#{}", entry.id, id), &entry.id, id, request_id, questions, *resolved, answers_or_sent) {
                                    rows.push(Placed { core: Arc::new(core), gap });
                                }
                                continue;
                            }
                            let text = if *resolved {
                                "Answered a question".to_owned()
                            } else {
                                questions.first().map_or("Awaiting your answer…".to_owned(), |q| q.question.clone())
                            };
                            rows.push(Placed { core: Arc::new(chip_row(ctx, &entry.id, id, "questionmark.bubble", ColorRole::Accent, &text)), gap });
                        }
                        MessagePart::Error { id, message } => {
                            let gap = gap_for(&rows, false);
                            rows.push(Placed { core: Arc::new(chip_row(ctx, &entry.id, id, "exclamationmark.triangle", ColorRole::Danger, message)), gap });
                        }
                        MessagePart::Fork { id, source_title, .. } => {
                            let gap = gap_for(&rows, false);
                            let text = format!("Forked from {source_title}");
                            rows.push(Placed { core: Arc::new(chip_row(ctx, &entry.id, id, "arrow.triangle.branch", ColorRole::TextTertiary, &text)), gap });
                        }
                        _ => {}
                    }
                }
            }
        }
        flush_tools(self, ctx, &mut rows, &mut tools, &mut group_ix, true);
        rows
    }

    fn work_fold(
        &mut self,
        ctx: &mut Ctx,
        entry: &SessionMessageEntry,
        first: bool,
        prev: Option<MessageRole>,
        input: &TranscriptInput,
        live_parts: &mut HashSet<String>,
    ) -> Option<Vec<Placed>> {
        let parts = &entry.parts;
        let prose = |p: &MessagePart| matches!(p, MessagePart::Text { text, .. } if !text.trim().is_empty());
        let reply_start = parts.iter().rposition(prose).map(|mut start| {
            while start > 0 && prose(&parts[start - 1]) {
                start -= 1;
            }
            start
        });
        let is_work = |ix: usize, part: &MessagePart| match part {
            MessagePart::Tool { .. } => true,
            MessagePart::Reasoning { text, .. } => !text.trim().is_empty(),
            MessagePart::Text { text, .. } => !text.trim().is_empty() && reply_start.is_none_or(|start| ix < start),
            _ => false,
        };
        let first_work = parts.iter().enumerate().position(|(ix, p)| is_work(ix, p))?;
        let pick = |keep: &dyn Fn(usize, &MessagePart) -> bool| SessionMessageEntry {
            parts: parts.iter().enumerate().filter(|(ix, p)| keep(*ix, p)).map(|(_, p)| p.clone()).collect(),
            ..entry.clone()
        };
        let before = pick(&|ix, p| ix < first_work && !is_work(ix, p));
        let work = pick(&|ix, p| is_work(ix, p));
        let after = pick(&|ix, p| ix > first_work && !is_work(ix, p));
        let mut rows = if before.parts.is_empty() { Vec::new() } else { self.entry_rows(ctx, &before, first, prev, input, live_parts) };
        let gap = match (rows.is_empty(), first, prev) {
            (false, ..) => Gap::Block,
            (true, true, _) => Gap::First,
            (true, false, Some(MessageRole::User) | None) => Gap::Reply,
            (true, false, _) => Gap::Block,
        };
        let key = format!("{}#work", entry.id);
        let open = self.expanded.contains(&row_key(&key));
        let worked_secs = entry.duration_ms.filter(|ms| *ms > 0).map(|ms| (ms / 1000).max(1));
        let header = self.work_header(ctx, &entry.id, &key, &work.parts, worked_secs, open);
        rows.push(Placed { core: Arc::new(header), gap });
        let settled = Some(MessageRole::Assistant);
        if open {
            rows.extend(self.entry_rows(ctx, &work, false, settled, input, live_parts));
        }
        if !after.parts.is_empty() {
            rows.extend(self.entry_rows(ctx, &after, false, settled, input, live_parts));
        }
        Some(rows)
    }

    fn text_blocks(&mut self, ctx: &mut Ctx, pkey: &str, text: &str, streaming: bool) -> Vec<(Arc<TopBlock>, Arc<RowCore>)> {
        let hash = quick_hash(text);
        let state = self.parts.entry(pkey.to_owned()).or_insert_with(|| PartState {
            parser: IncrementalParser::new(),
            source_len: usize::MAX,
            source_hash: 0,
            streaming,
            blocks: Vec::new(),
        });
        if state.source_len == text.len() && state.source_hash == hash && state.streaming == streaming {
            return state.blocks.clone();
        }
        state.parser.set_text(text);
        state.source_len = text.len();
        state.source_hash = hash;
        state.streaming = streaming;
        let tree = if streaming { state.parser.display_tree() } else { state.parser.tree().clone() };
        let mut next = Vec::with_capacity(tree.blocks.len());
        for (bi, block) in tree.blocks.iter().enumerate() {
            let copy = text.get(block.range.clone()).unwrap_or("").trim_end();
            // Same rendering isn't enough: the source can still differ (a
            // closing `**` that renders identically) and Copy reads the source.
            let reuse = state
                .blocks
                .get(bi)
                .filter(|(b, core)| (Arc::ptr_eq(b, block) || b.block == block.block) && core.copy_text == copy)
                .map(|(_, core)| core.clone());
            let core = reuse.unwrap_or_else(|| {
                let id = format!("{pkey}.{bi}");
                // Each block is its own row: its formulas share one budget.
                ctx.math_area = 0.0;
                let content = prepare_block(ctx, &block.block, 0, false);
                Arc::new(RowCore {
                    key: row_key(&id),
                    version: next_version(),
                    kind: RowKind::Markdown,
                    entry_id: Arc::from(pkey.split('#').next().unwrap_or("")),
                    content: Content::Block(content),
                    copy_text: copy.to_owned(),
                })
            });
            next.push((block.clone(), core));
        }
        state.blocks = next.clone();
        next
    }

    fn user_row(&mut self, ctx: &mut Ctx, id: &str, content: &str, pending: bool) -> RowCore {
        let key = row_key(&format!("{id}#u"));
        // Shared parser: strips the image trailer *and* hidden Appshot context.
        let mut parsed = zeron_client::attachments::parse_user_message(content);
        let (rest, badges) = zeron_proto::badges::split(&parsed.text);
        parsed.text = rest;
        let (size, lh) = TYPE.body;
        let style = ctx.typo.style(Family::Sans, Weight::Regular, false, size);
        let lh = ctx.typo.px(lh);
        // Chips stand in for their attachments (as on the desktop): those
        // leave the strip, and an image chip opens its upload on tap.
        let mentions = zeron_proto::attachment_mentions::attachment_mentions(&parsed.text);
        let chipped = |path: &str| mentions.iter().any(|mention| mention.names_attachment(path));
        let previews: Vec<(u32, String)> = mentions
            .iter()
            .filter(|mention| mention.is_image)
            .filter_map(|mention| {
                let image = parsed.images.iter().find(|image| mention.names_attachment(&image.path))?;
                Some((mention.index, image.path.clone()))
            })
            .collect();
        let trimmed = parsed.text.trim();
        let tree = user_chips(trimmed, &previews).is_empty().then(|| zeron_markdown::parser::user_markdown(trimmed)).flatten();
        ctx.math_area = 0.0;
        let md = tree.map_or_else(Vec::new, |tree| tree.blocks.iter().map(|top| prepare_block(ctx, &top.block, 0, false)).collect());
        let text = prepare_user_text(ctx, trimmed, style, lh, &previews);
        // Copied text reads attachment chips as their plain label.
        let body = zeron_proto::attachment_mentions::attachment_mention_prompt(&parsed.text);
        let (msize, mlh) = TYPE.small;
        let mstyle = ctx.typo.style(Family::Sans, Weight::Medium, false, msize);
        // Only images go to image widgets; other files (desktop-sent ZIPs)
        // get a name pill, never an image load.
        let (images, others): (Vec<_>, Vec<_>) = parsed
            .images
            .into_iter()
            .filter(|i| !chipped(&i.path))
            .partition(|i| zeron_proto::attachment_mentions::is_image_path(&i.path));
        let images = images.into_iter().map(|i| i.path).collect();
        let files = others
            .iter()
            .map(|f| {
                let name = zeron_proto::attachment_mentions::attachment_display_name(&f.name);
                (file_icon_asset(name), prepare_plain(ctx, name, mstyle, ctx.typo.px(mlh), ColorRole::Text, WhiteSpace::Pre))
            })
            .collect();
        let badges = badges
            .into_iter()
            .map(|badge| UserBadge {
                icon: badge_icon(badge.kind),
                label: prepare_plain(ctx, &badge.label, mstyle, ctx.typo.px(mlh), ColorRole::TextSecondary, WhiteSpace::Pre),
                title: badge.label,
                details: badge.details,
            })
            .collect();
        let expanded = self.expanded.contains(&key);
        let more = prepare_plain(
            ctx,
            if expanded { "Show less" } else { "Show more" },
            mstyle,
            ctx.typo.px(mlh),
            ColorRole::TextSecondary,
            WhiteSpace::Pre,
        );
        RowCore {
            key,
            version: next_version(),
            kind: RowKind::User,
            entry_id: Arc::from(id),
            content: Content::User(UserBubble {
                text,
                md,
                images,
                files,
                pending,
                expanded,
                more,
                badges,
            }),
            copy_text: body,
        }
    }

}

fn badge_icon(kind: zeron_proto::badges::BadgeKind) -> &'static str {
    use zeron_proto::badges::BadgeKind;
    match kind {
        BadgeKind::PullRequest => "arrow.triangle.pull",
        BadgeKind::Comments => "bubble.left",
        BadgeKind::Terminal | BadgeKind::Selection => "text.quote",
    }
}

/// One chip in a sent prompt: its source range, label and icon, and for an
/// image chip the link that opens its upload.
struct UserChip {
    range: std::ops::Range<usize>,
    label: String,
    icon: String,
    open: Option<String>,
}

/// The link an image chip opens: the host attachment ref, percent-encoded.
fn image_preview_link(reference: &str) -> String {
    let mut out = String::from("zeron-preview://image?ref=");
    for byte in reference.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The chips a sent prompt's canonical links read as, like the desktop's:
/// files and folders (`zeron-file:`), skills and commands (`zeron-invoke:`),
/// and attachments (`zeron-image:` / `zeron-attachment:`), in text order.
/// `previews` pairs image chip numbers with the uploads they open.
fn user_chips(body: &str, previews: &[(u32, String)]) -> Vec<UserChip> {
    let mut chips: Vec<UserChip> = zeron_proto::file_mentions::file_mention_links(body)
        .into_iter()
        .map(|link| UserChip {
            icon: if link.is_dir { "fileicon-folders-folder".to_owned() } else { file_icon_asset(&link.path) },
            range: link.range,
            label: link.basename,
            open: None,
        })
        .collect();
    chips.extend(zeron_proto::invocation::invocation_links(body).into_iter().map(|(range, invocation)| UserChip {
        range,
        label: invocation.name().to_owned(),
        icon: if invocation.prefix() == '/' { "command" } else { "wand.and.stars" }.to_owned(),
        open: None,
    }));
    chips.extend(zeron_proto::attachment_mentions::attachment_mentions(body).into_iter().map(|mention| UserChip {
        range: mention.range,
        icon: if mention.is_image { "photo".to_owned() } else { file_icon_asset(&mention.label) },
        open: previews
            .iter()
            .find(|(index, _)| mention.is_image && *index == mention.index)
            .map(|(_, reference)| image_preview_link(reference)),
        label: mention.label,
    }));
    chips.sort_by_key(|chip| chip.range.start);
    // Links never nest; drop any overlap rather than paint two chips at once.
    let mut end = 0;
    chips.retain(|chip| {
        let keep = chip.range.start >= end;
        if keep {
            end = chip.range.end;
        }
        keep
    });
    chips
}

/// User prompt text with its canonical links shown as atomic chips: the
/// desktop's soft pill with an icon well, the label in the body font.
fn prepare_user_text(ctx: &mut Ctx, body: &str, style: super::style::Resolved, lh: f32, previews: &[(u32, String)]) -> PText {
    let chips = user_chips(body, previews);
    if chips.is_empty() {
        return prepare_plain(ctx, body, style, lh, ColorRole::Text, WhiteSpace::PreWrap);
    }
    // The pill sits two points inside the line box, centered like the
    // desktop's; its icon well is the pill's height less a point each side.
    let chip_h = lh - ctx.typo.px(4.0);
    let chip_pad = (1.0 + (chip_h - 2.0) + ctx.typo.px(5.0), ctx.typo.px(6.0));
    let mut text = String::with_capacity(body.len());
    let mut spans = Vec::new();
    let mut paints = Vec::new();
    let mut badges = Vec::new();
    let mut links = Vec::new();
    let mut at = 0;
    // Returns the index of the span pushed for a non-empty piece.
    let mut push = |text: &mut String, piece: &str, pad: (f32, f32), atomic: bool, link: Option<u16>| {
        if piece.is_empty() {
            return None;
        }
        let start = text.len();
        text.push_str(piece);
        spans.push(zeron_text::Span {
            range: start..text.len(),
            style: style.id,
            pad_start: pad.0,
            pad_end: pad.1,
            atomic,
            inline_box: None,
        });
        paints.push(super::markdown::SpanPaint {
            color: ColorRole::Text,
            decoration: super::display::Decoration::None,
            link,
            chip: false,
        });
        Some(spans.len() - 1)
    };
    for chip in chips {
        push(&mut text, &body[at..chip.range.start], (0.0, 0.0), false, None);
        let link = chip.open.map(|url| {
            links.push(url);
            (links.len() - 1) as u16
        });
        if let Some(span) = push(&mut text, &chip.label, chip_pad, true, link) {
            badges.push((span, chip.icon));
        }
        at = chip.range.end;
    }
    push(&mut text, &body[at..], (0.0, 0.0), false, None);
    let p = zeron_text::prepare(
        &ctx.typo.book,
        ctx.cache,
        &text,
        &spans,
        &zeron_text::PrepareOptions {
            white_space: WhiteSpace::PreWrap,
            overflow_wrap: zeron_text::OverflowWrap::Anywhere,
            ..Default::default()
        },
    );
    PText {
        p,
        lh,
        base: super::style::baseline(lh, style),
        paints,
        links,
        chip: ((lh - chip_h) / 2.0, chip_h),
        badges,
        math: Vec::new(),
    }
}

fn is_agent_call(call: &zeron_proto::ToolCall) -> bool {
    call.is_subagent_spawn() || call.is_delegate_task()
}

/// Desktop `ToolGenus`: agents and device calls never share a group with
/// ordinary tools.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Genus {
    Agent,
    Computer,
    Device,
    Other,
}

impl Genus {
    fn of(call: &zeron_proto::ToolCall) -> Self {
        if is_agent_call(call) {
            Genus::Agent
        } else if computer_call(call).is_some() {
            Genus::Computer
        } else if zeron_proto::device_tools::device_call(call).is_some() {
            Genus::Device
        } else {
            Genus::Other
        }
    }
}

pub(crate) fn chip_row(ctx: &mut Ctx, entry_id: &str, part_id: &str, icon: &'static str, color: ColorRole, text: &str) -> RowCore {
    let (size, lh) = TYPE.small;
    let style = ctx.typo.style(Family::Sans, Weight::Medium, false, size);
    let lh = ctx.typo.px(lh);
    RowCore {
        key: row_key(&format!("{entry_id}#{part_id}")),
        version: next_version(),
        kind: RowKind::Chip,
        entry_id: Arc::from(entry_id),
        content: Content::Chip(Chip {
            icon,
            color,
            text: prepare_plain(ctx, text, style, lh, if color == ColorRole::Danger { ColorRole::Danger } else { ColorRole::TextSecondary }, WhiteSpace::Normal),
        }),
        copy_text: text.to_owned(),
    }
}

/// The message column's width in a viewport `width` wide.
pub(crate) fn column_width(px: Px, width: f32) -> f32 {
    use geom::*;
    // Comfortable measure on iPad/landscape: cap the column, center it.
    (width - px.v(MARGIN_X) * 2.0).clamp(40.0, px.v(READING_WIDTH))
}

/// Lays out (and optionally paints) one row at `width` (full viewport width).
/// `visual` is a visual row's measured height (its slot's, read with the
/// frame's version).
pub(crate) fn place_row(
    core: &RowCore,
    gap: Gap,
    px: Px,
    width: f32,
    visual: Option<f32>,
    mut out: Option<&mut DisplayBuilder>,
) -> f32 {
    use geom::*;
    let margin = px.v(MARGIN_X);
    let cw = column_width(px, width);
    let x = ((width - cw) / 2.0).max(margin).floor();
    let top = gap.px(px);
    let body = match &core.content {
        Content::Block(b) => place(b, px, x, top, cw, out),
        Content::User(u) => place_user(u, px, x, top, cw, out),
        Content::Tools(t) => place_tools(t, px, x, top, cw, out),
        Content::Chip(c) => {
            let h = px.v(CHIP_LINE);
            if let Some(out) = out.as_deref_mut() {
                let s = px.v(16.0);
                out.widget(
                    WidgetKind::Icon { name: c.icon.into(), color: c.color },
                    (x, top + (c.text.lh - s) / 2.0 + px.v(4.0), s, s),
                    None,
                );
            }
            let th = place_text(&c.text, x + px.v(24.0), top + px.v(4.0), cw - px.v(24.0), out);
            h.max(th + px.v(8.0))
        }
        Content::Image { reference } => {
            let side = px.v(IMAGE).min(cw);
            if let Some(out) = out {
                out.fill(x, top, side, side, px.v(14.0), ColorRole::ChipBackground);
                out.widget(WidgetKind::Image { reference: reference.clone() }, (x, top, side, side), None);
            }
            side
        }
        Content::Card(card) => place_card(card, px, x, top, cw, out),
        Content::Strip(strip) => place_strip(strip, px, x, top, cw, out),
        Content::Visual(slot) => {
            // Until its worker has measured it: the loading line.
            let h = visual.unwrap_or(px.v(VISUAL_LOADING));
            if let Some(out) = out {
                out.widget(WidgetKind::Visual { id: slot.id.clone() }, (x, top, cw, h), None);
            }
            h
        }
        Content::Working { since_ms, word, rate } => {
            let h = px.v(WORKING);
            if let Some(out) = out {
                out.widget(
                    WidgetKind::Working {
                        since_ms: *since_ms,
                        word: word.clone(),
                        rate: rate.map(|rate| format!("· {rate} tok/s")),
                    },
                    (x, top, cw, h),
                    None,
                );
            }
            h
        }
    };
    top + body
}

/// Draw at most `max_lines` of `t`; returns the drawn height. Overflow
/// fades instead of cutting: a one-line slot keeps the line unwrapped and
/// fades it out at the slot's right edge; a multi-line slot fades its last
/// visible line downward.
pub(crate) fn place_text_lines(t: &PText, x: f32, y: f32, width: f32, max_lines: usize, px: Px, out: &mut DisplayBuilder) -> f32 {
    let runs_before = out.runs.len();
    if max_lines == 1 && t.p.max_content_width() > width {
        // One unwrapped line; runs starting past the slot are dropped, the
        // straddling one is clipped by the fade.
        place_text(t, x, y, t.p.max_content_width() + 1.0, Some(out));
        let limit = x + width;
        let first_baseline = y + t.base;
        let mut i = runs_before;
        while i < out.runs.len() {
            let r = &out.runs[i];
            if r.x >= limit || r.baseline > first_baseline + 0.5 {
                out.runs.remove(i);
            } else {
                i += 1;
            }
        }
        out.links.retain(|l| l.x < limit && l.y < y + t.lh);
        let fw = px.v(geom::FADE).min(width);
        out.fade(limit - fw, y, fw, t.lh, FadeEdge::Trailing);
        return t.lh;
    }
    let h = place_text(t, x, y, width, Some(out));
    let limit = y + max_lines as f32 * t.lh;
    if h > max_lines as f32 * t.lh {
        out.runs.truncate(runs_before + out.runs[runs_before..].iter().take_while(|r: &&TextRun| r.baseline < limit).count());
        out.links.retain(|l| l.y < limit);
        out.fade(x, limit - t.lh, width, t.lh, FadeEdge::Bottom);
        return max_lines as f32 * t.lh;
    }
    h
}

fn place_user(u: &UserBubble, px: Px, x: f32, y: f32, cw: f32, out: Option<&mut DisplayBuilder>) -> f32 {
    use geom::*;
    let pad_x = px.v(BUBBLE_PAD_X);
    let pad_y = px.v(BUBBLE_PAD_Y);
    let max_w = (cw * 0.86).max(cw - px.v(56.0)).min(cw);
    let text_w = (max_w - pad_x * 2.0).max(20.0);
    let md_h = (!u.md.is_empty()).then(|| place_stack(&u.md, px, 0.0, 0.0, text_w, PARA_GAP, None));
    let stats = u.text.p.stats(text_w);
    let folds = md_h.is_none() && stats.line_count > BUBBLE_FOLD_LINES;
    let shown = if folds && !u.expanded { BUBBLE_FOLD_SHOW } else { stats.line_count };
    let text_h = md_h.unwrap_or(shown as f32 * u.text.lh);
    let more_h = if folds { u.more.lh + px.v(4.0) } else { 0.0 };
    let bubble_w = if folds || md_h.is_some() { max_w } else { stats.max_line_width.ceil() + pad_x * 2.0 };
    let bubble_h = if md_h.is_none() && stats.line_count == 0 { 0.0 } else { text_h + more_h + pad_y * 2.0 };
    let side = px.v(THUMB);
    let gap = px.v(6.0);
    let pill_h = px.v(FILE_PILL);
    // Thumbnails row, then one pill per file, stacked above the bubble.
    let files_y = if u.images.is_empty() { 0.0 } else { side + gap };
    let attach_h = (files_y + u.files.len() as f32 * (pill_h + gap) - gap).max(0.0);
    let badge_h = px.v(BADGE);
    let badge_rows = badge_rows(u, px, max_w);
    let badges_h = (badge_rows.len() as f32 * (badge_h + gap) - gap).max(0.0);
    let above = attach_h + if attach_h > 0.0 && badges_h > 0.0 { gap } else { 0.0 } + badges_h;
    let thumbs_h = if above > 0.0 { above + if bubble_h > 0.0 { px.v(8.0) } else { 0.0 } } else { 0.0 };
    let h = thumbs_h + bubble_h;
    let Some(out) = out else { return h };
    // Thumbnails right-aligned above the bubble.
    let mut tx = x + cw - side;
    for img in u.images.iter().rev() {
        out.fill(tx, y, side, side, px.v(12.0), ColorRole::ChipBackground);
        out.widget(WidgetKind::Image { reference: img.clone() }, (tx, y, side, side), None);
        tx -= side + gap;
    }
    // Files: right-aligned icon + name pills; long names fade at the edge.
    let is = px.v(16.0);
    let inset = px.v(10.0) + is + px.v(8.0);
    let chrome = inset + px.v(12.0);
    let name_w = (max_w - chrome).max(1.0);
    let mut fy = y + files_y;
    for (icon, name) in &u.files {
        let pw = chrome + name.p.max_content_width().ceil().min(name_w);
        let fx = x + cw - pw;
        out.fill(fx, fy, pw, pill_h, px.v(12.0), ColorRole::ChipBackground);
        out.widget(WidgetKind::Icon { name: icon.clone(), color: ColorRole::TextSoft }, (fx + px.v(10.0), fy + (pill_h - is) / 2.0, is, is), None);
        place_text_lines(name, fx + inset, fy + (pill_h - name.lh) / 2.0, name_w, 1, px, out);
        fy += pill_h + gap;
    }
    let mut by = y + attach_h + if attach_h > 0.0 && badges_h > 0.0 { gap } else { 0.0 };
    let bs = px.v(12.0);
    for row in &badge_rows {
        let mut bx = x + cw - row.iter().map(|(_, w)| w + gap).sum::<f32>() + gap;
        for &(i, w) in row {
            let badge = &u.badges[i];
            out.fill(bx, by, w, badge_h, px.v(8.0), ColorRole::ChipBackground);
            out.widget(WidgetKind::Icon { name: badge.icon.into(), color: ColorRole::TextTertiary }, (bx + px.v(8.0), by + (badge_h - bs) / 2.0, bs, bs), None);
            place_text_lines(&badge.label, bx + px.v(26.0), by + (badge_h - badge.label.lh) / 2.0, w - px.v(34.0), 1, px, out);
            if !badge.details.is_empty() {
                out.links.push(LinkHit { x: bx, y: by, w, h: badge_h, url: format!("zeron-badge://open?i={i}"), scroller: out.scroller });
            }
            bx += w + gap;
        }
        by += badge_h + gap;
    }
    if bubble_h > 0.0 {
        let bx = x + cw - bubble_w;
        let by = y + thumbs_h;
        out.fill(bx, by, bubble_w, bubble_h, px.v(BUBBLE_RADIUS).min(bubble_h / 2.0), ColorRole::UserBubble);
        if md_h.is_some() {
            place_stack(&u.md, px, bx + pad_x, by + pad_y, text_w, PARA_GAP, Some(&mut *out));
        } else {
            place_text_lines(&u.text, bx + pad_x, by + pad_y, text_w, shown, px, out);
        }
        if folds {
            let my = by + pad_y + text_h + px.v(4.0);
            let mw = u.more.p.max_content_width();
            place_text(&u.more, bx + pad_x, my, mw + 1.0, Some(out));
            out.widget(WidgetKind::Disclosure { expanded: u.expanded }, (bx, my - px.v(8.0), bubble_w, u.more.lh + px.v(16.0)), None);
        }
        if u.pending {
            // Faded until the host echoes it; the painter animates alpha.
            for r in out.runs.iter_mut().rev().take_while(|r| r.baseline >= by) {
                r.color = ColorRole::TextSecondary;
            }
        }
    }
    h
}

fn badge_rows(u: &UserBubble, px: Px, max_w: f32) -> Vec<Vec<(usize, f32)>> {
    let gap = px.v(6.0);
    let mut rows: Vec<Vec<(usize, f32)>> = Vec::new();
    let mut used = 0.0;
    for (i, badge) in u.badges.iter().enumerate() {
        let w = (px.v(34.0) + badge.label.p.max_content_width().ceil()).min(max_w);
        match rows.last_mut() {
            Some(row) if used + gap + w <= max_w => {
                row.push((i, w));
                used += gap + w;
            }
            _ => {
                rows.push(vec![(i, w)]);
                used = w;
            }
        }
    }
    rows
}

/// Heap held by a row's prepared text (diagnostics; blocks recurse).
pub(crate) fn content_heap_bytes(content: &Content) -> usize {
    fn formula_bytes(f: &zatex::Formula) -> usize {
        f.items.capacity() * std::mem::size_of::<zatex::Item>()
    }
    // The side table itself, then each formula's items.
    fn math_bytes(t: &PText) -> usize {
        t.math.capacity() * std::mem::size_of::<(usize, zatex::Formula)>() + t.math.iter().map(|(_, f)| formula_bytes(f)).sum::<usize>()
    }
    fn block(b: &PBlock) -> usize {
        match b {
            PBlock::Text(t) | PBlock::Heading(t) => t.p.heap_bytes() + math_bytes(t),
            PBlock::Code(c) => c.body.p.heap_bytes() + c.source.len() + c.label.as_ref().map_or(0, |l| l.p.heap_bytes()),
            PBlock::Math(m) => formula_bytes(&m.formula),
            PBlock::Quote(children) => children.iter().map(block).sum(),
            PBlock::List { items, .. } => items
                .iter()
                .map(|i| i.children.iter().map(block).sum::<usize>() + i.marker.as_ref().map_or(0, |m| m.p.heap_bytes()))
                .sum(),
            PBlock::Table(t) => t.cells.iter().flatten().map(|c| c.p.heap_bytes() + math_bytes(c)).sum(),
            PBlock::Rule => 0,
            PBlock::Image { reference, .. } => reference.len(),
        }
    }
    match content {
        Content::Block(b) => block(b),
        Content::User(u) => {
            u.text.p.heap_bytes()
                + u.md.iter().map(block).sum::<usize>()
                + u.more.p.heap_bytes()
                + u.files.iter().map(|(_, n)| n.p.heap_bytes()).sum::<usize>()
                + u.badges.iter().map(|b| b.label.p.heap_bytes()).sum::<usize>()
        }
        Content::Tools(t) => super::tools::heap_bytes(t),
        Content::Chip(c) => c.text.p.heap_bytes(),
        Content::Image { reference } => reference.len(),
        Content::Working { .. } | Content::Visual(_) => 0,
        Content::Card(card) => super::cards::heap_bytes(card),
        Content::Strip(strip) => super::strip::heap_bytes(strip),
    }
}
