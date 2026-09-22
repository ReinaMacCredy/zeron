//! One `cua-driver serve --embedded` child and its private unix socket.
//!
//! The driver trusts a socket peer only when that peer's pid is its parent
//! (`serve.rs` `authenticate_embedded_host_connection`), so this process must
//! be the one that spawns it; the TCC grants it acts under are Zeron's.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const NATIVE_REVISION: u64 = 39;
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 96 * 1024 * 1024;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
const CURSOR_IDLE_HIDE_MS: &str = "60000";

/// The driver shipped inside the running bundle
/// (`Contents/Resources/cua-driver/cua-driver`).
pub fn bundled_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe
        .parent()?
        .parent()?
        .join("Resources")
        .join("cua-driver")
        .join("cua-driver");
    path.is_file().then_some(path)
}

// A unix socket path must fit sun_path (104 bytes on macOS); data dirs under a
// worktree already exceed it, so the socket lives in a short private dir.
fn socket_dir() -> io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let dir = std::env::temp_dir().join(format!("zcua-{}", std::process::id()));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => Ok(dir),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(dir),
        Err(err) => Err(err),
    }
}

pub struct DriverProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    socket: PathBuf,
}

impl DriverProcess {
    pub fn spawn(binary: &Path, runtime_dir: &Path, bundle_id: &str) -> io::Result<Self> {
        std::fs::create_dir_all(runtime_dir)?;
        let socket = socket_dir()?.join("driver.sock");
        let _ = std::fs::remove_file(&socket);
        let mut child = Command::new(binary)
            .args(["serve", "--embedded", "--socket"])
            .arg(&socket)
            .args(["--compact-cursor", "--idle-hide-ms", CURSOR_IDLE_HIDE_MS])
            .env("CUA_DRIVER_EMBEDDED", "1")
            .env("CUA_DRIVER_HOST_BUNDLE_ID", bundle_id)
            .env("CUA_DRIVER_PERMISSION_MODE", "standard")
            .env("CUA_DRIVER_RS_TELEMETRY_ENABLED", "0")
            .env("CUA_DRIVER_RS_UPDATE_CHECK", "0")
            .env("SYNARA_CUA_FOREGROUND_OBSERVATION_MS", "100")
            .env("SYNARA_CUA_BACKGROUND_OBSERVATION_MS", "350")
            .env("CUA_DRIVER_PARENT_LIVENESS_STDIN", "1")
            .env(
                "CUA_DRIVER_EMBEDDED_HOST_PID",
                std::process::id().to_string(),
            )
            .env("CUA_DRIVER_RS_HOME", runtime_dir.join("state"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take();
        let driver = Self {
            child,
            stdin,
            socket,
        };
        driver.wait_for_socket(Duration::from_secs(10))?;
        Ok(driver)
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    fn wait_for_socket(&self, limit: Duration) -> io::Result<()> {
        let start = Instant::now();
        while start.elapsed() < limit {
            if UnixStream::connect(&self.socket).is_ok() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "cua-driver socket never came up",
        ))
    }

    /// One request per connection, newline-delimited JSON, bounded both ways.
    pub fn request(&self, request: &Value, timeout: Duration) -> io::Result<Value> {
        let mut encoded = serde_json::to_vec(request)?;
        encoded.push(b'\n');
        if encoded.len() > MAX_REQUEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "computer request exceeds its byte budget",
            ));
        }
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        stream.write_all(&encoded)?;
        let mut line = Vec::new();
        BufReader::new(stream)
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if line.len() > MAX_RESPONSE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "computer response exceeded its byte budget",
            ));
        }
        serde_json::from_slice(&line).map_err(io::Error::from)
    }

    pub fn call(&self, tool: &str, args: Value) -> io::Result<Value> {
        self.request(
            &serde_json::json!({ "method": "call", "name": tool, "args": args }),
            DEFAULT_TIMEOUT,
        )
    }
}

impl Drop for DriverProcess {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                let _ = std::fs::remove_file(&self.socket);
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}
