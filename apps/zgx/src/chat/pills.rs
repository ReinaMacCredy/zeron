use super::*;

const LOOK_TRIES: u32 = 10;
const LOOK_RETRY: std::time::Duration = std::time::Duration::from_millis(400);
const MAX_STEPS: usize = 8;

pub(super) struct Waiting {
    pub(super) request: String,
    questions: Vec<Json>,
    part: Json,
    steps: Vec<String>,
    answered: bool,
}

#[derive(Default)]
pub(super) struct Wizard {
    request: String,
    pages: Vec<Page>,
}

struct Page {
    question: String,
    labels: Vec<String>,
    typed: bool,
}

struct Pills {
    roster_usage: Shared,
    shell_output: Shared,
    rpc: RefCell<Option<Rpc>>,
    docs: RefCell<HashMap<String, DocLook>>,
    shells: RefCell<HashMap<String, ShellLook>>,
    wanted_docs: RefCell<Vec<String>>,
    wanted_shells: RefCell<Vec<(String, String)>>,
}

struct DocLook {
    _watch: Option<Watch>,
    entries: Vec<Entry>,
    tokens: Option<f64>,
    spawns: Vec<Value>,
    doing: Option<String>,
    error: Option<String>,
}

struct ShellLook {
    _watch: Option<Watch>,
    lines: Vec<String>,
}

pub(super) fn install(host: &ScriptHost) {
    let chats = CHATS.with(Rc::clone);
    chats.waiting_shared.borrow_mut().push(host.shared("chat.waiting", chats.waiting_value()));
    let pills = Rc::new(Pills {
        roster_usage: host.shared("chat.roster_usage", Value::record(Vec::<(&str, Value)>::new())),
        shell_output: host.shared("chat.shell_output", Value::record(Vec::<(&str, Value)>::new())),
        rpc: RefCell::default(),
        docs: RefCell::default(),
        shells: RefCell::default(),
        wanted_docs: RefCell::default(),
        wanted_shells: RefCell::default(),
    });
    let p = pills.clone();
    host.action("chat.roster_watch", move |args| {
        let docs = args.field("docs").map(Value::list).unwrap_or_default().iter().map(text).filter(|d| !d.is_empty()).collect();
        p.follow_docs(docs);
    });
    let p = pills.clone();
    host.action("chat.shell_watch", move |args| {
        let wanted = args
            .field("shells")
            .map(Value::list)
            .unwrap_or_default()
            .iter()
            .filter_map(|s| Some((s.field("session").map(text)?, s.field("task").map(text)?)))
            .filter(|(session, task)| !session.is_empty() && !task.is_empty())
            .collect();
        p.follow_shells(wanted);
    });
    let weak = Rc::downgrade(&pills);
    engine::get().on_connect(move |rpc| {
        let Some(p) = weak.upgrade() else { return };
        *p.rpc.borrow_mut() = Some(rpc.clone());
        let docs = p.wanted_docs.borrow().clone();
        p.docs.borrow_mut().clear();
        p.follow_docs(docs);
        let shells = p.wanted_shells.borrow().clone();
        p.shells.borrow_mut().clear();
        p.follow_shells(shells);
    });
    host.async_action("chat.cancel_delegate", move |args| async move { cancel_delegate(args).await.map_err(String::from) });
    host.async_action("chat.waiting_answer", move |args| {
        async move {
            crate::plugins::preview::refuse("chat.respond")?;
            let chats = CHATS.with(Rc::clone);
            chats.waiting_answer(args).await.map(|_| Value::Nil).map_err(String::from)
        }
    });
    host.async_action("chat.waiting_back", move |args| async move {
        let chats = CHATS.with(Rc::clone);
        Ok::<Value, String>(chats.page_back(&args))
    });
}

async fn cancel_delegate(args: Value) -> Result<Value, Outcome> {
    let id = args.field("session").map(text).unwrap_or_default();
    let task = args.field("task").map(text).unwrap_or_default();
    if id.is_empty() || task.is_empty() {
        return Err("missing");
    }
    engine::get()
        .call("TaskCancel", json!({ "chatId": id, "taskId": task }))
        .await
        .map_err(|e| engine::outcome(&e))?;
    Ok(Value::Bool(true))
}

