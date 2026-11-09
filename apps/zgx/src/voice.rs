//! Voice: Settings → Voice's speech model and microphones, and dictation
//! (ui/ds/dictation.luau, ui/screens/settings/voice.luau), as gpui's
//! crates/ui/src/dictation has them. Only words cross to the scripts: audio
//! stays in the host and never reaches the engine.
//!
//!   voice.model               { ready, cache, size, downloading, progress,
//!                             capable, error? }, also on the shared
//!                             `voice_model` as it changes (a download's
//!                             progress); `capable` false: this build has no
//!                             recorder, so no mic shows and nothing downloads
//!   voice.download            (invoke) the model fetched and checked
//!                             (crates/voice/model.json, the same folder as
//!                             gpui's, so one download serves both);
//!                             `unavailable` without a recorder
//!   voice.cancel              (invoke) that stopped
//!   voice.remove              the model's folder gone: the model's state;
//!                             `busy` while dictating, `remove_failed`
//!   voice.inputs              { devices = { { id, name } }, default? }
//!   voice.remote({ ids })     { [id] = true } for each of your devices that
//!                             can stream voice to Codex there
//!   dictation.start({ input? }) a session's number; how it goes on the shared
//!                             `dictation_events`: { n, seq, kind, text? },
//!                             kind listening, final, unavailable, failed
//!   dictation.finish(n) / dictation.cancel(n)   (invoke)
//!
//! Recording and transcribing are the Mac's (crates/voice: cpal and
//! Parakeet), not in this build yet: a session here says `unavailable`.
//! `ZERON_ZGX_MOCK=1` (a mock Dev): two made-up microphones, a download
//! that counts up without fetching, and a session that hears a fixed line.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Shared, Value};

use crate::auth::text;
use crate::engine;

/// What a mock session hears.
pub(crate) const MOCK_TRANSCRIPT: &str = "Dictated on a mock Dev, from a made-up microphone.";
const MOCK_LISTENING: Duration = Duration::from_millis(150);
const MOCK_TRANSCRIBING: Duration = Duration::from_millis(300);
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
const MODEL_FOLDER: &str = "models/parakeet-tdt-0.6b-v3-int8";

fn mock() -> bool {
    std::env::var("ZERON_ZGX_MOCK").is_ok_and(|v| v == "1")
}

/// Whether this build records and transcribes (crates/voice wired behind
/// `dictation.start`): not yet, so only a mock Dev's made-up microphone.
const RECORDER: bool = false;

fn capable() -> bool {
    RECORDER || mock()
}

#[derive(serde::Deserialize, Clone)]
pub(crate) struct Artifact {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(serde::Deserialize, Clone)]
pub(crate) struct Manifest {
    pub repository: String,
    pub revision: String,
    pub files: Vec<Artifact>,
}

/// The pinned model gpui downloads (crates/voice/model.json).
pub(crate) fn manifest() -> Manifest {
    serde_json::from_str(include_str!("../../../crates/voice/model.json")).expect("pinned model manifest")
}

impl Manifest {
    fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// Every file whole and the folder marked checked (zeron_voice::installed).
    pub(crate) fn installed(&self, dir: &Path) -> bool {
        self.files.iter().all(|f| std::fs::metadata(dir.join(&f.name)).is_ok_and(|m| m.len() == f.size))
            && std::fs::read_to_string(dir.join("verified")).is_ok_and(|r| r == self.revision)
    }
}

/// Each file fetched from `base`, its size and hash checked, then the folder
/// marked; staging files never count. `cancelled` or `download_failed`.
pub(crate) fn download(
    base: &str,
    m: &Manifest,
    dir: &Path,
    cancel: &AtomicBool,
    progress: &AtomicU64,
) -> Result<(), &'static str> {
    std::fs::create_dir_all(dir).map_err(|_| "download_failed")?;
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout_read(Duration::from_secs(30)).build();
    let mut total = 0;
    for file in &m.files {
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled");
        }
        let staging = dir.join(format!("{}.part", file.name));
        let fetched = (|| -> Result<(), &'static str> {
            let url = format!("{base}/{}/resolve/{}/{}", m.repository, m.revision, file.name);
            let response = agent.get(&url).call().map_err(|_| "download_failed")?;
            let mut body = response.into_reader();
            let mut out = std::fs::File::create(&staging).map_err(|_| "download_failed")?;
            let mut hash = Sha256::new();
            let mut size = 0u64;
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Err("cancelled");
                }
                let n = body.read(&mut buf).map_err(|_| "download_failed")?;
                if n == 0 {
                    break;
                }
                size += n as u64;
                if size > file.size {
                    return Err("download_failed");
                }
                hash.update(&buf[..n]);
                out.write_all(&buf[..n]).map_err(|_| "download_failed")?;
                total += n as u64;
                progress.store(total, Ordering::Relaxed);
            }
            if size != file.size || format!("{:x}", hash.finalize()) != file.sha256 {
                return Err("download_failed");
            }
            out.sync_all().map_err(|_| "download_failed")?;
            std::fs::rename(&staging, dir.join(&file.name)).map_err(|_| "download_failed")
        })();
        if let Err(word) = fetched {
            let _ = std::fs::remove_file(&staging);
            return Err(word);
        }
    }
    if cancel.load(Ordering::Acquire) {
        return Err("cancelled");
    }
    std::fs::write(dir.join("verified"), &m.revision).map_err(|_| "download_failed")
}

