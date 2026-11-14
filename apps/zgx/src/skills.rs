//! The Skills page's (ui/screens/skills): every agent's skills as the shipping
//! page lists them (crates/ui/src/shell/skills.rs): each offered agent's
//! `ListSkills`, byte-equal copies under one name grouped as one skill.
//!
//!   skills.list()          { harnesses = { { id, name, mark?, icon? } },
//!                            entries = { { name, title, path, paths, shown,
//!                            description, harnesses, on_disk, link, links } } }
//!   skills.read(path)      SKILL.md's text
//!   skills.open(path)      opened with the system; `skills.reveal(path)` shown
//!                          in its folder
//!   skills.trash(paths)    each skill's folder moved to the Trash
//!
//! Only a path the last list named is read, opened or moved. `link` is the
//! canonical reference a pick puts in a prompt (src/completion.rs);
//! `links` is that by agent, for one whose skill carries its own command.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
};

use serde_json::{Value as Json, json};
use zeron_proto::command_catalog::skill_display_name;
use zgx_script::{ScriptHost, Value};

use crate::engine;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    pub name: String,
    pub path: String,
    pub paths: Vec<String>,
    pub description: String,
    pub harnesses: Vec<String>,
    pub commands: BTreeMap<String, Json>,
}

pub fn install(host: &ScriptHost) {
    let known: Rc<RefCell<HashSet<String>>> = Rc::default();
    let k = known.clone();
    host.async_action("skills.list", move |_| {
        let k = k.clone();
        async move { list(k).await }
    });
    let k = known.clone();
    host.async_action("skills.read", move |arg| {
        let path = listed(&k, &arg);
        async move {
            let path = path?;
            std::fs::read_to_string(&path).map_err(|e| e.to_string()).map(|t| Value::from(t.as_str()))
        }
    });
    let k = known.clone();
    host.async_action("skills.open", move |arg| {
        let path = listed(&k, &arg);
        async move {
            crate::plugins::preview::refuse("skills.open")?;
            open::that_detached(path?).map_err(|e| e.to_string())?;
            Ok(Value::Nil)
        }
    });
    let k = known.clone();
    host.async_action("skills.reveal", move |arg| {
        let path = listed(&k, &arg);
        async move {
            crate::files::reveal_in_finder(Path::new(&path?)).map_err(|e| e.to_string())?;
            Ok(Value::Nil)
        }
    });
    let k = known;
    host.async_action("skills.trash", move |arg| {
        let paths: Result<Vec<String>, String> = match &arg {
            Value::List(list) => list.iter().map(|p| listed(&k, p)).collect(),
            _ => Err("no paths".into()),
        };
        let k = k.clone();
        async move {
            crate::plugins::preview::refuse("skills.trash")?;
            let home = std::env::var_os("HOME").map(PathBuf::from).ok_or("HOME is not set")?;
            for path in paths? {
                move_to_trash(Path::new(&path), &home.join(".Trash")).map_err(|e| e.to_string())?;
                k.borrow_mut().remove(&path);
            }
            Ok(Value::Nil)
        }
    });
}

fn listed(known: &RefCell<HashSet<String>>, arg: &Value) -> Result<String, String> {
    let path = crate::auth::text(arg);
    if known.borrow().contains(&path) { Ok(path) } else { Err("unknown skill".into()) }
}