impl Pills {
    fn follow_docs(self: &Rc<Self>, docs: Vec<String>) {
        self.docs.borrow_mut().retain(|doc, _| docs.contains(doc));
        let rpc = self.rpc.borrow().clone().filter(|r| !r.is_closed());
        if let Some(rpc) = &rpc {
            for doc in &docs {
                if self.docs.borrow().contains_key(doc) {
                    continue;
                }
                let weak = Rc::downgrade(self);
                let heard = doc.clone();
                let watch = rpc.watch("WatchDocMessages", json!({ "chatId": doc, "openingTail": true }), move |frame| {
                    if let Some(me) = weak.upgrade() {
                        me.heard_doc(&heard, frame);
                    }
                });
                self.docs.borrow_mut().insert(doc.clone(), DocLook { _watch: Some(watch), entries: Vec::new(), tokens: None, spawns: Vec::new(), doing: None, error: None });
            }
        }
        *self.wanted_docs.borrow_mut() = docs;
        self.publish_usage();
    }

    fn heard_doc(&self, doc: &str, mut frame: Json) {
        let moved = {
            let mut docs = self.docs.borrow_mut();
            let Some(look) = docs.get_mut(doc) else { return };
            let tokens = frame.get("contextUsage").and_then(|u| u["tokens"].as_f64()).or(look.tokens);
            let mut moved = tokens != look.tokens;
            look.tokens = tokens;
            if apply(&mut look.entries, &mut frame).is_ok() {
                let spawns = spawns_of(&look.entries);
                if spawns != look.spawns {
                    look.spawns = spawns;
                    moved = true;
                }
                let (doing, error) = (doing_of(&look.entries), error_of(&look.entries));
                if doing != look.doing || error != look.error {
                    (look.doing, look.error) = (doing, error);
                    moved = true;
                }
            }
            moved
        };
        if moved {
            self.publish_usage();
        }
    }

    fn publish_usage(&self) {
        let docs = self.docs.borrow();
        self.roster_usage.set(usage_value(docs.iter().map(|(doc, look)| (doc.as_str(), look))));
    }

    fn follow_shells(self: &Rc<Self>, wanted: Vec<(String, String)>) {
        self.shells.borrow_mut().retain(|task, _| wanted.iter().any(|(_, t)| t == task));
        let rpc = self.rpc.borrow().clone().filter(|r| !r.is_closed());
        if let Some(rpc) = &rpc {
            for (session, task) in &wanted {
                if self.shells.borrow().contains_key(task) {
                    continue;
                }
                let target = CHATS.with(|c| c.chat(session).and_then(|chat| c.target(&chat)));
                let mut params = json!({ "chatId": session, "taskId": task });
                if let Some(t) = &target {
                    params["targetDeviceId"] = json!(t);
                }
                let weak = Rc::downgrade(self);
                let heard = task.clone();
                let watch = rpc.watch("WatchBackgroundOutput", params, move |frame| {
                    let Some(me) = weak.upgrade() else { return };
                    let lines: Vec<String> = frame["lines"].as_array().into_iter().flatten().filter_map(Json::as_str).map(str::to_owned).collect();
                    if let Some(look) = me.shells.borrow_mut().get_mut(&heard) {
                        look.lines = lines;
                    }
                    me.publish_shells();
                });
                self.shells.borrow_mut().insert(task.clone(), ShellLook { _watch: Some(watch), lines: Vec::new() });
            }
        }
        *self.wanted_shells.borrow_mut() = wanted;
        self.publish_shells();
    }

    fn publish_shells(&self) {
        let shells = self.shells.borrow();
        self.shell_output.set(Value::record(shells.iter().map(|(task, look)| {
            let lines: Vec<Value> = look.lines.iter().map(|l| Value::from(l.as_str())).collect();
            (task.as_str(), Value::record([("lines", Value::List(lines.into()))]))
        })));
    }
}

fn usage_value<'a>(docs: impl Iterator<Item = (&'a str, &'a DocLook)>) -> Value {
    let words = |w: &Option<String>| w.as_deref().map_or(Value::Nil, Value::from);
    Value::record(docs.map(|(doc, look)| {
        (
            doc,
            Value::record([
                ("tokens", look.tokens.map_or(Value::Nil, Value::Num)),
                ("spawns", Value::List(look.spawns.clone().into())),
                ("doing", words(&look.doing)),
                ("error", words(&look.error)),
            ]),
        )
    }))
}

