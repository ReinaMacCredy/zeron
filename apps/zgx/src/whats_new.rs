use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};
use zeron_docs::changelog::{self, Entry, Resolution};
use zgx_script::ScriptHost;

use crate::about::APP_VERSION;

const SEEN: &str = "whatsNewSeenVersion";

fn boot<'a>(entries: &'a [Entry], current: &str, settings: &Path) -> Option<&'a Entry> {
    let seen = crate::settings::read_settings_at(settings).ok()?;
    let seen = seen.as_ref().and_then(|s| s[SEEN].as_str());
    match changelog::resolve(entries, current, seen) {
        Resolution::Show(entry) => Some(entry),
        Resolution::Advance => {
            mark_seen(current, settings);
            None
        }
        Resolution::Noop => None,
    }
}

fn mark_seen(current: &str, settings: &Path) {
    let Ok(file) = crate::settings::read_settings_at(settings) else { return };
    let seen = file.as_ref().and_then(|s| s[SEEN].as_str());
    if seen.is_none_or(|seen| zeron_update::version_newer(current, seen)) {
        let mut change = serde_json::Map::new();
        change.insert(SEEN.into(), json!(current));
        if crate::settings::write_settings_at(settings, change).is_err() {
            eprintln!("zeron-zgx: whats new: {} couldn't be written", settings.display());
        }
    }
}

fn picture(name: &str, dir: &Path) -> Option<String> {
    let (_, bytes) = zeron_docs::images::IMAGES.iter().find(|(n, _)| *n == name)?;
    let path = dir.join(name);
    if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(bytes.len() as u64) {
        std::fs::create_dir_all(dir).ok()?;
        std::fs::write(&path, bytes).ok()?;
    }
    Some(path.display().to_string())
}

fn entry_json(entry: &Entry, pictures: Option<&Path>) -> Json {
    let image = |name: Option<&String>| pictures.zip(name).and_then(|(dir, name)| picture(name, dir));
    json!({
        "version": entry.version,
        "date": entry.date,
        "hero": image(entry.hero.as_ref()),
        "features": entry.features.iter().map(|f| json!({
            "id": f.id,
            "title": f.title,
            "description": f.description,
            "details": f.details,
            "image": image(f.image.as_ref()),
        })).collect::<Vec<_>>(),
        "improvements": entry.improvements.iter().map(|i| i.text.clone()).collect::<Vec<_>>(),
    })
}

fn state(entries: &[Entry], current: &str, card: Option<&Entry>, pictures: &Path) -> Json {
    let released = changelog::released(entries, current);
    let highlights = changelog::entry_for(released, current).or(released.first());
    json!({
        "version": current,
        "card": card.map(|e| entry_json(e, Some(pictures))),
        "highlights": highlights.map(|e| entry_json(e, Some(pictures))),
        "releases": released.iter().map(|e| entry_json(e, None)).collect::<Vec<_>>(),
    })
}

fn settings_file() -> Option<PathBuf> {
    crate::engine::data_dir().map(|dir| dir.join("ui-settings.json"))
}

pub fn install(host: &ScriptHost, app_id: &str) {
    let entries = changelog::entries();
    let pictures = zgx_script::app_data_dir(app_id).map(|dir| dir.join("whats-new")).unwrap_or_default();
    let settings = settings_file();
    let card = settings.as_deref().and_then(|file| boot(entries, APP_VERSION, file));
    host.shared("whats_new", crate::plugins::json_value(&state(entries, APP_VERSION, card, &pictures)));
    host.action("whats_new.seen", move |_| {
        if let Some(file) = settings.as_deref() {
            mark_seen(APP_VERSION, file);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRIES: &str = r#"
[[entry]]
version = "0.3.0"
date = "Nov 20"
[[entry.features]]
id = "zgx"
title = "A new interface"
description = "Faster."
[[entry]]
version = "0.2.132"
date = "Nov 1"
[[entry.features]]
id = "tabs"
title = "Tabs"
description = "Drag them."
"#;

    #[test]
    fn the_card_shows_once_and_its_seen_version_is_the_other_interface_s_key() {
        let entries = changelog::parse(ENTRIES).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ui-settings.json");
        std::fs::write(&file, r#"{"whatsNewSeenVersion":"0.2.132","theme":"dark"}"#).unwrap();
        assert_eq!(boot(&entries, "0.3.0", &file).map(|e| e.version.as_str()), Some("0.3.0"));
        mark_seen("0.3.0", &file);
        let kept: Json = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(kept, json!({ "whatsNewSeenVersion": "0.3.0", "theme": "dark" }));
        assert!(boot(&entries, "0.3.0", &file).is_none());
        mark_seen("0.2.132", &file);
        assert_eq!(serde_json::from_slice::<Json>(&std::fs::read(&file).unwrap()).unwrap()[SEEN], json!("0.3.0"));
    }

    #[test]
    fn a_fresh_install_shows_nothing_and_records_its_version() {
        let entries = changelog::parse(ENTRIES).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ui-settings.json");
        assert!(boot(&entries, "0.3.0", &file).is_none());
        assert_eq!(serde_json::from_slice::<Json>(&std::fs::read(&file).unwrap()).unwrap()[SEEN], json!("0.3.0"));
    }

    #[test]
    fn the_scripts_read_the_released_entries_newest_first() {
        let entries = changelog::parse(ENTRIES).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let read = state(&entries, "0.2.132", None, dir.path());
        assert_eq!(read["highlights"]["version"], json!("0.2.132"));
        assert_eq!(read["releases"].as_array().unwrap().len(), 1);
        assert_eq!(read["card"], Json::Null);
    }
}
