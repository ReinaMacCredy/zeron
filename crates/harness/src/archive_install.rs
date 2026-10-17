//! managed installs for acp agents distributed as prebuilt zip archives
//! (the ACP registry's `binary` distribution), such as Google's Antigravity
//! server.
//!
//! same contract as the npm installs in [`crate::adapter_install`]: the pinned
//! archive lands ONCE in `~/.zeron/adapters/<name>/<version>`, extraction runs
//! in a `.tmp-*` sibling that is renamed into place only after the entry
//! resolves and the marker is written, so a killed download never passes for
//! a working install.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use sha2::{Digest, Sha256, Sha512};
use tokio::io::AsyncWriteExt;

use crate::HarnessError;
use crate::adapter_install::{OK_MARKER, adapters_root, install_lock};

/// a pinned archive: where to fetch it and which file inside runs the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchivePin {
    pub name: &'static str,
    pub version: &'static str,
    pub url: &'static str,
    /// path of the executable relative to the archive root.
    pub entry: &'static str,
    pub sha512: &'static str,
}

/// multi-hundred-megabyte archives outlast any whole-request deadline on a
/// slow link, so only a stalled stream counts as a failure.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ARCHIVE_BYTES: u64 = 768 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 4_096;

/// an archive release whose integrity is proven after extraction (by a
/// vendor code signature) rather than by a digest pinned in zeron's source.
pub struct VerifiedRelease<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub url: &'a str,
    pub entry: &'a str,
    /// written as the install marker once verification passes.
    pub marker: &'a str,
}

pub struct RegistryBinary<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub url: &'a str,
    pub cmd: &'a str,
    pub sha256: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    Raw,
}

impl ArchiveKind {
    fn from_url(url: &str) -> Self {
        let path = url
            .split(['?', '#'])
            .next()
            .unwrap_or(url)
            .to_ascii_lowercase();
        if path.ends_with(".zip") {
            Self::Zip
        } else if path.ends_with(".tar.gz") || path.ends_with(".tgz") {
            Self::TarGz
        } else if path.ends_with(".tar.bz2") || path.ends_with(".tbz2") {
            Self::TarBz2
        } else if path.ends_with(".tar") {
            Self::Tar
        } else {
            Self::Raw
        }
    }

    fn download_name(self) -> &'static str {
        match self {
            Self::Zip => "download.zip",
            Self::Tar => "download.tar",
            Self::TarGz => "download.tar.gz",
            Self::TarBz2 => "download.tar.bz2",
            Self::Raw => "download",
        }
    }
}

struct Source<'a> {
    name: &'a str,
    version: &'a str,
    url: &'a str,
    entry: &'a str,
    sha512: Option<&'a str>,
    sha256: Option<&'a str>,
    kind: ArchiveKind,
    marker: &'a str,
}

impl<'a> From<&'a ArchivePin> for Source<'a> {
    fn from(pin: &'a ArchivePin) -> Self {
        Self {
            name: pin.name,
            version: pin.version,
            url: pin.url,
            entry: pin.entry,
            sha512: Some(pin.sha512),
            sha256: None,
            kind: ArchiveKind::Zip,
            marker: pin.sha512,
        }
    }
}

impl<'a> From<&'a VerifiedRelease<'a>> for Source<'a> {
    fn from(release: &'a VerifiedRelease<'a>) -> Self {
        Self {
            name: release.name,
            version: release.version,
            url: release.url,
            entry: release.entry,
            sha512: None,
            sha256: None,
            kind: ArchiveKind::Zip,
            marker: release.marker,
        }
    }
}

fn install_dir(name: &str, version: &str) -> Option<PathBuf> {
    adapters_root().map(|root| root.join(name).join(version))
}

/// the entry of a completed install whose marker matches, `None` when absent.
pub fn installed_entry_with_marker(
    name: &str,
    version: &str,
    entry: &str,
    marker: &str,
) -> Option<PathBuf> {
    let dir = install_dir(name, version)?;
    if std::fs::read_to_string(dir.join(OK_MARKER)).ok()?.trim() != marker {
        return None;
    }
    let entry = dir.join(entry);
    entry.is_file().then_some(entry)
}

