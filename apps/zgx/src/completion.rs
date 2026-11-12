//! What the composer's `/` and `$` offer: an agent's slash commands and
//! skills for a chat's (or project's) folder on its device, as the shipping
//! composer finds them (crates/ui/src/composer.rs: `ListCommands` and
//! `ListSkills`, merged by `invocation_candidates`), each with the canonical
//! reference a pick puts in the prompt (`zeron_proto::invocation`'s
//! `Invocation::link`), which the engine reads at send.
//!
//!   chat.invocations { harness, target = { chat? | space?, device?, path?, home? } }
//!       { rows = { { kind = "command" | "skill", name, label, detail, link,
//!                    plain?, terminal_only?, blocked?, task? } },
//!         failed?, warning? }  the list that couldn't be had ("commands" or
//!                        "skills") when the other came, and that in words;
//!                        or an error when neither could be
//!
//! Zeron's own commands ride the agent's list as the shipping menu has them
//! (`with_zeron_commands`: /btw, /compact, then its built-ins), then its
//! enabled skills; a command that is a skill's own alias is left to the skill.
//! Names and paths the canonical reference couldn't carry are left out. A row
//! `plain` inserts its `link` as typed (`/name`, which the send reads); one
//! `terminal_only` runs only in Claude Code's terminal; one `blocked` says why
//! it can't run, in place of its description; one `task` can ride a task's
//! words (proto `inserts_message_text`). `home` lists for the home folder.

use std::{cell::RefCell, future::Future, pin::Pin, rc::Rc, task::Poll};

use serde_json::{Map, Value as Json, json};
use zeron_proto::{
    HarnessId, SlashCommand,
    app_commands::{COMPACT_COMMAND, COMPUTER_USE_COMMAND, TERMINAL_ONLY_COMMANDS},
    command_catalog::{
        InvocationCandidate, inserts_message_text, invocation_detail, skill_display_name, with_zeron_commands, zeron_owned_command,
    },
    invocation::Invocation,
};
use zgx_script::{ScriptHost, Value};

use crate::engine;

pub fn install(host: &ScriptHost) {
    let local: Rc<RefCell<Option<String>>> = Rc::default();
    host.async_action("chat.invocations", move |arg| {
        let local = local.clone();
        async move { invocations(arg, local).await }
    });
}

fn text(value: Option<&Value>) -> String {
    value.map(crate::auth::text).unwrap_or_default()
}

/// This engine's device (a target there needs no `targetDeviceId`).
async fn local_device(cache: &RefCell<Option<String>>) -> Option<String> {
    if let Some(id) = cache.borrow().clone() {
        return Some(id);
    }
    let info = engine::get().call("EngineInfo", json!({})).await.ok()?;
    let id = info["deviceId"].as_str()?.to_string();
    *cache.borrow_mut() = Some(id.clone());
    Some(id)
}

async fn invocations(arg: Value, local: Rc<RefCell<Option<String>>>) -> Result<Value, String> {
    let harness = text(arg.field("harness"));
    if harness.is_empty() {
        return Err("no agent".into());
    }
    let target = arg.field("target").ok_or("no target")?;
    let id = serde_json::from_value::<HarnessId>(json!(harness)).ok();
    let mut params = Map::new();
    params.insert("harness".into(), Json::String(harness.clone()));
    let chat = text(target.field("chat"));
    let space = text(target.field("space"));
    let path = text(target.field("path"));
    if !chat.is_empty() {
        params.insert("chatId".into(), Json::String(chat));
    } else if !space.is_empty() {
        params.insert("spaceId".into(), Json::String(space));
    } else if !path.is_empty() {
        params.insert("path".into(), Json::String(path));
    } else if !target.field("home").is_some_and(Value::truthy) {
        return Err("no session or project".into());
    }
    let device = text(target.field("device"));
    let elsewhere = !device.is_empty() && local_device(&local).await.is_none_or(|here| here != device);
    if elsewhere {
        params.insert("targetDeviceId".into(), Json::String(device.clone()));
    }
    let params = Json::Object(params);
    let ((commands, skills), unsupported) = join(
        join(
            engine::get().call("ListCommands", params.clone()),
            engine::get().call("ListSkills", params),
        ),
        crate::chat::compaction_unsupported(&harness, elsewhere.then_some(device.as_str())),
    )
    .await;
    let (rows, failed) = merge(commands.and_then(|listed| with_zeron(id, listed)), skills)?;
    let rows = flagged(rows, id, unsupported);
    let mut fields = vec![("rows", Value::List(rows.into_iter().map(row_value).collect::<Vec<_>>().into()))];
    if let Some(failed) = failed {
        fields.push(("failed", Value::from(failed)));
        fields.push(("warning", Value::from(failed_sentence(failed))));
    }
    Ok(Value::record(fields))
}

