use std::collections::HashMap;
use std::sync::Arc;

use zeron_doc::parts::{ClassifierRecord, ClassifierVerdict, DecidedBy, HandoffKind, HandoffRoute, MessagePart};
use zeron_doc::schema::SessionMessageEntry;
use zeron_proto::transcript as words;
use zeron_proto::{ChildRoute, DelegationStatus, HarnessId};

use super::display::ColorRole;
use super::markdown::Ctx;
use super::rows::{Content, RowCore, RowKind, next_version, row_key};
use super::strip::{Align, Block, Faces, Item, Rule, Strip, chat_link, linked, struck};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChatRef {
    pub title: Option<String>,
    pub harness: Option<HarnessId>,
    pub model: Option<String>,
    pub delegated_by: Option<String>,
    pub status: Option<DelegationStatus>,
    pub elapsed_secs: Option<i64>,
    pub route: Option<ChildRoute>,
}

impl ChatRef {
    pub fn of(chat: &zeron_proto::Chat) -> Self {
        let delegation = chat.delegation.as_ref();
        Self {
            title: chat.title.clone(),
            harness: chat.config.as_ref().map(|c| c.harness),
            model: chat.config.as_ref().and_then(|c| c.model.clone()),
            delegated_by: chat.delegated_by().map(str::to_owned),
            status: delegation.map(|d| d.status),
            elapsed_secs: delegation.and_then(|d| Some((d.completed_at? - d.started_at).num_seconds())),
            route: delegation.and_then(|d| d.route.clone()),
        }
    }
}

pub type Chats = HashMap<String, ChatRef>;