/// The model's folder gone, a partial one too.
pub(crate) fn remove(dir: &Path) -> Result<(), &'static str> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("remove_failed"),
    }
}

#[derive(Default)]
struct Voice {
    windows: RefCell<Vec<(u64, Shared, Shared)>>,
    next_window: Cell<u64>,
    downloading: RefCell<Option<Arc<AtomicBool>>>,
    progress: Arc<AtomicU64>,
    error: Cell<Option<&'static str>>,
    mock_ready: Cell<bool>,
    next_session: Cell<u64>,
    seq: Cell<u64>,
    sessions: RefCell<HashMap<u64, Session>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Session {
    Listening,
    Finishing,
}

thread_local! {
    static VOICE: Rc<Voice> = Rc::default();
}

fn dir() -> Option<PathBuf> {
    engine::data_dir().map(|d| d.join(MODEL_FOLDER))
}

impl Voice {
    fn state(&self) -> Value {
        let m = manifest();
        let downloading = self.downloading.borrow().is_some();
        let (ready, cache) = if mock() {
            (self.mock_ready.get(), self.mock_ready.get())
        } else {
            match dir() {
                Some(d) => (m.installed(&d), d.exists()),
                None => (false, false),
            }
        };
        let mut fields = vec![
            ("ready", Value::Bool(ready)),
            ("cache", Value::Bool(cache)),
            ("size", Value::from(m.size() as f64)),
            ("downloading", Value::Bool(downloading)),
            ("progress", Value::from(if downloading { self.progress.load(Ordering::Relaxed) as f64 } else { 0.0 })),
            ("capable", Value::Bool(capable())),
        ];
        if let Some(word) = self.error.get() {
            fields.push(("error", Value::from(word)));
        }
        Value::record(fields)
    }

    fn publish(&self) {
        let state = self.state();
        for (_, model, _) in self.windows.borrow().iter() {
            model.set(state.clone());
        }
    }

    fn event(&self, n: u64, kind: &str, text: Option<&str>) {
        self.seq.set(self.seq.get() + 1);
        let mut fields = vec![("n", Value::from(n as f64)), ("seq", Value::from(self.seq.get() as f64)), ("kind", Value::from(kind))];
        if let Some(text) = text {
            fields.push(("text", Value::from(text)));
        }
        let event = Value::record(fields);
        for (_, _, events) in self.windows.borrow().iter() {
            events.set(event.clone());
        }
    }