fn newest(entries: &[Entry]) -> Option<&Json> {
    entries.iter().rev().find_map(|e| e.json.as_ref())
}

fn doing_of(entries: &[Entry]) -> Option<String> {
    let last = newest(entries).filter(|j| j["role"].as_str() == Some("assistant"))?;
    let row = last["parts"].as_array()?.iter().rev().filter(|p| p["kind"].as_str() == Some("tool")).find_map(tool)?;
    let verb = row.field("verb").map(text)?;
    Some(match row.field("target").map(text) {
        Some(target) => format!("{verb} {target}"),
        None => verb,
    })
}

fn error_of(entries: &[Entry]) -> Option<String> {
    let last = newest(entries)?;
    let part = last["parts"].as_array()?.last().filter(|p| p["kind"].as_str() == Some("error"))?;
    part["message"].as_str().map(single_line).filter(|m| !m.is_empty())
}

fn spawns_of(entries: &[Entry]) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for entry in entries {
        let Some(json) = &entry.json else { continue };
        let at = json["createdAt"].as_f64().map_or(Value::Nil, |ms| Value::Num(ms / 1000.));
        for part in json["parts"].as_array().into_iter().flatten() {
            if !part["subagentRef"].is_string() {
                continue;
            }
            let Some(row) = tool(part) else { continue };
            let Some(sub) = row.field("subagent") else { continue };
            let Some(Value::Str(reference)) = sub.field("ref") else { continue };
            if seen.insert(reference.clone()) {
                out.push(subagent_row(sub, &row, at.clone(), true));
            }
        }
    }
    out
}

pub(super) fn subagent_row(sub: &Value, tool: &Value, at: Value, recent: bool) -> Value {
    let field = |key: &str| sub.field(key).cloned().unwrap_or(Value::Nil);
    Value::record([
        ("ref", field("ref")),
        ("call", tool.field("id").cloned().unwrap_or(Value::Nil)),
        ("title", field("title")),
        ("status", field("status")),
        ("model", field("model")),
        ("kind", field("kind")),
        ("agent", field("agent")),
        ("reasoning", field("reasoning")),
        ("at", at),
        ("recent", Value::Bool(recent)),
    ])
}

fn is_plan(questions: &[Json]) -> bool {
    questions.iter().any(|q| q["approval"].as_bool() == Some(true) && q["header"].as_str() == Some(zeron_proto::PLAN_APPROVAL_HEADER))
}

fn list_item(line: &str) -> Option<&str> {
    if line.starts_with([' ', '\t']) {
        return None;
    }
    let rest = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).or_else(|| line.strip_prefix("+ "));
    if rest.is_some() {
        return rest;
    }
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    (digits > 0).then(|| line[digits..].strip_prefix(". ").or_else(|| line[digits..].strip_prefix(") "))).flatten()
}

fn clean(line: &str) -> String {
    line.replace("**", "").replace('`', "").trim().to_owned()
}

fn plan_steps(plan: &str) -> Vec<String> {
    let items: Vec<String> = plan.lines().filter_map(list_item).map(clean).filter(|s| !s.is_empty()).collect();
    let steps: Vec<String> = if items.is_empty() {
        plan.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(clean).filter(|s| !s.is_empty()).collect()
    } else {
        items
    };
    steps.into_iter().take(MAX_STEPS).collect()
}

fn proposed_steps(last: &Json) -> Vec<String> {
    last["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .find_map(|p| {
            (p["kind"].as_str() == Some("tool") && p["call"]["name"].as_str() == Some("ExitPlanMode"))
                .then(|| p["call"]["input"]["plan"].as_str())
                .flatten()
        })
        .map(plan_steps)
        .unwrap_or_default()
}