/// the entry of a completed install, `None` when absent.
pub fn installed_entry(pin: &ArchivePin) -> Option<PathBuf> {
    installed_entry_with_marker(pin.name, pin.version, pin.entry, pin.sha512)
}

/// the marker of every completed-looking version directory under `name`.
pub(crate) fn installed_versions(name: &str) -> Vec<(String, String)> {
    let Some(root) = adapters_root().map(|root| root.join(name)) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let version = entry.file_name().into_string().ok()?;
            let marker = std::fs::read_to_string(entry.path().join(OK_MARKER)).ok()?;
            Some((version, marker.trim().to_owned()))
        })
        .collect()
}

pub async fn ensure_installed(
    pin: ArchivePin,
    display_name: &str,
) -> Result<PathBuf, HarnessError> {
    install(Source::from(&pin), display_name, |_| async { Ok(()) }).await
}

/// install a release that has no pinned digest. `verify` inspects the
/// extracted tree before it is committed; any error discards the download.
pub async fn install_verified<F, Fut>(
    release: &VerifiedRelease<'_>,
    display_name: &str,
    verify: F,
) -> Result<PathBuf, HarnessError>
where
    F: FnOnce(PathBuf) -> Fut,
    Fut: std::future::Future<Output = Result<(), HarnessError>>,
{
    install(Source::from(release), display_name, verify).await
}

pub async fn install_registry_binary(
    binary: &RegistryBinary<'_>,
    display_name: &str,
) -> Result<PathBuf, HarnessError> {
    let entry = registry_entry(binary.cmd)?;
    let entry = entry.to_str().ok_or_else(|| {
        HarnessError::Install(format!("the registry command {} is not UTF-8", binary.cmd))
    })?;
    let marker = match binary.sha256 {
        Some(sha256) => format!("sha256:{}", sha256.to_ascii_lowercase()),
        None => format!("unverified:{}", binary.url),
    };
    let sha256 = binary.sha256.map(str::to_ascii_lowercase);
    let source = Source {
        name: binary.name,
        version: binary.version,
        url: binary.url,
        entry,
        sha512: None,
        sha256: sha256.as_deref(),
        kind: ArchiveKind::from_url(binary.url),
        marker: &marker,
    };
    install(source, display_name, |_| async { Ok(()) }).await
}

pub fn registry_entry(cmd: &str) -> Result<PathBuf, HarnessError> {
    safe_relative(Path::new(cmd.strip_prefix("./").unwrap_or(cmd)))
        .filter(|path| path.components().next().is_some())
        .ok_or_else(|| {
            HarnessError::Install(format!(
                "the registry command {cmd} does not name a file inside the install"
            ))
        })
}

fn safe_relative(path: &Path) -> Option<PathBuf> {
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(relative)
}

