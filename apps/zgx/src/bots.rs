//! The engine's Bots (`WatchBots`, `Mutate` Bot ops) as `bots.list` and the `bots.*` actions ui/screens/bots reads and calls.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use serde_json::{Value as Json, json};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Shared, Value};

use crate::auth::text;
use crate::engine;

#[derive(Default)]
struct Bots {
    list: RefCell<Json>,
    ready: Cell<bool>,
    shared: RefCell<Vec<(u64, Shared)>>,
    next_window: Cell<u64>,
    watching: Cell<bool>,
    watch: RefCell<Option<crate::engine::rpc::Watch>>,
}

thread_local! {
    static BOTS: Rc<Bots> = Rc::new(Bots::default());
}

/// A Bot as scripts read it.
fn shaped(bot: &Json) -> Json {
    json!({
        "id": bot["id"],
        "name": bot["name"],
        "chat": bot["chatId"],
        "avatar": {
            "glyph": bot["avatar"]["glyph"],
            "frame": bot["avatar"]["frame"],
            "color": bot["avatar"]["color"],
            "image": bot["avatar"]["image"],
        },
        "pinned": bot["pinned"].as_bool().unwrap_or(false),
        "archived": bot["archived"].as_bool().unwrap_or(false),
        "unread": bot["unread"].as_bool().unwrap_or(false),
        "template": bot["template"],
        "harness": bot["harness"],
        "model": bot["model"],
        "reasoning": bot["reasoning"],
        "access": bot["access"],
        "instructions": bot["instructions"],
        "created": bot["createdAt"],
    })
}

impl Bots {
    fn value(&self) -> Value {
        let bots: Vec<Json> = self
            .list
            .borrow()
            .as_array()
            .into_iter()
            .flatten()
            .map(shaped)
            .collect();
        let state = if self.ready.get() { "ready" } else { "idle" };
        Value::from_json(&json!({ "state": state, "bots": bots }).to_string()).unwrap_or(Value::Nil)
    }

    fn publish(&self) {
        let value = self.value();
        for (_, shared) in self.shared.borrow().iter() {
            shared.set(value.clone());
        }
    }

    fn heard(self: &Rc<Self>, item: Json) {
        *self.list.borrow_mut() = item;
        self.ready.set(true);
        self.publish();
    }

    fn find(&self, id: &str) -> Option<Json> {
        self.list
            .borrow()
            .as_array()?
            .iter()
            .find(|b| b["id"].as_str() == Some(id))
            .cloned()
    }
}

fn json_of(value: Option<&Value>) -> Json {
    value
        .filter(|v| !matches!(v, Value::Nil))
        .and_then(|v| serde_json::from_str(&v.to_json()).ok())
        .unwrap_or(Json::Null)
}

/// The avatar a script names: its glyph, frame and colour, or its image.
fn avatar_of(args: &Value) -> Option<Json> {
    let avatar = json_of(args.field("avatar"));
    avatar.is_object().then(|| {
        json!({
            "glyph": avatar["glyph"].as_str().unwrap_or_default(),
            "frame": avatar["frame"].as_str().unwrap_or_default(),
            "color": avatar["color"].as_str().unwrap_or_default(),
            "image": avatar["image"].as_str().filter(|i| !i.is_empty()),
        })
    })
}

async fn mutate(params: Json) -> Result<Value, String> {
    engine::get()
        .call("Mutate", params)
        .await
        .map(|_| Value::Nil)
}

/// A patch of the Bot's own marks: `pinned`, `archived`, `unread`, as the call names them.
fn marks_of(args: &Value) -> Json {
    let mut patch = json!({});
    for key in ["pinned", "archived", "unread"] {
        if let Some(v) = args.field(key).filter(|v| !matches!(v, Value::Nil)) {
            patch[key] = json!(v.truthy());
        }
    }
    patch
}

pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let t = BOTS.with(Rc::clone);
    let window = t.next_window.get();
    t.next_window.set(window + 1);
    t.shared
        .borrow_mut()
        .push((window, host.shared("bots.list", t.value())));
    let me = Rc::downgrade(&t);
    cx.on_closed(move || {
        if let Some(t) = me.upgrade() {
            t.shared.borrow_mut().retain(|(w, _)| *w != window);
        }
    });
    if !t.watching.replace(true) {
        let weak = Rc::downgrade(&t);
        engine::get().on_connect(move |rpc| {
            let Some(t) = weak.upgrade() else { return };
            let heard = Rc::downgrade(&t);
            *t.watch.borrow_mut() = Some(rpc.watch("WatchBots", json!({}), move |item| {
                if let Some(t) = heard.upgrade() {
                    t.heard(item);
                }
            }));
        });
    }
    // A Bot and its chat: `name`, `avatar`, `template`, `instructions`, the
    // composer's `picks` (its agent, model, effort and access) and where its
    // chat works (`project`, `device`). Answers `{ id, chat }`.
    host.async_action("bots.create", |args| async move {
        crate::plugins::preview::refuse("bots.create")?;
        let field = |key: &str| args.field(key).map(text).filter(|s| !s.is_empty());
        let name = field("name").unwrap_or_default();
        if name.trim().is_empty() {
            return Err("empty".to_string());
        }
        let picks = args.field("picks").cloned().unwrap_or(Value::Nil);
        let mut config = crate::chat::draft_config(&picks);
        if config["harness"].is_null() {
            return Err("no agent".to_string());
        }
        if let Some(instructions) = field("instructions") {
            config["instructions"] = json!(instructions);
        }
        let (id, chat) = (crate::sessions::new_id(), crate::sessions::new_id());
        let mut params = json!({ "op": "createBot", "botId": id, "chatId": chat, "name": name, "config": config });
        if let Some(avatar) = avatar_of(&args) {
            params["avatar"] = avatar;
        }
        for (key, wire) in [("template", "template"), ("project", "spaceId"), ("device", "deviceId")] {
            if let Some(v) = field(key) {
                params[wire] = json!(v);
            }
        }
        mutate(params).await?;
        Ok(Value::from_json(&json!({ "id": id, "chat": chat }).to_string()).unwrap_or(Value::Nil))
    });
    // Its name and avatar (what the person edits), and its marks.
    let a = t.clone();
    host.async_action("bots.update", move |args| {
        let a = a.clone();
        async move {
            crate::plugins::preview::refuse("bots.update")?;
            let id = args.field("id").map(text).unwrap_or_default();
            let mut patch = marks_of(&args);
            if let Some(avatar) = avatar_of(&args) {
                patch["avatar"] = avatar;
            }
            if let Some(name) = args
                .field("name")
                .map(text)
                .filter(|n| !n.trim().is_empty())
            {
                let bot = a.find(&id).ok_or("missing")?;
                patch["profile"] = json!({
                    "name": name,
                    "harness": bot["harness"],
                    "model": bot["model"],
                    "reasoning": bot["reasoning"],
                    "access": bot["access"],
                    "modelOptions": bot.get("modelOptions").cloned().unwrap_or(json!({})),
                    "notes": bot.get("notes").cloned().unwrap_or(json!("")),
                    "instructions": bot.get("instructions").cloned().unwrap_or(json!("")),
                    "disabledTools": bot.get("disabledTools").cloned().unwrap_or(json!([])),
                });
            }
            mutate(json!({ "op": "updateBot", "botId": id, "patch": patch })).await
        }
    });
    host.async_action("bots.delete", |args| async move {
        crate::plugins::preview::refuse("bots.delete")?;
        let id = args.field("id").map(text).unwrap_or_default();
        mutate(json!({ "op": "deleteBot", "botId": id })).await
    });
}
