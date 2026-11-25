//! An uploaded avatar image has a `botAvatars` row of its own: the room refuses an op over 16 KB, and a Bot's instructions and its image would not both fit one row.

use super::*;
use zeron_proto::{
    AgentProfile, AgentProfileDraft, Bot, BotAvatar, BotPatch, pin_order_key_between,
    valid_pin_order_key,
};

pub const BOTS_MAX: usize = 200;
pub const BOT_NAME_MAX_CHARS: usize = 80;

const IMAGE: &str = "image";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawBot {
    #[serde(flatten)]
    profile: AgentProfile,
    #[serde(default)]
    avatar: BotAvatar,
    #[serde(default)]
    chat_id: String,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    order_key: String,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    unread: bool,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    deleted: bool,
}

#[derive(Debug, Clone)]
struct Kept {
    bot: Bot,
    deleted: bool,
}

impl From<RawBot> for Kept {
    fn from(raw: RawBot) -> Self {
        Kept {
            bot: Bot {
                profile: raw.profile,
                avatar: BotAvatar {
                    image: None,
                    ..raw.avatar
                },
                chat_id: raw.chat_id,
                pinned: raw.pinned,
                order_key: raw.order_key,
                archived: raw.archived,
                unread: raw.unread,
                template: raw.template,
                created_at: tasks::at(raw.created_at),
                updated_at: tasks::at(raw.updated_at),
            },
            deleted: raw.deleted,
        }
    }
}

fn invalid(message: &str) -> DocError {
    DocError::Schema(message.into())
}

fn checked_name(name: &str) -> Result<String, DocError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > BOT_NAME_MAX_CHARS {
        return Err(invalid("A Bot needs a name of up to 80 characters"));
    }
    Ok(name.to_string())
}

fn checked_avatar(avatar: &BotAvatar) -> Result<BotAvatar, DocError> {
    avatar
        .clone()
        .normalized()
        .map_err(|part| invalid(&format!("Invalid Bot avatar: {part}")))
}

fn avatar_field(avatar: &BotAvatar) -> Value {
    json!({ "glyph": avatar.glyph, "frame": avatar.frame, "color": avatar.color })
}

fn profile_fields(
    name: String,
    profile: &AgentProfile,
) -> Result<BTreeMap<String, Value>, DocError> {
    Ok(fields([
        ("name", json!(name)),
        ("harness", serde_json::to_value(&profile.harness)?),
        ("model", opt_str(profile.model.as_deref())),
        ("reasoning", serde_json::to_value(profile.reasoning)?),
        ("access", serde_json::to_value(profile.access)?),
        ("modelOptions", Value::Object(profile.model_options.clone())),
        ("notes", json!(profile.notes)),
        ("instructions", json!(profile.instructions)),
        ("disabledTools", json!(profile.disabled_tools)),
    ]))
}

fn check_fits(row: &RegistryRow) -> Result<(), DocError> {
    if fits_op_budget(&row_to_seed_op(row)) {
        return Ok(());
    }
    Err(invalid(if row.kind == KIND_BOT_AVATARS {
        "This image is too large to sync; choose a smaller one"
    } else {
        "This Bot is too large to sync; shorten its instructions"
    }))
}

pub fn check_bot_syncs(bot: &Bot) -> Result<(), DocError> {
    let name = checked_name(&bot.profile.name)?;
    let avatar = checked_avatar(&bot.avatar)?;
    let clock = encode_hlc(
        9_999_999_999_999,
        999_999,
        "00000000-0000-0000-0000-000000000000",
    );
    let row = |kind: &str, mut set: BTreeMap<String, Value>| {
        set.insert("updatedAt".into(), json!(9_999_999_999_999i64));
        check_fits(&RegistryRow {
            kind: kind.into(),
            id: bot.profile.id.clone(),
            seq: 0,
            deleted: false,
            del_hlc: None,
            clocks: set.keys().map(|key| (key.clone(), clock.clone())).collect(),
            fields: set,
        })
    };
    let mut set = profile_fields(name, &bot.profile)?;
    set.extend(fields([
        ("id", json!(bot.profile.id)),
        ("avatar", avatar_field(&avatar)),
        ("chatId", json!(bot.chat_id)),
        ("pinned", json!(true)),
        ("orderKey", json!("z".repeat(64))),
        ("archived", json!(true)),
        ("unread", json!(true)),
        ("template", opt_str(bot.template.as_deref())),
        ("createdAt", json!(9_999_999_999_999i64)),
        ("deleted", json!(false)),
    ]));
    row(KIND_BOTS, set)?;
    row(
        KIND_BOT_AVATARS,
        fields([(IMAGE, opt_str(avatar.image.as_deref()))]),
    )
}

