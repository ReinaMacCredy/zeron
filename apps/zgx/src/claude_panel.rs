use std::{cell::RefCell, rc::Rc};

use serde::de::DeserializeOwned;
use serde_json::{Value as Json, json};
use zeron_proto::claude_config::{self as cc, ClaudeConfigSnapshot, LocalCommandOutput};
use zeron_proto::claude_panel::{Change, ChipChoices, ConfigState, Filter, Load, RowView, STATS_DAYS, heat_level, stats_of, status_rows};
use zeron_proto::usage::{UsageSummary, format_tokens};
use zeron_proto::{AccessMode, HarnessId, ReasoningLevel};
use zgx_script::{ScriptHost, Shared, Value};

use crate::auth::text;
use crate::engine;
use crate::plugins::json_value;

struct Panel {
    generation: u64,
    open: bool,
    params: Json,
    note_chat: Option<(String, Option<String>)>,
    session_id: Option<String>,
    state: ConfigState,
    status: Load<LocalCommandOutput>,
    stats: Option<Load<UsageSummary>>,
}

impl Panel {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            open: false,
            params: json!({}),
            note_chat: None,
            session_id: None,
            state: ConfigState::default(),
            status: Load::Loading,
            stats: None,
        }
    }
}

struct Host {
    panel: RefCell<Panel>,
    shared: Shared,
}

pub fn install(host: &ScriptHost) {
    let me = Rc::new(Host { panel: RefCell::new(Panel::new(0)), shared: host.shared("claude_panel", Value::Nil) });
    let h = me.clone();
    host.action("claude_panel.open", move |args| h.open(&args));
    let h = me.clone();
    host.action("claude_panel.stats", move |_| h.load_stats());
    let h = me.clone();
    host.action("claude_panel.close", move |_| h.close());
    let h = me.clone();
    host.function("claude_panel.rows", move |args| h.rows_value(&args));
    let h = me.clone();
    host.function("claude_panel.status", move |args| h.status_value(&args));
    let h = me;
    host.function("claude_panel.set", move |args| {
        if crate::plugins::preview::refuse("claude_panel.set").is_err() {
            return json_value(&json!({ "change": "refused" }));
        }
        h.set(&args)
    });
}

async fn call_as<T: DeserializeOwned>(method: &str, params: Json) -> Result<T, String> {
    let answer = engine::get().call(method, params).await?;
    serde_json::from_value(answer).map_err(|e| e.to_string())
}

fn load_state<T>(load: &Load<T>) -> (&'static str, Option<&str>) {
    match load {
        Load::Loading => ("loading", None),
        Load::Ready(_) => ("ready", None),
        Load::Failed(error) => ("failed", Some(error.as_str())),
    }
}

impl Host {
    fn publish(&self) {
        let p = self.panel.borrow();
        let (values, error) = load_state(&p.state.values);
        let (status, status_error) = load_state(&p.status);
        let stats = match &p.stats {
            None => json!({ "state": "idle" }),
            Some(Load::Loading) => json!({ "state": "loading" }),
            Some(Load::Failed(error)) => json!({ "state": "failed", "error": error }),
            Some(Load::Ready(summary)) => stats_json(summary),
        };
        self.shared.set(json_value(&json!({
            "open": p.open,
            "generation": p.generation,
            "values": values,
            "error": error,
            "status": status,
            "status_error": status_error,
            "writing": p.state.writing.is_some(),
            "changes": p.state.changes.len(),
            "footer": p.state.footer(),
            "stats": stats,
        })));
    }