impl Chats {
    pub(super) fn waiting_value(&self) -> Value {
        let chats = self.chats.borrow();
        let waiting = self.waiting.borrow();
        let mut rows: Vec<(&String, &Waiting)> = waiting.iter().filter(|(_, w)| !w.answered).collect();
        let updated = |id: &String| chats.get(id).and_then(|chat| chat["lastMessageAt"].as_str().or(chat["createdAt"].as_str()).map(str::to_owned));
        rows.sort_by(|a, b| updated(b.0).cmp(&updated(a.0)).then_with(|| a.0.cmp(b.0)));
        let list: Vec<Value> = rows
            .into_iter()
            .filter_map(|(id, w)| {
                let chat = chats.get(id);
                if chat.is_none() && self.chats_heard.get() {
                    return None;
                }
                let ask = ask_value(&w.part, self.pages_answered(id, &w.request))?;
                let title = chat.and_then(|c| c["title"].as_str()).map(str::trim).filter(|t| !t.is_empty()).map_or(Value::Nil, Value::from);
                let plan = is_plan(&w.questions) && !w.steps.is_empty();
                let steps: Vec<Value> = w.steps.iter().map(|s| Value::from(s.as_str())).collect();
                Some(Value::record([
                    ("key", Value::from(format!("{id}:{}", w.request).as_str())),
                    ("chat", Value::from(id.as_str())),
                    ("request", Value::from(w.request.as_str())),
                    ("title", title),
                    ("project", chat.and_then(|c| c["spaceId"].as_str()).map_or(Value::Nil, Value::from)),
                    ("kind", Value::from(if plan { "plan" } else { "ask" })),
                    ("secret", Value::Bool(w.questions.iter().any(is_secret))),
                    ("ask", ask),
                    ("steps", Value::List(steps.into())),
                ]))
            })
            .collect();
        Value::List(list.into())
    }

    pub(super) fn publish_waiting(&self) {
        let value = self.waiting_value();
        for shared in self.waiting_shared.borrow().iter() {
            shared.set(value.clone());
        }
    }

