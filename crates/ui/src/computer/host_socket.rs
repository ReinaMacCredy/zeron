use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SocketIdentity {
    dev: u64,
    ino: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Probe {
    Refused,
    Missing,
}

fn socket_identity(endpoint: &Path) -> Result<Option<SocketIdentity>, String> {
    let meta = match std::fs::symlink_metadata(endpoint) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };
    if !meta.file_type().is_socket() {
        return Err(format!(
            "Refusing to replace a non-socket host path: {}.",
            endpoint.display()
        ));
    }
    if meta.uid() != unsafe { libc::getuid() } {
        return Err(format!(
            "Refusing to replace a host socket owned by another user: {}.",
            endpoint.display()
        ));
    }
    Ok(Some(SocketIdentity {
        dev: meta.dev(),
        ino: meta.ino(),
    }))
}

pub fn clear_stale_cua_host_socket(endpoint: &Path) -> Result<(), String> {
    let Some(candidate) = socket_identity(endpoint)? else {
        return Ok(());
    };
    let probe = match UnixStream::connect(endpoint) {
        Ok(_) => {
            return Err(format!(
                "A live host already listens on {}.",
                endpoint.display()
            ));
        }
        Err(err) if err.raw_os_error() == Some(libc::ECONNREFUSED) => Probe::Refused,
        Err(err) if err.kind() == io::ErrorKind::NotFound => Probe::Missing,
        Err(err) => return Err(err.to_string()),
    };
    let Some(current) = socket_identity(endpoint)? else {
        return Ok(());
    };
    if probe == Probe::Missing || candidate != current {
        return Err(format!(
            "The host socket changed while checking {}; retry setup.",
            endpoint.display()
        ));
    }
    std::fs::remove_file(endpoint).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn does_not_touch_a_missing_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = dir.path().join("host.sock");
        assert_eq!(clear_stale_cua_host_socket(&endpoint), Ok(()));
        assert!(!endpoint.exists());
    }

    #[test]
    fn preserves_ordinary_files_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("host.sock");
        std::fs::write(&file, "preserve").unwrap();
        assert!(
            clear_stale_cua_host_socket(&file)
                .unwrap_err()
                .contains("non-socket")
        );
        let link = dir.path().join("link.sock");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(
            clear_stale_cua_host_socket(&link)
                .unwrap_err()
                .contains("non-socket")
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "preserve");
        assert!(link.symlink_metadata().is_ok());
    }

    #[test]
    fn preserves_a_live_listener() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = dir.path().join("host.sock");
        let _listener = UnixListener::bind(&endpoint).unwrap();
        assert!(
            clear_stale_cua_host_socket(&endpoint)
                .unwrap_err()
                .contains("live host")
        );
        assert!(endpoint.exists());
    }

    #[test]
    fn does_not_interpret_access_errors_as_a_stale_socket() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let endpoint = dir.path().join("host.sock");
        drop(UnixListener::bind(&endpoint).unwrap());
        std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o000)).unwrap();
        let error = clear_stale_cua_host_socket(&endpoint).unwrap_err();
        assert!(error.contains("denied"), "{error}");
        assert!(endpoint.symlink_metadata().is_ok());
    }

    #[test]
    fn replaces_an_unchanged_socket_only_after_a_refused_connection() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = dir.path().join("host.sock");
        drop(UnixListener::bind(&endpoint).unwrap());
        assert!(endpoint.exists());
        assert_eq!(clear_stale_cua_host_socket(&endpoint), Ok(()));
        assert!(!endpoint.exists());
    }
}
