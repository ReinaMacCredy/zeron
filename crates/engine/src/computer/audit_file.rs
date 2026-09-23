use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerAuditFileTail {
    pub contents: Vec<u8>,
    pub start: u64,
    pub total_bytes: u64,
    pub requested_bytes: u64,
    pub device: u64,
    pub inode: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Could not read Computer activity history.")]
pub struct ComputerAuditReadError;

#[cfg(unix)]
fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

#[cfg(unix)]
fn identity(metadata: &Metadata, _path: &Path) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino())
}

#[cfg(not(unix))]
fn identity(_metadata: &Metadata, path: &Path) -> (u64, u64) {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    (0, hasher.finish())
}

#[cfg(unix)]
fn read_at(file: &File, buffer: &mut [u8], position: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buffer, position)
}

#[cfg(windows)]
fn read_at(file: &File, buffer: &mut [u8], position: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buffer, position)
}

pub fn read_computer_audit_file_tail(
    path: &Path,
    max_bytes: u64,
) -> Result<Option<ComputerAuditFileTail>, ComputerAuditReadError> {
    let entry = match std::fs::symlink_metadata(path) {
        Ok(entry) => entry,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ComputerAuditReadError),
    };
    if !entry.is_file() {
        return Err(ComputerAuditReadError);
    }
    let file = match open_no_follow(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ComputerAuditReadError),
    };
    let stat = file.metadata().map_err(|_| ComputerAuditReadError)?;
    let entry = std::fs::symlink_metadata(path).map_err(|_| ComputerAuditReadError)?;
    let (device, inode) = identity(&stat, path);
    if !stat.is_file() || !entry.is_file() || identity(&entry, path) != (device, inode) {
        return Err(ComputerAuditReadError);
    }
    let length = stat.len().min(max_bytes);
    let start = stat.len() - length;
    let mut contents = vec![0; length as usize];
    let mut bytes_read = 0;
    while bytes_read < contents.len() {
        let read = read_at(
            &file,
            &mut contents[bytes_read..],
            start + bytes_read as u64,
        )
        .map_err(|_| ComputerAuditReadError)?;
        if read == 0 {
            break;
        }
        bytes_read += read;
    }
    contents.truncate(bytes_read);
    Ok(Some(ComputerAuditFileTail {
        contents,
        start,
        total_bytes: stat.len(),
        requested_bytes: length,
        device,
        inode,
    }))
}

pub fn computer_audit_tail_lines(
    tail: &ComputerAuditFileTail,
) -> impl Iterator<Item = (&[u8], u64)> + '_ {
    let contents = tail.contents.as_slice();
    let mut end = contents.iter().rposition(|byte| *byte == b'\n');
    std::iter::from_fn(move || {
        let current = end?;
        let previous = contents[..current].iter().rposition(|byte| *byte == b'\n');
        let line_start = previous.map_or(0, |previous| previous + 1);
        if previous.is_none() && tail.start > 0 {
            end = None;
            return None;
        }
        end = previous;
        Some((
            &contents[line_start..=current],
            tail.start + line_start as u64,
        ))
    })
}

pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

fn private_file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
}

pub(crate) fn append_private(path: &Path, content: &[u8]) -> io::Result<()> {
    private_file_options()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(content)
}

pub(crate) fn replace_private(path: &Path, content: &[u8]) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    private_file_options()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temporary)?
        .write_all(content)?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tail(contents: &[u8], start: u64) -> ComputerAuditFileTail {
        ComputerAuditFileTail {
            contents: contents.to_vec(),
            start,
            total_bytes: start + contents.len() as u64,
            requested_bytes: contents.len() as u64,
            device: 0,
            inode: 0,
        }
    }

    #[test]
    fn yields_complete_lines_newest_first_with_their_file_offsets() {
        let whole = tail(b"a\nbb\npartial", 0);
        let lines: Vec<(&[u8], u64)> = computer_audit_tail_lines(&whole).collect();
        assert_eq!(lines, [(&b"bb\n"[..], 2), (&b"a\n"[..], 0)]);
    }

    #[test]
    fn drops_the_cut_first_line_of_a_tail_that_starts_mid_file() {
        let cut = tail(b"xx\nyy\n", 10);
        let lines: Vec<(&[u8], u64)> = computer_audit_tail_lines(&cut).collect();
        assert_eq!(lines, [(&b"yy\n"[..], 13)]);
    }

    #[test]
    fn reads_only_the_requested_tail_and_reports_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        assert_eq!(read_computer_audit_file_tail(&path, 10), Ok(None));
        std::fs::write(&path, b"0123456789").unwrap();
        let read = read_computer_audit_file_tail(&path, 4).unwrap().unwrap();
        assert_eq!(read.contents, b"6789");
        assert_eq!(
            (read.start, read.total_bytes, read.requested_bytes),
            (6, 10, 4)
        );
    }

    #[cfg(unix)]
    #[test]
    fn writes_private_files_and_directories() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("state");
        create_private_dir(&nested).unwrap();
        let path = nested.join("file");
        replace_private(&path, b"one").unwrap();
        append_private(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"onetwo");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&nested), 0o700);
        assert_eq!(mode(&path), 0o600);
    }
}
