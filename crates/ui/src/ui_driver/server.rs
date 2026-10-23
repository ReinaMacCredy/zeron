use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::App;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;
#[cfg(unix)]
use zeron_proto::ui_driver::socket_path;
use zeron_proto::ui_driver::{
    UiDriverBuild, UiDriverDescriptor, UiDriverEndpoint, descriptor_path, run_dir,
};

const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
const HANGUP_POLL: Duration = Duration::from_millis(50);
const MAX_TOKEN_BYTES: u64 = 256;

struct Job {
    request: Value,
    reply: std::sync::mpsc::Sender<Value>,
    cancel: Arc<AtomicBool>,
}

enum Listener {
    #[cfg(unix)]
    Unix(UnixListener),
    #[cfg_attr(unix, allow(dead_code))]
    Loopback(TcpListener, Arc<str>),
}

trait Conn: Read + Write + Send + Sized + 'static {
    fn try_clone(&self) -> std::io::Result<Self>;
    fn hung_up(&self) -> bool;
}

#[cfg(unix)]
impl Conn for UnixStream {
    fn try_clone(&self) -> std::io::Result<Self> {
        UnixStream::try_clone(self)
    }

    fn hung_up(&self) -> bool {
        use std::os::fd::AsRawFd as _;
        let mut byte = 0u8;
        // SAFETY: a one-byte peek into a live local buffer on a socket this function borrows.
        let read = unsafe {
            libc::recv(
                self.as_raw_fd(),
                (&raw mut byte).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        gone(if read < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(read as usize)
        })
    }
}

impl Conn for TcpStream {
    fn try_clone(&self) -> std::io::Result<Self> {
        TcpStream::try_clone(self)
    }

    fn hung_up(&self) -> bool {
        if self.set_nonblocking(true).is_err() {
            return true;
        }
        let peeked = self.peek(&mut [0u8]);
        let _ = self.set_nonblocking(false);
        gone(peeked)
    }
}

fn gone(peeked: std::io::Result<usize>) -> bool {
    match peeked {
        Ok(read) => read == 0,
        Err(error) => !matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        ),
    }
}

pub(crate) fn start(data_dir: PathBuf, ipc_port: u16, cx: &mut App) {
    super::epoch();
    let (dir, pid) = (run_dir(), std::process::id());
    let descriptor = descriptor_path(&dir, pid);
    let (listener, endpoint) = match listen(&dir, pid, data_dir, ipc_port) {
        Ok(listening) => listening,
        Err(error) => {
            tracing::warn!(%error, dir = %dir.display(), "ui driver disabled");
            return;
        }
    };
    let (jobs, mut queue) = unbounded::<Job>();
    cx.spawn(async move |cx| {
        while let Some(job) = queue.next().await {
            cx.spawn(async move |cx| {
                let reply = super::handle(job.request, job.cancel, cx).await;
                let _ = job.reply.send(reply);
            })
            .detach();
        }
    })
    .detach();
    let accepting = std::thread::Builder::new()
        .name("zeron-ui-driver".into())
        .spawn(move || match listener {
            #[cfg(unix)]
            Listener::Unix(listener) => accept(listener.incoming(), None, jobs),
            Listener::Loopback(listener, token) => accept(listener.incoming(), Some(token), jobs),
        });
    if let Err(error) = accepting {
        tracing::warn!(%error, "ui driver accept thread failed to start");
        remove(&endpoint, &descriptor);
        return;
    }
    tracing::info!(%endpoint, "ui driver listening");
    cx.on_app_quit(move |_| {
        remove(&endpoint, &descriptor);
        async {}
    })
    .detach();
}