/// Both futures, run together.
async fn join<A, B>(a: impl Future<Output = A>, b: impl Future<Output = B>) -> (A, B) {
    let mut a: Pin<Box<dyn Future<Output = A> + '_>> = Box::pin(a);
    let mut b: Pin<Box<dyn Future<Output = B> + '_>> = Box::pin(b);
    let (mut ra, mut rb) = (None, None);
    std::future::poll_fn(move |cx| {
        if ra.is_none()
            && let Poll::Ready(v) = a.as_mut().poll(cx)
        {
            ra = Some(v);
        }
        if rb.is_none()
            && let Poll::Ready(v) = b.as_mut().poll(cx)
        {
            rb = Some(v);
        }
        if ra.is_some() && rb.is_some() {
            Poll::Ready((ra.take().unwrap(), rb.take().unwrap()))
        } else {
            Poll::Pending
        }
    })
    .await
}

#[derive(Debug, Clone, PartialEq)]
struct Row {
    kind: &'static str,
    name: String,
    label: String,
    detail: String,
    link: String,
    plain: bool,
    terminal_only: bool,
    blocked: Option<String>,
    path: Option<String>,
    task: bool,
}

fn row_value(row: Row) -> Value {
    let mut fields = vec![
        ("kind", Value::from(row.kind)),
        ("name", Value::from(row.name.as_str())),
        ("label", Value::from(row.label.as_str())),
        ("detail", Value::from(row.detail.as_str())),
        ("link", Value::from(row.link.as_str())),
    ];
    if row.plain {
        fields.push(("plain", Value::Bool(true)));
    }
    if row.terminal_only {
        fields.push(("terminal_only", Value::Bool(true)));
    }
    if let Some(reason) = &row.blocked {
        fields.push(("blocked", Value::from(reason.as_str())));
    }
    if let Some(path) = &row.path {
        fields.push(("path", Value::from(path.as_str())));
    }
    if row.task {
        fields.push(("task", Value::Bool(true)));
    }
    Value::record(fields)
}

// `computer-use` is left out until U25: nothing here runs it yet.
fn with_zeron(harness: Option<HarnessId>, listed: Json) -> Result<Json, String> {
    let commands: Vec<SlashCommand> = serde_json::from_value(listed).map_err(|e| e.to_string())?;
    let mut commands = with_zeron_commands(harness, commands);
    commands.retain(|c| c.name != COMPUTER_USE_COMMAND);
    serde_json::to_value(commands).map_err(|e| e.to_string())
}

fn flagged(mut rows: Vec<Row>, harness: Option<HarnessId>, unsupported: Option<String>) -> Vec<Row> {
    for row in rows.iter_mut().filter(|r| r.kind == "command") {
        if zeron_owned_command(&row.name) {
            row.plain = true;
            row.link = format!("/{}", row.name);
        }
        row.terminal_only = harness == Some(HarnessId::ClaudeCode) && TERMINAL_ONLY_COMMANDS.contains(&row.name.as_str());
        if row.name == COMPACT_COMMAND
            && let Some(reason) = &unsupported
        {
            row.detail = reason.clone();
            row.blocked = Some(reason.clone());
        }
    }
    for row in &mut rows {
        let invocation = if row.kind == "skill" {
            Invocation::Skill { name: row.name.clone(), path: row.path.clone().unwrap_or_default(), command: None }
        } else {
            Invocation::Command { name: row.name.clone() }
        };
        row.task = inserts_message_text(&InvocationCandidate {
            name: row.name.clone(),
            description: String::new(),
            input_hint: None,
            invocation,
            workspace_command: None,
        });
    }
    rows
}

fn failed_sentence(failed: &str) -> &'static str {
    if failed == "skills" { "Skills couldn't be listed" } else { "Commands couldn't be listed" }
}

/// The shipping composer's `merge_invocation_results`: both lists, or the one
/// that came with which one didn't, or why neither came.
fn merge(commands: Result<Json, String>, skills: Result<Json, String>) -> Result<(Vec<Row>, Option<&'static str>), String> {
    let commands = commands.map(|v| v.as_array().cloned().unwrap_or_default());
    // (`null`: the agent has no skills to list.)
    let skills = skills.map(|v| v.as_array().cloned());
    match (commands, skills) {
        (Ok(commands), Ok(skills)) => Ok((candidates(&commands, &skills.unwrap_or_default()), None)),
        (Ok(commands), Err(_)) if !commands.is_empty() => {
            Ok((candidates(&commands, &[]), Some("skills")))
        }
        (Err(_), Ok(Some(skills))) if skills.iter().any(|s| s["enabled"].as_bool() == Some(true)) => {
            Ok((candidates(&[], &skills), Some("commands")))
        }
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

/// `zeron_proto::invocation`'s name and path rules: what the canonical
/// reference carries unchanged.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(|c| c.is_control() || c.is_whitespace())
}
fn valid_path(path: &str) -> bool {
    !path.is_empty() && !path.chars().any(char::is_control)
}
fn valid_command_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}
fn native_skill(path: &str) -> bool {
    path.starts_with("opencode-skill:") || path.starts_with("harness-skill:")
}