async fn list(known: Rc<RefCell<HashSet<String>>>) -> Result<Value, String> {
    let mock = std::env::var("ZERON_ZGX_MOCK").is_ok_and(|v| v == "1");
    let listed = engine::get()
        .call("ListHarnesses", json!({ "includeRegistry": true }))
        .await
        .map_err(|e| engine::outcome(&e).to_string())?;
    let offered: Vec<(String, String)> = listed
        .as_array()
        .into_iter()
        .flatten()
        .filter(|h| {
            let id = h["id"].as_str().unwrap_or_default();
            if id == "mock" {
                return mock;
            }
            h["installed"].as_bool() == Some(true) && h["enabled"].as_bool() != Some(false)
        })
        .filter_map(|h| Some((h["id"].as_str()?.to_string(), h["name"].as_str().unwrap_or_default().to_string())))
        .collect();
    let mut per_harness = Vec::new();
    for (id, _) in &offered {
        let skills = engine::get().call("ListSkills", json!({ "harness": id })).await;
        if let Ok(Json::Array(skills)) = skills {
            per_harness.push((id.clone(), skills));
        }
    }
    let entries = group(per_harness.clone(), &|path| std::fs::read(path).ok());
    *known.borrow_mut() = entries.iter().flat_map(|e| e.paths.iter().cloned()).collect();
    let home = std::env::var("HOME").unwrap_or_default();
    let harnesses: Vec<Value> = offered
        .iter()
        .filter(|(id, _)| per_harness.iter().any(|(h, _)| h == id))
        .map(|(id, name)| {
            Value::record([
                ("id", Value::from(id.as_str())),
                ("name", Value::from(name.as_str())),
                ("mark", crate::agents::glyph(id).map_or(Value::Nil, |m| Value::from(m.as_str()))),
                ("icon", crate::chat::agent_icon(id).map_or(Value::Nil, |i| Value::from(i.as_str()))),
            ])
        })
        .collect();
    let entries: Vec<Value> = entries.iter().map(|e| entry_value(e, &home)).collect();
    Ok(Value::record([("harnesses", Value::List(harnesses.into())), ("entries", Value::List(entries.into()))]))
}

fn native(path: &str) -> bool {
    path.starts_with("opencode-skill:") || path.starts_with("harness-skill:")
}

/// The shipping page's `identity`: copies equal to the byte under one name are one skill.
fn identity(name: &str, path: &str, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> String {
    use std::hash::{Hash, Hasher};
    if native(path) {
        return path.to_string();
    }
    match read(path) {
        Some(bytes) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut hasher);
            format!("{name}\0{:016x}", hasher.finish())
        }
        None => path.to_string(),
    }
}

/// The shipping page's `group`, sorted by name.
pub(crate) fn group(per_harness: Vec<(String, Vec<Json>)>, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (harness, skills) in per_harness {
        for skill in skills {
            let (Some(name), Some(path)) = (skill["name"].as_str(), skill["path"].as_str()) else { continue };
            let command = skill["command"].is_object().then(|| skill["command"].clone());
            match index.get(&identity(name, path, read)) {
                Some(&at) => {
                    let entry = &mut entries[at];
                    if !entry.harnesses.contains(&harness) {
                        entry.harnesses.push(harness.clone());
                    }
                    if !entry.paths.iter().any(|p| p == path) {
                        entry.paths.push(path.to_string());
                    }
                    if let Some(command) = command {
                        entry.commands.insert(harness.clone(), command);
                    }
                }
                None => {
                    index.insert(identity(name, path, read), entries.len());
                    entries.push(Entry {
                        name: name.to_string(),
                        path: path.to_string(),
                        paths: vec![path.to_string()],
                        description: skill["description"].as_str().unwrap_or_default().to_string(),
                        harnesses: vec![harness.clone()],
                        commands: command.map(|c| BTreeMap::from([(harness.clone(), c)])).unwrap_or_default(),
                    });
                }
            }
        }
    }
    entries.sort_by_key(|e| e.name.to_lowercase());
    entries
}

/// `path` as the page shows it: its folder, home as `~`.
pub(crate) fn shown(path: &str, home: &str) -> String {
    let path = path.strip_suffix("/SKILL.md").unwrap_or(path);
    match path.strip_prefix(home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => path.to_string(),
    }
}

fn entry_value(e: &Entry, home: &str) -> Value {
    let link = |command: Option<&Json>| {
        zeron_proto::invocation::Invocation::Skill {
            name: e.name.clone(),
            path: e.path.clone(),
            command: command.and_then(|c| serde_json::from_value::<zeron_proto::invocation::SkillCommand>(c.clone()).ok()),
        }
        .link()
    };
    let links = e.commands.iter().map(|(h, c)| (h.as_str(), Value::from(link(Some(c)).as_str())));
    let strings = |list: &[String]| Value::List(list.iter().map(|s| Value::from(s.as_str())).collect::<Vec<_>>().into());
    Value::record([
        ("name", Value::from(e.name.as_str())),
        ("title", Value::from(skill_display_name(&e.name).as_str())),
        ("path", Value::from(e.path.as_str())),
        ("paths", strings(&e.paths)),
        ("shown", Value::from(shown(&e.path, home).as_str())),
        ("description", Value::from(e.description.as_str())),
        ("harnesses", strings(&e.harnesses)),
        ("on_disk", Value::Bool(!native(&e.path))),
        ("link", Value::from(link(None).as_str())),
        ("links", Value::record(links)),
    ])
}

