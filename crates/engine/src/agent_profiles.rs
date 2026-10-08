use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use zeron_proto::{AgentProfile, AgentProfileDraft, AgentProfilesSnapshot};

use crate::EngineError;
use crate::project_actions::{persist_json, unique_slug_id};

pub const MAX_AGENT_PROFILES: usize = 50;
pub const MAX_AGENT_PROFILE_NAME_CHARS: usize = 80;
pub const MAX_AGENT_PROFILE_NOTES_CHARS: usize = 2000;
pub const MAX_AGENT_PROFILE_ID_BYTES: usize = 64;

const STORE_FILE: &str = "agent-profiles.json";
const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentProfilesFile {
    version: u32,
    #[serde(default)]
    profiles: Vec<AgentProfile>,
}

struct AgentProfilesStoreInner {
    path: PathBuf,
    state: Mutex<AgentProfilesFile>,
}

#[derive(Clone)]
pub struct AgentProfilesStore {
    inner: Arc<AgentProfilesStoreInner>,
}

impl AgentProfilesStore {
    pub fn open(profile_store_root: &Path) -> Result<Self, EngineError> {
        std::fs::create_dir_all(profile_store_root)?;
        let path = profile_store_root.join(STORE_FILE);
        let state = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<AgentProfilesFile>(&bytes) {
                Ok(file) if file.version == STORE_VERSION => file,
                Ok(file) => {
                    tracing::warn!(
                        path = %path.display(),
                        version = file.version,
                        "unsupported agent profiles store version; starting empty"
                    );
                    empty_file()
                }
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "invalid agent profiles store; starting empty"
                    );
                    empty_file()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => empty_file(),
            Err(err) => return Err(err.into()),
        };
        Ok(Self {
            inner: Arc::new(AgentProfilesStoreInner {
                path,
                state: Mutex::new(state),
            }),
        })
    }

    pub fn snapshot(&self) -> AgentProfilesSnapshot {
        AgentProfilesSnapshot {
            profiles: lock(&self.inner.state).profiles.clone(),
        }
    }

    pub fn profile(&self, id: &str) -> Option<AgentProfile> {
        lock(&self.inner.state)
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
    }

    pub fn upsert(
        &self,
        profile_id: Option<&str>,
        draft: AgentProfileDraft,
    ) -> Result<AgentProfilesSnapshot, EngineError> {
        let draft = normalize_draft(draft)?;
        self.mutate(|profiles| {
            let index = match profile_id {
                Some(id) => profiles
                    .iter()
                    .position(|profile| profile.id == id)
                    .ok_or_else(|| EngineError::Other("Agent profile not found".into()))?,
                None => {
                    if profiles.len() >= MAX_AGENT_PROFILES {
                        return Err(EngineError::Other(format!(
                            "At most {MAX_AGENT_PROFILES} agent profiles are allowed"
                        )));
                    }
                    profiles.len()
                }
            };
            let id = profile_id.map(str::to_string).unwrap_or_else(|| {
                unique_slug_id(&draft.name, "profile", MAX_AGENT_PROFILE_ID_BYTES, |id| {
                    profiles.iter().any(|profile| profile.id == id)
                })
            });
            let profile = AgentProfile {
                id,
                name: draft.name,
                icon: draft.icon,
                color: draft.color,
                harness: draft.harness,
                model: draft.model,
                reasoning: draft.reasoning,
                permission_mode: draft.permission_mode,
                model_options: draft.model_options,
                notes: draft.notes,
            };
            if index == profiles.len() {
                profiles.push(profile);
            } else {
                profiles[index] = profile;
            }
            Ok(())
        })
    }

    pub fn delete(&self, profile_id: &str) -> Result<AgentProfilesSnapshot, EngineError> {
        self.mutate(|profiles| {
            let before = profiles.len();
            profiles.retain(|profile| profile.id != profile_id);
            if profiles.len() == before {
                return Err(EngineError::Other("Agent profile not found".into()));
            }
            Ok(())
        })
    }

    pub fn reorder(&self, profile_ids: &[String]) -> Result<AgentProfilesSnapshot, EngineError> {
        self.mutate(|profiles| {
            let mut reordered = Vec::with_capacity(profiles.len());
            for id in profile_ids {
                let index = profiles
                    .iter()
                    .position(|profile| &profile.id == id)
                    .ok_or_else(|| {
                        EngineError::Other(
                            "Reorder must list every agent profile exactly once".into(),
                        )
                    })?;
                reordered.push(profiles.remove(index));
            }
            if !profiles.is_empty() {
                return Err(EngineError::Other(
                    "Reorder must list every agent profile exactly once".into(),
                ));
            }
            *profiles = reordered;
            Ok(())
        })
    }

    fn mutate(
        &self,
        change: impl FnOnce(&mut Vec<AgentProfile>) -> Result<(), EngineError>,
    ) -> Result<AgentProfilesSnapshot, EngineError> {
        let mut state = lock(&self.inner.state);
        let mut next = state.clone();
        change(&mut next.profiles)?;
        persist_json(&self.inner.path, &next)?;
        *state = next;
        Ok(AgentProfilesSnapshot {
            profiles: state.profiles.clone(),
        })
    }
}