/// `Invocation::link`: `[/name](zeron-invoke:<hex of its JSON>)`.
pub(crate) fn link(prefix: char, name: &str, invocation: &Json) -> String {
    let bytes = serde_json::to_vec(invocation).unwrap_or_default();
    let payload: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let label = name.replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]").replace('`', "\\`");
    format!("[{prefix}{label}](zeron-invoke:{payload})")
}

/// The shipping composer's `invocation_candidates`.
fn candidates(commands: &[Json], skills: &[Json]) -> Vec<Row> {
    let skills: Vec<&Json> = skills
        .iter()
        .filter(|s| {
            valid_name(s["name"].as_str().unwrap_or(""))
                && valid_path(s["path"].as_str().unwrap_or(""))
                && s["command"].as_object().is_none_or(|c| valid_command_name(c.get("name").and_then(Json::as_str).unwrap_or("")))
        })
        .collect();
    let aliases: std::collections::HashSet<&str> =
        skills.iter().filter_map(|s| s["command"]["name"].as_str()).collect();
    let mut rows = Vec::new();
    for c in commands {
        let name = c["name"].as_str().unwrap_or("");
        if !valid_name(name) || aliases.contains(name) {
            continue;
        }
        let hint = c["inputHint"].as_str().filter(|h| !h.is_empty());
        rows.push(Row {
            kind: "command",
            name: name.to_string(),
            label: format!("/{name}"),
            detail: invocation_detail(c["description"].as_str().unwrap_or(""), hint),
            link: link('/', name, &json!({ "kind": "command", "name": name })),
            plain: false,
            terminal_only: false,
            blocked: None,
            path: None,
            task: false,
        });
    }
    for s in skills {
        if s["enabled"].as_bool() != Some(true) {
            continue;
        }
        let name = s["name"].as_str().unwrap_or("");
        let path = s["path"].as_str().unwrap_or("");
        let description = s["description"].as_str().unwrap_or("");
        let mut invocation = json!({ "kind": "skill", "name": name, "path": path });
        if s["command"].is_object() {
            invocation["command"] = s["command"].clone();
        }
        rows.push(Row {
            kind: "skill",
            name: name.to_string(),
            label: skill_display_name(name),
            detail: if native_skill(path) || description.is_empty() {
                description.to_string()
            } else {
                format!("{description} — {path}")
            },
            link: link('$', name, &invocation),
            plain: false,
            terminal_only: false,
            blocked: None,
            path: Some(path.to_string()),
            task: false,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(link: &str) -> Json {
        let hex = link.split("zeron-invoke:").nth(1).unwrap().trim_end_matches(')');
        let bytes: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn commands_then_enabled_skills_a_skills_alias_left_to_it() {
        let commands = vec![
            json!({ "name": "review", "description": "Review the diff", "inputHint": "branch" }),
            json!({ "name": "deploy", "description": "" }),
            json!({ "name": "bad name", "description": "spaces" }),
        ];
        let skills = vec![
            json!({ "name": "code-review", "path": "/s/code-review/SKILL.md", "description": "Careful review", "enabled": true, "command": { "name": "deploy", "harness": "claude-code" } }),
            json!({ "name": "off", "path": "/s/off/SKILL.md", "description": "", "enabled": false }),
            json!({ "name": "native", "path": "harness-skill:native", "description": "Built in", "enabled": true }),
        ];
        let rows = candidates(&commands, &skills);
        let names: Vec<_> = rows.iter().map(|r| (r.kind, r.label.as_str())).collect();
        assert_eq!(names, vec![("command", "/review"), ("skill", "Code Review"), ("skill", "Native")]);
        assert_eq!(rows[0].detail, "Review the diff · <branch>");
        assert_eq!(rows[1].detail, "Careful review — /s/code-review/SKILL.md");
        assert_eq!(rows[2].detail, "Built in");
    }

    #[test]
    fn a_pick_carries_the_canonical_reference_the_engine_reads() {
        let rows = candidates(
            &[json!({ "name": "review" })],
            &[json!({ "name": "a[b]", "path": "/s/x.md", "description": "", "enabled": true })],
        );
        assert!(rows[0].link.starts_with("[/review](zeron-invoke:"));
        assert_eq!(decode(&rows[0].link), json!({ "kind": "command", "name": "review" }));
        assert!(rows[1].link.starts_with("[$a\\[b\\]](zeron-invoke:"));
        assert_eq!(decode(&rows[1].link), json!({ "kind": "skill", "name": "a[b]", "path": "/s/x.md" }));
    }

    /// Real lists from a live engine, when `ZGX_LIVE_INVOCATIONS` names a
    /// folder of `ListCommands-<harness>.json` and `ListSkills-<harness>.json`
    /// (an isolated engine's answers): every row merges, and every link reads
    /// back as the invocation it was made from.
    #[test]
    fn live_lists_merge_and_their_links_read_back() {
        let Some(dir) = std::env::var_os("ZGX_LIVE_INVOCATIONS") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        for harness in ["claude-code", "codex"] {
            let read = |m: &str| -> Json {
                serde_json::from_slice(&std::fs::read(dir.join(format!("{m}-{harness}.json"))).unwrap()).unwrap()
            };
            let (rows, failed) = merge(Ok(read("ListCommands")), Ok(read("ListSkills"))).unwrap();
            assert!(failed.is_none());
            let commands = rows.iter().filter(|r| r.kind == "command").count();
            let skills = rows.iter().filter(|r| r.kind == "skill").count();
            eprintln!("{harness}: {commands} commands, {skills} skills; first {:?}", rows.first().map(|r| (&r.label, &r.detail)));
            assert!(commands + skills > 0, "{harness} listed nothing");
            for r in &rows {
                let back = decode(&r.link);
                assert_eq!(back["name"], json!(r.name), "{}", r.link);
                assert_eq!(back["kind"], json!(r.kind));
            }
        }
    }

    #[test]
    fn one_list_failing_still_shows_the_other() {
        let commands = Ok(json!([{ "name": "review" }]));
        let (rows, failed) = merge(commands, Err("down".into())).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(failed, Some("skills"));
        assert!(merge(Err("down".into()), Ok(Json::Null)).is_err());
        let (rows, failed) = merge(Ok(json!([])), Ok(Json::Null)).unwrap();
        assert!(rows.is_empty() && failed.is_none());
    }

    fn menu(harness: HarnessId, listed: Json, unsupported: Option<&str>) -> Vec<Row> {
        let commands = with_zeron(Some(harness), listed).unwrap();
        let (rows, _) = merge(Ok(commands), Ok(Json::Null)).unwrap();
        flagged(rows, Some(harness), unsupported.map(str::to_owned))
    }

    #[test]
    fn the_menu_leads_with_zeron_s_own_and_leaves_computer_use_out() {
        let listed = json!([
            { "name": "review", "description": "Review the diff", "inputHint": "branch" },
            { "name": "computer-use", "description": "native" },
            { "name": "a-skill", "kind": "skill" },
        ]);
        let rows = menu(HarnessId::Codex, listed, None);
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["btw", "compact", "review"]);
        assert_eq!(rows[0].detail, "Side chat: ask about this chat without touching it \u{b7} <prompt>");
        assert_eq!(rows[2].detail, "Review the diff \u{b7} <branch>");
    }

    #[test]
    fn each_zeron_row_says_how_it_behaves() {
        let rows = menu(HarnessId::ClaudeCode, json!([{ "name": "review" }]), Some("Claude Code does not support compaction"));
        let row = |name: &str| rows.iter().find(|r| r.name == name).unwrap();
        assert!(row("btw").plain && row("compact").plain);
        assert_eq!(row("btw").link, "/btw", "inserted as typed, for the send to read");
        assert!(!row("review").plain && row("review").link.contains("zeron-invoke:"));
        assert_eq!(row("compact").blocked.as_deref(), Some("Claude Code does not support compaction"));
        assert_eq!(row("compact").detail, "Claude Code does not support compaction");
        assert!(row("review").blocked.is_none());
        let terminal = TERMINAL_ONLY_COMMANDS.iter().find(|n| rows.iter().any(|r| r.name == **n)).unwrap();
        assert!(row(terminal).terminal_only && !row("review").terminal_only);
        assert!(row("review").task, "the agent's own command goes into a task's words");
        assert!(!row("btw").task && !row("compact").task && !row(terminal).task, "Zeron's own and the terminal's don't");
        let rows = menu(HarnessId::Codex, json!([{ "name": "clear" }]), None);
        assert!(!rows.iter().find(|r| r.name == "clear").unwrap().task, "nor one that acts on a session");
        // The same name another agent runs itself isn't Claude's terminal's.
        let rows = menu(HarnessId::Codex, json!([{ "name": terminal }]), None);
        assert!(!rows.iter().find(|r| r.name == *terminal).unwrap().terminal_only);
    }
}