async fn install<F, Fut>(
    source: Source<'_>,
    display_name: &str,
    verify: F,
) -> Result<PathBuf, HarnessError>
where
    F: FnOnce(PathBuf) -> Fut,
    Fut: std::future::Future<Output = Result<(), HarnessError>>,
{
    let installed =
        || installed_entry_with_marker(source.name, source.version, source.entry, source.marker);
    if let Some(entry) = installed() {
        return Ok(entry);
    }
    let _guard = install_lock().lock().await;
    if let Some(entry) = installed() {
        return Ok(entry);
    }

    let root = adapters_root().ok_or_else(|| {
        HarnessError::Install("cannot locate an adapters directory (HOME is unset)".into())
    })?;
    let final_dir = install_dir(source.name, source.version).expect("root resolved");
    let tmp_dir = root.join(format!(
        ".tmp-{}-{}-{}",
        source.name.replace('/', "__"),
        source.version,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir)?;
    tracing::info!(
        target: "zeron_harness::adapter_install",
        url = source.url,
        dir = %tmp_dir.display(),
        "installing {display_name} ACP server"
    );
    let prepared = match download_and_extract(&source, &tmp_dir, display_name).await {
        Ok(()) => verify(tmp_dir.clone()).await,
        Err(error) => Err(error),
    };
    if let Err(error) = prepared {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(error);
    }
    std::fs::write(tmp_dir.join(OK_MARKER), format!("{}\n", source.marker))?;
    if let Some(parent) = final_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(&tmp_dir, &final_dir).is_err() {
        if installed().is_none() {
            let _ = std::fs::remove_dir_all(&final_dir);
            std::fs::rename(&tmp_dir, &final_dir)?;
        } else {
            let _ = std::fs::remove_dir_all(&tmp_dir);
        }
    }
    installed().ok_or_else(|| {
        HarnessError::Install(format!(
            "install of the {display_name} ACP server {} finished but {} did not resolve",
            source.version, source.entry
        ))
    })
}

async fn download_and_extract(
    source: &Source<'_>,
    dir: &Path,
    display_name: &str,
) -> Result<(), HarnessError> {
    let entry = dir.join(source.entry);
    if source.kind == ArchiveKind::Raw {
        if let Some(parent) = entry.parent() {
            std::fs::create_dir_all(parent)?;
        }
        download(source, &entry, display_name).await?;
    } else {
        let archive = dir.join(source.kind.download_name());
        download(source, &archive, display_name).await?;
        match source.kind {
            ArchiveKind::Zip => extract_zip(&archive, dir).await?,
            kind => extract_tar(kind, &archive, dir).await?,
        }
        let _ = std::fs::remove_file(&archive);
    }
    if !entry.is_file() {
        return Err(HarnessError::Install(format!(
            "the {display_name} archive ({}) has no {}",
            source.url, source.entry
        )));
    }
    mark_executables(dir)?;
    Ok(())
}

async fn download(
    source: &Source<'_>,
    dest: &Path,
    display_name: &str,
) -> Result<(), HarnessError> {
    let url = source.url;
    let failed = |detail: String| {
        HarnessError::Install(format!(
            "download of the {display_name} ACP server ({url}) failed: {detail}"
        ))
    };
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| failed(e.to_string()))?;
    let response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| failed(e.to_string()))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
    {
        return Err(failed(format!(
            "server reported an archive larger than the {} MiB limit",
            MAX_ARCHIVE_BYTES / (1024 * 1024)
        )));
    }
    let mut body = response.bytes_stream();
    let mut file = tokio::fs::File::create(dest).await?;
    let mut received = 0_u64;
    let mut sha512 = source.sha512.map(|_| Sha512::new());
    let mut sha256 = source.sha256.map(|_| Sha256::new());
    loop {
        let chunk = tokio::time::timeout(STALL_TIMEOUT, body.next())
            .await
            .map_err(|_| {
                failed(format!(
                    "no data for {}s — check your network",
                    STALL_TIMEOUT.as_secs()
                ))
            })?;
        match chunk {
            Some(Ok(bytes)) => {
                received = received.saturating_add(bytes.len() as u64);
                if received > MAX_ARCHIVE_BYTES {
                    return Err(failed(format!(
                        "archive exceeded the {} MiB limit",
                        MAX_ARCHIVE_BYTES / (1024 * 1024)
                    )));
                }
                if let Some(digest) = &mut sha512 {
                    digest.update(&bytes);
                }
                if let Some(digest) = &mut sha256 {
                    digest.update(&bytes);
                }
                file.write_all(&bytes).await?;
            }
            Some(Err(e)) => return Err(failed(e.to_string())),
            None => break,
        }
    }
    file.flush().await?;
    if let (Some(expected), Some(digest)) = (source.sha512, sha512) {
        let actual = format!("{:x}", digest.finalize());
        if actual != expected {
            return Err(failed(format!(
                "SHA-512 mismatch (expected {expected}, got {actual})"
            )));
        }
    }
    if let (Some(expected), Some(digest)) = (source.sha256, sha256) {
        let actual = format!("{:x}", digest.finalize());
        if actual != expected {
            return Err(failed(format!(
                "SHA-256 mismatch (expected {expected}, got {actual})"
            )));
        }
    }
    Ok(())
}

