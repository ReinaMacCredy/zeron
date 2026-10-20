use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::App;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read as _, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;
use zeron_proto::ui_driver::{
    UiDriverBuild, UiDriverDescriptor, descriptor_path, run_dir, socket_path,
};

const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
const HANGUP_POLL: Duration = Duration::from_millis(50);

struct Job {
    request: Value,
    reply: std::sync::mpsc::Sender<Value>,
    cancel: Arc<AtomicBool>,
}

pub(crate) fn start(data_dir: PathBuf, ipc_port: u16, cx: &mut App) {
    super::epoch();
    let (dir, pid) = (run_dir(), std::process::id());
    let (socket, descriptor) = (socket_path(&dir, pid), descriptor_path(&dir, pid));
    let listener = match listen(&dir, pid, data_dir, ipc_port) {
        Ok(listener) => listener,
        Err(error) => {
            tracing::warn!(%error, socket = %socket.display(), "ui driver disabled");
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
    let accept = std::thread::Builder::new()
        .name("zeron-ui-driver".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                };
                let jobs = jobs.clone();
                let _ = std::thread::Builder::new()
                    .name("zeron-ui-driver-conn".into())
                    .spawn(move || serve(stream, jobs));
            }
        });
    if let Err(error) = accept {
        tracing::warn!(%error, "ui driver accept thread failed to start");
        remove(&socket, &descriptor);
        return;
    }
    cx.on_app_quit(move |_| {
        remove(&socket, &descriptor);
        async {}
    })
    .detach();
    tracing::info!(socket = %socket_path(&dir, pid).display(), "ui driver listening");
}

fn listen(dir: &Path, pid: u32, data_dir: PathBuf, ipc_port: u16) -> std::io::Result<UnixListener> {
    let (socket, descriptor) = (socket_path(dir, pid), descriptor_path(dir, pid));
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    match fs::remove_file(&socket) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    let bundle_path = std::env::current_exe().ok().and_then(|exe| {
        exe.ancestors()
            .find(|dir| dir.extension().is_some_and(|ext| ext == "app"))
            .map(Path::to_path_buf)
    });
    let record = UiDriverDescriptor {
        pid,
        socket,
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
    let tmp = descriptor.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&record)?)?;
    fs::rename(&tmp, &descriptor)?;
    Ok(listener)
}

fn remove(socket: &Path, descriptor: &Path) {
    let _ = fs::remove_file(descriptor);
    let _ = fs::remove_file(socket);
}

fn serve(stream: UnixStream, jobs: UnboundedSender<Job>) {
    let Ok(reader) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader);
    let mut writer = stream;
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
                        Err(RecvTimeoutError::Timeout) if hung_up(&writer) => {
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

fn hung_up(stream: &UnixStream) -> bool {
    use std::os::fd::AsRawFd as _;
    let mut byte = 0u8;
    // SAFETY: a one-byte peek into a live local buffer on a socket this function borrows.
    let read = unsafe {
        libc::recv(
            stream.as_raw_fd(),
            (&raw mut byte).cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    read == 0
        || (read < 0
            && !matches!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::FileTypeExt;

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
        assert_eq!(record.socket, socket);
        assert_eq!(record.ipc_port, 49880);
        assert_eq!(record.data_dir, Path::new("/data"));
        assert!(!descriptor.with_extension("json.tmp").exists());
    }

    #[test]
    fn serve_answers_line_by_line_and_flags_a_line_that_is_not_json() {
        let (client, server) = UnixStream::pair().unwrap();
        let (jobs, mut queue) = unbounded::<Job>();
        let served = std::thread::spawn(move || serve(server, jobs));
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

    #[test]
    fn a_client_that_hangs_up_mid_request_cancels_its_job() {
        let (client, server) = UnixStream::pair().unwrap();
        let (jobs, mut queue) = unbounded::<Job>();
        let served = std::thread::spawn(move || serve(server, jobs));
        writeln!(&client, "{{\"id\":1,\"action\":\"click\",\"x\":1,\"y\":2}}").unwrap();
        let job = futures::executor::block_on(queue.next()).unwrap();
        assert!(!job.cancel.load(Ordering::SeqCst));
        drop(client);
        served.join().unwrap();
        assert!(job.cancel.load(Ordering::SeqCst));
    }
}
