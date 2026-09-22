use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const MARKER_NAME: &str = ".zeron-cua-runtime.json";
const MARKER_SCHEMA: &str = "zeron-cua-runtime";
const DIRECTORY_PREFIX: &str = "zeron-cua-";
const DIRECTORY_SUFFIX_LEN: usize = 6;
const STALE_AFTER: Duration = Duration::from_secs(30);
const MARKER_MAX_BYTES: u64 = 1_024;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

pub fn cua_host_process_is_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

fn is_runtime_directory_name(name: &str) -> bool {
    name.strip_prefix(DIRECTORY_PREFIX).is_some_and(|suffix| {
        suffix.len() == DIRECTORY_SUFFIX_LEN && suffix.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

pub fn create_cua_runtime_directory(parent: &Path) -> io::Result<PathBuf> {
    loop {
        let suffix: String = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(DIRECTORY_SUFFIX_LEN)
            .collect();
        let directory = parent.join(format!("{DIRECTORY_PREFIX}{suffix}"));
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {
                mark_cua_runtime_directory(&directory)?;
                return Ok(directory);
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
}

pub fn mark_cua_runtime_directory(directory: &Path) -> io::Result<()> {
    let marker = json!({
        "schema": MARKER_SCHEMA,
        "version": 1,
        "directory": directory.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
        "hostPid": std::process::id(),
        "ownerUid": current_uid(),
        "createdAt": now_ms(),
    });
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join(MARKER_NAME))?;
    file.write_all(format!("{marker}\n").as_bytes())
}

pub struct RuntimeSweep<'a> {
    pub directory: &'a Path,
    pub live_socket_dirs: &'a HashSet<PathBuf>,
    pub now_ms: u64,
    pub is_process_alive: &'a dyn Fn(u32) -> bool,
}

pub fn sweep_owned_cua_runtime_directories(sweep: RuntimeSweep<'_>) -> Vec<String> {
    let uid = current_uid();
    let protected: HashSet<PathBuf> = sweep
        .live_socket_dirs
        .iter()
        .map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
        .collect();
    let Ok(entries) = fs::read_dir(sweep.directory) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !is_runtime_directory_name(&name) {
            continue;
        }
        let directory = sweep.directory.join(&name);
        if sweep_one(&directory, &name, uid, &protected, &sweep) == Some(()) {
            removed.push(name);
        }
    }
    removed
}

fn private_directory(meta: &fs::Metadata, uid: u32) -> bool {
    meta.is_dir() && !meta.file_type().is_symlink() && meta.uid() == uid && meta.mode() & 0o077 == 0
}

fn sweep_one(
    directory: &Path,
    name: &str,
    uid: u32,
    protected: &HashSet<PathBuf>,
    sweep: &RuntimeSweep<'_>,
) -> Option<()> {
    let before = fs::symlink_metadata(directory).ok()?;
    let modified_ms = before
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    if !private_directory(&before, uid)
        || sweep.now_ms.saturating_sub(modified_ms) < STALE_AFTER.as_millis() as u64
        || protected.contains(&fs::canonicalize(directory).ok()?)
    {
        return None;
    }
    let mut marker = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(MARKER_NAME))
        .ok()?;
    let marker_meta = marker.metadata().ok()?;
    if !marker_meta.is_file()
        || marker_meta.uid() != uid
        || marker_meta.nlink() != 1
        || marker_meta.mode() & 0o077 != 0
        || marker_meta.len() == 0
        || marker_meta.len() > MARKER_MAX_BYTES
    {
        return None;
    }
    let mut bytes = Vec::new();
    (&mut marker as &mut File)
        .take(MARKER_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.is_empty() || bytes.len() as u64 > MARKER_MAX_BYTES {
        return None;
    }
    let data: Value = serde_json::from_slice(&bytes).ok()?;
    let data = data.as_object()?;
    let host_pid = data
        .get("hostPid")
        .and_then(Value::as_u64)
        .filter(|pid| *pid > 0 && *pid <= 0x7fff_ffff)?;
    let created_at = data
        .get("createdAt")
        .and_then(Value::as_f64)
        .filter(|at| at.is_finite() && *at > 0.0)?;
    let owned = data.get("schema").and_then(Value::as_str) == Some(MARKER_SCHEMA)
        && data.get("version").and_then(Value::as_u64) == Some(1)
        && data.get("directory").and_then(Value::as_str) == Some(name)
        && data.get("ownerUid").and_then(Value::as_u64) == Some(u64::from(uid))
        && (sweep.now_ms as f64) - created_at >= STALE_AFTER.as_millis() as f64
        && !(sweep.is_process_alive)(host_pid as u32);
    if !owned {
        return None;
    }
    let after = fs::symlink_metadata(directory).ok()?;
    if !private_directory(&after, uid) || after.ino() != before.ino() || after.dev() != before.dev()
    {
        return None;
    }
    fs::remove_dir_all(directory).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const OLD: Duration = Duration::from_secs(120);

    fn old_ms() -> u64 {
        now_ms() - OLD.as_millis() as u64
    }

    fn age(path: &Path) {
        let when = SystemTime::now() - OLD;
        File::open(path).unwrap().set_modified(when).unwrap();
    }

    struct Owned {
        directory: PathBuf,
        marker_path: PathBuf,
        marker: Value,
    }

    fn owned(root: &Path, name: &str) -> Owned {
        let directory = root.join(name);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        mark_cua_runtime_directory(&directory).unwrap();
        let marker_path = directory.join(MARKER_NAME);
        let mut marker: Value = serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
        marker["createdAt"] = json!(old_ms());
        fs::write(&marker_path, marker.to_string()).unwrap();
        fs::write(directory.join("state"), "owned temporary state").unwrap();
        age(&directory);
        Owned {
            directory,
            marker_path,
            marker,
        }
    }

    fn sweep(root: &Path, live: &HashSet<PathBuf>) -> Vec<String> {
        sweep_owned_cua_runtime_directories(RuntimeSweep {
            directory: root,
            live_socket_dirs: live,
            now_ms: now_ms(),
            is_process_alive: &|_| false,
        })
    }

    #[test]
    fn never_removes_similarly_named_tools_or_an_unmarked_legacy_runtime() {
        let root = tempfile::tempdir().unwrap();
        let names = [
            "zeron-cua-tools",
            "zeron-cua-native-work",
            "zeron-cua-Ab123Z",
        ];
        for name in names {
            let path = root.path().join(name);
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            fs::write(path.join("important"), "preserve").unwrap();
            age(&path);
        }
        assert!(sweep(root.path(), &HashSet::new()).is_empty());
        for name in names {
            assert_eq!(
                fs::read_to_string(root.path().join(name).join("important")).unwrap(),
                "preserve"
            );
        }
    }

    #[test]
    fn protects_a_live_lazy_host_even_when_it_never_spawned_a_driver() {
        let root = tempfile::tempdir().unwrap();
        let runtime = owned(root.path(), "zeron-cua-Ab123Z");
        let removed = sweep_owned_cua_runtime_directories(RuntimeSweep {
            directory: root.path(),
            live_socket_dirs: &HashSet::new(),
            now_ms: now_ms(),
            is_process_alive: &cua_host_process_is_alive,
        });
        assert!(removed.is_empty());
        assert!(runtime.directory.exists());
    }

    #[test]
    fn removes_only_a_marked_private_stale_directory_whose_owner_is_dead() {
        let root = tempfile::tempdir().unwrap();
        let runtime = owned(root.path(), "zeron-cua-Ab123Z");
        assert_eq!(
            sweep(root.path(), &HashSet::new()),
            vec!["zeron-cua-Ab123Z"]
        );
        assert!(!runtime.directory.exists());
    }

    #[test]
    fn keeps_the_directory_of_a_still_running_driver_after_its_host_died() {
        let root = tempfile::tempdir().unwrap();
        let runtime = owned(root.path(), "zeron-cua-Ab123Z");
        let live = HashSet::from([runtime.directory.clone()]);
        assert!(sweep(root.path(), &live).is_empty());
        assert!(runtime.directory.exists());
    }

    #[test]
    fn never_follows_a_runtime_directory_or_marker_symlink() {
        let root = tempfile::tempdir().unwrap();
        let external = root.path().join("preserved-evidence");
        fs::DirBuilder::new().mode(0o700).create(&external).unwrap();
        fs::write(external.join("important"), "preserve").unwrap();
        std::os::unix::fs::symlink(&external, root.path().join("zeron-cua-link99")).unwrap();
        let runtime = owned(root.path(), "zeron-cua-Ab123Z");
        let marker = fs::read(&runtime.marker_path).unwrap();
        let external_marker = external.join("marker.json");
        fs::write(&external_marker, marker).unwrap();
        fs::set_permissions(&external_marker, fs::Permissions::from_mode(0o600)).unwrap();
        fs::remove_file(&runtime.marker_path).unwrap();
        std::os::unix::fs::symlink(&external_marker, &runtime.marker_path).unwrap();
        age(&runtime.directory);
        assert!(sweep(root.path(), &HashSet::new()).is_empty());
        assert_eq!(
            fs::read_to_string(external.join("important")).unwrap(),
            "preserve"
        );
        assert!(runtime.directory.exists());
    }

    #[test]
    fn preserves_malformed_foreign_or_newly_created_ownership() {
        let overrides = [
            ("ownerUid", json!(current_uid() + 1)),
            ("directory", json!("zeron-cua-Other1")),
            ("schema", json!("some-other-application")),
            ("hostPid", json!(0)),
            ("hostPid", json!("123")),
            ("createdAt", json!(now_ms())),
        ];
        for (key, value) in overrides {
            let root = tempfile::tempdir().unwrap();
            let runtime = owned(root.path(), "zeron-cua-Ab123Z");
            let mut marker = runtime.marker.clone();
            marker["createdAt"] = json!(old_ms());
            marker[key] = value.clone();
            fs::write(&runtime.marker_path, marker.to_string()).unwrap();
            age(&runtime.directory);
            assert!(
                sweep(root.path(), &HashSet::new()).is_empty(),
                "{key}={value}"
            );
            assert!(runtime.directory.exists());
        }
    }

    #[test]
    fn preserves_a_fifo_marker_without_waiting_for_a_writer() {
        let root = tempfile::tempdir().unwrap();
        let runtime = owned(root.path(), "zeron-cua-Ab123Z");
        fs::remove_file(&runtime.marker_path).unwrap();
        let path =
            std::ffi::CString::new(runtime.marker_path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        age(&runtime.directory);
        assert!(sweep(root.path(), &HashSet::new()).is_empty());
        assert!(runtime.directory.exists());
    }

    #[test]
    fn does_not_accept_a_marker_writable_by_another_user() {
        let root = tempfile::tempdir().unwrap();
        let runtime = owned(root.path(), "zeron-cua-Ab123Z");
        fs::set_permissions(&runtime.marker_path, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(sweep(root.path(), &HashSet::new()).is_empty());
        assert!(runtime.directory.exists());
    }

    #[test]
    fn treats_inaccessible_process_ownership_as_alive() {
        assert!(cua_host_process_is_alive(std::process::id()));
        assert!(cua_host_process_is_alive(1));
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!cua_host_process_is_alive(pid));
    }

    #[test]
    fn creates_a_private_marked_runtime_directory() {
        let root = tempfile::tempdir().unwrap();
        let directory = create_cua_runtime_directory(root.path()).unwrap();
        let name = directory.file_name().unwrap().to_str().unwrap();
        assert!(is_runtime_directory_name(name), "{name}");
        let meta = fs::metadata(&directory).unwrap();
        assert_eq!(meta.mode() & 0o777, 0o700);
        let marker: Value =
            serde_json::from_slice(&fs::read(directory.join(MARKER_NAME)).unwrap()).unwrap();
        assert_eq!(marker["directory"], json!(name));
        assert_eq!(marker["hostPid"], json!(std::process::id()));
    }
}