fn accept<C: Conn>(
    incoming: impl Iterator<Item = std::io::Result<C>>,
    token: Option<Arc<str>>,
    jobs: UnboundedSender<Job>,
) {
    for stream in incoming {
        let Ok(stream) = stream else {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        let (jobs, token) = (jobs.clone(), token.clone());
        let _ = std::thread::Builder::new()
            .name("zeron-ui-driver-conn".into())
            .spawn(move || serve(stream, token.as_deref(), jobs));
    }
}

fn listen(
    dir: &Path,
    pid: u32,
    data_dir: PathBuf,
    ipc_port: u16,
) -> std::io::Result<(Listener, UiDriverEndpoint)> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    let (listener, endpoint) = {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        let socket = socket_path(dir, pid);
        match fs::remove_file(&socket) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        (Listener::Unix(listener), UiDriverEndpoint::Unix { socket })
    };
    #[cfg(not(unix))]
    let (listener, endpoint) = bind_loopback()?;
    let bundle_path = std::env::current_exe().ok().and_then(|exe| {
        exe.ancestors()
            .find(|dir| dir.extension().is_some_and(|ext| ext == "app"))
            .map(Path::to_path_buf)
    });
    let record = UiDriverDescriptor {
        pid,
        endpoint: endpoint.clone(),
        label: crate::window_title(),
        bundle_path,
        data_dir,
        ipc_port,
        build: if cfg!(debug_assertions) {
            UiDriverBuild::Debug
        } else {
            UiDriverBuild::Release
        },
        input: cfg!(debug_assertions),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        started_at: chrono::Utc::now(),
    };
    let descriptor = descriptor_path(dir, pid);
    let tmp = descriptor.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&record)?)?;
    fs::rename(&tmp, &descriptor)?;
    Ok((listener, endpoint))
}

#[cfg_attr(unix, allow(dead_code))]
fn bind_loopback() -> std::io::Result<(Listener, UiDriverEndpoint)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    Ok((
        Listener::Loopback(listener, token.as_str().into()),
        UiDriverEndpoint::Loopback { port, token },
    ))
}

fn remove(endpoint: &UiDriverEndpoint, descriptor: &Path) {
    let _ = fs::remove_file(descriptor);
    if let UiDriverEndpoint::Unix { socket } = endpoint {
        let _ = fs::remove_file(socket);
    }
}