/// The shipping page's `move_skill_to_trash`: SKILL.md's folder, under a free name.
pub(crate) fn move_to_trash(path: &Path, trash: &Path) -> std::io::Result<()> {
    let target = match path.parent() {
        Some(dir) if path.file_name().is_some_and(|name| name == "SKILL.md") => dir,
        _ => path,
    };
    if target.symlink_metadata().is_err() {
        return Ok(());
    }
    let name = target.file_name().ok_or_else(|| std::io::Error::other("the skill path has no name"))?;
    into_trash(target, trash, |n| format!("{} {n}", name.to_string_lossy()))
}

// The free name is reserved before the move, so a name taken in between is never overwritten.
pub(crate) fn into_trash(target: &Path, trash: &Path, nth: impl Fn(u32) -> String) -> std::io::Result<()> {
    let name = target.file_name().ok_or_else(|| std::io::Error::other("nothing to move to the Trash"))?;
    let folder = target.symlink_metadata()?.is_dir();
    std::fs::create_dir_all(trash)?;
    let mut n = 1;
    let dest = loop {
        let dest = if n == 1 { trash.join(name) } else { trash.join(nth(n)) };
        let reserved = if folder {
            std::fs::create_dir(&dest)
        } else {
            std::fs::OpenOptions::new().write(true).create_new(true).open(&dest).map(drop)
        };
        match reserved {
            Ok(()) => break dest,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    };
    std::fs::rename(target, &dest).inspect_err(|_| {
        let _ = if folder { std::fs::remove_dir(&dest) } else { std::fs::remove_file(&dest) };
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_copies_under_one_name_are_one_skill_across_agents() {
        let read = |path: &str| -> Option<Vec<u8>> {
            Some(match path {
                "/a/review/SKILL.md" | "/b/review/SKILL.md" => b"same".to_vec(),
                _ => path.as_bytes().to_vec(),
            })
        };
        let skill = |name: &str, path: &str| json!({ "name": name, "path": path, "description": format!("{name} it") });
        let entries = group(
            vec![
                ("claude-code".into(), vec![skill("review", "/a/review/SKILL.md"), skill("Zeta", "/a/zeta/SKILL.md")]),
                (
                    "codex".into(),
                    vec![
                        skill("review", "/b/review/SKILL.md"),
                        skill("zeta", "/b/zeta/SKILL.md"),
                        json!({ "name": "native", "path": "harness-skill:native", "description": "", "command": { "name": "native", "harness": "codex" } }),
                    ],
                ),
            ],
            &read,
        );
        let names: Vec<_> = entries.iter().map(|e| (e.name.as_str(), e.harnesses.len(), e.paths.len())).collect();
        assert_eq!(names, vec![("native", 1, 1), ("review", 2, 2), ("Zeta", 1, 1), ("zeta", 1, 1)]);
        assert_eq!(entries[1].paths, vec!["/a/review/SKILL.md", "/b/review/SKILL.md"]);
        assert!(entries[0].commands.contains_key("codex"));
        assert_eq!(shown("/home/ada/.claude/skills/review/SKILL.md", "/home/ada"), "~/.claude/skills/review");
        assert_eq!(shown("/srv/skills/x/SKILL.md", "/home/ada"), "/srv/skills/x");
    }

    #[test]
    fn a_skill_goes_to_the_trash_as_its_folder_under_a_free_name() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("Trash");
        for at in ["a", "b"] {
            let skill = dir.path().join(at).join("review");
            std::fs::create_dir_all(&skill).unwrap();
            std::fs::write(skill.join("SKILL.md"), at).unwrap();
            move_to_trash(&skill.join("SKILL.md"), &trash).unwrap();
            assert!(!skill.exists());
        }
        assert_eq!(std::fs::read_to_string(trash.join("review/SKILL.md")).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(trash.join("review 2/SKILL.md")).unwrap(), "b");
        move_to_trash(&dir.path().join("gone/SKILL.md"), &trash).unwrap();
    }
}