    fn open(self: &Rc<Self>, args: &Value) {
        let chat = args.field("chat").map(text).filter(|c| !c.is_empty());
        let device = args.field("device").map(text).filter(|d| !d.is_empty());
        let space = args.field("space").map(text).filter(|s| !s.is_empty());
        let claude = args.field("claude").is_some_and(Value::truthy);
        let mut params = json!({});
        if let Some(chat) = &chat {
            params["chatId"] = json!(chat);
        } else if let Some(space) = &space {
            params["spaceId"] = json!(space);
        }
        if let Some(device) = &device {
            params["targetDeviceId"] = json!(device);
        }
        let session_id = chat
            .as_deref()
            .and_then(crate::chat::chat_row)
            .and_then(|row| row["harnessSessionId"].as_str().map(str::to_owned))
            .filter(|id| !id.is_empty());
        let generation = {
            let mut p = self.panel.borrow_mut();
            let generation = p.generation + 1;
            *p = Panel::new(generation);
            p.open = true;
            p.params = params.clone();
            p.note_chat = chat.map(|c| (c, device));
            p.session_id = session_id;
            generation
        };
        self.publish();
        if !claude {
            return;
        }
        let me = self.clone();
        let read = params.clone();
        engine::get().spawn(async move {
            let reply = call_as::<ClaudeConfigSnapshot>("ReadClaudeConfig", read).await;
            if me.current(generation) {
                me.panel.borrow_mut().state.snapshot_arrived(reply);
                me.publish();
            }
        });
        let me = self.clone();
        let mut usage = params;
        usage["command"] = json!("/usage");
        usage["connectMcp"] = json!(true);
        engine::get().spawn(async move {
            let reply = call_as::<LocalCommandOutput>("RunClaudeLocalCommand", usage).await;
            if me.current(generation) {
                me.panel.borrow_mut().status = Load::from(reply);
                me.publish();
            }
        });
    }

    fn current(&self, generation: u64) -> bool {
        self.panel.borrow().generation == generation
    }

    fn load_stats(self: &Rc<Self>) {
        let (generation, params) = {
            let mut p = self.panel.borrow_mut();
            if p.stats.is_some() || !p.open {
                return;
            }
            p.stats = Some(Load::Loading);
            let mut params = json!({ "days": STATS_DAYS });
            if let Some(target) = p.params.get("targetDeviceId") {
                params["targetDeviceId"] = target.clone();
            }
            (p.generation, params)
        };
        self.publish();
        let me = self.clone();
        engine::get().spawn(async move {
            let reply = call_as::<UsageSummary>("GetUsageSummary", params).await;
            if me.current(generation) {
                me.panel.borrow_mut().stats = Some(Load::from(reply));
                me.publish();
            }
        });
    }

    fn close(&self) {
        let (summary, note_chat) = {
            let mut p = self.panel.borrow_mut();
            if !p.open {
                return;
            }
            p.open = false;
            (p.state.summary(), p.note_chat.clone())
        };
        self.publish();
        if crate::plugins::preview::refuse("claude_panel.close").is_err() {
            return;
        }
        let (Some(text), Some((chat_id, device))) = (summary, note_chat) else {
            return;
        };
        let mut params = json!({ "chatId": chat_id, "messageId": crate::sessions::new_id(), "text": text });
        if let Some(device) = device {
            params["targetDeviceId"] = json!(device);
        }
        engine::get().spawn(async move {
            if let Err(error) = engine::get().call("WriteChatNote", params).await {
                eprintln!("zeron-zgx: claude settings summary was not written: {error}");
            }
        });
    }

    fn rows_value(&self, args: &Value) -> Value {
        let chips = Chips::of(args.field("chips"));
        let filter = match args.field("filter").map(text).as_deref() {
            Some("zeron") => Filter::Zeron,
            Some("terminal") => Filter::Terminal,
            _ => Filter::All,
        };
        let query = args.field("query").map(text).unwrap_or_default();
        let p = self.panel.borrow();
        let rows: Vec<Json> = p
            .state
            .visible_rows(&chips.choices, filter, query.trim())
            .iter()
            .map(|row| row_json(row, &p.state))
            .collect();
        json_value(&json!({
            "rows": rows,
            "counts": { "all": Filter::All.count(), "zeron": Filter::Zeron.count(), "terminal": Filter::Terminal.count() },
        }))
    }

    fn status_value(&self, args: &Value) -> Value {
        let chips = Chips::of(args.field("chips"));
        let p = self.panel.borrow();
        let home = std::env::var("HOME").ok();
        let rows: Vec<Json> = status_rows(&p.status, &chips.choices, p.session_id.as_deref(), home.as_deref())
            .into_iter()
            .map(|row| json!({ "key": row.key, "value": row.value, "chip": row.chip }))
            .collect();
        json_value(&json!(rows))
    }

