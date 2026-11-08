//! A chat, for the chat page's scripts (ui/screens/chat): the session open
//! in a window, its transcript as it streams, the models to pick from, and
//! what can be asked of it. It speaks the engine's IPC as the shipping app's
//! chat view does (crates/ui: state.rs's transcript watch, composer.rs's
//! sends, queue.rs, pickers.rs).
//!
//! What scripts see, per window:
//!   `chat.session`   the open session: { id, state = "loading" | "ready" |
//!                    "missing" | "offline", title, project, device, harness,
//!                    model, effort, fast, access, plan, running, steers?,
//!                    stopped, compacting, prediction?, limit?, work_tree?,
//!                    context, context_tokens?, context_window?, ask?, queue
//!                    (rows: { id, text, cargo?, review?, editing?, locked? }),
//!                    todos, tasks }, nil for none
//!   `chat.messages`  its transcript as ds/transcript's messages, keyed by id
//!   `chat.draft`     the reply being written: { id, text }
//! and for every window, `chat.catalog`: { state, harnesses, models } as
//! ds.ModelPicker takes them, a model's `value` "<harness>/<model>".
//!
//! A reply streams as `append` frames, which carry only its new words: they
//! reach `chat.draft` alone, so a word costs a small state's write, not the
//! whole transcript's. Anything else (a tool, a new entry, the turn ending)
//! publishes `chat.messages` again whole, which `Shared::set` reconciles, so
//! the messages that didn't change keep their rows.
//!
//! Calls (each rejecting with an outcome word: offline, signed_out, missing,
//! failed):
//!   chat.open(id?) (an action)    watch that chat, the last one let go
//!   chat.catalog (an action)      the models, looked up again
//!   chat.send(req)                run a message, or queue it while a turn runs
//!                                 (then its queued row's id)
//!   chat.interrupt(id)
//!   chat.respond({ id, request, decision = "once" | "always" | "deny" |
//!                  "answer", values?, text? })
//!   chat.answer_computer({ id, request, question, answer })  a computer card, answered
//!   chat.configure({ id, model?, effort?, fast?, access?, plan? })
//!   chat.queue({ id, op = "remove" | "send" | "move", item, to? })
//!                                 ("gone": the row had left the queue)
//!   chat.steer_all({ id, items, message_id })
//!   chat.limit({ id, op = "switch" | "wait" | "arm" | "cancel" })
//!   chat.compact({ id, focus? })  (rejects with the reason, in words)
//!   chat.set_work_tree({ id, path? })
//!   chat.prediction_consume({ id })
//!   chat.read_text({ path, session? })  { text, lines }
//!   chat.stat_files({ paths })    { { path, size, is_file, readable } }
//!
//! The wire (crates/rpc, crates/doc): `WatchDocMessages` (a reset, then
//! deltas: upserts after an anchor, text appended to a part, removals, the
//! count to check against; byte lengths), `WatchSessions` and `WatchChats`
//! (whole lists), `WatchQueue`; runs, interrupts and answers through
//! `QueueCommand`; config through `Mutate setChatConfig`; uploads through
//! `UploadChunk` / `UploadCommit`. `ZERON_ZGX_MOCK=1` offers the engine's
//! mock agent, for trying a turn without a paid one.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::{Rc, Weak},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value as Json, json};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Shared, Value};

use crate::auth::text;
use crate::engine::{
    self,
    rpc::{Rpc, Watch},
};

mod code_links;
mod pills;
mod review;
mod tool_images;

type Outcome = &'static str;

/// As gpui's working row: the words wait this long, so they never flash on a send that goes at once.
const CLOUD_BOOT_WORD_AFTER: std::time::Duration = std::time::Duration::from_millis(1500);
const CLOUD_BOOT_POLL: std::time::Duration = std::time::Duration::from_secs(3);

/// A working session the engine hasn't touched for this long has died
/// with its host (proto/view.rs).
const STALE_SECONDS: i64 = 45;
/// How much of an upload goes in one chunk, in base64 characters (the
/// shipping app's).
const CHUNK: usize = 680_000;

thread_local! {
    static CHATS: Rc<Chats> = Rc::new(Chats::default());
}

/// What every window shares: the engine's chats and sessions, this device,
/// the agents and their models.
#[derive(Default)]
struct Chats {
    chats: RefCell<HashMap<String, Json>>,
    chats_heard: Cell<bool>,
    sessions: RefCell<HashMap<String, Json>>,
    here: RefCell<Option<String>>,
    harnesses: RefCell<Vec<Json>>,
    /// Models by harness, as the engine lists them.
    models: RefCell<HashMap<String, Vec<Json>>>,
    // What stopped an agent's models listing, for its row in the picker.
    model_errors: RefCell<HashMap<String, String>>,
    // Models a listing named new for the first time, each numbered, for the picker's notice.
    announced: RefCell<Vec<(u32, String, Vec<String>)>>,
    catalog: RefCell<Vec<Shared>>,
    catalog_state: Cell<CatalogState>,
    views: RefCell<Vec<Weak<View>>>,
    watches: RefCell<Vec<Watch>>,
    rates: RefCell<HashMap<String, RateWatch>>,
    /// By chat: its stats before its latest turn began, and that turn's number (`turn_base`).
    turn_bases: RefCell<HashMap<String, (zeron_proto::SessionStats, u64)>>,
    machine_polls: RefCell<std::collections::HashSet<String>>,
    rpc: RefCell<Option<Rpc>>,
    /// What each chat waiting on its person asks: "approval" or
    /// "question", by chat (`chat.asks`, for the sidebar's rows).
    asks: RefCell<HashMap<String, &'static str>>,
    asks_shared: RefCell<Vec<Shared>>,
    waiting: RefCell<HashMap<String, pills::Waiting>>,
    pages: RefCell<HashMap<String, pills::Wizard>>,
    engine_capabilities: RefCell<Vec<String>>,
    waiting_shared: RefCell<Vec<Shared>>,
    ask_tries: RefCell<HashMap<String, u32>>,
    /// A look at a waiting chat's latest entries, to tell which, until
    /// its first frame comes.
    ask_looks: RefCell<HashMap<String, Watch>>,
    /// The agents and models of the device the composer runs on, when that
    /// isn't this one (a remote session's, or Work in's for a new chat):
    /// `chat.catalog` is then its catalog, as the shipping app's pickers
    /// read their run device's (`list_harnesses(device)`).
    remote: RefCell<Option<Remote>>,
    /// Each device's catalog as last looked at whole (some looked at ahead of
    /// time, `look_ahead`): moving to it shows this at once, while it's looked
    /// at again, so a model is never missing for the moment that takes.
    known_remotes: RefCell<HashMap<String, Remote>>,
    /// The chat a view has just let go of, kept a moment: a chat on its own
    /// that becomes a split is opened again at once, in its pane, and has
    /// what it had rather than reading and building it all again.
    let_go: RefCell<Option<LetGo>>,
    let_go_turn: Cell<u64>,
    compacting: RefCell<HashSet<String>>,
    compaction_probed: RefCell<HashSet<String>>,
    compaction_unsupported: RefCell<HashMap<String, String>>,
}

/// A transcript's entry. A settled one, built into its row and older than
/// the newest few, keeps only its id and a sum of what it was: its row says
/// all it says, and the engine has it if it's wanted again.
struct Entry {
    id: Rc<str>,
    json: Option<Json>,
    /// `sum_of` its json, once that's gone.
    sum: u64,
}

impl Entry {
    fn new(json: Json) -> Self {
        Self { id: json["id"].as_str().unwrap_or_default().into(), json: Some(json), sum: 0 }
    }

    /// What it is now, to tell it from another copy of it.
    fn sum(&self) -> u64 {
        self.json.as_ref().map_or(self.sum, sum_of)
    }
}

/// `entry["field"]`: as the engine sent it; nil for one settled.
impl std::ops::Index<&str> for Entry {
    type Output = Json;
    fn index(&self, field: &str) -> &Json {
        self.json.as_ref().map_or(&Json::Null, |json| &json[field])
    }
}

/// How many of a chat's newest entries keep what the engine sent: what a turn
/// still changes, and what its question and draft are read from.
const WHOLE_TAIL: usize = 32;

/// A sum of a JSON value, the same for equal values (an object's keys are
/// sorted: this workspace's serde_json has no `preserve_order`).
fn sum_of(json: &Json) -> u64 {
    use std::hash::{Hash, Hasher};
    fn feed(json: &Json, hasher: &mut std::hash::DefaultHasher) {
        match json {
            Json::Null => 0u8.hash(hasher),
            Json::Bool(b) => b.hash(hasher),
            Json::Number(n) => n.to_string().hash(hasher),
            Json::String(s) => s.hash(hasher),
            Json::Array(items) => {
                items.len().hash(hasher);
                items.iter().for_each(|item| feed(item, hasher));
            }
            Json::Object(fields) => {
                fields.len().hash(hasher);
                for (key, value) in fields {
                    key.hash(hasher);
                    feed(value, hasher);
                }
            }
        }
    }
    let mut hasher = std::hash::DefaultHasher::new();
    feed(json, &mut hasher);
    hasher.finish()
}

/// A settled entry's row as its row holds it out of view: what makes it,
/// compressed, opened as a script reads it. The transcript's scripts, a
/// template's or a plugin's read the row it opens to, as they would any.
struct FoldedEntry {
    bytes: Box<[u8]>,
    context: Rc<RowContext>,
}

/// What a chat's rows are built with (`built_entry`).
struct RowContext {
    doc: String,
    device: Option<String>,
    model: Option<String>,
    cwd: Option<String>,
}

impl zgx_script::Unfold for FoldedEntry {
    fn unfold(&self) -> Value {
        let json = miniz_oxide::inflate::decompress_to_vec(&self.bytes).ok();
        let entry = json.and_then(|bytes| serde_json::from_slice::<Json>(&bytes).ok()).unwrap_or(Json::Null);
        let RowContext { doc, device, model, cwd } = &*self.context;
        code_links::with_root(cwd.as_deref(), device.is_none(), || built_entry(&entry, doc, device.as_deref(), model.as_deref())).unwrap_or(Value::Nil)
    }
}

/// How hard a settled entry is compressed: fast, as a long chat's are made in
/// slices between frames, and most of what it saves is had by the first level.
const FOLD_LEVEL: i32 = 1;

/// `bytes` compressed, the compressor's tables kept for the next (a compressor is
/// about 300 KB, against the kilobytes of an entry: one a call was gigabytes
/// allocated to open a few long chats).
fn compressed(mut bytes: &[u8]) -> Box<[u8]> {
    use miniz_oxide::deflate::core::{
        CompressorOxide, TDEFLFlush, TDEFLStatus, compress, create_comp_flags_from_zip_params,
    };
    thread_local! {
        static COMPRESSOR: RefCell<CompressorOxide> =
            RefCell::new(CompressorOxide::new(create_comp_flags_from_zip_params(FOLD_LEVEL, 0, 0)));
    }
    COMPRESSOR.with_borrow_mut(|compressor| {
        compressor.reset();
        let mut out = vec![0; (bytes.len() / 2).max(64)];
        let mut at = 0;
        loop {
            let (status, read, written) = compress(compressor, bytes, &mut out[at..], TDEFLFlush::Finish);
            at += written;
            bytes = &bytes[read..];
            if matches!(status, TDEFLStatus::Done) {
                break;
            }
            let room = out.len() * 2;
            out.resize(room, 0);
        }
        out.truncate(at);
        out.into_boxed_slice()
    })
}

/// An entry's row, as a view keeps it: its subagents apart, as the transcript
/// lists them without opening every row (`Subagents`).
#[derive(Clone, Default)]
struct Built {
    row: Option<Value>,
    subagents: Option<Rc<[Value]>>,
}

/// `entry` built: its row, folded if `fold`, which a row that shows or may
/// change isn't. What it shows of it is read as the row is, and held in `Built`.
fn built_for(entry: &Json, context: &Rc<RowContext>, fold: bool) -> Built {
    let RowContext { doc, device, model, .. } = &**context;
    let row = built_entry(entry, doc, device.as_deref(), model.as_deref());
    let subagents = row.as_ref().and_then(subagents_in);
    let row = match row {
        Some(_) if fold && entry["status"].as_str() != Some("streaming") && !kept_whole(entry) => match (entry["id"].as_str(), serde_json::to_vec(entry)) {
            (Some(id), Ok(bytes)) => Some(Value::folded(
                id.into(),
                FoldedEntry {
                    bytes: compressed(&bytes),
                    context: context.clone(),
                },
            )),
            _ => row,
        },
        row => row,
    };
    Built { row, subagents }
}

/// A long chat's settled entries, as a worker thread makes them: what they fold
/// to, and all it read of them. Building and compressing thousands of rows on the
/// main thread takes the window's time and strews its heap's pages with what
/// little stays, so they stay held after the chat has gone. The worker's are its own.
struct Folding {
    results: std::sync::mpsc::Receiver<Vec<FoldedOff>>,
}

/// An entry, folded on the worker (`Folding`).
struct FoldedOff {
    /// Where it stood, and its id, to find it again.
    at: usize,
    id: String,
    /// None: it shows nothing.
    bytes: Option<Box<[u8]>>,
    subagents: Vec<SubagentOff>,
    sum: u64,
    /// The plan in it, if there is one (`Open::older_todo`).
    todo: Option<Json>,
}

/// A subagent's row in `Open`'s list, as plain data to cross threads.
struct SubagentOff {
    words: Vec<(&'static str, Option<String>)>,
    at: Option<f64>,
}

/// `pills::subagent_row`'s words, `ref` first.
const SUBAGENT_WORDS: [&str; 8] = ["ref", "call", "title", "status", "model", "kind", "agent", "reasoning"];

impl SubagentOff {
    fn of(sub: &Value) -> Option<Self> {
        let words = |key: &str| match sub.field(key) {
            Some(Value::Str(words)) => Some(words.to_string()),
            _ => None,
        };
        words("ref")?;
        Some(Self {
            words: SUBAGENT_WORDS.iter().map(|&key| (key, words(key))).collect(),
            at: sub.field("at").and_then(Value::as_f64),
        })
    }

    fn value(&self) -> Value {
        let mut fields: Vec<(&str, Value)> =
            self.words.iter().map(|(key, words)| (*key, words.as_deref().map_or(Value::Nil, Value::from))).collect();
        fields.push(("at", self.at.map_or(Value::Nil, Value::Num)));
        fields.push(("recent", Value::Bool(true)));
        Value::record(fields)
    }
}

/// `entry`, as the worker folds it.
fn fold_off(at: usize, id: String, entry: &Json, doc: &str, device: Option<&str>, model: Option<&str>) -> FoldedOff {
    let row = built_entry(entry, doc, device, model);
    let subagents = row.as_ref().and_then(subagents_in).map_or(Vec::new(), |found| found.iter().filter_map(SubagentOff::of).collect());
    let bytes = row.and_then(|_| serde_json::to_vec(entry).ok()).map(|bytes| compressed(&bytes));
    FoldedOff { at, id, bytes, subagents, sum: sum_of(entry), todo: latest_todo(std::iter::once(entry)).cloned() }
}

/// How many entries the worker sends at a time.
const FOLDED_AT_ONCE: usize = 64;

/// A worker folding `entries`, newest first.
fn folding(entries: Vec<(usize, String, Json)>, context: &RowContext) -> Folding {
    let (sender, results) = std::sync::mpsc::channel();
    let (doc, device, model) = (context.doc.clone(), context.device.clone(), context.model.clone());
    let spawned = std::thread::Builder::new().name("zeron-fold".into()).spawn(move || {
        let mut batch = Vec::with_capacity(FOLDED_AT_ONCE);
        for (at, id, entry) in entries {
            batch.push(fold_off(at, id, &entry, &doc, device.as_deref(), model.as_deref()));
            if batch.len() == FOLDED_AT_ONCE {
                // (The chat has gone, or a reset replaced them: nothing wants the rest.)
                if sender.send(std::mem::take(&mut batch)).is_err() {
                    return;
                }
                batch.reserve(FOLDED_AT_ONCE);
            }
        }
        let _ = sender.send(batch);
    });
    if let Err(error) = spawned {
        eprintln!("zeron-zgx: a chat's rows: no worker ({error})");
    }
    Folding { results }
}

/// What a view's chat had read and built, as it let it go.
struct LetGo {
    id: String,
    entries: Vec<Entry>,
    older_todo: Option<(usize, Json)>,
    built: HashMap<Rc<str>, Built>,
    built_model: Option<Option<String>>,
    context_usage: (Option<f64>, Option<f64>),
    stats: Option<zeron_proto::SessionStats>,
}

/// How long a chat let go of is kept for another view to take it.
const LET_GO_KEPT: std::time::Duration = std::time::Duration::from_secs(2);

/// How long after connecting other devices' catalogs start being looked at.
const LOOK_AHEAD_AFTER: std::time::Duration = std::time::Duration::from_secs(6);

/// How many entries a reset may leave to build at once; more go to a worker and
/// in slices.
const BUILT_AT_ONCE: usize = 64;
/// A slice of building, and the rest after it, so frames get the thread.
const BUILD_SLICE: std::time::Duration = std::time::Duration::from_millis(3);
const BUILD_REST: std::time::Duration = std::time::Duration::from_millis(5);

/// Another device's catalog, read through the engine (`targetDeviceId`).
#[derive(Clone)]
struct Remote {
    device: String,
    harnesses: Vec<Json>,
    models: HashMap<String, Vec<Json>>,
    ready: bool,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum CatalogState {
    #[default]
    Looking,
    Loading,
    Ready,
}

/// A window's open chat.
struct View {
    session: Shared,
    messages: Shared,
    /// Its transcript's subagents, so reading them doesn't read a transcript.
    subagents: Shared,
    draft: Shared,
    stats: Shared,
    /// The live tok/s: on its own, so a rate moving repaints those digits and no reader of the session.
    token_rate: Shared,
    /// The running turn's output so far ("1.2K"), read by the working row on its clock's tick.
    turn_tokens: Shared,
    on_screen: Cell<bool>,
    rpc: RefCell<Option<Rpc>>,
    open: RefCell<Option<Open>>,
    /// Prompts not yet in the engine's transcript, by chat. They outlive
    /// the open view: a send can still be in flight while another chat shows.
    pending: RefCell<HashMap<String, Vec<Pending>>>,
    /// Moves on every open: an older watch's frames are dropped.
    generation: Cell<u64>,
    /// A finished subagent's transcript, read whole from its uploaded blob
    /// (a lane's): nothing to watch while it stands.
    snapshot: Cell<bool>,
    // Watching makes the engine predict for that chat, so only the window's selected chat does, as gpui's.
    predicts: bool,
}

/// A prompt sent and not yet in the transcript: shown under it meanwhile,
/// sending, or failed with what sends it again.
struct Pending {
    id: String,
    words: String,
    paths: Vec<String>,
    /// "sending" or "failed".
    waiting: &'static str,
    /// The command that sends it again, once it failed.
    retry: Option<Json>,
    command: Option<String>,
    /// When it was sent, as its hover strip says.
    at: Option<String>,
    alias: Option<String>,
}

impl Pending {
    fn value(&self) -> Value {
        // (Failed, it stays a prompt: its line to send it again is a prompt's.)
        if self.waiting == "sending" && self.words.trim() == zeron_proto::RESUME_PROMPT {
            return with_fields(
                resumed_row(&self.id),
                [
                    ("waiting", Value::from(self.waiting)),
                    ("here", Value::Bool(true)),
                    ("at", self.at.as_deref().map_or(Value::Nil, Value::from)),
                ],
            );
        }
        // (Its passages as chips, as the sent message shows them.)
        let (words, comments) = split_comments(&self.words);
        let (words, badges) = quote_badges(&words);
        let (words, quotes) = split_quotes(&words);
        let (words, picks) = split_annotations(&words);
        let (words, chosen) = page_picks(&words);
        let badges: Vec<Value> = badges.into_iter().chain(chosen).collect();
        Value::record([
            ("id", Value::from(self.id.as_str())),
            ("role", Value::from("user")),
            ("text", Value::from(words.as_str())),
            ("attachments", carried_value(&self.paths, &quotes, &picks, &comments, &badges)),
            ("waiting", Value::from(self.waiting)),
            // (This device's own copy: its pictures are the files it was
            // sent from, here, which the chat's device never had. The
            // chat's copy, once it comes, is marked `waiting` too until
            // its reply begins, so that can't tell them apart.)
            ("here", Value::Bool(true)),
            ("at", self.at.as_deref().map_or(Value::Nil, Value::from)),
            ("copy", copy_value(&words)),
        ])
    }
}

/// An edit lease on a queued message (the engine's `BeginQueuedMessageEdit`).
#[derive(Clone)]
struct Lease {
    item: String,
    lease: String,
    hash: Json,
}

struct RateWatch {
    _watch: Watch,
    rate: Option<f64>,
}

#[derive(Default)]
struct Open {
    id: String,
    /// By command id: a frame can bring these before `QueueCommand`'s answer.
    rejections: std::collections::HashMap<String, String>,
    refused: Option<String>,
    /// The transcript's entries, as the engine's doc has them (`Entry`).
    entries: Vec<Entry>,
    /// The plan as the newest settled entry holding one left it, and where
    /// that entry stands: the entries are gone, the plan still shows.
    older_todo: Option<(usize, Json)>,
    /// A reset has come (the newest entries, at least).
    heard: bool,
    /// What the context holds as the agent last reported it: tokens in it
    /// and the window's size (either may be missing), for the ring's card.
    context_usage: (Option<f64>, Option<f64>),
    stats: Option<zeron_proto::SessionStats>,
    /// The last stats a frame carried, through frames that carry none.
    stats_seen: Option<zeron_proto::SessionStats>,
    machine: Option<zeron_proto::CloudState>,
    in_flight_seen: bool,
    queue: Vec<Json>,
    /// The session as last published, to republish the transcript when its
    /// turn starts or ends (the prompt's working line follows it).
    session_seen: Json,
    /// A queued message being edited here: its lease, held while the
    /// composer has its text.
    editing: Option<Lease>,
    /// A prompt went and its turn hasn't been seen starting: its entry lands first on a slow session row.
    awaiting_turn: std::cell::Cell<Option<std::time::Instant>>,
    watches: Vec<Watch>,
    /// Each entry as last published (nil: it shows nothing), by its id, and
    /// the model's name it was built with: a republish builds again only
    /// the entries a frame changed, and hands the rest over as the same
    /// values, so the script's copy (reconciled by id) does no work for
    /// them.
    built: HashMap<Rc<str>, Built>,
    built_model: Option<Option<String>>,
    /// What its rows are built with, shared by those folded.
    row_context: Option<Rc<RowContext>>,
    /// The chat's model changed while settled entries' rows carry the last
    /// one's name: its whole copy is asked for again to build them from.
    model_moved: bool,
    /// Its whole copy, when it comes, builds every row again (`model_moved`).
    rebuild: bool,
    /// Opened with the rows another view had (`LetGo`), which the engine's
    /// first frames, its newest entries, mustn't replace.
    has_whole: bool,
    /// Building a reset's entries a slice at a time (`build_in_steps`): what
    /// shows stays as it is until that's done.
    stepping: bool,
    /// The worker folding its settled entries, meanwhile.
    folding: Option<Folding>,
    /// They have gone to it (once a build).
    builds_started: bool,
    /// This view is the one doing it (another view of the chat waits).
    builds: bool,
    host_watched: bool,
    prediction: Option<String>,
    limit: Option<Json>,
    // (A delegate's limit card stands in its parent's transcript.)
    limits: Vec<Json>,
}

pub fn install(_cx: &mut WindowContext, host: &ScriptHost) {
    let chats = CHATS.with(Rc::clone);
    chats.catalog.borrow_mut().push(host.shared("chat.catalog", chats.catalog_value()));
    chats.asks_shared.borrow_mut().push(host.shared("chat.asks", chats.asks_value()));
    pills::install(host);
    review::install(host);
    let view = Rc::new(View {
        session: host.shared("chat.session", Value::Nil),
        messages: host.shared("chat.messages", Value::List(Rc::from([]))),
        subagents: host.shared("chat.subagents", Value::List(Rc::from([]))),
        draft: host.shared("chat.draft", draft_value("", "")),
        stats: host.shared("chat.stats", Value::Nil),
        token_rate: host.shared("chat.token_rate", Value::Nil),
        turn_tokens: host.shared("chat.turn_tokens", Value::Nil),
        on_screen: Cell::new(true),
        rpc: RefCell::default(),
        open: RefCell::default(),
        pending: RefCell::default(),
        generation: Cell::new(0),
        snapshot: Cell::new(false),
        predicts: true,
    });
    chats.views.borrow_mut().push(Rc::downgrade(&view));
    if chats.views.borrow().len() == 1 {
        let weak = Rc::downgrade(&chats);
        engine::get().on_connect(move |rpc| {
            if let Some(chats) = weak.upgrade() {
                chats.connected(rpc);
            }
        });
    }
    let weak = Rc::downgrade(&view);
    engine::get().on_connect(move |rpc| {
        if let Some(view) = weak.upgrade() {
            *view.rpc.borrow_mut() = Some(rpc.clone());
            view.subscribe();
        }
    });

    let v = view.clone();
    host.action("chat.open", move |id| {
        let id = text(&id);
        v.open_chat((!id.is_empty()).then_some(id));
    });
    // A window's lanes: more chats watched beside the open one, each in a
    // slot of its own (a panel tab's side chat, or a subagent's doc), its
    // states `chat.lane.<slot>.session`, `.messages`, `.draft`, as the open
    // chat's are. `{ slot, id, frozen?, chat? }`; no id lets the slot's chat
    // go. `frozen`: a finished subagent's doc, read from its upload first.
    let lanes: Rc<RefCell<HashMap<String, Rc<View>>>> = Rc::default();
    {
        let (lanes, host) = (lanes.clone(), host.clone());
        host.clone().action("chat.open_lane", move |args| {
            let slot = args.field("slot").map(text).unwrap_or_default();
            if slot.is_empty() {
                return;
            }
            let id = args.field("id").map(text).filter(|s| !s.is_empty());
            let view = lanes.borrow_mut().entry(slot.clone()).or_insert_with(|| lane_view(&host, &slot)).clone();
            // A finished subagent: its uploaded transcript first.
            match (id, args.field("frozen").is_some_and(Value::truthy)) {
                (Some(id), true) => view.open_frozen(id, args.field("chat").map(text).filter(|s| !s.is_empty())),
                (id, _) => view.open_chat(id),
            }
        });
    }
    {
        let (lanes, host) = (lanes.clone(), host.clone());
        host.clone().action("chat.lane_on_screen", move |args| {
            let slot = args.field("slot").map(text).unwrap_or_default();
            if slot.is_empty() {
                return;
            }
            let view = lanes.borrow_mut().entry(slot.clone()).or_insert_with(|| lane_view(&host, &slot)).clone();
            view.on_screen.set(args.field("shown").is_some_and(Value::truthy));
            CHATS.with(Rc::clone).follow_rates();
        });
    }
    // The view a call is about: the lane that has its chat open, else the
    // window's open chat.
    let which = {
        let (main, lanes) = (view.clone(), lanes.clone());
        move |args: &Value| -> Rc<View> {
            let id = args.field("session").or_else(|| args.field("id")).map(text).unwrap_or_default();
            lanes.borrow().values().find(|l| l.open_id().as_deref() == Some(id.as_str())).cloned().unwrap_or_else(|| main.clone())
        }
    };
    let c = chats.clone();
    host.action("chat.catalog", move |_| {
        let c = c.clone();
        engine::get().spawn(async move { c.load_catalog(true).await });
    });
    // The composer's device: its catalog is `chat.catalog` from now on (nil
    // or this device: this one's).
    let c = chats.clone();
    host.action("chat.catalog_on", move |arg| {
        let device = arg.field("device").map(crate::auth::text).unwrap_or_default();
        c.catalog_on(&device);
    });
    let pick = which.clone();
    host.async_action("chat.send", move |req| {
        let v = pick(&req);
        async move {
            crate::plugins::preview::refuse("chat.send")?;
            // gpui: a failed send goes back to the draft and its echo goes, so Retry isn't a second way to send it.
            let restored = req.field("restores").is_some_and(Value::truthy).then(|| req.field("message_id").map(text)).flatten();
            let sent = v.send(req).await;
            if sent.is_err() {
                v.unshow(&restored);
            }
            sent.map(|queued| queued.map_or(Value::Nil, |id| Value::from(id.as_str()))).map_err(String::from)
        }
    });
    host.async_action("chat.interrupt", move |id| async move {
        command(&text(&id), json!({ "kind": "interrupt" })).await?;
        Ok(Value::Nil)
    });
    // A background process of the chat's stopped, on its host (the shipping
    // app's `stop_background_task`): whether it's stopping.
    host.async_action("chat.stop_task", move |args| async move { stop_task(args).await.map_err(String::from) });
    host.async_action("chat.stop_waiting", move |id| async move { stop_waiting(text(&id)).await.map_err(String::from) });
    let pick = which.clone();
    host.async_action("chat.respond", move |args| {
        let v = pick(&args);
        async move { crate::plugins::preview::refuse("chat.respond")?; v.respond(args).await.map(|_| Value::Nil).map_err(String::from) }
    });
    // A question's Back: its page before again, with what was picked there.
    let back = which.clone();
    host.async_action("chat.question_back", move |args| {
        let v = back(&args);
        async move { Ok::<Value, String>(v.question_back(&args)) }
    });
    let v = view.clone();
    host.async_action("chat.configure", move |args| {
        let v = v.clone();
        // (Its model and effort are a preview's to change; what an agent may
        // do unasked isn't.)
        let access = args.field("access").is_some();
        async move { if access { crate::plugins::preview::refuse("chat.configure")?; } v.configure(args).await.map(|_| Value::Nil).map_err(String::from) }
    });
    // Shown as the call is made: a chat on another device answers a round trip later.
    let pick = which.clone();
    // A profile applied to a chat, or none ("No profile"), as one config
    // write: `{ id, profile?, note?, note_hint? }`.
    host.async_action("chat.apply_profile", move |args| async move {
        crate::plugins::preview::refuse("chat.apply_profile")?;
        apply_profile(args).await.map(|_| Value::Nil).map_err(String::from)
    });
    host.async_action("chat.queue", move |args| {
        let v = pick(&args);
        let allowed = crate::plugins::preview::refuse("chat.queue");
        let shown = if allowed.is_ok() { v.queue_shown(&args) } else { None };
        async move { allowed?; v.queue_op(args, shown).await.map(|_| Value::Nil).map_err(String::from) }
    });
    let pick = which.clone();
    host.async_action("chat.steer_all", move |args| {
        let v = pick(&args);
        let allowed = crate::plugins::preview::refuse("chat.steer_all");
        let shown = if allowed.is_ok() { v.steer_all_shown(&args) } else { None };
        async move { allowed?; v.steer_all(args, shown).await.map(|_| Value::Nil).map_err(String::from) }
    });
    let pick = which.clone();
    host.async_action("chat.prediction_consume", move |args| {
        pick(&args).consume_prediction(&args);
        async move { Ok::<Value, String>(Value::Nil) }
    });
    host.async_action("chat.limit", move |args| async move {
        crate::plugins::preview::refuse("chat.limit")?;
        limit_op(args).await.map_err(String::from)
    });
    host.async_action("chat.compact", move |args| async move {
        crate::plugins::preview::refuse("chat.compact")?;
        compact(args).await
    });
    host.async_action("chat.answer_computer", move |args| async move {
        crate::plugins::preview::refuse("chat.answer_computer")?;
        answer_computer(args).await.map_err(String::from)
    });
    host.async_action("chat.answer_secret", move |args| async move {
        crate::plugins::preview::refuse("chat.answer_secret")?;
        answer_secret(args).await
    });
    host.async_action("chat.set_work_tree", move |args| async move {
        crate::plugins::preview::refuse("chat.set_work_tree")?;
        set_work_tree(args).await.map_err(String::from)
    });
    host.async_action("chat.read_text", move |args| async move { read_text(args).await.map_err(String::from) });
    host.async_action("chat.stat_files", move |args| async move { stat_files(args).await.map_err(String::from) });
    let pick = which.clone();
    host.async_action("chat.retry", move |args| {
        let v = pick(&args);
        async move { crate::plugins::preview::refuse("chat.retry")?; v.retry(args).await.map(|_| Value::Nil).map_err(String::from) }
    });
    let pick = which.clone();
    host.async_action("chat.queue_edit", move |args| {
        let v = pick(&args);
        async move { crate::plugins::preview::refuse("chat.queue_edit")?; v.begin_edit(args).await.map_err(String::from) }
    });
    let pick = which.clone();
    host.async_action("chat.queue_finish", move |args| {
        let v = pick(&args);
        async move { crate::plugins::preview::refuse("chat.queue_finish")?; v.finish_edit(args).await.map(|_| Value::Nil).map_err(String::from) }
    });
    // A project's branches, from git where it is (the shipping app's
    // `ensure_refs`): `{ path, device? }`, the device only when it isn't this
    // one (the engine forwards it there).
    host.async_action("chat.refs", move |args| async move { refs(args).await.map_err(String::from) });
}

/// Where a subagent's uploaded transcript is: `{chat}/{doc}`, the chat it
/// was spawned in named by its doc (`{chat}--sub--{n}`; a copied spawn chip
/// keeps its first chat's), else the one it's shown under.
fn blob_ref(chat: Option<&str>, doc: &str) -> String {
    let source = doc.split_once("--sub--").map(|(source, _)| source).or(chat).unwrap_or(doc);
    format!("{source}/{doc}")
}

/// `FetchToolBlob`'s reply as a transcript's reset frame: its `text`, a
/// JSON list of entries.
fn snapshot_frame(reply: &Json) -> Option<Json> {
    let entries: Json = serde_json::from_str(reply.get("text")?.as_str()?).ok()?;
    entries.is_array().then(|| json!({ "reset": entries }))
}

/// A lane's view: its own states, heard as the window's open chat is (the
/// catalog, sessions and every connection reach it too).
fn lane_view(host: &ScriptHost, slot: &str) -> Rc<View> {
    let chats = CHATS.with(Rc::clone);
    let view = Rc::new(View {
        session: host.shared(&format!("chat.lane.{slot}.session"), Value::Nil),
        messages: host.shared(&format!("chat.lane.{slot}.messages"), Value::List(Rc::from([]))),
        subagents: host.shared(&format!("chat.lane.{slot}.subagents"), Value::List(Rc::from([]))),
        draft: host.shared(&format!("chat.lane.{slot}.draft"), draft_value("", "")),
        stats: host.shared(&format!("chat.lane.{slot}.stats"), Value::Nil),
        token_rate: host.shared(&format!("chat.lane.{slot}.token_rate"), Value::Nil),
        turn_tokens: host.shared(&format!("chat.lane.{slot}.turn_tokens"), Value::Nil),
        on_screen: Cell::new(false),
        rpc: RefCell::default(),
        open: RefCell::default(),
        pending: RefCell::default(),
        generation: Cell::new(0),
        snapshot: Cell::new(false),
        predicts: false,
    });
    chats.views.borrow_mut().push(Rc::downgrade(&view));
    let weak = Rc::downgrade(&view);
    engine::get().on_connect(move |rpc| {
        if let Some(view) = weak.upgrade() {
            *view.rpc.borrow_mut() = Some(rpc.clone());
            view.subscribe();
        }
    });
    view
}

/// A project's refs, as the new chat's ref picker lists them: each branch,
/// whether it's the folder's current one, and the worktree it's checked out
/// in. The current first, then as git lists them.
async fn refs(args: Value) -> Result<Value, Outcome> {
    let field = |key: &str| args.field(key).map(text).filter(|s| !s.is_empty());
    let path = field("path").ok_or("missing")?;
    let mut params = json!({ "repoPath": path });
    if let Some(device) = field("device") {
        params["targetDeviceId"] = Json::from(device);
    }
    let found = engine::get().call("ListRefs", params).await.map_err(|e| engine::outcome(&e))?;
    Ok(refs_value(&found))
}

fn refs_value(found: &Json) -> Value {
    let mut rows: Vec<&Json> = found.as_array().into_iter().flatten().filter(|r| r["name"].is_string()).collect();
    rows.sort_by_key(|r| r["current"].as_bool() != Some(true));
    Value::List(
        rows.into_iter()
            .map(|r| {
                let mut fields = vec![
                    ("name", Value::from(r["name"].as_str().unwrap_or_default())),
                    ("current", Value::Bool(r["current"].as_bool() == Some(true))),
                ];
                if let Some(path) = r["worktreePath"].as_str() {
                    fields.push(("worktree", Value::from(path)));
                }
                let (state, commits) = work_tree_state(&r["workTreeState"]);
                fields.push(("state", state.map_or(Value::Nil, Value::from)));
                fields.push(("commits", Value::Num(commits)));
                fields.push(("dirty", Value::Num(r["dirtyFiles"].as_f64().unwrap_or(0.))));
                Value::record(fields)
            })
            .collect::<Vec<_>>()
            .into(),
    )
}

fn work_tree_state(state: &Json) -> (Option<&str>, f64) {
    (state["kind"].as_str(), state["commits"].as_f64().unwrap_or(0.))
}

// -- every window's -----------------------------------------------------------

/// A frame's list, its items by `field`. Taken, not copied: these lists come whole
/// for every change in any chat, a chat's every field, and a copy made here would
/// be blocks by the thousand among the window's own, where they'd stay till the next.
fn by_field(list: Json, field: &str) -> HashMap<String, Json> {
    let Json::Array(list) = list else { return HashMap::new() };
    list.into_iter().filter_map(|item| Some((item[field].as_str()?.to_owned(), item))).collect()
}

impl Chats {
    fn connected(self: &Rc<Self>, rpc: &Rpc) {
        let me = self.clone();
        engine::get().spawn(async move { me.look_ahead().await });
        let mut watches = Vec::new();
        let weak = Rc::downgrade(self);
        watches.push(rpc.watch("WatchSessions", json!({}), move |item| {
            let Some(me) = weak.upgrade() else { return };
            *me.sessions.borrow_mut() = by_field(item, "chatId");
            me.follow_asks();
            me.publish_views();
            review::sessions_changed();
            // A turn starting or ending: the newest prompt's line follows.
            for view in me.views() {
                let changed = {
                    let mut slot = view.open.borrow_mut();
                    let Some(open) = slot.as_mut() else { continue };
                    let now = me.sessions.borrow().get(&open.id).cloned().unwrap_or(Json::Null);
                    let changed = now["status"] != open.session_seen["status"]
                        || now["startedAt"] != open.session_seen["startedAt"];
                    open.session_seen = now;
                    changed
                };
                if changed {
                    view.publish_messages();
                }
            }
        }));
        let weak = Rc::downgrade(self);
        watches.push(rpc.watch("WatchChats", json!({}), move |item| {
            let Some(me) = weak.upgrade() else { return };
            *me.chats.borrow_mut() = by_field(crate::sessions::with_parents(item), "id");
            me.chats_heard.set(true);
            review::chats_changed();
            for view in me.views() {
                view.watch_host();
            }
            me.publish_views();
            me.publish_waiting();
        }));
        *self.watches.borrow_mut() = watches;
        *self.rpc.borrow_mut() = Some(rpc.clone());
        self.ask_looks.borrow_mut().clear();
        let me = self.clone();
        let rpc = rpc.clone();
        engine::get().spawn(async move {
            if let Ok(answer) = rpc.call("LocalDevice", json!({})).await {
                let here = answer["deviceId"].as_str().map(String::from);
                // A composer that named this device before the answer came took it for another's.
                if here.is_some() && me.remote.borrow().as_ref().map(|r| &r.device) == here.as_ref() {
                    me.remote.borrow_mut().take();
                }
                *me.here.borrow_mut() = here;
            }
            if let Ok(info) = rpc.call("EngineInfo", json!({})).await {
                let list = info["capabilities"].as_array().into_iter().flatten().filter_map(Json::as_str).map(str::to_owned).collect();
                *me.engine_capabilities.borrow_mut() = list;
                me.publish_views();
            }
            // Every composer needs it: looked up once a connection is there.
            me.load_catalog(false).await;
        });
    }

    fn follow_rates(self: &Rc<Self>) {
        let views = self.views();
        let wanted: Vec<String> = views
            .iter()
            .filter(|v| !v.snapshot.get() && v.on_screen.get())
            .filter_map(|v| v.open.borrow().as_ref().filter(|o| o.host_watched).map(|o| o.id.clone()))
            .collect();
        self.rates.borrow_mut().retain(|id, _| wanted.contains(id));
        let rpc = self.rpc.borrow().clone().filter(|r| !r.is_closed());
        if let Some(rpc) = rpc {
            for id in wanted {
                if self.rates.borrow().contains_key(&id) {
                    continue;
                }
                let Some(chat) = self.chat(&id) else { continue };
                let mut params = json!({ "chatId": id });
                if let Some(t) = self.target(&chat) {
                    params["targetDeviceId"] = json!(t);
                }
                let (heard, ended) = (Rc::downgrade(self), Rc::downgrade(self));
                let (heard_id, ended_id) = (id.clone(), id.clone());
                let watch = rpc.watch_ending(
                    "WatchTokenRate",
                    params,
                    move |item| {
                        let Some(me) = heard.upgrade() else { return };
                        let rate = item["rate"].as_f64();
                        if let Some(row) = me.rates.borrow_mut().get_mut(&heard_id) {
                            row.rate = rate;
                        }
                        me.publish_rate(&heard_id, rate);
                    },
                    move |_| {
                        let Some(me) = ended.upgrade() else { return };
                        me.rates.borrow_mut().remove(&ended_id);
                        me.publish_rate(&ended_id, None);
                    },
                );
                self.rates.borrow_mut().insert(id, RateWatch { _watch: watch, rate: None });
            }
        }
        for view in views {
            let id = view.open_id();
            let rate = id.as_ref().and_then(|id| self.rates.borrow().get(id).and_then(|r| r.rate));
            view.token_rate.set(rate.map_or(Value::Nil, Value::Num));
        }
    }

    fn follow_machine(self: &Rc<Self>, id: String, device: String) {
        if !self.machine_polls.borrow_mut().insert(id.clone()) {
            return;
        }
        let weak = Rc::downgrade(self);
        engine::get().spawn(async move {
            zgx::timer::sleep(CLOUD_BOOT_WORD_AFTER).await;
            loop {
                let answer = engine::get().call("CloudSessions", json!({})).await.ok();
                let Some(chats) = weak.upgrade() else { return };
                let machine = answer.and_then(|a| machine_state(&a, &id, &device));
                let mut any_waits = false;
                for view in chats.views() {
                    let waits = {
                        let mut slot = view.open.borrow_mut();
                        let Some(open) = slot.as_mut().filter(|o| o.id == id) else { continue };
                        let waits = view.sending(&id);
                        open.machine = machine.filter(|_| waits);
                        waits
                    };
                    any_waits |= waits;
                    view.publish_session();
                }
                if !any_waits {
                    chats.machine_polls.borrow_mut().remove(&id);
                    return;
                }
                zgx::timer::sleep(CLOUD_BOOT_POLL).await;
            }
        });
    }

    fn publish_rate(&self, id: &str, rate: Option<f64>) {
        for view in self.views() {
            if view.open_id().as_deref() == Some(id) {
                view.token_rate.set(rate.map_or(Value::Nil, Value::Num));
            }
        }
    }

    /// Keeps `chat.asks` to the chats waiting on their person: a new one
    /// is looked at (its latest entries) to tell an approval from a
    /// question; one no longer waiting goes.
    fn follow_asks(self: &Rc<Self>) {
        let waiting: Vec<String> = {
            let sessions = self.sessions.borrow();
            sessions
                .iter()
                .filter(|(_, s)| s["status"].as_str() == Some("awaitingInput") && !stale(&s["updatedAt"]))
                .map(|(id, _)| id.clone())
                .collect()
        };
        let mut changed = false;
        self.asks.borrow_mut().retain(|id, _| {
            let keep = waiting.contains(id);
            changed |= !keep;
            keep
        });
        self.ask_looks.borrow_mut().retain(|id, _| waiting.contains(id));
        self.ask_tries.borrow_mut().retain(|id, _| waiting.contains(id));
        let before = self.waiting.borrow().len();
        self.waiting.borrow_mut().retain(|id, _| waiting.contains(id));
        self.pages.borrow_mut().retain(|id, _| waiting.contains(id));
        let waiting_gone = self.waiting.borrow().len() != before;
        for id in waiting {
            if self.asks.borrow().contains_key(&id) || self.ask_looks.borrow().contains_key(&id) {
                continue;
            }
            // The open chat's own transcript says it at once.
            let open = self.views().iter().find_map(|v| {
                let open = v.open.borrow();
                let open = open.as_ref().filter(|o| o.id == id && o.heard)?;
                let newest = || open.entries.iter().rev().filter_map(|e| e.json.as_ref());
                self.land_ask(&id, newest());
                Some(ask_kind(newest()))
            });
            if let Some(kind) = open {
                self.asks.borrow_mut().insert(id, kind);
                changed = true;
                continue;
            }
            self.look_at_ask(id);
        }
        if changed {
            self.publish_asks();
        }
        if waiting_gone {
            self.publish_waiting();
        }
    }

    fn look_at_ask(self: &Rc<Self>, id: String) {
        let Some(rpc) = self.rpc.borrow().clone().filter(|r| !r.is_closed()) else { return };
        let weak = Rc::downgrade(self);
        let chat = id.clone();
        let look = rpc.watch(
            "WatchDocMessages",
            json!({ "chatId": id, "openingTail": true }),
            move |item| {
                let Some(me) = weak.upgrade() else { return };
                let Some(entries) = item["reset"].as_array() else { return };
                let kind = ask_kind(entries.iter().rev());
                me.asks.borrow_mut().insert(chat.clone(), kind);
                let done = me.ask_looks.borrow_mut().remove(&chat);
                me.publish_asks();
                me.land_ask(&chat, entries.iter().rev());
                drop(done);
            },
        );
        self.ask_looks.borrow_mut().insert(id, look);
    }

    fn asks_value(&self) -> Value {
        Value::record(self.asks.borrow().iter().map(|(id, kind)| (id.as_str(), Value::from(*kind))))
    }

    fn publish_asks(&self) {
        let value = self.asks_value();
        for shared in self.asks_shared.borrow().iter() {
            shared.set(value.clone());
        }
    }

    fn views(&self) -> Vec<Rc<View>> {
        self.views.borrow().iter().filter_map(Weak::upgrade).collect()
    }

    /// Keeps what a view's chat had for a moment (`LET_GO_KEPT`), for another
    /// view that's about to open it.
    fn keep_let_go(self: &Rc<Self>, old: Option<Open>) {
        // (Not while its rows are on a worker: it has only the ones they left.)
        let Some(old) = old.filter(|o| o.heard && !o.stepping) else { return };
        let Open { id, entries, older_todo, built, built_model, context_usage, stats, .. } = old;
        *self.let_go.borrow_mut() = Some(LetGo { id, entries, older_todo, built, built_model, context_usage, stats });
        let turn = self.let_go_turn.get() + 1;
        self.let_go_turn.set(turn);
        let me = self.clone();
        engine::get().spawn(async move {
            zgx::timer::sleep(LET_GO_KEPT).await;
            if me.let_go_turn.get() == turn {
                me.let_go.borrow_mut().take();
            }
        });
    }

    /// What a view let go of just now, if it was this chat: an `Open` that
    /// has its rows.
    fn take_let_go(&self, id: &str) -> Option<Open> {
        let mut slot = self.let_go.borrow_mut();
        if slot.as_ref()?.id != id {
            return None;
        }
        let held = slot.take()?;
        Some(Open {
            id: held.id,
            entries: held.entries,
            older_todo: held.older_todo,
            heard: true,
            context_usage: held.context_usage,
            stats: held.stats,
            stats_seen: held.stats,
            built: held.built,
            built_model: held.built_model,
            has_whole: true,
            ..Open::default()
        })
    }

    fn shells_stoppable(&self, chat: &Json) -> bool {
        let capability = zeron_proto::capabilities::BACKGROUND_TASKS_V1;
        let device = chat["deviceId"].as_str();
        self.engine_capabilities.borrow().iter().any(|c| c == capability)
            && (device.is_none() || device == self.here.borrow().as_deref() || device.is_some_and(|d| crate::sessions::device_supports(d, capability)))
    }

    fn publish_views(&self) {
        for view in self.views() {
            view.publish_session();
        }
    }

    /// The agents that can take a chat, and each one's models.
    async fn load_catalog(self: &Rc<Self>, again: bool) {
        if self.catalog_state.get() == CatalogState::Loading {
            return;
        }
        if self.catalog_state.get() == CatalogState::Ready && !again {
            return;
        }
        self.catalog_state.set(CatalogState::Loading);
        let mock = std::env::var("ZERON_ZGX_MOCK").is_ok_and(|v| v == "1");
        let list = match engine::get().call("ListHarnesses", json!({ "includeRegistry": true })).await {
            Ok(list) => list,
            Err(error) => {
                eprintln!("zeron-zgx: chat: the agents didn't list: {error}");
                self.catalog_state.set(CatalogState::Looking);
                return;
            }
        };
        let harnesses: Vec<Json> = list
            .as_array()
            .into_iter()
            .flatten()
            .filter(|h| {
                let id = h["id"].as_str().unwrap_or_default();
                if id == "mock" {
                    return mock;
                }
                h["installed"].as_bool() != Some(false) && h["enabled"].as_bool() != Some(false)
            })
            .cloned()
            .collect();
        // Each agent's models looked up at once, each shown as it comes (one
        // that has to start its agent takes seconds; one signed out fails).
        let ids: Vec<String> = harnesses
            .iter()
            .filter_map(|h| h["id"].as_str().map(Into::into))
            .collect();
        let seeded = if mock { mock_catalog() } else { None };
        let mut harnesses = harnesses;
        let mut ids = ids;
        if let Some(seed) = &seeded {
            for h in seed["harnesses"].as_array().into_iter().flatten() {
                let Some(id) = h["id"].as_str() else { continue };
                ids.retain(|x| x != id);
                harnesses.retain(|x| x["id"].as_str() != Some(id));
                harnesses.push(h.clone());
            }
        }
        crate::agents::remember_icons(&harnesses);
        *self.harnesses.borrow_mut() = harnesses;
        for id in &ids {
            let (me, id) = (self.clone(), id.clone());
            engine::get().spawn(async move { me.probe_compaction(&id, None).await });
        }
        if !again {
            self.models.borrow_mut().retain(|id, _| ids.contains(id));
        }
        if let Some(seed) = &seeded {
            for (id, list) in seed["models"].as_object().into_iter().flatten() {
                let reply = json!({ "models": list, "newIds": seed["newIds"], "announceIds": seed["announceIds"][id] });
                self.announce(id, &reply);
                self.models.borrow_mut().insert(id.clone(), listed_models(&reply));
            }
            for (id, error) in seed["errors"].as_object().into_iter().flatten() {
                self.model_errors.borrow_mut().insert(id.clone(), error.as_str().unwrap_or_default().to_owned());
            }
        }
        let left = Rc::new(Cell::new(ids.len()));
        if ids.is_empty() {
            self.catalog_state.set(CatalogState::Ready);
            self.publish_catalog();
        }
        for id in ids {
            let (me, left) = (self.clone(), left.clone());
            engine::get().spawn(async move {
                match engine::get()
                    .call("ListModels", json!({ "harness": id, "force": again, "announce": false }))
                    .await
                {
                    Ok(list) => {
                        me.model_errors.borrow_mut().remove(&id);
                        me.announce(&id, &list);
                        me.models
                            .borrow_mut()
                            .insert(id, listed_models(&list));
                    }
                    Err(error) => {
                        eprintln!("zeron-zgx: chat: {id}'s models didn't list: {error}");
                        me.model_errors.borrow_mut().insert(id, error.to_string());
                    }
                }
                left.set(left.get() - 1);
                if left.get() == 0 {
                    me.catalog_state.set(CatalogState::Ready);
                }
                me.publish_catalog();
            });
        }
    }

    /// Keeps what `reply` names new for the first time (the engine's
    /// `announceIds`, given once across every window), as Zeron's toast says it:
    /// the agent's name and the models' names.
    fn announce(&self, harness: &str, reply: &Json) {
        let Some(ids) = reply["announceIds"].as_array().filter(|ids| !ids.is_empty()) else { return };
        let models = reply["models"].as_array().or(reply.as_array()).cloned().unwrap_or_default();
        let labels = ids
            .iter()
            .filter_map(|id| models.iter().find(|m| &m["id"] == id).and_then(|m| m["label"].as_str()).or(id.as_str()))
            .map(str::to_owned)
            .collect();
        let agent = self
            .harnesses
            .borrow()
            .iter()
            .find(|h| h["id"].as_str() == Some(harness))
            .and_then(|h| h["name"].as_str().map(str::to_owned))
            .unwrap_or_else(|| harness.to_owned());
        let mut announced = self.announced.borrow_mut();
        let seq = announced.last().map_or(1, |(n, _, _)| n + 1);
        announced.push((seq, agent, labels));
        let extra = announced.len().saturating_sub(8);
        announced.drain(..extra);
    }

    fn publish_catalog(&self) {
        let value = self.catalog_value();
        for shared in self.catalog.borrow().iter() {
            shared.set(value.clone());
        }
        self.publish_views();
    }

    /// The composer now runs on `device` ("" or this one: here).
    fn catalog_on(self: &Rc<Self>, device: &str) {
        let here = self.here.borrow().clone();
        if device.is_empty() || here.as_deref() == Some(device) {
            if self.remote.borrow_mut().take().is_some() {
                self.publish_catalog();
            }
            return;
        }
        if self.remote.borrow().as_ref().is_some_and(|r| r.device == device) {
            return;
        }
        let known = self.known_remotes.borrow().get(device).cloned();
        *self.remote.borrow_mut() = Some(known.unwrap_or_else(|| Remote {
            device: device.to_owned(),
            harnesses: Vec::new(),
            models: HashMap::new(),
            ready: false,
        }));
        self.publish_catalog();
        let (me, device) = (self.clone(), device.to_owned());
        engine::get().spawn(async move { me.load_remote(device).await });
    }

    /// `device`'s agents, then each one's models as they come, while the
    /// composer is still on it.
    async fn load_remote(self: &Rc<Self>, device: String) {
        let still = |me: &Chats| me.remote.borrow().as_ref().is_some_and(|r| r.device == device);
        let list = engine::get()
            .call("ListHarnesses", json!({ "targetDeviceId": device, "includeRegistry": true }))
            .await;
        if !still(self) {
            return;
        }
        let harnesses: Vec<Json> = match list {
            Ok(list) => list
                .as_array()
                .into_iter()
                .flatten()
                .filter(|h| {
                    h["id"].as_str() != Some("mock")
                        && h["installed"].as_bool() != Some(false)
                        && h["enabled"].as_bool() != Some(false)
                })
                .cloned()
                .collect(),
            Err(error) => {
                // (Unreachable: this device's catalog stands in, as before.)
                eprintln!("zeron-zgx: chat: {device}'s agents didn't list: {error}");
                if !self.known_remotes.borrow().contains_key(&device) {
                    self.remote.borrow_mut().take();
                    self.publish_catalog();
                }
                return;
            }
        };
        let ids: Vec<String> = harnesses.iter().filter_map(|h| h["id"].as_str().map(Into::into)).collect();
        if let Some(r) = self.remote.borrow_mut().as_mut() {
            crate::agents::remember_icons(&harnesses);
            r.harnesses = harnesses;
            r.ready = ids.is_empty();
        }
        for id in &ids {
            let (me, id, device) = (self.clone(), id.clone(), device.clone());
            engine::get().spawn(async move { me.probe_compaction(&id, Some(&device)).await });
        }
        self.publish_catalog();
        let left = Rc::new(Cell::new(ids.len()));
        for id in ids {
            let (me, left, device) = (self.clone(), left.clone(), device.clone());
            engine::get().spawn(async move {
                let list = engine::get()
                    .call("ListModels", json!({ "harness": id, "targetDeviceId": device, "announce": false }))
                    .await;
                left.set(left.get() - 1);
                {
                    let mut remote = me.remote.borrow_mut();
                    let Some(r) = remote.as_mut().filter(|r| r.device == device) else {
                        return;
                    };
                    match list {
                        Ok(list) => {
                            r.models.insert(id, listed_models(&list));
                        }
                        Err(error) => eprintln!("zeron-zgx: chat: {id}'s models on {device} didn't list: {error}"),
                    }
                    r.ready = left.get() == 0;
                    if r.ready {
                        me.known_remotes.borrow_mut().insert(device.clone(), r.clone());
                    }
                }
                me.publish_catalog();
            });
        }
    }

    // The engine resolves this without an instance, so `acp` would always fail and be asked again.
    async fn probe_compaction(self: &Rc<Self>, harness: &str, device: Option<&str>) {
        let Ok(id) = serde_json::from_value::<zeron_proto::HarnessId>(json!(harness)) else { return };
        if id == zeron_proto::HarnessId::Acp || !self.compaction_probed.borrow_mut().insert(harness.to_owned()) {
            return;
        }
        let mut params = json!({ "harness": harness });
        if let Some(device) = device {
            params["targetDeviceId"] = json!(device);
        }
        let answer = engine::get()
            .call("HarnessCompaction", params)
            .await
            .ok()
            .and_then(|a| serde_json::from_value::<zeron_proto::HarnessCompactionResult>(a).ok());
        let Some(answer) = answer else {
            self.compaction_probed.borrow_mut().remove(harness);
            return;
        };
        if answer.compaction == zeron_proto::CompactionKind::None {
            let reason = zeron_proto::command_catalog::compaction_unsupported(id);
            self.compaction_unsupported.borrow_mut().insert(harness.to_owned(), reason);
            self.publish_catalog();
        }
    }

    /// The other devices' catalogs looked at ahead of time, one at a time so
    /// launch doesn't feel it, so a chat on one opens with its model known.
    async fn look_ahead(self: &Rc<Self>) {
        zgx::timer::sleep(LOOK_AHEAD_AFTER).await;
        for device in crate::sessions::other_devices_online() {
            if self.known_remotes.borrow().contains_key(&device) {
                continue;
            }
            let params = json!({ "targetDeviceId": device, "includeRegistry": true });
            let Ok(list) = engine::get().call("ListHarnesses", params).await else { continue };
            let harnesses: Vec<Json> = list
                .as_array()
                .into_iter()
                .flatten()
                .filter(|h| {
                    h["id"].as_str() != Some("mock")
                        && h["installed"].as_bool() != Some(false)
                        && h["enabled"].as_bool() != Some(false)
                })
                .cloned()
                .collect();
            let mut models = HashMap::new();
            for id in harnesses.iter().filter_map(|h| h["id"].as_str()) {
                let params = json!({ "harness": id, "targetDeviceId": device });
                if let Ok(list) = engine::get().call("ListModels", params).await {
                    models.insert(id.to_owned(), list.as_array().cloned().unwrap_or_default());
                }
            }
            crate::agents::remember_icons(&harnesses);
            // (One opened meanwhile has its own, as new.)
            self.known_remotes
                .borrow_mut()
                .entry(device.clone())
                .or_insert(Remote { device, harnesses, models, ready: true });
        }
    }

    fn catalog_value(&self) -> Value {
        let unsupported = self.compaction_unsupported.borrow();
        if let Some(r) = self.remote.borrow().as_ref() {
            return catalog_of(&r.harnesses, &r.models, r.ready, Some(&r.device), &unsupported, &HashMap::new(), &[]);
        }
        catalog_of(
            &self.harnesses.borrow(),
            &self.models.borrow(),
            self.catalog_state.get() == CatalogState::Ready,
            None,
            &unsupported,
            &self.model_errors.borrow(),
            &self.announced.borrow(),
        )
    }

    fn chat(&self, id: &str) -> Option<Json> {
        self.chats.borrow().get(id).cloned()
    }

    /// A chat this window just made (`sessions.create`), known before the
    /// engine's list says so: its first message, sent at once, goes where
    /// and how it was made (a side chat in its parent's folder, with its
    /// parent's agent and model). The engine's next list replaces it.
    fn made(&self, id: &str, row: Json) {
        self.chats.borrow_mut().entry(id.to_owned()).or_insert(row);
    }

    /// Whether the chat's turn runs: working or waiting on an answer, and
    /// heard from lately.
    fn running(&self, id: &str) -> bool {
        let sessions = self.sessions.borrow();
        let Some(s) = sessions.get(id) else { return false };
        let live = matches!(s["status"].as_str(), Some("working" | "awaitingInput"));
        live && !stale(&s["updatedAt"])
    }

    /// gpui's `turn_in_flight`: a parked turn, held only by its subagents, is not one.
    fn turn_in_flight(&self, open: &Open, sending: bool) -> bool {
        let held = self.sessions.borrow().get(&open.id).is_some_and(|s| s["heldBySubagents"].as_bool() == Some(true));
        sending || (self.running(&open.id) && !held)
    }

    fn chat_compacting(&self, id: &str) -> bool {
        self.compacting.borrow().contains(id)
            || self.sessions.borrow().get(id).is_some_and(|s| s["compacting"].as_bool() == Some(true))
    }

    fn label_of(&self, harness: &str, model: &str) -> Option<String> {
        let find = |by: &HashMap<String, Vec<Json>>| {
            by.get(harness)?
                .iter()
                .find(|m| m["id"].as_str() == Some(model))
                .and_then(|m| m["label"].as_str().map(Into::into))
        };
        // (This device's, else the composer's other device's.)
        find(&self.models.borrow()).or_else(|| self.remote.borrow().as_ref().and_then(|r| find(&r.models)))
    }

    fn steers_mid_turn(&self, harness: &str, device: Option<&str>) -> Option<bool> {
        let read = |list: &[Json]| list.iter().find(|h| h["id"].as_str() == Some(harness)).map(steers_of);
        let there = || self.remote.borrow().as_ref().filter(|r| Some(r.device.as_str()) == device).and_then(|r| read(&r.harnesses));
        there().or_else(|| read(&self.harnesses.borrow()))
    }

    fn offers_plan(&self, harness: &str, device: Option<&str>) -> Option<bool> {
        let read = |list: &[Json]| list.iter().find(|h| h["id"].as_str() == Some(harness)).map(|h| h["supportsPlan"].as_bool() == Some(true));
        let there = || self.remote.borrow().as_ref().filter(|r| Some(r.device.as_str()) == device).and_then(|r| read(&r.harnesses));
        there().or_else(|| read(&self.harnesses.borrow()))
    }

    fn harness_name(&self, harness: &str) -> Option<String> {
        let read = |list: &[Json]| list.iter().find(|h| h["id"].as_str() == Some(harness)).and_then(|h| h["name"].as_str().map(Into::into));
        read(&self.harnesses.borrow()).or_else(|| self.remote.borrow().as_ref().and_then(|r| read(&r.harnesses)))
    }

    /// The device to name for a call routed to the chat's host, when that
    /// isn't this one.
    fn target(&self, chat: &Json) -> Option<String> {
        let device = chat["deviceId"].as_str()?;
        (self.here.borrow().as_deref() != Some(device)).then(|| device.to_owned())
    }
}

/// A chat just made here (`sessions.create`), as the engine's list will
/// have it: `params` is its `createChat` (`chatId`, `deviceId`, `spaceId`,
/// `config`, `cwd`, `branch`, `parentChatId`).
pub(crate) fn made(params: &Json) {
    let Some(id) = params["chatId"].as_str() else { return };
    let mut row = json!({ "id": id });
    for wire in ["deviceId", "spaceId", "config", "cwd", "branch", "parentChatId"] {
        if !params[wire].is_null() {
            row[wire] = params[wire].clone();
        }
    }
    CHATS.with(|c| c.made(id, row));
}

pub(crate) async fn compaction_unsupported(harness: &str, device: Option<&str>) -> Option<String> {
    let chats = CHATS.with(Rc::clone);
    chats.probe_compaction(harness, device).await;
    chats.compaction_unsupported.borrow().get(harness).cloned()
}

/// `chat.catalog`: the agents with models listed, each model as the picker
/// takes it; ready once any agent's models are in (more may follow), or all
/// have answered (`done`). (`device`: whose, when it isn't this one.)
// The fork's engine answers a catalog (`{models, newIds, announceIds}`), upstream's a bare list.
fn listed_models(reply: &Json) -> Vec<Json> {
    let mut models = reply["models"].as_array().or(reply.as_array()).cloned().unwrap_or_default();
    if let Some(new) = reply["newIds"].as_array() {
        for m in &mut models {
            if new.contains(&m["id"]) {
                m["isNew"] = json!(true);
            }
        }
    }
    models
}

/// Every agent the picker has a tab for, in the catalog's order, those with no
/// models yet saying why: still coming, or what stopped them.
fn tabs_of(listed: &[Json], by: &HashMap<String, Vec<Json>>, errors: &HashMap<String, String>, done: bool) -> Value {
    let tabs: Vec<Value> = listed
        .iter()
        .map(|h| {
            let id = h["id"].as_str().unwrap_or_default();
            let error = errors.get(id);
            let state = if by.get(id).is_some_and(|l| !l.is_empty()) {
                "ready"
            } else if error.is_some() {
                "error"
            } else if done {
                "ready"
            } else {
                "loading"
            };
            Value::record([
                ("id", Value::from(id)),
                ("label", Value::from(h["name"].as_str().unwrap_or(id))),
                ("mark", Value::from(mark(id).as_str())),
                ("icon", h["icon"].as_str().map_or(Value::Nil, Value::from)),
                ("state", Value::from(state)),
                ("error", error.map_or(Value::Nil, |e| Value::from(e.as_str()))),
            ])
        })
        .collect();
    Value::List(tabs.into())
}

/// A dev catalog for the picker's proofs, from `ZERON_ZGX_MOCK_CATALOG` (a JSON
/// file: `harnesses` as the engine lists them, `models`, `errors` and
/// `announceIds` by agent, `newIds`; `profiles`, saved once, and
/// `profilesError`, src/agent_profiles.rs) on a mock Dev. Its agents are
/// listed, never run.
fn mock_catalog() -> Option<Json> {
    let path = std::env::var("ZERON_ZGX_MOCK_CATALOG").ok()?;
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn steers_of(harness: &Json) -> bool {
    harness["supportsSteering"].as_bool() == Some(true) && harness["steeringMode"].as_str() == Some("step-boundary")
}

fn catalog_of(
    listed: &[Json],
    by: &HashMap<String, Vec<Json>>,
    done: bool,
    device: Option<&str>,
    unsupported: &HashMap<String, String>,
    errors: &HashMap<String, String>,
    announced: &[(u32, String, Vec<String>)],
) -> Value {
    let any = by.values().any(|l| !l.is_empty());
    let state = if any || done { "ready" } else { "looking" };
    let mut harnesses = Vec::new();
    let mut models = Vec::new();
    for h in listed {
        let id = h["id"].as_str().unwrap_or_default();
        let Some(list) = by.get(id).filter(|l| !l.is_empty()) else {
            continue;
        };
        harnesses.push(Value::record([
            ("id", Value::from(id)),
            ("label", Value::from(h["name"].as_str().unwrap_or(id))),
            ("mark", Value::from(mark(id).as_str())),
            // An agent added from the ACP registry draws its own icon.
            ("icon", h["icon"].as_str().map_or(Value::Nil, Value::from)),
            ("access_modes", access_modes_value(&h["accessModes"])),
            ("supports_plan", Value::Bool(h["supportsPlan"].as_bool() == Some(true))),
            ("plan_asks", Value::Bool(matches!(id, "claude-code" | "mock"))),
            ("steers", Value::Bool(steers_of(h))),
            ("compaction_unsupported", unsupported.get(id).map_or(Value::Nil, |r| Value::from(r.as_str()))),
        ]));
        for m in list {
            models.push(model_value(id, m));
        }
    }
    let mut fields = vec![
        ("state", Value::from(state)),
        // (Every agent's models in: till then, a model may just not be here yet.)
        ("complete", Value::Bool(done)),
        ("harnesses", Value::List(harnesses.into())),
        ("models", Value::List(models.into())),
        ("tabs", tabs_of(listed, by, errors, done)),
        (
            "announced",
            Value::List(
                announced
                    .iter()
                    .map(|(seq, agent, labels)| {
                        let models: Vec<Value> = labels.iter().map(|l| Value::from(l.as_str())).collect();
                        Value::record([("seq", Value::Num(f64::from(*seq))), ("agent", Value::from(agent.as_str())), ("models", Value::List(models.into()))])
                    })
                    .collect::<Vec<_>>()
                    .into(),
            ),
        ),
    ];
    if let Some(device) = device {
        fields.push(("device", Value::from(device)));
    }
    Value::record(fields)
}

/// The model picker's entry for a model: its value "<harness>/<id>", its
/// reasoning (low to high) and where it starts, whether it has Fast, and
/// its other options (Devin Fusion's lead and sidekick): each its id, its
/// name, its choices (id and name) and the one it starts on.
/// The engine leaves an empty list out: such a harness offers full access alone.
fn access_modes_value(modes: &Json) -> Value {
    let named = modes.as_array().into_iter().flatten().filter_map(|m| match m.as_str()? {
        "supervised" => Some("supervised"),
        "auto-accept-edits" => Some("edits"),
        "full-access" => Some("full"),
        _ => None,
    });
    Value::List(named.map(Value::from).collect::<Vec<_>>().into())
}

fn model_value(harness: &str, m: &Json) -> Value {
    let id = m["id"].as_str().unwrap_or_default();
    let levels: Vec<&str> = m["reasoningLevels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Json::as_str)
        .collect();
    let efforts: Vec<Value> = levels
        .iter()
        .map(|l| Value::record([("value", Value::from(*l)), ("label", Value::from(effort_label(l)))]))
        .collect();
    let start = ["high", "medium"]
        .into_iter()
        .find(|l| levels.contains(l))
        .or(levels.first().copied());
    let fast = fast_option(m);
    let options: Vec<Value> = m["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| {
            // (Fast is `fast`, not an option of its own.)
            let id = o["id"].as_str().unwrap_or_default();
            !id.is_empty() && fast.as_ref().is_none_or(|(key, _, _)| key != id)
        })
        .filter_map(|o| {
            let id = o["id"].as_str()?;
            let choices: Vec<Value> = o["choices"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| {
                    let cid = c["id"].as_str()?;
                    Some(Value::record([
                        ("id", Value::from(cid)),
                        ("label", Value::from(c["label"].as_str().unwrap_or(cid))),
                    ]))
                })
                .collect();
            if choices.is_empty() {
                return None;
            }
            let default = o["defaultChoice"]
                .as_str()
                .filter(|d| o["choices"].as_array().is_some_and(|l| l.iter().any(|c| c["id"].as_str() == Some(d))))
                .map(str::to_owned)
                .or_else(|| o["choices"][0]["id"].as_str().map(str::to_owned))?;
            Some(Value::record([
                ("id", Value::from(id)),
                ("label", Value::from(o["label"].as_str().unwrap_or(id))),
                ("default", Value::from(default.as_str())),
                ("choices", Value::List(choices.into())),
            ]))
        })
        .collect();
    let mut fields = vec![
        ("value", Value::from(format!("{harness}/{id}").as_str())),
        ("label", Value::from(m["label"].as_str().unwrap_or(id))),
        ("harness", Value::from(harness)),
        ("fast", Value::Bool(fast.is_some())),
    ];
    if let Some(description) = m["description"].as_str().filter(|d| !d.trim().is_empty()) {
        fields.push(("description", Value::from(description)));
    }
    if m["isNew"].as_bool() == Some(true) {
        fields.push(("new", Value::Bool(true)));
    }
    if !efforts.is_empty() {
        fields.push(("efforts", Value::List(efforts.into())));
    }
    if let Some(start) = start {
        fields.push(("effort", Value::from(start)));
    }
    if !options.is_empty() {
        fields.push(("options", Value::List(options.into())));
    }
    Value::record(fields)
}

/// A model's Fast option: its id, the choice that turns it on, and the one
/// that turns it off (the shipping app's `fast_mode_values`: Claude's
/// `fastMode` on/off, Codex's `serviceTier` fast/default, Devin's `speed`
/// fast/standard).
fn fast_option(m: &Json) -> Option<(String, String, String)> {
    m["options"].as_array()?.iter().find_map(|o| {
        let id = o["id"].as_str()?;
        let choices: Vec<&str> = o["choices"].as_array()?.iter().filter_map(|c| c["id"].as_str()).collect();
        let has = |c: &str| choices.contains(&c);
        let on = if matches!(id, "fastMode" | "fast_mode") && has("on") {
            "on"
        } else if id == "fast" && has("true") {
            "true"
        } else if has("fast") {
            "fast"
        } else {
            return None;
        };
        let default = o["defaultChoice"].as_str().filter(|d| *d != on && has(d));
        let off = default.or_else(|| choices.iter().copied().find(|c| *c != on))?;
        Some((id.to_owned(), on.to_owned(), off.to_owned()))
    })
}

/// Whether a chat's model options have Fast on.
fn fast_in(options: &Json) -> bool {
    options["fastMode"].as_str() == Some("on")
        || options["fast_mode"].as_str() == Some("on")
        || options["fast"].as_str() == Some("true")
        || options["serviceTier"].as_str() == Some("fast")
        || options["speed"].as_str() == Some("fast")
}

/// A chat's model options as the composer's picks: each option's chosen
/// choice, by id (strings only; Fast's is `fast`, read by `fast_in`).
fn options_value(options: &Json) -> Value {
    let fields: Vec<(&str, Value)> = options
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.as_str(), Value::from(s))))
        .collect();
    Value::record(fields)
}

/// A provider switch's transcript divider, as gpui's `provider_switch_prompt`.
fn provider_note(from: &str, to: &str) -> Json {
    json!({
        "text": format!("Provider changed from {from} to {to}."),
        "hint": format!("{from} session closed. The messages above were handed to {to} as text on the next turn; {from}'s tool results and file state were not carried."),
    })
}

fn effort_label(level: &str) -> &str {
    match level {
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra high",
        "max" => "Max",
        "ultra" => "Ultra",
        "ultracode" => "Ultracode",
        "ultrathink" => "Ultrathink",
        other => other,
    }
}

/// ds.Mark's name for a harness. The mock agent (development only) borrows
/// Pi's.
fn mark(id: &str) -> String {
    crate::agents::glyph(id).unwrap_or_else(|| "pi".into())
}

// -- a window's open chat -----------------------------------------------------

impl View {
    fn open_chat(self: &Rc<Self>, id: Option<String>) {
        let same = self.open.borrow().as_ref().map(|o| o.id.clone()) == id;
        if same {
            return;
        }
        let chats = CHATS.with(Rc::clone);
        let was_snapshot = self.snapshot.replace(false);
        self.generation.set(self.generation.get() + 1);
        let next = id.map(|id| chats.take_let_go(&id).unwrap_or(Open { id, ..Open::default() }));
        let has_rows = next.as_ref().is_some_and(|o| o.heard);
        let old = self.open.replace(next);
        if !was_snapshot {
            chats.keep_let_go(old);
        }
        if has_rows {
            self.publish_messages();
        } else {
            self.messages.set(Value::List(Rc::from([])));
            self.subagents.set(Value::List(Rc::from([])));
        }
        self.draft.set(draft_value("", ""));
        self.subscribe();
        self.publish_messages();
        self.publish_session();
        self.publish_stats();
        self.publish_turn_tokens();
        chats.follow_rates();
        if let Some(id) = self.open_id() {
            // Its finished state seen, as the shipping app marks it.
            engine::get().spawn(async move {
                let _ = engine::get().call("Mutate", json!({ "op": "markChatSeen", "chatId": id })).await;
            });
        }
    }

    /// A finished subagent's doc `id`, under `chat`: its uploaded transcript
    /// first (the shipping app's `spawn_subagent_snapshot_fetch`), its live
    /// doc if that can't be had.
    fn open_frozen(self: &Rc<Self>, id: String, chat: Option<String>) {
        if self.open_id().as_deref() == Some(id.as_str()) {
            return;
        }
        self.generation.set(self.generation.get() + 1);
        self.snapshot.set(true);
        *self.open.borrow_mut() = Some(Open { id: id.clone(), ..Open::default() });
        self.messages.set(Value::List(Rc::from([])));
        self.subagents.set(Value::List(Rc::from([])));
        self.draft.set(draft_value("", ""));
        self.publish_session();
        self.publish_stats();
        self.publish_turn_tokens();
        CHATS.with(Rc::clone).follow_rates();
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        let blob = blob_ref(chat.as_deref(), &id);
        engine::get().spawn(async move {
            let reply = engine::get().call("FetchToolBlob", json!({ "blobRef": blob })).await;
            let Some(view) = weak.upgrade() else { return };
            if view.generation.get() != generation {
                return;
            }
            match reply.ok().as_ref().and_then(snapshot_frame) {
                Some(frame) => view.heard_frame(frame),
                // (Not uploaded, or not readable: its live doc.)
                None => {
                    view.snapshot.set(false);
                    view.subscribe();
                }
            }
        });
    }

    fn open_id(&self) -> Option<String> {
        self.open.borrow().as_ref().map(|o| o.id.clone())
    }

    fn sending(&self, id: &str) -> bool {
        self.pending.borrow().get(id).is_some_and(|p| p.iter().any(|p| p.waiting == "sending"))
    }

    /// The open chat's watches, on the connection there is (again on each
    /// new one).
    fn subscribe(self: &Rc<Self>) {
        if self.snapshot.get() {
            return;
        }
        let Some(rpc) = self.rpc.borrow().clone().filter(|r| !r.is_closed()) else {
            return;
        };
        let Some(id) = self.open_id() else { return };
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        let transcript = rpc.watch(
            "WatchDocMessages",
            json!({ "chatId": id, "openingTail": true }),
            move |item| {
                if let Some(view) = weak.upgrade()
                    && view.generation.get() == generation
                {
                    view.heard_frame(item);
                }
            },
        );
        let weak = Rc::downgrade(self);
        let queue = rpc.watch("WatchQueue", json!({ "chatId": id }), move |item| {
            if let Some(view) = weak.upgrade()
                && view.generation.get() == generation
            {
                if let Some(open) = view.open.borrow_mut().as_mut() {
                    open.queue = item["items"].as_array().cloned().unwrap_or_default();
                }
                view.publish_session();
            }
        });
        if let Some(open) = self.open.borrow_mut().as_mut() {
            open.watches = vec![transcript, queue];
            open.host_watched = false;
        }
        self.watch_host();
    }

    // Waits for the chat's row to name its host; a stream an old engine refuses just ends, unheard.
    fn watch_host(self: &Rc<Self>) {
        if self.snapshot.get() {
            return;
        }
        let Some(rpc) = self.rpc.borrow().clone().filter(|r| !r.is_closed()) else {
            return;
        };
        let Some(id) = self.open.borrow().as_ref().filter(|o| !o.host_watched).map(|o| o.id.clone()) else {
            return;
        };
        let chats = CHATS.with(Rc::clone);
        let Some(chat) = chats.chat(&id) else { return };
        let routed = |mut p: Json| {
            if let Some(t) = chats.target(&chat) {
                p["targetDeviceId"] = json!(t);
            }
            p
        };
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        let chat_id = id.clone();
        let limit = rpc.watch("WatchLimitResumes", routed(json!({})), move |item| {
            if let Some(view) = weak.upgrade()
                && view.generation.get() == generation
            {
                if let Some(open) = view.open.borrow_mut().as_mut() {
                    open.limit = limit_row(&item, &chat_id);
                    open.limits = offered_limits(&item, &chat_id);
                }
                view.publish_session();
            }
        });
        let mut watches = vec![limit];
        if self.predicts {
            let weak = Rc::downgrade(self);
            watches.push(rpc.watch("WatchComposerPrediction", routed(json!({ "chatId": id })), move |item| {
                if let Some(view) = weak.upgrade()
                    && view.generation.get() == generation
                {
                    if let Some(open) = view.open.borrow_mut().as_mut() {
                        open.prediction = item["prediction"]["text"].as_str().map(str::to_owned);
                    }
                    view.publish_session();
                }
            }));
        }
        if let Some(open) = self.open.borrow_mut().as_mut() {
            open.watches.extend(watches);
            open.host_watched = true;
        }
        chats.follow_rates();
    }

    fn consume_prediction(&self, args: &Value) {
        let id = args.field("id").map(text).unwrap_or_default();
        let dropped = self.open.borrow_mut().as_mut().filter(|o| o.id == id).is_some_and(|o| o.prediction.take().is_some());
        if dropped {
            self.publish_session();
        }
    }

    /// Watches the open chat again from a reset (its copy went out of step).
    fn resubscribe(self: &Rc<Self>) {
        if let Some(open) = self.open.borrow_mut().as_mut() {
            open.watches.clear();
            open.folding = None;
            open.stepping = false;
            open.builds = false;
        }
        self.generation.set(self.generation.get() + 1);
        self.subscribe();
    }

    fn heard_frame(self: &Rc<Self>, mut frame: Json) {
        if let Some(rejections) = frame.get("commandRejections").and_then(Json::as_array) {
            if let Some(open) = self.open.borrow_mut().as_mut() {
                open.rejections = rejections
                    .iter()
                    .filter_map(|r| Some((r["commandId"].as_str()?.to_owned(), r["reason"].as_str().unwrap_or_default().to_owned())))
                    .collect();
            }
            self.fail_rejected();
        }
        let stats_moved = {
            let mut slot = self.open.borrow_mut();
            let Some(open) = slot.as_mut() else { return };
            if let Some(usage) = frame.get("contextUsage").filter(|u| !u.is_null()) {
                open.context_usage = (usage["tokens"].as_f64(), usage["window"].as_f64());
            }
            let stats = stats_of(&frame);
            let moved = open.stats != stats;
            if let Some(now) = &stats {
                let chats = CHATS.with(Rc::clone);
                let mut bases = chats.turn_bases.borrow_mut();
                match turn_base(bases.get(&open.id).copied(), open.stats_seen.as_ref(), now) {
                    Some(base) => bases.insert(open.id.clone(), base),
                    None => bases.remove(&open.id),
                };
                open.stats_seen = Some(*now);
            }
            open.stats = stats;
            moved
        };
        if stats_moved {
            self.publish_stats();
        }
        let applied = {
            let mut slot = self.open.borrow_mut();
            let Some(open) = slot.as_mut() else { return };
            // (Opened with all its rows: not down to the newest few, until
            // the engine's whole copy comes.)
            if open.has_whole {
                if frame.get("reset").is_none() || frame["historyPending"].as_bool() == Some(true) {
                    return;
                }
                open.has_whole = false;
                if std::mem::take(&mut open.rebuild) {
                    open.built.clear();
                }
            }
            let before = frame.get("reset").map(|_| std::mem::take(&mut open.entries));
            if before.is_some() {
                open.folding = None;
            }
            let result = apply(&mut open.entries, &mut frame);
            // What it changed, to build again: on a reset, the entries that
            // differ from what this view had, unless another view of this
            // chat has them built; else the ones it touched.
            if let Some(before) = before {
                keep_unchanged(&mut open.built, &before, &open.entries);
                open.older_todo = None;
                self.share_built(open);
            } else if result.is_err() {
                open.built.clear();
            } else {
                for key in ["upsert", "append"] {
                    for item in frame[key].as_array().into_iter().flatten() {
                        let id = if key == "upsert" { &item["entry"]["id"] } else { &item["entry"] };
                        if let Some(id) = id.as_str() {
                            open.built.remove(id);
                        }
                    }
                }
                for id in frame["remove"].as_array().into_iter().flatten() {
                    if let Some(id) = id.as_str() {
                        open.built.remove(id);
                    }
                }
            }
            if result.is_ok() {
                open.heard = true;
                let ids: Vec<&str> = open.entries.iter().map(|e| &*e.id).collect();
                let mut pending = self.pending.borrow_mut();
                if let Some(prompts) = pending.get_mut(&open.id) {
                    prompts.retain(|p| {
                        !ids.contains(&p.id.as_str()) && p.alias.as_deref().is_none_or(|a| !ids.contains(&a))
                    });
                    if prompts.is_empty() {
                        pending.remove(&open.id);
                    }
                }
            }
            result
        };
        if matches!(applied, Ok(Change::Words | Change::Rows)) {
            self.publish_turn_tokens();
        }
        match applied {
            Ok(Change::Quiet) => {}
            Ok(Change::Words) => self.publish_draft(),
            Ok(Change::Rows) => {
                if self.long_to_build(&frame) {
                    self.build_in_steps();
                } else if !self.open.borrow().as_ref().is_some_and(|o| o.stepping) {
                    self.publish_messages();
                }
                self.publish_session();
            }
            Err(()) => self.resubscribe(),
        }
    }

    /// What another view of this chat has built, for the entries here that
    /// are the same there: a chat in a split is watched by its pane and, when
    /// focused, by the window too.
    fn share_built(&self, open: &mut Open) {
        let chats = CHATS.with(Rc::clone);
        let let_go = chats.let_go.borrow();
        if let Some(held) = let_go.as_ref().filter(|h| h.id == open.id) {
            reuse_built(open, &held.entries, &held.built, &held.built_model);
        }
        for view in chats.views() {
            if std::ptr::eq(&*view, self) {
                continue;
            }
            let Ok(other) = view.open.try_borrow() else { continue };
            if let Some(other) = other.as_ref().filter(|o| o.id == open.id) {
                reuse_built(open, &other.entries, &other.built, &other.built_model);
            }
        }
    }

    /// The reply being written: the last entry's words while it streams.
    fn publish_draft(&self) {
        let value = {
            let open = self.open.borrow();
            match open.as_ref().and_then(|o| o.entries.last()) {
                Some(e) if e["status"].as_str() == Some("streaming") => {
                    // The words being written: the reply's last text part
                    // (its block, `part`), apart from the rest of the reply.
                    let tail = e["parts"].as_array().and_then(|parts| {
                        parts.iter().rev().find(|p| p["kind"].as_str() == Some("text"))
                    });
                    let id = e["id"].as_str().unwrap_or_default();
                    let words = match tail {
                        // (`text`, the tail's words too: the page reads the
                        // block's, and of `text` only whether any are yet.)
                        Some(tail) => Value::record([
                            ("id", Value::from(id)),
                            ("text", Value::from(tail["text"].as_str().unwrap_or_default())),
                            ("part", Value::from(tail["id"].as_str().unwrap_or_default())),
                            ("part_text", Value::from(tail["text"].as_str().unwrap_or_default())),
                        ]),
                        None => draft_value(id, &e.json.as_ref().map(entry_text).unwrap_or_default()),
                    };
                    with_thought(words, e["parts"].as_array())
                }
                _ => draft_value("", ""),
            }
        };
        self.draft.set(value);
    }

    /// Whether a reset left so much to build that it goes a little at a time
    /// (`build_in_steps`): the newest entries already show from the frame
    /// before it, and a long chat's whole history would hold up the window.
    fn long_to_build(&self, frame: &Json) -> bool {
        if frame.get("reset").is_none() || frame["historyPending"].as_bool() == Some(true) {
            return false;
        }
        let slot = self.open.borrow();
        let Some(open) = slot.as_ref() else { return false };
        !open.stepping
            && open.entries.iter().filter(|e| !e.id.is_empty() && !open.built.contains_key(&e.id)).count()
                > BUILT_AT_ONCE
    }

    /// Builds this chat's entries a slice at a time between frames, newest
    /// first, then publishes them. Another view of the chat that's building
    /// them is waited for, and what it built taken.
    fn build_in_steps(self: &Rc<Self>) {
        let waits = self.sibling_building();
        if let Some(open) = self.open.borrow_mut().as_mut() {
            open.stepping = true;
            open.builds = !waits;
            open.builds_started = false;
        }
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        engine::get().spawn(async move {
            loop {
                zgx::timer::sleep(BUILD_REST).await;
                let Some(view) = weak.upgrade() else { return };
                if view.generation.get() != generation {
                    return;
                }
                if view.step() {
                    continue;
                }
                if let Some(open) = view.open.borrow_mut().as_mut() {
                    open.stepping = false;
                    open.builds = false;
                }
                view.publish_messages();
                view.publish_session();
                return;
            }
        });
    }

    /// Another view of this chat, building it now.
    fn sibling_building(&self) -> bool {
        let Some(id) = self.open_id() else { return false };
        CHATS.with(Rc::clone).views().iter().any(|view| {
            !std::ptr::eq(&**view, self)
                && view.open.try_borrow().is_ok_and(|o| o.as_ref().is_some_and(|o| o.id == id && o.builds))
        })
    }

    /// One slice of `build_in_steps`: whether there's more.
    fn step(&self) -> bool {
        if self.sibling_building() {
            return true;
        }
        let chats = CHATS.with(Rc::clone);
        let mut slot = self.open.borrow_mut();
        let Some(open) = slot.as_mut() else { return false };
        if !open.builds {
            open.builds = true;
            self.share_built(open);
        }
        let context = build_context(open, &chats);
        let whole_from = open.entries.len().saturating_sub(WHOLE_TAIL);
        let started = std::time::Instant::now();
        code_links::opening(true);
        // The settled ones to the worker, once, with what's left of them here.
        if !open.builds_started {
            open.builds_started = true;
            let Open { entries, built, .. } = open;
            let off: Vec<(usize, String, Json)> = entries
                .iter_mut()
                .enumerate()
                .take(whole_from)
                .rev()
                .filter(|(_, e)| !e.id.is_empty() && !built.contains_key(&e.id) && !e.json.as_ref().is_some_and(kept_whole))
                .filter_map(|(at, e)| Some((at, e.id.to_string(), e.json.take()?)))
                .collect();
            if !off.is_empty() {
                open.folding = Some(folding(off, &context));
            }
        }
        // What it has done.
        while let Some(folding) = &open.folding {
            let batch = match folding.results.try_recv() {
                Ok(batch) => batch,
                Err(std::sync::mpsc::TryRecvError::Empty) => return true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    open.folding = None;
                    break;
                }
            };
            for folded in batch {
                settle_off(open, folded, &context);
            }
            if started.elapsed() >= BUILD_SLICE {
                return true;
            }
        }
        // The newest, whole, here; and any left settled that hasn't its row.
        let Open { entries, older_todo, built, .. } = open;
        for (at, entry) in entries.iter_mut().enumerate().rev() {
            if entry.id.is_empty() || built.contains_key(&entry.id) {
                if at < whole_from && entry.json.is_some() && !entry.id.is_empty() {
                    settle(entry, at, older_todo);
                }
                continue;
            }
            let Some(json) = &entry.json else { continue };
            built.insert(entry.id.clone(), built_for(json, &context, at < whole_from));
            if at < whole_from {
                settle(entry, at, older_todo);
            }
            if started.elapsed() >= BUILD_SLICE {
                return true;
            }
        }
        code_links::opening(false);
        false
    }

    fn publish_messages(&self) {
        let chats = CHATS.with(Rc::clone);
        let subagents;
        let list = {
            let mut slot = self.open.borrow_mut();
            let Some(open) = slot.as_mut() else { return };
            // (Being built a slice at a time: it publishes when that's done.)
            if open.stepping {
                return;
            }
            code_links::opening(false);
            let context = build_context(open, &chats);
            let whole_from = open.entries.len().saturating_sub(WHOLE_TAIL);
            let Open { entries, older_todo, built, .. } = open;
            let mut list: Vec<Value> = Vec::with_capacity(entries.len());
            let mut seen = Subagents::default();
            for (at, e) in entries.iter_mut().enumerate() {
                if e.id.is_empty() {
                    if let Some(row) = e.json.as_ref().and_then(|json| message(json, context.model.as_deref())) {
                        seen.row(&row);
                        list.push(row);
                    }
                    continue;
                }
                let row = match (built.get(&e.id), &e.json) {
                    (Some(row), _) => row.clone(),
                    (None, Some(json)) => {
                        let row = built_for(json, &context, at < whole_from);
                        built.insert(e.id.clone(), row.clone());
                        row
                    }
                    (None, None) => Built::default(),
                };
                if let Some(row) = &row.row {
                    seen.row(row);
                }
                list.extend(row.row);
                seen.add(row.subagents.as_deref());
                if at < whole_from && e.json.is_some() {
                    settle(e, at, older_todo);
                }
            }
            merge_compaction_rows(&mut list);
            let open = &*open;
            // The newest prompt, while its reply hasn't begun: sending,
            // then working, as the shipping app's working trailer says
            // (transcript.rs `sending_bridge`): sending while the send is
            // newer than the session's turn, working once the turn starts.
            if let Some(last) = open.entries.last()
                && last["role"].as_str() == Some("user")
                && let Some(Value::Map(_)) = list.last()
            {
                let sent = last["createdAt"].as_f64().unwrap_or(0.) / 1000.;
                let session = chats.sessions.borrow().get(&open.id).cloned().unwrap_or(Json::Null);
                let turn = seconds(&session["startedAt"]).map(|t| t as f64);
                let running = chats.running(&open.id);
                let fresh = now_seconds() as f64 - sent < SEND_GRACE_SECONDS;
                let waiting = match turn {
                    Some(turn) if running && turn + 1. >= sent => Some(("working", turn)),
                    _ if running || fresh => Some(("sending", sent)),
                    _ => None,
                };
                if let Some((waiting, since)) = waiting
                    && let Some(shown) = list.pop()
                {
                    list.push(with_fields(
                        shown,
                        [("waiting", Value::from(waiting)), ("since", Value::Num(since))],
                    ));
                }
            }
            if let Some(pending) = self.pending.borrow().get(&open.id) {
                for row in pending.iter().map(Pending::value) {
                    seen.row(&row);
                    list.push(row);
                }
            }
            review::place(&open.id, &mut list);
            subagents = seen.rows();
            list
        };
        // The list first: a reply that ended takes its words into its row
        // before the draft lets them go.
        self.subagents.set(Value::List(subagents.into()));
        self.messages.set(Value::List(list.into()));
        self.publish_draft();
        self.after_model_moved();
        let in_flight_moved = self.open.borrow_mut().as_mut().is_some_and(|open| {
            let now = chats.turn_in_flight(open, self.sending(&open.id));
            std::mem::replace(&mut open.in_flight_seen, now) != now
        });
        if in_flight_moved {
            self.publish_session();
        }
    }

    /// The whole copy asked for again, if the model moved (`Open::model_moved`):
    /// what shows stays until it has all come.
    fn after_model_moved(self: &View) {
        let Some(me) = CHATS.with(Rc::clone).views().into_iter().find(|v| std::ptr::eq(&**v, self)) else { return };
        let moved = me.open.borrow_mut().as_mut().is_some_and(|open| {
            let moved = std::mem::take(&mut open.model_moved);
            if moved && !me.snapshot.get() {
                open.has_whole = true;
                open.rebuild = true;
            }
            moved
        });
        if moved && !me.snapshot.get() {
            me.resubscribe();
        }
    }

    fn publish_session(&self) {
        let chats = CHATS.with(Rc::clone);
        let value = {
            let open = self.open.borrow();
            let offline = self.rpc.borrow().as_ref().is_none_or(Rpc::is_closed);
            match open.as_ref() {
                None => Value::Nil,
                Some(open) => session_value(&chats, open, offline, self.sending(&open.id)),
            }
        };
        self.session.set(value);
    }

    fn publish_stats(&self) {
        let value = self.open.borrow().as_ref().and_then(|o| {
            let stats = o.stats?;
            let base = CHATS.with(|c| c.turn_bases.borrow().get(&o.id).copied());
            Some(stats_value(&stats, base.filter(|(_, n)| *n == stats.turns).map(|(before, _)| last_turn(&before, &stats))))
        });
        self.stats.set(value.unwrap_or(Value::Nil));
    }

    fn publish_turn_tokens(&self) {
        let tokens = self.open.borrow().as_ref().and_then(|o| turn_tokens(&o.entries));
        self.turn_tokens.set(tokens.map_or(Value::Nil, |n| Value::from(zeron_proto::usage::format_tokens(n).as_str())));
    }

    async fn send(self: &Rc<Self>, req: Value) -> Result<Option<String>, Outcome> {
        let chats = CHATS.with(Rc::clone);
        let id = req.field("session").map(text).filter(|s| !s.is_empty()).ok_or("missing")?;
        let fresh = req.field("fresh").is_some_and(Value::truthy);
        let chat = chats.chat(&id).unwrap_or(Json::Null);
        if chat.is_null() && !fresh {
            return Err("missing");
        }
        let target = chats.target(&chat);
        let words = req.field("text").map(text).unwrap_or_default();
        // Its id: the page's (it shows the prompt at once) or a new one.
        let message_id = req
            .field("message_id")
            .map(text)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(new_id);
        self.consume_prediction(&Value::record([("id", Value::from(id.as_str()))]));
        // Shown until the transcript has it (the page's own copy kept in
        // step as the list is published again meanwhile), sending.
        self.pending.borrow_mut().entry(id.clone()).or_default().push(Pending {
            id: message_id.clone(),
            words: words.clone(),
            paths: local_paths(&req),
            waiting: "sending",
            retry: None,
            command: None,
            at: stamp(now_seconds()),
            alias: None,
        });
        if let Some(device) = chat["deviceId"].as_str().filter(|d| zeron_proto::is_cloud_device(d, "")) {
            chats.follow_machine(id.clone(), device.to_owned());
        }
        self.publish_messages();
        // What it carries, uploaded to the chat's host.
        let mut paths = Vec::new();
        let mut images = 0;
        // (Each file here, by what it was sent as: an Appshot's context names
        // its picture by the path the agent has.)
        let mut sent_as: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for a in req.field("attachments").map(Value::list).unwrap_or_default() {
            let path = a.field("path").map(text).unwrap_or_default();
            let name = a.field("name").map(text).unwrap_or_else(|| path.clone());
            if path.is_empty() {
                continue;
            }
            if a.field("kind").map(text).as_deref() == Some("image") {
                images += 1;
            }
            // A file of the session's own project (the files pane's): on its
            // computer already, so it goes by its path there.
            if a.field("on_host").is_some_and(Value::truthy) {
                paths.push(path);
                continue;
            }
            match upload(&path, &name, target.as_deref()).await {
                Ok(at) => {
                    sent_as.insert(path.clone(), at.clone());
                    paths.push(at)
                }
                Err(why) => {
                    // Failed before it could go: the prompt says so.
                    self.mark_failed(&id, &message_id, None);
                    return Err(why);
                }
            }
        }
        // Appshots: the apps they show, as untrusted observed content after
        // the words (src/appshots, the shipping app's `with_appshots`).
        let shots: Vec<crate::appshots::Context> = req
            .field("appshots")
            .map(Value::list)
            .unwrap_or_default()
            .iter()
            .map(|s| {
                let field = |k: &str| s.field(k).map(text).filter(|v| !v.is_empty());
                let path = field("path").unwrap_or_default();
                crate::appshots::Context {
                    app: field("app").unwrap_or_else(|| "Application".into()),
                    bundle: field("bundle"),
                    title: field("title"),
                    image: sent_as.get(&path).cloned().unwrap_or(path),
                    text: field("text").unwrap_or_default(),
                    truncated: s.field("truncated").is_some_and(Value::truthy),
                }
            })
            .collect();
        let words = crate::appshots::with_appshots(&words, &shots);
        let prompt = with_attachments(&words, &paths, images);
        if !fresh && chats.running(&id) {
            // Queued: it shows in the queue, not the transcript.
            self.forget_pending(&id, &message_id);
            self.publish_messages();
            let mut params = json!({ "chatId": id, "text": words, "holdForTurnEnd": true });
            if !paths.is_empty() {
                params["attachments"] = json!(paths);
            }
            if req.field("steer_now").is_some_and(Value::truthy) {
                params["steerNow"] = json!(true);
            }
            return engine::get()
                .call("QueueMessage", params)
                .await
                .map(|answer| answer["id"].as_str().map(str::to_owned))
                .map_err(|e| engine::outcome(&e));
        }
        let mut config = merged_config(&chat["config"], &req);
        // A new chat started on a profile: its instructions and tools too (its
        // model, effort, mode and options came as the composer's picks).
        if fresh
            && let Some(profile) = req.field("profile").map(text).and_then(|id| crate::agent_profiles::find(&id))
        {
            config["profileId"] = json!(profile.id);
            if profile.instructions.is_empty() {
                if let Some(map) = config.as_object_mut() {
                    map.remove("instructions");
                }
            } else {
                config["instructions"] = json!(profile.instructions);
            }
            config["disabledTools"] = json!(profile.disabled_tools);
        }
        // gpui's `effective_plan`: off on a harness without one, whatever the switch says.
        if config["harness"].as_str().is_some_and(|h| chats.offers_plan(h, target.as_deref()) == Some(false))
            && let Some(fields) = config.as_object_mut()
        {
            fields.remove("plan");
        }
        // A chat just made takes the config it was started with. (Never one
        // without an agent: the engine turns that away as bad params, and
        // the chat keeps the config it was made with: a side chat's, its
        // parent's.)
        if fresh && config["harness"].is_null() {
            eprintln!("zeron-zgx: chat: {id}: no agent to send with (the chat isn't known here yet)");
            self.mark_failed(&id, &message_id, None);
            return Err("missing");
        }
        // Another agent goes first, as the shipping app's picker does: the
        // engine answers a switch by ending the live turn, which would be
        // this run's.
        let switching = !fresh && chat["config"]["harness"] != config["harness"];
        if fresh || switching {
            let mut params = json!({ "op": "setChatConfig", "chatId": id, "config": config });
            // (Its divider, with the conversation handed over.)
            if switching {
                let name = |h: &Json| {
                    let id = h.as_str().unwrap_or_default();
                    chats.harness_name(id).unwrap_or_else(|| id.to_owned())
                };
                let (from, to) = (name(&chat["config"]["harness"]), name(&config["harness"]));
                params["note"] = provider_note(&from, &to);
            }
            if let Err(e) = engine::get().call("Mutate", params).await {
                eprintln!("zeron-zgx: chat: a chat's config wasn't set: {e}");
                self.mark_failed(&id, &message_id, None);
                return Err(engine::outcome(&e));
            }
        }
        let field = |key: &str| req.field(key).map(text).filter(|s| !s.is_empty());
        let cwd = field("cwd")
            .or_else(|| chat["cwd"].as_str().map(Into::into))
            .unwrap_or_else(|| "~".into());
        let mut request = json!({
            "prompt": prompt,
            "harness": config["harness"],
            "model": config["model"],
            "reasoning": config["reasoning"],
            "modelOptions": config.get("modelOptions").cloned().unwrap_or(json!({})),
            "cwd": cwd,
            "sandbox": "workspace-write",
            "autoApprove": false,
            "resume": null,
        });
        if let Some(access) = config.get("access") {
            request["access"] = access.clone();
        }
        if let Some(plan) = config.get("plan") {
            request["plan"] = plan.clone();
        }
        if !paths.is_empty() {
            request["attachments"] = json!(paths);
        }
        if fresh && req.field("worktree").is_some_and(Value::truthy) {
            request["worktree"] = json!({
                "repoPath": cwd,
                "base": field("branch").unwrap_or_else(|| "HEAD".into()),
                "spaceId": field("project"),
            });
        }
        let run = json!({ "kind": "run", "messageId": message_id, "request": request });
        let sent = self.send_run(&id, &message_id, &run).await;
        if sent.is_err() {
            // Not delivered: the prompt stays, failed, to send again.
            self.mark_failed(&id, &message_id, Some(run));
        } else if !fresh && runs_otherwise(&chat["config"], &config) {
            // Sent with another agent, model or effort than the chat had (the
            // composer's pick, kept as a draft until now): the chat takes
            // them, as the shipping app's next turn does, and its icon with
            // them. (Shown at once; the engine's word follows. A config that
            // doesn't stick leaves the run as sent.)
            if let Some(c) = chats.chats.borrow_mut().get_mut(&id) {
                c["config"] = config.clone();
            }
            chats.publish_views();
            if !switching && let Err(e) = engine::get()
                .call("Mutate", json!({ "op": "setChatConfig", "chatId": id, "config": config }))
                .await
            {
                eprintln!("zeron-zgx: chat: a sent pick didn't reach the chat's config: {e}");
            }
        }
        self.publish_messages();
        sent.map(|()| None)
    }

    /// A pending prompt that didn't go: failed, with the command that sends
    /// it again (none: it never got that far, and is sent again whole).
    fn mark_failed(&self, id: &str, message_id: &str, retry: Option<Json>) {
        if let Some(pending) = self.pending.borrow_mut().get_mut(id)
            && let Some(p) = pending.iter_mut().find(|p| p.id == message_id)
        {
            p.waiting = "failed";
            p.retry = retry;
        }
        self.publish_messages();
    }

    fn forget_pending(&self, id: &str, message_id: &str) {
        let mut pending = self.pending.borrow_mut();
        if let Some(prompts) = pending.get_mut(id) {
            prompts.retain(|p| p.id != message_id);
            if prompts.is_empty() {
                pending.remove(id);
            }
        }
    }

    async fn send_run(self: &Rc<Self>, chat: &str, message_id: &str, run: &Json) -> Result<(), Outcome> {
        self.set_refused(None);
        match queue_command(chat, run.clone()).await {
            Ok(command) => {
                if let Some(prompts) = self.pending.borrow_mut().get_mut(chat)
                    && let Some(p) = prompts.iter_mut().find(|p| p.id == message_id)
                {
                    p.command = Some(command);
                    p.retry = Some(run.clone());
                }
                self.fail_rejected();
                self.expect_delivery(chat, message_id, run);
                self.expect_turn();
                Ok(())
            }
            Err((outcome, reason)) => {
                if outcome == "failed" {
                    self.set_refused(Some(reason));
                }
                Err(outcome)
            }
        }
    }

    fn set_refused(&self, reason: Option<String>) {
        if let Some(open) = self.open.borrow_mut().as_mut() {
            open.refused = reason;
        }
        self.publish_session();
    }

    fn fail_rejected(&self) {
        let reason = {
            let mut slot = self.open.borrow_mut();
            let Some(open) = slot.as_mut() else { return };
            let rejections = std::mem::take(&mut open.rejections);
            let mut reason = None;
            let mut pending = self.pending.borrow_mut();
            for p in pending.get_mut(&open.id).into_iter().flatten().filter(|p| p.waiting == "sending") {
                if let Some(why) = p.command.as_ref().and_then(|c| rejections.get(c)) {
                    p.waiting = "failed";
                    reason = Some(why.clone());
                }
            }
            open.rejections = rejections;
            reason
        };
        if let Some(reason) = reason {
            self.set_refused(Some(reason));
            self.publish_messages();
        }
    }

    /// A command the engine took can still be rejected at its drain (a
    /// worktree off a repo with no commits): `commandRejections` says so, on
    /// an engine that sends it. On one that doesn't, past the grace the prompt
    /// reads failed, as the shipping app's "Not delivered" does. (A host
    /// elsewhere may only be away: left sending.)
    fn expect_turn(self: &Rc<Self>) {
        if let Some(open) = self.open.borrow().as_ref() {
            open.awaiting_turn.set(Some(std::time::Instant::now()));
        }
        let me = Rc::downgrade(self);
        engine::get().spawn(async move {
            zgx::timer::sleep(TURN_START_GRACE).await;
            if let Some(me) = me.upgrade() {
                me.publish_session();
            }
        });
    }

    fn expect_delivery(self: &Rc<Self>, chat: &str, message_id: &str, run: &Json) {
        let chats = CHATS.with(Rc::clone);
        if chats.chat(chat).is_some_and(|c| chats.target(&c).is_some()) {
            return;
        }
        let (me, chat, message_id, run) = (Rc::downgrade(self), chat.to_owned(), message_id.to_owned(), run.clone());
        engine::get().spawn(async move {
            zgx::timer::sleep(std::time::Duration::from_secs_f64(SEND_GRACE_SECONDS)).await;
            if let Some(me) = me.upgrade() {
                me.mark_failed(&chat, &message_id, Some(run));
            }
        });
    }

    /// A failed prompt sent again (its run as it was), sending meanwhile.
    async fn retry(self: &Rc<Self>, args: Value) -> Result<(), Outcome> {
        let id = args.field("id").map(text).unwrap_or_default();
        let message_id = args.field("message").map(text).unwrap_or_default();
        if self.open_id().as_deref() != Some(&id) {
            return Err("missing");
        }
        let run = {
            let mut pending = self.pending.borrow_mut();
            let prompts = pending.get_mut(&id).ok_or("missing")?;
            let p = prompts.iter_mut().find(|p| p.id == message_id).ok_or("missing")?;
            p.waiting = "sending";
            p.command = None;
            p.retry.clone()
        };
        let chats = CHATS.with(Rc::clone);
        if let Some(device) = chats.chat(&id).and_then(|c| c["deviceId"].as_str().map(str::to_owned)).filter(|d| zeron_proto::is_cloud_device(d, "")) {
            chats.follow_machine(id.clone(), device);
        }
        self.publish_messages();
        let Some(run) = run else {
            // It never got as far as a run (an upload failed): the composer
            // has the words; the row goes.
            self.forget_pending(&id, &message_id);
            self.publish_messages();
            return Err("failed");
        };
        let sent = self.send_run(&id, &message_id, &run).await;
        if sent.is_err() {
            self.mark_failed(&id, &message_id, Some(run));
        }
        self.publish_messages();
        sent
    }

    async fn respond(self: &Rc<Self>, args: Value) -> Result<(), Outcome> {
        let id = args.field("id").map(text).unwrap_or_default();
        let request = args.field("request").map(text).unwrap_or_default();
        let questions = {
            let open = self.open.borrow();
            let open = open.as_ref().filter(|o| o.id == id).ok_or("missing")?;
            asked(open.entries.iter().rev().filter_map(|e| e.json.as_ref()))
                .filter(|p| p["requestId"].as_str() == Some(&request))
                .map(|p| p["questions"].as_array().cloned().unwrap_or_default())
                .ok_or("missing")?
        };
        // A secret goes in its own card in the Zeron app, never through a
        // command: those stay in the chat's doc, which syncs.
        CHATS.with(Rc::clone).answer(&id, &request, &questions, &args).await
    }

    /// Back a page in the question being answered (the shipping app's
    /// wizard `back`): its last page's answer taken off, so that page shows
    /// again; what was picked there (its labels), for the composer to show
    /// chosen. Nil on the first page.
    fn question_back(&self, args: &Value) -> Value {
        CHATS.with(|c| c.page_back(args))
    }

    async fn configure(self: &Rc<Self>, args: Value) -> Result<(), Outcome> {
        let chats = CHATS.with(Rc::clone);
        let id = args.field("id").map(text).unwrap_or_default();
        let chat = chats.chat(&id).ok_or("missing")?;
        let config = merged_config(&chat["config"], &args);
        // Shown at once; the engine's word follows.
        if let Some(c) = chats.chats.borrow_mut().get_mut(&id) {
            c["config"] = config.clone();
        }
        chats.publish_views();
        let mut params = json!({ "op": "setChatConfig", "chatId": id, "config": config });
        // The switch's divider, as the dialog worded it.
        if let Some(note) = args.field("note").map(text).filter(|t| !t.is_empty()) {
            params["note"] = json!({ "text": note, "hint": args.field("note_hint").map(text) });
        }
        engine::get().call("Mutate", params).await.map(|_| ()).map_err(|e| engine::outcome(&e))
    }

    /// A queued message's text, taken to be edited here: the lease asked
    /// for, and kept (renewed before it lapses) until the edit finishes.
    async fn begin_edit(self: &Rc<Self>, args: Value) -> Result<Value, Outcome> {
        let chats = CHATS.with(Rc::clone);
        let id = args.field("id").map(text).unwrap_or_default();
        let item = args.field("item").map(text).unwrap_or_default();
        let chat = chats.chat(&id).ok_or("missing")?;
        let mut params = json!({
            "chatId": id,
            "id": item,
            "editorDeviceId": chats.here.borrow().clone(),
            "editorInstanceId": instance_id(),
        });
        if let Some(t) = chats.target(&chat) {
            params["targetDeviceId"] = json!(t);
        }
        let answer = engine::get()
            .call("BeginQueuedMessageEdit", params)
            .await
            .map_err(|e| engine::outcome(&e))?;
        match answer["outcome"].as_str() {
            Some("acquired") => {}
            Some("locked") => return Err("locked"),
            _ => return Err("missing"),
        }
        let lease = Lease {
            item: item.clone(),
            lease: answer["leaseId"].as_str().unwrap_or_default().to_owned(),
            hash: answer["baseTextHash"].clone(),
        };
        if let Some(open) = self.open.borrow_mut().as_mut().filter(|o| o.id == id) {
            open.editing = Some(lease.clone());
        }
        self.publish_session();
        self.keep_lease(id, lease, answer["expiresAtMs"].as_f64());
        // Its files too, as the composer's chips: already on the chat's
        // host (they went with the message), so saving names them again
        // rather than sending them up a second time.
        let attachments: Vec<Value> = answer["attachments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Json::as_str)
            .map(queued_attachment)
            .collect();
        Ok(Value::record([
            ("text", Value::from(answer["text"].as_str().unwrap_or_default())),
            ("attachments", Value::List(attachments.into())),
        ]))
    }

    /// Renews the lease halfway to each expiry, while it is still held.
    fn keep_lease(self: &Rc<Self>, chat: String, lease: Lease, expires: Option<f64>) {
        let weak = Rc::downgrade(self);
        engine::get().spawn(async move {
            let mut expires = expires;
            loop {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0., |d| d.as_millis() as f64);
                let wait = expires.map_or(10_000., |at| ((at - now) / 2.).clamp(1_000., 30_000.));
                zgx::timer::sleep(std::time::Duration::from_millis(wait as u64)).await;
                let Some(me) = weak.upgrade() else { return };
                let held = me
                    .open
                    .borrow()
                    .as_ref()
                    .and_then(|o| o.editing.as_ref())
                    .is_some_and(|e| e.lease == lease.lease);
                if !held {
                    return;
                }
                let answer = engine::get()
                    .call(
                        "RenewQueuedMessageEdit",
                        json!({ "chatId": chat, "id": lease.item, "leaseId": lease.lease }),
                    )
                    .await;
                match answer {
                    Ok(a) if a["outcome"].as_str() == Some("renewed") => expires = a["expiresAtMs"].as_f64(),
                    _ => {
                        // Lost (taken elsewhere, or the message went): the
                        // composer keeps the words; saving says so.
                        return;
                    }
                }
            }
        });
    }

    /// The edit ended: its words saved over the queued message ("commit"),
    /// or the message left as it was ("cancel").
    async fn finish_edit(self: &Rc<Self>, args: Value) -> Result<(), Outcome> {
        let chats = CHATS.with(Rc::clone);
        let id = args.field("id").map(text).unwrap_or_default();
        let action = args.field("action").map(text).unwrap_or_else(|| "cancel".into());
        let lease = self
            .open
            .borrow_mut()
            .as_mut()
            .filter(|o| o.id == id)
            .and_then(|o| o.editing.take())
            .ok_or("missing")?;
        self.publish_session();
        let chat = chats.chat(&id).ok_or("missing")?;
        let mut params = json!({
            "chatId": id,
            "id": lease.item,
            "leaseId": lease.lease,
            "action": action,
        });
        let target = chats.target(&chat);
        if action == "commit" {
            params["text"] = json!(args.field("text").map(text).unwrap_or_default());
            params["expectedTextHash"] = lease.hash.clone();
            // The files it carries now (the composer's chips): the ones it
            // had by their paths on the host, any added sent up first. (None
            // given: it keeps the ones it had.)
            if let Some(list) = args.field("attachments") {
                let mut paths = Vec::new();
                for a in list.list() {
                    let path = a.field("path").map(text).unwrap_or_default();
                    if path.is_empty() {
                        continue;
                    }
                    if a.field("on_host").is_some_and(Value::truthy) {
                        paths.push(path);
                        continue;
                    }
                    let name = a.field("name").map(text).unwrap_or_else(|| path.clone());
                    paths.push(upload(&path, &name, target.as_deref()).await?);
                }
                params["attachments"] = json!(paths);
            }
        }
        if let Some(t) = &target {
            params["targetDeviceId"] = json!(t);
        }
        let answer = engine::get()
            .call("FinishQueuedMessageEdit", params)
            .await
            .map_err(|e| engine::outcome(&e))?;
        match answer["outcome"].as_str() {
            Some("committed" | "cancelled" | "discarded" | "released") => Ok(()),
            Some("conflict") => Err("conflict"),
            _ => Err("missing"),
        }
    }

    // Under the row's id: the engine keeps it through promotion (doc_host `dispatch_queued`).
    fn queue_shown(&self, args: &Value) -> Option<String> {
        if args.field("op").map(text).as_deref() != Some("send") {
            return None;
        }
        let chats = CHATS.with(Rc::clone);
        let id = args.field("id").map(text).unwrap_or_default();
        let item = args.field("item").map(text).unwrap_or_default();
        let chat = chats.chat(&id)?;
        let harness = chat["config"]["harness"].as_str().unwrap_or_default();
        let steers = chats.steers_mid_turn(harness, chats.target(&chat).as_deref());
        {
            let mut slot = self.open.borrow_mut();
            let open = slot.as_mut().filter(|o| o.id == id)?;
            let row = open.queue.iter().find(|q| q["id"].as_str() == Some(item.as_str()))?.clone();
            if !runs_now(&row, chats.running(&id), steers) {
                return None;
            }
            let shown = |e: &Entry| &*e.id == item.as_str();
            let mut pending = self.pending.borrow_mut();
            if pending.get(&id).is_some_and(|p| p.iter().any(|p| p.id == item)) || open.entries.iter().any(shown) {
                return None;
            }
            pending.entry(id.clone()).or_default().push(Pending {
                id: item.clone(),
                words: row["text"].as_str().unwrap_or_default().to_owned(),
                paths: row_paths(&row),
                waiting: "sending",
                retry: None,
                command: None,
                at: stamp(now_seconds()),
                alias: None,
            });
        }
        self.publish_messages();
        Some(item)
    }

    fn unshow(&self, shown: &Option<String>) {
        let Some(item) = shown else { return };
        {
            let mut pending = self.pending.borrow_mut();
            for prompts in pending.values_mut() {
                prompts.retain(|p| &p.id != item);
            }
            pending.retain(|_, prompts| !prompts.is_empty());
        }
        self.publish_messages();
    }

    fn steer_all_rows(&self, args: &Value) -> Option<SteerAll> {
        let id = args.field("id").map(text).unwrap_or_default();
        let items: Vec<String> = args.field("items").map(Value::list).unwrap_or_default().iter().map(text).collect();
        let slot = self.open.borrow();
        let open = slot.as_ref().filter(|o| o.id == id)?;
        let rows: Vec<&Json> = open
            .queue
            .iter()
            .filter(|q| q["id"].as_str().is_some_and(|i| items.iter().any(|x| x == i)))
            .collect();
        if rows.len() < 2 {
            return None;
        }
        Some(
            rows.iter()
                .filter_map(|q| {
                    let id = q["id"].as_str()?.to_owned();
                    Some(SteerRow { id, text: q["text"].as_str().unwrap_or_default().to_owned(), paths: row_paths(q) })
                })
                .collect(),
        )
    }

    fn steer_all_shown(&self, args: &Value) -> Option<String> {
        let rows = self.steer_all_rows(args)?;
        let (words, paths) = whole_of(&rows);
        let message_id = args.field("message_id").map(text).filter(|s| !s.is_empty())?;
        let id = args.field("id").map(text).unwrap_or_default();
        self.pending.borrow_mut().entry(id).or_default().push(Pending {
            id: message_id.clone(),
            words,
            paths,
            waiting: "sending",
            retry: None,
            command: None,
            at: stamp(now_seconds()),
            alias: None,
        });
        self.publish_messages();
        Some(message_id)
    }

    // Rows come off first and only those that came off are sent, so no row's words are in two places.
    async fn steer_all(self: &Rc<Self>, args: Value, shown: Option<String>) -> Result<(), Outcome> {
        let chats = CHATS.with(Rc::clone);
        let id = args.field("id").map(text).unwrap_or_default();
        let early = |why: Outcome| {
            self.unshow(&shown);
            Err(why)
        };
        let Some(chat) = chats.chat(&id) else { return early("missing") };
        let Some(rows) = self.steer_all_rows(&args) else { return early("missing") };
        let target = chats.target(&chat);
        let routed = |mut p: Json| {
            if let Some(t) = &target {
                p["targetDeviceId"] = json!(t);
            }
            p
        };
        let mut taken: SteerAll = Vec::new();
        let mut stopped = None;
        for row in rows {
            let params = routed(json!({ "chatId": id, "id": row.id }));
            match self.row_call(&id, &row.id, "RemoveQueuedMessage", params, "removed", false).await {
                Ok(true) => taken.push(row),
                Ok(false) => {}
                Err(why) => {
                    eprintln!("zeron-zgx: chat: steer all: a queued row wasn't removed ({why})");
                    stopped = Some(why);
                    break;
                }
            }
        }
        if taken.is_empty() {
            return early(stopped.unwrap_or("gone"));
        }
        let (words, paths) = whole_of(&taken);
        let mut params = json!({ "chatId": id, "text": words, "holdForTurnEnd": true });
        if !paths.is_empty() {
            params["attachments"] = json!(paths);
        }
        let shows_whole = |alias: Option<String>| {
            if let Some(prompts) = self.pending.borrow_mut().get_mut(&id)
                && let Some(p) = prompts.iter_mut().find(|p| Some(&p.id) == shown.as_ref())
            {
                p.words = words.clone();
                p.paths = paths.clone();
                p.alias = alias;
            }
        };
        let whole = match engine::get().call("QueueMessage", params).await {
            Ok(answer) => answer["id"].as_str().map(str::to_owned),
            Err(e) => {
                // The rows are gone from the queue: the transcript's failed row holds their words, to send again.
                shows_whole(None);
                if let Some(item) = &shown {
                    self.mark_failed(&id, item, None);
                }
                return Err(engine::outcome(&e));
            }
        };
        let Some(whole) = whole else { return early("failed") };
        shows_whole(Some(whole.clone()));
        self.publish_messages();
        if let Some(why) = stopped {
            // The rest stayed queued, so what came off stays queued too, as one row.
            self.unshow(&shown);
            return Err(why);
        }
        let method = queue_send_method(!paths.is_empty());
        match self.row_call(&id, &whole, method, routed(json!({ "chatId": id, "id": whole })), "sent", true).await {
            Ok(_) => Ok(()),
            Err(why) => {
                self.unshow(&shown);
                Err(why)
            }
        }
    }

    async fn queue_op(self: &Rc<Self>, args: Value, shown: Option<String>) -> Result<(), Outcome> {
        let result = self.queue_call(&args).await;
        if !matches!(result, Ok(true)) {
            self.unshow(&shown);
        }
        match result {
            Ok(true) => Ok(()),
            Ok(false) => Err("gone"),
            Err(why) => Err(why),
        }
    }

    async fn queue_call(self: &Rc<Self>, args: &Value) -> Result<bool, Outcome> {
        let chats = CHATS.with(Rc::clone);
        let id = args.field("id").map(text).unwrap_or_default();
        let item = args.field("item").map(text).unwrap_or_default();
        let chat = chats.chat(&id).ok_or("missing")?;
        let carries = self.open.borrow().as_ref().is_some_and(|o| {
            o.queue
                .iter()
                .any(|q| q["id"].as_str() == Some(item.as_str()) && !row_paths(q).is_empty())
        });
        let mut params = json!({ "chatId": id, "id": item });
        let target = chats.target(&chat);
        let routed = |mut p: Json| {
            if let Some(t) = &target {
                p["targetDeviceId"] = json!(t);
            }
            p
        };
        let (method, said, params) = match args.field("op").map(text).as_deref() {
            Some("remove") => ("RemoveQueuedMessage", "removed", routed(params)),
            Some("send") => (queue_send_method(carries), "sent", routed(params)),
            Some("move") => {
                let to = args.field("to").and_then(Value::as_f64).unwrap_or(1.);
                params["toIndex"] = json!((to as i64 - 1).max(0));
                engine::get()
                    .call("MoveQueuedMessage", params)
                    .await
                    .map_err(|e| engine::outcome(&e))?;
                return Ok(true);
            }
            _ => return Err("failed"),
        };
        self.row_call(&id, &item, method, params, said, false).await
    }

    // A row queued a moment ago reaches another device as the doc syncs, so a miss is asked again.
    // `just_queued`: this row was queued here a moment ago, so not being listed yet doesn't mean it left.
    async fn row_call(&self, chat: &str, item: &str, method: &str, params: Json, said: &str, just_queued: bool) -> Result<bool, Outcome> {
        let mut wait = std::time::Duration::from_millis(100);
        for _ in 0..6 {
            let answer = engine::get()
                .call(method, params.clone())
                .await
                .map_err(|e| engine::outcome(&e))?;
            if answer[said].as_bool() != Some(false) {
                return Ok(true);
            }
            let here = self.open.borrow().as_ref().is_some_and(|o| {
                o.id == chat && o.queue.iter().any(|q| q["id"].as_str() == Some(item))
            });
            if !here && !just_queued {
                return Ok(false);
            }
            zgx::timer::sleep(wait).await;
            wait *= 2;
        }
        // Six seconds on, heard here and gone from the list: it ran as the next turn.
        let here = self.open.borrow().as_ref().is_some_and(|o| {
            o.id == chat && o.queue.iter().any(|q| q["id"].as_str() == Some(item))
        });
        if just_queued && !here {
            return Ok(false);
        }
        eprintln!("zeron-zgx: chat: {method} for {chat}: the host never found queued row {item}");
        Err("failed")
    }
}

// gpui: Steer and Send next never interrupt (the host holds a row for a turn-boundary agent); only files send now.
fn queue_send_method(carries_files: bool) -> &'static str {
    if carries_files { "SendQueuedMessageNow" } else { "SteerQueuedMessageNow" }
}

fn runs_now(row: &Json, running: bool, steers: Option<bool>) -> bool {
    !row_paths(row).is_empty() || !running || steers == Some(true)
}

struct SteerRow {
    id: String,
    text: String,
    paths: Vec<String>,
}

type SteerAll = Vec<SteerRow>;

fn whole_of(rows: &[SteerRow]) -> (String, Vec<String>) {
    let texts: Vec<String> = rows.iter().map(|r| r.text.clone()).collect();
    let words = if let [one] = texts.as_slice() { one.trim().to_owned() } else { numbered(&texts) };
    (words, rows.iter().flat_map(|r| r.paths.clone()).collect())
}

fn numbered(texts: &[String]) -> String {
    texts
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}. {}", i + 1, t.trim()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn row_paths(row: &Json) -> Vec<String> {
    row["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().map(str::to_owned))
        .collect()
}

async fn limit_op(args: Value) -> Result<Value, Outcome> {
    let id = args.field("id").map(text).unwrap_or_default();
    let (method, mut params) = match args.field("op").map(text).as_deref() {
        Some(choice @ ("switch" | "wait" | "hold")) => ("ResolveLimitOffer", json!({ "chatId": id, "choice": choice })),
        Some("arm") => ("SetLimitResume", json!({ "chatId": id, "auto": true })),
        Some("cancel") => ("SetLimitResume", json!({ "chatId": id, "auto": false })),
        _ => return Err("failed"),
    };
    let chats = CHATS.with(Rc::clone);
    let chat = chats.chat(&id).ok_or("missing")?;
    if let Some(t) = chats.target(&chat) {
        params["targetDeviceId"] = json!(t);
    }
    engine::get().call(method, params).await.map_err(|e| engine::outcome(&e))?;
    Ok(Value::Nil)
}

async fn compact(args: Value) -> Result<Value, String> {
    use zeron_proto::{app_commands::COMPACT_NEEDS_CHAT, compaction::compact_outcome_notice};
    let chats = CHATS.with(Rc::clone);
    let id = args.field("id").map(text).unwrap_or_default();
    let chat = chats.chat(&id).ok_or(COMPACT_NEEDS_CHAT)?;
    if chats.chat_compacting(&id) {
        return Err("Compaction in progress".into());
    }
    if chats.running(&id) {
        return Err("Wait for the current turn to finish".into());
    }
    if let Some(reason) = chat["config"]["harness"].as_str().and_then(|h| chats.compaction_unsupported.borrow().get(h).cloned()) {
        return Err(reason);
    }
    let mut params = json!({ "chatId": id });
    if let Some(t) = chats.target(&chat) {
        params["targetDeviceId"] = json!(t);
    }
    if let Some(focus) = args.field("focus").map(text).filter(|f| !f.is_empty()) {
        params["focus"] = json!(focus);
    }
    chats.compacting.borrow_mut().insert(id.clone());
    chats.publish_views();
    let answer = engine::get().call("CompactChat", params).await;
    chats.compacting.borrow_mut().remove(&id);
    chats.publish_views();
    match answer {
        Ok(reply) => compact_outcome_notice(&reply).map_or(Ok(Value::Nil), Err),
        Err(why) => Err(format!("Compaction failed: {why}")),
    }
}

/// A computer card's answer, one of the labels the engine reads (gpui's `consent_answers`).
async fn answer_computer(args: Value) -> Result<Value, Outcome> {
    let id = args.field("id").map(text).unwrap_or_default();
    let request = args.field("request").map(text).unwrap_or_default();
    let question = args.field("question").map(text).unwrap_or_default();
    let answer = args.field("answer").map(text).unwrap_or_default();
    if !computer_answer(&answer) || request.is_empty() || question.is_empty() {
        return Err("failed");
    }
    let answers = json!([{ "questionId": question, "labels": [answer] }]);
    command(&id, json!({ "kind": "respondInput", "requestId": request, "answers": answers })).await?;
    Ok(Value::Nil)
}

fn computer_answer(label: &str) -> bool {
    use zeron_proto::computer::cards::*;
    [COMPUTER_CONSENT_ALLOW, COMPUTER_CONSENT_DECLINE, COMPUTER_FOREGROUND_ALLOW, COMPUTER_FOREGROUND_DECLINE].contains(&label)
        || label.strip_prefix(COMPUTER_CONSENT_ALWAYS_PREFIX).is_some_and(|app| !app.trim().is_empty())
}

/// A transcript secret card's answer: the secret saved, or declined with none.
async fn answer_secret(args: Value) -> Result<Value, String> {
    let id = args.field("id").map(text).unwrap_or_default();
    let request = args.field("request").map(text).unwrap_or_default();
    let secret = args.field("secret").map(text).filter(|s| !s.trim().is_empty());
    let params = json!({ "chatId": id, "requestId": request, "secret": secret });
    engine::get().call("AnswerSecret", params).await.map(|_| Value::Nil).map_err(|e| e.to_string())
}

async fn set_work_tree(args: Value) -> Result<Value, Outcome> {
    let chats = CHATS.with(Rc::clone);
    let id = args.field("id").map(text).unwrap_or_default();
    let chat = chats.chat(&id).ok_or("missing")?;
    let path = args.field("path").map(text).filter(|p| !p.is_empty());
    let mut params = json!({ "op": "setChatWorkTree", "chatId": id, "path": path });
    if let Some(t) = chats.target(&chat) {
        params["targetDeviceId"] = json!(t);
    }
    engine::get().call("Mutate", params).await.map_err(|e| engine::outcome(&e))?;
    Ok(Value::Nil)
}

const MAX_READ_TEXT: u64 = 32 * 1024 * 1024;
const MAX_READ_CHUNKS: usize = 1_000;

async fn read_text(args: Value) -> Result<Value, Outcome> {
    let path = args.field("path").map(text).filter(|p| !p.is_empty()).ok_or("missing")?;
    let host = args.field("session").map(text).and_then(|id| host_of(&id));
    let (words, lines) = match host {
        Some(host) => {
            let encoded = read_chunks(&path, &host).await?;
            off_thread(move || -> Result<(String, usize), Outcome> {
                use base64::Engine as _;
                let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| "failed")?;
                Ok(decoded_text(&bytes))
            })
            .await
            .ok_or("failed")??
        }
        None => off_thread(move || -> Result<(String, usize), Outcome> {
            let meta = std::fs::metadata(&path).map_err(|_| "unreadable")?;
            if !meta.is_file() {
                return Err("not_file");
            }
            if meta.len() > MAX_READ_TEXT {
                return Err("too_large");
            }
            let bytes = std::fs::read(&path).map_err(|_| "unreadable")?;
            Ok(decoded_text(&bytes))
        })
        .await
        .ok_or("failed")??,
    };
    Ok(Value::record([("text", Value::from(words.as_str())), ("lines", Value::Num(lines as f64))]))
}

fn decoded_text(bytes: &[u8]) -> (String, usize) {
    let words = String::from_utf8_lossy(bytes).into_owned();
    let lines = words.lines().count();
    (words, lines)
}

async fn read_chunks(path: &str, host: &str) -> Result<String, Outcome> {
    let most = (MAX_READ_TEXT as usize).div_ceil(3) * 4;
    let mut encoded = String::new();
    let mut offset = 0u64;
    for _ in 0..MAX_READ_CHUNKS {
        let chunk = engine::get()
            .call("ReadAttachmentChunk", json!({ "path": path, "offset": offset, "targetDeviceId": host }))
            .await
            .map_err(|e| engine::outcome(&e))?;
        let data = chunk["data"].as_str().ok_or("failed")?;
        if encoded.len().saturating_add(data.len()) > most {
            return Err("too_large");
        }
        encoded.push_str(data);
        if chunk["done"].as_bool() == Some(true) {
            return Ok(encoded);
        }
        let next = chunk["nextOffset"].as_u64().ok_or("failed")?;
        if next <= offset {
            return Err("failed");
        }
        offset = next;
    }
    Err("failed")
}

async fn stat_files(args: Value) -> Result<Value, Outcome> {
    let paths: Vec<String> = args.field("paths").map(Value::list).unwrap_or_default().iter().map(text).collect();
    let stats = off_thread(move || {
        paths
            .into_iter()
            .map(|path| {
                let meta = std::fs::metadata(&path).ok();
                let readable = std::fs::File::open(&path).is_ok();
                let size = meta.as_ref().map_or(0, std::fs::Metadata::len);
                let is_file = meta.is_some_and(|m| m.is_file());
                (path, size, is_file, readable)
            })
            .collect::<Vec<_>>()
    })
    .await
    .ok_or("failed")?;
    Ok(Value::List(
        stats
            .into_iter()
            .map(|(path, size, is_file, readable)| {
                Value::record([
                    ("path", Value::from(path.as_str())),
                    ("size", Value::Num(size as f64)),
                    ("is_file", Value::Bool(is_file)),
                    ("readable", Value::Bool(readable)),
                ])
            })
            .collect::<Vec<_>>()
            .into(),
    ))
}

/// A queued message's file as the composer's chip: its path on the chat's
/// host (or the `pending://` ref it travels by from another device), named
/// as it was attached (an upload's id prefix off), a picture shown as one
/// where it can be read.
fn queued_attachment(path: &str) -> Value {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let name = match base.split_once('-') {
        Some((id, rest)) if id.len() == 8 && id.bytes().all(|b| b.is_ascii_hexdigit()) && !rest.is_empty() => rest,
        _ => base,
    };
    let lower = name.to_ascii_lowercase();
    let image = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".heic", ".bmp", ".tiff"]
        .iter()
        .any(|e| lower.ends_with(e));
    let mut fields = vec![
        ("path", Value::from(path)),
        ("name", Value::from(name)),
        ("kind", Value::from(if image { "image" } else { "file" })),
        ("on_host", Value::Bool(true)),
    ];
    if image && !path.starts_with("pending://") {
        fields.push(("src", Value::from(path)));
    }
    Value::record(fields)
}

/// A command for a chat (`QueueCommand`): a run, an interrupt, an answer.
/// `StopBackgroundTask` for `{ session, task }`, routed to the chat's host.
async fn stop_task(args: Value) -> Result<Value, Outcome> {
    let id = args.field("session").map(text).unwrap_or_default();
    let task = args.field("task").map(text).unwrap_or_default();
    if id.is_empty() || task.is_empty() {
        return Err("missing");
    }
    let chats = CHATS.with(Rc::clone);
    let mut params = json!({ "chatId": id, "taskId": task });
    if let Some(t) = chats.chat(&id).and_then(|c| chats.target(&c)) {
        params["targetDeviceId"] = json!(t);
    }
    let answer = engine::get()
        .call("StopBackgroundTask", params)
        .await
        .map_err(|e| engine::outcome(&e))?;
    Ok(Value::Bool(answer["stopping"].as_bool() == Some(true)))
}

/// The person's Stop waiting for a chat (`StopWaiting`), routed to its host:
/// what its agent said it waits on no longer holds it.
async fn stop_waiting(id: String) -> Result<Value, Outcome> {
    if id.is_empty() {
        return Err("missing");
    }
    let chats = CHATS.with(Rc::clone);
    let mut params = json!({ "chatId": id });
    if let Some(t) = chats.chat(&id).and_then(|c| chats.target(&c)) {
        params["targetDeviceId"] = json!(t);
    }
    let answer = engine::get()
        .call("StopWaiting", params)
        .await
        .map_err(|e| engine::outcome(&e))?;
    Ok(Value::Bool(answer["stopped"].as_bool() == Some(true)))
}

async fn command(chat: &str, command: Json) -> Result<(), Outcome> {
    queue_command(chat, command).await.map(drop).map_err(|(outcome, _)| outcome)
}

async fn queue_command(chat: &str, command: Json) -> Result<String, (Outcome, String)> {
    engine::get()
        .call("QueueCommand", json!({ "chatId": chat, "command": command }))
        .await
        .map(|taken| taken["commandId"].as_str().unwrap_or_default().to_owned())
        .map_err(|e| {
            eprintln!("zeron-zgx: chat: {} for {chat} wasn't taken: {e}", command["kind"]);
            (engine::outcome(&e), e)
        })
}

/// A file uploaded to the device the chat runs on: its path there.
/// The most a file attached may weigh (the shipping app's
/// `MAX_ATTACHMENT_BYTES`).
const MAX_ATTACHMENT_BYTES: u64 = 24 * 1024 * 1024;

/// `work` on a thread of its own, its answer awaited here (None only if it
/// panicked): a file read and encoded off the window's thread.
async fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, mut rx) = zgx::task::latest();
    std::thread::spawn(move || {
        tx.send(work());
    });
    rx.recv().await
}

pub(crate) async fn upload(path: &str, name: &str, target: Option<&str>) -> Result<String, Outcome> {
    // Read and encoded off the window's thread, and no larger than the cap.
    let owned = path.to_owned();
    let remote = target.is_some();
    let encoded = off_thread(move || -> Result<String, Outcome> {
        let meta = std::fs::metadata(&owned).map_err(|_| "unreadable")?;
        if !meta.is_file() {
            return Err("not_file");
        }
        if meta.len() > MAX_ATTACHMENT_BYTES {
            return Err(if remote { "too_large_remote" } else { "too_large" });
        }
        let bytes = std::fs::read(&owned).map_err(|_| "unreadable")?;
        Ok(base64(&bytes))
    })
    .await
    .ok_or("failed")??;
    let upload_id = new_id();
    let routed = |mut p: Json| {
        if let Some(t) = target {
            p["targetDeviceId"] = json!(t);
        }
        p
    };
    let mut seq = 0;
    let mut at = 0;
    while at < encoded.len() || seq == 0 {
        let end = (at + CHUNK).min(encoded.len());
        engine::get()
            .call(
                "UploadChunk",
                routed(json!({ "uploadId": upload_id, "seq": seq, "data": &encoded[at..end] })),
            )
            .await
            .map_err(|e| engine::outcome(&e))?;
        seq += 1;
        at = end;
    }
    let file_name = std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(name);
    let committed = engine::get()
        .call("UploadCommit", routed(json!({ "uploadId": upload_id, "fileName": file_name })))
        .await
        .map_err(|e| engine::outcome(&e))?;
    committed["path"].as_str().map(Into::into).ok_or("failed")
}

/// A chat's config with a profile applied, or none: shown at once, the
/// engine's word following, its divider when the switch asked first.
async fn apply_profile(args: Value) -> Result<(), Outcome> {
    let chats = CHATS.with(Rc::clone);
    let id = args.field("id").map(text).unwrap_or_default();
    let chat = chats.chat(&id).ok_or("missing")?;
    let profile = match args.field("profile").map(text).filter(|p| !p.is_empty()) {
        Some(pid) => Some(crate::agent_profiles::find(&pid).ok_or("missing")?),
        None => None,
    };
    let config = crate::agent_profiles::apply(&chat["config"], profile.as_ref()).ok_or("failed")?;
    if let Some(c) = chats.chats.borrow_mut().get_mut(&id) {
        c["config"] = config.clone();
    }
    chats.publish_views();
    let mut params = json!({ "op": "setChatConfig", "chatId": id, "config": config });
    if let Some(note) = args.field("note").map(text).filter(|t| !t.is_empty()) {
        params["note"] = json!({ "text": note, "hint": args.field("note_hint").map(text) });
    }
    engine::get().call("Mutate", params).await.map(|_| ()).map_err(|e| engine::outcome(&e))
}

/// A model as its agent's catalog lists it (this device's, else the
/// composer's device's).
pub(crate) fn catalog_model(harness: &str, model: &str) -> Option<Json> {
    CHATS.with(|chats| {
        let find = |by: &HashMap<String, Vec<Json>>| by.get(harness).and_then(|l| l.iter().find(|m| m["id"].as_str() == Some(model)).cloned());
        let here = find(&chats.models.borrow());
        here.or_else(|| chats.remote.borrow().as_ref().and_then(|r| find(&r.models)))
    })
}

/// An effort's word, as the picker says it ("Extra high").
pub(crate) fn effort_word(level: &str) -> String {
    effort_label(level).to_owned()
}

pub(crate) fn agent_name(harness: &str) -> Option<String> {
    CHATS.with(|chats| chats.harness_name(harness))
}

/// An ACP registry agent's own icon (its SVG), as the catalog lists it.
pub(crate) fn agent_icon(harness: &str) -> Option<String> {
    CHATS.with(|chats| chats.harnesses.borrow().iter().find(|h| h["id"].as_str() == Some(harness)).and_then(|h| h["icon"].as_str().map(Into::into)))
}

pub(crate) fn chat_config(id: &str) -> Option<Json> {
    CHATS.with(|chats| chats.chat(id)).map(|c| c["config"].clone())
}

/// A new chat's picks as a config (`model`, `effort`, `access`, `options`,
/// `fast`, as `chat.send` reads them).
pub(crate) fn draft_config(args: &Value) -> Json {
    merged_config(&json!({}), args)
}

/// The profiles changed: each chat's comparison with its own again.
pub(crate) fn profiles_changed() {
    CHATS.with(|chats| chats.publish_views());
}

/// A mock Dev's seed (`ZERON_ZGX_MOCK_CATALOG`), nil elsewhere.
pub(crate) fn mock_seed() -> Option<Json> {
    if !std::env::var("ZERON_ZGX_MOCK").is_ok_and(|v| v == "1") {
        return None;
    }
    mock_catalog()
}

/// The chat's config with what the script changed: a model as
/// "<harness>/<model>", its effort, Fast, the access mode and plan mode.
/// Whether a run's config picks another agent, model, effort or model option
/// than the chat's.
fn runs_otherwise(chat: &Json, run: &Json) -> bool {
    ["harness", "model", "reasoning", "modelOptions"].iter().any(|k| {
        let a = chat.get(*k).filter(|v| !v.is_null());
        let b = run.get(*k).filter(|v| !v.is_null() && **v != json!({}));
        let a = a.filter(|v| **v != json!({}));
        a != b
    })
}

fn merged_config(config: &Json, args: &Value) -> Json {
    let mut c = if config.is_object() { config.clone() } else { json!({}) };
    if c.get("sandbox").is_none() {
        c["sandbox"] = json!("workspace-write");
    }
    if c.get("modelOptions").is_none() {
        c["modelOptions"] = json!({});
    }
    for key in ["harness", "model", "reasoning"] {
        if c.get(key).is_none() {
            c[key] = Json::Null;
        }
    }
    let field = |key: &str| args.field(key).map(text).filter(|s| !s.is_empty());
    if let Some(model) = field("model") {
        match model.split_once('/') {
            Some((harness, model)) => {
                c["harness"] = json!(harness);
                c["model"] = json!(model);
            }
            None => c["model"] = json!(model),
        }
    }
    if let Some(harness) = field("harness") {
        c["harness"] = json!(harness);
    }
    // An instance runs on its own agent. (A profile stays, drifted to another
    // agent, as gpui's `update_chat_config` keeps `profile_id`.)
    if c["harness"] != config["harness"]
        && let Some(map) = c.as_object_mut()
    {
        map.remove("instanceId");
    }
    if let Some(effort) = field("effort") {
        c["reasoning"] = json!(effort);
    }
    if let Some(access) = field("access") {
        match access.as_str() {
            "supervised" => c["access"] = json!("supervised"),
            "edits" => c["access"] = json!("auto-accept-edits"),
            _ => {
                if let Some(map) = c.as_object_mut() {
                    map.remove("access");
                }
            }
        }
    }
    if let Some(plan) = args.field("plan") {
        if plan.truthy() {
            c["plan"] = json!(true);
        } else if let Some(map) = c.as_object_mut() {
            map.remove("plan");
        }
    }
    // The model's Fast option, as the catalog lists it (nil: it has none,
    // or the catalog isn't in yet).
    let fast_of_model = {
        let chats = CHATS.with(Rc::clone);
        let harness = c["harness"].as_str().unwrap_or_default().to_owned();
        let model = c["model"].as_str().unwrap_or_default().to_owned();
        let found = chats
            .models
            .borrow()
            .get(&harness)
            .and_then(|l| l.iter().find(|m| m["id"].as_str() == Some(&model)).cloned());
        found.and_then(|m| fast_option(&m))
    };
    // The model's other options, picked whole: each option's choice by id
    // (one at its default is left out, as the shipping app leaves it);
    // Fast's own stays as it is (`fast` below sets it).
    if let Some(Value::Map(picked)) = args.field("options") {
        let mut next = serde_json::Map::new();
        if let Some((key, _, _)) = &fast_of_model
            && let Some(v) = c["modelOptions"].get(key)
        {
            next.insert(key.clone(), v.clone());
        }
        for (key, choice) in picked.iter() {
            if let Value::Str(choice) = choice
                && !choice.is_empty()
            {
                next.insert(key.to_string(), json!(&**choice));
            }
        }
        c["modelOptions"] = Json::Object(next);
    }
    if let Some(fast) = args.field("fast")
        && let Some((key, on, off)) = fast_of_model
    {
        c["modelOptions"][key] = json!(if fast.truthy() { on } else { off });
    }
    c
}

/// The prompt with its attachments named after it, as the shipping app
/// writes it (crates/ui/src/attachments.rs).
fn with_attachments(words: &str, paths: &[String], images: usize) -> String {
    if paths.is_empty() {
        return words.to_owned();
    }
    let body = if !words.trim().is_empty() {
        words.to_owned()
    } else if images == paths.len() {
        "See the attached image(s).".into()
    } else {
        "See the attached file(s).".into()
    };
    let mut out = format!("{body}\n\nAttached images (local files — open them to view):");
    for p in paths {
        out.push_str("\n- ");
        out.push_str(p);
    }
    out
}

/// A person's words, without the attachments' trailer, and the paths it
/// named.
fn split_attachments(prompt: &str) -> (String, Vec<String>) {
    const TRAILER: &str = "\n\nAttached images (local files — open them to view):";
    let Some(at) = prompt.find(TRAILER) else {
        return (prompt.to_owned(), Vec::new());
    };
    let paths = prompt[at + TRAILER.len()..]
        .lines()
        .filter_map(|l| l.strip_prefix("- "))
        .map(|l| l.trim().to_owned())
        .collect();
    let body = &prompt[..at];
    let body = match body {
        "See the attached image(s)." | "See the attached file(s)." => "",
        other => other,
    };
    (body.to_owned(), paths)
}

/// The paths a request's attachments have on this device.
fn local_paths(req: &Value) -> Vec<String> {
    req.field("attachments")
        .map(Value::list)
        .unwrap_or_default()
        .iter()
        .filter_map(|a| a.field("path").map(text))
        .filter(|p| !p.is_empty())
        .collect()
}

/// Text staged on the composer and folded into the prompt as Zeron writes it
/// (zeron_proto quotes.rs, which ds/quote.luau writes byte for byte): its
/// plain selections' block, then its annotations', lifted off the words as the
/// shipping app's badges (zeron_proto badges.rs), so a chat sent from either
/// app reads the same in both. Each badge a sent message's pill: what it
/// quotes, by number and file, with its comment.
fn quote_badges(words: &str) -> (String, Vec<Value>) {
    use zeron_proto::badges::{ANNOTATION_COMMENT_LABEL, SELECTION_LOCATION, extract_annotation_badge, extract_selection_badge};
    let mut rest = words.to_owned();
    let mut out = Vec::new();
    if let Some((left, badge)) = extract_selection_badge(&rest) {
        rest = left;
        let details: Vec<Value> = badge.details.iter().map(|d| Value::record([("text", Value::from(d.body.as_str()))])).collect();
        out.push(("selections", details));
    }
    if let Some((left, badge)) = extract_annotation_badge(&rest) {
        rest = left;
        let said = format!("\n\n{ANNOTATION_COMMENT_LABEL} ");
        let details: Vec<Value> = badge
            .details
            .iter()
            .enumerate()
            .map(|(ix, d)| {
                let place = d.location.split_once(". ").map_or("", |(_, place)| place);
                let (text, comment) = d.body.split_once(&said).map_or((d.body.as_str(), None), |(t, c)| (t, Some(c)));
                let mut fields = vec![("n", Value::from((ix + 1) as f64)), ("text", Value::from(text))];
                if place != SELECTION_LOCATION && !place.is_empty() {
                    fields.push(("path", Value::from(place)));
                }
                if let Some(comment) = comment {
                    fields.push(("comment", Value::from(comment)));
                }
                Value::record(fields)
            })
            .collect();
        out.insert(0, ("annotations", details));
    }
    if out.is_empty() {
        return (words.to_owned(), Vec::new());
    }
    let badges = out
        .into_iter()
        .map(|(badge, details)| Value::record([("kind", Value::from("badge")), ("badge", Value::from(badge)), ("name", Value::from("")), ("details", Value::List(details.into()))]))
        .collect();
    (rest, badges)
}

/// The block an earlier zgx added for passages attached from the conversation:
/// its header, then `- who, when: words`, a further line of words set in two.
/// Still read, so a chat sent from it shows its chips.
const QUOTES_HEADER: &str = "Selected from this conversation:";

/// A prompt's words apart from its attached passages (where each was said,
/// and its words), as the shipping app parts review comments from a prompt
/// (crates/ui comments.rs `extract_badge`): only a whole trailing block.
fn split_quotes(words: &str) -> (String, Vec<(String, String)>) {
    split_block(words, QUOTES_HEADER)
}

/// The block the composer adds for review comments left on a file (the
/// shipping app's comments.rs `with_comments`): its header, then `- path:line:
/// body`, a further line of a body set in two.
const COMMENTS_HEADER: &str = "Review comments (each cites the workspace file and line it belongs to):";

/// The block for comments left on a diff (the shipping app's
/// `COMMENT_BLOCK_HEADER`): each `- path:line (L|R): body`, a range
/// `path:10-14`.
const DIFF_COMMENTS_HEADER: &str = "Comments on the diff (each cites the file and line it belongs to; L = line number in the original file, R = in the changed file):";

/// A prompt's words apart from its review comments (each its file and line,
/// and its words): only a whole trailing block, as `extract_badge` takes it,
/// under either header.
fn split_comments(words: &str) -> (String, Vec<(String, String)>) {
    let (mut head, mut bullets) = split_block_raw(words, COMMENTS_HEADER);
    if bullets.is_empty() {
        (head, bullets) = split_block_raw(words, DIFF_COMMENTS_HEADER);
    }
    let comments = bullets
        .into_iter()
        .map(|bullet| {
            // "path:line (R): body" (a side's marker, the earliest), or
            // "path:line: body" (the first ": " after a line number).
            let side = [" (L): ", " (R): "].into_iter().filter_map(|m| bullet.find(m).map(|at| (at, m.len()))).min_by_key(|(at, _)| *at);
            let mut cut = None;
            for (at, _) in bullet.match_indices(": ") {
                let before = &bullet[..at];
                if before.rsplit_once(':').is_some_and(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit() || b == b'-')) {
                    cut = Some((at, 2));
                    break;
                }
            }
            let cut = match (side, cut) {
                (Some(s), Some(c)) => Some(if s.0 < c.0 { s } else { c }),
                (s, c) => s.or(c),
            };
            match cut {
                Some((at, len)) => (bullet[..at].to_owned(), bullet[at + len..].to_owned()),
                None => (String::new(), bullet),
            }
        })
        .collect();
    (head, comments)
}

/// A comment chip on the sent message (the shipping app's badge): how many,
/// and where they are.
fn comments_chip(comments: &[(String, String)]) -> Value {
    let label = if comments.len() == 1 { "1 comment".to_owned() } else { format!("{} comments", comments.len()) };
    let places = comments.iter().map(|(place, _)| place.as_str()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", ");
    Value::record([
        ("kind", Value::from("file")),
        ("icon", Value::from("chat-round-line")),
        ("name", Value::from(label.as_str())),
        ("meta", Value::from(places.as_str())),
    ])
}

/// A trailing block under `header` of `- …` bullets (continued in two):
/// the words before it and each bullet's text, joined with its continuation.
fn split_block_raw(words: &str, header: &str) -> (String, Vec<String>) {
    let marker = format!("\n\n{header}\n");
    let start = format!("{header}\n");
    let (head, block) = if let Some(at) = words.rfind(&marker) {
        (&words[..at], &words[at + marker.len()..])
    } else if let Some(rest) = words.strip_prefix(&start) {
        ("", rest)
    } else {
        return (words.to_owned(), Vec::new());
    };
    if block.is_empty() || !block.lines().all(|l| l.starts_with("- ") || l.starts_with("  ")) {
        return (words.to_owned(), Vec::new());
    }
    let mut out: Vec<String> = Vec::new();
    for line in block.lines() {
        if let Some(bullet) = line.strip_prefix("- ") {
            out.push(bullet.to_owned());
        } else if let (Some(more), Some(last)) = (line.strip_prefix("  "), out.last_mut()) {
            last.push('\n');
            last.push_str(more);
        }
    }
    (head.to_owned(), out)
}

/// A trailing block of `- place: words` bullets under `header`.
fn split_block(words: &str, header: &str) -> (String, Vec<(String, String)>) {
    let (head, bullets) = split_block_raw(words, header);
    let pairs = bullets
        .into_iter()
        .map(|bullet| match bullet.split_once(": ") {
            Some((place, body)) => (place.to_owned(), body.to_owned()),
            None => (String::new(), bullet),
        })
        .collect();
    (head, pairs)
}

/// A passage's chip on the sent message (as the composer showed it): where
/// it was said, its words on one line, cut at a word.
fn quote_chip(place: &str, body: &str) -> Value {
    let line = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let excerpt = if line.chars().count() <= 72 {
        line
    } else {
        let cut: String = line.chars().take(72).collect();
        match cut.rfind(' ') {
            Some(at) => format!("{}…", &cut[..at]),
            None => format!("{cut}…"),
        }
    };
    Value::record([
        ("kind", Value::from("file")),
        ("icon", Value::from("cursor")),
        ("name", Value::from(place)),
        ("meta", Value::from(excerpt.as_str())),
    ])
}

/// The shipping app's browser annotation token (crates/proto annotation.rs):
/// `[Annotation N](zeron-annotation:<hex JSON>)`, as ds/annotation.luau
/// writes it after a prompt's words.
const ANNOTATION_SCHEME: &str = "](zeron-annotation:";

/// An element picked on a page, as a sent message shows it (the composer's
/// chip, ds/composer_cargo): its page, then its number and what it says.
struct Pick {
    index: u32,
    element: String,
    url: String,
    words: String,
}

/// A prompt's words apart from its annotation tokens (each read as its
/// label, "Annotation N", a line of nothing but a label dropped), and the
/// picks they stood for.
fn split_annotations(words: &str) -> (String, Vec<Pick>) {
    if !words.contains(ANNOTATION_SCHEME) {
        return (words.to_owned(), Vec::new());
    }
    let mut out = String::with_capacity(words.len());
    let mut picks = Vec::new();
    let mut rest = words;
    while let Some(at) = rest.find(ANNOTATION_SCHEME) {
        let head = &rest[..at];
        let Some(open) = head.rfind('[') else {
            out.push_str(&rest[..at + ANNOTATION_SCHEME.len()]);
            rest = &rest[at + ANNOTATION_SCHEME.len()..];
            continue;
        };
        let label = &head[open + 1..];
        let hex_start = at + ANNOTATION_SCHEME.len();
        let Some(close) = rest[hex_start..].find(')') else {
            out.push_str(rest);
            rest = "";
            break;
        };
        let hex = &rest[hex_start..hex_start + close];
        let pick = (hex.len() % 2 == 0)
            .then(|| {
                (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
                    .collect::<Option<Vec<u8>>>()
            })
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<Json>(&bytes).ok())
            .map(|json| {
                let text = |key: &str| json[key].as_str().unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
                let words = [text("text"), text("name")].into_iter().find(|w| !w.is_empty()).unwrap_or_default();
                Pick {
                    index: json["index"].as_u64().unwrap_or(0) as u32,
                    element: json["element"].as_str().unwrap_or_default().to_owned(),
                    url: json["url"].as_str().unwrap_or_default().to_owned(),
                    words,
                }
            });
        match pick {
            Some(pick) => {
                out.push_str(&head[..open]);
                out.push_str(label);
                picks.push(pick);
            }
            None => out.push_str(&rest[..hex_start + close + 1]),
        }
        rest = &rest[hex_start + close + 1..];
    }
    out.push_str(rest);
    // A line that was only a token says nothing now.
    let kept: Vec<&str> = out
        .lines()
        .filter(|l| !(l.starts_with("Annotation ") && l[11..].trim().chars().all(|c| c.is_ascii_digit())))
        .collect();
    let mut body = kept.join("\n");
    while body.ends_with('\n') {
        body.pop();
    }
    (body, picks)
}

/// A page's address as a chip names it: no scheme, no "www.", no lone
/// trailing slash (ds/composer_cargo's `page_of`).
fn page_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    match rest.strip_suffix('/') {
        Some(host) if !host.contains('/') => host.to_owned(),
        _ => rest.to_owned(),
    }
}

/// The chip a sent pick shows, as the composer's did: its page over its
/// number and what it says (else its element).
fn annotation_chip(pick: &Pick) -> Value {
    let label = format!("Annotation {}", pick.index);
    let page = page_of(&pick.url);
    let what = if pick.words.is_empty() { pick.element.as_str() } else { pick.words.as_str() };
    let (name, meta) = if page.is_empty() {
        (label, pick.element.clone())
    } else {
        (page, format!("{label} · {what}"))
    };
    Value::record([
        ("kind", Value::from("file")),
        ("icon", Value::from("cursor")),
        ("name", Value::from(name.as_str())),
        ("meta", Value::from(meta.as_str())),
    ])
}

/// The chips a sent prompt carries: its files and pictures, then its
/// passages, then the elements picked on a page.
fn carried_value(paths: &[String], quotes: &[(String, String)], picks: &[Pick], comments: &[(String, String)], badges: &[Value]) -> Value {
    let Value::List(files) = attachments_value(paths) else { unreachable!() };
    let mut all: Vec<Value> = files.iter().cloned().collect();
    all.extend(badges.iter().cloned());
    all.extend(quotes.iter().map(|(place, body)| quote_chip(place, body)));
    all.extend(picks.iter().map(annotation_chip));
    if !comments.is_empty() {
        all.push(comments_chip(comments));
    }
    Value::List(all.into())
}

fn attachments_value(paths: &[String]) -> Value {
    Value::List(
        paths
            .iter()
            .map(|p| {
                let name = std::path::Path::new(p)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(p);
                let image = is_image(p);
                let mut fields = vec![
                    ("kind", Value::from(if image { "image" } else { "file" })),
                    ("name", Value::from(name)),
                    ("path", Value::from(p.as_str())),
                ];
                if image {
                    fields.push(("src", Value::from(p.as_str())));
                }
                Value::record(fields)
            })
            .collect::<Vec<_>>()
            .into(),
    )
}

/// A picture the window can draw (zgx decodes no GIF or HEIC yet: those go
/// as files, so a thumbnail never fails).
fn is_image(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".webp"]
        .iter()
        .any(|e| lower.ends_with(e))
}

// -- the transcript -----------------------------------------------------------

/// What a frame changed: only words of a reply being written, rows, or
/// nothing that shows (more of a tool's output, which no row shows).
enum Change {
    Quiet,
    Words,
    Rows,
}

/// The model's name a chat's replies carry, building them again if it's
/// another now, and the device its visuals are read from (the chat's host,
/// when it isn't this one).
fn build_context(open: &mut Open, chats: &Chats) -> Rc<RowContext> {
    let chat = chats.chat(&open.id).unwrap_or(Json::Null);
    let harness = chat["config"]["harness"].as_str().unwrap_or("claude-code").to_owned();
    let model = chat["config"]["model"].as_str().map(|m| chats.label_of(&harness, m).unwrap_or_else(|| m.to_owned()));
    if open.built_model.as_ref() != Some(&model) {
        // (Settled entries are the engine's now: the rows show the last name
        // until its copy comes, `model_moved`, and are built again from that.)
        if open.entries.iter().all(|e| e.json.is_some()) {
            open.built.clear();
        } else {
            open.model_moved = true;
        }
        open.built_model = Some(model.clone());
    }
    let device = chats.target(&chat);
    let cwd = chat["cwd"].as_str().map(str::to_owned);
    code_links::set_root(cwd.as_deref(), device.is_none());
    match &open.row_context {
        Some(context) if context.doc == open.id && context.device == device && context.model == model && context.cwd == cwd => context.clone(),
        _ => {
            let context = Rc::new(RowContext { doc: open.id.clone(), device, model, cwd });
            open.row_context = Some(context.clone());
            context
        }
    }
}

/// An entry as its row reads it.
fn built_entry(entry: &Json, doc: &str, device: Option<&str>, model: Option<&str>) -> Option<Value> {
    // (One still being written is built again with its next change.)
    let settled = entry["status"].as_str() != Some("streaming");
    if settled {
        ROW_BUILD.set(Some(RowBuild::default()));
    }
    let mut row = match with_host(entry, doc, device) {
        Some(entry) => message(&entry, model),
        None => message(entry, model),
    };
    if let Some(mut build) = ROW_BUILD.take()
        && let Some(row) = &mut row
    {
        share_parts(row, &mut build.seen);
    }
    row
}

/// The newest plan in these entries, newest first.
fn latest_todo<'a>(newest_first: impl Iterator<Item = &'a Json>) -> Option<&'a Json> {
    newest_first
        .flat_map(|e| e["parts"].as_array().into_iter().flatten().rev())
        .find(|p| p["kind"].as_str() == Some("tool") && p["call"]["kind"].as_str() == Some("todo"))
}

/// What the worker made of an entry, kept: its row folded, as `built_for` and
/// `settle` would have left it.
fn settle_off(open: &mut Open, folded: FoldedOff, context: &Rc<RowContext>) {
    let FoldedOff { at, id, bytes, subagents, sum, todo } = folded;
    // (Where it stood, unless a frame has moved it. Gone: nothing to keep.)
    let Open { entries, built, older_todo, .. } = open;
    let found = entries.get(at).filter(|e| *e.id == *id).map(|_| at).or_else(|| entries.iter().position(|e| *e.id == *id));
    let Some(place) = found else { return };
    let entry = &mut entries[place];
    // (A newer copy came meanwhile: it's built from that.)
    if entry.json.is_some() {
        return;
    }
    entry.sum = sum;
    if let Some(part) = todo
        && older_todo.as_ref().is_none_or(|(kept, _)| *kept < place)
    {
        *older_todo = Some((place, part));
    }
    let row = bytes.map(|bytes| Value::folded(entry.id.clone(), FoldedEntry { bytes, context: context.clone() }));
    let subagents = (!subagents.is_empty()).then(|| subagents.iter().map(SubagentOff::value).collect());
    built.insert(entry.id.clone(), Built { row, subagents });
}

/// `entry`, built, down to its id and sum; a plan in it is kept (`todos`).
fn settle(entry: &mut Entry, at: usize, older_todo: &mut Option<(usize, Json)>) {
    let Some(json) = entry.json.take() else { return };
    entry.sum = sum_of(&json);
    if older_todo.as_ref().is_none_or(|(place, _)| *place < at)
        && let Some(part) = latest_todo(std::iter::once(&json))
    {
        *older_todo = Some((at, part.clone()));
    }
}

/// A row's own repeats, one each: a reply holds its tools in `tools`, in
/// `blocks` and in `compact`, its words in `text`, `copy` and a block, and
/// every record its keys. Only one of each stays, shared.
#[derive(Default)]
struct Seen {
    words: std::collections::HashSet<Rc<str>>,
    records: HashMap<Rc<str>, Value>,
}

/// A short word is kept for every row, while there are few of them.
const COMMON_LEN: usize = 16;
const COMMON_MOST: usize = 4096;
thread_local! {
    static COMMON: RefCell<std::collections::HashSet<Rc<str>>> = RefCell::default();
    /// The settled row being built (`built_entry`): its tools so far, by the
    /// part each is of, each built once however many of its layouts show it.
    static ROW_BUILD: RefCell<Option<RowBuild>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct RowBuild {
    seen: Seen,
    tools: HashMap<*const Json, Option<Value>>,
}

fn share_word(text: &mut Rc<str>, own: &mut std::collections::HashSet<Rc<str>>) {
    if text.len() <= COMMON_LEN {
        let shared = COMMON.with_borrow_mut(|common| {
            if let Some(known) = common.get(&**text) {
                return Some(known.clone());
            }
            (common.len() < COMMON_MOST).then(|| {
                common.insert(text.clone());
                text.clone()
            })
        });
        if let Some(shared) = shared {
            *text = shared;
            return;
        }
    }
    match own.get(&**text) {
        Some(known) => *text = known.clone(),
        None => {
            own.insert(text.clone());
        }
    }
}

fn share_parts(value: &mut Value, seen: &mut Seen) {
    match value {
        Value::Str(text) => share_word(text, &mut seen.words),
        Value::List(items) => {
            if let Some(items) = Rc::get_mut(items) {
                items.iter_mut().for_each(|item| share_parts(item, seen));
            }
        }
        Value::Map(fields) => {
            if let Some(fields) = Rc::get_mut(fields) {
                for (key, field) in fields.iter_mut() {
                    share_word(key, &mut seen.words);
                    share_parts(field, seen);
                }
            }
            // (A record with an id, as a tool is: its one copy.)
            let Some(Value::Str(id)) = value.field("id").cloned() else { return };
            match seen.records.get(&id) {
                Some(known) if *known == *value => *value = known.clone(),
                Some(_) => {}
                None => {
                    seen.records.insert(id, value.clone());
                }
            }
        }
        _ => {}
    }
}

/// Entries by their ids.
fn by_id(entries: &[Entry]) -> HashMap<&str, &Entry> {
    entries.iter().map(|e| (&*e.id, e)).collect()
}

/// `built` less what a reset changed: only the entries it has as they were.
fn keep_unchanged(built: &mut HashMap<Rc<str>, Built>, before: &[Entry], now: &[Entry]) {
    if built.is_empty() {
        return;
    }
    let (before, now) = (by_id(before), by_id(now));
    built.retain(|id, _| {
        let id: &str = id;
        before.get(id).zip(now.get(id)).is_some_and(|(was, is)| was.sum() == is.sum())
    });
}

/// Into `open.built`, the values built from `entries` for those of its own
/// entries that are the same. (Built for another model's name: none.)
fn reuse_built(
    open: &mut Open,
    entries: &[Entry],
    built: &HashMap<Rc<str>, Built>,
    built_model: &Option<Option<String>>,
) {
    if built_model.is_none() || (open.built_model.is_some() && open.built_model != *built_model) {
        return;
    }
    open.built_model = built_model.clone();
    let theirs = by_id(entries);
    for entry in &open.entries {
        if !entry.id.is_empty()
            && !open.built.contains_key(&entry.id)
            && let Some(value) = built.get(&entry.id)
            && theirs.get(&*entry.id).is_some_and(|their| their.sum() == entry.sum())
        {
            open.built.insert(entry.id.clone(), value.clone());
        }
    }
}

/// A frame applied to `entries` (crates/doc/src/transcript_delta.rs's
/// `apply_transcript_frame`); `Err` when the copy is out of step.
fn apply(entries: &mut Vec<Entry>, frame: &mut Json) -> Result<Change, ()> {
    if let Some(reset) = frame.get_mut("reset") {
        // (Taken, not copied: a long chat's whole transcript, twice over.)
        *entries = match reset.take() {
            Json::Array(list) => list.into_iter().map(Entry::new).collect(),
            _ => Vec::new(),
        };
        return Ok(Change::Rows);
    }
    let frame = &*frame;
    let mut rows = false;
    let mut words_changed = false;
    for id in frame["remove"].as_array().into_iter().flatten() {
        let before = entries.len();
        entries.retain(|e| id.as_str() != Some(&*e.id));
        rows |= entries.len() != before;
    }
    for up in frame["upsert"].as_array().into_iter().flatten() {
        let entry = &up["entry"];
        let id = entry["id"].as_str().unwrap_or_default();
        entries.retain(|e| &*e.id != id);
        let at = match up["after"].as_str() {
            None => 0,
            Some(after) => entries
                .iter()
                .position(|e| &*e.id == after)
                .map(|i| i + 1)
                .ok_or(())?,
        };
        entries.insert(at, Entry::new(entry.clone()));
        rows = true;
    }
    for add in frame["append"].as_array().into_iter().flatten() {
        let entry = entries
            .iter_mut()
            .find(|e| add["entry"].as_str() == Some(&*e.id))
            .ok_or(())?;
        // (A settled entry growing again: its whole copy is the engine's.)
        let entry = entry.json.as_mut().ok_or(())?;
        let streaming = entry["status"].as_str() == Some("streaming");
        let part = entry["parts"]
            .as_array_mut()
            .and_then(|parts| parts.iter_mut().find(|p| p["id"] == add["part"]))
            .ok_or(())?;
        let kind = part["kind"].as_str().unwrap_or_default().to_owned();
        let more = add["text"].as_str().unwrap_or_default();
        // Grown in place (a long reply isn't copied whole for each word).
        let was_empty = match part.get_mut("text") {
            Some(Json::String(text)) => {
                let empty = text.trim().is_empty();
                text.push_str(more);
                empty
            }
            _ => {
                part["text"] = Json::String(more.to_owned());
                true
            }
        };
        let len = part["text"].as_str().map_or(0, str::len);
        if add["len"].as_u64().is_some_and(|want| want as usize != len) {
            return Err(());
        }
        match kind.as_str() {
            // Words of the reply being written: the draft; of a finished
            // one, its row.
            "text" if streaming => words_changed = true,
            "text" => rows = true,
            // Thinking shows as a row once it has words; more of them, as
            // it's written, are the draft's, as a reply's are. (More of a
            // tool's output shows nothing new.)
            "reasoning" if was_empty && !more.trim().is_empty() => rows = true,
            "reasoning" if streaming => words_changed = true,
            _ => {}
        }
    }
    if frame["count"].as_u64().is_some_and(|n| n as usize != entries.len()) {
        return Err(());
    }
    Ok(if rows {
        Change::Rows
    } else if words_changed {
        Change::Words
    } else {
        Change::Quiet
    })
}

/// `entry` with what its parts read from the chat's host: each visual names
/// `device`, its host, when that isn't this one; each command whose output
/// the doc cut to its last lines names `doc`, the doc it is in, so its row
/// can ask the host for the rest (`chat.tool_output`). Nil when it has
/// neither: the entry as it is.
fn with_host(entry: &Json, doc: &str, device: Option<&str>) -> Option<Json> {
    let parts = entry["parts"].as_array()?;
    let visual = |p: &Json| device.is_some() && (p["kind"].as_str() == Some("visual") || p["htmlRender"].is_object());
    let cut = |p: &Json| p["kind"].as_str() == Some("tool") && p["outputTruncated"].as_bool() == Some(true);
    if !parts.iter().any(|p| visual(p) || cut(p)) {
        return None;
    }
    let mut entry = entry.clone();
    for part in entry["parts"].as_array_mut().into_iter().flatten() {
        if visual(part)
            && let Some(device) = device
        {
            part["device"] = Json::String(device.to_owned());
        }
        if cut(part) {
            part["chat"] = Json::String(doc.to_owned());
        }
    }
    Some(entry)
}

/// The device a read of `doc`'s host data goes to (`targetDeviceId`), when
/// that isn't this one: a subagent's doc (`{chat}--sub--{id}`) is its root
/// chat's.
pub(crate) fn host_of(doc: &str) -> Option<String> {
    let chats = CHATS.with(Rc::clone);
    let source = doc.split_once("--sub--").map_or(doc, |(chat, _)| chat);
    chats.target(&chats.chat(source)?)
}

/// An entry's words: its text parts, each a paragraph.
fn entry_text(entry: &Json) -> String {
    let mut out = String::new();
    for p in entry["parts"].as_array().into_iter().flatten() {
        let words = match p["kind"].as_str() {
            Some("text") => p["text"].as_str().unwrap_or_default().to_owned(),
            Some("error") => format!("> {}", p["message"].as_str().unwrap_or_default()),
            _ => continue,
        };
        if words.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&words);
    }
    out
}

/// A transcript's subagents: each spawn that names its doc, once, in the order they
/// were spawned, with when their turn began (a message's `tools` hold all of its).
fn subagents_in(message: &Value) -> Option<Rc<[Value]>> {
    let Some(Value::List(tools)) = message.field("tools") else { return None };
    let started = message.field("started").cloned().unwrap_or(Value::Nil);
    let found: Vec<Value> = tools
        .iter()
        .filter_map(|tool| {
            let sub = tool.field("subagent")?;
            sub.field("ref").filter(|r| matches!(r, Value::Str(_)))?;
            Some(pills::subagent_row(sub, tool, started.clone(), true))
        })
        .collect();
    (!found.is_empty()).then(|| found.into())
}

/// A transcript's subagents as its rows are added, each once, in order; those
/// spawned before its latest prompt aren't `recent`.
#[derive(Default)]
struct Subagents(Vec<Value>, std::collections::HashSet<Rc<str>>, usize);

impl Subagents {
    fn add(&mut self, found: Option<&[Value]>) {
        for sub in found.into_iter().flatten() {
            if let Some(Value::Str(reference)) = sub.field("ref")
                && self.1.insert(reference.clone())
            {
                self.0.push(sub.clone());
            }
        }
    }

    fn row(&mut self, row: &Value) {
        if is_prompt(row) {
            self.2 = self.0.len();
        }
    }

    fn rows(self) -> Vec<Value> {
        let Subagents(rows, _, recent_from) = self;
        rows.into_iter()
            .enumerate()
            .map(|(at, row)| if at < recent_from { with_fields(row, [("recent", Value::Bool(false))]) } else { row })
            .collect()
    }
}

/// A prompt of the person's (not a question's answers). A folded row never is
/// one (`kept_whole`), and isn't opened to tell.
fn is_prompt(row: &Value) -> bool {
    !matches!(row, Value::Folded(_))
        && matches!(row.field("role"), Some(Value::Str(r)) if &**r == "user")
        && matches!(row.field("answers"), None | Some(Value::Nil))
}

/// An entry whose row each publish reads (the prompts, `merge_compaction_rows`'
/// lines): never folded, so reading it never opens one.
fn kept_whole(entry: &Json) -> bool {
    entry["role"].as_str() == Some("user")
        || entry["parts"].as_array().into_iter().flatten().any(|p| matches!(p["kind"].as_str(), Some("classifier" | "compaction")))
}

fn resumed_row(id: &str) -> Value {
    Value::record([
        ("id", Value::from(id)),
        ("role", Value::from("system")),
        ("line", Value::from("resumed")),
        ("text", Value::from("Resumed")),
        ("hint", Value::from(zeron_proto::RESUME_PROMPT)),
    ])
}

/// An entry as ds/transcript's message; nil for one it doesn't show.
pub(crate) fn message(entry: &Json, model: Option<&str>) -> Option<Value> {
    let id = entry["id"].as_str()?;
    match entry["role"].as_str()? {
        "user" => {
            let text = entry["parts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| p["kind"].as_str() == Some("text"))
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            if text.trim() == zeron_proto::RESUME_PROMPT {
                return Some(resumed_row(id));
            }
            // The shipping app's `wake_task_ids`, `step_handback_threads`.
            let wake = zeron_proto::transcript::wake_task_ids(&text);
            let steps = zeron_proto::transcript::step_handback_threads(&text);
            let text = handback_quotes(&text).unwrap_or(text);
            // A message from another chat: its routing header is the agent's to
            // read; the transcript shows who sent it, over its words.
            let sender = zeron_proto::agent_messages::agent_message_sender(&text);
            let body = sender.as_ref().map_or(text.as_str(), |(_, body)| *body);
            let sender = sender.as_ref().map_or(Value::Nil, |(from, _)| {
                Value::record([("chat", Value::from(from.chat_id.as_str())), ("title", Value::from(from.title.as_str()))])
            });
            let (words, mut paths) = split_attachments(body);
            // (An Appshot's context isn't what the person said: its picture
            // says which app instead.)
            let (words, shots) = crate::appshots::split_appshots(&words);
            let (words, comments) = split_comments(&words);
            let (words, badges) = quote_badges(&words);
            let (words, quotes) = split_quotes(&words);
            let (words, picks) = split_annotations(&words);
            let (words, chosen) = page_picks(&words);
            let badges: Vec<Value> = badges.into_iter().chain(chosen).collect();
            for p in entry["parts"].as_array().into_iter().flatten() {
                if p["kind"].as_str() == Some("image")
                    && let Some(path) = p["path"].as_str()
                    && !paths.iter().any(|q| q == path)
                {
                    paths.push(path.to_owned());
                }
            }
            // Always its time under it, as the shipping app's prompts carry
            // it (the strip shows on hover), and its words to copy.
            let created = entry["createdAt"].as_f64().unwrap_or(0.) / 1000.;
            Some(Value::record([
                ("id", Value::from(id)),
                ("role", Value::from("user")),
                ("text", Value::from(words.as_str())),
                ("attachments", crate::appshots::tag_attachments(carried_value(&paths, &quotes, &picks, &comments, &badges), &shots)),
                ("at", stamp(created as i64).map_or(Value::Nil, |at| Value::from(at.as_str()))),
                ("created", Value::Num(created)),
                ("copy", copy_value(&words)),
                ("sender", sender),
                ("answers", entry["answersRequestId"].as_str().map_or(Value::Nil, Value::from)),
                ("from_chat", entry["fromChatId"].as_str().filter(|f| !f.is_empty()).map_or(Value::Nil, Value::from)),
                ("wake", wake.map_or(Value::Nil, |ids| Value::List(ids.iter().map(|id| Value::from(id.as_str())).collect::<Vec<_>>().into()))),
                ("steps", if steps.is_empty() { Value::Nil } else { Value::List(steps.iter().map(|id| Value::from(id.as_str())).collect::<Vec<_>>().into()) }),
                ("clock", stamp(created as i64).and_then(|at| at.rsplit_once(", ").map(|(_, clock)| clock.to_owned())).map_or(Value::Nil, |c| Value::from(c.as_str()))),
            ]))
        }
        "assistant" => {
            let streaming = entry["status"].as_str() == Some("streaming");
            let created = entry["createdAt"].as_f64().unwrap_or(0.) / 1000.;
            let tools: Vec<Value> = entry["parts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| !display_tool(p))
                .filter_map(tool)
                .collect();
            // A settled reply with nothing to show (only an empty thought,
            // say) has no row, as in the shipping app: drawn, it would be an
            // empty lane whose time floats over the next reply.
            let mut shown = blocks(entry);
            if !streaming && matches!(&shown, Value::List(list) if list.is_empty()) {
                return None;
            }
            let files = html_files_block(entry);
            if let Some(files) = files.clone() {
                shown = push_block(shown, files);
            }
            let mut fields = vec![
                ("id", Value::from(id)),
                ("role", Value::from("assistant")),
                ("text", Value::from(entry_text(entry).as_str())),
                ("tools", Value::List(tools.into())),
                ("blocks", shown),
                ("streaming", Value::Bool(streaming)),
                ("started", Value::Num(created)),
            ];
            if let Some(model) = model {
                fields.push(("model", Value::from(model)));
            }
            // Its time and words to copy once it has settled, as the
            // shipping app's strip under a reply ("No timestamp hover
            // mid-stream").
            if !streaming {
                if let Some(at) = stamp(created as i64) {
                    fields.push(("at", Value::from(at.as_str())));
                }
                fields.push(("copy", copy_value(&reply_copy_text(entry))));
            }
            if let Some(ms) = entry["durationMs"].as_f64() {
                fields.push(("worked", Value::Num((ms / 1000.).round().max(1.))));
            }
            // The same reply laid out for Settings' compact transcript.
            if let Some(compact) = compact_blocks(entry) {
                fields.push(("compact", match files {
                    Some(files) => push_block(compact, files),
                    None => compact,
                }));
            }
            Some(Value::record(fields))
        }
        // The provider-switch seam (the engine's system entry when a chat
        // moves to another agent): `[mark] From → [mark] To` over the prompt
        // that switched, as the shipping app's `switch_marker`.
        "system" => {
            let Some(part) = entry["parts"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|p| p["kind"].as_str() == Some("switch"))
                .cloned()
                .or_else(|| handoff_switch(entry))
            else {
                return system_row(entry, id);
            };
            let side = |name: &str, harness: &str| {
                let mark = part[harness].as_str().and_then(switch_mark);
                (
                    Value::from(single_line(part[name].as_str().unwrap_or_default()).as_str()),
                    mark.map_or(Value::Nil, |m| Value::from(m.as_str())),
                )
            };
            let (from, from_mark) = side("from", "fromHarness");
            let (to, to_mark) = side("to", "toHarness");
            // (Its handoff is text the new agent reads: the picker's switch dialog counts it as history.)
            let handoff = entry["parts"].as_array().into_iter().flatten().any(|p| p["kind"].as_str() == Some("handoff"));
            Some(Value::record([
                ("id", Value::from(id)),
                ("role", Value::from("switch")),
                ("text", Value::from("")),
                ("from", from),
                ("to", to),
                ("from_mark", from_mark),
                ("to_mark", to_mark),
                ("handoff", Value::Bool(handoff)),
            ]))
        }
        _ => None,
    }
}

fn system_row(entry: &Json, id: &str) -> Option<Value> {
    use zeron_proto::computer::cards::{ComputerNotice, computer_notice, revoked_label};
    let parts = entry["parts"].as_array()?;
    let part = |kind: &str| parts.iter().find(|p| p["kind"].as_str() == Some(kind));
    let texts: Vec<&str> = parts.iter().filter(|p| p["kind"].as_str() == Some("text")).filter_map(|p| p["text"].as_str()).collect();
    let text = texts.first().copied().unwrap_or_default();
    let hint = texts.get(1).copied();
    let row = |line: &str, words: &str, hint: Option<&str>, title: Option<&str>| {
        Some(Value::record([
            ("id", Value::from(id)),
            ("role", Value::from("system")),
            ("line", Value::from(line)),
            ("text", Value::from(words)),
            ("hint", hint.map_or(Value::Nil, Value::from)),
            ("title", title.map_or(Value::Nil, Value::from)),
        ]))
    };
    if let Some(decided) = part("classifier") {
        return row("classifier", text, None, None).map(|r| with_fields(r, [("decision", decision_value(&decided["record"]))]));
    }
    if let Some(compacted) = part("compaction") {
        return row("compaction", text, None, None).map(|r| with_fields(r, [("kept", kept_value(&compacted["record"]))]));
    }
    if let Some(fork) = part("fork") {
        return row("fork", "", None, Some(&single_line(fork["sourceTitle"].as_str().unwrap_or_default())));
    }
    if let Some(handoff) = part("handoff") {
        return row("handoff", "", None, handoff["route"]["sourceTitle"].as_str());
    }
    if let Some(card) = zeron_proto::claude_config::CommandCard::from_note(text, texts.last().copied()) {
        let context = (card.command.trim_start_matches('/') == "context").then(|| context_report(&card)).flatten();
        return row("claude_command", text, None, None).map(|r| with_fields(r, [("context", context.unwrap_or(Value::Nil)), ("card", card_value(&card))]));
    }
    if let Some(parts) = zeron_proto::claude_panel::summary_parts(text) {
        let parts: Vec<Value> = parts.iter().map(|(label, value)| Value::record([("label", Value::from(label.as_str())), ("value", Value::from(value.as_str()))])).collect();
        return row("claude_config", text, None, None).map(|r| with_fields(r, [("parts", Value::List(parts.into()))]));
    }
    match computer_notice(text) {
        Some(ComputerNotice::Disclosure) => return None,
        Some(notice) => {
            let words = if notice == ComputerNotice::Revoked { revoked_label(text, hint) } else { text.to_owned() };
            return row("computer_notice", &words, hint, None).map(|r| with_fields(r, computer_notice_fields(notice, text, hint)));
        }
        None => {}
    }
    if text.trim().is_empty() {
        return None;
    }
    row("divider", text, hint, None)
}

/// A Claude Code command's card, as gpui's `render_claude_card` reads it.
fn card_value(card: &zeron_proto::claude_config::CommandCard) -> Value {
    use zeron_proto::claude_cards::{STALE_PENDING_MS, terminal_action, terminal_only};
    let terminal = terminal_only(card);
    let opt = |v: Option<&str>| v.map_or(Value::Nil, Value::from);
    Value::record([
        ("command", Value::from(card.command.as_str())),
        ("text", Value::from(card.text.as_str())),
        ("error", opt(card.error.as_deref())),
        ("pending", Value::Bool(card.pending)),
        ("started", card.started_at.map_or(Value::Nil, |ms| Value::Num(ms as f64))),
        ("stale_ms", Value::Num(STALE_PENDING_MS as f64)),
        ("terminal", opt(terminal)),
        ("action", opt(terminal.and_then(terminal_action).map(|(label, _)| label))),
        ("model", opt(card.model.as_deref())),
        ("note", opt(card.note.as_deref())),
        ("models", Value::List(card.models.iter().map(|(id, label)| Value::record([("id", Value::from(id.as_str())), ("label", Value::from(label.as_str()))])).collect::<Vec<_>>().into())),
    ])
}

/// Claude Code's `/context` answer as gpui's card draws it (claude_cards.rs `render_context`): the bar's parts and the legend, the buffer, what is free, the tip.
fn context_report(card: &zeron_proto::claude_config::CommandCard) -> Option<Value> {
    use zeron_proto::claude_cards::{Series, parse_context, percent, series};
    use zeron_proto::usage::format_tokens;
    let report = parse_context(&card.text)?;
    let total = report.total.max(1);
    let part = |name: &str, n: u64, ink: &str| {
        Value::record([
            ("name", Value::from(name)),
            ("tokens", Value::from(format_tokens(n).as_str())),
            ("percent", Value::from(percent(n, total).as_str())),
            ("w", Value::Num(if n > 0 { f64::from(report.bar_fraction(n)) } else { 0. })),
            ("ink", Value::from(ink)),
        ])
    };
    let ink = |name: &str| match series(name) {
        Series::System => "system",
        Series::Mcp => "mcp",
        Series::Agents => "agents",
        Series::Memory => "memory",
        Series::Skills => "skills",
        Series::Messages => "messages",
        Series::Other => "other",
    };
    let mut parts: Vec<Value> = report.categories.iter().map(|(name, n)| part(name, *n, ink(name))).collect();
    if report.buffer > 0 {
        parts.push(part("Autocompact buffer", report.buffer, "buffer"));
    }
    let free = part("Free", report.free, "free");
    let groups: Vec<Value> = report
        .groups
        .iter()
        .map(|g| {
            let rows: Vec<Value> = g
                .rows
                .iter()
                .map(|(name, n)| {
                    Value::record([
                        ("name", Value::from(name.as_str())),
                        ("tokens", Value::from(format_tokens(*n).as_str())),
                        ("w", Value::Num(*n as f64 / g.total.max(1) as f64)),
                    ])
                })
                .collect();
            Value::record([("title", Value::from(g.title.as_str())), ("tokens", Value::from(format_tokens(g.total).as_str())), ("rows", Value::List(rows.into()))])
        })
        .collect();
    Some(Value::record([
        ("model", Value::from(report.model.as_str())),
        ("pill", Value::from(report.pill().as_str())),
        ("scale", Value::Num(f64::from(report.bar_scale()))),
        ("parts", Value::List(parts.into())),
        ("free", free),
        ("tip", report.tip().map_or(Value::Nil, |t| Value::from(t.as_str()))),
        ("note", card.note.as_deref().map_or(Value::Nil, Value::from)),
        ("groups", Value::List(groups.into())),
        ("raw", Value::from(card.text.as_str())),
    ]))
}

/// A classifier's verdict as the engine writes it (`ClassifierVerdict`: anything else reads as no answer).
fn verdict_code(v: &Json) -> &'static str {
    match v.as_str() {
        Some("new") => "new",
        Some("carry") => "carry",
        Some("continue") => "continue",
        _ => "failed",
    }
}

/// Who decided (`DecidedBy`: anything else reads as the first look).
fn decided_code(v: &Json) -> &'static str {
    match v.as_str() {
        Some("stage2") => "stage2",
        Some("streak") => "streak",
        Some("ceiling") => "ceiling",
        Some("agentNote") => "agent_note",
        Some("turnEnd") => "turn_end",
        Some("backgroundCopy") => "background_copy",
        _ => "stage1",
    }
}

/// A classifier's record, for the compaction card and its open panel (gpui's `render_compaction_card`).
fn decision_value(r: &Json) -> Value {
    let num = |k: &str| r[k].as_f64().map_or(Value::Nil, Value::Num);
    let words = |k: &str| r[k].as_str().filter(|s| !s.trim().is_empty()).map_or(Value::Nil, Value::from);
    let harness = r["harness"].as_str();
    let agent = harness
        .and_then(|h| serde_json::from_value::<zeron_proto::HarnessId>(Json::from(h)).ok())
        .map(|h| h.display_name());
    Value::record([
        ("verdict", Value::from(verdict_code(&r["verdict"]))),
        ("said", if r["classifierVerdict"].is_string() { Value::from(verdict_code(&r["classifierVerdict"])) } else { Value::Nil }),
        ("by", Value::from(decided_code(&r["decidedBy"]))),
        ("ms", Value::Num(r["durationMs"].as_f64().unwrap_or(0.))),
        ("context", Value::Num(r["contextTokens"].as_f64().unwrap_or(0.))),
        ("input", Value::from(r["input"].as_str().unwrap_or_default())),
        ("reply", Value::from(r["reply"].as_str().unwrap_or_default())),
        ("stage2", words("stage2Reply")),
        ("tokens_in", num("inputTokens")),
        ("tokens_out", num("outputTokens")),
        ("reason", words("reason")),
        ("mark", harness.and_then(switch_mark).map_or(Value::Nil, |m| Value::from(m.as_str()))),
        ("agent", agent.map_or(Value::Nil, Value::from)),
        ("model", words("model")),
    ])
}

/// The turns a carried-over block holds, in its words (gpui's `carried_body`).
fn carried_body(carried: &str) -> &str {
    let inner = carried.trim().trim_start_matches("<carried_over>").trim_end_matches("</carried_over>").trim();
    inner.split_once("\n\n").map_or(inner, |(_, rest)| rest)
}

fn carried_turns(carried: Option<&str>) -> usize {
    carried.map_or(0, |c| carried_body(c).split("\n\n").filter(|b| !b.starts_with("Still running when the chat compacted:")).count())
}

/// What a compaction kept (`CompactionRecord`): its sizes, how long, the agent's summary, the turns carried word for word.
fn kept_value(r: &Json) -> Value {
    let num = |k: &str| r[k].as_f64().map_or(Value::Nil, Value::Num);
    let carried = r["carried"].as_str();
    Value::record([
        ("pre", num("preTokens")),
        ("post", num("postTokens")),
        ("ms", num("durationMs")),
        ("summary", r["summary"].as_str().filter(|s| !s.trim().is_empty()).map_or(Value::Nil, Value::from)),
        ("carried", carried.map_or(Value::Nil, |c| Value::from(carried_body(c)))),
        ("turns", Value::Num(carried_turns(carried) as f64)),
    ])
}

/// A compaction folds into the decision right before it, past the messages sent meanwhile (gpui's `merge_compaction_rows`).
fn merge_compaction_rows(list: &mut Vec<Value>) {
    // (A folded row is neither a prompt nor one of these lines, `kept_whole`: it isn't opened.)
    let field = |v: &Value, k: &str| if matches!(v, Value::Folded(_)) { String::new() } else { v.field(k).map(crate::auth::text).unwrap_or_default() };
    let mut ix = 0;
    while ix + 1 < list.len() {
        let next = (ix + 1..list.len()).find(|&at| field(&list[at], "role") != "user");
        if field(&list[ix], "line") == "classifier"
            && list[ix].field("kept").is_none_or(|k| matches!(k, Value::Nil))
            && let Some(next) = next
            && field(&list[next], "line") == "compaction"
        {
            let kept = list.remove(next).field("kept").cloned().unwrap_or(Value::Nil);
            list[ix] = with_fields(list[ix].clone(), [("kept", kept)]);
        }
        ix += 1;
    }
}

/// The compaction a chat waits on (gpui's `CompactionWait::of`): queued after
/// the agent's turn, its classifier checking, or compacting; with the row
/// that shows it when the decision itself is the boundary, and how long
/// compactions here usually take.
fn compaction_wait(chats: &Chats, open: &Open) -> Value {
    use zeron_proto::transcript::{Boundary, CompactionWait};
    let session = chats.sessions.borrow().get(&open.id).cloned().unwrap_or(Json::Null);
    let classifying: Option<zeron_proto::Classifying> = serde_json::from_value(session["classifying"].clone()).ok();
    let plan: Option<zeron_proto::CompactPlan> = serde_json::from_value(session["compactPlan"].clone()).ok();
    let entries = || open.entries.iter().filter_map(|e| e.json.as_ref());
    let tail = entries().rev().find(|e| e["role"].as_str() != Some("user"));
    let decision = tail.and_then(|e| e["parts"].as_array()?.iter().find(|p| p["kind"].as_str() == Some("classifier")));
    let boundary = decision.and_then(|p| {
        let record = &p["record"];
        let carry = match verdict_code(&record["verdict"]) {
            "new" => false,
            "carry" => true,
            _ => return None,
        };
        Some(Boundary { tokens: record["contextTokens"].as_u64().unwrap_or(0), carry, cause: None })
    });
    let compacting = chats.chat_compacting(&open.id).then_some(boundary);
    let Some(wait) = CompactionWait::of(compacting, classifying.as_ref(), plan, !chats.running(&open.id)) else {
        return Value::Nil;
    };
    let mut durations: Vec<u64> = entries()
        .flat_map(|e| e["parts"].as_array().into_iter().flatten())
        .filter(|p| p["kind"].as_str() == Some("compaction"))
        .filter_map(|p| p["record"]["durationMs"].as_u64())
        .collect();
    durations.sort_unstable();
    let usual = durations.get(durations.len() / 2).map_or(Value::Nil, |ms| Value::Num(((ms + 500) / 1000) as f64));
    let mut fields = vec![("usual", usual)];
    match wait {
        CompactionWait::Queued => fields.push(("phase", Value::from("queued"))),
        CompactionWait::Classifying { harness, model, agent_note } => {
            fields.push(("phase", Value::from("checking")));
            fields.push(("agent_note", Value::Bool(agent_note)));
            fields.push(("mark", switch_mark(harness.as_str()).map_or(Value::Nil, |m| Value::from(m.as_str()))));
            fields.push(("agent", Value::from(harness.display_name())));
            fields.push(("model", model.as_deref().map_or(Value::Nil, Value::from)));
        }
        CompactionWait::Compacting(boundary) => {
            fields.push(("phase", Value::from("compacting")));
            if let Some(b) = boundary {
                fields.push(("tag", Value::from(if b.carry { "carry" } else { "new" })));
                fields.push(("tokens", Value::Num(b.tokens as f64)));
                fields.push(("row", tail.and_then(|e| e["id"].as_str()).map_or(Value::Nil, Value::from)));
            }
        }
    }
    Value::record(fields)
}

/// Why compacting this chat is refused now, as gpui's `compact_disabled_reason` (a key; the agent's own words when it can't).
fn compact_blocked(chats: &Chats, open: &Open, harness: &str) -> Value {
    if chats.chat_compacting(&open.id) {
        return Value::from("compacting");
    }
    if chats.running(&open.id) {
        return Value::from("running");
    }
    match chats.compaction_unsupported.borrow().get(harness) {
        Some(reason) => Value::record([("text", Value::from(reason.as_str()))]),
        None => Value::Nil,
    }
}

/// The "Resume with less context" offer on a large Claude chat idle past its prompt cache (gpui's `resume_compaction`).
fn resume_offer(open: &Open, chat: &Json, harness: &str, blocked: &Value) -> Value {
    if !matches!(blocked, Value::Nil) {
        return Value::Nil;
    }
    let Ok(id) = serde_json::from_value::<zeron_proto::HarnessId>(Json::from(harness)) else { return Value::Nil };
    let at = serde_json::from_value(chat["lastMessageAt"].clone()).ok();
    let Ok(now) = serde_json::from_value(Json::from(utc_stamp(now_seconds()))) else { return Value::Nil };
    let tokens = open.context_usage.0.map(|t| t as u64);
    let Some(offer) = zeron_proto::compaction::resume_compaction_due(id, tokens, at, now) else { return Value::Nil };
    Value::record([
        ("tokens", Value::from(zeron_proto::model_options::compact_tokens(offer.tokens).as_str())),
        ("idle", Value::from(offer.idle_label().as_str())),
        ("at", Value::from(chat["lastMessageAt"].as_str().unwrap_or_default())),
    ])
}

/// A planned or running compaction, for the footer ring's dot and its card's section (`stats_view::CompactSection`).
fn compact_plan(chats: &Chats, open: &Open) -> Value {
    use zeron_proto::stats_view::CompactSection;
    let session = chats.sessions.borrow().get(&open.id).cloned().unwrap_or(Json::Null);
    let plan: Option<zeron_proto::CompactPlan> = serde_json::from_value(session["compactPlan"].clone()).ok();
    let compacting = chats.chat_compacting(&open.id);
    // (The prompt cache's row ticks in the card itself, from the stats.)
    let Some(section) = CompactSection::of(plan, compacting, None, now_seconds() * 1000) else { return Value::Nil };
    let rows: Vec<Value> = section
        .rows
        .iter()
        .map(|(label, value)| Value::record([("label", Value::from(*label)), ("value", Value::from(value.as_str()))]))
        .collect();
    let copy = match plan {
        Some(zeron_proto::CompactPlan::Copy { since, ready }) => {
            Value::record([("since", Value::Num(since as f64 / 1000.)), ("ready", Value::Bool(ready))])
        }
        _ => Value::Nil,
    };
    Value::record([
        ("heading", Value::from(section.heading)),
        ("note", section.note.map_or(Value::Nil, Value::from)),
        ("rows", Value::List(rows.into())),
        ("busy", Value::Bool(compacting)),
        ("copy", copy),
    ])
}

/// `secs` since the epoch as an RFC 3339 stamp in UTC.
fn utc_stamp(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn line_block(part: &Json, streaming: bool) -> Option<Value> {
    let id = part["id"].as_str().unwrap_or_default();
    let block = |line: &str, fields: Vec<(&'static str, Value)>| {
        let mut all = vec![("id", Value::from(id)), ("kind", Value::from("line")), ("line", Value::from(line))];
        all.extend(fields);
        Some(Value::record(all))
    };
    match part["kind"].as_str()? {
        "fork" => block("fork", vec![("title", Value::from(single_line(part["sourceTitle"].as_str().unwrap_or_default()).as_str()))]),
        "switch" => block(
            "switch",
            vec![
                ("from", Value::from(single_line(part["from"].as_str().unwrap_or_default()).as_str())),
                ("to", Value::from(single_line(part["to"].as_str().unwrap_or_default()).as_str())),
            ],
        ),
        "input" => {
            use zeron_proto::view::{InputOutcome, input_header, input_outcome};
            let questions: Vec<zeron_proto::UserInputQuestion> = serde_json::from_value(part["questions"].clone()).ok()?;
            let header = input_header(&questions);
            // Only that one was asked for: whatever came back isn't the transcript's.
            let answers: Option<Vec<zeron_proto::UserInputAnswer>> = serde_json::from_value(part["answers"].clone()).ok();
            let resolved = part["resolved"].as_bool() == Some(true);
            if let Some(row) = computer_input(id, part, &questions, resolved, answers.as_deref()) {
                return row;
            }
            if let Some(q) = questions.iter().find(|q| q.is_secret()) {
                use zeron_proto::view::{SecretStatus, secret_status};
                let status = match secret_status(resolved, answers.as_deref()) {
                    SecretStatus::Pending if !streaming => SecretStatus::Cancelled,
                    status => status,
                };
                let placeholder = match &q.kind {
                    Some(zeron_proto::UserInputKind::Secret { placeholder }) => placeholder.as_deref(),
                    _ => None,
                };
                return block(
                    "secret",
                    vec![
                        ("title", Value::from(header.as_str())),
                        ("request", part["requestId"].as_str().map_or(Value::Nil, Value::from)),
                        ("reason", Value::from(q.question.as_str())),
                        ("placeholder", placeholder.map_or(Value::Nil, Value::from)),
                        ("status", Value::from(format!("{status:?}").to_lowercase().as_str())),
                        ("text", Value::from(status.label())),
                    ],
                );
            }
            let said = |line: &zeron_proto::view::AnswerLine| format!("{} → {}", line.label, line.answer);
            let row = |line: &zeron_proto::view::AnswerLine| {
                Value::record([("label", Value::from(line.label.as_str())), ("answer", Value::from(line.answer.as_str())), ("skipped", Value::Bool(line.skipped))])
            };
            let (outcome, text, lines) = match input_outcome(&questions, resolved, answers.as_deref()) {
                InputOutcome::Pending => return block("input", vec![("title", Value::from(header.as_str())), ("outcome", Value::from("pending"))]),
                InputOutcome::Unrecorded => ("unrecorded", header, Vec::new()),
                InputOutcome::Closed(line) => ("closed", said(&line), vec![row(&line)]),
                InputOutcome::Answered(lines) => ("answered", lines.iter().map(said).collect::<Vec<_>>().join(" · "), lines.iter().map(row).collect()),
            };
            block("input", vec![("text", Value::from(text.as_str())), ("outcome", Value::from(outcome)), ("answers", Value::List(lines.into()))])
        }
        _ => None,
    }
}

/// A computer ask as gpui's transcript draws it (`ComputerConsent`, `ComputerForeground`):
/// its card, the row it settled to, or none; `None` when it isn't one.
fn computer_input(
    id: &str,
    part: &Json,
    questions: &[zeron_proto::UserInputQuestion],
    resolved: bool,
    answers: Option<&[zeron_proto::UserInputAnswer]>,
) -> Option<Option<Value>> {
    use zeron_proto::UserInputKind;
    use zeron_proto::computer::ComputerConsentScope;
    use zeron_proto::computer::cards::*;
    let block = |line: &str, fields: Vec<(&'static str, Value)>| {
        let mut all = vec![("id", Value::from(id)), ("kind", Value::from("line")), ("line", Value::from(line))];
        all.extend(fields);
        Some(Value::record(all))
    };
    let request = || part["requestId"].as_str().map_or(Value::Nil, Value::from);
    let opt = |s: Option<&str>| s.map_or(Value::Nil, Value::from);
    let choice = |key: &str, label: &str, answer: &str, primary: bool, settled: Option<String>| {
        Value::record([
            ("key", Value::from(key)),
            ("label", Value::from(label)),
            ("answer", Value::from(answer)),
            ("primary", Value::Bool(primary)),
            ("settled", settled.as_deref().map_or(Value::Nil, Value::from)),
        ])
    };
    let foreground = questions.iter().find_map(|q| match &q.kind {
        Some(UserInputKind::Foreground { app }) => Some((q, app.as_ref())),
        _ => None,
    });
    if let Some((question, app)) = foreground {
        if let Some(text) = foreground_outcome_label(questions, answers) {
            return Some(block("computer_notice", vec![("notice", Value::from("foreground")), ("text", Value::from(text.as_str()))]));
        }
        if resolved {
            return Some(None);
        }
        let app = app.map(|a| a.name.as_str());
        let choices = vec![
            choice("no", COMPUTER_FOREGROUND_DECLINE, COMPUTER_FOREGROUND_DECLINE, false, None),
            choice("allow", COMPUTER_FOREGROUND_ALLOW, COMPUTER_FOREGROUND_ALLOW, true, None),
        ];
        return Some(block(
            "computer_foreground",
            vec![
                ("request", request()),
                ("question", Value::from(question.id.as_str())),
                ("app", opt(app)),
                ("title", Value::from(foreground_title(app).as_str())),
                ("detail", Value::from(COMPUTER_FOREGROUND_DETAIL)),
                ("choices", Value::List(choices.into())),
            ],
        ));
    }
    let question = consent_question(questions)?;
    let scope = consent_scope(question);
    let app = consent_app(question);
    let outcome = consent_outcome(resolved, answers);
    let key = |o: ConsentOutcome| match o {
        ConsentOutcome::Allowed => "allowed",
        ConsentOutcome::AlwaysAllowed => "always",
        ConsentOutcome::Declined => "declined",
        ConsentOutcome::Cancelled => "cancelled",
    };
    // gpui lists them Allow first and draws them reversed, Allow last and filled.
    let choices: Vec<Value> = consent_choices(scope, app.as_ref())
        .into_iter()
        .rev()
        .map(|(c, label)| {
            let settled = consent_event_label(c.outcome(), scope, app.as_ref());
            choice(consent_choice_key(&c), &label, c.answer_label(), c == ConsentChoice::Allow, Some(settled))
        })
        .collect();
    let detail = consent_detail(question);
    Some(block(
        "computer_consent",
        vec![
            ("request", request()),
            ("question", Value::from(question.id.as_str())),
            ("scope", Value::from(if scope == ComputerConsentScope::Call { "call" } else { "task" })),
            ("app", opt(app.as_ref().map(|a| a.name.as_str()))),
            ("title", Value::from(consent_card_title(scope, app.as_ref()).as_str())),
            ("detail", if detail.is_empty() { Value::Nil } else { Value::from(detail.as_str()) }),
            ("risk", opt(app.as_ref().and_then(elevated_risk))),
            ("disclosure_text", Value::from(COMPUTER_FIRST_USE_DISCLOSURE)),
            ("choices", Value::List(choices.into())),
            ("outcome", opt(outcome.map(key))),
            ("text", outcome.map_or(Value::Nil, |o| Value::from(consent_event_label(o, scope, app.as_ref()).as_str()))),
        ],
    ))
}

/// What a computer note says, by gpui's `render_computer_notice`: its kind, and for a card its lines.
fn computer_notice_fields(notice: zeron_proto::computer::cards::ComputerNotice, text: &str, hint: Option<&str>) -> Vec<(&'static str, Value)> {
    use zeron_proto::computer::cards::*;
    let (kind, lines): (&str, Vec<&str>) = match notice {
        ComputerNotice::Disclosure => ("disclosure", Vec::new()),
        ComputerNotice::Revoked => ("revoked", Vec::new()),
        ComputerNotice::Paused => ("paused", Vec::new()),
        ComputerNotice::Foreground => ("foreground", Vec::new()),
        ComputerNotice::LeftOpen => ("left_open", Vec::new()),
        ComputerNotice::Unavailable => ("unavailable", hint.into_iter().collect()),
        ComputerNotice::ControlDenied => ("control_denied", vec![COMPUTER_DENIED_HINT]),
        ComputerNotice::SetupRequired => ("setup", hint.into_iter().collect()),
    };
    let mut fields = vec![
        ("notice", Value::from(kind)),
        ("lines", Value::List(lines.into_iter().map(Value::from).collect::<Vec<_>>().into())),
    ];
    if notice == ComputerNotice::ControlDenied {
        fields.push(("action", Value::from(COMPUTER_DENIED_ACTION)));
    }
    if let Some(app) = left_open_app(text) {
        fields.push(("app", Value::from(app)));
    }
    fields
}

fn denied_block(part: &Json, earlier: &[&Json]) -> Option<Value> {
    use zeron_proto::computer::cards::{COMPUTER_DENIED_TITLE, is_control_denied};
    let denied = |p: &Json| {
        is_control_denied(p["isError"].as_bool() == Some(true), p["output"].as_str())
            .then(|| crate::tool_details::computer_tool_of(p))
            .flatten()
    };
    let tool = denied(part)?;
    if earlier.iter().any(|p| denied(p) == Some(tool)) {
        return None;
    }
    let mut fields = vec![
        ("id", Value::from(format!("{}-denied", part["id"].as_str().unwrap_or_default()).as_str())),
        ("kind", Value::from("line")),
        ("line", Value::from("computer_notice")),
        ("text", Value::from(COMPUTER_DENIED_TITLE)),
    ];
    fields.extend(computer_notice_fields(zeron_proto::computer::cards::ComputerNotice::ControlDenied, COMPUTER_DENIED_TITLE, None));
    Some(Value::record(fields))
}

/// A finished delegation's wake, its quoted reply as a markdown quote
/// (crates/proto `handback_quotes`; zgx doesn't link crates/proto).
fn handback_quotes(text: &str) -> Option<String> {
    let handback = |line: &str| line.starts_with("Delegated task ") || line.starts_with("Step \"");
    if !text.contains("<agent-response>") || !text.lines().any(handback) {
        return None;
    }
    let mut quoted = false;
    let lines: Vec<String> = text
        .lines()
        .filter_map(|line| match line {
            "<agent-response>" => {
                quoted = true;
                None
            }
            "</agent-response>" => {
                quoted = false;
                None
            }
            "" if quoted => Some(">".to_owned()),
            _ if quoted => Some(format!("> {line}")),
            _ => Some(line.to_owned()),
        })
        .collect();
    Some(lines.join("\n"))
}

/// The fork's seam for an agent change: a `handoff` part, read as the
/// `switch` part this page draws.
fn handoff_switch(entry: &Json) -> Option<Json> {
    let part = entry["parts"].as_array()?.iter().find(|p| p["kind"].as_str() == Some("handoff"))?;
    let route = &part["route"];
    let from = route["fromHarness"].as_str()?;
    if !route["sourceChatId"].is_null() {
        return None;
    }
    let to = part["harness"].as_str().unwrap_or_default();
    let name = |model: &Json, harness: &str| model.as_str().filter(|m| !m.is_empty()).unwrap_or(harness).to_owned();
    Some(json!({
        "from": name(&route["fromModel"], from),
        "to": name(&route["toModel"], to),
        "fromHarness": from,
        "toHarness": to,
    }))
}

/// A switch side's mark: its agent's, the mock drawn as Claude, as the
/// shipping app's `harness_brand_icon`.
fn switch_mark(harness: &str) -> Option<String> {
    if harness == "mock" {
        return Some("claude".into());
    }
    crate::agents::glyph(harness)
}

/// A name on one line (the shipping app's `single_line`).
fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What a message's copy action copies; nil (no action) with nothing to copy.
fn copy_value(text: &str) -> Value {
    if text.trim().is_empty() { Value::Nil } else { Value::from(text) }
}

/// A reply's words to copy: its text parts as written, no tools, no errors
/// (the shipping app's `assistant_copy_text`).
fn reply_copy_text(entry: &Json) -> String {
    entry["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["kind"].as_str() == Some("text"))
        .filter_map(|p| p["text"].as_str())
        .filter(|t| !t.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// A reply's parts in order, as the shipping app lays them out (crates/ui
/// transcript.rs, its non-compact rows): its text, and between the text each
/// run of consecutive tools (thinking with them) as a group of its own,
/// keyed by its first part. A subagent's tools never share a group with the
/// others, nor thinking with them. A group is open while the reply streams
/// and it is the reply's tail.
fn blocks(entry: &Json) -> Value {
    let parts: Vec<&Json> = entry["parts"].as_array().into_iter().flatten().collect();
    blocks_of(&parts, entry["status"].as_str() == Some("streaming"))
}

/// The blocks of these parts of a reply (`streaming`: still being written).
fn blocks_of(parts: &[&Json], streaming: bool) -> Value {
    let last_ix = parts.len().saturating_sub(1);
    let mut out: Vec<Value> = Vec::new();
    // The group being gathered: its parts, and the last one's index.
    let mut group: Vec<&Json> = Vec::new();
    let mut group_last = 0usize;
    let flush = |out: &mut Vec<Value>, group: &mut Vec<&Json>, group_last: usize| {
        if group.is_empty() {
            return;
        }
        let first = group[0]["id"].as_str().unwrap_or_default();
        // (The part being written, if the group is the reply's tail: a
        // thought then shows its latest lines.)
        let live = streaming && group_last == last_ix;
        let tools: Vec<Value> = group
            .iter()
            .enumerate()
            .filter_map(|(i, p)| tool_live(p, live && i + 1 == group.len()))
            .collect();
        out.push(Value::record([
            ("id", Value::from(format!("g:{first}").as_str())),
            ("kind", Value::from("tools")),
            ("summary", Value::from(group_summary(group).as_str())),
            ("live", Value::Bool(streaming && group_last == last_ix)),
            ("tools", Value::List(tools.into())),
        ]));
        group.clear();
    };
    // The pictures an agent showed one after another (`show_image`, a
    // generated image), gathered into one block: a display call that worked
    // between them doesn't part them; anything else does.
    let mut pictures: Vec<&Json> = Vec::new();
    for (ix, &part) in parts.iter().enumerate() {
        let kind = part["kind"].as_str();
        if kind == Some("image") {
            flush(&mut out, &mut group, group_last);
            pictures.push(part);
            continue;
        }
        if display_tool(part) && display_failed_block(part, parts[ix + 1..].iter().copied()).is_none() {
            continue;
        }
        out.extend(images_block(&mut pictures));
        if kind == Some("tool") && !display_tool(part) {
            // The shipping app's `SentMessage`, out of the tool group.
            if let Some(sent) = sent_blocks(part) {
                flush(&mut out, &mut group, group_last);
                out.extend(sent);
                continue;
            }
            if part["delegateRoute"].is_object() && part["subagentRef"].as_str().is_none_or(str::is_empty) {
                flush(&mut out, &mut group, group_last);
                out.push(native_route_block(part));
                continue;
            }
        }
        match kind {
            // A display tool (`show_visual`, `show_image`) is its output
            // alone: no row, in no group or count. Failed, a quiet line at
            // its place says so (unless a later call of it showed).
            Some("tool") if display_tool(part) => {
                if let Some(failed) = display_failed_block(part, parts[ix + 1..].iter().copied()) {
                    flush(&mut out, &mut group, group_last);
                    out.push(failed);
                }
            }
            Some("tool") => {
                if group.first().is_some_and(|head| genus(head) != genus(part)) {
                    flush(&mut out, &mut group, group_last);
                }
                group.push(part);
                group_last = ix;
                if let Some(denied) = denied_block(part, &parts[..ix]) {
                    flush(&mut out, &mut group, group_last);
                    out.push(denied);
                }
                // (The shipping app's: outside the group, so the page stays above the reply.)
                if let Some(page) = html_block(part) {
                    flush(&mut out, &mut group, group_last);
                    out.push(page);
                }
            }
            Some("reasoning") => {
                if part["text"].as_str().unwrap_or_default().trim().is_empty() {
                    continue;
                }
                if group.first().is_some_and(|head| genus(head) != Genus::Other) {
                    flush(&mut out, &mut group, group_last);
                }
                group.push(part);
                group_last = ix;
            }
            kind => {
                flush(&mut out, &mut group, group_last);
                // An agent's visual (`show_visual`): its script on the
                // chat's host, run and drawn by src/visuals.rs and
                // ds/visual (`device` only when that isn't this one).
                if kind == Some("visual") {
                    out.push(visual_block(part));
                    continue;
                }
                let words = match kind {
                    Some("text") => reply_words(part, streaming),
                    Some("error") => {
                        out.push(error_block(part));
                        continue;
                    }
                    _ => {
                        out.extend(line_block(part, streaming));
                        continue;
                    }
                };
                if words.trim().is_empty() {
                    continue;
                }
                out.push(Value::record([
                    ("id", Value::from(part["id"].as_str().unwrap_or_default())),
                    ("kind", Value::from("text")),
                    ("text", Value::from(words.as_str())),
                ]));
            }
        }
    }
    flush(&mut out, &mut group, group_last);
    out.extend(images_block(&mut pictures));
    Value::List(out.into())
}

/// A run's error (gpui's `error_chip`): what it said, and whether signing in fixes it.
fn error_block(part: &Json) -> Value {
    use zeron_proto::run_errors::{ProviderProblem, provider_problem};
    let message = part["message"].as_str().unwrap_or_default();
    let problem = match provider_problem(message) {
        Some(ProviderProblem::SignedOut) => Value::from("signed_out"),
        Some(ProviderProblem::LimitReached) => Value::from("limit"),
        None => Value::Nil,
    };
    Value::record([
        ("id", Value::from(part["id"].as_str().unwrap_or_default())),
        ("kind", Value::from("line")),
        ("line", Value::from("error")),
        ("text", Value::from(message)),
        ("problem", problem),
    ])
}

fn reply_words(part: &Json, streaming: bool) -> String {
    let text = part["text"].as_str().unwrap_or_default();
    (!streaming).then(|| code_links::linked(text)).flatten().unwrap_or_else(|| text.to_owned())
}

fn sent_blocks(part: &Json) -> Option<Vec<Value>> {
    let call: zeron_proto::ToolCall = serde_json::from_value(part["call"].clone()).ok()?;
    let sent = zeron_proto::agent_messages::outgoing_messages(&call)?;
    let id = part["id"].as_str().unwrap_or_default();
    let pending = part["resolved"].as_bool() != Some(true);
    let failed = part["isError"].as_bool() == Some(true);
    Some(
        sent.iter()
            .enumerate()
            .map(|(ix, m)| {
                Value::record([
                    ("id", Value::from(format!("{id}.m{ix}").as_str())),
                    ("kind", Value::from("sent")),
                    ("to", Value::from(m.chat.as_str())),
                    ("pending", Value::Bool(pending && !failed)),
                    ("failed", Value::Bool(failed)),
                ])
            })
            .collect(),
    )
}

fn native_route_block(part: &Json) -> Value {
    let input = &part["call"]["input"];
    let title = input["title"]
        .as_str()
        .or(input["task"].as_str())
        .or(input["description"].as_str())
        .map(single_line)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Delegated task".into());
    Value::record([
        ("id", Value::from(format!("{}-native", part["id"].as_str().unwrap_or_default()).as_str())),
        ("kind", Value::from("native_route")),
        ("title", Value::from(title.as_str())),
        ("route", route_value(&part["delegateRoute"])),
    ])
}

/// The shipping app's `route_badge`.
pub(crate) fn route_value(route: &Json) -> Value {
    let Ok(parsed) = serde_json::from_value::<zeron_proto::ChildRoute>(route.clone()) else {
        return Value::Nil;
    };
    if !matches!(parsed.source.as_str(), "routed" | "chosen_by_agent" | "fell_back") {
        return Value::Nil;
    }
    Value::record([
        ("source", Value::from(parsed.source.as_str())),
        ("short", Value::from(zeron_proto::transcript::route_short_reason(&parsed).as_str())),
        ("resets_at", parsed.resets_at.map_or(Value::Nil, |ms| Value::Num(ms as f64))),
        ("role", parsed.role.as_deref().map_or(Value::Nil, Value::from)),
    ])
}

/// A reply as the shipping app lays it out in compact mode (transcript.rs
/// `rows_for_entry` with `compact`): all its work, its tools, its thinking
/// and what it said between them, folded into one group where the first
/// of it was, its line the tools' summary while it works and "Worked for
/// 42s" once done; then the reply, the run of text it ends on, shown. While
/// it streams there is no reply yet: everything rides the group. Inside the
/// group, opened, the reply's ordinary blocks. Nil when it did no work.
fn compact_blocks(entry: &Json) -> Option<Value> {
    let parts = entry["parts"].as_array()?;
    let streaming = entry["status"].as_str() == Some("streaming");
    let said = |p: &Json| {
        p["kind"].as_str() == Some("text") && !p["text"].as_str().unwrap_or_default().trim().is_empty()
    };
    let thought = |p: &Json| {
        p["kind"].as_str() == Some("reasoning") && !p["text"].as_str().unwrap_or_default().trim().is_empty()
    };
    let reply_start = if streaming {
        None
    } else {
        parts.iter().rposition(said).map(|mut start| {
            while start > 0 && said(&parts[start - 1]) {
                start -= 1;
            }
            start
        })
    };
    // (A display tool is no work: its output shows in the reply.)
    let worked_tool = |p: &Json| p["kind"].as_str() == Some("tool") && !display_tool(p);
    let is_work = |ix: usize, p: &Json| {
        worked_tool(p) || thought(p) || (said(p) && reply_start.is_none_or(|start| ix < start))
    };
    let tools: Vec<&Json> = parts.iter().filter(|p| worked_tool(p) || thought(p)).collect();
    if tools.is_empty() {
        return None;
    }
    let work_parts: Vec<&Json> =
        parts.iter().enumerate().filter(|(ix, p)| is_work(*ix, p)).map(|(_, p)| p).collect();
    let worked = (!streaming)
        .then(|| entry["durationMs"].as_f64())
        .flatten()
        .filter(|ms| *ms > 0.)
        .map(|ms| ((ms / 1000.) as i64).max(1));
    let line = match worked {
        Some(secs) => format!("Worked for {}", elapsed(secs)),
        None => group_summary(&tools),
    };
    let work = Value::record([
        ("id", Value::from("work")),
        ("kind", Value::from("work")),
        ("summary", Value::from(line.as_str())),
        ("live", Value::Bool(streaming)),
        ("tools", Value::List(tools.iter().filter_map(|p| tool(p)).collect::<Vec<_>>().into())),
        ("blocks", without_pages(blocks_of(&work_parts, streaming))),
    ]);
    let mut out: Vec<Value> = Vec::new();
    let mut placed = false;
    let mut pictures: Vec<&Json> = Vec::new();
    for (ix, part) in parts.iter().enumerate() {
        if part["kind"].as_str() == Some("image") {
            pictures.push(part);
            continue;
        }
        if display_tool(part) && display_failed_block(part, &parts[ix + 1..]).is_none() {
            continue;
        }
        out.extend(images_block(&mut pictures));
        if is_work(ix, part) {
            if !placed {
                out.push(work.clone());
                placed = true;
            }
            let earlier: Vec<&Json> = parts[..ix].iter().collect();
            out.extend(denied_block(part, &earlier));
            out.extend(html_block(part));
            continue;
        }
        let words = match part["kind"].as_str() {
            Some("text") => reply_words(part, streaming),
            Some("error") => {
                out.push(error_block(part));
                continue;
            }
            Some("visual") => {
                out.push(visual_block(part));
                continue;
            }
            Some("tool") => {
                out.extend(display_failed_block(part, &parts[ix + 1..]));
                continue;
            }
            _ => {
                out.extend(line_block(part, streaming));
                continue;
            }
        };
        if words.trim().is_empty() {
            continue;
        }
        out.push(Value::record([
            ("id", Value::from(part["id"].as_str().unwrap_or_default())),
            ("kind", Value::from("text")),
            ("text", Value::from(words.as_str())),
        ]));
    }
    out.extend(images_block(&mut pictures));
    Some(Value::List(out.into()))
}

/// Tools whose output is what they show (a picture, a visual): drawn as
/// that output alone, never as a tool row, in no group, count or summary.
/// The images chat and the visuals chat share this rule.
const DISPLAY_TOOLS: &[&str] = &["show_image", "show_visual"];

/// Whether `part` is a display tool's call: an MCP tool by its name, bare or
/// as a harness names it (`mcp__zeron__show_visual`).
fn display_tool(part: &Json) -> bool {
    if part["kind"].as_str() != Some("tool") {
        return false;
    }
    let call = &part["call"];
    let name = match call["kind"].as_str() {
        Some("mcp") => call["tool"].as_str(),
        Some("unknown") => call["name"].as_str(),
        _ => None,
    };
    name.and_then(|n| n.rsplit("__").next()).is_some_and(|n| DISPLAY_TOOLS.contains(&n))
}

/// A display tool's name without its server (`show_image`), if `part` is one.
fn display_name(part: &Json) -> Option<&str> {
    if !display_tool(part) {
        return None;
    }
    let call = &part["call"];
    call["tool"].as_str().or(call["name"].as_str()).and_then(|n| n.rsplit("__").next())
}

/// A display tool's call that failed, as the quiet line ds/transcript draws
/// in its place (`display_failed`): which it was (`tool`: "image" or
/// "visual") and why, in its own words (ds/reply_picture's `plain` keeps
/// only what's plain of it). Nil for one that worked or is still working
/// (its output, or nothing yet, shows), and for one a later call of the
/// same tool in the reply (`later`, its parts after this one) got past:
/// the agent tried again and it showed, so the stumble isn't worth a line.
fn display_failed_block<'a>(part: &Json, later: impl IntoIterator<Item = &'a Json>) -> Option<Value> {
    let name = display_name(part)?;
    if part["isError"].as_bool() != Some(true) {
        return None;
    }
    if later.into_iter().any(|p| display_name(p) == Some(name) && p["isError"].as_bool() != Some(true)) {
        return None;
    }
    let why = part["output"].as_str().map(str::trim).unwrap_or_default();
    Some(Value::record([
        ("id", Value::from(part["id"].as_str().unwrap_or_default())),
        ("kind", Value::from("display_failed")),
        ("tool", Value::from(name.strip_prefix("show_").unwrap_or(name))),
        ("text", Value::from(why)),
    ]))
}

/// The pictures gathered (image parts in a row) as one ds/transcript block,
/// `images`, keyed by the first: each its path on the chat's host and its
/// name. Nil for none; the gathering is emptied.
fn images_block(pictures: &mut Vec<&Json>) -> Option<Value> {
    let first = pictures.first()?["id"].as_str().unwrap_or_default().to_owned();
    let list: Vec<Value> = pictures
        .drain(..)
        .map(|p| {
            Value::record([
                ("id", Value::from(p["id"].as_str().unwrap_or_default())),
                ("path", Value::from(p["path"].as_str().unwrap_or_default())),
                ("name", Value::from(p["name"].as_str().unwrap_or_default())),
            ])
        })
        .collect();
    Some(Value::record([
        ("id", Value::from(first.as_str())),
        ("kind", Value::from("images")),
        ("images", Value::List(list.into())),
    ]))
}

/// A visual part as ds/transcript's block: its id (the visual's, which
/// seeds it), its script's path and title, and its host's device when that
/// isn't this one.
fn visual_block(part: &Json) -> Value {
    let field = |k: &str| part[k].as_str().map_or(Value::Nil, Value::from);
    Value::record([
        ("id", Value::from(part["id"].as_str().unwrap_or_default())),
        ("kind", Value::from("visual")),
        ("path", field("path")),
        ("title", field("title")),
        ("device", field("device")),
    ])
}

/// The shipping app's `html_render_row`.
fn html_block(part: &Json) -> Option<Value> {
    let page = part.get("htmlRender").filter(|p| p.is_object())?;
    let call: zeron_proto::ToolCall = serde_json::from_value(part["call"].clone()).ok()?;
    if !call.is_html_render() {
        return None;
    }
    let field = |k: &str| page[k].as_str().filter(|v| !v.is_empty()).map_or(Value::Nil, Value::from);
    Some(Value::record([
        ("id", Value::from(format!("{}-html", part["id"].as_str().unwrap_or_default()).as_str())),
        ("kind", Value::from("html")),
        ("path", field("path")),
        ("title", Value::from(page["title"].as_str().unwrap_or_default())),
        ("height", Value::Num(page["height"].as_f64().unwrap_or(f64::from(zeron_proto::html_render::MIN_HEIGHT)))),
        ("url", field("url")),
        ("device", part["device"].as_str().map_or(Value::Nil, Value::from)),
    ]))
}

fn without_pages(blocks: Value) -> Value {
    match blocks {
        Value::List(list) => Value::List(list.iter().filter(|b| b.field("kind").map(crate::auth::text).as_deref() != Some("html")).cloned().collect::<Vec<_>>().into()),
        other => other,
    }
}

/// The shipping app's `html_link_targets`.
fn html_files_block(entry: &Json) -> Option<Value> {
    if entry["status"].as_str() == Some("streaming") {
        return None;
    }
    let texts: Vec<&str> = entry["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["kind"].as_str() == Some("text"))
        .filter_map(|p| p["text"].as_str())
        .collect();
    let targets = zeron_proto::html_render::html_link_targets(texts.into_iter());
    if targets.is_empty() {
        return None;
    }
    let list: Vec<Value> = targets
        .iter()
        .map(|t| Value::record([("dest", Value::from(t.dest.as_str())), ("linked", Value::Bool(t.linked))]))
        .collect();
    Some(Value::record([("id", Value::from("html-files")), ("kind", Value::from("html_files")), ("targets", Value::List(list.into()))]))
}

fn push_block(blocks: Value, block: Value) -> Value {
    match blocks {
        Value::List(list) => {
            let mut all: Vec<Value> = list.iter().cloned().collect();
            all.push(block);
            Value::List(all.into())
        }
        other => other,
    }
}

/// Read back from zeron_proto's `with_picks`, so a reload or another device sees the same picks.
fn page_picks(words: &str) -> (String, Vec<Value>) {
    let (rest, cards) = zeron_proto::html_render::extract_picks(words);
    if cards.is_empty() {
        return (words.to_owned(), Vec::new());
    }
    let chips = cards
        .iter()
        .flat_map(|(card, picks)| {
            picks.iter().map(move |pick| {
                Value::record([
                    ("kind", Value::from("pick")),
                    ("name", Value::from(pick.label.as_str())),
                    ("card", Value::from(card.as_str())),
                    ("group", Value::from(pick.group.as_deref().unwrap_or_default())),
                    ("code", Value::from(pick.code.as_str())),
                    ("label", Value::from(pick.label.as_str())),
                ])
            })
        })
        .collect();
    (rest, chips)
}

/// How long, as the shipping app says it (transcript.rs `format_elapsed`).
fn elapsed(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86_400 {
        format!("{}h {}m", secs / 3_600, (secs % 3_600) / 60)
    } else {
        format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3_600)
    }
}

/// The model a spawn names, never its parent's (`ToolCall::subagent_model`).
fn subagent_model(call: &Json) -> Option<&str> {
    zeron_proto::SUBAGENT_MODEL_KEYS
        .iter()
        .find_map(|key| call["input"][key].as_str())
        .map(str::trim)
        .filter(|model| !model.is_empty())
}

fn subagent_reasoning(call: &Json) -> Option<String> {
    ["effort", "reasoning_effort", "reasoningEffort", "reasoning"].iter().find_map(|key| {
        let raw = call["input"][key].as_str()?.trim().to_ascii_lowercase().replace(['-', '_'], "");
        serde_json::from_value::<zeron_proto::ReasoningLevel>(Json::String(raw.clone())).ok().map(|_| raw)
    })
}

fn subagent_kind(call: &Json) -> Option<&str> {
    call["input"]["subagent_type"].as_str().map(str::trim).filter(|kind| !kind.is_empty())
}

fn subagent_mark(call: &Json) -> Option<String> {
    let maker = zeron_proto::HarnessId::brand_of(None, subagent_model(call))?;
    crate::agents::glyph(maker.as_str())
}

/// The shipping app's `spawn_tag`: never the parent's model.
fn spawn_tag(call: &Json) -> Option<String> {
    match subagent_model(call) {
        Some(model) => {
            let maker = zeron_proto::HarnessId::brand_of(None, Some(model)).map_or("claude-code", |h| h.as_str());
            Some(CHATS.with(|chats| chats.label_of(maker, model)).unwrap_or_else(|| model.to_owned()))
        }
        None => subagent_kind(call).map(Into::into),
    }
}

/// A subagent's tab title, as the shipping app's `subagent_tab_title`: the
/// tool's name, else its description, else its prompt, the "Agent"/"Task"
/// genus taken off, the first line, at most 40 characters.
fn subagent_title(call: &Json) -> String {
    fn strip(text: &str) -> &str {
        let t = text.trim();
        for prefix in ["agent", "task"] {
            if t.len() >= prefix.len() && t.is_char_boundary(prefix.len()) && t[..prefix.len()].eq_ignore_ascii_case(prefix) {
                let rest = &t[prefix.len()..];
                if rest.is_empty() {
                    return "";
                }
                if rest.starts_with(':') || rest.starts_with(char::is_whitespace) {
                    return rest.trim_start_matches(':').trim();
                }
            }
        }
        t
    }
    fn line(text: &str) -> Option<String> {
        let first = text.lines().find(|l| !l.trim().is_empty())?.trim();
        let mut out: String = first.chars().take(40).collect();
        if first.chars().count() > 40 {
            out.push('…');
        }
        Some(out)
    }
    let name = match call["kind"].as_str() {
        Some("mcp") => call["tool"].as_str(),
        Some("unknown") => call["name"].as_str(),
        _ => return "Subagent".into(),
    };
    let input = &call["input"];
    [name, input["description"].as_str(), input["prompt"].as_str()]
        .into_iter()
        .flatten()
        .find_map(|t| line(strip(t)))
        .unwrap_or_else(|| "Subagent".into())
}

fn call_name(call: &Json) -> Option<&str> {
    match call["kind"].as_str() {
        Some("mcp") => call["tool"].as_str(),
        Some("unknown") => call["name"].as_str(),
        _ => None,
    }
}

/// (The call itself, never a stray `subagentRef`.)
fn is_agent(part: &Json) -> bool {
    part["kind"].as_str() == Some("tool")
        && call_name(&part["call"]).is_some_and(|n| n == "Agent" || n.starts_with("Agent: ") || is_delegate_name(&part["call"], n))
}

fn is_delegate_name(call: &Json, name: &str) -> bool {
    match call["kind"].as_str() {
        Some("mcp") => name == "delegate_task",
        _ => name.ends_with("delegate_task"),
    }
}

fn is_delegate(part: &Json) -> bool {
    call_name(&part["call"]).is_some_and(|n| is_delegate_name(&part["call"], n))
}

fn is_thread_link(part: &Json) -> bool {
    let call = &part["call"];
    let made = match call["kind"].as_str() {
        Some("mcp") => call["tool"].as_str() == Some("create_thread"),
        Some("unknown") => call["name"].as_str().is_some_and(|n| n.ends_with("create_thread")),
        _ => false,
    };
    made && part["subagentRef"].as_str().is_some_and(|r| !r.is_empty())
}

#[derive(PartialEq)]
enum Genus {
    Agent,
    Thread,
    Computer,
    Other,
}

fn genus(part: &Json) -> Genus {
    if is_agent(part) {
        Genus::Agent
    } else if is_thread_link(part) {
        Genus::Thread
    } else if crate::tool_details::computer_tool_of(part).is_some() {
        Genus::Computer
    } else {
        Genus::Other
    }
}

/// A group's line, as the shipping app writes it (crates/ui transcript.rs
/// `tool_group_summary` over crates/proto view.rs's): its thinking, then
/// what its tools did, then how many failed; the first letter raised.
fn group_summary(group: &[&Json]) -> String {
    let thoughts = group.iter().filter(|p| p["kind"].as_str() == Some("reasoning")).count();
    let computers: Vec<crate::tool_details::ComputerRow> = group.iter().filter_map(|p| crate::tool_details::computer_row(p)).collect();
    let calls: Vec<&&Json> = group
        .iter()
        .filter(|p| p["kind"].as_str() == Some("tool") && crate::tool_details::computer_tool_of(p).is_none())
        .collect();
    // (Parsed only for a group of setup steps alone, as the JSON says first.)
    if thoughts == 0 && !calls.is_empty() && calls.iter().all(|p| p["call"]["input"][zeron_proto::CLOUD_SETUP_KEY].is_string()) {
        let parsed: Vec<(zeron_proto::ToolCall, bool)> =
            calls.iter().filter_map(|p| Some((serde_json::from_value(p["call"].clone()).ok()?, p["isError"].as_bool() == Some(true)))).collect();
        if parsed.len() == calls.len() && parsed.iter().all(|(c, _)| zeron_proto::cloud_setup_step(c).is_some()) {
            return zeron_proto::view::tool_group_summary(&parsed);
        }
    }
    let mut segments: Vec<String> = Vec::new();
    match thoughts {
        0 => {}
        1 => segments.push("thought process".into()),
        n => segments.push(format!("thought {n} times")),
    }
    if !computers.is_empty() {
        let mut apps: Vec<&str> = Vec::new();
        for app in computers.iter().filter_map(|c| c.app.as_deref()) {
            if !apps.contains(&app) {
                apps.push(app);
            }
        }
        segments.push(zeron_proto::computer::presentation::computer_group_segment(&apps, computers.len()));
    }
    if !calls.is_empty() {
        let (mut commands, mut reads, mut searches, mut fetches, mut todos, mut other, mut failed) =
            (0, 0, 0, 0, 0, 0, 0);
        let mut edited: Vec<String> = Vec::new();
        for p in &calls {
            if p["isError"].as_bool() == Some(true) {
                failed += 1;
            }
            let call = &p["call"];
            match call["kind"].as_str().unwrap_or_default() {
                "exec" => commands += 1,
                "writeFile" | "editFile" => {
                    let path = call["path"].as_str().unwrap_or_default().to_owned();
                    if !edited.contains(&path) {
                        edited.push(path);
                    }
                }
                "applyPatch" => {
                    let path = call["path"].as_str().unwrap_or("patch").to_owned();
                    if !edited.contains(&path) {
                        edited.push(path);
                    }
                }
                "readFile" => reads += 1,
                "search" | "glob" | "webSearch" => searches += 1,
                "webFetch" => fetches += 1,
                "todo" => todos += 1,
                _ => other += 1,
            }
        }
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut tools: Vec<String> = Vec::new();
        if commands > 0 {
            tools.push(format!("ran {}", plural(commands, "command", "commands")));
        }
        if !edited.is_empty() {
            tools.push(format!("edited {}", plural(edited.len(), "file", "files")));
        }
        if reads > 0 {
            tools.push(format!("read {}", plural(reads, "file", "files")));
        }
        if searches > 0 {
            tools.push(format!("searched {}", plural(searches, "time", "times")));
        }
        if fetches > 0 {
            tools.push(format!("fetched {}", plural(fetches, "page", "pages")));
        }
        if todos > 0 {
            tools.push("updated todos".into());
        }
        if other > 0 {
            tools.push(format!("called {}", plural(other, "tool", "tools")));
        }
        if tools.is_empty() {
            tools.push(plural(calls.len(), "tool", "tools"));
        }
        if failed > 0 {
            tools.push(format!("{failed} failed"));
        }
        segments.push(tools.join(" · "));
    }
    let mut summary = segments.join(" · ");
    if let Some(first) = summary.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    summary
}

/// A part as ds/activity's tool: its kind, what it did, to what.
pub(crate) fn tool(part: &Json) -> Option<Value> {
    tool_live(part, false)
}

/// A thought's words as its row shows them: all of them. The row folds
/// them itself (ds/activity `thought_row`: four lines, "Show all N lines"),
/// so a cut here would leave "Show all" short of the whole thought.
fn thought_words(text: &str) -> String {
    text.trim().to_owned()
}

/// `tool`, for the part a reply is writing now (`live`): built once for the
/// settled row being built, however many of its layouts show it.
fn tool_live(part: &Json, live: bool) -> Option<Value> {
    let at = std::ptr::from_ref(part);
    if let Some(built) = ROW_BUILD.with_borrow(|build| build.as_ref()?.tools.get(&at).cloned()) {
        return built;
    }
    let mut built = tool_row(part, live);
    ROW_BUILD.with_borrow_mut(|build| {
        if let Some(build) = build {
            if let Some(built) = &mut built {
                share_parts(built, &mut build.seen);
            }
            build.tools.insert(at, built.clone());
        }
    });
    built
}

fn tool_row(part: &Json, live: bool) -> Option<Value> {
    let id = part["id"].as_str()?;
    let (kind, verb, target): (&str, String, Option<String>) = match part["kind"].as_str()? {
        "reasoning" => {
            let words = part["text"].as_str().unwrap_or_default();
            if words.trim().is_empty() {
                return None;
            }
            // Its row opens to its words, as prose.
            return Some(Value::record([
                ("id", Value::from(id)),
                ("kind", Value::from("thought")),
                ("verb", Value::from("Thought process")),
                ("call", Value::from(thought_words(words).as_str())),
                ("prose", Value::Bool(true)),
                // (Its row opens itself while it's written.)
                ("live", Value::Bool(live)),
            ]));
        }
        "tool" => {
            if let Some(computer) = crate::tool_details::computer_row(part) {
                return Some(computer_tool(id, part, computer));
            }
            let call = &part["call"];
            let s = |k: &str| call[k].as_str().map(str::to_owned);
            // (Parsed once, and only for a kind its JSON doesn't name.)
            let parsed_once: std::cell::OnceCell<Option<zeron_proto::ToolCall>> = std::cell::OnceCell::new();
            let parsed = || parsed_once.get_or_init(|| serde_json::from_value(call.clone()).ok()).as_ref();
            match call["kind"].as_str().unwrap_or_default() {
                "exec" => ("exec", "Ran".into(), s("command")),
                "readFile" => ("read", "Read".into(), s("path").map(|p| base(&p))),
                "writeFile" => ("write", "Wrote".into(), s("path").map(|p| base(&p))),
                "editFile" => ("edit", "Edited".into(), s("path").map(|p| base(&p))),
                "applyPatch" => ("edit", "Patched".into(), Some(s("path").map_or_else(|| "workspace".into(), |p| base(&p)))),
                "search" => ("search", "Searched".into(), s("pattern")),
                "glob" => ("glob", "Listed".into(), s("pattern")),
                "webFetch" => ("web", "Fetched".into(), s("url")),
                "webSearch" => ("web", "Searched the web".into(), s("query")),
                "todo" => ("todo", "Updated the plan".into(), todo_count(call)),
                _ if is_thread_link(part) => ("thread", "Started thread".into(), call["input"]["title"].as_str().map(Into::into)),
                // (The doc keeps a delegate's `title` alone: crates/doc `zeron_badge`.)
                _ if is_delegate(part) => ("delegate", "Delegated".into(), call["input"]["title"].as_str().or(call["input"]["task"].as_str()).map(Into::into)),
                "mcp" => {
                    let tool = s("tool").unwrap_or_default();
                    let server = s("server").unwrap_or_default();
                    if tool == "Agent" || tool.starts_with("Agent: ") {
                        ("agent", "Ran a subagent".into(), tool.strip_prefix("Agent: ").map(Into::into))
                    } else {
                        let action = (server == "zeron").then(|| zeron_proto::tools::action_label(&tool, &call["input"])).flatten();
                        ("mcp", "Used".into(), Some(format!("{server}.{}", action.unwrap_or(&tool))))
                    }
                }
                _ if parsed().is_some_and(zeron_proto::ToolCall::is_advisor) => ("advisor", "Advisor".into(), None),
                _ if parsed().and_then(zeron_proto::cloud_setup_step).is_some() => {
                    let step = parsed().and_then(zeron_proto::cloud_setup_step).unwrap_or_default();
                    (if matches!(step, "clone" | "branch") { "cloud_git" } else { "cloud" }, "Cloud".into(), s("name"))
                }
                _ => {
                    let name = s("name").unwrap_or_else(|| "a tool".into());
                    if name == "Agent" || name.starts_with("Agent: ") {
                        ("agent", "Ran a subagent".into(), name.strip_prefix("Agent: ").map(Into::into))
                    } else if name == "Wait for agents" {
                        ("wait", "Waited for agents".into(), None)
                    } else if let Some(skill) = parsed().and_then(zeron_proto::ToolCall::skill_name) {
                        ("skill", "Used".into(), Some(skill.to_owned()))
                    } else {
                        ("mcp", "Used".into(), Some(name))
                    }
                }
            }
        }
        _ => return None,
    };
    let mut fields = vec![
        ("id", Value::from(id)),
        ("kind", Value::from(kind)),
        ("verb", Value::from(verb.as_str())),
    ];
    // Its line is one line, whatever the agent wrote (the shipping app's
    // `tool_chip_content`: a command's lines, a pattern's newlines, joined).
    let target = target.map(|t| single_line(&t)).filter(|t| !t.is_empty());
    if let Some(target) = &target {
        fields.push(("target", Value::from(target.as_str())));
    }
    // The whole call, for the row opened, when it says more than the line.
    if let Some(call) = call_text(part).filter(|c| Some(c) != target.as_ref()) {
        fields.push(("call", Value::from(call.as_str())));
    }
    if part["isError"].as_bool() == Some(true) {
        fields.push(("failed", Value::Bool(true)));
    }
    // A subagent's spawn: the doc it runs in (its lane, a panel tab of its
    // own), how it stands, and its tab's title (the shipping app's).
    if kind == "agent"
        && let Some(doc) = part["subagentRef"].as_str().filter(|r| !r.is_empty())
    {
        fields.push((
            "subagent",
            Value::record([
                ("ref", Value::from(doc)),
                ("status", part["subagentStatus"].as_str().map_or(Value::Nil, Value::from)),
                ("title", Value::from(subagent_title(&part["call"]).as_str())),
                ("model", subagent_model(&part["call"]).map_or(Value::Nil, Value::from)),
                ("kind", subagent_kind(&part["call"]).map_or(Value::Nil, Value::from)),
                ("agent", subagent_mark(&part["call"]).map_or(Value::Nil, |mark| Value::from(mark.as_str()))),
                ("reasoning", subagent_reasoning(&part["call"]).map_or(Value::Nil, |level| Value::from(level.as_str()))),
            ]),
        ));
    }
    let bound = part["subagentRef"].as_str().filter(|r| !r.is_empty());
    if kind == "agent"
        && let Some(tag) = spawn_tag(&part["call"])
    {
        fields.push(("tag", Value::from(tag.as_str())));
    }
    if let (Some(child), true) = (bound, kind == "thread" || kind == "delegate") {
        fields.push((
            if kind == "thread" { "thread" } else { "delegate" },
            Value::record([
                ("ref", Value::from(child)),
                ("title", target.as_deref().map_or(Value::Nil, Value::from)),
            ]),
        ));
    }
    if let Some(path) = tool_file(part, kind) {
        fields.push(("path", Value::from(path)));
    }
    if part["kind"].as_str() == Some("tool") {
        let images = tool_images::image_paths(part, code_links::cwd().as_deref());
        if !images.is_empty() {
            fields.push(("images", Value::List(images.iter().map(|p| Value::from(p.as_str())).collect::<Vec<_>>().into())));
        }
    }
    // What its row opens to (src/tool_details.rs).
    fields.extend(crate::tool_details::tool_fields(part));
    let stats = part["diffStats"].as_array();
    if let Some(stats) = stats.filter(|s| !s.is_empty()) {
        let sum = |k: &str| stats.iter().filter_map(|s| s[k].as_f64()).sum::<f64>();
        fields.push(("added", Value::Num(sum("additions"))));
        fields.push(("removed", Value::Num(sum("deletions"))));
    }
    Some(Value::record(fields))
}

/// The file a row's name opens (gpui's `tool_file_path`, and a skill's SKILL.md).
fn tool_file<'a>(part: &'a Json, kind: &str) -> Option<&'a str> {
    let call = &part["call"];
    match kind {
        "read" | "write" | "edit" => call["path"].as_str().filter(|p| !p.is_empty()),
        "skill" => call["input"]["path"].as_str().filter(|p| p.ends_with("/SKILL.md")),
        _ => None,
    }
}

/// The shipping app's Todo detail, "N/M done".
fn todo_count(call: &Json) -> Option<String> {
    let items = call["items"].as_array()?;
    let done = items.iter().filter(|i| i["done"].as_bool() == Some(true) || i["status"].as_str() == Some("completed")).count();
    Some(format!("{done}/{} done", items.len()))
}

fn computer_tool(id: &str, part: &Json, computer: crate::tool_details::ComputerRow) -> Value {
    let mut fields = vec![
        ("id", Value::from(id)),
        ("kind", Value::from("exec")),
        ("verb", Value::from(computer.label.as_str())),
    ];
    if !computer.rows.is_empty() {
        fields.push(("call", Value::from(computer.rows.join("\n").as_str())));
    }
    if part["isError"].as_bool() == Some(true) {
        fields.push(("failed", Value::Bool(true)));
    }
    if !computer.actions.is_empty() {
        fields.push(("output", Value::from(computer.actions.join("\n").as_str())));
    } else if !computer.hidden {
        fields.extend(crate::tool_details::tool_fields(part));
    }
    Value::record(fields)
}

/// How many lines of a call the opened row shows, and how long a line may
/// run (a minified megabyte isn't shaped whole).
const CALL_MAX_LINES: usize = 200;
const CALL_LINE_MAX_CHARS: usize = 2_000;

/// A tool's whole call, as the shipping app's `call_block` writes it: the
/// whole command, pattern (and where), url (and what was asked of it), the
/// plan's items one a line, an MCP or other tool's input as JSON. None for
/// thinking, and where the call is only a path (the line names it).
fn call_text(part: &Json) -> Option<String> {
    if part["kind"].as_str() != Some("tool") {
        return None;
    }
    let call = &part["call"];
    let s = |k: &str| call[k].as_str().filter(|v| !v.trim().is_empty());
    let input = |head: String| match call.get("input").filter(|v| !v.is_null()) {
        Some(v) => format!("{head}\n{}", serde_json::to_string_pretty(v).unwrap_or_default()),
        None => head,
    };
    let text = match call["kind"].as_str().unwrap_or_default() {
        "exec" => s("command")?.to_owned(),
        "search" => match s("path") {
            Some(path) => format!("{} in {path}", s("pattern")?),
            None => s("pattern")?.to_owned(),
        },
        "glob" => s("pattern")?.to_owned(),
        "webFetch" => match s("prompt") {
            Some(prompt) => format!("{}\n{prompt}", s("url")?),
            None => s("url")?.to_owned(),
        },
        "webSearch" => s("query")?.to_owned(),
        "todo" => call["items"]
            .as_array()?
            .iter()
            .map(|i| {
                let mark = match (i["status"].as_str(), i["done"].as_bool()) {
                    (Some("completed"), _) | (None, Some(true)) => "[x]",
                    (Some("inProgress"), _) => "[~]",
                    _ => "[ ]",
                };
                format!("{mark} {}", i["text"].as_str().unwrap_or_default())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "mcp" => input(format!("{} · {}", s("server").unwrap_or_default(), s("tool").unwrap_or_default())),
        "unknown" => input(s("name").unwrap_or("Tool").to_owned()),
        _ => return None,
    };
    let text = text.trim_end();
    if text.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = text
        .lines()
        .take(CALL_MAX_LINES)
        .map(|l| match l.char_indices().nth(CALL_LINE_MAX_CHARS) {
            Some((cut, _)) => format!("{}…", &l[..cut]),
            None => l.to_owned(),
        })
        .collect();
    let more = text.lines().count().saturating_sub(CALL_MAX_LINES);
    if more > 0 {
        lines.push(format!("… {more} more lines"));
    }
    Some(lines.join("\n"))
}

fn base(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_owned()
}

/// The question the agent waits on: the last reply's unresolved input.
fn asked<'a>(mut newest_first: impl Iterator<Item = &'a Json>) -> Option<&'a Json> {
    let last = newest_first.find(|e| e["role"].as_str() == Some("assistant"))?;
    last["parts"]
        .as_array()?
        .iter()
        .rev()
        .find(|p| p["kind"].as_str() == Some("input") && p["resolved"].as_bool() != Some(true))
}

/// What a waiting chat asks, from its latest entries: an approval, else a
/// question (also when the ask isn't among them).
fn ask_kind<'a>(newest_first: impl Iterator<Item = &'a Json>) -> &'static str {
    match asked(newest_first).and_then(|p| p["questions"].as_array()) {
        Some(questions) if is_approval(questions) => "approval",
        _ => "question",
    }
}

fn is_approval(questions: &[Json]) -> bool {
    questions.len() == 1 && {
        let q = &questions[0];
        let options: Vec<&str> = q["options"].as_array().into_iter().flatten().filter_map(Json::as_str).collect();
        options == ["Yes", "No"] && q["header"].as_str().is_some_and(|h| h.starts_with("Approve"))
    }
}

fn single(questions: &[Json], label: &str) -> Vec<Json> {
    questions
        .iter()
        .filter_map(|q| q["id"].as_str())
        .map(|id| json!({ "questionId": id, "labels": [label] }))
        .collect()
}

/// What the composer shows of a question: an approval (what it would do),
/// or the page of questions not yet answered.
fn is_secret(question: &Json) -> bool {
    question["kind"]["type"].as_str() == Some("secret")
}

fn ask_value(part: &Json, answered: usize) -> Option<Value> {
    let request = part["requestId"].as_str()?;
    let questions = part["questions"].as_array()?;
    // gpui's `pending_input_request`: a computer ask is its transcript card's alone.
    if questions.iter().any(|q| matches!(q["kind"]["type"].as_str(), Some("consent" | "foreground"))) {
        return None;
    }
    if is_approval(questions) {
        let q = &questions[0];
        let words = q["question"].as_str().unwrap_or_default();
        let title = match q["header"].as_str().unwrap_or_default() {
            "Approve command" => "Run this command?",
            "Approve file change" => "Make this change?",
            _ => "Allow this?",
        };
        // The command, when the question quotes one ("wants to run `x`").
        let command = words.split('`').nth(1).filter(|c| !c.is_empty());
        let mut fields = vec![("title", Value::from(title))];
        match command {
            Some(c) => fields.push(("command", Value::from(c))),
            None => fields.push(("detail", Value::from(words))),
        }
        return Some(Value::record([
            ("request", Value::from(request)),
            ("kind", Value::from("approval")),
            ("approval", Value::record(fields)),
        ]));
    }
    let page = answered.min(questions.len().saturating_sub(1));
    let q = questions.get(page)?;
    if is_secret(q) {
        let words = q["question"].as_str().unwrap_or_default();
        return Some(Value::record([
            ("request", Value::from(request)),
            ("kind", Value::from("question")),
            (
                "question",
                Value::record([
                    ("text", Value::from(words)),
                    ("secret", Value::Bool(true)),
                    ("options", Value::List(Vec::new().into())),
                    ("multi", Value::Bool(false)),
                    ("page", Value::Num(1.)),
                    ("pages", Value::Num(1.)),
                ]),
            ),
        ]));
    }
    let options: Vec<Value> = q["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Json::as_str)
        .map(|o| Value::record([("value", Value::from(o)), ("label", Value::from(o))]))
        .collect();
    let mut question = vec![
        ("text", Value::from(q["question"].as_str().unwrap_or_default())),
        ("options", Value::List(options.into())),
        ("multi", Value::Bool(q["multiSelect"].as_bool() == Some(true))),
        ("page", Value::Num((page + 1) as f64)),
        ("pages", Value::Num(questions.len() as f64)),
    ];
    if let Some(h) = q["header"].as_str().filter(|h| !h.is_empty()) {
        question.push(("header", Value::from(h)));
    }
    Some(Value::record([
        ("request", Value::from(request)),
        ("kind", Value::from("question")),
        ("question", Value::record(question)),
    ]))
}

fn session_value(chats: &Chats, open: &Open, offline: bool, sending: bool) -> Value {
    let chat = chats.chat(&open.id);
    // Ready once its transcript has come and the chat itself (its config)
    // is known: the composer takes its model from it.
    let state = if open.heard && chat.is_some() {
        "ready"
    } else if chats.chats_heard.get() && chat.is_none() {
        "missing"
    } else if offline {
        "offline"
    } else {
        "loading"
    };
    let chat = chat.unwrap_or(Json::Null);
    let config = &chat["config"];
    let s = |j: &Json| j.as_str().map(Value::from).unwrap_or(Value::Nil);
    let harness = config["harness"].as_str().unwrap_or("claude-code");
    let model = config["model"]
        .as_str()
        .map(|m| Value::from(format!("{harness}/{m}").as_str()))
        .unwrap_or(Value::Nil);
    let access = match config["access"].as_str() {
        Some("supervised") => "supervised",
        Some("auto-accept-edits") => "edits",
        _ => "full",
    };
    let fast = fast_in(&config["modelOptions"]);
    let options = options_value(&config["modelOptions"]);
    let ask = asked(open.entries.iter().rev().filter_map(|e| e.json.as_ref()))
        .and_then(|p| ask_value(p, chats.pages_answered(&open.id, p["requestId"].as_str().unwrap_or_default())))
        .unwrap_or(Value::Nil);
    let session = chats.sessions.borrow().get(&open.id).cloned().unwrap_or(Json::Null);
    let live = chats.running(&open.id);
    let tasks = background_tasks(&session);
    let steers = chats.steers_mid_turn(harness, chats.target(&chat).as_deref());
    if live {
        open.awaiting_turn.set(None);
    }
    let awaiting = open.awaiting_turn.get().is_some_and(|at| at.elapsed() < TURN_START_GRACE);
    let stopped = offers_resume(live, sending, awaiting, open.entries.last().and_then(|e| e.json.as_ref()));
    let queue: Vec<Value> = open
        .queue
        .iter()
        .map(|q| {
            let mut fields = vec![("id", s(&q["id"])), ("text", s(&q["text"]))];
            let paths = row_paths(q);
            if !paths.is_empty() {
                fields.push(("cargo", attachments_value(&paths)));
            }
            let mine = open.editing.as_ref().is_some_and(|e| q["id"].as_str() == Some(&e.item));
            if mine {
                fields.push(("editing", Value::Bool(true)));
            }
            let gate = &q["deliveryGate"];
            if gate["kind"].as_str() == Some("editing") && !mine {
                let who = gate["ownerDeviceId"].as_str().unwrap_or_default();
                if chats.here.borrow().as_deref() != Some(who) {
                    fields.push(("locked", Value::from("Being edited on another device")));
                }
            }
            if gate["kind"].as_str() == Some("reviewRequired") {
                fields.push(("review", Value::Bool(true)));
            }
            Value::record(fields)
        })
        .collect();
    let (context_tokens, reported_window) = open.context_usage;
    let context_window = chat_budget(config).or(reported_window);
    let blocked = compact_blocked(chats, open, harness);
    let mut fields = vec![
        ("id", Value::from(open.id.as_str())),
        ("state", Value::from(state)),
        ("title", s(&chat["title"])),
        ("project", s(&chat["spaceId"])),
        ("device", s(&chat["deviceId"])),
        // Its folder on that device (a reply's pictures resolve by it).
        ("cwd", s(&chat["cwd"])),
        ("harness", Value::from(harness)),
        ("model", model),
        ("effort", s(&config["reasoning"])),
        ("fast", Value::Bool(fast)),
        ("options", options),
        ("access", Value::from(access)),
        ("plan", Value::Bool(config["plan"].as_bool() == Some(true))),
        // Its profile, and how the chat stands against it (read as the
        // session is published, never as the draft is typed).
        ("profile", crate::agent_profiles::applied(config)),
        ("running", Value::Bool(live)),
        ("in_flight", Value::Bool(chats.turn_in_flight(open, sending))),
        ("steers", steers.map_or(Value::Nil, Value::Bool)),
        ("stopped", Value::Bool(stopped)),
        ("compacting", Value::Bool(chats.chat_compacting(&open.id))),
        ("compaction_wait", compaction_wait(chats, open)),
        ("compact_plan", compact_plan(chats, open)),
        ("compact_blocked", blocked.clone()),
        ("resume", resume_offer(open, &chat, harness, &blocked)),
        ("prediction", open.prediction.as_deref().map_or(Value::Nil, Value::from)),
        ("limit", open.limit.as_ref().map_or(Value::Nil, |row| limit_value(chats, row))),
        ("limits", Value::List(open.limits.iter().map(|row| limit_value(chats, row)).collect::<Vec<_>>().into())),
        ("work_tree", work_tree_value(&chat["workTree"])),
        ("context", context_tokens.zip(context_window).filter(|(_, w)| *w > 0.).map_or(Value::Nil, |(t, w)| Value::Num((t / w).min(1.)))),
        ("context_tokens", context_tokens.map_or(Value::Nil, Value::Num)),
        ("context_window", context_window.map_or(Value::Nil, Value::Num)),
        ("consulting", Value::Bool(open.entries.last().and_then(|e| e.json.as_ref()).is_some_and(consulting_advisor))),
        ("machine", booting_word(open.machine).map_or(Value::Nil, Value::from)),
        ("ask", ask),
        ("queue", Value::List(queue.into())),
        ("todos", todos(open)),
        ("tasks", Value::List(tasks.into())),
        ("shells_stoppable", Value::Bool(chats.shells_stoppable(&chat))),
        ("turn_at", turn_started(open).map_or(Value::Nil, Value::Num)),
    ];
    if let Some(branch) = chat["branch"].as_str() {
        fields.push(("branch", Value::from(branch)));
    }
    if let Some(reason) = &open.refused {
        fields.push(("notice", Value::record([("tone", Value::from("danger")), ("refused", Value::from(reason.as_str()))])));
    }
    Value::record(fields)
}

fn turn_started(open: &Open) -> Option<f64> {
    open.entries
        .iter()
        .rev()
        .filter_map(|e| e.json.as_ref())
        .find(|e| e["role"].as_str() == Some("user") && e["answersRequestId"].is_null())
        .and_then(|e| e["createdAt"].as_f64())
        .map(|ms| ms / 1000.)
}

/// A chat's own picks (200K, an auto-compact window) are its budget: Claude reports the model's whole window.
fn chat_budget(config: &Json) -> Option<f64> {
    if config["modelOptions"].as_object().is_none_or(serde_json::Map::is_empty) {
        return None;
    }
    let config: zeron_proto::ChatConfig = serde_json::from_value(config.clone()).ok()?;
    zeron_proto::model_options::chat_budget(&config).map(|b| b as f64)
}

fn consulting_advisor(last: &Json) -> bool {
    last["status"].as_str() == Some("streaming")
        && last["parts"].as_array().is_some_and(|parts| {
            parts.iter().any(|p| {
                p["kind"].as_str() == Some("tool")
                    && p["resolved"].as_bool() != Some(true)
                    && p["call"]["kind"].as_str() == Some("unknown")
                    && p["call"]["name"].as_str() == Some(zeron_proto::ADVISOR_TOOL_NAME)
            })
        })
}

fn machine_state(answer: &Json, chat: &str, device: &str) -> Option<zeron_proto::CloudState> {
    let sessions = answer["sessions"].as_array()?;
    let row = sessions
        .iter()
        .find(|s| s["chatId"].as_str() == Some(chat))
        .or_else(|| sessions.iter().find(|s| s["deviceId"].as_str() == Some(device)))?;
    serde_json::from_value(row["state"].clone()).ok()
}

fn booting_word(machine: Option<zeron_proto::CloudState>) -> Option<&'static str> {
    use zeron_proto::CloudState as S;
    match machine? {
        S::Provisioning => Some("starting"),
        S::Starting | S::Sleeping | S::Stopping => Some("waking"),
        _ => None,
    }
}

fn stats_of(frame: &Json) -> Option<zeron_proto::SessionStats> {
    serde_json::from_value(frame.get("sessionStats")?.clone()).ok()
}

fn stats_value(stats: &zeron_proto::SessionStats, last: Option<zeron_proto::SessionStats>) -> Value {
    use zeron_proto::usage::format_tokens;
    let last = last.filter(|t| t.output_tokens > 0).map_or(Value::Nil, |t| {
        Value::record([
            ("tps", t.tokens_per_second().map_or(Value::Nil, |r| Value::from(format!("{r:.0}").as_str()))),
            ("output", Value::from(format_tokens(t.output_tokens).as_str())),
        ])
    });
    Value::record([
        ("turns", Value::Num(stats.turns as f64)),
        ("cache_ttl", stats.cache_ttl_secs.map_or(Value::Nil, |t| Value::Num(f64::from(t)))),
        ("cache_touched", stats.cache_touched_at.map_or(Value::Nil, |t| Value::Num(t as f64))),
        ("last", last),
    ])
}

/// Its stats before the turn `now` ends on began, and that turn's number: a turn seen
/// starting, the one kept while it's still the latest, or a first turn's from nothing.
fn turn_base(
    kept: Option<(zeron_proto::SessionStats, u64)>,
    seen: Option<&zeron_proto::SessionStats>,
    now: &zeron_proto::SessionStats,
) -> Option<(zeron_proto::SessionStats, u64)> {
    let still = kept.filter(|(_, n)| *n == now.turns);
    match seen {
        Some(was) if now.turns == was.turns + 1 => Some((*was, now.turns)),
        Some(was) if now.turns == was.turns => still,
        Some(_) => None,
        None => still.or_else(|| (now.turns == 1).then(|| (zeron_proto::SessionStats::default(), 1))),
    }
}

fn last_turn(before: &zeron_proto::SessionStats, now: &zeron_proto::SessionStats) -> zeron_proto::SessionStats {
    zeron_proto::SessionStats {
        output_tokens: now.output_tokens.saturating_sub(before.output_tokens),
        usage_llm_ms: now.usage_llm_ms.saturating_sub(before.usage_llm_ms),
        ..Default::default()
    }
}

/// The running turn's output as the engine's live meter counts it before a bill corrects it:
/// its replies' words and reasoning since the prompt, a token to four characters.
fn turn_tokens(entries: &[Entry]) -> Option<u64> {
    let last = entries.last()?.json.as_ref()?;
    if last["status"].as_str() != Some("streaming") {
        return None;
    }
    let chars: usize = entries
        .iter()
        .rev()
        .map_while(|e| e.json.as_ref().filter(|j| j["role"].as_str() != Some("user")))
        .flat_map(|j| j["parts"].as_array().into_iter().flatten())
        .filter(|p| matches!(p["kind"].as_str(), Some("text" | "reasoning")))
        .map(|p| p["text"].as_str().map_or(0, |t| t.chars().count()))
        .sum();
    (chars > 0).then(|| zeron_proto::transcript::estimated_tokens(chars))
}

/// `zeron_doc::schema::turn_stopped` on the wire's entry: this app doesn't depend on zeron-doc.
fn offers_resume(live: bool, sending: bool, awaiting_turn: bool, last: Option<&Json>) -> bool {
    !live && !sending && !awaiting_turn && last.is_some_and(turn_stopped)
}

fn turn_stopped(last: &Json) -> bool {
    let status = last["status"].as_str();
    last["role"].as_str() == Some("user")
        || status == Some("aborted")
        || (status != Some("streaming") && last["parts"].as_array().and_then(|p| p.last()).is_some_and(|p| p["kind"].as_str() == Some("error")))
}

fn limit_row(frame: &Json, chat: &str) -> Option<Json> {
    frame["chats"].as_array()?.iter().find(|c| c["chatId"].as_str() == Some(chat)).cloned()
}

fn offered_limits(frame: &Json, chat: &str) -> Vec<Json> {
    frame["chats"].as_array().into_iter().flatten().filter(|c| c["chatId"].as_str() != Some(chat) && c["offer"].is_object()).cloned().collect()
}

fn limit_value(chats: &Chats, row: &Json) -> Value {
    let offer = &row["offer"];
    let to = &offer["to"];
    let label = to["harness"].as_str().and_then(|harness| {
        to["model"].as_str().and_then(|model| chats.label_of(harness, model)).or_else(|| chats.harness_name(harness)).or_else(|| Some(harness.to_owned()))
    });
    // The shipping app's `target_text`.
    let target = to["harness"].as_str().map(|harness| {
        let mut parts = vec![chats.harness_name(harness).unwrap_or_else(|| harness.to_owned())];
        if let Some(model) = to["model"].as_str() {
            parts.push(chats.label_of(harness, model).unwrap_or_else(|| model.to_owned()));
        }
        if let Some(level) = to["reasoning"].as_str() {
            parts.push(effort_label(level).to_owned());
        }
        parts.join(" · ")
    });
    let mark = to["harness"].as_str().and_then(|harness| {
        let id = serde_json::from_value::<zeron_proto::HarnessId>(json!(harness)).ok();
        zeron_proto::HarnessId::brand_of(id, to["model"].as_str()).and_then(|maker| crate::agents::glyph(maker.as_str())).or_else(|| crate::agents::glyph(harness))
    });
    let harness = chats.chat(row["chatId"].as_str().unwrap_or_default()).and_then(|c| c["config"]["harness"].as_str().map(|h| chats.harness_name(h).unwrap_or_else(|| h.to_owned())));
    Value::record([
        ("chat", row["chatId"].as_str().map_or(Value::Nil, Value::from)),
        ("resets_at", row["resetsAt"].as_f64().map_or(Value::Nil, Value::Num)),
        ("auto", Value::Bool(row["auto"].as_bool() == Some(true))),
        ("to_label", label.as_deref().map_or(Value::Nil, Value::from)),
        ("to", Value::Bool(to.is_object())),
        ("preselect", offer["preselect"].as_str().map_or(Value::Nil, Value::from)),
        ("act_at", offer["actAt"].as_f64().map_or(Value::Nil, Value::Num)),
        ("target", target.as_deref().map_or(Value::Nil, Value::from)),
        ("to_mark", mark.as_deref().map_or(Value::Nil, Value::from)),
        ("harness", harness.as_deref().map_or(Value::Nil, Value::from)),
    ])
}

fn work_tree_value(tree: &Json) -> Value {
    let Some(path) = tree["path"].as_str() else { return Value::Nil };
    let (state, commits) = work_tree_state(&tree["state"]);
    let folder = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);
    Value::record([
        ("path", Value::from(path)),
        ("folder", Value::from(folder)),
        ("branch", tree["branch"].as_str().map_or(Value::Nil, Value::from)),
        ("state", state.map_or(Value::Nil, Value::from)),
        ("commits", Value::Num(commits)),
        ("dirty", Value::Num(tree["dirtyFiles"].as_f64().unwrap_or(0.))),
        ("removed", Value::Bool(tree["removed"].as_bool() == Some(true))),
    ])
}

/// The processes the chat's agent left running in the background, oldest
/// first: listed while its session row is fresh, whatever the turn does
/// (the shipping app's `view::background_tasks`). They outlive the turn
/// that started them, and the engine's heartbeat keeps the row fresh while
/// they run; gating them on a live turn emptied the tray each time a turn
/// ended and filled it again as the next began.
fn background_tasks(session: &Json) -> Vec<Value> {
    if stale(&session["updatedAt"]) {
        return Vec::new();
    }
    let s = |j: &Json| j.as_str().map(Value::from).unwrap_or(Value::Nil);
    session["backgroundTasks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let command = t["command"].as_str().unwrap_or_default();
            let name = t["description"].as_str().filter(|d| !d.is_empty()).unwrap_or(command);
            let mut fields = vec![
                ("id", s(&t["id"])),
                ("kind", Value::from("process")),
                ("task_kind", s(&t["kind"])),
                ("name", Value::from(name)),
            ];
            if name != command {
                fields.push(("detail", Value::from(command)));
            }
            if let Some(since) = seconds(&t["startedAt"]) {
                fields.push(("since", Value::Num(since as f64)));
            }
            Value::record(fields)
        })
        .collect()
}

/// The plan: the latest todo list the agent wrote.
fn todos(open: &Open) -> Value {
    let latest = latest_todo(open.entries.iter().rev().filter_map(|e| e.json.as_ref()))
        .or(open.older_todo.as_ref().map(|(_, part)| part));
    let Some(part) = latest else {
        return Value::List(Rc::from([]));
    };
    let items: Vec<Value> = part["call"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, t)| {
            let state = match t["status"].as_str() {
                Some("completed") => "done",
                Some("inProgress") => "doing",
                _ if t["done"].as_bool() == Some(true) => "done",
                _ => "pending",
            };
            Value::record([
                ("id", Value::from(format!("todo{i}").as_str())),
                ("text", Value::from(t["text"].as_str().unwrap_or_default())),
                ("state", Value::from(state)),
            ])
        })
        .collect();
    Value::List(items.into())
}

/// How long a sent prompt reads "sending" with no turn to show for it, as
/// the shipping app's grace before it calls a send undelivered.
const TURN_START_GRACE: std::time::Duration = std::time::Duration::from_secs(10);
const SEND_GRACE_SECONDS: f64 = 120.;

/// A record with more fields (or some replaced).
fn with_fields<'a>(record: Value, extra: impl IntoIterator<Item = (&'a str, Value)>) -> Value {
    let Value::Map(fields) = &record else { return record };
    let mut all: Vec<(std::rc::Rc<str>, Value)> = fields.iter().cloned().collect();
    for (key, value) in extra {
        all.retain(|(k, _)| &**k != key);
        all.push((key.into(), value));
    }
    Value::record(all)
}

fn draft_value(id: &str, text: &str) -> Value {
    Value::record([("id", Value::from(id)), ("text", Value::from(text))])
}

/// `draft` with the thought a reply is at, when its last part is one: its id
/// and its words so far, as its row shows them (`tool_live`). The row reads
/// them here, so a tick of them doesn't publish the whole transcript.
fn with_thought(draft: Value, parts: Option<&Vec<Json>>) -> Value {
    match parts.and_then(|parts| parts.last()).filter(|p| p["kind"].as_str() == Some("reasoning")) {
        Some(thought) => with_fields(
            draft,
            [
                ("thought", Value::from(thought["id"].as_str().unwrap_or_default())),
                ("thought_text", Value::from(thought_words(thought["text"].as_str().unwrap_or_default()).as_str())),
            ],
        ),
        None => draft,
    }
}

// -- small things ---------------------------------------------------------------

fn now_seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A time as Unix seconds: milliseconds since 1970, or RFC 3339.
fn seconds(value: &Json) -> Option<i64> {
    if let Some(ms) = value.as_f64() {
        return Some((ms / 1000.) as i64);
    }
    let stamp = value.as_str()?;
    let (date, time) = stamp.split_once('T')?;
    let mut date = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (date.next()??, date.next()??, date.next()??);
    let clock: String = time.chars().take(8).collect();
    let mut clock = clock.split(':').map(|p| p.parse::<i64>().ok());
    let (h, min, s) = (clock.next()??, clock.next()??, clock.next()??);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let mut out = days * 86_400 + h * 3600 + min * 60 + s;
    if let Some(sign_at) = time.get(8..).and_then(|t| t.find(['+', '-'])) {
        let offset = &time[8 + sign_at..];
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let mut parts = offset[1..].split(':').map(|p| p.parse::<i64>().unwrap_or(0));
        let (oh, om) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        out -= sign * (oh * 3600 + om * 60);
    }
    Some(out)
}

fn stale(updated: &Json) -> bool {
    seconds(updated).is_some_and(|at| now_seconds() - at > STALE_SECONDS)
}

/// "Jul 1, 4:12 PM", in this Mac's time zone (the shipping app's
/// `format_timestamp`).
#[cfg(unix)]
fn stamp(at: i64) -> Option<String> {
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: std::ffi::c_long,
        zone: *const std::ffi::c_char,
    }
    unsafe extern "C" {
        fn localtime_r(time: *const i64, out: *mut Tm) -> *mut Tm;
    }
    if at <= 0 {
        return None;
    }
    let mut tm = std::mem::MaybeUninit::<Tm>::zeroed();
    // SAFETY: `localtime_r` writes the broken-down local time into the
    // caller's struct (the C library's `struct tm`, laid out as above on
    // macOS and glibc) and reads only `at`.
    let tm = unsafe {
        if localtime_r(&at, tm.as_mut_ptr()).is_null() {
            return None;
        }
        tm.assume_init()
    };
    let hour = match tm.hour % 12 {
        0 => 12,
        h => h,
    };
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let month = MONTHS.get(tm.mon as usize).copied().unwrap_or("Jan");
    Some(format!(
        "{month} {}, {hour}:{:02} {}",
        tm.mday,
        tm.min,
        if tm.hour < 12 { "AM" } else { "PM" }
    ))
}

#[cfg(not(unix))]
fn stamp(_at: i64) -> Option<String> {
    None
}

/// This window process's id, as an editor of queued messages.
fn instance_id() -> String {
    thread_local! {
        static ID: String = new_id();
    }
    ID.with(Clone::clone)
}

/// A random UUID (v4), as the engine's own are.
fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for (i, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        hasher.write_usize(i);
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = match chunk.len() {
            3 => (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]),
            2 => (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8),
            _ => u32::from(chunk[0]) << 16,
        };
        let sym = |shift: u32| TABLE[((n >> shift) & 63) as usize] as char;
        out.push(sym(18));
        out.push(sym(12));
        out.push(if chunk.len() > 1 { sym(6) } else { '=' });
        out.push(if chunk.len() > 2 { sym(0) } else { '=' });
    }
    out
}

#[cfg(test)]
#[path = "chat_pending_tests.rs"]
mod pending_tests;

#[cfg(test)]
mod tests {
    /// A queued message's files come back to an edit as chips named as they
    /// were attached, already on the host, a picture shown where it can be.
    #[test]
    fn a_queued_file_is_a_chip_named_as_it_was_attached() {
        let chip = super::queued_attachment("/data/uploads/9e97a72f-shot.png");
        assert_eq!(chip.field("name"), Some(&Value::from("shot.png")));
        assert_eq!(chip.field("kind"), Some(&Value::from("image")));
        assert_eq!(chip.field("on_host"), Some(&Value::Bool(true)));
        assert_eq!(chip.field("src"), Some(&Value::from("/data/uploads/9e97a72f-shot.png")));
        let file = super::queued_attachment("/repo/notes/design-notes.txt");
        assert_eq!(file.field("name"), Some(&Value::from("design-notes.txt")), "only an upload's id prefix goes");
        assert_eq!(file.field("kind"), Some(&Value::from("file")));
        assert_eq!(file.field("src"), None);
        let remote = super::queued_attachment("pending://up-1/photo.jpg");
        assert_eq!(remote.field("name"), Some(&Value::from("photo.jpg")));
        assert_eq!(remote.field("kind"), Some(&Value::from("image")));
        assert_eq!(remote.field("src"), None, "nothing here to show yet");
        assert_eq!(remote.field("path"), Some(&Value::from("pending://up-1/photo.jpg")));
    }

    #[test]
    fn a_queued_row_is_steered_unless_it_carries_files() {
        assert_eq!(queue_send_method(false), "SteerQueuedMessageNow");
        assert_eq!(queue_send_method(true), "SendQueuedMessageNow");
        let plain = json!({ "id": "r", "text": "hi" });
        let carrying = json!({ "id": "r", "text": "hi", "attachments": ["/up/a.png"] });
        assert!(runs_now(&carrying, true, Some(false)));
        assert!(runs_now(&plain, false, Some(false)));
        assert!(runs_now(&plain, true, Some(true)));
        assert!(!runs_now(&plain, true, Some(false)));
        assert!(!runs_now(&plain, true, None));
    }

    #[test]
    fn steering_mid_turn_needs_both_of_the_harness_s_words() {
        assert!(steers_of(&json!({ "supportsSteering": true, "steeringMode": "step-boundary" })));
        assert!(!steers_of(&json!({ "supportsSteering": true, "steeringMode": "turn-boundary" })));
        assert!(!steers_of(&json!({ "supportsSteering": false, "steeringMode": "step-boundary" })));
    }

    #[test]
    fn rows_steered_as_one_are_numbered_blocks() {
        assert_eq!(numbered(&["  first \n".into(), "second".into()]), "1. first\n\n2. second");
    }

    #[test]
    fn steer_all_sends_only_the_rows_that_came_off() {
        let row = |text: &str| SteerRow { id: text.into(), text: text.into(), paths: vec![] };
        assert_eq!(whole_of(&[row("a"), row("b")]).0, "1. a\n\n2. b");
        assert_eq!(whole_of(&[row("b")]).0, "b");
    }

    #[test]
    fn resume_waits_for_the_turn_a_send_just_started() {
        let user = json!({ "role": "user" });
        assert!(offers_resume(false, false, false, Some(&user)));
        assert!(!offers_resume(false, false, true, Some(&user)));
        assert!(!offers_resume(true, false, false, Some(&user)));
        assert!(!offers_resume(false, true, false, Some(&user)));
    }

    #[test]
    fn the_resume_prompt_is_a_line_not_a_bubble() {
        let said = |words: &str| json!({ "id": "u1", "role": "user", "parts": [{ "kind": "text", "text": words }] });
        let row = message(&said(zeron_proto::RESUME_PROMPT), None).unwrap();
        assert_eq!(text_of(&row, "role"), "system");
        assert_eq!(text_of(&row, "line"), "resumed");
        assert_eq!(text_of(&row, "hint"), zeron_proto::RESUME_PROMPT);
        assert_eq!(text_of(&message(&said("Resume, then explain."), None).unwrap(), "role"), "user");
        let pending = |waiting| Pending {
            id: "p".into(),
            words: zeron_proto::RESUME_PROMPT.into(),
            paths: Vec::new(),
            waiting,
            retry: None,
            command: None,
            at: None,
            alias: None,
        };
        assert_eq!(text_of(&pending("sending").value(), "line"), "resumed");
        assert_eq!(text_of(&pending("failed").value(), "role"), "user", "its line to send it again is a prompt's");
    }

    #[test]
    fn a_turn_stopped_short_is_the_gpui_three() {
        assert!(turn_stopped(&json!({ "role": "user", "parts": [] })));
        assert!(turn_stopped(&json!({ "role": "assistant", "status": "aborted", "parts": [] })));
        assert!(turn_stopped(&json!({ "role": "assistant", "status": "complete", "parts": [{ "kind": "text" }, { "kind": "error" }] })));
        assert!(!turn_stopped(&json!({ "role": "assistant", "status": "streaming", "parts": [{ "kind": "error" }] })));
        assert!(!turn_stopped(&json!({ "role": "assistant", "status": "complete", "parts": [{ "kind": "error" }, { "kind": "text" }] })));
    }

    #[test]
    fn a_reset_keeps_what_it_left_as_it_was() {
        let built_of = |ids: &[&str]| -> std::collections::HashMap<Rc<str>, super::Built> {
            ids.iter().map(|id| (Rc::from(*id), super::Built::default())).collect()
        };
        let mut before: Vec<super::Entry> = [
            serde_json::json!({ "id": "a", "text": "same" }),
            serde_json::json!({ "id": "b", "text": "was" }),
            serde_json::json!({ "id": "gone", "text": "left" }),
        ]
        .into_iter()
        .map(super::Entry::new)
        .collect();
        // (Settled: told from its sum alone.)
        let mut no_plan = None;
        super::settle(&mut before[0], 0, &mut no_plan);
        assert!(before[0].json.is_none());
        let now: Vec<super::Entry> = [
            serde_json::json!({ "id": "a", "text": "same" }),
            serde_json::json!({ "id": "b", "text": "now" }),
            serde_json::json!({ "id": "new", "text": "came" }),
        ]
        .into_iter()
        .map(super::Entry::new)
        .collect();
        let mut built = built_of(&["a", "b", "gone"]);
        super::keep_unchanged(&mut built, &before, &now);
        assert!(built.contains_key("a"));
        assert!(!built.contains_key("b"), "changed");
        assert!(!built.contains_key("gone"), "left the transcript");
        assert!(!built.contains_key("new"), "never built");
    }

    #[test]
    fn a_settled_entry_keeps_its_plan_and_grows_only_from_a_whole_copy() {
        let plan = |text: &str| {
            serde_json::json!({ "id": text, "role": "assistant", "parts": [
                { "id": "p", "kind": "tool", "call": { "kind": "todo", "items": [{ "text": text, "status": "pending" }] } },
            ] })
        };
        let mut older = None;
        let (mut first, mut second) = (super::Entry::new(plan("first")), super::Entry::new(plan("second")));
        // (Newest first, as a long chat's rows are built, and the other way.)
        super::settle(&mut second, 1, &mut older);
        super::settle(&mut first, 0, &mut older);
        assert_eq!(older.as_ref().unwrap().1["call"]["items"][0]["text"], "second");
        let open = super::Open { entries: vec![first, second], older_todo: older, ..super::Open::default() };
        assert_eq!(super::todos(&open).list()[0].field("text"), Some(&Value::from("second")));
        // Words for an entry whose copy is gone: out of step, so a reset comes.
        let mut entries = open.entries;
        let mut words = serde_json::json!({ "append": [{ "entry": "first", "part": "p", "text": "x", "len": 1 }] });
        assert!(super::apply(&mut entries, &mut words).is_err());
    }

    /// The worker's fold, kept, is what folding here keeps.
    #[test]
    fn the_worker_folds_as_the_window_would() {
        let entry = serde_json::json!({
            "id": "e1", "role": "assistant", "status": "complete", "createdAt": 1000.0,
            "parts": [
                { "id": "t", "kind": "text", "text": "Some words" },
                { "id": "k", "kind": "tool", "subagentRef": "doc--sub--1", "subagentStatus": "done",
                  "call": { "kind": "mcp", "tool": "Agent: look", "server": "s" } },
            ],
        });
        let context = Rc::new(super::RowContext { doc: "chat".into(), device: None, model: Some("Opus".into()), cwd: None });
        let here = super::built_for(&entry, &context, true);
        let folding = super::folding(vec![(0, "e1".into(), entry.clone())], &context);
        let folded = folding.results.recv().unwrap().pop().unwrap();
        let mut open = super::Open { entries: vec![super::Entry::new(entry.clone())], ..super::Open::default() };
        open.entries[0].json = None;
        super::settle_off(&mut open, folded, &context);
        let there = open.built.get("e1").expect("kept");
        assert_eq!(there.row.as_ref().unwrap().to_json(), here.row.as_ref().unwrap().to_json());
        assert_eq!(there.subagents.as_ref().map(|s| s[0].to_json()), here.subagents.as_ref().map(|s| s[0].to_json()));
        assert_eq!(open.entries[0].sum, super::sum_of(&entry));
    }

    /// A row out of view holds what makes it, not itself: what it opens to is
    /// the row it would have been, and a row with subagents still lists them.
    #[test]
    fn a_folded_row_opens_to_the_row_it_was() {
        let entry = serde_json::json!({
            "id": "e1", "role": "assistant", "status": "complete", "createdAt": 1000.0,
            "parts": [
                { "id": "t", "kind": "text", "text": "Some words ".repeat(200) },
                { "id": "k", "kind": "tool", "subagentRef": "doc--sub--1", "subagentStatus": "done",
                  "call": { "kind": "mcp", "tool": "Agent: look", "server": "s" } },
            ],
        });
        let context = Rc::new(super::RowContext { doc: "chat".into(), device: None, model: Some("Opus".into()), cwd: None });
        let whole = super::built_for(&entry, &context, false);
        let folded = super::built_for(&entry, &context, true);
        let Some(Value::Folded(_)) = folded.row else { panic!("folded") };
        let (whole_row, folded_row) = (whole.row.unwrap(), folded.row.unwrap());
        // What a script gets, and what it reads of it.
        assert_eq!(folded_row.to_json(), whole_row.to_json());
        assert_eq!(folded_row.field("id").map(text).as_deref(), Some("e1"));
        assert_eq!(folded.subagents.as_deref().map(<[Value]>::len), Some(1));
        assert_eq!(folded.subagents, whole.subagents);
        // It's kept small: words that repeat, a good deal less than they are.
        let streaming = serde_json::json!({ "id": "e2", "role": "assistant", "status": "streaming", "parts": [] });
        assert!(!matches!(super::built_for(&streaming, &context, true).row, Some(Value::Folded(_))), "still being written");
    }

    #[test]
    fn what_a_kept_compressor_compresses_opens_again() {
        let long = "words that repeat ".repeat(4000);
        for input in [long.as_bytes(), b"short", b""] {
            let out = super::compressed(input);
            assert_eq!(miniz_oxide::inflate::decompress_to_vec(&out).unwrap(), input);
        }
        assert!(super::compressed(long.as_bytes()).len() < long.len() / 10);
    }

    #[test]
    fn a_row_holds_each_of_its_repeats_once() {
        let tool = || Value::record([("id", Value::from("t1")), ("kind", Value::from("exec")), ("call", Value::from("cargo build"))]);
        let words = "the same words";
        let mut row = Value::record([
            ("text", Value::from(words)),
            ("copy", Value::from(words)),
            ("tools", Value::List(Rc::from([tool()]))),
            ("blocks", Value::List(Rc::from([Value::record([("kind", Value::from("group")), ("tools", Value::List(Rc::from([tool()])))])]))),
        ]);
        let before = row.clone();
        // (Built just now: nothing else holds its parts.)
        drop(before.clone());
        let same = row == before;
        drop(before);
        super::share_parts(&mut row, &mut super::Seen::default());
        assert!(same);
        let tools = row.field("tools").unwrap().list();
        let grouped = row.field("blocks").unwrap().list()[0].field("tools").unwrap().list();
        let (Value::Map(one), Value::Map(two)) = (&tools[0], &grouped[0]) else { panic!("records") };
        assert!(Rc::ptr_eq(one, two), "one copy of a tool");
        let (Some(Value::Str(text)), Some(Value::Str(copy))) = (row.field("text"), row.field("copy")) else { panic!("words") };
        assert!(Rc::ptr_eq(text, copy), "one copy of its words");
    }

    /// A settled reply's tools are built once, for all its layouts, and read
    /// exactly as they would built for each.
    #[test]
    fn a_settled_row_builds_each_tool_once_and_reads_the_same() {
        let entry = serde_json::json!({
            "id": "e", "role": "assistant", "status": "complete", "createdAt": 1000., "durationMs": 5000.,
            "parts": [
                { "id": "r", "kind": "reasoning", "text": "thinking it over" },
                { "id": "t", "kind": "tool", "call": { "kind": "exec", "command": "cargo build" }, "output": "ok" },
                { "id": "w", "kind": "text", "text": "Done." },
            ],
        });
        let row = super::built_entry(&entry, "doc", None, None).unwrap();
        assert_eq!(row, super::message(&entry, None).unwrap());
        let tools = row.field("tools").unwrap().list();
        let grouped = row.field("blocks").unwrap().list()[0].field("tools").unwrap().list();
        let (Value::Map(one), Value::Map(two)) = (&tools[1], &grouped[1]) else { panic!("records") };
        assert!(Rc::ptr_eq(one, two), "one tool, in both layouts");
        // (Built for this row alone: nothing's kept for the next.)
        assert!(super::ROW_BUILD.with_borrow(Option::is_none));
    }

    #[test]
    fn another_views_rows_are_taken_only_as_they_are_here() {
        let entries: Vec<super::Entry> = [
            serde_json::json!({ "id": "a", "text": "same" }),
            serde_json::json!({ "id": "b", "text": "theirs" }),
        ]
        .into_iter()
        .map(super::Entry::new)
        .collect();
        let mut open = super::Open {
            id: "chat".into(),
            entries: [
                serde_json::json!({ "id": "a", "text": "same" }),
                serde_json::json!({ "id": "b", "text": "mine" }),
            ]
            .into_iter()
            .map(super::Entry::new)
            .collect(),
            ..super::Open::default()
        };
        let theirs: std::collections::HashMap<Rc<str>, super::Built> =
            [(Rc::from("a"), super::Built::default()), (Rc::from("b"), super::Built::default())].into_iter().collect();
        // (Built for another model's name: none of it.)
        open.built_model = Some(Some("mine".into()));
        super::reuse_built(&mut open, &entries, &theirs, &Some(Some("theirs".into())));
        assert!(open.built.is_empty());
        open.built_model = None;
        super::reuse_built(&mut open, &entries, &theirs, &Some(Some("theirs".into())));
        assert!(open.built.contains_key("a"));
        assert!(!open.built.contains_key("b"));
        assert_eq!(open.built_model, Some(Some("theirs".into())));
    }


    #[test]
    fn a_sent_pick_shows_its_page_over_its_number_and_words() {
        let pick = Pick {
            index: 2,
            element: "button#export.btn".into(),
            url: "https://www.example.com/dashboard/revenue?tab=1".into(),
            words: "Export report".into(),
        };
        let chip = annotation_chip(&pick);
        assert!(matches!(chip.field("name"), Some(Value::Str(s)) if s.as_ref() == "example.com/dashboard/revenue?tab=1"));
        assert!(matches!(chip.field("meta"), Some(Value::Str(s)) if s.as_ref() == "Annotation 2 · Export report"));
        assert_eq!(page_of("http://localhost:3000/"), "localhost:3000");
        let bare = Pick { index: 1, element: "div.card".into(), url: String::new(), words: String::new() };
        assert!(matches!(annotation_chip(&bare).field("name"), Some(Value::Str(s)) if s.as_ref() == "Annotation 1"));
    }

    #[test]
    fn a_subagent_spawn_carries_its_doc_status_and_title() {
        let spawn = serde_json::json!({
            "id": "t1", "kind": "tool",
            "call": { "kind": "unknown", "name": "Agent: scan the repo", "input": { "prompt": "Look around" } },
            "subagentRef": "chat-1--sub--7", "subagentStatus": "running",
        });
        let row = super::tool(&spawn).unwrap();
        let sub = row.field("subagent").unwrap();
        assert_eq!(sub.field("ref").map(crate::auth::text).as_deref(), Some("chat-1--sub--7"));
        assert_eq!(sub.field("status").map(crate::auth::text).as_deref(), Some("running"));
        assert_eq!(sub.field("title").map(crate::auth::text).as_deref(), Some("scan the repo"));
        // A spawn the engine hasn't stamped yet, and any other tool: none.
        let unstamped = serde_json::json!({ "id": "t2", "kind": "tool", "call": { "kind": "unknown", "name": "Agent" } });
        assert!(super::tool(&unstamped).unwrap().field("subagent").is_none());
        let read = serde_json::json!({ "id": "t3", "kind": "tool", "call": { "kind": "readFile", "path": "a.rs" }, "subagentRef": "x" });
        assert!(super::tool(&read).unwrap().field("subagent").is_none());
    }

    /// A side chat made here and sent to at once (before the engine's list
    /// has it): its sender knows it as it was made, its parent's agent and
    /// folder, never a config with no agent (which the engine refuses as
    /// bad params: the user's "Your message didn't send", Nov 2).
    #[test]
    fn a_chat_just_made_is_sent_as_it_was_made() {
        let id = "77777777-7777-4777-8777-777777777777";
        let params = serde_json::json!({
            "op": "createChat",
            "chatId": id,
            "parentChatId": "p",
            "deviceId": "d",
            "spaceId": "s",
            "cwd": "/repo",
            "config": { "harness": "devin", "model": "swe-2-high", "reasoning": "high", "modelOptions": {}, "sandbox": "workspace-write" },
        });
        super::made(&params);
        let row = super::CHATS.with(|c| c.chat(id)).expect("known at once");
        assert_eq!(row["cwd"], "/repo");
        assert_eq!(row["parentChatId"], "p");
        assert!(row.get("op").is_none() && row.get("chatId").is_none());
        let none = zgx_script::Value::from_json("{}").unwrap();
        let config = super::merged_config(&row["config"], &none);
        assert_eq!(config["harness"], "devin");
        assert_eq!(config["model"], "swe-2-high");
        // Unknown, with nothing picked: no agent (the sender stops there).
        assert!(super::merged_config(&serde_json::Value::Null, &none)["harness"].is_null());
        // The composer's pick wins.
        let picked = zgx_script::Value::from_json(r#"{"model":"codex/gpt-6.1"}"#).unwrap();
        let config = super::merged_config(&row["config"], &picked);
        assert_eq!((config["harness"].as_str(), config["model"].as_str()), (Some("codex"), Some("gpt-6.1")));
    }

    #[test]
    fn a_finished_subagent_s_upload_is_its_transcript() {
        assert_eq!(super::blob_ref(Some("c2"), "c1--sub--3"), "c1/c1--sub--3", "a copied chip keeps its first chat");
        assert_eq!(super::blob_ref(Some("c2"), "odd-doc"), "c2/odd-doc");
        let reply = serde_json::json!({ "text": "[{\"id\":\"e1\",\"role\":\"assistant\"}]" });
        assert_eq!(super::snapshot_frame(&reply).unwrap()["reset"][0]["id"], "e1");
        assert!(super::snapshot_frame(&serde_json::json!({ "text": "not json" })).is_none());
        assert!(super::snapshot_frame(&serde_json::json!({})).is_none());
    }

    #[test]
    fn a_thought_s_row_gets_all_its_words_and_folds_them_itself() {
        let part = json!({ "id": "r", "kind": "reasoning", "text": "Look at the parser.\nThen the host." });
        let row = super::tool(&part).unwrap();
        assert!(matches!(row.field("verb"), Some(Value::Str(s)) if s.as_ref() == "Thought process"));
        assert!(matches!(row.field("call"), Some(Value::Str(s)) if s.as_ref() == "Look at the parser.\nThen the host."));
        assert!(matches!(row.field("prose"), Some(Value::Bool(true))));
        // Settled, its fold shows its first lines; being written, its latest.
        assert!(matches!(row.field("live"), Some(Value::Bool(false))));
        let writing = super::tool_live(&part, true).unwrap();
        assert!(matches!(writing.field("live"), Some(Value::Bool(true))));
        assert!(super::tool(&json!({ "id": "e", "kind": "reasoning", "text": "  " })).is_none());
        // Long: every line reaches the row (it folds them under "Show all").
        let long: Vec<String> = (1..=30).map(|n| format!("line {n}")).collect();
        let words = super::thought_words(&format!("\n{}\n", long.join("\n")));
        assert!(words.starts_with("line 1\n") && words.ends_with("\nline 30"), "{words}");
        assert_eq!(words.lines().count(), 30);
    }

    #[test]
    fn another_devices_catalog_is_the_composers_and_names_its_device() {
        let chats = Rc::new(Chats::default());
        *chats.here.borrow_mut() = Some("mac".into());
        *chats.harnesses.borrow_mut() = vec![json!({ "id": "codex", "name": "Codex" })];
        chats.models.borrow_mut().insert("codex".into(), vec![json!({ "id": "gpt", "label": "GPT" })]);
        let local = chats.catalog_value();
        assert!(local.field("device").is_none());
        // On the box: its agents, once listed (looking until then).
        *chats.remote.borrow_mut() = Some(Remote {
            device: "box".into(),
            harnesses: vec![json!({ "id": "claude-code", "name": "Claude Code" })],
            models: HashMap::new(),
            ready: false,
        });
        let waiting = chats.catalog_value();
        assert!(matches!(waiting.field("state"), Some(Value::Str(s)) if s.as_ref() == "looking"));
        assert!(matches!(waiting.field("device"), Some(Value::Str(s)) if s.as_ref() == "box"));
        if let Some(r) = chats.remote.borrow_mut().as_mut() {
            r.models.insert("claude-code".into(), vec![json!({ "id": "opus", "label": "Opus" })]);
            r.ready = true;
        }
        let there = chats.catalog_value();
        let Some(Value::List(harnesses)) = there.field("harnesses") else { panic!("harnesses") };
        assert_eq!(harnesses.len(), 1);
        assert!(matches!(harnesses[0].field("id"), Some(Value::Str(s)) if s.as_ref() == "claude-code"));
        // Labels: this device's, else the box's.
        assert_eq!(chats.label_of("codex", "gpt").as_deref(), Some("GPT"));
        assert_eq!(chats.label_of("claude-code", "opus").as_deref(), Some("Opus"));
        // Back here: this device's again.
        chats.catalog_on("mac");
        assert!(chats.remote.borrow().is_none());
        assert!(chats.catalog_value().field("device").is_none());
    }

    #[test]
    fn subagent_titles_follow_the_shipping_app() {
        let t = |call: serde_json::Value| super::subagent_title(&call);
        assert_eq!(t(serde_json::json!({ "kind": "unknown", "name": "Agent: scan repo" })), "scan repo");
        // A bare genus has nothing to say: its description, then its prompt.
        assert_eq!(t(serde_json::json!({ "kind": "unknown", "name": "Agent", "input": { "description": "Find the flake" } })), "Find the flake");
        assert_eq!(t(serde_json::json!({ "kind": "mcp", "tool": "Task", "input": { "prompt": "\n  Read lib.rs\nthen more" } })), "Read lib.rs");
        assert_eq!(t(serde_json::json!({ "kind": "unknown", "name": "Taskmaster" })), "Taskmaster");
        assert_eq!(t(serde_json::json!({ "kind": "exec" })), "Subagent");
        let long = "x".repeat(50);
        assert_eq!(t(serde_json::json!({ "kind": "unknown", "name": long })).chars().count(), 41);
    }

    use super::*;

    /// The activity fixture (ui/activity_fixture.luau), headless.
    fn activity_app() -> zgx_script::testing::TestApp {
        let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/activity_fixture.luau");
        let app = zgx_script::testing::TestApp::new(|runtime| ScriptHost::new(runtime, entry));
        app.frames(10);
        app
    }

    const TOGGLE: &str = "role:button \"Read 1 file · ran 1 command\"";

    #[test]
    fn a_press_on_a_groups_toggle_selects_no_text() {
        let mut app = activity_app();
        app.click(TOGGLE);
        app.frames(20);
        assert!(!app.ui.has_text_selection(), "a click on the toggle selected text");
        app.click(TOGGLE);
        app.frames(20);
        app.double_click(TOGGLE);
        app.frames(20);
        assert!(!app.ui.has_text_selection(), "a double click on the toggle selected text");
    }

    /// The group open (its agent working), then closed by a press, a double
    /// press, a press that drags off: the fold moves the words under the
    /// pointer as it closes.
    #[test]
    fn closing_an_open_group_selects_no_text() {
        let mut app = activity_app();
        app.host.shared("activity_live", zgx_script::Value::Bool(false)).set(zgx_script::Value::Bool(true));
        app.frames(20);
        app.click(TOGGLE);
        app.frames(30);
        assert!(!app.ui.has_text_selection(), "closing the group with a click selected text");
        app.double_click(TOGGLE);
        app.frames(30);
        assert!(!app.ui.has_text_selection(), "a double click on the toggle selected text");
        app.drag(TOGGLE, "~\"And more words\"");
        app.frames(30);
        assert!(!app.ui.has_text_selection(), "a drag from the toggle selected text");
    }

    /// The same in a session's transcript (ui/chat_fixture.luau): its
    /// selectable list of measured rows, a reply's group folding in it.
    #[test]
    fn toggling_a_group_in_the_transcript_selects_no_text() {
        let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/chat_fixture.luau");
        let mut app = zgx_script::testing::TestApp::new(|runtime| ScriptHost::new(runtime, entry));
        app.frames(10);
        app.host.run_command("go_s1");
        app.frames(30);
        app.host.run_command("start_reply");
        app.host.run_command("add_tool");
        app.host.run_command("add_tool");
        app.frames(30);
        let toggle = "role:button \"Ran 2 commands\"";
        for _ in 0..3 {
            app.click(toggle);
            app.frames(30);
            assert!(!app.ui.has_text_selection(), "a click on the toggle selected text");
        }
        app.double_click(toggle);
        app.frames(30);
        assert!(!app.ui.has_text_selection(), "a double click on the toggle selected text");
        app.drag(toggle, "~\"Part 30 works\"");
        app.frames(30);
        assert!(!app.ui.has_text_selection(), "a drag from the toggle selected text");
    }

    /// The interface as boxes, without node ids (a rebuild makes new
    /// ones): what a person sees move, appear or go.
    /// A tree line's box: its top and height.
    fn top_and_height(line: &str) -> Option<(f32, f32)> {
        let words: Vec<&str> = line.split_whitespace().collect();
        words.windows(2).rev().find_map(|pair| {
            let (_, y) = pair[0].split_once(',')?;
            let (_, h) = pair[1].split_once('x')?;
            Some((y.parse().ok()?, h.parse().ok()?))
        })
    }

    fn boxes(app: &mut zgx_script::testing::TestApp) -> Vec<String> {
        let tree = app.tree();
        // The window's height (the root's box): what lies wholly above or
        // below it isn't seen, and moves no one sees.
        let window = tree.lines().next().and_then(top_and_height).map_or(f32::MAX, |(_, h)| h);
        let mut clock = None;
        tree.lines()
            // (The elapsed digits split into glyphs while a second rolls, on the clock.)
            .filter(|line| {
                let depth = line.len() - line.trim_start().len();
                if clock.is_some_and(|at| depth > at) {
                    return false;
                }
                clock = line.contains("#working-clock").then_some(depth);
                clock.is_none()
            })
            // (The transcript's scrollbar shows only while it scrolls, and
            // goes once the view rests, on the clock: the frames here don't
            // run that clock, so it is still out from opening the session.)
            .filter(|line| !line.contains("#transcript-rail"))
            .filter(|line| top_and_height(line).is_none_or(|(y, h)| y + h > 0. && y < window))
            .map(|line| {
                let line = line.trim_start();
                match line.split_once(' ') {
                    Some((id, rest)) if id.starts_with('@') => rest.to_owned(),
                    _ => line.to_owned(),
                }
            })
            .collect()
    }

    /// What moved between two pictures of the interface, line by line.
    /// Two tree lines the same but for rounding: the tree prints boxes in
    /// whole points, so a node a hair either side of a half point (a pill
    /// at 156.9993 and at 157.0, its centred words at 163.4993 and 163.5)
    /// prints a point apart without moving.
    fn same_but_rounding(a: &str, b: &str) -> bool {
        let split = |s: &str| -> (String, Vec<i64>) {
            let mut shape = String::new();
            let mut numbers = Vec::new();
            let mut digits = String::new();
            for c in s.chars().chain(std::iter::once(' ')) {
                if c.is_ascii_digit() || (c == '-' && digits.is_empty()) {
                    digits.push(c);
                    continue;
                }
                if let Ok(n) = digits.parse::<i64>() {
                    numbers.push(n);
                    shape.push('#');
                } else {
                    shape.push_str(&digits);
                }
                digits.clear();
                shape.push(c);
            }
            (shape, numbers)
        };
        let ((sa, na), (sb, nb)) = (split(a), split(b));
        sa == sb && na.len() == nb.len() && na.iter().zip(&nb).all(|(x, y)| (x - y).abs() <= 1)
    }

    fn moved(before: &[String], after: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for i in 0..before.len().max(after.len()) {
            let (a, b) = (before.get(i), after.get(i));
            let same = match (a, b) {
                (Some(a), Some(b)) => same_but_rounding(a, b),
                _ => false,
            };
            if !same {
                out.push(format!("{a:?} -> {b:?}"));
            }
        }
        out
    }

    /// A save reloads the scripts, which rebuilds the interface from kept
    /// state: nothing should move, rise or fade for it, on a new chat or a
    /// session.
    #[test]
    fn a_reload_moves_nothing() {
        let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/chat_fixture.luau");
        let mut app = zgx_script::testing::TestApp::new(|runtime| ScriptHost::new(runtime, entry));
        // A new chat; a session; its composer's queue tray; an approval in
        // the composer; a reply streaming with its group open and
        // the working line; a reply's groups; long folded messages; a view
        // scrolled up, the way back to the latest out.
        let scenes: [(&str, &[&str]); 9] = [
            ("home", &["go_home"]),
            ("session", &["go_s1"]),
            ("queue tray", &["go_s2", "queue_one"]),
            ("approval", &["go_s2", "ask_approval"]),
            ("streaming", &["go_s1", "start_reply", "add_tool", "stream_words"]),
            ("groups", &["go_s5"]),
            ("long messages", &["go_s4"]),
            ("scrolled up", &["go_s1"]),
            // Replies far taller than a row's estimate: the view put back
            // among rows not yet measured, a reply's lane out under the
            // pointer.
            ("scrolled up among tall replies", &["go_s7"]),
        ];
        for (route, commands) in scenes {
            for command in commands {
                app.host.run_command(command);
                app.frames(10);
            }
            if route == "scrolled up" {
                app.scroll("#transcript", -600.);
            }
            if route == "scrolled up among tall replies" {
                // Far up, several replies' worth, then a reply hovered.
                for _ in 0..6 {
                    app.scroll("#transcript", -400.);
                    app.frames(5);
                }
                app.frames(30);
                app.hover("#transcript");
            }
            app.frames(60);
            // (What settles on the clock, a folded group's rows let go, a
            // scroll's rest: let it, as a person waits before a save.)
            // Twice: what the first wait's timers start (the way back to
            // the latest rising in once the view is away) runs its course.
            for _ in 0..2 {
                std::thread::sleep(std::time::Duration::from_millis(1000));
                app.frames(60);
            }
            let settled = boxes(&mut app);
            let _ = app.host.reload();
            let mut changes = Vec::new();
            for frame in 1..=40 {
                app.frame();
                let now = boxes(&mut app);
                let diff = moved(&settled, &now);
                if !diff.is_empty() {
                    changes.push(format!("{route}, frame {frame}: {}", diff.join("; ")));
                }
            }
            assert!(changes.is_empty(), "a reload moved things:\n{}", changes[..changes.len().min(6)].join("\n"));
        }
    }

    #[test]
    fn a_double_click_on_the_words_still_selects_them() {
        let mut app = activity_app();
        app.double_click("~\"Some reply text\"");
        app.frames(5);
        assert!(app.ui.has_text_selection(), "the reply's words should stay selectable");
    }

    #[test]
    fn review_comments_part_from_the_prompt_as_one_chip() {
        let text = "Fix these\n\nReview comments (each cites the workspace file and line it belongs to):\n- src/app.rs:12: Rename this.\n  It reads badly.\n- docs/a: b.md:3: Typo: here";
        let (words, comments) = split_comments(text);
        assert_eq!(words, "Fix these");
        assert_eq!(
            comments,
            vec![
                ("src/app.rs:12".to_owned(), "Rename this.\nIt reads badly.".to_owned()),
                ("docs/a: b.md:3".to_owned(), "Typo: here".to_owned()),
            ]
        );
        let chip = comments_chip(&comments);
        assert!(matches!(chip.field("name"), Some(Value::Str(s)) if s.as_ref() == "2 comments"));
        assert!(matches!(chip.field("meta"), Some(Value::Str(s)) if s.as_ref() == "src/app.rs:12, docs/a: b.md:3"));
        // The header mid-prompt, not a trailing block: left alone.
        let plain = "See Review comments (each cites the workspace file and line it belongs to): later";
        assert_eq!(split_comments(plain), (plain.to_owned(), Vec::new()));
    }

    #[test]
    fn comments_on_a_diff_part_from_the_prompt_with_their_sides_and_ranges() {
        // As screens/chat/comments writes them (production's
        // COMMENT_BLOCK_HEADER): a side each, a range, the code fenced under.
        let text = "Address the review comments below.\n\nComments on the diff (each cites the file and line it belongs to; L = line number in the original file, R = in the changed file):\n- src/a.rs:10-12 (R): Why: this?\n  ```\n  let x = 1;\n  ```\n- src/a.rs:4 (L): Gone.";
        let (words, comments) = split_comments(text);
        assert_eq!(words, "Address the review comments below.");
        assert_eq!(
            comments,
            vec![("src/a.rs:10-12".to_owned(), "Why: this?\n```\nlet x = 1;\n```".to_owned()), ("src/a.rs:4".to_owned(), "Gone.".to_owned())]
        );
    }

    #[test]
    fn appends_change_only_what_shows() {
        let mut entries = Vec::new();
        let mut reset = json!({ "reset": [
            { "id": "a", "role": "assistant", "status": "streaming", "parts": [
                { "kind": "tool", "id": "k", "text": "out" },
                { "kind": "reasoning", "id": "r", "text": "" },
                { "kind": "text", "id": "t", "text": "Hi" },
            ] },
            { "id": "b", "role": "assistant", "status": "complete", "parts": [{ "kind": "text", "id": "t", "text": "Done" }] },
        ] });
        apply(&mut entries, &mut reset).unwrap();
        let frame = |entry: &str, part: &str, text: &str, len: usize| {
            json!({ "upsert": [], "remove": [], "append": [{ "entry": entry, "part": part, "text": text, "len": len }], "count": 2 })
        };
        // More of a tool's output: nothing shows it.
        assert!(matches!(apply(&mut entries, &mut frame("a", "k", "put", 6)), Ok(Change::Quiet)));
        assert_eq!(entries[0]["parts"][0]["text"], "output");
        // Thinking's first words make its row; more of them are the draft's.
        assert!(matches!(apply(&mut entries, &mut frame("a", "r", "Hmm", 3)), Ok(Change::Rows)));
        assert!(matches!(apply(&mut entries, &mut frame("a", "r", "…", 6)), Ok(Change::Words)));
        // Words of the reply being written: the draft; of a finished one,
        // its row.
        assert!(matches!(apply(&mut entries, &mut frame("a", "t", " there", 8)), Ok(Change::Words)));
        assert!(matches!(apply(&mut entries, &mut frame("b", "t", "!", 5)), Ok(Change::Rows)));
        // Out of step (a length that doesn't match): resubscribe.
        assert!(apply(&mut entries, &mut frame("a", "t", "x", 99)).is_err());
    }

    #[test]
    fn the_draft_carries_the_thought_a_reply_is_at() {
        let draft = || draft_value("a", "");
        let thought = json!({ "kind": "reasoning", "id": "r", "text": "  Weighing it.\n" });
        let text = json!({ "kind": "text", "id": "t", "text": "Hi" });
        // At a thought: its id, and its words as its row shows them.
        let parts = vec![text.clone(), thought.clone()];
        let at_it = with_thought(draft(), Some(&parts));
        assert_eq!(text_of(&at_it, "thought"), "r");
        assert_eq!(text_of(&at_it, "thought_text"), "Weighing it.");
        // Past it (the reply's text or a tool has begun): its row has its words.
        let parts = vec![thought, text];
        assert!(with_thought(draft(), Some(&parts)).field("thought").is_none());
        assert!(with_thought(draft(), None).field("thought").is_none());
    }

    #[test]
    fn a_frame_grows_a_reply_by_its_words_alone() {
        let mut entries = Vec::new();
        let mut reset = json!({ "reset": [
            { "id": "u", "role": "user", "parts": [{ "kind": "text", "id": "t0", "text": "hi" }] },
            { "id": "a", "role": "assistant", "status": "streaming", "parts": [{ "kind": "text", "id": "t0", "text": "Hel" }] },
        ] });
        assert!(matches!(apply(&mut entries, &mut reset), Ok(Change::Rows)));
        let mut words = json!({ "upsert": [], "remove": [], "append": [{ "entry": "a", "part": "t0", "text": "lo", "len": 5 }], "count": 2 });
        assert!(matches!(apply(&mut entries, &mut words), Ok(Change::Words)));
        assert_eq!(entry_text(entries[1].json.as_ref().unwrap()), "Hello");
        // Out of step: a length that doesn't add up.
        let mut wrong = json!({ "upsert": [], "remove": [], "append": [{ "entry": "a", "part": "t0", "text": "!", "len": 9 }], "count": 2 });
        assert!(apply(&mut entries, &mut wrong).is_err());
    }

    #[test]
    fn a_system_entry_reaches_the_transcript_as_a_typed_line() {
        let entry = |parts: Json| json!({ "id": "s", "role": "system", "parts": parts });
        let text = |id: &str, words: &str| json!({ "kind": "text", "id": id, "text": words });
        let field = |m: &Value, k: &str| m.field(k).map(crate::auth::text).unwrap_or_default();
        let classified = message(&entry(json!([text("t0", "Classifier: same task"), { "kind": "classifier", "id": "c", "record": { "verdict": "continue" } }])), None).unwrap();
        assert_eq!((field(&classified, "role").as_str(), field(&classified, "line").as_str()), ("system", "classifier"));
        let compacted = message(&entry(json!([text("t0", "Compacted · 212.0k → 40.0k tokens"), { "kind": "compaction", "id": "c", "record": { "preTokens": 212000 } }])), None).unwrap();
        assert_eq!((field(&compacted, "line").as_str(), field(&compacted, "text").as_str()), ("compaction", "Compacted · 212.0k → 40.0k tokens"));
        let fork = message(&entry(json!([{ "kind": "fork", "id": "f", "sourceChatId": "c1", "sourceTitle": "Old\nchat" }])), None).unwrap();
        assert_eq!((field(&fork, "line").as_str(), field(&fork, "title").as_str()), ("fork", "Old chat"));
        let continued = message(&entry(json!([{ "kind": "handoff", "id": "h", "harness": "codex", "text": "packet", "route": { "fromHarness": "codex", "sourceChatId": "c1", "sourceTitle": "Old" } }])), None).unwrap();
        assert_eq!((field(&continued, "line").as_str(), field(&continued, "title").as_str(), field(&continued, "text").as_str()), ("handoff", "Old", ""));
        let card = message(&entry(json!([text("t0", "Claude Code /context"), text("t1", r#"{"command":"/context","text":"12%"}"#)])), None).unwrap();
        assert_eq!((field(&card, "line").as_str(), field(&card, "hint").as_str()), ("claude_command", ""));
        let summary = format!("{}model opus", zeron_proto::claude_panel::SUMMARY_PREFIX);
        assert_eq!(field(&message(&entry(json!([text("t0", &summary)])), None).unwrap(), "line"), "claude_config");
        let note = message(&entry(json!([text("t0", "Resumed"), text("t1", "Why it did")])), None).unwrap();
        assert_eq!((field(&note, "line").as_str(), field(&note, "hint").as_str()), ("divider", "Why it did"));
        let revoked = message(&entry(json!([text("t0", zeron_proto::computer::cards::COMPUTER_SETUP_NOTE)])), None).unwrap();
        assert_eq!(field(&revoked, "line"), "computer_notice");
        let disclosure = entry(json!([text("t0", zeron_proto::computer::cards::COMPUTER_CONTROL_ON_DISCLOSURE)]));
        assert!(message(&disclosure, None).is_none(), "the disclosure has no row, as in the shipping app");
        assert!(message(&entry(json!([])), None).is_none());
        let seam = entry(json!([{ "kind": "switch", "id": "x", "from": "A", "to": "B" }]));
        assert_eq!(field(&message(&seam, None).unwrap(), "role"), "switch", "a seam is still a seam");
        let handed = |m: &Value| matches!(m.field("handoff"), Some(Value::Bool(true)));
        assert!(!handed(&message(&seam, None).unwrap()), "a bare seam holds nothing to read again");
        let moved = message(&entry(json!([{ "kind": "handoff", "id": "h", "harness": "codex", "text": "packet", "route": { "fromHarness": "claude-code", "fromModel": "opus" } }])), None).unwrap();
        assert_eq!(field(&moved, "role"), "switch");
        assert!(handed(&moved), "a provider switch's handoff is history, as the shipping app's chat_has_history counts it");
    }

    #[test]
    fn a_compaction_s_rows_carry_their_records_and_fold_into_the_decision_before_them() {
        let entry = |id: &str, role: &str, parts: Json| json!({ "id": id, "role": role, "parts": parts });
        let text = |words: &str| json!({ "kind": "text", "id": "t0", "text": words });
        let field = |m: &Value, k: &str| m.field(k).map(crate::auth::text).unwrap_or_default();
        let num = |m: &Value, k: &str| m.field(k).and_then(Value::as_f64);
        let decided = entry("k", "system", json!([text("Classifier"), { "kind": "classifier", "id": "c", "record": {
            "harness": "claude-code", "model": "claude-sonnet-5-5", "verdict": "carry", "classifierVerdict": "continue", "decidedBy": "stage2",
            "durationMs": 1670, "contextTokens": 212000, "input": "the turns", "reply": "CONTINUE", "stage2Reply": "CARRY",
            "inputTokens": 2300, "outputTokens": 4, "reason": "Same task, the last turns matter" } }]));
        let carried = "<carried_over>\nThe last turns.\n\nperson: a\nagent: b\n\nperson: c\nagent: d\n\nStill running when the chat compacted:\n- shell\n</carried_over>";
        let compacted = entry("x", "system", json!([text("Context compacted"), { "kind": "compaction", "id": "p", "record": {
            "preTokens": 212000, "postTokens": 69000, "durationMs": 53000, "summary": "What was done", "carried": carried } }]));
        let row = message(&decided, None).unwrap();
        let d = row.field("decision").unwrap();
        assert_eq!((field(d, "verdict").as_str(), field(d, "said").as_str(), field(d, "by").as_str()), ("carry", "continue", "stage2"));
        assert_eq!((num(d, "ms"), num(d, "context"), num(d, "tokens_in")), (Some(1670.), Some(212000.), Some(2300.)));
        assert_eq!((field(d, "stage2").as_str(), field(d, "agent").as_str(), field(d, "mark").as_str()), ("CARRY", "Claude Code", "claude"));
        let unknown = message(&entry("k2", "system", json!([{ "kind": "classifier", "id": "c", "record": { "verdict": "maybe", "decidedBy": "later" } }])), None).unwrap();
        let u = unknown.field("decision").unwrap();
        assert_eq!((field(u, "verdict").as_str(), field(u, "by").as_str()), ("failed", "stage1"), "an unknown verdict reads as no answer, as serde(other)");
        let kept = message(&compacted, None).unwrap();
        let k = kept.field("kept").unwrap();
        assert_eq!((num(k, "pre"), num(k, "post"), num(k, "ms"), num(k, "turns")), (Some(212000.), Some(69000.), Some(53000.), Some(2.)));
        assert!(field(k, "carried").starts_with("person: a"), "the block's own heading and tags go: {}", field(k, "carried"));

        let user = message(&entry("u", "user", json!([text("next")])), None).unwrap();
        let mut list = vec![row.clone(), user.clone(), kept.clone(), row.clone(), message(&entry("d", "system", json!([text("Host restarted")])), None).unwrap(), kept.clone()];
        merge_compaction_rows(&mut list);
        let lines: Vec<String> = list.iter().map(|m| format!("{}:{}", field(m, "role"), field(m, "line"))).collect();
        assert_eq!(lines, ["system:classifier", "user:", "system:classifier", "system:divider", "system:compaction"], "a message sent meanwhile stays; a divider between keeps them apart");
        assert_eq!(list[0].field("kept").and_then(|k| num(k, "post")), Some(69000.));
        assert!(matches!(list[2].field("kept"), None | Some(Value::Nil)));
    }

    #[test]
    fn a_chat_says_which_compaction_it_waits_on_and_when_it_offers_to_resume_with_less() {
        let chats = Chats::default();
        chats.chats.borrow_mut().insert("c".to_owned(), json!({ "id": "c", "config": { "harness": "claude-code" }, "lastMessageAt": utc_stamp(now_seconds() - 145 * 60) }));
        let session = |extra: Json| {
            let mut s = json!({ "chatId": "c", "status": "idle", "updatedAt": utc_stamp(now_seconds()) });
            s.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            chats.sessions.borrow_mut().insert("c".to_owned(), s);
        };
        let decided = |verdict: &str| Entry::new(json!({ "id": "k", "role": "system", "parts": [{ "kind": "classifier", "id": "c", "record": { "verdict": verdict, "contextTokens": 212000 } }] }));
        let done = Entry::new(json!({ "id": "x", "role": "system", "parts": [{ "kind": "compaction", "id": "p", "record": { "durationMs": 52600 } }] }));
        let user = Entry::new(json!({ "id": "u", "role": "user", "parts": [] }));
        let open = Open { id: "c".to_owned(), entries: vec![done, decided("carry"), user], context_usage: (Some(184_312.), Some(1_000_000.)), ..Open::default() };
        let wait = |open: &Open| compaction_wait(&chats, open);
        let field = |m: &Value, k: &str| m.field(k).map(crate::auth::text).unwrap_or_default();

        session(json!({}));
        assert!(matches!(wait(&open), Value::Nil), "nothing planned, nothing waits");
        session(json!({ "compactPlan": { "kind": "agent" } }));
        assert_eq!(field(&wait(&open), "phase"), "queued", "the agent asked: queued until its turn settles");
        session(json!({ "compactPlan": { "kind": "agent" }, "status": "working" }));
        assert!(matches!(wait(&open), Value::Nil), "still its turn: the trailer stays");
        session(json!({ "compactPlan": { "kind": "agent" }, "classifying": { "harness": "claude-code", "model": "claude-sonnet-5-5" } }));
        let checking = wait(&open);
        assert_eq!((field(&checking, "phase").as_str(), field(&checking, "agent").as_str(), field(&checking, "model").as_str()), ("checking", "Claude Code", "claude-sonnet-5-5"));
        assert!(matches!(checking.field("agent_note"), Some(Value::Bool(true))));
        session(json!({ "compacting": true }));
        let compacting = wait(&open);
        assert_eq!((field(&compacting, "phase").as_str(), field(&compacting, "tag").as_str(), field(&compacting, "row").as_str()), ("compacting", "carry", "k"));
        assert_eq!(compacting.field("tokens").and_then(Value::as_f64), Some(212000.));
        assert_eq!(compacting.field("usual").and_then(Value::as_f64), Some(53.), "the median of what compactions here took");
        let kept = Open { entries: vec![decided("continue")], ..Open { id: "c".to_owned(), ..Open::default() } };
        let plain = wait(&kept);
        assert_eq!(field(&plain, "phase"), "compacting");
        assert!(plain.field("tag").is_none() && plain.field("row").is_none(), "a same-task decision is no boundary");

        session(json!({ "compactPlan": { "kind": "copy", "since": 1_700_000_000_000i64, "ready": true } }));
        let copy = compact_plan(&chats, &open);
        assert_eq!(field(&copy, "heading"), "Background compaction");
        assert_eq!(copy.field("copy").and_then(|c| c.field("since")).and_then(Value::as_f64), Some(1_700_000_000.));
        session(json!({ "compacting": true, "compactPlan": { "kind": "agent" } }));
        let busy = compact_plan(&chats, &open);
        assert_eq!(field(&busy, "heading"), "Compacting");
        assert!(matches!(busy.field("busy"), Some(Value::Bool(true))));
        session(json!({}));
        assert!(matches!(compact_plan(&chats, &open), Value::Nil), "nothing planned: no dot, no section");
        let blocked = compact_blocked(&chats, &open, "claude-code");
        let chat = chats.chat("c").unwrap();
        let offer = resume_offer(&open, &chat, "claude-code", &blocked);
        assert_eq!((field(&offer, "tokens").as_str(), field(&offer, "idle").as_str()), ("184K", "2h 25m"));
        assert!(matches!(resume_offer(&open, &chat, "codex", &blocked), Value::Nil), "Claude's prompt cache only");
        let small = Open { context_usage: (Some(99_999.), None), ..Open { id: "c".to_owned(), ..Open::default() } };
        assert!(matches!(resume_offer(&small, &chat, "claude-code", &blocked), Value::Nil), "under 100K");
        session(json!({ "compacting": true }));
        let busy = compact_blocked(&chats, &open, "claude-code");
        assert_eq!(crate::auth::text(&busy), "compacting");
        assert!(matches!(resume_offer(&open, &chat, "claude-code", &busy), Value::Nil), "never while compacting");
        chats.chats.borrow_mut().insert("c".to_owned(), json!({ "id": "c", "config": { "harness": "claude-code" }, "lastMessageAt": utc_stamp(now_seconds() - 69 * 60) }));
        assert!(matches!(resume_offer(&open, &chats.chat("c").unwrap(), "claude-code", &Value::Nil), Value::Nil), "the cache lasts 70 minutes");
        assert_eq!(utc_stamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_stamp(1_709_251_199), "2024-02-29T23:59:59Z");
    }

    #[test]
    fn a_context_answer_becomes_the_card_s_bar_and_legend() {
        let text = "## Context Usage\n\n**Model:** claude-opus-5-5  \n**Tokens:** 61.2k / 200k (31%)\n\n### Estimated usage by category\n\n| Category | Tokens | Percentage |\n|----------|--------|------------|\n| System prompt | 3.1k | 1.6% |\n| Messages | 40k | 20% |\n| Free space | 106k | 53% |\n| Autocompact buffer | 33k | 16.5% |\n";
        let note = json!({ "command": "/context", "text": text }).to_string();
        let entry = json!({ "id": "s", "role": "system", "parts": [{ "kind": "text", "id": "t0", "text": "Claude Code /context" }, { "kind": "text", "id": "t1", "text": note }] });
        let row = message(&entry, None).unwrap();
        let report = row.field("context").expect("a /context card");
        let names: Vec<String> = report.field("parts").unwrap().list().iter().map(|p| crate::auth::text(p.field("name").unwrap())).collect();
        assert_eq!(names, ["System prompt", "Messages", "Autocompact buffer"]);
        assert_eq!(crate::auth::text(report.field("free").unwrap().field("tokens").unwrap()), "106K");
        let other = json!({ "id": "s", "role": "system", "parts": [{ "kind": "text", "id": "t0", "text": "Claude Code /cost" }, { "kind": "text", "id": "t1", "text": json!({ "command": "/cost", "text": "$1" }).to_string() }] });
        assert!(matches!(message(&other, None).unwrap().field("context"), None | Some(Value::Nil)));
    }

    #[test]
    fn a_reply_s_question_and_denied_call_are_lines_and_a_secret_stays_out() {
        let reply = |parts: Json| json!({ "id": "a", "role": "assistant", "status": "complete", "parts": parts });
        let asked = |kind: Json, resolved: bool, labels: Json| {
            json!({ "kind": "input", "id": "i", "requestId": "r", "resolved": resolved,
                "questions": [{ "id": "q", "header": "Pick", "question": "Which?", "options": ["A"], "kind": kind }],
                "answers": [{ "questionId": "q", "labels": labels }] })
        };
        let lines = |part: Json| blocks(&reply(json!([part]))).list().to_vec();
        let answered = lines(asked(Json::Null, true, json!(["A"])));
        assert_eq!(answered[0].field("line"), Some(&Value::from("input")));
        assert_eq!(answered[0].field("text"), Some(&Value::from("Pick → A")));
        let secret = lines(asked(json!({ "type": "secret" }), true, json!(["hunter2-secret"])));
        assert_eq!(secret[0].field("line"), Some(&Value::from("secret")));
        assert!(!format!("{:?}", secret[0]).contains("hunter2"));
        let denied = |id: &str| json!({ "kind": "tool", "id": id, "resolved": true, "isError": true, "output": "capability_denied",
            "call": { "kind": "mcp", "server": "zeron", "tool": "computer_click", "input": {} } });
        let shown = blocks(&reply(json!([denied("c1"), denied("c2")])));
        let kinds: Vec<String> = shown.list().iter().map(|b| crate::auth::text(b.field("kind").unwrap())).collect();
        assert_eq!(kinds, ["tools", "line", "tools"], "one notice for the tool, however often it's turned away");
        for dropped in ["handoff", "classifier", "compaction"] {
            assert!(lines(json!({ "kind": dropped, "id": "p" })).is_empty(), "{dropped}");
        }
    }

    #[test]
    fn a_computer_ask_is_its_card_or_what_it_settled_to_and_never_the_composer_s() {
        use zeron_proto::computer::cards::*;
        let reply = |parts: Json| json!({ "id": "a", "role": "assistant", "status": "streaming", "parts": parts });
        let ask = |kind: Json, question: &str, resolved: bool, labels: Option<Json>| {
            let mut part = json!({ "kind": "input", "id": "i", "requestId": "r", "resolved": resolved,
                "questions": [{ "id": "q", "header": "Computer", "question": question, "options": [], "kind": kind }] });
            if let Some(labels) = labels {
                part["answers"] = json!([{ "questionId": "q", "labels": labels }]);
            }
            part
        };
        let one = |part: Json| blocks(&reply(json!([part]))).list().first().cloned();
        let f = |v: &Value, k: &str| v.field(k).map(text).unwrap_or_default();
        let keys = |v: &Value| -> Vec<String> { v.field("choices").unwrap().list().iter().map(|c| f(c, "key")).collect() };
        let ghostty = json!({ "type": "consent", "scope": "task", "app": { "name": "Ghostty", "bundleId": "com.mitchellh.ghostty" } });
        let card = one(ask(ghostty.clone(), "Use Ghostty? It runs commands.", false, None)).unwrap();
        assert_eq!((f(&card, "line"), f(&card, "title"), f(&card, "detail")), ("computer_consent".into(), "Allow Zeron to use Ghostty?".into(), "It runs commands.".into()));
        assert_eq!(f(&card, "risk"), "Zeron could run commands that change files and settings on this Mac.");
        assert_eq!(keys(&card), ["deny", "always", "allow"], "gpui's order, Allow last");
        let allow = &card.field("choices").unwrap().list()[2];
        assert_eq!((f(allow, "label"), f(allow, "answer"), f(allow, "settled")), ("Allow this conversation".into(), "Allow".into(), "Allowed Ghostty for this task".into()));
        assert!(matches!(allow.field("primary"), Some(Value::Bool(true))));
        assert_eq!(f(&card.field("choices").unwrap().list()[1], "answer"), "Always allow Ghostty");
        assert!(matches!(card.field("outcome"), Some(Value::Nil) | None));
        let unverified = one(ask(json!({ "type": "consent", "scope": "task", "app": { "name": "Notes" } }), "Use Notes?", false, None)).unwrap();
        assert_eq!(keys(&unverified), ["deny", "allow"], "Always only for a verified bundle");
        assert!(matches!(unverified.field("risk"), Some(Value::Nil) | None));
        let call = one(ask(json!({ "type": "consent", "scope": "call", "tool": "computer_click" }), "Click?", false, None)).unwrap();
        assert_eq!((f(&call, "title"), f(&call.field("choices").unwrap().list()[1], "label")), ("Approve this tool call?".into(), "Allow once".into()));
        let allowed = one(ask(ghostty.clone(), "Use Ghostty?", true, Some(json!(["Allow"])))).unwrap();
        assert_eq!((f(&allowed, "outcome"), f(&allowed, "text")), ("allowed".into(), "Allowed Ghostty for this task".into()));
        let always = one(ask(ghostty.clone(), "Use Ghostty?", true, Some(json!(["Always allow Ghostty"])))).unwrap();
        assert_eq!(f(&always, "text"), "Always allowed Ghostty");
        let cancelled = one(ask(ghostty, "Use Ghostty?", true, None)).unwrap();
        assert_eq!(f(&cancelled, "text"), "Cancelled");

        let safari = json!({ "type": "foreground", "app": { "name": "Safari" } });
        let front = one(ask(safari.clone(), "Front?", false, None)).unwrap();
        assert_eq!((f(&front, "line"), f(&front, "title"), f(&front, "detail")), ("computer_foreground".into(), "Bring Safari to the front?".into(), COMPUTER_FOREGROUND_DETAIL.into()));
        assert_eq!(keys(&front), ["no", "allow"]);
        let kept = one(ask(safari.clone(), "Front?", true, Some(json!(["No"])))).unwrap();
        assert_eq!((f(&kept, "line"), f(&kept, "notice"), f(&kept, "text")), ("computer_notice".into(), "foreground".into(), "You kept Safari in the background".into()));
        assert!(one(ask(safari.clone(), "Front?", true, None)).is_none(), "settled unanswered: no row, as gpui's");

        let session = |part: Json| json!({ "id": "a", "role": "assistant", "status": "streaming", "parts": [part] });
        assert!(ask_value(&session(ask(safari, "Front?", false, None))["parts"][0], 0).is_none(), "the composer doesn't ask it again");
        assert!(ask_value(&session(ask(Json::Null, "Pick?", false, None))["parts"][0], 0).is_some());

        let entry = |words: &[&str]| json!({ "id": "s", "role": "system", "parts": words.iter().enumerate().map(|(i, w)| json!({ "kind": "text", "id": format!("t{i}"), "text": w })).collect::<Vec<_>>() });
        let note = |words: &[&str]| message(&entry(words), None).unwrap();
        let esc = note(&[COMPUTER_ESCAPE_STOP_NOTICE]);
        assert_eq!((f(&esc, "notice"), f(&esc, "text")), ("revoked".into(), "Desktop access revoked by you (Esc)".into()));
        let left = note(&["Left 2 Ghostty windows open"]);
        assert_eq!((f(&left, "notice"), f(&left, "app")), ("left_open".into(), "Ghostty".into()));
        assert_eq!(f(&note(&["Paused while you used Safari"]), "notice"), "paused");
        let away = note(&[COMPUTER_UNAVAILABLE_NOTE, "The desktop host is offline."]);
        assert_eq!(f(&away, "notice"), "unavailable");
        assert_eq!(away.field("lines").unwrap().list().iter().map(text).collect::<Vec<_>>(), ["The desktop host is offline."]);
        assert_eq!(f(&note(&[COMPUTER_SETUP_NOTE]), "notice"), "setup");
        let denied = json!({ "kind": "tool", "id": "c1", "resolved": true, "isError": true, "output": "capability_denied",
            "call": { "kind": "mcp", "server": "zeron", "tool": "computer_click", "input": {} } });
        let line = blocks(&reply(json!([denied]))).list().iter().find(|b| f(b, "kind") == "line").cloned().unwrap();
        assert_eq!((f(&line, "notice"), f(&line, "action")), ("control_denied".into(), COMPUTER_DENIED_ACTION.into()));
        assert_eq!(line.field("lines").unwrap().list().iter().map(text).collect::<Vec<_>>(), [COMPUTER_DENIED_HINT]);

        for ok in ["Allow", "Deny", "Always allow Ghostty", "Allow for this turn", "No"] {
            assert!(computer_answer(ok), "{ok}");
        }
        for refused in ["Yes", "Always allow ", "", "allow"] {
            assert!(!computer_answer(refused), "{refused:?}");
        }
    }

    #[test]
    fn tool_rows_by_kind_question_secret_and_claude_rows_carry_what_gpui_draws() {
        let call = |id: &str, call: Json| json!({ "kind": "tool", "id": id, "resolved": true, "call": call });
        let row = |c: Json| tool(&c).unwrap();
        let f = |v: &Value, k: &str| v.field(k).map(text).unwrap_or_default();
        let mcp = row(call("m", json!({ "kind": "mcp", "server": "zeron", "tool": "device", "input": { "action": "screenshot" } })));
        assert_eq!((f(&mcp, "kind"), f(&mcp, "target")), ("mcp".into(), "zeron.device_screenshot".into()));
        let other = row(call("o", json!({ "kind": "mcp", "server": "github", "tool": "device", "input": { "action": "screenshot" } })));
        assert_eq!(f(&other, "target"), "github.device", "only zeron's own tools take an action's label");
        let wait = row(call("w", json!({ "kind": "unknown", "name": "Wait for agents" })));
        assert_eq!((f(&wait, "kind"), f(&wait, "verb")), ("wait".into(), "Waited for agents".into()));
        let skill = row(call("s", json!({ "kind": "unknown", "name": "Skill", "input": { "skill": "zgx-port", "path": "/w/skills/zgx-port/SKILL.md" } })));
        assert_eq!((f(&skill, "kind"), f(&skill, "target"), f(&skill, "path")), ("skill".into(), "zgx-port".into(), "/w/skills/zgx-port/SKILL.md".into()));
        let todo = row(call("t", json!({ "kind": "todo", "items": [{ "text": "a", "done": true }, { "text": "b", "status": "pending" }, { "text": "c", "status": "completed" }] })));
        assert_eq!(f(&todo, "target"), "2/3 done");
        let patch = row(call("p", json!({ "kind": "applyPatch" })));
        assert_eq!((f(&patch, "verb"), f(&patch, "target")), ("Patched".into(), "workspace".into()));
        let read = row(call("r", json!({ "kind": "readFile", "path": "/w/src/parse.rs" })));
        assert_eq!((f(&read, "target"), f(&read, "path")), ("parse.rs".into(), "/w/src/parse.rs".into()));
        let reply = |parts: Json| json!({ "id": "a", "role": "assistant", "status": "complete", "parts": parts });
        let q = |id: &str, header: &str| json!({ "id": id, "header": header, "question": "Which?", "options": ["A", "B"] });
        let two = json!({ "kind": "input", "id": "i", "requestId": "r", "resolved": true, "questions": [q("a", "Scope"), q("b", "Reviewer")],
            "answers": [{ "questionId": "a", "labels": ["A"] }, { "questionId": "b", "labels": [] }] });
        let b = blocks(&reply(json!([two]))).list().to_vec();
        assert_eq!(f(&b[0], "outcome"), "answered");
        let answers = b[0].field("answers").unwrap().list().to_vec();
        assert_eq!((f(&answers[0], "label"), f(&answers[0], "answer")), ("Scope".into(), "A".into()));
        assert_eq!(answers[1].field("skipped"), Some(&Value::Bool(true)));
        let waiting = json!({ "kind": "input", "id": "i", "requestId": "r", "resolved": false, "questions": [q("a", "Deploy")] });
        assert_eq!(f(&blocks(&reply(json!([waiting]))).list()[0], "outcome"), "pending");
        let secret = |resolved: bool, labels: Json, status: &str| {
            json!({ "id": "a", "role": "assistant", "status": status, "parts": [{ "kind": "input", "id": "x", "requestId": "rq", "resolved": resolved,
                "questions": [{ "id": "q", "header": "Tailscale key", "question": "Create one at https://x.dev.", "options": [], "kind": { "type": "secret", "placeholder": "tskey-…" } }],
                "answers": [{ "questionId": "q", "labels": labels }] }] })
        };
        let pending = blocks(&secret(false, json!([]), "streaming")).list()[0].clone();
        assert_eq!((f(&pending, "status"), f(&pending, "request"), f(&pending, "placeholder")), ("pending".into(), "rq".into(), "tskey-…".into()));
        assert_eq!(f(&pending, "reason"), "Create one at https://x.dev.");
        assert_eq!(f(&blocks(&secret(false, json!([]), "complete")).list()[0], "status"), "cancelled", "a settled reply no longer asks");
        assert_eq!(f(&blocks(&secret(true, json!(["saved"]), "complete")).list()[0], "status"), "saved");
        assert_eq!(f(&blocks(&secret(true, json!(["timed_out"]), "complete")).list()[0], "status"), "timedout");
        let note = |hint: Json| json!({ "id": "s", "role": "system", "parts": [{ "kind": "text", "id": "t0", "text": "Claude Code /status" }, { "kind": "text", "id": "t1", "text": hint.to_string() }] });
        let status = message(&note(json!({ "command": "/status" })), None).unwrap();
        let card = status.field("card").unwrap();
        assert_eq!((f(card, "terminal"), f(card, "action")), ("status".into(), "Open Status".into()));
        let pending = message(&note(json!({ "command": "/doctor", "pending": true, "startedAt": 1_000 })), None).unwrap();
        assert_eq!(pending.field("card").unwrap().field("started").and_then(Value::as_f64), Some(1_000.));
        let model = message(&note(json!({ "command": "/model", "model": "opus", "models": [["default", "Default"], ["opus", "Opus"]] })), None).unwrap();
        let models = model.field("card").unwrap().field("models").unwrap().list().to_vec();
        assert_eq!((models.len(), f(&models[1], "id"), f(model.field("card").unwrap(), "model")), (2, "opus".into(), "opus".into()));
        let failed = blocks(&reply(json!([{ "kind": "error", "id": "e", "message": "Claude Code is not logged in. Please run /login." }]))).list().to_vec();
        assert_eq!((f(&failed[0], "line"), f(&failed[0], "problem")), ("error".into(), "signed_out".into()));
        let plain = blocks(&reply(json!([{ "kind": "error", "id": "e", "message": "exit 101" }]))).list().to_vec();
        assert!(matches!(plain[0].field("problem"), None | Some(Value::Nil)));
        let summary = json!({ "id": "c", "role": "system", "parts": [{ "kind": "text", "id": "t0", "text": "Claude Code settings · Theme → dark, Verbose → on" }] });
        let parts = message(&summary, None).unwrap().field("parts").unwrap().list().to_vec();
        assert_eq!((f(&parts[1], "label"), f(&parts[1], "value")), ("Verbose".into(), "on".into()));
    }

    #[test]
    fn a_transcript_s_subagents_are_listed_once_each_in_order() {
        let tool = |r: &str, title: &str| {
            Value::record([("id", Value::from(r)), ("subagent", Value::record([("ref", Value::from(r)), ("title", Value::from(title)), ("model", Value::from("gpt-5.5"))]))])
        };
        let message = |started: f64, tools: Vec<Value>| {
            Value::record([("started", Value::Num(started)), ("tools", Value::List(tools.into()))])
        };
        let plain = Value::record([("id", Value::from("t")), ("verb", Value::from("Ran"))]);
        let list = [
            message(1., vec![tool("a", "first"), plain]),
            Value::record([("role", Value::from("user"))]),
            message(2., vec![tool("a", "again"), tool("b", "second")]),
        ];
        let mut seen = Subagents::default();
        for message in &list {
            seen.add(subagents_in(message).as_deref());
        }
        let found = seen.0;
        let refs: Vec<String> = found.iter().map(|s| s.field("ref").map(text).unwrap_or_default()).collect();
        assert_eq!(refs, ["a", "b"]);
        assert_eq!(found[0].field("title").map(text).as_deref(), Some("first"));
        assert_eq!(found[1].field("at").and_then(Value::as_f64), Some(2.));
        assert_eq!(found[1].field("model").map(text).as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn a_shell_stops_only_where_engine_and_device_both_can() {
        let chats = Chats::default();
        *chats.here.borrow_mut() = Some("mac".into());
        let here = json!({ "deviceId": "mac" });
        let box_ = json!({ "deviceId": "box" });
        assert!(!chats.shells_stoppable(&here), "an engine that hasn't said");
        *chats.engine_capabilities.borrow_mut() = vec![zeron_proto::capabilities::BACKGROUND_TASKS_V1.to_owned()];
        assert!(chats.shells_stoppable(&here));
        assert!(!chats.shells_stoppable(&box_), "a device not listed as able");
    }

    #[test]
    fn only_the_latest_prompt_s_spawns_are_recent() {
        let user = |id: &str, answers: Option<&str>| json!({ "id": id, "role": "user", "createdAt": 1000., "answersRequestId": answers, "parts": [{ "kind": "text", "text": "go" }] });
        let spawn = |id: &str, doc: &str, status: &str| json!({ "id": id, "role": "assistant", "createdAt": 2000., "parts": [
            { "kind": "tool", "id": format!("t-{doc}"), "subagentRef": doc, "subagentStatus": status, "resolved": status != "running",
              "call": { "kind": "unknown", "name": "Agent: x", "input": { "description": doc } } },
        ] });
        let entries = [
            user("u1", None),
            spawn("a1", "old", "done"),
            user("u2", None),
            spawn("a2", "now", "running"),
            user("u3", Some("r1")),
            spawn("a3", "after-answer", "done"),
        ];
        let mut seen = Subagents::default();
        for row in entries.iter().filter_map(|e| message(e, None)) {
            seen.row(&row);
            seen.add(subagents_in(&row).as_deref());
        }
        let found = seen.rows();
        let recent: Vec<(String, bool)> = found
            .iter()
            .map(|s| (s.field("ref").map(text).unwrap_or_default(), matches!(s.field("recent"), Some(Value::Bool(true)))))
            .collect();
        assert_eq!(recent, [("old".to_owned(), false), ("now".to_owned(), true), ("after-answer".to_owned(), true)]);
    }

    #[test]
    fn a_spawn_names_its_model_kind_and_makers_mark() {
        let call = serde_json::json!({ "kind": "unknown", "name": "Agent: x", "input": { "modelId": " gpt-5.5 ", "subagent_type": "Explore" } });
        assert_eq!(super::subagent_model(&call), Some("gpt-5.5"));
        assert_eq!(super::subagent_kind(&call), Some("Explore"));
        assert_eq!(super::subagent_mark(&call).as_deref(), crate::agents::glyph("codex").as_deref());
        let plain = serde_json::json!({ "kind": "unknown", "name": "Agent: x", "input": { "model": "opus" } });
        assert_eq!(super::subagent_mark(&plain), None);
        assert_eq!(super::subagent_kind(&plain), None);
    }

    #[test]
    fn a_message_from_another_chat_says_who_over_its_words() {
        let id = "cd27c5c2-0000-4000-8000-000000000000";
        let said = |text: String| json!({ "id": "u", "role": "user", "parts": [{ "kind": "text", "id": "t", "text": text }] });
        let m = message(&said(zeron_proto::agent_messages::agent_message("Layout pass (cd27c5c2)", id, "All done.")), None).unwrap();
        let sender = m.field("sender").unwrap();
        let is = |v: Option<&Value>, want: &str| matches!(v, Some(Value::Str(s)) if s.as_ref() == want);
        assert!(is(sender.field("chat"), id));
        assert!(is(sender.field("title"), "Layout pass"));
        // Its words alone, to show and to copy: the header is the agent's.
        assert!(is(m.field("text"), "All done."));
        assert!(is(m.field("copy"), "All done."));
        // A person's that merely starts alike is theirs, as written.
        let typed = "[Message from Zeron chat x]\n\nhi";
        let m = message(&said(typed.to_owned()), None).unwrap();
        assert!(m.field("sender").is_none_or(|s| matches!(s, Value::Nil)));
        assert!(is(m.field("text"), typed));
    }

    #[test]
    fn a_prompts_attached_passages_show_as_chips_on_the_sent_message() {
        let text = "Why this one?\n\nSelected from this conversation:\n- GPT-5.5, Sep 8, 1:43 PM: Layout dominates.\n  Paint is second.\n- You: the first ask\n\nAttached images (local files — open them to view):\n- /tmp/shot.png";
        let entry = json!({ "id": "u", "role": "user", "createdAt": 1_788_900_000_000_i64,
            "parts": [{ "id": "t", "kind": "text", "text": text }] });
        let shown = message(&entry, None).expect("a prompt shows");
        assert!(matches!(shown.field("text"), Some(Value::Str(s)) if s.as_ref() == "Why this one?"));
        let Some(Value::List(chips)) = shown.field("attachments") else { panic!("no attachments") };
        let names: Vec<String> = chips
            .iter()
            .map(|c| match c.field("name") {
                Some(Value::Str(s)) => s.to_string(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(names, vec!["shot.png", "GPT-5.5, Sep 8, 1:43 PM", "You"]);
        assert!(matches!(chips[1].field("meta"), Some(Value::Str(s)) if s.as_ref() == "Layout dominates. Paint is second."));
        // A prompt only quoting (no words of its own), and one quoting the
        // header mid-text, untouched.
        let (words, quotes) = split_quotes("Selected from this conversation:\n- You: hi");
        assert_eq!((words.as_str(), quotes.len()), ("", 1));
        let plain = "See: Selected from this conversation: is a header";
        assert_eq!(split_quotes(plain), (plain.to_owned(), Vec::new()));
    }

    #[test]
    fn zerons_annotation_and_selection_blocks_show_as_its_badges() {
        use zeron_proto::quotes::{Quote, with_quotes};
        let note = |text: &str, comment: &str, source: Option<&str>| Quote { comment: Some(comment.into()), ..Quote::annotation(text, source.map(str::to_owned)) };
        let sent = with_quotes("look", &[note("first\n\nline", "why?", None), note("second", "", Some("docs/plan.md")), Quote::new("plain")]);
        let entry = json!({ "id": "u", "role": "user", "createdAt": 1_788_900_000_000_i64,
            "parts": [{ "id": "t", "kind": "text", "text": sent }] });
        let shown = message(&entry, None).expect("a prompt shows");
        assert!(matches!(shown.field("text"), Some(Value::Str(s)) if s.as_ref() == "look"));
        let Some(Value::List(chips)) = shown.field("attachments") else { panic!("no attachments") };
        let str_of = |v: Option<&Value>| match v {
            Some(Value::Str(s)) => Some(s.to_string()),
            _ => None,
        };
        let kinds: Vec<Option<String>> = chips.iter().map(|c| str_of(c.field("badge"))).collect();
        assert_eq!(kinds, vec![Some("annotations".to_owned()), Some("selections".to_owned())]);
        let Some(Value::List(notes)) = chips[0].field("details") else { panic!("no details") };
        assert_eq!(notes.len(), 2);
        assert_eq!(str_of(notes[0].field("text")).as_deref(), Some("first\n\nline"));
        assert_eq!(str_of(notes[0].field("comment")).as_deref(), Some("why?"));
        assert_eq!(str_of(notes[0].field("path")), None);
        assert_eq!(str_of(notes[1].field("path")).as_deref(), Some("docs/plan.md"));
        assert_eq!(str_of(notes[1].field("comment")), None);
        let Some(Value::List(plain)) = chips[1].field("details") else { panic!("no details") };
        assert_eq!(str_of(plain[0].field("text")).as_deref(), Some("plain"));
        // Sent with no words: Zeron's line in their place, as it shows there.
        let (words, badges) = quote_badges(&with_quotes("", &[note("x", "", None)]));
        assert_eq!((words.as_str(), badges.len()), (zeron_proto::quotes::QUOTE_ONLY_TEXT, 1));
        let typed = format!("{} is a header I typed", zeron_proto::badges::ANNOTATION_BLOCK_HEADER);
        assert_eq!(quote_badges(&typed).1.len(), 0);
    }

    #[test]
    fn a_run_with_another_agent_model_or_effort_changes_the_chat_and_nothing_else_does() {
        let chat = json!({ "harness": "claude-code", "model": "sonnet", "reasoning": "medium", "modelOptions": {}, "sandbox": "workspace-write" });
        // The chat's own config, filled out as a run's is: the same.
        let same = merged_config(&chat, &Value::record([("text", Value::from("hi"))]));
        assert!(!runs_otherwise(&chat, &same));
        // A config with no options or nulls reads as one with empty ones.
        assert!(!runs_otherwise(&json!({ "harness": "claude-code", "model": "sonnet", "reasoning": "medium" }), &same));
        // Another agent's model, or another effort: the chat takes it.
        let other = merged_config(&chat, &Value::record([("model", Value::from("gpt")), ("harness", Value::from("codex"))]));
        assert!(runs_otherwise(&chat, &other));
        assert_eq!(other["harness"], json!("codex"));
        // An instance belongs to the agent it was picked on; a profile stays, drifted.
        let bound = json!({ "harness": "claude-code", "model": "sonnet", "instanceId": "work", "profileId": "lead", "instructions": "Lead", "disabledTools": ["x"] });
        let moved = merged_config(&bound, &Value::record([("harness", Value::from("codex"))]));
        assert!(moved.get("instanceId").is_none());
        assert_eq!((moved["profileId"].as_str(), moved["instructions"].as_str()), (Some("lead"), Some("Lead")));
        assert_eq!(moved["disabledTools"], json!(["x"]));
        let stayed = merged_config(&bound, &Value::record([("model", Value::from("opus"))]));
        assert_eq!(stayed["instanceId"], json!("work"));
        let effort = merged_config(&chat, &Value::record([("effort", Value::from("high"))]));
        assert!(runs_otherwise(&chat, &effort));
        // A model's options (Devin Fusion's lead and sidekick), picked
        // whole: what's picked is the chat's, what isn't is gone; a choice
        // at its default is left out of the picks.
        let chat = json!({ "harness": "devin", "model": "fusion", "reasoning": "high", "modelOptions": { "lead": "a", "sidekick": "x" } });
        let picked = merged_config(&chat, &Value::record([("options", Value::record([("lead", Value::from("b"))]))]));
        assert!(runs_otherwise(&chat, &picked));
        assert_eq!(picked["modelOptions"], json!({ "lead": "b" }));
        let none = merged_config(&chat, &Value::record([("options", Value::record::<&str>([]))]));
        assert_eq!(none["modelOptions"], json!({}));
    }

    #[test]
    fn a_models_options_reach_the_picker_but_fast_is_its_own() {
        let m = json!({
            "id": "fusion", "label": "Fusion", "reasoningLevels": ["low", "high"],
            "options": [
                { "id": "lead", "label": "Lead", "defaultChoice": "a", "choices": [{ "id": "a", "label": "Lead A" }, { "id": "b", "label": "Lead B" }] },
                { "id": "speed", "label": "Fast Mode", "defaultChoice": "standard", "choices": [{ "id": "standard", "label": "Standard" }, { "id": "fast", "label": "Fast" }] },
                { "id": "empty", "label": "Nothing", "defaultChoice": "", "choices": [] }
            ]
        });
        assert_eq!(fast_option(&m), Some(("speed".into(), "fast".into(), "standard".into())));
        let v = model_value("devin", &m);
        assert!(matches!(v.field("fast"), Some(Value::Bool(true))));
        let options = v.field("options").map(Value::list).unwrap_or_default();
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].field("id").map(text).as_deref(), Some("lead"));
        assert_eq!(options[0].field("default").map(text).as_deref(), Some("a"));
        assert_eq!(options[0].field("choices").map(Value::list).map(<[Value]>::len), Some(2));
        // A chat's picks: Fast read as Fast, the rest as they are.
        let config = json!({ "speed": "fast", "lead": "b", "n": 3 });
        assert!(fast_in(&config));
        assert_eq!(options_value(&config), Value::record([("lead", Value::from("b")), ("speed", Value::from("fast"))]));
        // Claude's fastMode, off by default.
        let claude = json!({ "id": "opus", "options": [{ "id": "fastMode", "label": "Fast", "defaultChoice": "off", "choices": [{ "id": "off", "label": "Off" }, { "id": "on", "label": "On" }] }] });
        assert_eq!(fast_option(&claude), Some(("fastMode".into(), "on".into(), "off".into())));
        assert!(model_value("claude-code", &claude).field("options").is_none());
    }

    #[test]
    fn the_picker_has_a_tab_per_agent_and_hears_what_is_new() {
        let chats = Rc::new(Chats::default());
        *chats.harnesses.borrow_mut() = vec![json!({ "id": "codex", "name": "Codex" }), json!({ "id": "fx", "name": "Fx" }), json!({ "id": "pi", "name": "Pi" })];
        let reply = json!({
            "models": [{ "id": "gpt", "label": "GPT", "description": "Frontier" }, { "id": "sol", "label": "GPT-6 Sol", "description": " " }],
            "newIds": ["sol"], "announceIds": ["sol"]
        });
        chats.announce("codex", &reply);
        chats.models.borrow_mut().insert("codex".into(), listed_models(&reply));
        chats.model_errors.borrow_mut().insert("fx".into(), "Not signed in to Fx".into());
        let catalog = chats.catalog_value();
        // A tab for each agent, one with no models yet saying why.
        let tabs = catalog.field("tabs").map(Value::list).unwrap_or_default();
        let state = |tabs: &[Value], i: usize| tabs[i].field("state").map(text);
        assert_eq!(tabs.len(), 3);
        assert_eq!(state(tabs, 0).as_deref(), Some("ready"));
        assert_eq!(state(tabs, 1).as_deref(), Some("error"));
        assert_eq!(tabs[1].field("error").map(text).as_deref(), Some("Not signed in to Fx"));
        assert_eq!(state(tabs, 2).as_deref(), Some("loading"));
        assert_eq!(catalog.field("harnesses").map(Value::list).map(<[Value]>::len), Some(1));
        // Each model's line and whether it's new.
        let models = catalog.field("models").map(Value::list).unwrap_or_default();
        assert_eq!(models[0].field("description").map(text).as_deref(), Some("Frontier"));
        assert!(models[0].field("new").is_none());
        assert!(models[1].field("description").is_none());
        assert!(matches!(models[1].field("new"), Some(Value::Bool(true))));
        // Announced once, in Zeron's words: the agent and the models' names.
        let heard = |chats: &Chats| chats.catalog_value().field("announced").map(Value::list).map(|l| l.to_vec()).unwrap_or_default();
        let announced = heard(&chats);
        assert_eq!(announced.len(), 1);
        assert_eq!(announced[0].field("agent").map(text).as_deref(), Some("Codex"));
        let names: Vec<String> = announced[0].field("models").map(Value::list).unwrap_or_default().iter().map(text).collect();
        assert_eq!(names, ["GPT-6 Sol"]);
        chats.announce("codex", &json!({ "models": [], "newIds": ["sol"], "announceIds": [] }));
        assert_eq!(heard(&chats).len(), 1);
        // All answered: an agent with none is listed, empty.
        chats.catalog_state.set(CatalogState::Ready);
        let done = chats.catalog_value();
        assert_eq!(state(done.field("tabs").map(Value::list).unwrap_or_default(), 2).as_deref(), Some("ready"));
    }

    #[test]
    fn a_tools_line_is_one_line_and_its_whole_call_rides_with_it() {
        let str_of = |v: &Value, k: &str| match v.field(k) {
            Some(Value::Str(s)) => Some(s.to_string()),
            _ => None,
        };
        let part = |call: Json| json!({ "id": "p", "kind": "tool", "call": call });
        // A command of five lines: its line joins them; its call keeps them.
        let script = "echo '--- a'; git grep -n x\necho '--- b'; ls -d crates\necho '--- c'; git branch -a\necho '--- d'; gh pr list\necho '--- e'; gh issue list";
        let ran = tool(&part(json!({ "kind": "exec", "command": script }))).expect("a tool");
        let line = str_of(&ran, "target").expect("a line");
        assert!(!line.contains('\n'));
        assert!(line.starts_with("echo '--- a'; git grep -n x echo '--- b'"));
        assert_eq!(str_of(&ran, "call").as_deref(), Some(script));
        // A command of one short line: nothing more to open to.
        let short = tool(&part(json!({ "kind": "exec", "command": "cargo test" }))).expect("a tool");
        assert_eq!(str_of(&short, "target").as_deref(), Some("cargo test"));
        assert_eq!(str_of(&short, "call"), None);
        // A pattern with a newline and where it looked; an MCP call's input.
        let search = tool(&part(json!({ "kind": "search", "pattern": "fn a\\(\n", "path": "src" }))).expect("a tool");
        assert_eq!(str_of(&search, "target").as_deref(), Some("fn a\\("));
        assert_eq!(str_of(&search, "call").as_deref(), Some("fn a\\(\n in src"));
        let mcp = tool(&part(json!({ "kind": "mcp", "server": "gh", "tool": "issues", "input": { "q": "plugin" } }))).expect("a tool");
        assert_eq!(str_of(&mcp, "target").as_deref(), Some("gh.issues"));
        assert_eq!(str_of(&mcp, "call").as_deref(), Some("gh · issues\n{\n  \"q\": \"plugin\"\n}"));
        // An edit names its file; its strings never reach its line.
        let edit = tool(&part(json!({ "kind": "editFile", "path": "src/a.rs", "old_string": "x\ny", "new_string": "z" }))).expect("a tool");
        assert_eq!(str_of(&edit, "target").as_deref(), Some("a.rs"));
        assert_eq!(str_of(&edit, "call"), None);
        // A call past its cap says how much more there was.
        let long = (0..250).map(|i| format!("echo {i}")).collect::<Vec<_>>().join("\n");
        let big = tool(&part(json!({ "kind": "exec", "command": long }))).expect("a tool");
        let call = str_of(&big, "call").expect("a call");
        assert_eq!(call.lines().count(), CALL_MAX_LINES + 1);
        assert!(call.ends_with("… 50 more lines"));
    }

    #[test]
    fn a_compact_reply_folds_its_work_and_shows_the_text_it_ends_on() {
        let tool = |id: &str, command: &str| {
            json!({ "id": id, "kind": "tool", "isError": false, "resolved": true, "call": { "kind": "exec", "command": command } })
        };
        let text = |id: &str, words: &str| json!({ "id": id, "kind": "text", "text": words });
        let reply = |status: &str| {
            json!({ "id": "a", "role": "assistant", "status": status, "durationMs": 65_000,
                "parts": [text("p1", "Looking."), tool("k1", "ls"), text("p2", "Then:"), tool("k2", "cargo test"), text("p3", "Done."), text("p4", "All green.")] })
        };
        let kinds = |v: &Value| -> Vec<(String, String)> {
            let Value::List(list) = v else { panic!("a list") };
            list.iter()
                .map(|b| {
                    let field = |k: &str| match b.field(k) {
                        Some(Value::Str(s)) => s.to_string(),
                        _ => String::new(),
                    };
                    (field("id"), field("kind"))
                })
                .collect()
        };
        // Settled: one work group where the work began ("Worked for 1m 5s"),
        // then the run of text it ends on.
        let done = compact_blocks(&reply("complete")).expect("it did work");
        assert_eq!(
            kinds(&done),
            vec![("work".into(), "work".into()), ("p3".into(), "text".into()), ("p4".into(), "text".into())]
        );
        let Value::List(list) = &done else { unreachable!() };
        assert!(matches!(list[0].field("summary"), Some(Value::Str(s)) if s.as_ref() == "Worked for 1m 5s"));
        // Inside it, the ordinary blocks of its work: its words and groups.
        assert_eq!(
            kinds(list[0].field("blocks").unwrap()),
            vec![
                ("p1".into(), "text".into()),
                ("g:k1".into(), "tools".into()),
                ("p2".into(), "text".into()),
                ("g:k2".into(), "tools".into()),
            ]
        );
        // Still being written: no reply yet, everything rides the group, its
        // line the tools' summary.
        let live = compact_blocks(&reply("streaming")).expect("it did work");
        assert_eq!(kinds(&live), vec![("work".into(), "work".into())]);
        // No work, no group.
        let plain = json!({ "id": "b", "role": "assistant", "status": "complete", "parts": [text("t", "Hi.")] });
        assert!(compact_blocks(&plain).is_none());
    }

    #[test]
    fn a_settled_reply_with_nothing_to_show_has_no_row() {
        let reply = |status: &str| {
            json!({ "id": "a", "role": "assistant", "status": status, "createdAt": 1_788_900_000_000_i64,
                "parts": [{ "id": "r", "kind": "reasoning", "text": "" }] })
        };
        // Settled and empty: no row (no lane, no time over the next reply).
        assert!(message(&reply("complete"), None).is_none());
        // Still being written: its row holds the working line, and no time.
        let live = message(&reply("streaming"), None).expect("a reply being written has a row");
        assert!(live.field("at").is_none_or(|v| matches!(v, Value::Nil)));
    }

    #[test]
    fn an_upsert_lands_after_its_anchor() {
        let mut entries = vec![Entry::new(json!({ "id": "a" })), Entry::new(json!({ "id": "c" }))];
        let mut frame = json!({ "upsert": [{ "after": "a", "entry": { "id": "b" } }], "remove": [], "append": [], "count": 3 });
        assert!(matches!(apply(&mut entries, &mut frame), Ok(Change::Rows)));
        let ids: Vec<_> = entries.iter().map(|e| e["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[test]
    fn attachments_round_trip_through_the_prompt() {
        let prompt = with_attachments("", &["/tmp/a.png".into()], 1);
        let (words, paths) = split_attachments(&prompt);
        assert_eq!(words, "");
        assert_eq!(paths, ["/tmp/a.png"]);
    }

    #[test]
    fn a_replys_tools_group_between_its_text() {
        let text = |id: &str| json!({ "kind": "text", "id": id, "text": "words" });
        let exec = |id: &str| json!({ "kind": "tool", "id": id, "call": { "kind": "exec", "command": "ls" }, "resolved": true, "isError": false });
        let read = |id: &str| json!({ "kind": "tool", "id": id, "call": { "kind": "readFile", "path": "a.rs" }, "resolved": true, "isError": false });
        let entry = json!({ "id": "a", "role": "assistant", "status": "streaming", "parts": [
            { "kind": "reasoning", "id": "r1", "text": "hmm" },
            text("t1"), exec("x1"), exec("x2"), read("x3"), text("t2"), exec("x4"), read("x5"),
        ] });
        let Value::List(blocks) = blocks(&entry) else { panic!("no blocks") };
        let shape: Vec<String> = blocks
            .iter()
            .map(|b| format!("{}:{}", text_of(b, "kind"), text_of(b, "id")))
            .collect();
        assert_eq!(shape, ["tools:g:r1", "text:t1", "tools:g:x1", "text:t2", "tools:g:x4"]);
        assert_eq!(text_of(&blocks[0], "summary"), "Thought process");
        assert_eq!(text_of(&blocks[2], "summary"), "Ran 2 commands · read 1 file");
        // Open only at the tail of a streaming reply.
        assert!(!blocks[2].field("live").is_some_and(Value::truthy));
        assert!(blocks[4].field("live").is_some_and(Value::truthy));
    }

    fn text_of(v: &Value, key: &str) -> String {
        v.field(key).map(text).unwrap_or_default()
    }

    #[test]
    fn rows_for_threads_across_chats_come_from_the_engine_s_parts() {
        let kinds = |v: &Value| -> Vec<String> { v.list().iter().map(|b| text_of(b, "kind")).collect() };
        let send = |id: &str, chat: &str, resolved: bool, error: bool| {
            json!({ "kind": "tool", "id": id, "resolved": resolved, "isError": error,
                    "call": { "kind": "mcp", "server": "zeron", "tool": "send_message", "input": { "chat": chat, "text": "hi" } } })
        };
        let exec = |id: &str| json!({ "kind": "tool", "id": id, "resolved": true, "call": { "kind": "exec", "command": "ls" } });
        let reply = |parts: Vec<Json>| json!({ "id": "a1", "role": "assistant", "status": "complete", "parts": parts });
        let shown = blocks(&reply(vec![exec("x1"), send("m1", "Parser", false, false), send("m2", "c2", true, false), send("m3", "nobody", true, true), exec("x2")]));
        assert_eq!(kinds(&shown), ["tools", "sent", "sent", "sent", "tools"]);
        let sent = shown.list();
        assert_eq!((text_of(&sent[1], "to"), sent[1].field("pending") == Some(&Value::Bool(true))), ("Parser".into(), true));
        assert_eq!(sent[2].field("pending"), Some(&Value::Bool(false)));
        assert_eq!(sent[2].field("failed"), Some(&Value::Bool(false)));
        assert_eq!(sent[3].field("failed"), Some(&Value::Bool(true)));
        assert_eq!(sent[3].field("pending"), Some(&Value::Bool(false)), "a failed one isn't on its way");
        let many = json!({ "kind": "tool", "id": "m4", "resolved": true,
            "call": { "kind": "mcp", "server": "zeron", "tool": "send_messages", "input": { "requests": [ { "chat": "a", "text": "1" }, { "chat": "b", "text": "2" } ] } } });
        let two = blocks(&reply(vec![many]));
        assert_eq!(kinds(&two), ["sent", "sent"]);
        assert_eq!(text_of(&two.list()[1], "id"), "m4.m1");
        // Another server's tool of that name is a tool.
        let other = json!({ "kind": "tool", "id": "m5", "resolved": true, "call": { "kind": "mcp", "server": "slack", "tool": "send_message", "input": { "chat": "a", "text": "1" } } });
        assert_eq!(kinds(&blocks(&reply(vec![other]))), ["tools"]);

        let thread = json!({ "kind": "tool", "id": "t1", "resolved": true, "subagentRef": "th-1",
            "call": { "kind": "mcp", "server": "zeron", "tool": "create_thread", "input": { "title": "Write the notes" } } });
        let delegate = json!({ "kind": "tool", "id": "t2", "resolved": true, "subagentRef": "dg-1",
            "call": { "kind": "mcp", "server": "zeron", "tool": "delegate_task", "input": { "task": "Port the parser\nCarefully" } } });
        let spawn = json!({ "kind": "tool", "id": "t3", "subagentRef": "c--sub--1", "subagentStatus": "running",
            "call": { "kind": "unknown", "name": "Agent: scan", "input": { "model": "opus" } } });
        let shown = blocks(&reply(vec![exec("x1"), thread, delegate, spawn]));
        assert_eq!(kinds(&shown), ["tools", "tools", "tools"]);
        let tools = |b: &Value| b.field("tools").unwrap().list().to_vec();
        let started = &tools(&shown.list()[1])[0];
        assert_eq!(text_of(started, "kind"), "thread");
        assert_eq!(text_of(started.field("thread").unwrap(), "ref"), "th-1");
        assert_eq!(text_of(started.field("thread").unwrap(), "title"), "Write the notes");
        let agents = tools(&shown.list()[2]);
        assert_eq!(agents.iter().map(|t| text_of(t, "kind")).collect::<Vec<_>>(), ["delegate", "agent"], "a delegate and a spawn share a group");
        assert_eq!(text_of(agents[0].field("delegate").unwrap(), "ref"), "dg-1");
        assert_eq!(text_of(agents[0].field("delegate").unwrap(), "title"), "Port the parser Carefully");
        assert_eq!(text_of(&agents[1], "tag"), "opus", "the model the spawn named, labelled when the catalog has it");
        // Unbound (the engine hasn't stamped its chat yet): no chat to name.
        let unbound = json!({ "kind": "tool", "id": "t4", "call": { "kind": "mcp", "server": "zeron", "tool": "delegate_task", "input": { "task": "x" } } });
        let row = super::tool(&unbound).unwrap();
        assert_eq!(text_of(&row, "kind"), "delegate");
        assert!(matches!(row.field("delegate"), None | Some(Value::Nil)));
        let explore = json!({ "kind": "tool", "id": "t5", "subagentRef": "c--sub--2", "call": { "kind": "unknown", "name": "Agent: look", "input": { "subagent_type": "Explore" } } });
        assert_eq!(text_of(&super::tool(&explore).unwrap(), "tag"), "Explore");

        let native = json!({ "kind": "tool", "id": "n1", "resolved": true,
            "delegateRoute": { "source": "fell_back", "reason": "claude out of quota", "role": "scout", "resetsAt": 1_800_000_000_000_i64 },
            "call": { "kind": "unknown", "name": "Agent: summarize", "input": { "description": "Summarize the logs" } } });
        let shown = blocks(&reply(vec![native]));
        assert_eq!(kinds(&shown), ["native_route"]);
        let b = &shown.list()[0];
        assert_eq!(text_of(b, "title"), "Summarize the logs");
        let route = b.field("route").unwrap();
        assert_eq!(text_of(route, "source"), "fell_back");
        assert_eq!(text_of(route, "short"), "claude quota");
        assert_eq!(text_of(route, "role"), "scout");
        assert_eq!(route.field("resets_at"), Some(&Value::Num(1_800_000_000_000.)));
        // A route of a source this app doesn't know reads as none.
        assert_eq!(super::route_value(&json!({ "source": "future" })), Value::Nil);
    }

    #[test]
    fn a_message_from_another_thread_says_whose_and_what_woke() {
        let user = |text: &str, from: Option<&str>| {
            let mut e = json!({ "id": "u1", "role": "user", "createdAt": 1_700_000_000_000_i64, "parts": [ { "kind": "text", "id": "p1", "text": text } ] });
            if let Some(f) = from {
                e["fromChatId"] = json!(f);
            }
            e
        };
        let peer = message(&user("The tests pass.", Some("c2")), None).unwrap();
        assert_eq!(text_of(&peer, "from_chat"), "c2");
        assert!(matches!(peer.field("wake"), None | Some(Value::Nil)));
        assert!(text_of(&peer, "clock").ends_with("M"), "a clock: {}", text_of(&peer, "clock"));
        let wake = message(&user("Delegated task dg-1 (\"Review\") completed.\n<agent-response>\nAll good\n</agent-response>", Some("dg-1")), None).unwrap();
        let ids: Vec<String> = wake.field("wake").unwrap().list().iter().map(text).collect();
        assert_eq!(ids, ["dg-1"]);
        assert!(text_of(&wake, "text").contains("> All good"), "the reply set as a quote");
        let mine = message(&user("Hello", None), None).unwrap();
        assert!(matches!(mine.field("from_chat"), None | Some(Value::Nil)));
    }

    #[test]
    fn a_delegate_s_limit_offer_is_published_with_its_countdown() {
        let frame = json!({ "autoResume": true, "chats": [
            { "chatId": "me", "resetsAt": 1, "auto": true },
            { "chatId": "dg-1", "resetsAt": 2, "auto": false, "offer": { "to": { "harness": "codex", "model": "gpt-6.1", "reasoning": "high" }, "preselect": "switch", "actAt": 99 } },
            { "chatId": "dg-2", "resetsAt": 3, "auto": true },
        ] });
        let rows = super::offered_limits(&frame, "me");
        assert_eq!(rows.len(), 1);
        let chats = Chats::default();
        let v = super::limit_value(&chats, &rows[0]);
        assert_eq!(text_of(&v, "chat"), "dg-1");
        assert_eq!(v.field("act_at"), Some(&Value::Num(99.)));
        assert_eq!(text_of(&v, "preselect"), "switch");
        assert_eq!(text_of(&v, "target"), "codex · gpt-6.1 · High");
        assert_eq!(v.field("to"), Some(&Value::Bool(true)));
    }

    #[test]
    fn base64_pads() {
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn a_visual_part_is_a_block_of_its_own_read_from_its_host() {
        let entry = json!({ "id": "a1", "role": "assistant", "status": "complete", "parts": [
            { "kind": "text", "id": "t1", "text": "Here it is:" },
            { "kind": "visual", "id": "v:1", "path": "/u/v_1-visual.luau", "title": "Latency" },
        ]});
        let shown = blocks(&entry);
        let list = shown.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].field("kind"), Some(&Value::from("visual")));
        assert_eq!(list[1].field("id"), Some(&Value::from("v:1")));
        assert!(list[1].field("device").is_none_or(|d| *d == Value::Nil), "this device's: no device");
        // A chat on another device: its visuals are read from there.
        assert!(with_host(&entry, "c1", None).is_none());
        let remote = with_host(&entry, "c1", Some("dev-2")).expect("it has a visual");
        let shown = blocks(&remote);
        assert_eq!(shown.list()[1].field("device"), Some(&Value::from("dev-2")));
    }

    #[test]
    fn a_published_page_is_a_block_after_its_tool_and_a_prompt_s_picks_its_chips() {
        let page = json!({ "kind": "tool", "id": "tc1", "resolved": true, "isError": false,
            "call": { "kind": "mcp", "server": "zeron", "tool": "html_render", "input": { "path": "out/tray.html", "title": "Tray", "height": 420 } },
            "htmlRender": { "path": "/u/html/c1/ab.html", "title": "Tray", "height": 420 } });
        let entry = json!({ "id": "a1", "role": "assistant", "status": "complete", "parts": [
            page,
            { "kind": "text", "id": "t1", "text": "Made [it](out/tray.html) and `notes.md`." },
        ]});
        let list = blocks(&entry).list().to_vec();
        let kinds: Vec<String> = list.iter().map(|b| text_of(b, "kind")).collect();
        assert_eq!(kinds, ["tools", "html", "text"], "the page after its tool, above the reply");
        assert_eq!(text_of(&list[1], "path"), "/u/html/c1/ab.html");
        assert_eq!(list[1].field("height"), Some(&Value::Num(420.)));
        let row = message(&entry, None).unwrap();
        let shown = row.field("blocks").unwrap().list();
        let files = shown.last().unwrap();
        assert_eq!(text_of(files, "kind"), "html_files");
        assert_eq!(text_of(&files.field("targets").unwrap().list()[0], "dest"), "out/tray.html");
        let compact = row.field("compact").unwrap().list();
        let kinds: Vec<String> = compact.iter().map(|b| text_of(b, "kind")).collect();
        assert_eq!(kinds, ["work", "html", "text", "html_files"], "compact: the page outside the fold");
        assert!(compact[0].field("blocks").unwrap().list().iter().all(|b| text_of(b, "kind") != "html"));
        let remote = with_host(&entry, "c1", Some("dev-2")).expect("a page is read from its host");
        assert_eq!(text_of(&blocks(&remote).list()[1], "device"), "dev-2");
        let sent = zeron_proto::html_render::with_picks(
            "go",
            &[("Tray".into(), vec![zeron_proto::html_render::Pick { group: Some("A".into()), code: "A2".into(), label: "Floating".into() }], vec![])],
        );
        let prompt = json!({ "id": "u2", "role": "user", "createdAt": 1000., "parts": [{ "kind": "text", "text": sent }] });
        let row = message(&prompt, None).unwrap();
        assert_eq!(text_of(&row, "text"), "go");
        let chip = &row.field("attachments").unwrap().list()[0];
        assert_eq!((text_of(chip, "kind"), text_of(chip, "card"), text_of(chip, "group"), text_of(chip, "code")), ("pick".into(), "Tray".into(), "A".into(), "A2".into()));
    }

    /// A command whose output the doc cut names the doc it is in (its row
    /// asks that doc's host for the rest), with its exit code and the whole
    /// output's lines; one kept whole names nothing.
    #[test]
    fn a_cut_command_output_names_its_doc_lines_and_exit_code() {
        let exec = |id: &str, cut: bool| {
            let mut part = json!({ "kind": "tool", "id": id, "resolved": true, "isError": true, "exitCode": 2,
                                   "output": "error: boom", "call": { "kind": "exec", "command": "make" } });
            if cut {
                part["outputTruncated"] = json!(true);
                part["outputLines"] = json!(300);
            }
            part
        };
        let entry = json!({ "id": "a1", "role": "assistant", "status": "complete",
                            "parts": [exec("t1", true), exec("t2", false)] });
        let stamped = with_host(&entry, "c1--sub--x", None).expect("a cut output");
        let row = tool(&stamped["parts"][0]).unwrap();
        assert_eq!(row.field("chat"), Some(&Value::from("c1--sub--x")));
        assert_eq!(row.field("output_truncated"), Some(&Value::Bool(true)));
        assert!(matches!(row.field("output_lines"), Some(Value::Num(n)) if *n == 300.0));
        assert!(matches!(row.field("exit_code"), Some(Value::Num(n)) if *n == 2.0));
        let whole = tool(&stamped["parts"][1]).unwrap();
        assert!(whole.field("chat").is_none() && whole.field("output_truncated").is_none());
        assert!(with_host(&json!({ "parts": [exec("t2", false)] }), "c1", None).is_none());
    }

    #[test]
    fn display_tools_are_their_output_alone() {
        let mcp = |id: &str, tool: &str, resolved: bool, error: bool| {
            json!({ "kind": "tool", "id": id, "resolved": resolved, "isError": error, "output": "no such file: chart.png",
                    "call": { "kind": "mcp", "server": "zeron", "tool": tool, "input": {} } })
        };
        let exec = |id: &str| json!({ "kind": "tool", "id": id, "resolved": true, "isError": false, "call": { "kind": "exec", "command": "ls" } });
        let reply = |parts: Vec<Json>| json!({ "id": "a1", "role": "assistant", "status": "complete", "parts": parts });
        // A call that worked, or is working: nothing of its own, in no group.
        let entry = reply(vec![exec("x1"), mcp("s1", "show_visual", true, false), exec("x2")]);
        let shown = blocks(&entry);
        assert_eq!(shown.list().len(), 1, "one group round it");
        assert_eq!(shown.list()[0].field("summary"), Some(&Value::from("Ran 2 commands")));
        assert_eq!(shown.list()[0].field("tools").unwrap().list().len(), 2);
        let running = reply(vec![mcp("s1", "mcp__zeron__show_image", false, false)]);
        assert!(blocks(&running).list().is_empty());
        let message = message(&entry, None).unwrap();
        assert_eq!(message.field("tools").unwrap().list().len(), 2, "the message's tools leave it out");
        // Failed: a quiet line with why, where it was.
        let failed = reply(vec![exec("x1"), mcp("s1", "show_image", true, true), exec("x2")]);
        let list = blocks(&failed);
        let kinds: Vec<String> = list.list().iter().map(|b| crate::auth::text(b.field("kind").unwrap())).collect();
        assert_eq!(kinds, ["tools", "display_failed", "tools"]);
        assert_eq!(list.list()[1].field("text"), Some(&Value::from("no such file: chart.png")));
        // Compact: no work of its own, the line in the reply.
        let only = reply(vec![mcp("s1", "show_visual", true, true), json!({ "kind": "text", "id": "t1", "text": "Done." })]);
        assert!(compact_blocks(&only).is_none(), "no work at all");
        let mixed = reply(vec![exec("x1"), mcp("s1", "show_visual", true, true), json!({ "kind": "text", "id": "t1", "text": "Done." })]);
        let compact = compact_blocks(&mixed).unwrap();
        let kinds: Vec<String> = compact.list().iter().map(|b| crate::auth::text(b.field("kind").unwrap())).collect();
        assert_eq!(kinds, ["work", "display_failed", "text"]);
    }

    #[test]
    fn pictures_an_agent_shows_sit_in_the_reply_in_place() {
        let mcp = |id: &str, error: bool| {
            json!({ "kind": "tool", "id": id, "resolved": true, "isError": error, "output": "visual:2: attempt to index nil",
                    "call": { "kind": "mcp", "server": "zeron", "tool": "mcp__zeron__show_image", "input": {} } })
        };
        let image = |id: &str, name: &str| json!({ "kind": "image", "id": id, "path": format!("/u/{name}"), "name": name, "mimeType": "image/png" });
        let exec = |id: &str| json!({ "kind": "tool", "id": id, "resolved": true, "isError": false, "call": { "kind": "exec", "command": "ls" } });
        let text = |id: &str, t: &str| json!({ "kind": "text", "id": id, "text": t });
        let reply = |parts: Vec<Json>| json!({ "id": "a1", "role": "assistant", "status": "complete", "parts": parts });
        let kinds = |v: &Value| -> Vec<String> { v.list().iter().map(|b| crate::auth::text(b.field("kind").unwrap())).collect() };
        // In a run of tools: the picture breaks out where its call was, the
        // tools either side fold as ever, and it counts in neither.
        let entry = reply(vec![exec("x1"), mcp("s1", false), image("i1", "a.png"), exec("x2"), exec("x3")]);
        let shown = blocks(&entry);
        assert_eq!(kinds(&shown), ["tools", "images", "tools"]);
        assert_eq!(shown.list()[0].field("summary"), Some(&Value::from("Ran 1 command")));
        assert_eq!(shown.list()[2].field("summary"), Some(&Value::from("Ran 2 commands")));
        let pics = shown.list()[1].field("images").unwrap().list();
        assert_eq!(pics.len(), 1);
        assert_eq!(pics[0].field("path"), Some(&Value::from("/u/a.png")));
        assert_eq!(pics[0].field("name"), Some(&Value::from("a.png")));
        // Two shown one after another (each its own call): one block, keyed
        // by the first, so a second arriving joins it.
        let two = reply(vec![text("t1", "Two:"), mcp("s1", false), image("i1", "a.png"), mcp("s2", false), image("i2", "b.png"), text("t2", "Done.")]);
        let shown = blocks(&two);
        assert_eq!(kinds(&shown), ["text", "images", "text"]);
        assert_eq!(shown.list()[1].field("id"), Some(&Value::from("i1")));
        assert_eq!(shown.list()[1].field("images").unwrap().list().len(), 2);
        // A reply of a picture alone has a row.
        assert!(message(&reply(vec![mcp("s1", false), image("i1", "a.png")]), None).is_some());
        // Compact: the picture in the reply, not the work.
        let compact = compact_blocks(&entry).unwrap();
        assert_eq!(kinds(&compact), ["work", "images"]);
        // Failed: which tool, and why as said (ds/reply_picture keeps what's plain).
        let failed = blocks(&reply(vec![mcp("s1", true), text("t1", "Sorry.")]));
        assert_eq!(kinds(&failed), ["display_failed", "text"]);
        assert_eq!(failed.list()[0].field("tool"), Some(&Value::from("image")));
        // Failed, then shown on another try: the stumble isn't drawn.
        let retried = blocks(&reply(vec![mcp("s1", true), mcp("s2", false), image("i1", "a.png")]));
        assert_eq!(kinds(&retried), ["images"]);
    }

    #[test]
    fn the_last_turn_is_what_its_turn_added_to_the_stats() {
        use zeron_proto::SessionStats as S;
        let at = |turns: u64, output_tokens: u64, usage_llm_ms: u64| S { turns, output_tokens, usage_llm_ms, ..Default::default() };
        let (three, four) = (at(3, 900, 30_000), at(4, 2_100, 53_000));
        // Opened on a chat with turns behind it: which was the last can't be told.
        assert_eq!(turn_base(None, None, &three), None);
        // One seen starting: what it added.
        let base = turn_base(None, Some(&three), &four);
        assert_eq!(base, Some((three, 4)));
        let last = last_turn(&base.unwrap().0, &four);
        assert_eq!((last.output_tokens, last.usage_llm_ms), (1_200, 23_000));
        // Its later segments' writes still count to it; it's kept, opened again, while it's the latest.
        let later = at(4, 2_500, 60_000);
        assert_eq!(turn_base(base, Some(&four), &later), base);
        assert_eq!(turn_base(base, None, &later), base);
        assert_eq!(turn_base(base, None, &at(5, 3_000, 70_000)), None);
        // Two turns at once, or the stats starting over: no turn of its own.
        assert_eq!(turn_base(base, Some(&three), &at(5, 3_000, 70_000)), None);
        assert_eq!(turn_base(base, Some(&four), &at(1, 10, 100)), None);
        // A chat's first turn is its whole stats.
        assert_eq!(turn_base(None, None, &at(1, 288, 4_000)), Some((S::default(), 1)));
        let v = stats_value(&four, Some(last));
        let said = |k: &str| v.field("last").and_then(|l| l.field(k)).map(text);
        assert_eq!(said("tps").as_deref(), Some("52"));
        assert_eq!(said("output").as_deref(), Some("1.2K"));
        // A harness that bills nothing: no last turn to say.
        assert_eq!(stats_value(&four, Some(at(0, 0, 9_000))).field("last"), Some(&Value::Nil));
        assert_eq!(stats_value(&four, None).field("last"), Some(&Value::Nil));
        let no_time = stats_value(&four, Some(at(0, 40, 0)));
        assert_eq!(no_time.field("last").and_then(|l| l.field("tps")), Some(&Value::Nil));
    }

    #[test]
    fn a_running_turns_output_is_its_words_since_the_prompt() {
        let entry = |id: &str, role: &str, status: &str, parts: Json| Entry::new(json!({ "id": id, "role": role, "status": status, "parts": parts }));
        let text = |t: &str| json!({ "kind": "text", "text": t });
        let thought = |t: &str| json!({ "kind": "reasoning", "text": t });
        let tool = json!({ "kind": "tool", "call": { "kind": "unknown", "name": "Bash" }, "resolved": true });
        let before = entry("a0", "assistant", "complete", json!([text(&"x".repeat(4_000))]));
        let prompt = entry("u1", "user", "complete", json!([text("go")]));
        let segment = entry("a1", "assistant", "complete", json!([thought(&"t".repeat(400)), tool]));
        let streaming = entry("a2", "assistant", "streaming", json!([text(&"é".repeat(4_001))]));
        assert_eq!(turn_tokens(&[before, prompt, segment, streaming]), Some(1_101));
        let quiet = entry("a3", "assistant", "streaming", json!([]));
        assert_eq!(turn_tokens(&[entry("u2", "user", "complete", json!([text("go")])), quiet]), None);
        let done = entry("a4", "assistant", "complete", json!([text("done")]));
        assert_eq!(turn_tokens(&[done]), None);
        assert_eq!(turn_tokens(&[]), None);
    }

    #[test]
    fn a_chats_picks_set_the_rings_window() {
        let config = |harness: &str, picks: Json| json!({ "harness": harness, "model": null, "reasoning": null, "modelOptions": picks, "sandbox": "workspace-write" });
        assert_eq!(chat_budget(&config("claude-code", json!({}))), None);
        assert_eq!(chat_budget(&config("claude-code", json!({ "autoCompact": "400k" }))), Some(400_000.));
        assert_eq!(chat_budget(&config("claude-code", json!({ "contextWindow": "200k", "autoCompact": "400k" }))), Some(200_000.));
        assert_eq!(chat_budget(&config("codex", json!({ "autoCompact": "400k" }))), None);
        assert_eq!(chat_budget(&Json::Null), None);

        let chats = Chats::default();
        chats.chats.borrow_mut().insert("c".to_owned(), json!({ "id": "c", "config": config("claude-code", json!({ "autoCompact": "400k" })) }));
        let open = Open { id: "c".to_owned(), context_usage: (Some(100_000.), Some(1_000_000.)), ..Open::default() };
        let v = session_value(&chats, &open, false, false);
        assert_eq!(v.field("context_window").and_then(Value::as_f64), Some(400_000.));
        assert_eq!(v.field("context").and_then(Value::as_f64), Some(0.25));
    }

    #[test]
    fn an_unresolved_advisor_call_on_a_streaming_entry_is_consulting() {
        let call = serde_json::to_value(zeron_proto::ToolCall::Unknown { name: zeron_proto::ADVISOR_TOOL_NAME.to_owned(), input: None }).unwrap();
        let entry = |status: &str, resolved: bool, call: &Json| json!({ "status": status, "parts": [{ "kind": "tool", "id": "a", "resolved": resolved, "call": call }] });
        assert!(consulting_advisor(&entry("streaming", false, &call)));
        assert!(!consulting_advisor(&entry("streaming", true, &call)));
        assert!(!consulting_advisor(&entry("complete", false, &call)));
        assert!(!consulting_advisor(&entry("streaming", false, &json!({ "kind": "exec", "command": "ls" }))));
    }

    #[test]
    fn a_cloud_machine_says_what_the_send_waits_on() {
        use zeron_proto::CloudState as S;
        let answer = json!({ "sessions": [
            { "chatId": "a", "deviceId": "cloud-a", "spaceId": "p", "state": "provisioning" },
            { "chatId": "b", "deviceId": "cloud-b", "spaceId": "p", "state": "sleeping" },
            { "chatId": "c", "deviceId": "cloud-c", "spaceId": "p", "state": "ready" },
        ] });
        assert_eq!(machine_state(&answer, "a", "x"), Some(S::Provisioning));
        assert_eq!(machine_state(&answer, "side", "cloud-b"), Some(S::Sleeping));
        assert_eq!(booting_word(machine_state(&answer, "a", "x")), Some("starting"));
        assert_eq!(booting_word(machine_state(&answer, "b", "x")), Some("waking"));
        assert_eq!(booting_word(machine_state(&answer, "c", "x")), None);
        assert_eq!(booting_word(None), None);
    }

    #[cfg(unix)]
    #[test]
    fn one_rate_stream_for_each_chat_on_screen() {
        let (rpc, sent, _bell) = Rpc::recording();
        let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/activity_fixture.luau");
        let mut made: Vec<Rc<View>> = Vec::new();
        let _app = zgx_script::testing::TestApp::new(|runtime| {
            let host = ScriptHost::new(runtime, entry);
            for slot in ["a", "b"] {
                made.push(Rc::new(View {
                    session: host.shared(&format!("t.{slot}.session"), Value::Nil),
                    messages: host.shared(&format!("t.{slot}.messages"), Value::Nil),
                    subagents: host.shared(&format!("t.{slot}.subagents"), Value::Nil),
                    draft: host.shared(&format!("t.{slot}.draft"), Value::Nil),
                    stats: host.shared(&format!("t.{slot}.stats"), Value::Nil),
                    token_rate: host.shared(&format!("t.{slot}.rate"), Value::Nil),
                    turn_tokens: host.shared(&format!("t.{slot}.turn_tokens"), Value::Nil),
                    on_screen: Cell::new(true),
                    rpc: RefCell::default(),
                    open: RefCell::default(),
                    pending: RefCell::default(),
                    generation: Cell::new(0),
                    snapshot: Cell::new(false),
                    predicts: false,
                }));
            }
            host
        });
        let chats = Rc::new(Chats::default());
        *chats.rpc.borrow_mut() = Some(rpc.clone());
        *chats.here.borrow_mut() = Some("me".to_owned());
        for id in ["c1", "c2"] {
            chats.chats.borrow_mut().insert(id.to_owned(), json!({ "id": id, "deviceId": "me" }));
        }
        for view in &made {
            chats.views.borrow_mut().push(Rc::downgrade(view));
        }
        let show = |view: &View, id: Option<&str>| {
            *view.open.borrow_mut() = id.map(|id| Open { id: id.to_owned(), host_watched: true, ..Open::default() });
        };
        let frames = |rx: &std::sync::mpsc::Receiver<String>| -> Vec<Json> {
            rx.try_iter().map(|f| serde_json::from_str(&f).unwrap()).collect()
        };
        let rate_of = |view: &View| view.token_rate.get().as_f64();
        show(&made[0], Some("c1"));
        show(&made[1], Some("c1"));
        chats.follow_rates();
        let opened = frames(&sent);
        assert_eq!(opened.len(), 1, "{opened:?}");
        assert_eq!(opened[0]["method"], "WatchTokenRate");
        assert_eq!(opened[0]["params"]["chatId"], "c1");
        let c1 = opened[0]["id"].as_u64().unwrap();
        rpc.heard(json!({ "id": c1, "item": { "rate": 41 } }));
        assert_eq!((rate_of(&made[0]), rate_of(&made[1])), (Some(41.), Some(41.)));
        show(&made[1], Some("c2"));
        chats.follow_rates();
        let opened = frames(&sent);
        assert_eq!(opened.len(), 1, "{opened:?}");
        assert_eq!(opened[0]["params"]["chatId"], "c2");
        let c2 = opened[0]["id"].as_u64().unwrap();
        assert_eq!((rate_of(&made[0]), rate_of(&made[1])), (Some(41.), None));
        rpc.heard(json!({ "id": c1, "item": { "rate": null } }));
        assert_eq!(rate_of(&made[0]), None);
        rpc.heard(json!({ "id": c1, "item": { "rate": 12 } }));
        rpc.heard(json!({ "id": c1, "done": true }));
        assert_eq!(rate_of(&made[0]), None);
        assert!(chats.rates.borrow().get("c1").is_none());
        show(&made[1], None);
        chats.follow_rates();
        let closed = frames(&sent);
        assert!(closed.iter().any(|f| f["id"].as_u64() == Some(c2) && f["cancel"] == true), "{closed:?}");
        assert!(chats.rates.borrow().get("c2").is_none());

        show(&made[1], Some("c2"));
        chats.follow_rates();
        let c2 = frames(&sent)[0]["id"].as_u64().unwrap();
        made[1].on_screen.set(false);
        chats.follow_rates();
        let closed = frames(&sent);
        assert!(closed.iter().any(|f| f["id"].as_u64() == Some(c2) && f["cancel"] == true), "{closed:?}");
        assert!(chats.rates.borrow().get("c2").is_none());
        assert_eq!(rate_of(&made[1]), None);
        made[1].on_screen.set(true);
        chats.follow_rates();
        assert_eq!(frames(&sent)[0]["params"]["chatId"], "c2");
    }

    #[test]
    fn the_chips_refreshing_follows_gpuis_turn_in_flight() {
        let chats = Chats::default();
        let row = |status: &str, held: bool| json!({ "status": status, "heldBySubagents": held, "updatedAt": now_seconds() * 1000 });
        let open = Open { id: "c".to_owned(), ..Open::default() };
        let in_flight = |chats: &Chats, open: &Open, sending: bool| chats.turn_in_flight(open, sending);
        assert!(!in_flight(&chats, &open, false));
        chats.sessions.borrow_mut().insert("c".to_owned(), row("working", false));
        assert!(in_flight(&chats, &open, false));
        chats.sessions.borrow_mut().insert("c".to_owned(), row("working", true));
        assert!(!in_flight(&chats, &open, false));
        assert!(in_flight(&chats, &open, true));
        chats.sessions.borrow_mut().insert("c".to_owned(), row("idle", false));
        assert!(!in_flight(&chats, &open, false));
    }

    #[test]
    fn a_frame_without_stats_clears_them() {
        let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/activity_fixture.luau");
        let mut made = None;
        let _app = zgx_script::testing::TestApp::new(|runtime| {
            let host = ScriptHost::new(runtime, entry);
            made = Some(Rc::new(View {
                session: host.shared("t.session", Value::Nil),
                messages: host.shared("t.messages", Value::Nil),
                subagents: host.shared("t.subagents", Value::Nil),
                draft: host.shared("t.draft", Value::Nil),
                stats: host.shared("t.stats", Value::Nil),
                token_rate: host.shared("t.rate", Value::Nil),
                turn_tokens: host.shared("t.turn_tokens", Value::Nil),
                on_screen: Cell::new(true),
                rpc: RefCell::default(),
                open: RefCell::default(),
                pending: RefCell::default(),
                generation: Cell::new(0),
                snapshot: Cell::new(false),
                predicts: false,
            }));
            host
        });
        let view = made.unwrap();
        *view.open.borrow_mut() = Some(Open { id: "c".to_owned(), ..Open::default() });
        let stats = zeron_proto::SessionStats { turns: 2, ..Default::default() };
        view.heard_frame(json!({ "sessionStats": stats }));
        assert_eq!(view.stats.get().field("turns").and_then(Value::as_f64), Some(2.));
        view.heard_frame(json!({ "sessionStats": null }));
        assert_eq!(view.stats.get(), Value::Nil);
        view.heard_frame(json!({ "sessionStats": stats }));
        view.heard_frame(json!({}));
        assert_eq!(view.stats.get(), Value::Nil);
        assert!(view.open.borrow().as_ref().unwrap().stats.is_none());
        // Its second turn was there before it opened: no last turn; the next one seen ending is.
        let last = |v: &View| v.stats.get().field("last").and_then(|l| l.field("output")).map(text);
        view.heard_frame(json!({ "sessionStats": stats }));
        assert_eq!(last(&view), None);
        let third = zeron_proto::SessionStats { turns: 3, output_tokens: 1_200, usage_llm_ms: 23_000, ..Default::default() };
        view.heard_frame(json!({ "sessionStats": third }));
        assert_eq!(last(&view).as_deref(), Some("1.2K"));
    }

    #[test]
    fn background_tasks_outlive_the_turn_and_go_with_a_stale_row() {
        use super::{Value, background_tasks, now_seconds};
        let fresh = (now_seconds() * 1000) as f64;
        let row = |status: &str, updated: f64| {
            serde_json::json!({
                "chatId": "c1", "status": status, "updatedAt": updated,
                "backgroundTasks": [
                    { "id": "b1", "command": "cargo test", "startedAt": fresh },
                    { "id": "b2", "command": "npm run dev", "description": "Dev server" },
                ],
            })
        };
        let ids = |tasks: &[Value]| -> Vec<String> {
            tasks.iter().map(|t| crate::auth::text(t.field("id").unwrap())).collect()
        };
        // Working, the turn ended (idle, completed) or waiting: the same rows,
        // each time, so the tray neither empties nor fills again.
        for status in ["working", "idle", "completed", "awaitingInput", "failed"] {
            assert_eq!(ids(&background_tasks(&row(status, fresh))), ["b1", "b2"], "{status}");
        }
        let tasks = background_tasks(&row("idle", fresh));
        assert_eq!(tasks[1].field("name"), Some(&Value::from("Dev server")));
        assert_eq!(tasks[1].field("detail"), Some(&Value::from("npm run dev")));
        assert_eq!(tasks[0].field("detail"), None);
        // A row gone quiet (its engine stopped beating): they went with it.
        let quiet = fresh - 120_000.;
        assert!(background_tasks(&row("idle", quiet)).is_empty());
        assert!(background_tasks(&serde_json::Value::Null).is_empty());
    }
}
