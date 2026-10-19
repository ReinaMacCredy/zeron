use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use zeron_harness::{AcpHarness, AcpLaunch, Harness, LaunchOverrides};
use zeron_proto::{
    AcpSource, HarnessId, HomeDir, InstanceConfig, ProviderInstance, ProviderInstancesSnapshot,
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
    changes: tokio::sync::watch::Sender<ProviderInstancesSnapshot>,
}

#[derive(Clone)]
pub struct ProviderInstancesStore {
    inner: Arc<ProviderInstancesStoreInner>,
}

impl ProviderInstancesStore {
    pub fn open(data_dir: &Path) -> Result<Self, EngineError> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join(STORE_FILE);
        #[cfg(unix)]
        if path.is_file() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        let state = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<ProviderInstancesFile>(&bytes) {
                Ok(file) if file.version == STORE_VERSION => file,
                Ok(file) => set_aside(&path, &format!("unsupported version {}", file.version))?,
                Err(err) => set_aside(&path, &err.to_string())?,
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => empty_file(),
            Err(err) => return Err(err.into()),
        };
        let changes = tokio::sync::watch::Sender::new(ProviderInstancesSnapshot {
            instances: state.instances.clone(),
        });
        Ok(Self {
            inner: Arc::new(ProviderInstancesStoreInner {
                path,
                state: Mutex::new(state),
                changes,
            }),
        })
    }

    pub fn watch(&self) -> tokio::sync::watch::Receiver<ProviderInstancesSnapshot> {
        self.inner.changes.subscribe()
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

    pub(crate) fn require(&self, id: &str) -> Result<ProviderInstance, EngineError> {
        self.instance(id).ok_or_else(|| {
            EngineError::Other(format!(
                "Provider instance '{id}' is not set up on this device"
            ))
        })
    }

    pub(crate) fn resolve(
        &self,
        registry: &HarnessRegistry,
        driver: HarnessId,
        id: &str,
    ) -> Result<Arc<dyn Harness>, EngineError> {
        let instance = self.require(id)?;
        if instance.driver != driver {
            return Err(EngineError::Other(format!(
                "Provider instance '{id}' runs on {}, not {}",
                instance.driver.display_name(),
                driver.display_name()
            )));
        }
        registry.resolve_instance(&instance, self.data_dir())
    }

    pub(crate) fn launch_overrides(&self, id: &str) -> Result<LaunchOverrides, EngineError> {
        launch_overrides(&self.require(id)?, self.data_dir())
    }

    pub(crate) fn acp_harness(&self, id: &str) -> Result<AcpHarness, EngineError> {
        acp_harness(&self.require(id)?, self.data_dir())
    }

    pub(crate) fn data_dir(&self) -> &Path {
        self.inner.path.parent().unwrap_or(Path::new("."))
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
                // The id names directories, and a case-insensitive filesystem would share them.
                None => match instances
                    .iter()
                    .find(|existing| existing.id.eq_ignore_ascii_case(&instance.id))
                {
                    Some(existing) => {
                        return Err(EngineError::Other(format!(
                            "Provider instance ID '{}' is taken by '{}'",
                            instance.id, existing.id
                        )));
                    }
                    None => instances.push(instance),
                },
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
        let snapshot = ProviderInstancesSnapshot {
            instances: state.instances.clone(),
        };
        self.inner.changes.send_replace(snapshot.clone());
        Ok(snapshot)
    }

    pub(crate) fn launch_config(&self, id: &str) -> Option<InstanceConfig> {
        let mut config = self.instance(id)?.config;
        match &mut config {
            InstanceConfig::Acp { icon_path, .. } => *icon_path = None,
            InstanceConfig::Native { executable, .. } => {
                *executable = executable.take().filter(|path| !path.trim().is_empty());
            }
        }
        Some(config)
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

fn check(instance: &ProviderInstance) -> Result<(), EngineError> {
    let name = instance.driver.display_name();
    let reason = match (&instance.config, instance.driver) {
        (InstanceConfig::Acp { source, .. }, HarnessId::Acp) => match source {
            AcpSource::Local { command, .. } if command.trim().is_empty() => {
                Some("has no command to run".to_string())
            }
            _ => None,
        },
        (InstanceConfig::Acp { .. }, _) => {
            Some(format!("holds an ACP agent's settings, not {name}'s"))
        }
        (InstanceConfig::Native { .. }, HarnessId::Acp) => Some("has no ACP agent settings".into()),
        (
            InstanceConfig::Native {
                executable,
                home_dir,
                launch_args,
                ..
            },
            driver,
        ) => match driver {
            _ if matches!(home_dir, Some(HomeDir::Custom { path }) if path.trim().is_empty()) => {
                Some("has an empty home directory".into())
            }
            HarnessId::ClaudeCode | HarnessId::Codex | HarnessId::Pi | HarnessId::Antigravity => {
                None
            }
            HarnessId::Devin | HarnessId::Grok | HarnessId::Hermes | HarnessId::Opencode
                if home_dir.is_some() =>
            {
                Some(format!("{name} instances take no home directory"))
            }
            HarnessId::Devin | HarnessId::Grok | HarnessId::Hermes | HarnessId::Opencode => None,
            HarnessId::Cursor
                if home_dir.is_some()
                    || executable
                        .as_deref()
                        .is_some_and(|path| !path.trim().is_empty())
                    || !launch_args.is_empty() =>
            {
                Some(format!(
                    "{name} instances run on Zeron's managed Cursor SDK and take only environment variables"
                ))
            }
            HarnessId::Cursor => None,
            _ => Some(format!("{name} does not run as a provider instance yet")),
        },
    };
    match reason {
        Some(reason) => Err(EngineError::Other(format!(
            "Provider instance '{}': {reason}",
            instance.id
        ))),
        None => Ok(()),
    }
}

pub(crate) fn launch_overrides(
    instance: &ProviderInstance,
    data_dir: &Path,
) -> Result<LaunchOverrides, EngineError> {
    check(instance)?;
    let InstanceConfig::Native {
        executable,
        home_dir,
        launch_args,
        env,
    } = &instance.config
    else {
        return Err(EngineError::Other(format!(
            "Provider instance '{}' holds an ACP agent's settings, not {}'s",
            instance.id,
            instance.driver.display_name()
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

pub(crate) fn acp_launch(
    instance: &ProviderInstance,
    data_dir: &Path,
) -> Result<AcpLaunch, EngineError> {
    check(instance)?;
    let id = &instance.id;
    let InstanceConfig::Acp { source, env, .. } = &instance.config else {
        return Err(EngineError::Other(format!(
            "Provider instance '{id}' has no ACP agent settings"
        )));
    };
    let (program, args, env) = match source {
        AcpSource::Local { command, .. } if command.trim().is_empty() => {
            return Err(EngineError::Other(format!(
                "Provider instance '{id}' has no command to run"
            )));
        }
        AcpSource::Local { command, args } => (expand(command)?, args.clone(), env.clone()),
        AcpSource::Registry {
            agent_id,
            version,
            distribution,
        } => {
            let prepared =
                crate::acp_registry::prepared_launch(data_dir, agent_id, version, *distribution)?
                    .ok_or_else(|| {
                        EngineError::Other(format!(
                            "Provider instance '{id}': the ACP Registry agent '{agent_id}' is not prepared on this device yet"
                        ))
                    })?;
            let mut merged = prepared.env;
            merged.extend(env.clone());
            (prepared.program, prepared.args, merged)
        }
    };
    Ok(AcpLaunch {
        display_name: instance.label.clone(),
        program,
        args,
        env,
    })
}

pub(crate) fn runs_over_acp(driver: HarnessId) -> bool {
    matches!(
        driver,
        HarnessId::Acp
            | HarnessId::Devin
            | HarnessId::Grok
            | HarnessId::Hermes
            | HarnessId::Antigravity
    )
}

pub(crate) fn acp_harness(
    instance: &ProviderInstance,
    data_dir: &Path,
) -> Result<AcpHarness, EngineError> {
    let refuse = |reason: String| {
        EngineError::Other(format!("Provider instance '{}': {reason}", instance.id))
    };
    let name = instance.driver.display_name();
    let builtin = match instance.driver {
        HarnessId::Acp => {
            return AcpHarness::generic(acp_launch(instance, data_dir)?)
                .map_err(|error| refuse(error.to_string()));
        }
        HarnessId::Devin => AcpHarness::devin(),
        HarnessId::Grok => AcpHarness::grok(),
        HarnessId::Hermes => AcpHarness::hermes(),
        HarnessId::Antigravity => AcpHarness::antigravity(),
        _ => return Err(refuse(format!("{name} is not an ACP agent"))),
    };
    Ok(builtin.with_launch(launch_overrides(instance, data_dir)?))
}

pub(crate) fn provider_home(
    id: &str,
    home: &HomeDir,
    data_dir: &Path,
) -> Result<PathBuf, EngineError> {
    let dir = home_path(id, home, data_dir)?;
    if *home == HomeDir::Managed {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&dir)?;
    }
    Ok(dir)
}

pub(crate) fn home_path(id: &str, home: &HomeDir, data_dir: &Path) -> Result<PathBuf, EngineError> {
    match home {
        HomeDir::Custom { path } if path.trim().is_empty() => Err(EngineError::Other(format!(
            "Provider instance '{id}' has an empty home directory"
        ))),
        HomeDir::Custom { path } => expand(path),
        HomeDir::Managed => Ok(managed_home(id, data_dir)),
    }
}

pub(crate) fn keeps_sessions(
    before: &ProviderInstance,
    after: &ProviderInstance,
    data_dir: &Path,
) -> bool {
    let home = |instance: &ProviderInstance, home: &Option<HomeDir>| {
        home.as_ref()
            .map(|home| home_path(&instance.id, home, data_dir).ok())
    };
    match (&before.config, &after.config) {
        (
            InstanceConfig::Native { home_dir: was, .. },
            InstanceConfig::Native { home_dir: now, .. },
        ) => home(before, was) == home(after, now),
        (InstanceConfig::Acp { source: was, .. }, InstanceConfig::Acp { source: now, .. }) => {
            match (was, now) {
                (
                    AcpSource::Registry { agent_id: was, .. },
                    AcpSource::Registry { agent_id: now, .. },
                ) => was == now,
                (AcpSource::Local { command: was, .. }, AcpSource::Local { command: now, .. }) => {
                    was.trim() == now.trim()
                }
                _ => false,
            }
        }
        _ => false,
    }
}

pub(crate) fn managed_home(id: &str, data_dir: &Path) -> PathBuf {
    data_dir.join("provider-homes").join(id)
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
    let instance = ProviderInstance {
        id,
        label,
        accent,
        ..instance
    };
    check(&instance)?;
    Ok(instance)
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

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file = temp.path().join(STORE_FILE);
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "instance env holds API keys");
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let reopened = ProviderInstancesStore::open(temp.path()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file = temp.path().join(STORE_FILE);
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o777,
                0o600,
                "a file from an older build is tightened"
            );
        }
        assert_eq!(reopened.snapshot(), snapshot);
        let mut watcher = reopened.watch();
        assert_eq!(*watcher.borrow_and_update(), snapshot);
        assert_eq!(
            reopened.instance("acp_gemini").unwrap().driver,
            HarnessId::Acp
        );
        assert_eq!(ids(&reopened.delete("codex_work").unwrap()), ["acp_gemini"]);
        assert!(watcher.has_changed().unwrap());
        assert_eq!(ids(&watcher.borrow_and_update()), ["acp_gemini"]);
        assert!(reopened.delete("codex_work").is_err());
        assert!(!watcher.has_changed().unwrap());
        reopened
            .upsert(instance("codex_work", HarnessId::Codex))
            .unwrap();
        assert_eq!(
            ids(&watcher.borrow_and_update()),
            ["acp_gemini", "codex_work"]
        );
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
    fn acp_drivers_refuse_a_home_except_antigravity_and_cursor_refuses_a_binary() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderInstancesStore::open(temp.path()).unwrap();
        let registry = HarnessRegistry::new();
        for (driver, id) in [
            (HarnessId::Grok, "grok_work"),
            (HarnessId::Devin, "devin_work"),
        ] {
            let error = store.upsert(instance(id, driver)).err().unwrap();
            assert!(
                error.to_string().contains("take no home directory"),
                "{id}: {error}"
            );
        }
        store
            .upsert(instance("antigravity_work", HarnessId::Antigravity))
            .unwrap();
        let mut grok_env = instance("grok_env", HarnessId::Grok);
        grok_env.config = InstanceConfig::Native {
            executable: None,
            home_dir: None,
            launch_args: vec!["--verbose".into()],
            env: [("GROK_API_KEY".into(), "key".into())].into(),
        };
        store.upsert(grok_env).unwrap();
        let mut cursor = instance("cursor_work", HarnessId::Cursor);
        cursor.config = InstanceConfig::Native {
            executable: Some("/usr/local/bin/cursor-agent".into()),
            home_dir: None,
            launch_args: Vec::new(),
            env: Default::default(),
        };
        assert!(store.upsert(cursor).is_err());
        let mut cursor_env = instance("cursor_env", HarnessId::Cursor);
        cursor_env.config = InstanceConfig::Native {
            executable: None,
            home_dir: None,
            launch_args: Vec::new(),
            env: [("CURSOR_API_KEY".into(), "key".into())].into(),
        };
        store.upsert(cursor_env).unwrap();
        store
            .resolve(&registry, HarnessId::Cursor, "cursor_env")
            .unwrap();
        let grok = store
            .resolve(&registry, HarnessId::Grok, "grok_env")
            .unwrap();
        assert_eq!(grok.id(), HarnessId::Grok);
        store
            .resolve(&registry, HarnessId::Antigravity, "antigravity_work")
            .unwrap();
    }

    #[cfg(unix)]
    fn fake_acp_agent(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("fake-agent");
        std::fs::write(
            &script,
            r#"#!/bin/sh
printf 'args=%s count=%s\n' "$*" "$#" >> "$FAKE_AGENT_RECORD"
emit() { printf '%s\n' "$1"; }
rid() { printf '%s' "$1" | sed 's/.*"id":\([0-9]*\).*/\1/'; }
has() { case "$1" in *"$2"*) return 0 ;; *) return 1 ;; esac; }
while read -r line; do
  has "$line" '"id":' || continue
  id=$(rid "$line")
  if has "$line" '"method":"initialize"'; then
    emit "{\"id\":$id,\"result\":{\"protocolVersion\":1,\"authMethods\":[{\"id\":\"browser\",\"name\":\"Log in\",\"description\":\"Opens a browser\"},{\"id\":\"setup\",\"name\":\"Run setup\",\"type\":\"terminal\",\"args\":[\"--setup\"]}]}}"
  elif has "$line" '"method":"session/new"'; then
    emit "{\"id\":$id,\"result\":{\"sessionId\":\"s\",\"configOptions\":[{\"id\":\"model\",\"name\":\"Model\",\"category\":\"model\",\"type\":\"select\",\"currentValue\":\"m1\",\"options\":[{\"value\":\"m1\",\"name\":\"Model One\"}]}]}}"
  elif has "$line" '"method":"authenticate"'; then
    printf 'authenticate=%s\n' "$line" >> "$FAKE_AGENT_RECORD"
    printf 'Visit https://agent.example/login?code=1\n' >&2
    emit "{\"id\":$id,\"result\":{}}"
  else
    emit "{\"id\":$id,\"result\":{}}"
  fi
done
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[cfg(unix)]
    fn local_acp_instance(id: &str, command: &Path, record: &Path) -> ProviderInstance {
        ProviderInstance {
            id: id.into(),
            driver: HarnessId::Acp,
            label: "My Agent".into(),
            accent: None,
            config: InstanceConfig::Acp {
                source: AcpSource::Local {
                    command: command.display().to_string(),
                    args: vec!["--acp".into(), "two words".into()],
                },
                env: [("FAKE_AGENT_RECORD".into(), record.display().to_string())].into(),
                icon_path: None,
            },
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_local_acp_instance_runs_its_command_with_its_args_and_env() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderInstancesStore::open(temp.path()).unwrap();
        let registry = HarnessRegistry::new();
        let record = temp.path().join("record");
        let agent = fake_acp_agent(temp.path());
        store
            .upsert(local_acp_instance("acp_local", &agent, &record))
            .unwrap();
        let harness = store
            .resolve(&registry, HarnessId::Acp, "acp_local")
            .unwrap();
        assert_eq!(harness.id(), HarnessId::Acp);
        assert_eq!(harness.display_name(), "My Agent");
        assert!(harness.installed());
        let models = harness.models().await.unwrap();
        assert_eq!(
            models.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(),
            ["Model One"]
        );
        assert_eq!(
            std::fs::read_to_string(&record).unwrap(),
            "args=--acp two words count=2\n"
        );

        let mut registry_agent = local_acp_instance("acp_registry", &agent, &record);
        registry_agent.config = InstanceConfig::Acp {
            source: AcpSource::Registry {
                agent_id: "gemini".into(),
                version: "0.1.0".into(),
                distribution: AcpDistribution::Npx,
            },
            env: Default::default(),
            icon_path: None,
        };
        store.upsert(registry_agent).unwrap();
        let error = store
            .resolve(&registry, HarnessId::Acp, "acp_registry")
            .err()
            .unwrap();
        assert!(error.to_string().contains("not prepared"), "{error}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_local_acp_instance_lists_its_auth_methods_and_signs_in_only_in_app() {
        use crate::agent_accounts::{AgentAccounts, AgentAccountsConfig};
        use zeron_proto::AgentLoginStatus;
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderInstancesStore::open(temp.path()).unwrap();
        let record = temp.path().join("record");
        let agent = fake_acp_agent(temp.path());
        store
            .upsert(local_acp_instance("acp_local", &agent, &record))
            .unwrap();
        let methods = store
            .acp_harness("acp_local")
            .unwrap()
            .auth_methods()
            .await
            .unwrap();
        assert_eq!(
            methods
                .iter()
                .map(|m| (m.id.as_str(), m.kind.as_str()))
                .collect::<Vec<_>>(),
            [("browser", "agent"), ("setup", "terminal")]
        );

        let accounts = AgentAccounts::new(AgentAccountsConfig::isolated(temp.path()));
        let settle = |method: &str| {
            let start = accounts.start_acp_instance_login(
                HarnessId::Acp,
                store.acp_harness("acp_local").unwrap(),
                Some(method.into()),
                None,
            );
            let accounts = accounts.clone();
            async move {
                for _ in 0..200 {
                    let poll = accounts.poll_login(&start.login_id).await.unwrap();
                    if poll.status != AgentLoginStatus::Pending {
                        return poll;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                panic!("login never settled");
            }
        };
        let refused = settle("setup").await;
        assert_eq!(refused.status, AgentLoginStatus::Error);
        assert!(
            refused
                .message
                .as_deref()
                .unwrap()
                .contains("runs in a terminal"),
            "{refused:?}"
        );
        assert!(
            !std::fs::read_to_string(&record)
                .unwrap()
                .contains("authenticate=")
        );
        assert_eq!(settle("browser").await.status, AgentLoginStatus::Done);
        let authenticated = std::fs::read_to_string(&record).unwrap();
        assert_eq!(authenticated.matches("authenticate=").count(), 1);
        assert!(
            authenticated.contains(r#""methodId":"browser""#),
            "{authenticated}"
        );
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
        assert!(store.upsert(instance("Work", HarnessId::Codex)).is_err());
        let mut acp_on_codex = instance("codex_acp", HarnessId::Codex);
        acp_on_codex.config = InstanceConfig::Acp {
            source: AcpSource::Local {
                command: "agent".into(),
                args: Vec::new(),
            },
            env: Default::default(),
            icon_path: None,
        };
        assert!(store.upsert(acp_on_codex).is_err());
        assert!(
            store
                .upsert(instance("acp_native", HarnessId::Acp))
                .is_err()
        );
        assert_eq!(ids(&store.snapshot()), ["work"]);
    }
}
