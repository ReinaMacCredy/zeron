//! Persisted composer drafts (t3code `composerDraftStore`, localStorage key
//! `t3code:composer-drafts:v1`): the text, staged images and appshots a
//! composer holds per chat key, restored after a restart. Own file beside
//! `ui-settings.json` (d19) — that file is rewritten on every layout change
//! and a draft can run to tens of thousands of characters — written debounced
//! and atomically. Only keys with user content are on disk (d20); image bytes
//! live under `composer-drafts/` and the JSON carries their paths (d22 as it
//! applies here: uploads only happen at send, so a path IS the committed ref).
//!
//! One process-wide store: every composer (the main one and each sidechat's)
//! writes its own keys in, so no instance can erase another's entries.

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use gpui::{App, Global, Image, Task};
use serde::{Deserialize, Serialize};

use crate::appshots::{AccessibilitySnapshot, CapturedAppshot};
use crate::attachments::{StagedAttachment, format_by_extension};

const FILE_NAME: &str = "composer-drafts.json";
const DIR_NAME: &str = "composer-drafts";
const VERSION: u32 = 1;
const SAVE_DEBOUNCE_MS: u64 = 300;

/// Everything a composer key holds that counts as user content.
#[derive(Clone, Default)]
pub struct DraftEntry {
    pub prompt: String,
    pub attachments: Vec<StagedAttachment>,
    pub appshots: Vec<CapturedAppshot>,
}

impl DraftEntry {
    /// t3code `composerDraftHasUserContent`: typed text or any attachment.
    /// Model and mode picks alone are ambient defaults, not work in progress.
    pub fn has_content(&self) -> bool {
        !self.prompt.trim().is_empty() || !self.attachments.is_empty() || !self.appshots.is_empty()
    }
}

/// A pre-session draft (t3code `DraftSessionState`): the target a new
/// session will be minted onto, mutable until the first send. `id` IS the
/// chat id that send mints, so promotion re-keys nothing — the tab, the
/// composer entry and the panel flags all keep their string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DraftSession {
    pub id: String,
    pub space_id: Option<String>,
    pub no_project: bool,
    pub device_id: Option<String>,
    /// The picked ref (`None` = the checkout's current branch).
    pub branch: Option<String>,
    /// "New worktree" mode (t3code env-mode `worktree`).
    pub new_worktree: bool,
    pub created_at: DateTime<Utc>,
}