pub(crate) fn single_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn user_text(entry: &SessionMessageEntry) -> String {
    entry
        .parts
        .iter()
        .filter_map(|p| match p {
            MessagePart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn entry_refs(entry: &SessionMessageEntry) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(from) = &entry.from_chat_id {
        ids.push(from.clone());
        let text = user_text(entry);
        ids.extend(words::wake_task_ids(&text).unwrap_or_default());
        ids.extend(words::step_handback_threads(&text));
    }
    for part in &entry.parts {
        if let MessagePart::Tool { call, subagent_ref: Some(child), .. } = part
            && (call.is_delegate_task() || call.is_create_thread())
        {
            ids.push(child.clone());
        }
    }
    ids
}

fn wire(harness: HarnessId) -> String {
    serde_json::to_value(harness).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

pub(crate) fn brand_icon(harness: Option<HarnessId>, model: Option<&str>) -> String {
    HarnessId::brand_of(harness, model)
        .map(|brand| format!("mark-{}", wire(brand)))
        .filter(|name| name != "mark-")
        .unwrap_or_else(|| "tool-bot".to_owned())
}

pub(crate) fn model_label(harness: Option<HarnessId>, model: &str) -> String {
    match harness {
        Some(harness) => zeron_client::catalog::model_label(&wire(harness), model),
        None => model.to_owned(),
    }
}

fn side_detail(harness: HarnessId, model: Option<&str>) -> String {
    match model {
        Some(model) => format!("{} · {}", harness.display_name(), model_label(Some(harness), model)),
        None => harness.display_name().to_owned(),
    }
}

fn side_items(faces: &Faces, ctx: &mut Ctx, harness: HarnessId, model: Option<&str>, color: ColorRole) -> [Item; 2] {
    let label = match model {
        Some(model) => model_label(Some(harness), model),
        None => harness.display_name().to_owned(),
    };
    [
        Item::Icon {
            name: brand_icon(Some(harness), model),
            color: ColorRole::TextSecondary,
        },
        Item::Text(faces.text(ctx, &label, color)),
    ]
}

fn core(key: String, entry_id: &str, strip: Strip, copy: String) -> RowCore {
    RowCore {
        key: row_key(&key),
        version: next_version(),
        kind: RowKind::Chip,
        entry_id: Arc::from(entry_id),
        content: Content::Strip(strip),
        copy_text: copy,
    }
}

pub(crate) fn divider(ctx: &mut Ctx, key: String, entry_id: &str, text: &str, hint: Option<&str>) -> RowCore {
    let faces = Faces::new(ctx);
    let mut items = vec![Item::Text(faces.text(ctx, text, ColorRole::TextFaint))];
    if let Some(hint) = hint.filter(|h| !h.trim().is_empty()) {
        items.push(Item::Hint {
            title: text.to_owned(),
            text: hint.to_owned(),
        });
    }
    let strip = Strip {
        items,
        align: Align::Center,
        rule: Rule::Around,
        open: None,
        card: Vec::new(),
        after: Vec::new(),
    };
    core(key, entry_id, strip, text.to_owned())
}

pub(crate) fn peer_header(ctx: &mut Ctx, entry: &SessionMessageEntry, senders: &[(String, String)]) -> RowCore {
    let faces = Faces::new(ctx);
    let mut items = vec![
        Item::Icon {
            name: "tool-chat-round-line".into(),
            color: ColorRole::TextFaint,
        },
        Item::Text(faces.text(ctx, "From", ColorRole::TextSecondary)),
    ];
    for (n, (id, title)) in senders.iter().enumerate() {
        let title = if n + 1 < senders.len() { format!("{title},") } else { title.clone() };
        items.push(Item::Text(linked(faces.medium(ctx, &title, ColorRole::TextSoft), chat_link(id))));
    }
    if let Some(clock) = clock(entry.created_at) {
        items.push(Item::Text(faces.mono(ctx, &clock, ColorRole::TextFaint)));
    }
    let copy = format!("From {}", senders.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(", "));
    let strip = Strip {
        items,
        align: Align::Leading,
        rule: Rule::After,
        open: None,
        card: Vec::new(),
        after: Vec::new(),
    };
    core(format!("{}#from", entry.id), &entry.id, strip, copy)
}

fn clock(ms: i64) -> Option<String> {
    let utc = chrono::DateTime::from_timestamp_millis(ms).filter(|_| ms > 0)?;
    Some(utc.with_timezone(&chrono::Local).format("%-I:%M %p").to_string())
}

pub(crate) fn auto_continue(ctx: &mut Ctx, entry_id: &str) -> RowCore {
    let faces = Faces::new(ctx);
    let strip = Strip {
        items: vec![
            Item::Icon {
                name: "arrow.clockwise".into(),
                color: ColorRole::TextFaint,
            },
            Item::Text(faces.text(ctx, "Auto-continue", ColorRole::TextSecondary)),
        ],
        align: Align::Trailing,
        rule: Rule::None,
        open: None,
        card: Vec::new(),
        after: Vec::new(),
    };
    core(format!("{entry_id}#auto"), entry_id, strip, String::new())
}

pub(crate) fn wake_row(ctx: &mut Ctx, entry_id: &str, ids: &[String], chats: &Chats) -> RowCore {
    let faces = Faces::new(ctx);
    let views: Vec<(String, Option<DelegationStatus>)> = ids
        .iter()
        .map(|id| {
            let chat = chats.get(id);
            let title = chat.and_then(|c| c.title.as_deref()).map(single_line).unwrap_or_else(|| "Delegated task".into());
            (title, chat.and_then(|c| c.status))
        })
        .collect();
    let tasks: Vec<(&str, Option<DelegationStatus>)> = views.iter().map(|(t, s)| (t.as_str(), *s)).collect();
    let summary = words::wake_summary(&tasks);
    let failed = tasks.iter().any(|(_, s)| *s == Some(DelegationStatus::Failed));
    let color = if failed { ColorRole::Danger } else { ColorRole::TextSecondary };
    let icon = match ids {
        [one] => {
            let chat = chats.get(one);
            brand_icon(chat.and_then(|c| c.harness), chat.and_then(|c| c.model.as_deref()))
        }
        _ => "tool-bot".to_owned(),
    };
    let mut items = vec![
        Item::Icon { name: icon, color },
        Item::Text(faces.medium(ctx, &summary, color)),
    ];
    if let [one] = ids {
        items.push(Item::Pill {
            label: faces.medium(ctx, "Open subagent", ColorRole::Text),
            url: chat_link(one),
        });
    }
    let strip = Strip {
        items,
        align: Align::Leading,
        rule: Rule::After,
        open: None,
        card: Vec::new(),
        after: Vec::new(),
    };
    core(format!("{entry_id}#wake"), entry_id, strip, summary)
}

pub(crate) fn started_thread(ctx: &mut Ctx, key: String, entry_id: &str, chat_id: &str, title: &str) -> RowCore {
    let faces = Faces::new(ctx);
    let link = chat_link(chat_id);
    let strip = Strip {
        items: vec![
            Item::Icon {
                name: "tool-chat-round-line".into(),
                color: ColorRole::TextFaint,
            },
            Item::Text(faces.text(ctx, "Started thread", ColorRole::TextSecondary)),
            Item::Text(linked(faces.medium(ctx, title, ColorRole::TextSoft), link.clone())),
            Item::Text(faces.text(ctx, "·", ColorRole::TextSecondary)),
            Item::Text(linked(faces.text(ctx, "Open", ColorRole::TextSecondary), link)),
        ],
        align: Align::Leading,
        rule: Rule::After,
        open: None,
        card: Vec::new(),
        after: Vec::new(),
    };
    core(key, entry_id, strip, format!("Started thread {title}"))
}

fn mono_block(faces: &Faces, ctx: &mut Ctx, text: &str, title: &str, accent: bool) -> Block {
    let head: String = text.lines().take(40).collect::<Vec<_>>().join("\n");
    Block::Mono {
        text: faces.mono(ctx, &head, ColorRole::TextFaint),
        full: text.to_owned(),
        title: title.to_owned(),
        accent,
    }
}

fn field(faces: &Faces, ctx: &mut Ctx, key: &str, value: &str) -> Block {
    Block::Field(faces.text(ctx, key, ColorRole::TextFaint), faces.text(ctx, value, ColorRole::Text))
}

pub(crate) fn handoff(ctx: &mut Ctx, key: String, entry_id: &str, harness: HarnessId, text: &str, route: &HandoffRoute, open: bool) -> RowCore {
    let faces = Faces::new(ctx);
    let pointer = route.kind == HandoffKind::Pointer;
    let mut label = match &route.source_title {
        Some(title) => format!("Continued from {}", single_line(title)),
        None => "Context handoff".to_owned(),
    };
    if pointer {
        label.push_str(&format!(" · handed over as a link · {}", if open { "hide" } else { "view" }));
    }
    let mut items = vec![
        Item::Icon {
            name: "arrow.left.arrow.right".into(),
            color: ColorRole::TextFaint,
        },
        Item::Text(faces.text(ctx, &label, ColorRole::TextFaint)),
    ];
    if let Some(from) = route.from_harness {
        items.extend(side_items(&faces, ctx, from, route.from_model.as_deref(), ColorRole::TextFaint));
    }
    items.push(Item::Icon {
        name: "arrow.right".into(),
        color: ColorRole::TextFaint,
    });
    items.extend(side_items(&faces, ctx, harness, route.to_model.as_deref(), ColorRole::TextFaint));
    let (mut card, mut after) = (Vec::new(), Vec::new());
    if open {
        if let Some(from) = route.from_harness {
            card.push(field(&faces, ctx, "From", &side_detail(from, route.from_model.as_deref())));
        }
        card.push(field(&faces, ctx, "To", &side_detail(harness, route.to_model.as_deref())));
        card.push(field(&faces, ctx, "Context", &words::handoff_context_label(text, pointer)));
        if !text.is_empty() {
            after.push(Block::Caption(faces.text(ctx, "Transferred context", ColorRole::TextSecondary)));
            after.push(mono_block(&faces, ctx, text, "Transferred context", false));
            after.push(Block::Note(faces.text(ctx, words::handoff_footnote(pointer), ColorRole::TextFaint)));
        }
    }
    let strip = Strip {
        items,
        align: Align::Center,
        rule: Rule::Around,
        open: Some(open),
        card,
        after,
    };
    core(key, entry_id, strip, label)
}

pub(crate) fn classifier(ctx: &mut Ctx, key: String, entry_id: &str, record: &ClassifierRecord, open: bool) -> RowCore {
    let faces = Faces::new(ctx);
    let label = ClassifierVerdict::label;
    let verdict_color = match record.verdict {
        ClassifierVerdict::New => ColorRole::Accent,
        ClassifierVerdict::Carry => ColorRole::Success,
        ClassifierVerdict::Continue => ColorRole::TextFaint,
        ClassifierVerdict::Failed => ColorRole::Warning,
    };
    let verdict = match record.classifier_verdict {
        Some(said) => format!("{} (said {})", label(record.verdict), label(said)),
        None => label(record.verdict).to_owned(),
    };
    let overturned = record.decided_by == DecidedBy::Stage2 && record.stage2_reply.is_some() && record.verdict != ClassifierVerdict::Continue;
    let secs = format!("{:.1}s", record.duration_ms as f64 / 1000.0);
    let mut items = vec![
        Item::Icon {
            name: "line.3.horizontal.decrease".into(),
            color: ColorRole::TextFaint,
        },
        Item::Text(faces.text(ctx, "Classifier", ColorRole::TextFaint)),
    ];
    match (record.harness, record.model.as_deref()) {
        (Some(harness), model) => items.extend(side_items(&faces, ctx, harness, model, ColorRole::TextFaint)),
        (None, Some(model)) => items.push(Item::Text(faces.text(ctx, model, ColorRole::TextFaint))),
        (None, None) => {}
    }
    items.push(Item::Text(faces.text(ctx, &format!("· {secs} ·"), ColorRole::TextFaint)));
    if overturned {
        items.push(Item::Text(struck(faces.text(ctx, label(ClassifierVerdict::Continue), ColorRole::TextFaint))));
        items.push(Item::Text(faces.text(ctx, "→", ColorRole::TextFaint)));
    }
    items.push(Item::Text(faces.text(ctx, &verdict, verdict_color)));
    if overturned {
        items.push(Item::Text(faces.text(ctx, "second look", ColorRole::Accent)));
    }
    items.push(Item::Chevron(open));
    let (mut card, mut after) = (Vec::new(), Vec::new());
    if open {
        match (record.harness, record.model.as_deref()) {
            (Some(harness), model) => card.push(field(&faces, ctx, "Model", &side_detail(harness, model))),
            (None, Some(model)) => card.push(field(&faces, ctx, "Model", model)),
            (None, None) => {}
        }
        card.push(field(&faces, ctx, "Verdict", &verdict));
        card.push(field(&faces, ctx, "Decided", record.decided_by.label()));
        card.push(field(&faces, ctx, "Time", &secs));
        card.push(field(&faces, ctx, "Context", &format!("{} tokens", words::compact_count(record.context_tokens))));
        if let (Some(input), Some(output)) = (record.input_tokens, record.output_tokens) {
            card.push(field(&faces, ctx, "Tokens", &format!("{} in · {} out", words::compact_count(input), words::compact_count(output))));
        }
        let sent = if matches!(record.decided_by, DecidedBy::Stage1 | DecidedBy::Stage2) {
            "Sent to the classifier"
        } else {
            "Not sent: decided without asking"
        };
        after.push(Block::Caption(faces.text(ctx, sent, ColorRole::TextSecondary)));
        after.push(mono_block(&faces, ctx, &record.input, sent, false));
        match record.stage2_reply.as_deref() {
            None => {
                after.push(Block::Caption(faces.text(ctx, "Reply", ColorRole::TextSecondary)));
                after.push(mono_block(&faces, ctx, &record.reply, "Reply", false));
            }
            Some(stage2) => {
                for (title, text, decides) in [
                    ("First look", record.reply.as_str(), record.decided_by == DecidedBy::Stage1),
                    ("Second look", stage2, record.decided_by == DecidedBy::Stage2),
                ] {
                    let caption = if decides { format!("{title} · decides") } else { title.to_owned() };
                    after.push(Block::Caption(faces.text(ctx, &caption, ColorRole::TextSecondary)));
                    after.push(mono_block(&faces, ctx, text, title, decides));
                }
            }
        }
    }
    let strip = Strip {
        items,
        align: Align::Center,
        rule: Rule::Around,
        open: Some(open),
        card,
        after,
    };
    core(key, entry_id, strip, format!("Classifier · {secs} · {verdict}"))
}

pub(crate) fn route_badge(route: &ChildRoute) -> Option<(String, ColorRole)> {
    let short = || {
        let short = words::route_short_reason(route);
        match route.resets_at.and_then(clock) {
            Some(at) => format!("{short}, resets {at}"),
            None => short,
        }
    };
    match route.source.as_str() {
        ChildRoute::ROUTED => Some(("routed".into(), ColorRole::Accent)),
        ChildRoute::CHOSEN_BY_AGENT => Some(("chosen by agent".into(), ColorRole::Warning)),
        ChildRoute::FELL_BACK => Some((format!("fell back ({})", short()), ColorRole::Cyan)),
        _ => None,
    }
}

pub(crate) fn route_detail(route: &ChildRoute, ran: Option<(HarnessId, Option<&str>)>, parent: Option<(HarnessId, Option<&str>)>, native: bool) -> (String, String) {
    let role = route.role.clone().unwrap_or_else(|| "general".into());
    let quota = route.reason.as_deref().is_some_and(|r| r.ends_with(" out of quota"));
    let target = |t: &zeron_proto::RouteTarget| side_detail(t.harness, t.model.as_deref());
    let same = |a: Option<(HarnessId, Option<&str>)>, b: Option<(HarnessId, Option<&str>)>| a.is_some() && a == b;
    let mut ran_as = ran.map(|(harness, model)| {
        if native {
            match model {
                Some(model) => format!("{} native sub-agent, {}", harness.display_name(), model_label(Some(harness), model)),
                None => format!("{} native sub-agent", harness.display_name()),
            }
        } else {
            side_detail(harness, model)
        }
    });
    if route.source == ChildRoute::FELL_BACK
        && (native || same(ran, parent))
        && let Some(text) = ran_as.as_mut()
    {
        text.push_str(" (this chat's model)");
    }
    let mut rows: Vec<String> = Vec::new();
    let title = match route.source.as_str() {
        ChildRoute::CHOSEN_BY_AGENT => {
            rows.push(format!("Role: {role}"));
            rows.extend(ran_as.map(|r| format!("Runs on: {r}")));
            rows.extend(route.table.as_ref().map(|t| format!("Route was: {}", target(t))));
            rows.push(format!("{}, which wins over the route.", route.reason.as_deref().unwrap_or("The lead passed model")));
            "Chosen by agent".to_owned()
        }
        ChildRoute::FELL_BACK => {
            rows.extend(route.table.as_ref().map(|t| format!("Route: {}", target(t))));
            if quota {
                rows.push(match route.resets_at.and_then(clock) {
                    Some(at) => format!("Quota: Used up · resets {at}"),
                    None => "Quota: Used up".to_owned(),
                });
            }
            rows.extend(ran_as.map(|r| format!("Ran as: {r}")));
            let rule = if !quota {
                "Harness not available: runs on this chat's model"
            } else if native {
                "When out of quota: Native sub-agent"
            } else if same(ran, parent) {
                "When out of quota: Native sub-agent, on this chat's model"
            } else {
                "When out of quota: Next in fallback list"
            };
            rows.push(format!("Rule: {rule}"));
            if quota && let (Some(at), Some(table)) = (route.resets_at.and_then(clock), route.table.as_ref()) {
                rows.push(format!("After {at}, {role} routes to {} again.", table.harness.display_name()));
            }
            format!("Fell back: {}", route.reason.as_deref().unwrap_or("route unavailable"))
        }
        _ => {
            rows.push(format!("Role: {role}"));
            rows.extend(ran_as.map(|r| format!("Runs on: {r}")));
            rows.push("Scope: Global default".to_owned());
            rows.push("Override: pass model in delegate_task.".to_owned());
            "Routed by Settings > Routing".to_owned()
        }
    };
    (title, rows.join("\n"))
}

pub(crate) fn sender_title(chats: &Chats, id: &str) -> String {
    chats.get(id).and_then(|c| c.title.as_deref()).map(single_line).filter(|t| !t.is_empty()).unwrap_or_else(|| id.to_owned())
}