    pub(super) fn land_ask<'a>(self: &Rc<Self>, chat: &str, newest_first: impl Iterator<Item = &'a Json> + Clone) {
        let last = newest_first.clone().find(|e| e["role"].as_str() == Some("assistant"));
        let part = asked(newest_first).filter(|p| {
            let request = p["requestId"].as_str();
            self.waiting.borrow().get(chat).is_none_or(|w| !w.answered || Some(w.request.as_str()) != request)
        });
        let Some(part) = part else {
            self.look_again(chat.to_owned());
            return;
        };
        let request = part["requestId"].as_str().unwrap_or_default().to_owned();
        let questions = part["questions"].as_array().cloned().unwrap_or_default();
        let steps = if is_plan(&questions) { last.map(proposed_steps).unwrap_or_default() } else { Vec::new() };
        let same = self.waiting.borrow().get(chat).is_some_and(|w| w.request == request);
        if !same {
            self.waiting.borrow_mut().insert(chat.to_owned(), Waiting { request, questions, part: part.clone(), steps, answered: false });
            self.publish_waiting();
        }
        self.ask_tries.borrow_mut().remove(chat);
    }

    pub(super) fn look_again(self: &Rc<Self>, chat: String) {
        let tries = *self.ask_tries.borrow_mut().entry(chat.clone()).and_modify(|n| *n += 1).or_insert(1);
        if tries > LOOK_TRIES {
            return;
        }
        let me = self.clone();
        engine::get().spawn(async move {
            zgx::timer::sleep(LOOK_RETRY).await;
            if me.waits(&chat) && !me.ask_looks.borrow().contains_key(&chat) {
                me.look_at_ask(chat);
            }
        });
    }

    fn waits(&self, chat: &str) -> bool {
        self.sessions.borrow().get(chat).is_some_and(|s| s["status"].as_str() == Some("awaitingInput") && !stale(&s["updatedAt"]))
    }

    pub(super) async fn waiting_answer(self: &Rc<Self>, args: Value) -> Result<(), Outcome> {
        let id = args.field("id").map(text).unwrap_or_default();
        let request = args.field("request").map(text).unwrap_or_default();
        let questions = self.waiting.borrow().get(&id).filter(|w| w.request == request && !w.answered).ok_or("missing")?.questions.clone();
        self.answer(&id, &request, &questions, &args).await
    }

    pub(super) async fn answer(self: &Rc<Self>, id: &str, request: &str, questions: &[Json], args: &Value) -> Result<(), Outcome> {
        let decision = args.field("decision").map(text).unwrap_or_default();
        let secret = questions.iter().any(is_secret);
        if secret && decision != "deny" {
            return Err("failed");
        }
        let (answers, took_page) = match decision.as_str() {
            "once" | "always" => (single(questions, "Yes"), false),
            "deny" if is_approval(questions) || is_plan(questions) => (single(questions, "No"), false),
            "deny" => (Vec::new(), false),
            _ => match self.take_page(id, request, questions, args) {
                Some(answers) => (answers, true),
                None => {
                    self.publish_answers();
                    return Ok(());
                }
            },
        };
        self.mark_answered(id, request, true);
        self.publish_answers();
        let sent = if secret {
            engine::get().call("AnswerSecret", json!({ "chatId": id, "requestId": request })).await.map(drop).map_err(|e| engine::outcome(&e))
        } else {
            command(id, json!({ "kind": "respondInput", "requestId": request, "answers": answers })).await
        };
        match sent {
            Ok(()) => {
                if self.pages.borrow().get(id).is_some_and(|w| w.request == request) {
                    self.pages.borrow_mut().remove(id);
                }
                self.ask_tries.borrow_mut().remove(id);
                if self.waits(id) {
                    self.look_again(id.to_owned());
                }
                self.publish_answers();
                Ok(())
            }
            Err(outcome) => {
                self.answer_failed(id, request, took_page);
                self.publish_answers();
                Err(outcome)
            }
        }
    }

    fn take_page(&self, id: &str, request: &str, questions: &[Json], args: &Value) -> Option<Vec<Json>> {
        let mut labels: Vec<String> = args.field("values").map(Value::list).unwrap_or_default().iter().map(text).collect();
        let words = args.field("text").map(text).unwrap_or_default();
        let typed = labels.is_empty() && !words.trim().is_empty();
        if typed {
            labels.push(words.trim().to_owned());
        }
        let mut all = self.pages.borrow_mut();
        let wizard = all.entry(id.to_owned()).or_default();
        if wizard.request != request {
            *wizard = Wizard { request: request.to_owned(), pages: Vec::new() };
        }
        let question = questions.get(wizard.pages.len()).and_then(|q| q["id"].as_str()).unwrap_or_default().to_owned();
        wizard.pages.push(Page { question, labels, typed });
        (wizard.pages.len() >= questions.len())
            .then(|| wizard.pages.iter().map(|p| json!({ "questionId": p.question, "labels": p.labels })).collect())
    }

    fn mark_answered(&self, id: &str, request: &str, answered: bool) {
        if let Some(row) = self.waiting.borrow_mut().get_mut(id).filter(|w| w.request == request) {
            row.answered = answered;
        }
    }

    fn answer_failed(&self, id: &str, request: &str, took_page: bool) {
        self.mark_answered(id, request, false);
        if took_page && let Some(wizard) = self.pages.borrow_mut().get_mut(id).filter(|w| w.request == request) {
            wizard.pages.pop();
        }
    }

    fn publish_answers(&self) {
        self.publish_waiting();
        self.publish_views();
    }

    pub(super) fn pages_answered(&self, id: &str, request: &str) -> usize {
        self.pages.borrow().get(id).filter(|w| w.request == request).map_or(0, |w| w.pages.len())
    }

    pub(super) fn page_back(&self, args: &Value) -> Value {
        let id = args.field("id").map(text).unwrap_or_default();
        let request = args.field("request").map(text).unwrap_or_default();
        let popped = self.pages.borrow_mut().get_mut(&id).filter(|w| w.request == request).and_then(|w| w.pages.pop());
        let Some(page) = popped else { return Value::Nil };
        self.publish_answers();
        let chosen: Vec<Value> = if page.typed { Vec::new() } else { page.labels.iter().map(|l| Value::from(l.as_str())).collect() };
        let words = if page.typed { page.labels.first().map_or("", String::as_str) } else { "" };
        Value::record([("chosen", Value::List(chosen.into())), ("text", Value::from(words))])
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    fn input_entry(request: &str, questions: Json) -> Json {
        json!({ "id": "a1", "role": "assistant", "parts": [{ "kind": "input", "requestId": request, "questions": questions }] })
    }

    fn with_waiting(chat: &str, request: &str, questions: Json) -> Rc<Chats> {
        let chats = Rc::new(Chats::default());
        chats.sessions.borrow_mut().insert(chat.into(), json!({ "status": "awaitingInput", "updatedAt": "2099-01-01T00:00:00Z" }));
        let entry = input_entry(request, questions);
        chats.land_ask(chat, std::iter::once(&entry));
        chats
    }

    fn answer_of(values: &[&str], words: &str) -> Value {
        let values: Vec<Value> = values.iter().map(|v| Value::from(*v)).collect();
        Value::record([("values", Value::List(values.into())), ("text", Value::from(words))])
    }

    fn two_pages() -> Json {
        json!([
            { "id": "q1", "question": "First?", "options": ["A", "B"] },
            { "id": "q2", "question": "Second?", "options": ["C"] },
        ])
    }

    fn page(chats: &Chats) -> Option<f64> {
        let Value::List(rows) = chats.waiting_value() else { panic!("a list") };
        rows.first()?.field("ask")?.field("question")?.field("page").and_then(Value::as_f64)
    }

    #[test]
    fn a_chat_not_open_waiting_is_listed_with_its_page() {
        let chats = with_waiting("c", "r", two_pages());
        assert_eq!(page(&chats), Some(1.));
        chats.take_page("c", "r", two_pages().as_array().unwrap(), &answer_of(&["A"], ""));
        assert_eq!(page(&chats), Some(2.));
    }

    #[test]
    fn answering_goes_page_by_page_and_back_gives_what_the_page_held() {
        let chats = with_waiting("c", "r", two_pages());
        let questions = two_pages();
        let questions = questions.as_array().unwrap();
        let back = |request: &str| chats.page_back(&Value::record([("id", Value::from("c")), ("request", Value::from(request))]));
        assert!(chats.take_page("c", "r", questions, &answer_of(&["A"], "")).is_none());
        assert!(matches!(back("old"), Value::Nil));
        let picked = back("r");
        assert!(matches!(picked.field("chosen"), Some(Value::List(l)) if l.len() == 1));
        assert_eq!(picked.field("text").map(text).as_deref(), Some(""));
        assert_eq!(page(&chats), Some(1.));
        assert!(chats.take_page("c", "r", questions, &answer_of(&[], " use dev ")).is_none());
        let typed = back("r");
        assert!(matches!(typed.field("chosen"), Some(Value::List(l)) if l.is_empty()));
        assert_eq!(typed.field("text").map(text).as_deref(), Some("use dev"));
        assert!(matches!(back("r"), Value::Nil), "on the first page");
    }

    #[test]
    fn a_failed_send_keeps_the_pages_before_and_leaves_a_newer_request_alone() {
        let chats = with_waiting("c", "r", two_pages());
        let questions = two_pages();
        let questions = questions.as_array().unwrap();
        chats.take_page("c", "r", questions, &answer_of(&["A"], ""));
        let answers = chats.take_page("c", "r", questions, &answer_of(&["C"], "")).expect("the last page");
        assert_eq!(answers.len(), 2);
        chats.mark_answered("c", "r", true);
        assert_eq!(page(&chats), None, "out of the list while it goes");
        chats.answer_failed("c", "r", true);
        assert_eq!(page(&chats), Some(2.), "back on the page that failed, the one before kept");

        chats.mark_answered("c", "r", true);
        let next = input_entry("r2", two_pages());
        chats.land_ask("c", std::iter::once(&next));
        chats.take_page("c", "r2", questions, &answer_of(&["B"], ""));
        chats.answer_failed("c", "r", true);
        assert_eq!(page(&chats), Some(2.), "r's failure leaves r2's page");
    }

    #[test]
    fn a_question_has_one_wizard_and_an_answer_here_takes_its_row_away() {
        let chats = with_waiting("c", "r", two_pages());
        let questions = two_pages();
        chats.take_page("c", "r", questions.as_array().unwrap(), &answer_of(&["A"], ""));
        assert_eq!(chats.pages_answered("c", "r"), 1);
        assert_eq!(chats.pages_answered("c", "other"), 0);
        chats.mark_answered("c", "r", true);
        assert_eq!(page(&chats), None);
    }

    #[test]
    fn a_row_follows_the_chat_list() {
        let chats = with_waiting("c", "r", two_pages());
        let row = |chats: &Chats| {
            let Value::List(rows) = chats.waiting_value() else { panic!("a list") };
            rows.first().cloned()
        };
        assert!(matches!(row(&chats).unwrap().field("title"), Some(Value::Nil)), "no title of its own yet");
        chats.chats.borrow_mut().insert("c".into(), json!({ "id": "c", "title": " Named " }));
        assert_eq!(row(&chats).unwrap().field("title").map(text).as_deref(), Some("Named"));
        chats.chats_heard.set(true);
        chats.chats.borrow_mut().clear();
        assert!(row(&chats).is_none(), "gone from the list");
    }

    #[test]
    fn an_answered_request_is_not_taken_for_the_next() {
        let chats = with_waiting("c", "r", two_pages());
        chats.waiting.borrow_mut().get_mut("c").unwrap().answered = true;
        let same = input_entry("r", two_pages());
        chats.land_ask("c", std::iter::once(&same));
        assert_eq!(page(&chats), None, "still answered: its row stays away");
        let next = input_entry("r2", two_pages());
        chats.land_ask("c", std::iter::once(&next));
        assert_eq!(page(&chats), Some(1.));
    }

    #[test]
    fn a_plan_question_says_its_steps() {
        let entry = json!({ "id": "a1", "role": "assistant", "parts": [
            { "kind": "tool", "call": { "kind": "unknown", "name": "ExitPlanMode", "input": { "plan": "# Plan\n1. **Do** it\n- `Then` that\n  - nested\n" } } },
            { "kind": "input", "requestId": "r", "questions": [{ "id": "q", "header": zeron_proto::PLAN_APPROVAL_HEADER, "approval": true, "question": "Approve?", "options": ["Yes", "No"] }] },
        ] });
        let chats = Rc::new(Chats::default());
        chats.land_ask("c", std::iter::once(&entry));
        let Value::List(rows) = chats.waiting_value() else { panic!("a list") };
        assert_eq!(rows[0].field("kind").map(text).as_deref(), Some("plan"));
        let Some(Value::List(steps)) = rows[0].field("steps") else { panic!("steps") };
        assert_eq!(steps.iter().map(text).collect::<Vec<_>>(), ["Do it", "Then that"]);
    }

    #[test]
    fn a_doc_says_the_tool_it_is_on_and_why_it_stopped() {
        let working = vec![Entry::new(json!({ "id": "e1", "role": "assistant", "parts": [
            { "kind": "tool", "id": "t1", "call": { "kind": "readFile", "path": "src/a.rs" } },
            { "kind": "tool", "id": "t2", "call": { "kind": "exec", "command": "cargo test" } },
            { "kind": "text", "text": "Running the tests." },
        ] }))];
        assert_eq!(doing_of(&working).as_deref(), Some("Ran cargo test"));
        assert_eq!(error_of(&working), None);
        let failed = vec![Entry::new(json!({ "id": "e2", "role": "assistant", "parts": [{ "kind": "error", "message": "Usage limit\nreached" }] }))];
        assert_eq!(error_of(&failed).as_deref(), Some("Usage limit reached"));
        let asked = vec![Entry::new(json!({ "id": "u1", "role": "user", "parts": [] }))];
        assert_eq!(doing_of(&asked), None);
    }

    #[test]
    fn a_runs_nested_spawns_are_read_from_its_doc() {
        let entries = vec![Entry::new(json!({
            "id": "e1", "role": "assistant", "createdAt": 2000.,
            "parts": [
                { "kind": "tool", "id": "t1", "subagentRef": "doc--sub--1", "subagentStatus": "running", "resolved": false,
                  "call": { "kind": "unknown", "name": "Agent: Inner", "input": { "description": "Inner", "modelId": "gpt-5.5", "effort": "high" } } },
                { "kind": "tool", "id": "t2", "call": { "kind": "readFile", "path": "a.rs" } },
            ],
        }))];
        let spawns = spawns_of(&entries);
        assert_eq!(spawns.len(), 1);
        assert_eq!(spawns[0].field("call").map(text).as_deref(), Some("t1"));
        assert_eq!(spawns[0].field("model").map(text).as_deref(), Some("gpt-5.5"));
        assert_eq!(spawns[0].field("reasoning").map(text).as_deref(), Some("high"));
        assert_eq!(spawns[0].field("at").and_then(Value::as_f64), Some(2.));
    }
}