    fn set(self: &Rc<Self>, args: &Value) -> Value {
        let chips = Chips::of(args.field("chips"));
        let id = args.field("id").map(text).unwrap_or_default();
        let value = args.field("value").map(text).unwrap_or_default();
        let answer = {
            let mut p = self.panel.borrow_mut();
            let Some(row) = p.state.rows(&chips.choices).into_iter().find(|r| r.setting.id == id) else {
                return json_value(&json!({ "change": "unchanged" }));
            };
            match p.state.set_value(&row, value) {
                Change::Chip(chip, label) => {
                    p.state.record_change(row.setting.id, &row.value, &label);
                    let (chip, value) = chips.value_of(chip, &label);
                    json!({ "change": "chip", "chip": chip, "value": value })
                }
                Change::Queued => json!({ "change": "queued" }),
                Change::Unchanged => json!({ "change": "unchanged" }),
                Change::Refused => json!({ "change": "refused" }),
            }
        };
        if answer["change"] == "queued" {
            self.drain();
        }
        self.publish();
        json_value(&answer)
    }

    // One /config at a time: each rewrites the whole file, so two at once can drop a change.
    fn drain(self: &Rc<Self>) {
        let (generation, params, read) = {
            let mut p = self.panel.borrow_mut();
            let Some(command) = p.state.next_write() else {
                return;
            };
            let mut params = p.params.clone();
            params["command"] = json!(command);
            params["connectMcp"] = json!(false);
            (p.generation, params, p.params.clone())
        };
        let me = self.clone();
        engine::get().spawn(async move {
            let reply = call_as::<LocalCommandOutput>("RunClaudeLocalCommand", params).await;
            let snapshot = if ConfigState::write_saved(&reply) {
                Some(call_as::<ClaudeConfigSnapshot>("ReadClaudeConfig", read).await)
            } else {
                None
            };
            if !me.current(generation) {
                return;
            }
            me.panel.borrow_mut().state.write_done(reply, snapshot);
            me.publish();
            me.drain();
        });
    }
}

fn row_json(row: &RowView, state: &ConfigState) -> Json {
    let id = row.setting.id;
    let kind = if row.is_bool() {
        "bool"
    } else if row.is_text() {
        "text"
    } else {
        "enum"
    };
    json!({
        "id": id,
        "label": row.setting.label,
        "value": row.value,
        "options": row.options,
        "writable": row.writable,
        "note": row.note,
        "noted": state.notes.contains_key(id),
        "kind": kind,
        "tag": row.tag(),
        "chip": row.setting.chip.is_some(),
        "changed": state.changed(id),
    })
}

fn stats_json(summary: &UsageSummary) -> Json {
    let Some(stats) = stats_of(summary) else {
        return json!({ "state": "empty" });
    };
    let max = stats.days.iter().copied().max().unwrap_or(0).max(1);
    let weeks: Vec<Vec<u8>> = stats.days.chunks(7).map(|week| week.iter().map(|t| heat_level(*t, max)).collect()).collect();
    let favorite = stats.favorite_model.as_deref().map(|id| {
        crate::chat::catalog_model(HarnessId::ClaudeCode.as_str(), id)
            .and_then(|m| m["label"].as_str().map(str::to_owned))
            .unwrap_or_else(|| id.to_owned())
    });
    json!({
        "state": "ready",
        "weeks": weeks,
        "favorite": favorite,
        "sessions": stats.sessions,
        "active_days": stats.active_days,
        "streak": stats.streak,
        "tokens": format_tokens(stats.tokens),
    })
}

// The composer's picks as the proto's chips: a model is `harness/id` there, access a short word.
struct Chips {
    choices: ChipChoices,
    values: Vec<String>,
}

const ACCESS: [(&str, AccessMode); 4] = [
    ("supervised", AccessMode::Supervised),
    ("edits", AccessMode::AutoAcceptEdits),
    ("auto", AccessMode::AutoReview),
    ("full", AccessMode::FullAccess),
];

fn bare(value: &str) -> &str {
    value.split_once('/').map_or(value, |(_, id)| id)
}