impl RegistryDoc {
    fn kept_bot(&self, bot_id: &str) -> Option<Kept> {
        self.overlay_row(KIND_BOTS, bot_id)
            .and_then(|row| row_to::<RawBot>(&row))
            .map(Kept::from)
    }

    fn bot_image(&self, bot_id: &str) -> Option<String> {
        self.overlay_row(KIND_BOT_AVATARS, bot_id)?
            .fields
            .get(IMAGE)?
            .as_str()
            .filter(|image| !image.is_empty())
            .map(str::to_owned)
    }

    pub fn bot(&self, bot_id: &str) -> Option<Bot> {
        let mut bot = self.kept_bot(bot_id).filter(|kept| !kept.deleted)?.bot;
        bot.avatar.image = self.bot_image(bot_id);
        Some(bot)
    }

    pub fn read_bots(&self) -> Result<Vec<Bot>, DocError> {
        let mut bots: Vec<Bot> = self
            .read_cached::<RawBot, Kept>(KIND_BOTS, Kept::from)
            .into_iter()
            .filter(|kept| !kept.deleted)
            .map(|kept| kept.bot)
            .collect();
        for bot in &mut bots {
            bot.avatar.image = self.bot_image(&bot.profile.id);
        }
        bots.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| {
                    if a.pinned {
                        a.order_key.cmp(&b.order_key)
                    } else {
                        b.created_at.cmp(&a.created_at)
                    }
                })
                .then_with(|| a.profile.id.cmp(&b.profile.id))
        });
        Ok(bots)
    }

    fn last_pinned_key(&self, but: &str) -> Option<String> {
        self.overlay_rows(KIND_BOTS)
            .into_iter()
            .filter(|row| row.id != but)
            .filter(|row| {
                row.fields.get("pinned") == Some(&json!(true))
                    && row.fields.get("deleted") != Some(&json!(true))
            })
            .filter_map(|row| {
                row.fields
                    .get("orderKey")
                    .and_then(Value::as_str)
                    .filter(|key| valid_pin_order_key(key))
                    .map(str::to_owned)
            })
            .max()
    }

    fn bot_op(
        &self,
        kind: &str,
        id: &str,
        op: OpKind,
        set: BTreeMap<String, Value>,
        hlc: String,
    ) -> Result<RowOp, DocError> {
        let op = RowOp {
            kind: kind.into(),
            id: id.into(),
            op,
            set: Some(set),
            hlc,
            clocks: None,
        };
        let row = overlay(self.overlay_row(kind, id).as_ref(), [&op])
            .ok_or_else(|| invalid("Bot no longer exists"))?;
        check_fits(&row)?;
        Ok(op)
    }

    fn bot_image_op(
        &mut self,
        bot_id: &str,
        image: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Option<RowOp>, DocError> {
        if self.bot_image(bot_id).as_deref() == image {
            return Ok(None);
        }
        self.observe_sidebar_row(KIND_BOT_AVATARS, bot_id);
        let hlc = self.next_hlc();
        let set = fields([
            (IMAGE, opt_str(image)),
            ("updatedAt", json!(now.timestamp_millis())),
        ]);
        self.bot_op(KIND_BOT_AVATARS, bot_id, OpKind::Upsert, set, hlc)
            .map(Some)
    }

    pub fn create_bot(&mut self, bot: &Bot) -> Result<bool, DocError> {
        let id = bot.profile.id.as_str();
        if !valid_row_id(id) {
            return Err(invalid("Invalid Bot ID"));
        }
        if !valid_row_id(&bot.chat_id) {
            return Err(invalid("Invalid Bot chat ID"));
        }
        let name = checked_name(&bot.profile.name)?;
        let avatar = checked_avatar(&bot.avatar)?;
        if self.overlay_row(KIND_BOTS, id).is_some() {
            return Ok(false);
        }
        if self.read_bots()?.len() >= BOTS_MAX {
            return Err(invalid("You can have up to 200 Bots"));
        }
        let hlc = self.next_hlc();
        let order_key = if !bot.pinned {
            String::new()
        } else if bot.order_key.is_empty() {
            pin_order_key_between(self.last_pinned_key(id).as_deref(), None, &hlc)
                .map_err(invalid)?
        } else {
            if !valid_pin_order_key(&bot.order_key) {
                return Err(invalid("Invalid Bot order key"));
            }
            bot.order_key.clone()
        };
        let mut set = profile_fields(name, &bot.profile)?;
        set.extend(fields([
            ("id", json!(id)),
            ("avatar", avatar_field(&avatar)),
            ("chatId", json!(bot.chat_id)),
            ("pinned", json!(bot.pinned)),
            ("orderKey", json!(order_key)),
            ("archived", json!(bot.archived)),
            ("unread", json!(bot.unread)),
            ("template", opt_str(bot.template.as_deref())),
            ("createdAt", json!(bot.created_at.timestamp_millis())),
            ("updatedAt", json!(bot.updated_at.timestamp_millis())),
            ("deleted", json!(false)),
        ]));
        let mut ops = vec![self.bot_op(KIND_BOTS, id, OpKind::Upsert, set, hlc)?];
        ops.extend(self.bot_image_op(id, avatar.image.as_deref(), bot.created_at)?);
        self.enqueue_ops(ops);
        Ok(true)
    }

    pub fn update_bot(
        &mut self,
        bot_id: &str,
        patch: &BotPatch,
        now: DateTime<Utc>,
    ) -> Result<bool, DocError> {
        let Some(bot) = self.bot(bot_id) else {
            return Ok(false);
        };
        let mut set = BTreeMap::new();
        if let Some(draft) = &patch.profile {
            let AgentProfileDraft {
                name,
                harness,
                model,
                reasoning,
                access,
                model_options,
                notes,
                instructions,
                disabled_tools,
            } = draft.clone();
            let next = AgentProfile {
                id: bot_id.to_owned(),
                name: checked_name(&name)?,
                harness,
                model,
                reasoning,
                access,
                model_options,
                notes,
                instructions,
                disabled_tools,
                by_agent: false,
            };
            let now_is = profile_fields(bot.profile.name.clone(), &bot.profile)?;
            for (key, value) in profile_fields(next.name.clone(), &next)? {
                if now_is.get(&key) != Some(&value) {
                    set.insert(key, value);
                }
            }
        }
        let mut image = None;
        if let Some(avatar) = &patch.avatar {
            let avatar = checked_avatar(avatar)?;
            if avatar_field(&avatar) != avatar_field(&bot.avatar) {
                set.insert("avatar".into(), avatar_field(&avatar));
            }
            image = Some(avatar.image);
        }
        if let Some(archived) = patch.archived.filter(|a| *a != bot.archived) {
            set.insert("archived".into(), json!(archived));
        }
        if let Some(unread) = patch.unread.filter(|u| *u != bot.unread) {
            set.insert("unread".into(), json!(unread));
        }
        self.observe_sidebar_row(KIND_BOTS, bot_id);
        let hlc = self.next_hlc();
        let pinned = patch.pinned.unwrap_or(bot.pinned);
        if pinned != bot.pinned {
            set.insert("pinned".into(), json!(pinned));
        }
        match &patch.order_key {
            Some(key) => {
                if !valid_pin_order_key(key) {
                    return Err(invalid("Invalid Bot order key"));
                }
                if *key != bot.order_key {
                    set.insert("orderKey".into(), json!(key));
                }
            }
            None if pinned && !bot.pinned => {
                let key =
                    pin_order_key_between(self.last_pinned_key(bot_id).as_deref(), None, &hlc)
                        .map_err(invalid)?;
                set.insert("orderKey".into(), json!(key));
            }
            None => {}
        }
        let mut ops = Vec::new();
        if let Some(image) = image {
            ops.extend(self.bot_image_op(bot_id, image.as_deref(), now)?);
        }
        if !set.is_empty() || !ops.is_empty() {
            set.insert("updatedAt".into(), json!(now.timestamp_millis()));
            ops.insert(0, self.bot_op(KIND_BOTS, bot_id, OpKind::Update, set, hlc)?);
            self.enqueue_ops(ops);
        }
        Ok(true)
    }

    /// A flagged row, not a tombstone, so a retried create cannot revive it.
    pub fn delete_bot(
        &mut self,
        bot_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<String>, DocError> {
        let Some(bot) = self.bot(bot_id) else {
            return Ok(None);
        };
        self.observe_sidebar_row(KIND_BOTS, bot_id);
        let hlc = self.next_hlc();
        let mut ops = vec![RowOp {
            kind: KIND_BOTS.into(),
            id: bot_id.into(),
            op: OpKind::Update,
            set: Some(fields([
                ("deleted", json!(true)),
                ("updatedAt", json!(now.timestamp_millis())),
            ])),
            hlc,
            clocks: None,
        }];
        ops.extend(self.bot_image_op(bot_id, None, now)?);
        self.enqueue_ops(ops);
        self.delete_chat(&bot.chat_id)?;
        Ok(Some(bot.chat_id))
    }
}
