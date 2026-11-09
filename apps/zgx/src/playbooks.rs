//! Settings > Playbooks (ui/screens/settings/playbooks.luau), as the shipping
//! page (crates/ui/src/shell/playbooks.rs), through the engine's
//! `ListPlaybooks` and `CreatePlaybook`:
//!
//!   playbooks.list()          { folder, playbooks = { { id, title, question,
//!                               subtitle?, source, root?, path?, prompt? } } }
//!   playbooks.create()        a new one in the folder, opened with the system: its path
//!   playbooks.triage()        a triage folder made (the source cloned beside it): the draft's link
//!   playbooks.open(path)      opened with the system (Edit)
//!   playbooks.reveal(folder)  the folder made and shown in Finder
//!   playbooks.trash(path)     moved to the Trash
//!
//! Only a path or folder the last list named is opened, shown or moved.

use std::{
    cell::RefCell,
    collections::HashSet,
    path::{Path, PathBuf},
    rc::Rc,
};

use serde_json::json;
use zgx_script::{ScriptHost, Value};

use crate::engine;

pub fn install(host: &ScriptHost) {
    let known: Rc<RefCell<HashSet<String>>> = Rc::default();
    let k = known.clone();
    host.async_action("playbooks.list", move |_| {
        let k = k.clone();
        async move {
            let listed = engine::get().call("ListPlaybooks", json!({})).await.map_err(|e| failed(&e))?;
            *k.borrow_mut() = named(&listed);
            Ok(crate::plugins::json_value(&listed))
        }
    });
    host.async_action("playbooks.create", |_| async move {
        crate::plugins::preview::refuse("playbooks.create")?;
        let made = engine::get().call("CreatePlaybook", json!({})).await.map_err(|e| failed(&e))?;
        let path = made["path"].as_str().ok_or("failed")?.to_owned();
        open::that_detached(&path).map_err(|e| e.to_string())?;
        Ok(Value::from(path.as_str()))
    });
    host.async_action("playbooks.triage", |_| async move {
        crate::plugins::preview::refuse("playbooks.triage")?;
        let started = engine::get().call("StartTriage", json!({})).await.map_err(|e| failed(&e))?;
        Ok(Value::from(started["prompt"].as_str().ok_or("failed")?))
    });
    let k = known.clone();
    host.async_action("playbooks.open", move |arg| {
        let path = listed(&k, &arg);
        async move {
            crate::plugins::preview::refuse("playbooks.open")?;
            open::that_detached(path?).map_err(|e| e.to_string())?;
            Ok(Value::Nil)
        }
    });
    let k = known.clone();
    host.async_action("playbooks.reveal", move |arg| {
        let path = listed(&k, &arg);
        async move {
            let folder = PathBuf::from(path?);
            std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
            crate::files::reveal_in_finder(&folder).map_err(|e| e.to_string())?;
            Ok(Value::Nil)
        }
    });
    let k = known;
    host.async_action("playbooks.trash", move |arg| {
        let path = listed(&k, &arg);
        let k = k.clone();
        async move {
            crate::plugins::preview::refuse("playbooks.trash")?;
            let path = path?;
            let home = std::env::var_os("HOME").map(PathBuf::from).ok_or("HOME is not set")?;
            trash(Path::new(&path), &home.join(".Trash")).map_err(|e| e.to_string())?;
            k.borrow_mut().remove(&path);
            Ok(Value::Nil)
        }
    });
}

fn named(listed: &serde_json::Value) -> HashSet<String> {
    let rows = listed["playbooks"].as_array().into_iter().flatten();
    rows.filter_map(|row| row["path"].as_str())
        .chain(listed["folder"].as_str())
        .map(str::to_owned)
        .collect()
}

fn listed(known: &RefCell<HashSet<String>>, arg: &Value) -> Result<String, String> {
    let path = crate::auth::text(arg);
    if known.borrow().contains(&path) { Ok(path) } else { Err("unknown playbook".into()) }
}

fn failed(error: &str) -> String {
    if error.to_ascii_lowercase().contains("unknown method") { "outdated".into() } else { engine::outcome(error).into() }
}

fn trash(path: &Path, trash: &Path) -> std::io::Result<()> {
    if !path.is_file() {
        return Err(std::io::Error::other("only a playbook's file goes to the Trash"));
    }
    let name = path.file_name().ok_or_else(|| std::io::Error::other("the playbook has no name"))?;
    std::fs::create_dir_all(trash)?;
    let mut dest = trash.join(name);
    let mut n = 2;
    while dest.symlink_metadata().is_ok() {
        dest = trash.join(format!("{n} {}", name.to_string_lossy()));
        n += 1;
    }
    std::fs::rename(path, dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_the_list_named_is_touched_and_a_file_goes_to_the_trash_under_a_free_name() {
        let listed = json!({ "folder": "/p", "playbooks": [{ "path": "/p/a.md" }, { "path": null }] });
        let known = RefCell::new(named(&listed));
        assert_eq!(listed_ok(&known, "/p/a.md"), true);
        assert_eq!(listed_ok(&known, "/p"), true);
        assert_eq!(listed_ok(&known, "/etc/passwd"), false);

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("Trash");
        for _ in 0..2 {
            let file = dir.path().join("a.md");
            std::fs::write(&file, "x").unwrap();
            trash(&file, &bin).unwrap();
            assert!(!file.exists());
        }
        assert!(bin.join("a.md").is_file() && bin.join("2 a.md").is_file());
        assert!(trash(dir.path(), &bin).is_err(), "never a folder");
    }

    fn listed_ok(known: &RefCell<HashSet<String>>, path: &str) -> bool {
        listed(known, &Value::from(path)).is_ok()
    }
}