pub(crate) async fn extract_zip(archive: &Path, dest: &Path) -> Result<(), HarnessError> {
    let archive = archive.to_owned();
    let dest = dest.to_owned();
    tokio::task::spawn_blocking(move || extract_zip_blocking(&archive, &dest))
        .await
        .map_err(|error| {
            HarnessError::Install(format!("archive extraction task failed: {error}"))
        })?
}

fn archive_entry_path(
    dest: &Path,
    enclosed_name: Option<&Path>,
    unix_mode: Option<u32>,
) -> Result<PathBuf, HarnessError> {
    let name = enclosed_name
        .ok_or_else(|| HarnessError::Install("archive contains an unsafe entry path".into()))?;
    if let Some(mode) = unix_mode {
        let kind = mode & 0o170000;
        if !matches!(kind, 0 | 0o040000 | 0o100000) {
            return Err(HarnessError::Install(format!(
                "archive entry {} is a link or special file",
                name.display()
            )));
        }
    }
    Ok(dest.join(name))
}

fn extract_zip_blocking(archive_path: &Path, dest: &Path) -> Result<(), HarnessError> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| HarnessError::Install(format!("invalid zip archive: {error}")))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(HarnessError::Install(format!(
            "archive has {} entries, exceeding the {MAX_ARCHIVE_ENTRIES} entry limit",
            archive.len()
        )));
    }
    let mut extracted = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            HarnessError::Install(format!("could not read archive entry {index}: {error}"))
        })?;
        extracted = extracted.checked_add(entry.size()).ok_or_else(|| {
            HarnessError::Install("archive's extracted size overflows its limit".into())
        })?;
        if extracted > MAX_EXTRACTED_BYTES {
            return Err(HarnessError::Install(format!(
                "archive expands beyond the {} MiB limit",
                MAX_EXTRACTED_BYTES / (1024 * 1024)
            )));
        }
        let enclosed_name = entry.enclosed_name();
        let output = archive_entry_path(dest, enclosed_name.as_deref(), entry.unix_mode())?;
        if output == archive_path {
            return Err(HarnessError::Install(
                "archive attempts to overwrite its download file".into(),
            ));
        }
        if entry.is_dir() {
            std::fs::create_dir_all(&output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output_file = std::fs::File::create(&output)?;
        let copied = std::io::copy(&mut entry, &mut output_file)?;
        if copied != entry.size() {
            return Err(HarnessError::Install(format!(
                "archive entry {} extracted {copied} bytes but declared {}",
                output.display(),
                entry.size()
            )));
        }
        output_file.flush()?;
    }
    Ok(())
}

async fn extract_tar(kind: ArchiveKind, archive: &Path, dest: &Path) -> Result<(), HarnessError> {
    let archive = archive.to_owned();
    let dest = dest.to_owned();
    tokio::task::spawn_blocking(move || extract_tar_blocking(kind, &archive, &dest))
        .await
        .map_err(|error| {
            HarnessError::Install(format!("archive extraction task failed: {error}"))
        })?
}

/// counts decompressed bytes, so a small archive cannot expand past the limit.
struct Capped<R> {
    inner: R,
    remaining: u64,
    exceeded: bool,
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        match self.remaining.checked_sub(read as u64) {
            Some(remaining) => {
                self.remaining = remaining;
                Ok(read)
            }
            None => {
                self.exceeded = true;
                Err(std::io::Error::other("archive expands beyond its limit"))
            }
        }
    }
}