fn empty_file() -> AgentProfilesFile {
    AgentProfilesFile {
        version: STORE_VERSION,
        profiles: Vec::new(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn normalize_draft(draft: AgentProfileDraft) -> Result<AgentProfileDraft, EngineError> {
    let name = draft.name.trim().to_string();
    if name.is_empty() {
        return Err(EngineError::Other("Profile name is required".into()));
    }
    if name.chars().count() > MAX_AGENT_PROFILE_NAME_CHARS {
        return Err(EngineError::Other(format!(
            "Profile name must not exceed {MAX_AGENT_PROFILE_NAME_CHARS} characters"
        )));
    }
    let notes = draft.notes.trim().to_string();
    if notes.chars().count() > MAX_AGENT_PROFILE_NOTES_CHARS {
        return Err(EngineError::Other(format!(
            "Profile notes must not exceed {MAX_AGENT_PROFILE_NOTES_CHARS} characters"
        )));
    }
    let model = draft
        .model
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty());
    Ok(AgentProfileDraft {
        name,
        notes,
        model,
        ..draft
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::{HarnessId, PermissionMode, ReasoningLevel};

    fn draft(name: &str) -> AgentProfileDraft {
        AgentProfileDraft {
            name: name.into(),
            icon: Default::default(),
            color: Default::default(),
            harness: HarnessId::ClaudeCode,
            model: Some(" opus ".into()),
            reasoning: Some(ReasoningLevel::High),
            permission_mode: Some(PermissionMode::Ask),
            model_options: Default::default(),
            notes: " Review a diff. ".into(),
        }
    }

    fn ids(snapshot: &AgentProfilesSnapshot) -> Vec<&str> {
        snapshot.profiles.iter().map(|p| p.id.as_str()).collect()
    }

    #[test]
    fn profiles_round_trip_with_slug_ids_and_trimmed_fields() {
        let temp = tempfile::tempdir().unwrap();
        let store = AgentProfilesStore::open(temp.path()).unwrap();
        store.upsert(None, draft("UI work")).unwrap();
        let snapshot = store.upsert(None, draft("UI work")).unwrap();
        assert_eq!(ids(&snapshot), ["ui-work", "ui-work-2"]);
        assert_eq!(snapshot.profiles[0].model.as_deref(), Some("opus"));
        assert_eq!(snapshot.profiles[0].notes, "Review a diff.");

        let reopened = AgentProfilesStore::open(temp.path()).unwrap();
        assert_eq!(reopened.snapshot(), snapshot);
        assert_eq!(reopened.profile("ui-work-2").unwrap().name, "UI work");
    }

    #[test]
    fn update_keeps_id_and_position_and_delete_removes() {
        let temp = tempfile::tempdir().unwrap();
        let store = AgentProfilesStore::open(temp.path()).unwrap();
        store.upsert(None, draft("Reviewer")).unwrap();
        store.upsert(None, draft("Quick")).unwrap();
        let mut renamed = draft("Strict reviewer");
        renamed.model = Some("  ".into());
        let snapshot = store.upsert(Some("reviewer"), renamed).unwrap();
        assert_eq!(ids(&snapshot), ["reviewer", "quick"]);
        assert_eq!(snapshot.profiles[0].name, "Strict reviewer");
        assert_eq!(snapshot.profiles[0].model, None);

        assert!(store.upsert(Some("missing"), draft("x")).is_err());
        assert_eq!(ids(&store.delete("reviewer").unwrap()), ["quick"]);
        assert!(store.delete("reviewer").is_err());
    }

    #[test]
    fn reorder_requires_a_permutation() {
        let temp = tempfile::tempdir().unwrap();
        let store = AgentProfilesStore::open(temp.path()).unwrap();
        for name in ["a", "b", "c"] {
            store.upsert(None, draft(name)).unwrap();
        }
        let order = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        assert_eq!(
            ids(&store.reorder(&order(&["c", "a", "b"])).unwrap()),
            ["c", "a", "b"]
        );
        assert!(store.reorder(&order(&["c", "a"])).is_err());
        assert!(store.reorder(&order(&["c", "a", "a"])).is_err());
        assert_eq!(ids(&store.snapshot()), ["c", "a", "b"]);
    }

    #[test]
    fn invalid_drafts_and_corrupt_files_are_handled() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(STORE_FILE), b"not json").unwrap();
        let store = AgentProfilesStore::open(temp.path()).unwrap();
        assert!(store.snapshot().profiles.is_empty());
        assert!(store.upsert(None, draft("   ")).is_err());
        let mut long = draft("x");
        long.notes = "n".repeat(MAX_AGENT_PROFILE_NOTES_CHARS + 1);
        assert!(store.upsert(None, long).is_err());
        assert_eq!(ids(&store.upsert(None, draft("ok")).unwrap()), ["ok"]);
    }
}
