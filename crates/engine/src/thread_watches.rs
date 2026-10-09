use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::EngineError;
use crate::project_actions::persist_json;

pub const MAX_WATCHED_THREADS: usize = 50;

const STORE_FILE: &str = "thread-watches.json";
const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadWatchesFile {
    version: u32,
    #[serde(default)]
    watches: BTreeMap<String, BTreeSet<String>>,
}

struct ThreadWatchesInner {
    path: PathBuf,
    state: Mutex<ThreadWatchesFile>,
}

#[derive(Clone)]
pub struct ThreadWatches {
    inner: Arc<ThreadWatchesInner>,
}

impl ThreadWatches {
    pub fn open(profile_store_root: &Path) -> Result<Self, EngineError> {
        std::fs::create_dir_all(profile_store_root)?;
        let path = profile_store_root.join(STORE_FILE);
        let state = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<ThreadWatchesFile>(&bytes) {
                Ok(file) if file.version == STORE_VERSION => file,
                Ok(_) | Err(_) => {
                    tracing::warn!(path = %path.display(), "unreadable thread watches; starting empty");
                    empty_file()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => empty_file(),
            Err(err) => return Err(err.into()),
        };
        Ok(Self {
            inner: Arc::new(ThreadWatchesInner {
                path,
                state: Mutex::new(state),
            }),
        })
    }

    pub fn watched_by(&self, watcher: &str) -> Vec<String> {
        lock(&self.inner.state)
            .watches
            .get(watcher)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn watchers_of(&self, chat_id: &str) -> Vec<String> {
        lock(&self.inner.state)
            .watches
            .iter()
            .filter(|(_, watched)| watched.contains(chat_id))
            .map(|(watcher, _)| watcher.clone())
            .collect()
    }

    pub fn change(
        &self,
        watcher: &str,
        add: &[String],
        remove: &[String],
        live: impl Fn(&str) -> bool,
    ) -> Result<Vec<String>, EngineError> {
        let mut state = lock(&self.inner.state);
        let mut next = state.clone();
        next.watches.retain(|watcher, watched| {
            watched.retain(|id| live(id));
            live(watcher) && !watched.is_empty()
        });
        let watched = next.watches.entry(watcher.to_string()).or_default();
        for id in remove {
            watched.remove(id);
        }
        watched.extend(add.iter().cloned());
        if watched.len() > MAX_WATCHED_THREADS {
            return Err(EngineError::Other(format!(
                "a chat may watch at most {MAX_WATCHED_THREADS} threads"
            )));
        }
        let now: Vec<String> = watched.iter().cloned().collect();
        if now.is_empty() {
            next.watches.remove(watcher);
        }
        persist_json(&self.inner.path, &next)?;
        *state = next;
        Ok(now)
    }
}

fn empty_file() -> ThreadWatchesFile {
    ThreadWatchesFile {
        version: STORE_VERSION,
        watches: BTreeMap::new(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watches_survive_reopen_and_drop_gone_chats() {
        let dir = tempfile::tempdir().unwrap();
        let watches = ThreadWatches::open(dir.path()).unwrap();
        let ids = |list: &[&str]| list.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        watches
            .change("lead", &ids(&["a", "b"]), &[], |_| true)
            .unwrap();
        watches
            .change("other", &ids(&["a"]), &[], |_| true)
            .unwrap();
        let reopened = ThreadWatches::open(dir.path()).unwrap();
        assert_eq!(reopened.watched_by("lead"), ids(&["a", "b"]));
        let mut watchers = reopened.watchers_of("a");
        watchers.sort();
        assert_eq!(watchers, ids(&["lead", "other"]));

        let now = reopened
            .change("lead", &[], &ids(&["a"]), |id| id != "other")
            .unwrap();
        assert_eq!(now, ids(&["b"]));
        assert!(reopened.watchers_of("a").is_empty());
        assert!(reopened.watched_by("other").is_empty());
    }
}