fn extract_tar_blocking(
    kind: ArchiveKind,
    archive_path: &Path,
    dest: &Path,
) -> Result<(), HarnessError> {
    let file = std::io::BufReader::new(std::fs::File::open(archive_path)?);
    let reader: Box<dyn Read> = match kind {
        ArchiveKind::TarGz => Box::new(flate2::read::MultiGzDecoder::new(file)),
        ArchiveKind::TarBz2 => Box::new(bzip2::read::MultiBzDecoder::new(file)),
        _ => Box::new(file),
    };
    let mut capped = Capped {
        inner: reader,
        remaining: MAX_EXTRACTED_BYTES,
        exceeded: false,
    };
    let result = unpack_tar(&mut capped, archive_path, dest);
    if capped.exceeded {
        return Err(HarnessError::Install(format!(
            "archive expands beyond the {} MiB limit",
            MAX_EXTRACTED_BYTES / (1024 * 1024)
        )));
    }
    result
}

fn unpack_tar(reader: impl Read, archive_path: &Path, dest: &Path) -> Result<(), HarnessError> {
    let invalid =
        |error: std::io::Error| HarnessError::Install(format!("invalid tar archive: {error}"));
    let mut archive = tar::Archive::new(reader);
    for (index, entry) in archive.entries().map_err(invalid)?.enumerate() {
        if index >= MAX_ARCHIVE_ENTRIES {
            return Err(HarnessError::Install(format!(
                "archive has more than {MAX_ARCHIVE_ENTRIES} entries"
            )));
        }
        let mut entry = entry.map_err(invalid)?;
        let kind = entry.header().entry_type();
        if kind == tar::EntryType::XGlobalHeader {
            continue;
        }
        let path = entry.path().map_err(invalid)?.into_owned();
        let output = safe_relative(&path)
            .map(|relative| dest.join(relative))
            .ok_or_else(|| {
                HarnessError::Install(format!(
                    "archive entry {} has an unsafe path",
                    path.display()
                ))
            })?;
        if output == archive_path {
            return Err(HarnessError::Install(
                "archive attempts to overwrite its download file".into(),
            ));
        }
        match kind {
            tar::EntryType::Directory => std::fs::create_dir_all(&output)?,
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                if let Some(parent) = output.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut output_file = std::fs::File::create(&output)?;
                std::io::copy(&mut entry, &mut output_file).map_err(invalid)?;
                output_file.flush()?;
            }
            _ => {
                return Err(HarnessError::Install(format!(
                    "archive entry {} is a link or special file",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

/// archives built off-unix can lose the executable bit, and the entry spawns
/// sibling helper binaries of its own.
#[cfg(unix)]
fn mark_executables(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            mark_executables(&entry.path())?;
        } else if entry.file_type()?.is_file() {
            let mut permissions = entry.metadata()?.permissions();
            permissions.set_mode(permissions.mode() | 0o755);
            std::fs::set_permissions(entry.path(), permissions)?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn mark_executables(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn extracts_zip_and_marks_files_executable() {
        use std::os::unix::fs::PermissionsExt;
        if crate::acp::find_on_paths("zip", Vec::new()).is_none() {
            return;
        }
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("server.par"), "#!/bin/sh\n").unwrap();
        std::fs::write(src.path().join("helper"), "bin").unwrap();
        let archive = src.path().join("bundle.zip");
        let zipped = std::process::Command::new("zip")
            .args(["-q", "bundle.zip", "server.par", "helper"])
            .current_dir(src.path())
            .status()
            .unwrap();
        assert!(zipped.success());

        let dest = tempfile::tempdir().unwrap();
        extract_zip(&archive, dest.path()).await.unwrap();
        mark_executables(dest.path()).unwrap();
        for name in ["server.par", "helper"] {
            let mode = std::fs::metadata(dest.path().join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "{name} is executable");
        }
    }

    #[tokio::test]
    async fn corrupt_archive_reports_the_parser_failure() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("broken.zip");
        std::fs::write(&archive, "not a zip").unwrap();
        let error = extract_zip(&archive, dir.path()).await.unwrap_err();
        assert!(
            matches!(error, HarnessError::Install(ref m) if m.contains("invalid zip archive")),
            "{error}"
        );
    }

    #[test]
    fn archive_entries_reject_traversal_links_and_special_files() {
        let dest = Path::new("/safe/root");
        assert!(archive_entry_path(dest, None, Some(0o100644)).is_err());
        assert!(archive_entry_path(dest, Some(Path::new("link")), Some(0o120777)).is_err());
        assert!(archive_entry_path(dest, Some(Path::new("device")), Some(0o020666)).is_err());
        assert_eq!(
            archive_entry_path(dest, Some(Path::new("bin/server")), Some(0o100755)).unwrap(),
            dest.join("bin/server")
        );
    }

    /// runs in a child process, since installs resolve through the
    /// process-wide `ZERON_ADAPTERS_DIR`.
    #[test]
    fn verified_installs_commit_only_after_verification_passes() {
        if std::env::var_os("ZERON_TEST_VERIFIED_INSTALL").is_none() {
            let root = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "archive_install::tests::verified_installs_commit_only_after_verification_passes",
                    "--nocapture",
                ])
                .env("ZERON_ADAPTERS_DIR", root.path())
                .env("ZERON_TEST_VERIFIED_INSTALL", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
                zip.start_file("server", zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(b"#!/bin/sh\n").unwrap();
                let bytes = zip.finish().unwrap().into_inner();
                let url = serve(bytes).await;
                let release = VerifiedRelease {
                    name: "verified-acp",
                    version: "2.0.0",
                    url: &url,
                    entry: "server",
                    marker: "vendor-signature",
                };

                let rejected = install_verified(&release, "Test", |dir| async move {
                    assert!(dir.join("server").is_file());
                    Err(HarnessError::Install("not signed".into()))
                })
                .await
                .unwrap_err();
                assert!(rejected.to_string().contains("not signed"), "{rejected}");
                let root = adapters_root().unwrap();
                assert!(!root.join("verified-acp").exists());
                assert!(
                    std::fs::read_dir(&root)
                        .unwrap()
                        .flatten()
                        .all(|entry| !entry.file_name().to_string_lossy().starts_with(".tmp-")),
                    "the rejected download is discarded"
                );

                let entry = install_verified(&release, "Test", |_| async { Ok(()) })
                    .await
                    .unwrap();
                assert_eq!(entry, root.join("verified-acp/2.0.0/server"));
                assert_eq!(
                    installed_versions("verified-acp"),
                    vec![("2.0.0".to_owned(), "vendor-signature".to_owned())]
                );
                assert_eq!(
                    installed_entry_with_marker("verified-acp", "2.0.0", "server", "other"),
                    None
                );
            });
    }

    /// answer every request with `body`, returning the url to fetch it.
    async fn serve(body: Vec<u8>) -> String {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/archive.zip", listener.local_addr().unwrap());
        let body = std::sync::Arc::new(body);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    let mut headers = Vec::new();
                    let mut buf = [0; 4096];
                    while !headers.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => headers.extend_from_slice(&buf[..read]),
                        }
                    }
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        url
    }

    fn tar_bytes(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        build(&mut builder);
        builder.into_inner().unwrap()
    }

    fn agent_tar() -> Vec<u8> {
        tar_bytes(|builder| {
            let mut dir = tar::Header::new_gnu();
            dir.set_entry_type(tar::EntryType::Directory);
            dir.set_mode(0o755);
            dir.set_size(0);
            builder
                .append_data(&mut dir, "./bin/", std::io::empty())
                .unwrap();
            for (path, body) in [("./bin/agent", "#!/bin/sh\n"), ("README", "docs")] {
                let mut file = tar::Header::new_gnu();
                file.set_mode(0o644);
                file.set_size(body.len() as u64);
                builder
                    .append_data(&mut file, path, body.as_bytes())
                    .unwrap();
            }
        })
    }

    fn compress(kind: ArchiveKind, tar: &[u8]) -> Vec<u8> {
        match kind {
            ArchiveKind::TarGz => {
                let mut encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(tar).unwrap();
                encoder.finish().unwrap()
            }
            ArchiveKind::TarBz2 => {
                let mut encoder =
                    bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
                encoder.write_all(tar).unwrap();
                encoder.finish().unwrap()
            }
            _ => tar.to_vec(),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn extracts_tar_archives_and_marks_nested_files_executable() {
        use std::os::unix::fs::PermissionsExt;
        for (kind, url) in [
            (ArchiveKind::Tar, "https://x/a.tar"),
            (ArchiveKind::TarGz, "https://x/a.tar.gz?download=1"),
            (ArchiveKind::TarGz, "https://x/a.TGZ"),
            (ArchiveKind::TarBz2, "https://x/a.tar.bz2"),
            (ArchiveKind::TarBz2, "https://x/a.tbz2"),
        ] {
            assert_eq!(ArchiveKind::from_url(url), kind, "{url}");
            let dest = tempfile::tempdir().unwrap();
            let archive = dest.path().join(kind.download_name());
            std::fs::write(&archive, compress(kind, &agent_tar())).unwrap();
            extract_tar(kind, &archive, dest.path()).await.unwrap();
            mark_executables(dest.path()).unwrap();
            let agent = dest.path().join("bin/agent");
            assert_eq!(std::fs::read_to_string(&agent).unwrap(), "#!/bin/sh\n");
            let mode = std::fs::metadata(&agent).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o111,
                0o111,
                "{kind:?}: nested bin/agent is executable"
            );
        }
        assert_eq!(ArchiveKind::from_url("https://x/sigit"), ArchiveKind::Raw);
        assert_eq!(ArchiveKind::from_url("https://x/a.zip"), ArchiveKind::Zip);
    }

    #[tokio::test]
    async fn tar_entries_reject_traversal_links_and_special_files() {
        let regular = |name: &[u8]| {
            let name = name.to_vec();
            tar_bytes(move |builder| {
                let mut header = tar::Header::new_old();
                header.as_old_mut().name[..name.len()].copy_from_slice(&name);
                header.set_mode(0o644);
                header.set_size(4);
                header.set_entry_type(tar::EntryType::Regular);
                header.set_cksum();
                builder.append(&header, &b"evil"[..]).unwrap();
            })
        };
        let linked = |kind: tar::EntryType| {
            tar_bytes(move |builder| {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(kind);
                header.set_size(0);
                header.set_mode(0o644);
                if kind == tar::EntryType::Fifo {
                    builder
                        .append_data(&mut header, "pipe", std::io::empty())
                        .unwrap();
                } else {
                    builder
                        .append_link(&mut header, "link", "../outside")
                        .unwrap();
                }
            })
        };
        let cases = [
            ("traversal", regular(b"../evil"), "unsafe path"),
            (
                "nested traversal",
                regular(b"bin/../../evil"),
                "unsafe path",
            ),
            ("absolute", regular(b"/tmp/evil"), "unsafe path"),
            (
                "symlink",
                linked(tar::EntryType::Symlink),
                "link or special",
            ),
            ("hardlink", linked(tar::EntryType::Link), "link or special"),
            ("fifo", linked(tar::EntryType::Fifo), "link or special"),
        ];
        for (case, bytes, expected) in cases {
            let root = tempfile::tempdir().unwrap();
            let dest = root.path().join("dest");
            std::fs::create_dir_all(&dest).unwrap();
            let archive = dest.join("download.tar.gz");
            std::fs::write(&archive, compress(ArchiveKind::TarGz, &bytes)).unwrap();
            let error = extract_tar(ArchiveKind::TarGz, &archive, &dest)
                .await
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{case}: {error}");
            assert!(!root.path().join("evil").exists(), "{case}");
            assert!(
                !dest.join("link").exists() && !dest.join("pipe").exists(),
                "{case}"
            );
        }
    }

    #[test]
    fn registry_commands_resolve_inside_the_install() {
        assert_eq!(
            registry_entry("./bin/devin").unwrap(),
            Path::new("bin/devin")
        );
        assert_eq!(registry_entry("goose").unwrap(), Path::new("goose"));
        for bad in ["../x", "./../x", "bin/../../x", "/usr/bin/x", "./", ""] {
            assert!(registry_entry(bad).is_err(), "{bad}");
        }
    }

    /// a child process, since installs read the process-wide `ZERON_ADAPTERS_DIR`.
    #[test]
    fn registry_binaries_check_sha256_and_install_archives_or_raw_files() {
        if std::env::var_os("ZERON_TEST_REGISTRY_INSTALL").is_none() {
            let root = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "archive_install::tests::registry_binaries_check_sha256_and_install_archives_or_raw_files",
                    "--nocapture",
                ])
                .env("ZERON_ADAPTERS_DIR", root.path())
                .env("ZERON_TEST_REGISTRY_INSTALL", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let bytes = compress(ArchiveKind::TarGz, &agent_tar());
                let digest = format!("{:x}", Sha256::digest(&bytes));
                let url = serve(bytes).await.replace("archive.zip", "agent.tar.gz");
                let root = adapters_root().unwrap();
                let binary = |sha256| RegistryBinary {
                    name: "acp-registry/demo",
                    version: "1.0.0",
                    url: &url,
                    cmd: "./bin/agent",
                    sha256,
                };

                let wrong = "0".repeat(64);
                let refused = install_registry_binary(&binary(Some(&wrong)), "Demo")
                    .await
                    .unwrap_err();
                assert!(
                    refused.to_string().contains("SHA-256 mismatch"),
                    "{refused}"
                );
                assert!(!root.join("acp-registry/demo").exists());

                let upper = digest.to_ascii_uppercase();
                let entry = install_registry_binary(&binary(Some(&upper)), "Demo")
                    .await
                    .unwrap();
                assert_eq!(entry, root.join("acp-registry/demo/1.0.0/bin/agent"));
                assert_eq!(
                    installed_versions("acp-registry/demo"),
                    vec![("1.0.0".to_owned(), format!("sha256:{digest}"))]
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = std::fs::metadata(&entry).unwrap().permissions().mode();
                    assert_eq!(mode & 0o111, 0o111);
                }

                let raw_url = serve(b"#!/bin/sh\necho raw\n".to_vec())
                    .await
                    .replace("/archive.zip", "/downloads/sigit-macos-arm64");
                let raw = RegistryBinary {
                    name: "acp-registry/raw",
                    version: "2.0.0",
                    url: &raw_url,
                    cmd: "./sigit",
                    sha256: None,
                };
                let entry = install_registry_binary(&raw, "Raw").await.unwrap();
                assert_eq!(entry, root.join("acp-registry/raw/2.0.0/sigit"));
                assert_eq!(
                    std::fs::read_to_string(&entry).unwrap(),
                    "#!/bin/sh\necho raw\n"
                );
                assert_eq!(
                    installed_versions("acp-registry/raw"),
                    vec![("2.0.0".to_owned(), format!("unverified:{raw_url}"))]
                );

                let escaping = RegistryBinary {
                    cmd: "../outside",
                    ..binary(None)
                };
                assert!(install_registry_binary(&escaping, "Demo").await.is_err());
            });
    }

    #[test]
    fn entry_resolves_only_after_a_completed_install() {
        let pin = ArchivePin {
            name: "never-installed-acp",
            version: "0.0.0-test",
            url: "https://example.invalid/x.zip",
            entry: "server",
            sha512: "test-digest",
        };
        assert!(installed_entry(&pin).is_none());
        if std::env::var_os("HOME").is_some() || std::env::var_os("ZERON_ADAPTERS_DIR").is_some() {
            let expected = install_dir(pin.name, pin.version).unwrap().join(pin.entry);
            assert!(expected.ends_with("never-installed-acp/0.0.0-test/server"));
        }
    }
}