fn serve<C: Conn>(stream: C, token: Option<&str>, jobs: UnboundedSender<Job>) {
    let Ok(reader) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader);
    let mut writer = stream;
    if let Some(token) = token {
        let mut line = String::new();
        let _ = (&mut reader).take(MAX_TOKEN_BYTES).read_line(&mut line);
        if !same(
            line.trim_end_matches(['\r', '\n']).as_bytes(),
            token.as_bytes(),
        ) {
            let error = "the UI driver refused this connection: its first line must be the token from the app's ui-<pid>.json";
            let _ = writeln!(writer, "{}", json!({ "id": null, "error": error }));
            return;
        }
    }
    loop {
        let mut line = String::new();
        match (&mut reader).take(MAX_REQUEST_BYTES).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if !line.ends_with('\n') && line.len() as u64 >= MAX_REQUEST_BYTES {
            let error = format!("request is longer than {MAX_REQUEST_BYTES} bytes");
            let _ = writeln!(writer, "{}", json!({ "id": null, "error": error }));
            return;
        }
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(request) => {
                let (reply, answer) = std::sync::mpsc::channel();
                let cancel = Arc::new(AtomicBool::new(false));
                let job = Job {
                    request,
                    reply,
                    cancel: cancel.clone(),
                };
                if jobs.unbounded_send(job).is_err() {
                    return;
                }
                loop {
                    match answer.recv_timeout(HANGUP_POLL) {
                        Ok(reply) => break reply,
                        Err(RecvTimeoutError::Timeout) if writer.hung_up() => {
                            cancel.store(true, Ordering::SeqCst);
                            return;
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
            Err(error) => json!({ "id": null, "error": format!("request is not JSON: {error}") }),
        };
        if writeln!(writer, "{reply}").is_err() {
            return;
        }
    }
}

fn same(sent: &[u8], token: &[u8]) -> bool {
    sent.len() == token.len()
        && sent
            .iter()
            .zip(token)
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::FileTypeExt;

    #[cfg(unix)]
    #[test]
    fn listen_locks_the_run_dir_replaces_a_stale_socket_and_renames_the_descriptor_in() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("run");
        let (socket, descriptor) = (dir.join("ui-1.sock"), dir.join("ui-1.json"));
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        drop(UnixListener::bind(&socket).unwrap());
        assert!(UnixStream::connect(&socket).is_err());

        let _listener = listen(&dir, 1, "/data".into(), 49880).unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&socket), 0o600);
        assert!(fs::metadata(&socket).unwrap().file_type().is_socket());
        UnixStream::connect(&socket).unwrap();
        let record: UiDriverDescriptor =
            serde_json::from_slice(&fs::read(&descriptor).unwrap()).unwrap();
        assert_eq!(record.pid, 1);
        assert_eq!(record.endpoint, UiDriverEndpoint::Unix { socket });
        assert_eq!(record.ipc_port, 49880);
        assert_eq!(record.data_dir, Path::new("/data"));
        assert!(!descriptor.with_extension("json.tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn serve_answers_line_by_line_and_flags_a_line_that_is_not_json() {
        let (client, server) = UnixStream::pair().unwrap();
        let (jobs, mut queue) = unbounded::<Job>();
        let served = std::thread::spawn(move || serve(server, None, jobs));
        let mut writer = client.try_clone().unwrap();
        let mut lines = BufReader::new(client).lines();
        let mut reply =
            || -> Value { serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap() };

        writeln!(writer, "not json\n\n{{\"id\":7,\"action\":\"state\"}}").unwrap();
        let first = reply();
        assert_eq!(first["id"], Value::Null);
        let error = first["error"].as_str().unwrap();
        assert!(error.starts_with("request is not JSON"), "{error}");
        let job = futures::executor::block_on(queue.next()).unwrap();
        assert_eq!(job.request, json!({ "id": 7, "action": "state" }));
        job.reply.send(json!({ "id": 7, "ok": "drawn" })).unwrap();
        assert_eq!(reply(), json!({ "id": 7, "ok": "drawn" }));

        drop(queue);
        writeln!(writer, "{{\"id\":8,\"action\":\"state\"}}").unwrap();
        served.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_client_that_hangs_up_mid_request_cancels_its_job() {
        let (client, server) = UnixStream::pair().unwrap();
        let (jobs, mut queue) = unbounded::<Job>();
        let served = std::thread::spawn(move || serve(server, None, jobs));
        writeln!(&client, "{{\"id\":1,\"action\":\"click\",\"x\":1,\"y\":2}}").unwrap();
        let job = futures::executor::block_on(queue.next()).unwrap();
        assert!(!job.cancel.load(Ordering::SeqCst));
        drop(client);
        served.join().unwrap();
        assert!(job.cancel.load(Ordering::SeqCst));
    }

    #[test]
    fn a_loopback_connection_opens_with_the_token_and_a_hangup_cancels_its_job() {
        let (
            Listener::Loopback(listener, token),
            UiDriverEndpoint::Loopback {
                port,
                token: written,
            },
        ) = bind_loopback().unwrap()
        else {
            unreachable!()
        };
        assert_eq!((&*token, written.len()), (written.as_str(), 32));
        let (jobs, mut queue) = unbounded::<Job>();
        std::thread::spawn(move || accept(listener.incoming(), Some(token), jobs));
        let connect = || TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        let line = |stream: &TcpStream| -> String {
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            line
        };

        let refused = connect();
        writeln!(
            &refused,
            "{}x\n{{\"id\":1,\"action\":\"state\"}}",
            &written[1..]
        )
        .unwrap();
        let error = line(&refused);
        assert!(error.contains("refused this connection"), "{error}");
        assert!(!error.contains(&written[1..]), "{error}");

        let client = connect();
        writeln!(&client, "{written}\n{{\"id\":2,\"action\":\"state\"}}").unwrap();
        let job = futures::executor::block_on(queue.next()).unwrap();
        assert_eq!(job.request, json!({ "id": 2, "action": "state" }));
        job.reply.send(json!({ "id": 2, "ok": "drawn" })).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line(&client)).unwrap(),
            json!({ "id": 2, "ok": "drawn" })
        );

        writeln!(&client, "{{\"id\":3,\"action\":\"click\",\"x\":1,\"y\":2}}").unwrap();
        let job = futures::executor::block_on(queue.next()).unwrap();
        assert!(!job.cancel.load(Ordering::SeqCst));
        drop(client);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !job.cancel.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(job.cancel.load(Ordering::SeqCst));
    }
}