impl Default for DraftSession {
    fn default() -> Self {
        Self {
            id: String::new(),
            space_id: None,
            no_project: false,
            device_id: None,
            branch: None,
            new_worktree: false,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct DraftFile {
    version: u32,
    drafts: HashMap<String, PersistedEntry>,
    /// Only sessions whose entry has content (t3code partialize: everything
    /// else is a zombie and is re-minted fresh on the next launch).
    sessions: Vec<DraftSession>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct PersistedEntry {
    prompt: String,
    attachments: Vec<PersistedAttachment>,
    appshots: Vec<PersistedAppshot>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedAttachment {
    id: String,
    name: String,
    path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedAppshot {
    id: String,
    app_name: String,
    bundle_identifier: Option<String>,
    window_title: Option<String>,
    accessibility_format_version: u32,
    accessibility_content: String,
    accessibility_truncated: bool,
    screenshot: PersistedAttachment,
    screenshot_dimensions: Option<(u32, u32)>,
    captured_at: DateTime<Utc>,
}

pub struct DraftStore {
    data_dir: Option<PathBuf>,
    entries: HashMap<String, DraftEntry>,
    sessions: HashMap<String, DraftSession>,
    revision: u64,
    saved_revision: Arc<Mutex<u64>>,
    save_task: Option<Task<()>>,
}

impl Global for DraftStore {}

#[cfg(test)]
pub fn init(data_dir: impl Into<PathBuf>, cx: &mut App) {
    let data_dir = data_dir.into();
    let loaded = load(&data_dir);
    init_loaded(data_dir, loaded, cx);
}

pub(crate) fn init_loaded(
    data_dir: PathBuf,
    (entries, sessions): (HashMap<String, DraftEntry>, HashMap<String, DraftSession>),
    cx: &mut App,
) {
    cx.set_global(DraftStore {
        data_dir: Some(data_dir),
        entries,
        sessions,
        revision: 0,
        saved_revision: Arc::new(Mutex::new(0)),
        save_task: None,
    });
}

/// Restored draft sessions, oldest first.
pub fn sessions(cx: &App) -> Vec<DraftSession> {
    let mut sessions: Vec<DraftSession> = cx
        .try_global::<DraftStore>()
        .map(|store| store.sessions.values().cloned().collect())
        .unwrap_or_default();
    sessions.sort_by_key(|s| s.created_at);
    sessions
}

pub fn set_session(session: DraftSession, cx: &mut App) {
    if !cx.has_global::<DraftStore>() {
        return;
    }
    {
        let store = cx.global_mut::<DraftStore>();
        if store.sessions.get(&session.id) == Some(&session) {
            return;
        }
        store.sessions.insert(session.id.clone(), session);
        store.revision += 1;
    }
    schedule(cx);
}

pub fn remove_session(id: &str, cx: &mut App) {
    if !cx.has_global::<DraftStore>() {
        return;
    }
    let removed = {
        let store = cx.global_mut::<DraftStore>();
        let removed = store.sessions.remove(id).is_some();
        store.revision += u64::from(removed);
        removed
    };
    if removed {
        schedule(cx);
    }
}

pub(crate) fn ensure(cx: &mut App) {
    if !cx.has_global::<DraftStore>() {
        cx.set_global(DraftStore {
            data_dir: None,
            entries: HashMap::new(),
            sessions: HashMap::new(),
            revision: 0,
            saved_revision: Arc::new(Mutex::new(0)),
            save_task: None,
        });
    }
}

pub fn entry(key: &str, cx: &App) -> DraftEntry {
    cx.try_global::<DraftStore>()
        .and_then(|store| store.entries.get(key).cloned())
        .unwrap_or_default()
}

pub fn has_content(key: &str, cx: &App) -> bool {
    cx.try_global::<DraftStore>()
        .and_then(|store| store.entries.get(key))
        .is_some_and(DraftEntry::has_content)
}

/// The row text for a draft with content: its prompt, or a stand-in when the
/// content is attachments alone.
pub fn entry_prompt(key: &str, cx: &App) -> Option<String> {
    let entry = cx.try_global::<DraftStore>()?.entries.get(key)?;
    if !entry.has_content() {
        return None;
    }
    let prompt = entry.prompt.trim();
    Some(if prompt.is_empty() {
        format!(
            "{} attachment{}",
            entry.attachments.len() + entry.appshots.len(),
            if entry.attachments.len() + entry.appshots.len() == 1 {
                ""
            } else {
                "s"
            }
        )
    } else {
        prompt.to_string()
    })
}

/// Replace `key`'s entry (dropping it when empty). Returns whether the key's
/// has-content answer flipped, so callers can repaint the surfaces that show
/// it (the sidebar pen) on the transition only, never per keystroke.
pub fn set(key: &str, entry: DraftEntry, cx: &mut App) -> bool {
    if !cx.has_global::<DraftStore>() {
        return false;
    }
    let flipped = {
        let store = cx.global_mut::<DraftStore>();
        let before = store.entries.get(key).is_some_and(DraftEntry::has_content);
        let after = entry.has_content();
        if after {
            store.entries.insert(key.to_string(), entry);
        } else {
            store.entries.remove(key);
        }
        store.revision += 1;
        before != after
    };
    schedule(cx);
    flipped
}

pub fn remove(key: &str, cx: &mut App) -> bool {
    set(key, DraftEntry::default(), cx)
}

/// Drop every entry `keep` rejects (chats that no longer exist). Callers
/// must only judge once the chat list has synced, or the pre-sync empty
/// list would erase every draft.
pub fn prune(keep: impl Fn(&str) -> bool, cx: &mut App) {
    if !cx.has_global::<DraftStore>() {
        return;
    }
    let removed = {
        let store = cx.global_mut::<DraftStore>();
        let before = store.entries.len() + store.sessions.len();
        store.entries.retain(|key, _| keep(key));
        store.sessions.retain(|key, _| keep(key));
        let removed = store.entries.len() + store.sessions.len() != before;
        store.revision += u64::from(removed);
        removed
    };
    if removed {
        schedule(cx);
    }
}

fn schedule(cx: &mut App) {
    let old = cx.global_mut::<DraftStore>().save_task.take();
    drop(old);
    let task = cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(SAVE_DEBOUNCE_MS))
            .await;
        cx.update(|cx| flush(cx).detach());
    });
    cx.global_mut::<DraftStore>().save_task = Some(task);
}

/// Save a snapshot off the app thread. Await the task at shutdown.
pub fn flush(cx: &mut App) -> Task<()> {
    if !cx.has_global::<DraftStore>() {
        return Task::ready(());
    }
    let store = cx.global_mut::<DraftStore>();
    store.save_task = None;
    let revision = store.revision;
    let saved_revision = store.saved_revision.clone();
    let Some(data_dir) = store.data_dir.clone() else {
        return Task::ready(());
    };
    let entries = store.entries.clone();
    let sessions = store.sessions.clone();
    cx.background_executor().spawn(async move {
        // Serialize writes and orphan cleanup. A late older task must not
        // overwrite a newer snapshot (including the shutdown snapshot).
        let mut saved = saved_revision.lock().unwrap_or_else(|err| err.into_inner());
        if revision <= *saved {
            return;
        }
        match save(&data_dir, &entries, &sessions) {
            Ok(()) => *saved = revision,
            Err(err) => tracing::warn!(error = %err, "composer-drafts save failed"),
        }
    })
}

pub(crate) fn load(
    data_dir: &Path,
) -> (HashMap<String, DraftEntry>, HashMap<String, DraftSession>) {
    let text = match std::fs::read_to_string(data_dir.join(FILE_NAME)) {
        Ok(text) => text,
        Err(_) => return Default::default(),
    };
    let file: DraftFile = match serde_json::from_str(&text) {
        Ok(file) => file,
        Err(err) => {
            tracing::warn!(error = %err, "composer-drafts corrupt; starting empty");
            return Default::default();
        }
    };
    let entries: HashMap<String, DraftEntry> = file
        .drafts
        .into_iter()
        .map(|(key, entry)| (key, restore_entry(entry)))
        .filter(|(_, entry)| entry.has_content())
        .collect();
    // A session whose content did not survive (every image file gone) is a
    // zombie; the next launch mints a fresh one instead.
    let sessions = file
        .sessions
        .into_iter()
        .filter(|s| !s.id.is_empty() && entries.contains_key(&s.id))
        .map(|s| (s.id.clone(), s))
        .collect();
    (entries, sessions)
}

/// A referenced image whose file is gone (a manual clean of the data dir)
/// drops out with a warning rather than blocking the whole draft.
fn restore_entry(entry: PersistedEntry) -> DraftEntry {
    DraftEntry {
        prompt: entry.prompt,
        attachments: entry
            .attachments
            .into_iter()
            .filter_map(restore_attachment)
            .collect(),
        appshots: entry
            .appshots
            .into_iter()
            .filter_map(|shot| {
                let screenshot = restore_attachment(shot.screenshot)?;
                Some(CapturedAppshot {
                    id: shot.id,
                    app_name: shot.app_name,
                    bundle_identifier: shot.bundle_identifier,
                    window_title: shot.window_title,
                    accessibility: AccessibilitySnapshot {
                        format_version: shot.accessibility_format_version,
                        content: shot.accessibility_content,
                        truncated: shot.accessibility_truncated,
                    },
                    screenshot,
                    screenshot_dimensions: shot.screenshot_dimensions,
                    app_icon: None,
                    captured_at: shot.captured_at,
                })
            })
            .collect(),
    }
}

fn restore_attachment(persisted: PersistedAttachment) -> Option<StagedAttachment> {
    let format = format_by_extension(&persisted.path)?;
    match std::fs::read(&persisted.path) {
        Ok(bytes) => Some(StagedAttachment {
            id: persisted.id,
            name: persisted.name,
            image: Arc::new(Image::from_bytes(format, bytes)),
        }),
        Err(err) => {
            tracing::warn!(error = %err, path = %persisted.path.display(), "draft attachment missing; dropped");
            None
        }
    }
}

fn save(
    data_dir: &Path,
    entries: &HashMap<String, DraftEntry>,
    sessions: &HashMap<String, DraftSession>,
) -> io::Result<()> {
    let dir = data_dir.join(DIR_NAME);
    std::fs::create_dir_all(&dir)?;
    let mut referenced: Vec<PathBuf> = Vec::new();
    let mut file = DraftFile {
        version: VERSION,
        drafts: HashMap::new(),
        sessions: sessions
            .values()
            .filter(|s| entries.get(&s.id).is_some_and(DraftEntry::has_content))
            .cloned()
            .collect(),
    };
    file.sessions.sort_by_key(|s| s.created_at);
    for (key, entry) in entries {
        let attachments = entry
            .attachments
            .iter()
            .map(|att| persist_attachment(&dir, att, &mut referenced))
            .collect::<io::Result<Vec<_>>>()?;
        let appshots = entry
            .appshots
            .iter()
            .map(|shot| {
                Ok(PersistedAppshot {
                    id: shot.id.clone(),
                    app_name: shot.app_name.clone(),
                    bundle_identifier: shot.bundle_identifier.clone(),
                    window_title: shot.window_title.clone(),
                    accessibility_format_version: shot.accessibility.format_version,
                    accessibility_content: shot.accessibility.content.clone(),
                    accessibility_truncated: shot.accessibility.truncated,
                    screenshot: persist_attachment(&dir, &shot.screenshot, &mut referenced)?,
                    screenshot_dimensions: shot.screenshot_dimensions,
                    captured_at: shot.captured_at,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        file.drafts.insert(
            key.clone(),
            PersistedEntry {
                prompt: entry.prompt.clone(),
                attachments,
                appshots,
            },
        );
    }
    write_atomic(
        &data_dir.join(FILE_NAME),
        &serde_json::to_vec_pretty(&file)?,
    )?;
    // Orphans: images a draft no longer references (removed, sent, discarded).
    if let Ok(read) = std::fs::read_dir(&dir) {
        for existing in read.flatten() {
            let path = existing.path();
            if !referenced.contains(&path) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    Ok(())
}

/// Bytes are written once per attachment id; a later save that still
/// references the id only re-lists it.
fn persist_attachment(
    dir: &Path,
    att: &StagedAttachment,
    referenced: &mut Vec<PathBuf>,
) -> io::Result<PersistedAttachment> {
    let ext = Path::new(&att.name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png");
    let path = dir.join(format!("{}.{ext}", att.id));
    if !path.exists() {
        write_atomic(&path, att.bytes())?;
    }
    referenced.push(path.clone());
    Ok(PersistedAttachment {
        id: att.id.clone(),
        name: att.name.clone(),
        path,
    })
}

/// Temp file + rename so a crash mid-write never leaves a torn file.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::ImageFormat;

    fn png() -> StagedAttachment {
        // 1x1 PNG header is enough: nothing decodes it here.
        StagedAttachment {
            id: "att-1".into(),
            name: "shot.png".into(),
            image: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                vec![0x89, b'P', b'N', b'G'],
            )),
        }
    }

    #[gpui::test]
    async fn background_saves_keep_the_latest_snapshot(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| init(dir.path(), cx));
        let saved = cx.update(|cx| cx.global::<DraftStore>().saved_revision.clone());
        let blocked = saved.lock().unwrap();
        let first = cx.update(|cx| {
            set(
                "chat",
                DraftEntry {
                    prompt: "old".into(),
                    attachments: vec![png()],
                    ..Default::default()
                },
                cx,
            );
            flush(cx)
        });
        let latest = cx.update(|cx| {
            set(
                "chat",
                DraftEntry {
                    prompt: "latest".into(),
                    ..Default::default()
                },
                cx,
            );
            flush(cx)
        });
        // Both app-thread calls returned while the disk writer was blocked.
        drop(blocked);
        latest.await;
        first.await;
        assert_eq!(load(dir.path()).0["chat"].prompt, "latest");
        assert!(!dir.path().join(DIR_NAME).join("att-1.png").exists());
    }

    #[gpui::test]
    async fn a_failed_save_is_retried_at_shutdown(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| init(dir.path(), cx));
        std::fs::create_dir(dir.path().join(FILE_NAME)).unwrap();
        cx.update(|cx| {
            set(
                "chat",
                DraftEntry {
                    prompt: "keep me".into(),
                    ..Default::default()
                },
                cx,
            );
            flush(cx)
        })
        .await;
        std::fs::remove_dir(dir.path().join(FILE_NAME)).unwrap();
        cx.update(flush).await;
        assert_eq!(load(dir.path()).0["chat"].prompt, "keep me");
    }

    #[test]
    fn round_trips_prompt_and_attachment_paths() {
        let dir = tempfile::tempdir().unwrap();
        let mut entries = HashMap::new();
        entries.insert(
            "chat-1".to_string(),
            DraftEntry {
                prompt: "hello".into(),
                attachments: vec![png()],
                appshots: Vec::new(),
            },
        );
        save(dir.path(), &entries, &HashMap::new()).unwrap();
        let (restored, _) = load(dir.path());
        let entry = restored.get("chat-1").expect("entry restored");
        assert_eq!(entry.prompt, "hello");
        assert_eq!(entry.attachments.len(), 1);
        assert_eq!(entry.attachments[0].id, "att-1");
        assert_eq!(entry.attachments[0].bytes(), &[0x89, b'P', b'N', b'G']);
    }

    #[test]
    fn sessions_persist_only_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut entries = HashMap::new();
        entries.insert(
            "draft-a".to_string(),
            DraftEntry {
                prompt: "invested".into(),
                attachments: Vec::new(),
                appshots: Vec::new(),
            },
        );
        let mut sessions = HashMap::new();
        for id in ["draft-a", "draft-empty"] {
            sessions.insert(
                id.to_string(),
                DraftSession {
                    id: id.into(),
                    space_id: Some("space-1".into()),
                    branch: Some("main".into()),
                    new_worktree: true,
                    ..Default::default()
                },
            );
        }
        save(dir.path(), &entries, &sessions).unwrap();
        let (_, restored) = load(dir.path());
        assert_eq!(restored.len(), 1);
        let session = restored.get("draft-a").unwrap();
        assert_eq!(session.space_id.as_deref(), Some("space-1"));
        assert_eq!(session.branch.as_deref(), Some("main"));
        assert!(session.new_worktree);
    }

    #[test]
    fn empty_entries_are_not_written_and_orphan_images_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let mut entries = HashMap::new();
        entries.insert(
            "chat-1".to_string(),
            DraftEntry {
                prompt: String::new(),
                attachments: vec![png()],
                appshots: Vec::new(),
            },
        );
        save(dir.path(), &entries, &HashMap::new()).unwrap();
        assert!(dir.path().join(DIR_NAME).join("att-1.png").exists());
        entries.clear();
        save(dir.path(), &entries, &HashMap::new()).unwrap();
        assert!(!dir.path().join(DIR_NAME).join("att-1.png").exists());
        let text = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        let file: DraftFile = serde_json::from_str(&text).unwrap();
        assert!(file.drafts.is_empty());
        assert_eq!(file.version, VERSION);
    }

    #[test]
    fn missing_image_file_drops_the_attachment_not_the_draft() {
        let dir = tempfile::tempdir().unwrap();
        let mut entries = HashMap::new();
        entries.insert(
            "chat-1".to_string(),
            DraftEntry {
                prompt: "text".into(),
                attachments: vec![png()],
                appshots: Vec::new(),
            },
        );
        save(dir.path(), &entries, &HashMap::new()).unwrap();
        std::fs::remove_file(dir.path().join(DIR_NAME).join("att-1.png")).unwrap();
        let (restored, _) = load(dir.path());
        let entry = restored.get("chat-1").unwrap();
        assert_eq!(entry.prompt, "text");
        assert!(entry.attachments.is_empty());
    }

    #[test]
    fn corrupt_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), "{not json").unwrap();
        assert!(load(dir.path()).0.is_empty());
    }
}