    fn start_download(self: &Rc<Self>) {
        if self.downloading.borrow().is_some() {
            return;
        }
        // (670 MB for a mic that could never record.)
        if !capable() {
            self.error.set(Some("unavailable"));
            self.publish();
            return;
        }
        self.error.set(None);
        self.progress.store(0, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        *self.downloading.borrow_mut() = Some(cancel.clone());
        self.publish();
        let (done, finished) = std::sync::mpsc::channel::<Result<(), &'static str>>();
        let progress = self.progress.clone();
        let m = manifest();
        if mock() {
            let size = m.size();
            std::thread::spawn(move || {
                for step in 1..=10u64 {
                    std::thread::sleep(PROGRESS_EVERY);
                    if cancel.load(Ordering::Acquire) {
                        let _ = done.send(Err("cancelled"));
                        return;
                    }
                    progress.store(size * step / 10, Ordering::Relaxed);
                }
                let _ = done.send(Ok(()));
            });
        } else {
            let Some(target) = dir() else {
                *self.downloading.borrow_mut() = None;
                self.error.set(Some("download_failed"));
                self.publish();
                return;
            };
            std::thread::spawn(move || {
                let _ = done.send(download("https://huggingface.co", &m, &target, &cancel, &progress));
            });
        }
        let me = Rc::downgrade(self);
        engine::get().spawn(async move {
            loop {
                zgx::timer::sleep(PROGRESS_EVERY).await;
                let Some(voice) = me.upgrade() else { return };
                if let Ok(result) = finished.try_recv() {
                    *voice.downloading.borrow_mut() = None;
                    match result {
                        Ok(()) => voice.mock_ready.set(mock()),
                        Err(word) => voice.error.set(Some(word)),
                    }
                    voice.publish();
                    return;
                }
                voice.publish();
            }
        });
    }

    fn start(self: &Rc<Self>) -> u64 {
        let n = self.next_session.get() + 1;
        self.next_session.set(n);
        if !mock() {
            let me = Rc::downgrade(self);
            engine::get().spawn(async move {
                zgx::timer::sleep(Duration::from_millis(1)).await;
                if let Some(voice) = me.upgrade() {
                    voice.event(n, "unavailable", None);
                }
            });
            return n;
        }
        self.sessions.borrow_mut().insert(n, Session::Listening);
        let me = Rc::downgrade(self);
        engine::get().spawn(async move {
            zgx::timer::sleep(MOCK_LISTENING).await;
            if let Some(voice) = me.upgrade()
                && voice.sessions.borrow().contains_key(&n)
            {
                voice.event(n, "listening", None);
            }
        });
        n
    }

    fn finish(self: &Rc<Self>, n: u64) {
        if self.sessions.borrow_mut().get_mut(&n).map(|s| std::mem::replace(s, Session::Finishing)) != Some(Session::Listening) {
            return;
        }
        let me = Rc::downgrade(self);
        engine::get().spawn(async move {
            zgx::timer::sleep(MOCK_TRANSCRIBING).await;
            if let Some(voice) = me.upgrade()
                && voice.sessions.borrow_mut().remove(&n).is_some()
            {
                voice.event(n, "final", Some(MOCK_TRANSCRIPT));
            }
        });
    }

    fn busy(&self) -> bool {
        !self.sessions.borrow().is_empty()
    }
}

/// Microphones: the mock's two; none known without the Mac's recorder.
pub(crate) fn inputs(mock: bool) -> Value {
    if !mock {
        return Value::record([("devices", Value::List(Vec::new().into()))]);
    }
    let device = |id: &str, name: &str| Value::record([("id", Value::from(id)), ("name", Value::from(name))]);
    Value::record([
        ("devices", Value::List(vec![device("mock:built-in", "Mock Microphone"), device("mock:usb", "USB Microphone")].into())),
        ("default", Value::from("mock:built-in")),
    ])
}

pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let v = VOICE.with(Rc::clone);
    let window = v.next_window.get();
    v.next_window.set(window + 1);
    let model = host.shared("voice_model", v.state());
    let events = host.shared("dictation_events", Value::Nil);
    v.windows.borrow_mut().push((window, model, events));
    let me = Rc::downgrade(&v);
    cx.on_closed(move || {
        if let Some(v) = me.upgrade() {
            v.windows.borrow_mut().retain(|(w, ..)| *w != window);
        }
    });
    host.async_action("voice.model", |_| async { Ok(VOICE.with(|v| v.state())) });
    host.action("voice.download", |_| VOICE.with(|v| v.start_download()));
    host.action("voice.cancel", |_| {
        VOICE.with(|v| {
            if let Some(cancel) = v.downloading.borrow().as_ref() {
                cancel.store(true, Ordering::Release);
            }
        })
    });
    host.async_action("voice.remove", |_| async {
        let v = VOICE.with(Rc::clone);
        if v.busy() {
            return Err("busy".into());
        }
        if mock() {
            v.mock_ready.set(false);
        } else if let Some(d) = dir() {
            remove(&d).map_err(String::from)?;
        }
        v.error.set(None);
        v.publish();
        Ok(v.state())
    });
    host.async_action("voice.inputs", |_| async { Ok(inputs(mock())) });
    host.async_action("voice.remote", |args| {
        let ids: Vec<String> = match args.field("ids") {
            Some(Value::List(list)) => list.iter().map(text).collect(),
            _ => Vec::new(),
        };
        async move {
            let capable = ids
                .iter()
                .filter(|id| crate::sessions::device_supports(id, zeron_proto::voice::remote::CAPABILITY))
                .map(|id| (id.as_str(), Value::Bool(true)))
                .collect::<Vec<_>>();
            Ok(Value::record(capable))
        }
    });
    host.async_action("dictation.start", |_| async {
        let v = VOICE.with(Rc::clone);
        Ok(Value::from(v.start() as f64))
    });
    host.action("dictation.finish", |n| {
        let n = n.as_f64().unwrap_or(0.0) as u64;
        VOICE.with(|v| v.finish(n));
    });
    host.action("dictation.cancel", |n| {
        let n = n.as_f64().unwrap_or(0.0) as u64;
        VOICE.with(|v| {
            v.sessions.borrow_mut().remove(&n);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// A server answering each GET with `body(path)`, for `requests` requests.
    fn serve(requests: usize, body: impl Fn(&str) -> Vec<u8> + Send + 'static) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().take(requests) {
                let mut stream = stream.unwrap();
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let bytes = body(&path);
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
                let _ = stream.write_all(&bytes);
            }
        });
        base
    }

    #[test]
    fn the_pinned_model_is_gpui_s_and_installed_only_when_whole_and_checked() {
        let m = manifest();
        assert_eq!(m.files.len(), 3);
        assert!(m.size() > 600_000_000, "the 670 MB Parakeet v3");
        let dir = std::env::temp_dir().join(format!("zeron-voice-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let small = Manifest {
            repository: "r".into(),
            revision: "rev1".into(),
            files: vec![Artifact { name: "a.onnx".into(), size: 3, sha256: sha(b"abc") }],
        };
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.onnx.part"), b"ab").unwrap();
        assert!(!small.installed(&dir), "a partial download is not the model");
        std::fs::write(dir.join("a.onnx"), b"abc").unwrap();
        assert!(!small.installed(&dir), "not checked yet");
        std::fs::write(dir.join("verified"), "rev1").unwrap();
        assert!(small.installed(&dir));
        assert_eq!(remove(&dir), Ok(()));
        assert!(!dir.exists());
        assert_eq!(remove(&dir), Ok(()), "gone already is fine");
    }

    #[test]
    fn a_download_checks_each_file_and_marks_the_folder_only_when_all_match() {
        let files: [(&str, &[u8]); 2] = [("enc.onnx", b"encoder bytes"), ("vocab.txt", b"a\nb\n")];
        let m = Manifest {
            repository: "org/model".into(),
            revision: "abc123".into(),
            files: files.iter().map(|(n, b)| Artifact { name: (*n).into(), size: b.len() as u64, sha256: sha(b) }).collect(),
        };
        let dir = std::env::temp_dir().join(format!("zeron-voice-dl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let base = serve(2, move |path| {
            let name = path.rsplit('/').next().unwrap_or_default().to_string();
            assert!(path.starts_with("/org/model/resolve/abc123/"), "{path}");
            files.iter().find(|(n, _)| *n == name).map(|(_, b)| b.to_vec()).unwrap_or_default()
        });
        let progress = AtomicU64::new(0);
        assert_eq!(download(&base, &m, &dir, &AtomicBool::new(false), &progress), Ok(()));
        assert!(m.installed(&dir));
        assert_eq!(progress.load(Ordering::Relaxed), 17);
        let _ = std::fs::remove_dir_all(&dir);

        let bad = serve(1, |_| b"tampered!!!!!".to_vec());
        assert_eq!(download(&bad, &m, &dir, &AtomicBool::new(false), &AtomicU64::new(0)), Err("download_failed"));
        assert!(!m.installed(&dir));
        assert!(!dir.join("enc.onnx").exists() && !dir.join("enc.onnx.part").exists(), "a bad file is not kept");
        assert_eq!(download(&bad, &m, &dir, &AtomicBool::new(true), &AtomicU64::new(0)), Err("cancelled"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_build_without_a_recorder_says_so_and_downloads_nothing() {
        if mock() {
            return;
        }
        let v = Rc::new(Voice::default());
        assert_eq!(v.state().field("capable"), Some(&Value::Bool(false)));
        v.start_download();
        assert!(v.downloading.borrow().is_none(), "nothing fetched");
        assert_eq!(v.state().field("error"), Some(&Value::from("unavailable")));
    }

    #[test]
    fn microphones_are_the_mock_s_two_and_none_without_a_recorder() {
        let mock = crate::plugins::value_json(&inputs(true));
        assert_eq!(mock["devices"].as_array().map(Vec::len), Some(2));
        assert_eq!(mock["default"], "mock:built-in");
        let none = crate::plugins::value_json(&inputs(false));
        assert_eq!(none["devices"], serde_json::json!([]));
    }
}
