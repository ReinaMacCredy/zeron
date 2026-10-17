use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use zeron_harness::{Harness, LaunchOverrides};
use zeron_proto::{
    HarnessId, HomeDir, InstanceConfig, ProviderInstance, ProviderInstancesSnapshot,
    validate_instance_id,
};

use crate::EngineError;
use crate::project_actions::persist_json;
use crate::registry::HarnessRegistry;

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

    pub(crate) fn resolve(
        &self,
        registry: &HarnessRegistry,
        driver: HarnessId,
        id: &str,
    ) -> Result<Arc<dyn Harness>, EngineError> {
        let instance = self.instance(id).ok_or_else(|| {
            EngineError::Other(format!(
                "Provider instance '{id}' is not set up on this device"
            ))
        })?;
        if instance.driver != driver {
            return Err(EngineError::Other(format!(
                "Provider instance '{id}' runs on {}, not {}",
                instance.driver.display_name(),
                driver.display_name()
            )));
        }
        let data_dir = self.inner.path.parent().unwrap_or(Path::new("."));
        registry.resolve_instance(&instance, data_dir)
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

pub(crate) fn launch_overrides(
    instance: &ProviderInstance,
    data_dir: &Path,
) -> Result<LaunchOverrides, EngineError> {
    let InstanceConfig::Native {
        executable,
        home_dir,
        launch_args,
        env,
    } = &instance.config
    else {
        return Err(EngineError::Other(format!(
            "Provider instance '{}' is an ACP agent, which this engine cannot run yet",
            instance.id
        )));
    };
    Ok(LaunchOverrides {
        executable: executable
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(expand)
            .transpose()?,
        home: home_dir
            .as_ref()
            .map(|home| provider_home(&instance.id, home, data_dir))
            .transpose()?,
        args: launch_args.clone(),
        env: env.clone(),
    })
}

pub(crate) fn provider_home(
    id: &str,
    home: &HomeDir,
    data_dir: &Path,
) -> Result<PathBuf, EngineError> {
    match home {
        HomeDir::Custom { path } if path.trim().is_empty() => Err(EngineError::Other(format!(
            "Provider instance '{id}' has an empty home directory"
        ))),
        HomeDir::Custom { path } => expand(path),
        HomeDir::Managed => {
            let dir = data_dir.join("provider-homes").join(id);
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            builder.create(&dir)?;
            Ok(dir)
        }
    }
}

fn expand(path: &str) -> Result<PathBuf, EngineError> {
    crate::repos::expand_home(path.trim())
        .map(PathBuf::from)
        .map_err(|error| EngineError::Other(error.into()))
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
    fn each_instance_resolves_to_its_own_harness_until_it_changes() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderInstancesStore::open(temp.path()).unwrap();
        let registry = HarnessRegistry::new();
        for id in ["codex_work", "codex_home"] {
            store.upsert(instance(id, HarnessId::Codex)).unwrap();
        }
        let resolve = |id: &str| store.resolve(&registry, HarnessId::Codex, id).unwrap();
        let work = resolve("codex_work");
        assert!(Arc::ptr_eq(&work, &resolve("codex_work")));
        assert!(!Arc::ptr_eq(&work, &resolve("codex_home")));

        let home = temp.path().join("provider-homes").join("codex_work");
        assert!(home.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&home).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        assert_eq!(
            launch_overrides(&store.instance("codex_work").unwrap(), temp.path())
                .unwrap()
                .home,
            Some(home)
        );

        let mut edited = instance("codex_work", HarnessId::Codex);
        if let InstanceConfig::Native { launch_args, .. } = &mut edited.config {
            launch_args.push("--profile=work".into());
        }
        store.upsert(edited).unwrap();
        let rebuilt = resolve("codex_work");
        assert!(!Arc::ptr_eq(&work, &rebuilt));
        assert!(Arc::ptr_eq(&rebuilt, &resolve("codex_work")));

        let unknown = store
            .resolve(&registry, HarnessId::Codex, "codex_gone")
            .err()
            .unwrap();
        assert!(unknown.to_string().contains("'codex_gone'"), "{unknown}");
        let wrong_driver = store
            .resolve(&registry, HarnessId::ClaudeCode, "codex_work")
            .err()
            .unwrap();
        assert!(wrong_driver.to_string().contains("Codex"), "{wrong_driver}");
    }

    #[test]
    fn drivers_without_native_instances_and_cursor_binaries_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderInstancesStore::open(temp.path()).unwrap();
        let registry = HarnessRegistry::new();
        store
            .upsert(instance("grok_work", HarnessId::Grok))
            .unwrap();
        let mut cursor = instance("cursor_work", HarnessId::Cursor);
        cursor.config = InstanceConfig::Native {
            executable: Some("/usr/local/bin/cursor-agent".into()),
            home_dir: None,
            launch_args: Vec::new(),
            env: Default::default(),
        };
        store.upsert(cursor).unwrap();
        let mut cursor_env = instance("cursor_env", HarnessId::Cursor);
        cursor_env.config = InstanceConfig::Native {
            executable: None,
            home_dir: None,
            launch_args: Vec::new(),
            env: [("CURSOR_API_KEY".into(), "key".into())].into(),
        };
        store.upsert(cursor_env).unwrap();
        for (driver, id) in [
            (HarnessId::Grok, "grok_work"),
            (HarnessId::Cursor, "cursor_work"),
        ] {
            assert!(store.resolve(&registry, driver, id).is_err(), "{id}");
        }
        store
            .resolve(&registry, HarnessId::Cursor, "cursor_env")
            .unwrap();
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
