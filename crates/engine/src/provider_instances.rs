use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use zeron_proto::{HarnessId, ProviderInstance, ProviderInstancesSnapshot, validate_instance_id};

use crate::EngineError;
use crate::project_actions::persist_json;

const STORE_FILE: &str = "provider-instances.json";
const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderInstancesFile {
    version: u32,
    #[serde(default)]
    instances: Vec<ProviderInstance>,
}

struct ProviderInstancesStoreInner {
    path: PathBuf,
    state: Mutex<ProviderInstancesFile>,
}

#[derive(Clone)]
pub struct ProviderInstancesStore {
    inner: Arc<ProviderInstancesStoreInner>,
}

impl ProviderInstancesStore {
    pub fn open(data_dir: &Path) -> Result<Self, EngineError> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join(STORE_FILE);
        let state = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<ProviderInstancesFile>(&bytes) {
                Ok(file) if file.version == STORE_VERSION => file,
                Ok(file) => set_aside(&path, &format!("unsupported version {}", file.version))?,
                Err(err) => set_aside(&path, &err.to_string())?,
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => empty_file(),
            Err(err) => return Err(err.into()),
        };
        Ok(Self {
            inner: Arc::new(ProviderInstancesStoreInner {
                path,
                state: Mutex::new(state),
            }),
        })
    }

    pub fn snapshot(&self) -> ProviderInstancesSnapshot {
        ProviderInstancesSnapshot {
            instances: lock(&self.inner.state).instances.clone(),
        }
    }

    pub fn instance(&self, id: &str) -> Option<ProviderInstance> {
        lock(&self.inner.state)
            .instances
            .iter()
            .find(|instance| instance.id == id)
            .cloned()
    }

    pub fn upsert(
        &self,
        instance: ProviderInstance,
    ) -> Result<ProviderInstancesSnapshot, EngineError> {
        let instance = normalize(instance)?;
        self.mutate(|instances| {
            match instances
                .iter_mut()
                .find(|existing| existing.id == instance.id)
            {
                Some(existing) if existing.driver != instance.driver => {
                    return Err(EngineError::Other(format!(
                        "Provider instance '{}' runs on {}, not {}",
                        instance.id,
                        existing.driver.display_name(),
                        instance.driver.display_name()
                    )));
                }
                Some(existing) => *existing = instance,
                None => instances.push(instance),
            }
            Ok(())
        })
    }

    pub fn delete(&self, instance_id: &str) -> Result<ProviderInstancesSnapshot, EngineError> {
        self.mutate(|instances| {
            let before = instances.len();
            instances.retain(|instance| instance.id != instance_id);
            if instances.len() == before {
                return Err(EngineError::Other("Provider instance not found".into()));
            }
            Ok(())
        })
    }

    fn mutate(
        &self,
        change: impl FnOnce(&mut Vec<ProviderInstance>) -> Result<(), EngineError>,
    ) -> Result<ProviderInstancesSnapshot, EngineError> {
        let mut state = lock(&self.inner.state);
        let mut next = state.clone();
        change(&mut next.instances)?;
        persist_json(&self.inner.path, &next)?;
        *state = next;
        Ok(ProviderInstancesSnapshot {
            instances: state.instances.clone(),
        })
    }
}

// Instances name paths, env and managed homes; never overwrite a file this build cannot read.
fn set_aside(path: &Path, reason: &str) -> Result<ProviderInstancesFile, EngineError> {
    let kept = path.with_extension(format!("unreadable-{}.json", uuid::Uuid::new_v4()));
    tracing::warn!(
        path = %path.display(),
        kept = %kept.display(),
        reason,
        "provider instances store unreadable; kept aside, starting empty"
    );
    std::fs::rename(path, &kept)?;
    Ok(empty_file())
}