fn level_word(level: ReasoningLevel) -> String {
    serde_json::to_value(level).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

impl Chips {
    fn of(chips: Option<&Value>) -> Self {
        let Some(chips) = chips else {
            return Self { choices: ChipChoices::default(), values: Vec::new() };
        };
        let mut values = Vec::new();
        let mut models = Vec::new();
        for m in chips.field("models").map(Value::list).unwrap_or_default() {
            let value = m.field("value").map(text).unwrap_or_default();
            let label = m.field("label").map(text).unwrap_or_default();
            if value.is_empty() {
                continue;
            }
            models.push((bare(&value).to_owned(), if label.is_empty() { bare(&value).to_owned() } else { label }));
            values.push(value);
        }
        let efforts = chips
            .field("efforts")
            .map(Value::list)
            .unwrap_or_default()
            .iter()
            .filter_map(|e| serde_json::from_value::<ReasoningLevel>(json!(text(e))).ok())
            .collect();
        let word = |key: &str| chips.field(key).map(text).filter(|v| !v.is_empty());
        let access = word("access").unwrap_or_default();
        Self {
            choices: ChipChoices {
                models,
                model: word("model").map(|m| bare(&m).to_owned()),
                efforts,
                effort: word("effort").and_then(|e| serde_json::from_value(json!(e)).ok()),
                permission: ACCESS.iter().find(|(w, _)| *w == access).map_or(AccessMode::FullAccess, |(_, m)| *m),
            },
            values,
        }
    }

    fn value_of(&self, chip: cc::Chip, label: &str) -> (&'static str, String) {
        match chip {
            cc::Chip::Model => {
                let at = self.choices.models.iter().position(|(_, l)| l == label);
                ("model", at.and_then(|i| self.values.get(i).cloned()).unwrap_or_default())
            }
            cc::Chip::Effort => {
                let level = self.choices.efforts.iter().find(|l| l.label() == label);
                ("effort", level.map(|l| level_word(*l)).unwrap_or_default())
            }
            cc::Chip::Permission => {
                let word = ACCESS.iter().find(|(_, m)| m.label() == label).map_or("full", |(w, _)| *w);
                ("access", word.to_owned())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chips() -> Chips {
        Chips::of(Some(&json_value(&json!({
            "models": [{ "value": "claude-code/claude-opus", "label": "Opus" }, { "value": "claude-code/claude-fable", "label": "Fable" }],
            "model": "claude-code/claude-fable",
            "efforts": ["low", "xhigh"],
            "effort": "xhigh",
            "access": "edits",
        }))))
    }

    #[test]
    fn composer_picks_read_as_proto_chips_and_a_chip_choice_goes_back_as_a_composer_value() {
        let chips = chips();
        assert_eq!(chips.choices.model.as_deref(), Some("claude-fable"));
        assert_eq!(chips.choices.model_label(), "Fable");
        assert_eq!(chips.choices.effort, Some(ReasoningLevel::XHigh));
        assert_eq!(chips.choices.permission, AccessMode::AutoAcceptEdits);
        assert_eq!(chips.value_of(cc::Chip::Model, "Opus"), ("model", "claude-code/claude-opus".to_string()));
        assert_eq!(chips.value_of(cc::Chip::Effort, ReasoningLevel::Low.label()), ("effort", "low".to_string()));
        assert_eq!(chips.value_of(cc::Chip::Permission, AccessMode::AutoReview.label()), ("access", "auto".to_string()));
    }

    #[test]
    fn rows_carry_their_kind_tag_and_whether_a_written_note_stands() {
        let mut state = ConfigState::default();
        state.snapshot_arrived(Ok(ClaudeConfigSnapshot { values: [("autoCompact".to_string(), "true".to_string())].into(), ..Default::default() }));
        let chips = chips();
        let rows = state.rows(&chips.choices);
        let by = |id: &str| row_json(rows.iter().find(|r| r.setting.id == id).unwrap(), &state);
        let auto = by("autoCompact");
        assert_eq!((auto["kind"].as_str(), auto["tag"].is_null(), auto["noted"].as_bool()), (Some("bool"), true, Some(false)));
        let model = by("model");
        assert_eq!((model["kind"].as_str(), model["tag"].as_str(), model["value"].as_str()), (Some("enum"), Some("= chip"), Some("Fable")));
        assert_eq!(model["options"], json!(["Opus", "Fable"]));
    }
}