fn empty_file() -> ProviderInstancesFile {
    ProviderInstancesFile {
        version: STORE_VERSION,
        instances: Vec::new(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn normalize(instance: ProviderInstance) -> Result<ProviderInstance, EngineError> {
    let id = instance.id.trim().to_string();
    validate_instance_id(&id).map_err(EngineError::Other)?;
    if instance.driver == HarnessId::Mock {
        return Err(EngineError::Other(
            "The mock harness has no provider instances".into(),
        ));
    }
    let label = instance.label.trim().to_string();
    if label.is_empty() {
        return Err(EngineError::Other(
            "Provider instance label is required".into(),
        ));
    }
    let accent = instance
        .accent
        .map(|accent| accent.trim().to_string())
        .filter(|accent| !accent.is_empty());
    Ok(ProviderInstance {
        id,
        label,
        accent,
        ..instance
    })
}

#[cfg(test)]
mod tests {
    use zeron_proto::{AcpDistribution, AcpSource, HomeDir, InstanceConfig};

    use super::*;

    fn instance(id: &str, driver: HarnessId) -> ProviderInstance {
        ProviderInstance {
            id: id.into(),
            driver,
            label: " Work ".into(),
            accent: Some("  ".into()),
            config: InstanceConfig::Native {
                executable: None,
                home_dir: Some(HomeDir::Managed),
                launch_args: Vec::new(),
                env: Default::default(),
            },
        }
    }

    fn ids(snapshot: &ProviderInstancesSnapshot) -> Vec<&str> {
        snapshot.instances.iter().map(|i| i.id.as_str()).collect()
    }

    #[test]
    fn instances_upsert_by_id_and_survive_a_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderInstancesStore::open(temp.path()).unwrap();
        store
            .upsert(instance("codex_work", HarnessId::Codex))
            .unwrap();
        let mut acp = instance("acp_gemini", HarnessId::Acp);
        acp.config = InstanceConfig::Acp {
            source: AcpSource::Registry {
                agent_id: "gemini".into(),
                version: "0.1.0".into(),
                distribution: AcpDistribution::Npx,
            },
            env: Default::default(),
            icon_path: None,
        };
        store.upsert(acp).unwrap();
        let mut renamed = instance("codex_work", HarnessId::Codex);
        renamed.label = "Personal".into();
        let snapshot = store.upsert(renamed).unwrap();
        assert_eq!(ids(&snapshot), ["codex_work", "acp_gemini"]);
        assert_eq!(snapshot.instances[0].label, "Personal");
        assert_eq!(snapshot.instances[1].label, "Work");
        assert_eq!(snapshot.instances[1].accent, None);

        let reopened = ProviderInstancesStore::open(temp.path()).unwrap();
        assert_eq!(reopened.snapshot(), snapshot);
        assert_eq!(
            reopened.instance("acp_gemini").unwrap().driver,
            HarnessId::Acp
        );
        assert_eq!(ids(&reopened.delete("codex_work").unwrap()), ["acp_gemini"]);
        assert!(reopened.delete("codex_work").is_err());
    }

    #[test]
    fn invalid_instances_and_corrupt_files_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(STORE_FILE), b"not json").unwrap();
        let store = ProviderInstancesStore::open(temp.path()).unwrap();
        assert!(store.snapshot().instances.is_empty());
        let kept: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| std::fs::read(entry.unwrap().path()).unwrap())
            .collect();
        assert_eq!(kept, [b"not json".to_vec()]);
        for bad in ["codex", "2work", "work space", ""] {
            assert!(
                store.upsert(instance(bad, HarnessId::Codex)).is_err(),
                "{bad}"
            );
        }
        assert!(store.upsert(instance("mock_x", HarnessId::Mock)).is_err());
        let mut unlabeled = instance("codex_x", HarnessId::Codex);
        unlabeled.label = "  ".into();
        assert!(store.upsert(unlabeled).is_err());
        store.upsert(instance("work", HarnessId::Codex)).unwrap();
        assert!(
            store
                .upsert(instance("work", HarnessId::ClaudeCode))
                .is_err()
        );
        assert_eq!(ids(&store.snapshot()), ["work"]);
    }
}
