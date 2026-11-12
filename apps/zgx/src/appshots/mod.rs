//! Appshots: a capture of the frontmost application's window, staged on the
//! composer as a picture, never sent on its own, as the shipping app's
//! (crates/ui/src/appshots.rs, its AppSnap flow). The hotkey is a
//! double-modifier chord (both ⌘ keys by default) watched by one listen-only
//! event tap, which also serves the Quick capture chord; the tap needs Input
//! Monitoring, the picture Screen Recording, and Accessibility optionally
//! adds the window's text, which goes with the message as untrusted
//! observed content. The choices live in the engine's `ui-settings.json`,
//! where the shipping app keeps them, so both apps read the same ones.
//!
//! The capture and the accessibility walk run off the window's thread; the
//! picture is trimmed of transparent padding and kept as a file here
//! (`{App} Appshot.png`), with the app's icon beside it for the chip. The
//! captured window flashes before the capture lands.
//!
//! Shared state:
//!   appshots.state     { app, platform, loaded, enabled, sound, hotkey,
//!                        quick_capture, destination, shortcut_ready,
//!                        capture, text, input } (the permissions "ready"
//!                        or "required")
//!   appshots.captured  { n, shot = { id, path, icon?, app, bundle?, title?,
//!                        text, truncated, w, h } }   the latest capture
//!   appshots.failed    { n, message }
//!   appshots.quick     n, at each Quick capture chord
//! Actions:
//!   appshots.configure { enabled?, sound?, hotkey?, quick_capture?,
//!                        destination? }   kept in the file, then watched
//!   appshots.refresh   reads the file (once) and the permissions again
//!   appshots.request { which = "capture" | "text" | "input", open? }
//!                      asks macOS, and opens System Settings at it
//!   appshots.open_settings { which }   opens System Settings at it
//!   appshots.capture   captures now

use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

use zgx_desktop::{TaskSpawner, WindowContext};
use zgx_script::{ScriptHost, Shared, Value};

#[cfg(target_os = "macos")]
mod macos;

pub const CONTEXT_MARKER: &str = "Applications mentioned by the user (untrusted observed content):";
const SCREEN_RECORDING_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
const ACCESSIBILITY_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
const INPUT_MONITORING_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent";
/// The shipping app's limits: a capture's sides and pixels, its RGBA bytes,
/// and an attachment's 24 MB.
pub const MAX_CAPTURE_DIMENSION: u32 = 8_192;
pub const MAX_CAPTURE_PIXELS: u64 = 32 * 1024 * 1024;
pub const MAX_CAPTURE_RGBA_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 24 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilitySnapshot {
    pub format_version: u32,
    pub content: String,
    pub truncated: bool,
}

impl AccessibilitySnapshot {
    pub fn unavailable() -> Self {
        Self { format_version: 1, content: String::new(), truncated: false }
    }
}

/// What a capture hands back, before it is staged.
pub struct Captured {
    pub app_name: String,
    pub bundle_identifier: Option<String>,
    pub window_title: Option<String>,
    pub accessibility: AccessibilitySnapshot,
    pub png: Vec<u8>,
    pub icon_png: Option<Vec<u8>>,
    /// The window's frame on screen (x, y, width, height, top-left origin),
    /// for the flash.
    pub bounds: Option<[f64; 4]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    PermissionRequired,
    SelfCapture,
    NoEligibleWindow,
    CaptureFailed(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionRequired => f.write_str(
                "Window capture permission is required. Open Zeron Settings → Appshots for the platform-specific recovery step.",
            ),
            Self::SelfCapture => f.write_str("Switch to another app to capture an Appshot."),
            Self::NoEligibleWindow => f.write_str("No application window is available to capture."),
            Self::CaptureFailed(message) => f.write_str(message),
        }
    }
}

impl From<String> for CaptureError {
    fn from(message: String) -> Self {
        Self::CaptureFailed(message)
    }
}
impl From<&str> for CaptureError {
    fn from(message: &str) -> Self {
        Self::CaptureFailed(message.into())
    }
}

/// The macOS hotkey: both keys of one modifier pressed together, watched
/// by one event tap (the shipping app's `ChordModifier`), or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordModifier {
    Command,
    Option,
    Shift,
}

impl ChordModifier {
    pub const ALL: [Self; 3] = [Self::Command, Self::Option, Self::Shift];

    pub fn combo(self) -> &'static str {
        match self {
            Self::Command => "command-command",
            Self::Option => "option-option",
            Self::Shift => "shift-shift",
        }
    }

    pub fn from_combo(combo: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|chord| chord.combo() == combo)
    }
}

pub const NO_HOTKEY: &str = "none";

pub fn validate_capture_dimensions(width: u32, height: u32) -> Result<usize, CaptureError> {
    if width == 0 || height == 0 || width > MAX_CAPTURE_DIMENSION || height > MAX_CAPTURE_DIMENSION {
        return Err(format!("The captured window dimensions ({width}×{height}) are not supported.").into());
    }
    let pixels = u64::from(width) * u64::from(height);
    let rgba = pixels * 4;
    if pixels > MAX_CAPTURE_PIXELS || rgba > MAX_CAPTURE_RGBA_BYTES {
        return Err(format!("The captured window ({width}×{height}) exceeds Zeron's capture budget.").into());
    }
    Ok(rgba as usize)
}

/// A PNG's size from its header, without decoding it.
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// An app's name as one safe file name part (the shipping app's).
pub fn safe_app_name(value: &str) -> String {
    let mut safe = String::new();
    for ch in value.chars().take(100) {
        safe.push(if ch.is_control() || matches!(ch, '/' | '\\' | ':') { '-' } else { ch });
    }
    let safe = safe.split_whitespace().collect::<Vec<_>>().join(" ");
    let safe = safe.trim_matches(['.', '-', ' ']);
    if safe.is_empty() { "Application".into() } else { safe.into() }
}

/// The picture without rows and columns that are wholly transparent (a
/// capture's backing surface can be wider than its window), as RGBA8.
/// None when there's nothing to trim, or it isn't an RGBA picture.
fn trim_padding(bytes: &[u8]) -> Result<Option<Vec<u8>>, CaptureError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("Could not decode the captured window: {e}"))?;
    let (color, _) = reader.output_color_type();
    if color != png::ColorType::Rgba {
        return Ok(None);
    }
    let (width, height) = (reader.info().width, reader.info().height);
    validate_capture_dimensions(width, height)?;
    let mut pixels = vec![0; reader.output_buffer_size().ok_or("The captured window is too large.")?];
    reader.next_frame(&mut pixels).map_err(|e| format!("Could not decode the captured window: {e}"))?;
    let visible = |x: u32, y: u32| pixels[(y as usize * width as usize + x as usize) * 4 + 3] != 0;
    let Some(top) = (0..height).find(|y| (0..width).any(|x| visible(x, *y))) else {
        return Ok(None);
    };
    let bottom = (top..height).rfind(|y| (0..width).any(|x| visible(x, *y))).unwrap() + 1;
    let left = (0..width).find(|x| (top..bottom).any(|y| visible(*x, y))).unwrap();
    let right = (left..width).rfind(|x| (top..bottom).any(|y| visible(*x, y))).unwrap() + 1;
    if (left, top, right, bottom) == (0, 0, width, height) {
        return Ok(None);
    }
    let (w, h) = (right - left, bottom - top);
    let row = w as usize * 4;
    let mut out = Vec::with_capacity(row * h as usize);
    for y in top..bottom {
        let start = (y as usize * width as usize + left as usize) * 4;
        out.extend_from_slice(&pixels[start..start + row]);
    }
    let mut png_out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&out))
            .map_err(|e| format!("Could not encode the captured window: {e}"))?;
    }
    Ok(Some(png_out))
}

/// A capture, staged: its picture trimmed and kept as `{App} Appshot.png` in
/// a folder of its own under `dir`, its app's icon beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Staged {
    pub id: String,
    pub path: PathBuf,
    pub icon: Option<PathBuf>,
    pub app: String,
    pub bundle: Option<String>,
    pub title: Option<String>,
    pub text: String,
    pub truncated: bool,
    pub size: (u32, u32),
    pub bounds: Option<[f64; 4]>,
}

pub fn stage(captured: Captured, dir: &std::path::Path) -> Result<Staged, CaptureError> {
    if captured.png.len() as u64 > MAX_IMAGE_BYTES {
        return Err("The captured window is larger than Zeron's 24 MB image limit.".into());
    }
    let png = trim_padding(&captured.png)?.unwrap_or(captured.png);
    let size = png_dimensions(&png).ok_or("The captured window is not a valid PNG image.")?;
    validate_capture_dimensions(size.0, size.1)?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let id = format!("appshot-{stamp}-{}-{n}", std::process::id());
    let folder = dir.join(&id);
    std::fs::create_dir_all(&folder).map_err(|e| format!("The Appshot couldn't be kept: {e}"))?;
    let path = folder.join(format!("{} Appshot.png", safe_app_name(&captured.app_name)));
    std::fs::write(&path, &png).map_err(|e| format!("The Appshot couldn't be kept: {e}"))?;
    let icon = captured.icon_png.and_then(|bytes| {
        let icon = folder.join("icon.png");
        std::fs::write(&icon, bytes).ok().map(|_| icon)
    });
    Ok(Staged {
        id,
        path,
        icon,
        app: captured.app_name,
        bundle: captured.bundle_identifier,
        title: captured.window_title,
        text: captured.accessibility.content,
        truncated: captured.accessibility.truncated,
        size,
        bounds: captured.bounds,
    })
}

/// Where Appshots are kept: the system's temporary folder.
fn appshot_dir() -> PathBuf {
    std::env::temp_dir().join("zeron-zgx-appshots")
}

pub fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\r' => out.push_str("&#13;"),
            '\n' => out.push_str("&#10;"),
            '\t' => out.push_str("&#9;"),
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => out.push('\u{fffd}'),
            _ => out.push(ch),
        }
    }
    out
}

/// What a message says of the apps it carries Appshots of (the shipping
/// app's `with_appshots`): the words, then the marker and one `<appshot>`
/// each, its picture by the path it was sent as.
pub struct Context {
    pub app: String,
    pub bundle: Option<String>,
    pub title: Option<String>,
    pub image: String,
    pub text: String,
    pub truncated: bool,
}

pub fn with_appshots(text: &str, shots: &[Context]) -> String {
    if shots.is_empty() {
        return text.to_string();
    }
    let mut out = format!("{text}\n\n{CONTEXT_MARKER}");
    for s in shots {
        out.push_str(&format!("\n<appshot app=\"{}\"", xml_escape(&s.app)));
        if let Some(bundle) = &s.bundle {
            out.push_str(&format!(" bundle-identifier=\"{}\"", xml_escape(bundle)));
        }
        if let Some(title) = &s.title {
            out.push_str(&format!(" window-title=\"{}\"", xml_escape(title)));
        }
        out.push_str(&format!(
            " image=\"{}\" accessibility-format=\"1\" truncated=\"{}\">\n{}\n</appshot>",
            xml_escape(&s.image),
            s.truncated,
            xml_escape(&s.text)
        ));
    }
    out
}

/// A sent message's words without its Appshots' context (the shipping
/// app's `strip_context_for_display`), and each Appshot it names: its
/// picture's path, app, window title and bundle.
pub fn split_appshots(words: &str) -> (String, Vec<Shown>) {
    let marker = format!("\n\n{CONTEXT_MARKER}");
    let (body, block) = match words.find(&marker) {
        Some(at) => (&words[..at], &words[at + marker.len()..]),
        None if words.starts_with(CONTEXT_MARKER) => ("", &words[CONTEXT_MARKER.len()..]),
        None => return (words.to_owned(), Vec::new()),
    };
    let mut shots: Vec<Shown> = Vec::new();
    for tag in block.split("<appshot ").skip(1) {
        let Some(end) = tag.find('>') else { continue };
        let attrs = &tag[..end];
        let get = |name: &str| -> Option<String> {
            let key = format!("{name}=\"");
            let from = attrs.find(&key)? + key.len();
            let to = attrs[from..].find('"')? + from;
            Some(xml_unescape(&attrs[from..to]))
        };
        let (Some(app), Some(image)) = (get("app"), get("image")) else { continue };
        // (One picture, one Appshot: a repeat is dropped, as the shipping app's.)
        if image.is_empty() || shots.iter().any(|s| s.image == image) {
            continue;
        }
        shots.push(Shown { image, app, title: get("window-title"), bundle: get("bundle-identifier") });
    }
    (body.to_owned(), shots)
}

/// An Appshot as a sent message shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub image: String,
    pub app: String,
    pub title: Option<String>,
    pub bundle: Option<String>,
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&#13;", "\r")
        .replace("&#10;", "\n")
        .replace("&#9;", "\t")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// A sent message's attachments, each picture that is an Appshot saying so
/// (`appshot = { app, title?, bundle? }`): its card names the app.
pub fn tag_attachments(list: Value, shots: &[Shown]) -> Value {
    if shots.is_empty() {
        return list;
    }
    let Value::List(items) = list else { return list };
    let tagged: Vec<Value> = items
        .iter()
        .map(|item| {
            let src = item.field("src").map(crate::auth::text).unwrap_or_default();
            let (Some(shot), Value::Map(fields)) = (shots.iter().find(|s| s.image == src), item) else {
                return item.clone();
            };
            let mut meta = vec![("app", Value::from(shot.app.as_str()))];
            if let Some(title) = &shot.title {
                meta.push(("title", Value::from(title.as_str())));
            }
            if let Some(bundle) = &shot.bundle {
                meta.push(("bundle", Value::from(bundle.as_str())));
            }
            let mut out: Vec<(&str, Value)> = fields.iter().map(|(k, v)| (k.as_ref(), v.clone())).collect();
            out.push(("appshot", Value::record(meta)));
            Value::record(out)
        })
        .collect();
    Value::List(tagged.into())
}

/// What captures: the platform's, or a test's.
pub type Capturer = Arc<dyn Fn() -> Result<Captured, CaptureError> + Send + Sync>;

fn platform_capturer() -> Capturer {
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::capture_frontmost_window)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(|| Err(CaptureError::CaptureFailed("Appshots are not available on this platform.".into())))
    }
}

/// Captures with `capturer` and stages it in `dir`, off the calling thread.
pub async fn capture_off_thread(capturer: Capturer, dir: PathBuf) -> Result<Staged, CaptureError> {
    let (tx, mut rx) = zgx::task::latest();
    std::thread::spawn(move || {
        tx.send(capturer().and_then(|c| stage(c, &dir)));
    });
    rx.recv().await.unwrap_or_else(|| Err("The capture stopped.".into()))
}

fn staged_value(s: &Staged) -> Value {
    let mut fields = vec![
        ("id", Value::from(s.id.as_str())),
        ("path", Value::from(s.path.to_string_lossy().as_ref())),
        ("app", Value::from(s.app.as_str())),
        ("text", Value::from(s.text.as_str())),
        ("truncated", Value::Bool(s.truncated)),
        ("bytes", Value::Num(std::fs::metadata(&s.path).map(|m| m.len() as f64).unwrap_or(0.0))),
        ("w", Value::Num(f64::from(s.size.0))),
        ("h", Value::Num(f64::from(s.size.1))),
    ];
    if let Some(icon) = &s.icon {
        fields.push(("icon", Value::from(icon.to_string_lossy().as_ref())));
    }
    if let Some(bundle) = &s.bundle {
        fields.push(("bundle", Value::from(bundle.as_str())));
    }
    if let Some(title) = &s.title {
        fields.push(("title", Value::from(title.as_str())));
    }
    Value::record(fields)
}

/// The choices, as the shipping app keeps them in its `ui-settings.json`
/// (crates/ui settings.rs), so both apps read the same ones.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub enabled: bool,
    pub sound: bool,
    pub hotkey: String,
    pub quick_capture: String,
    pub destination: String,
}

const DESTINATIONS: [&str; 3] = ["automatic", "last-session", "new-session"];

impl Default for Prefs {
    fn default() -> Self {
        Self {
            enabled: false,
            sound: true,
            hotkey: ChordModifier::Command.combo().into(),
            quick_capture: ChordModifier::Option.combo().into(),
            destination: "automatic".into(),
        }
    }
}

/// A hotkey as the shipping app reads it on the Mac: a chord or none; any
/// other (a key recorded before the chords) is the default.
fn hotkey_or(value: Option<&str>, default: &'static str) -> String {
    match value {
        Some(v) if v == NO_HOTKEY || ChordModifier::from_combo(v).is_some() => v.into(),
        _ => default.into(),
    }
}

impl Prefs {
    pub fn read(file: Option<&serde_json::Value>) -> Self {
        let d = Self::default();
        let Some(f) = file else { return d };
        let keymap = &f["keymap"];
        Self {
            enabled: f["appshotsEnabled"].as_bool().unwrap_or(d.enabled),
            // (A file from before the Appshot chime had one: it followed the chimes.)
            sound: f["appshotSoundEnabled"].as_bool().or_else(|| f["soundEnabled"].as_bool()).unwrap_or(d.sound),
            hotkey: hotkey_or(keymap["captureAppshot"].as_str(), ChordModifier::Command.combo()),
            // (gpui watches no chord for any other Quick capture value, and keeps it.)
            quick_capture: match keymap["quickCapture"].as_str() {
                None => ChordModifier::Option.combo().into(),
                Some(v) if ChordModifier::from_combo(v).is_some() => v.into(),
                Some(_) => NO_HOTKEY.into(),
            },
            destination: f["appshotDestination"]
                .as_str()
                .filter(|d| DESTINATIONS.contains(d))
                .unwrap_or("automatic")
                .into(),
        }
    }

    pub fn changes(&self, next: &Self, file: Option<&serde_json::Value>) -> serde_json::Map<String, serde_json::Value> {
        use serde_json::Value as Json;
        let mut out = serde_json::Map::new();
        if next.enabled != self.enabled {
            out.insert("appshotsEnabled".into(), Json::Bool(next.enabled));
        }
        if next.sound != self.sound {
            out.insert("appshotSoundEnabled".into(), Json::Bool(next.sound));
        }
        if next.destination != self.destination {
            out.insert("appshotDestination".into(), Json::from(next.destination.as_str()));
        }
        if next.hotkey != self.hotkey || next.quick_capture != self.quick_capture {
            let mut keymap = file.and_then(|f| f["keymap"].as_object().cloned()).unwrap_or_default();
            if next.hotkey != self.hotkey {
                keymap.insert("captureAppshot".into(), Json::from(next.hotkey.as_str()));
            }
            if next.quick_capture != self.quick_capture {
                keymap.insert("quickCapture".into(), Json::from(next.quick_capture.as_str()));
            }
            out.insert("keymap".into(), Json::Object(keymap));
        }
        out
    }

    fn appshot_chord(&self) -> Option<ChordModifier> {
        self.enabled.then(|| ChordModifier::from_combo(&self.hotkey)).flatten()
    }
}

struct Appshots {
    prefs: RefCell<Prefs>,
    loaded: std::cell::Cell<bool>,
    listening: std::cell::Cell<bool>,
    state: Shared,
    captured: Shared,
    failed: Shared,
    quick: Shared,
    capturing: std::cell::Cell<bool>,
    counter: std::cell::Cell<u64>,
    quick_counter: std::cell::Cell<u64>,
    tasks: TaskSpawner,
}

impl Appshots {
    fn load(&self) {
        if self.loaded.get() {
            return;
        }
        let Some(path) = crate::settings::ui_settings_path() else { return };
        let Ok(file) = crate::settings::read_settings_at(&path) else { return };
        *self.prefs.borrow_mut() = Prefs::read(file.as_ref());
        self.loaded.set(true);
    }

    fn save(&self, next: Prefs) {
        let Some(path) = crate::settings::ui_settings_path() else { return };
        let Ok(file) = crate::settings::read_settings_at(&path) else { return };
        let changes = self.prefs.borrow().changes(&next, file.as_ref());
        if !changes.is_empty() && crate::settings::write_settings_at(&path, changes).is_err() {
            return;
        }
        *self.prefs.borrow_mut() = next;
    }

    fn publish(&self) {
        self.load();
        let p = self.prefs.borrow().clone();
        #[cfg(target_os = "macos")]
        {
            self.listening.set(macos::refresh_chords(p.appshot_chord(), ChordModifier::from_combo(&p.quick_capture)));
        }
        #[cfg(target_os = "macos")]
        let (capture, text, input) = macos::capabilities();
        #[cfg(not(target_os = "macos"))]
        let (capture, text, input) = (false, false, false);
        #[cfg(target_os = "macos")]
        let app = macos::app_display_name();
        #[cfg(not(target_os = "macos"))]
        let app = String::from("Zeron");
        let said = |on: bool| Value::from(if on { "ready" } else { "required" });
        self.state.set(Value::record([
            ("app", Value::from(app.as_str())),
            ("platform", Value::from(if cfg!(target_os = "macos") { "macos" } else { "unsupported" })),
            ("loaded", Value::Bool(self.loaded.get())),
            ("enabled", Value::Bool(p.enabled)),
            ("sound", Value::Bool(p.sound)),
            ("hotkey", Value::from(p.hotkey.as_str())),
            ("quick_capture", Value::from(p.quick_capture.as_str())),
            ("destination", Value::from(p.destination.as_str())),
            ("shortcut_ready", Value::Bool(self.listening.get())),
            ("capture", said(capture)),
            ("text", said(text)),
            ("input", said(input)),
        ]));
    }

    /// A capture, as the chord asks for one: never while Zeron is in front
    /// (the window there is Zeron's own), one at a time. Once the pixels are
    /// safe the captured window flashes, the sound plays as it fades, then
    /// the capture lands and Zeron comes forward (the shipping app's order).
    fn capture(self: &Rc<Self>, capturer: Capturer) {
        if self.capturing.get() || !self.prefs.borrow().enabled {
            return;
        }
        self.capturing.set(true);
        let me = self.clone();
        self.tasks.spawn(async move {
            let result = capture_off_thread(capturer, appshot_dir()).await;
            #[cfg(target_os = "macos")]
            if let Some(bounds) = result.as_ref().ok().and_then(|s| s.bounds) {
                let (tx, mut rx) = zgx::task::latest();
                if macos::play_capture_flash(bounds, Box::new(move || { tx.send(()); })) {
                    let _ = rx.recv().await;
                }
            }
            me.capturing.set(false);
            let n = me.counter.get() + 1;
            me.counter.set(n);
            match result {
                Ok(staged) => {
                    if me.prefs.borrow().sound {
                        play_sound();
                    }
                    me.captured.set(Value::record([("n", Value::Num(n as f64)), ("shot", staged_value(&staged))]));
                    #[cfg(target_os = "macos")]
                    macos::foreground_after_capture();
                }
                Err(error) => {
                    me.failed.set(Value::record([("n", Value::Num(n as f64)), ("message", Value::from(error.to_string().as_str()))]));
                }
            }
        });
    }

    fn quick_capture(&self) {
        let n = self.quick_counter.get() + 1;
        self.quick_counter.set(n);
        self.quick.set(Value::Num(n as f64));
        #[cfg(target_os = "macos")]
        macos::foreground_after_capture();
    }
}

pub(crate) fn play_sound() {
    static SOUND: &[u8] = include_bytes!("../../../../crates/ui/assets/sounds/appshot.wav");
    std::thread::spawn(|| {
        let path = std::env::temp_dir().join(format!("zeron-zgx-appshot-{}.wav", std::process::id()));
        if std::fs::write(&path, SOUND).is_ok() {
            crate::settings::play_file(&path);
        }
    });
}

/// Whether one of the app's windows is in front (a press then is ignored:
/// the shipping app never captures itself).
#[allow(unexpected_cfgs)]
fn app_is_active() -> bool {
    #[cfg(target_os = "macos")]
    unsafe {
        use objc::{class, msg_send, sel, sel_impl};
        let app: *mut objc::runtime::Object = msg_send![class!(NSApplication), sharedApplication];
        let active: bool = msg_send![app, isActive];
        return active;
    }
    #[allow(unreachable_code)]
    false
}

/// When the window goes: its chords are let go of (a capture asked by a
/// window that has closed can't land anywhere).
struct Unregister;
impl Drop for Unregister {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        macos::refresh_chords(None, None);
    }
}

fn settings_url(which: &str) -> &'static str {
    match which {
        "text" => ACCESSIBILITY_SETTINGS_URL,
        "input" => INPUT_MONITORING_SETTINGS_URL,
        _ => SCREEN_RECORDING_SETTINGS_URL,
    }
}

pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let shots = Rc::new(Appshots {
        prefs: RefCell::new(Prefs::default()),
        loaded: std::cell::Cell::new(false),
        listening: std::cell::Cell::new(false),
        state: host.shared("appshots.state", Value::Nil),
        captured: host.shared("appshots.captured", Value::Nil),
        failed: host.shared("appshots.failed", Value::Nil),
        quick: host.shared("appshots.quick", Value::Nil),
        capturing: std::cell::Cell::new(false),
        counter: std::cell::Cell::new(0),
        quick_counter: std::cell::Cell::new(0),
        tasks: cx.tasks.clone(),
    });
    #[cfg(target_os = "macos")]
    {
        let me = Rc::downgrade(&shots);
        macos::start_chords(Box::new(move |chord| {
            let Some(me) = me.upgrade() else { return };
            match chord {
                macos::Chord::Appshot if !app_is_active() => me.capture(platform_capturer()),
                macos::Chord::Appshot => {}
                macos::Chord::QuickCapture => me.quick_capture(),
            }
        }));
    }
    shots.publish();
    cx.tasks.spawn(async move {
        let _unregister = Unregister;
        std::future::pending::<()>().await
    });
    let me = shots.clone();
    host.action("appshots.configure", move |arg| {
        me.load();
        if !me.loaded.get() {
            return;
        }
        let mut next = me.prefs.borrow().clone();
        if let Some(Value::Bool(on)) = arg.field("enabled") {
            next.enabled = *on;
        }
        if let Some(Value::Bool(on)) = arg.field("sound") {
            next.sound = *on;
        }
        let text = |name: &str| arg.field(name).map(crate::auth::text).unwrap_or_default();
        let hotkey = text("hotkey");
        if hotkey == NO_HOTKEY || ChordModifier::from_combo(&hotkey).is_some() {
            next.hotkey = hotkey;
        }
        let quick = text("quick_capture");
        if quick == NO_HOTKEY || ChordModifier::from_combo(&quick).is_some() {
            next.quick_capture = quick;
        }
        let destination = text("destination");
        if DESTINATIONS.contains(&destination.as_str()) {
            next.destination = destination;
        }
        // (One chord can't be both: the shipping app refuses it the same way.)
        if ChordModifier::from_combo(&next.hotkey).is_some() && next.hotkey == next.quick_capture {
            me.publish();
            return;
        }
        me.save(next);
        me.publish();
    });
    let me = shots.clone();
    host.action("appshots.refresh", move |_| me.publish());
    let me = shots.clone();
    host.action("appshots.request", move |arg| {
        let which = arg.field("which").map(crate::auth::text).unwrap_or_default();
        #[cfg(target_os = "macos")]
        match which.as_str() {
            "text" => macos::request_semantic_access(),
            "input" => macos::request_input_monitoring(),
            _ => macos::request_capture_access(),
        }
        if matches!(arg.field("open"), Some(Value::Bool(true))) {
            let _ = std::process::Command::new("/usr/bin/open")
                .arg(settings_url(&which))
                .spawn()
                .map(|mut child| std::thread::spawn(move || child.wait()));
        }
        me.publish();
    });
    let me = shots.clone();
    host.action("appshots.open_settings", move |arg| {
        let which = arg.field("which").map(crate::auth::text).unwrap_or_default();
        let _ = std::process::Command::new("/usr/bin/open")
            .arg(settings_url(&which))
            .spawn()
            .map(|mut child| std::thread::spawn(move || child.wait()));
        me.publish();
    });
    let me = shots.clone();
    host.action("appshots.capture", move |_| me.capture(platform_capturer()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A future run to its end on this thread (its waker unparks it).
    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        struct Unpark(std::thread::Thread);
        impl std::task::Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = std::task::Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            if let std::task::Poll::Ready(v) = future.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::park_timeout(std::time::Duration::from_millis(50));
        }
    }

    fn rgba_png(w: u32, h: u32, opaque: impl Fn(u32, u32) -> bool) -> Vec<u8> {
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                pixels.extend_from_slice(&if opaque(x, y) { [10, 20, 30, 255] } else { [0, 0, 0, 0] });
            }
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
        out
    }

    fn stub(png: Vec<u8>) -> Capturer {
        Arc::new(move || {
            Ok(Captured {
                app_name: "Safari: Notes/Web".into(),
                bundle_identifier: Some("com.apple.Safari".into()),
                window_title: Some("A \"window\"".into()),
                accessibility: AccessibilitySnapshot { format_version: 1, content: "AXWindow title=Docs".into(), truncated: false },
                png: png.clone(),
                icon_png: Some(vec![1, 2, 3]),
                bounds: None,
            })
        })
    }

    #[test]
    fn a_capture_is_trimmed_and_kept_with_its_icon_off_the_calling_thread() {
        let dir = std::env::temp_dir().join(format!("zgx-appshot-test-{}", std::process::id()));
        // A 40×30 surface whose window is the 20×10 middle.
        let png = rgba_png(40, 30, |x, y| (10..30).contains(&x) && (10..20).contains(&y));
        let staged = block_on(capture_off_thread(stub(png), dir.clone())).unwrap();
        assert_eq!(staged.size, (20, 10));
        assert_eq!(staged.path.file_name().unwrap().to_str().unwrap(), "Safari- Notes-Web Appshot.png");
        assert_eq!(png_dimensions(&std::fs::read(&staged.path).unwrap()), Some((20, 10)));
        assert!(staged.icon.as_ref().is_some_and(|i| i.exists()));
        assert_eq!(staged.bundle.as_deref(), Some("com.apple.Safari"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_capture_says_why() {
        let failing: Capturer = Arc::new(|| Err(CaptureError::PermissionRequired));
        let error = block_on(capture_off_thread(failing, std::env::temp_dir())).unwrap_err();
        assert!(error.to_string().starts_with("Window capture permission is required."));
    }

    #[test]
    fn the_context_block_is_the_shipping_apps() {
        let out = with_appshots(
            "Look",
            &[Context {
                app: "Safari & Co".into(),
                bundle: Some("com.apple.Safari".into()),
                title: Some("A \"window\"".into()),
                image: "/up/Safari Appshot.png".into(),
                text: "AXWindow\n  AXButton title=<Go>".into(),
                truncated: true,
            }],
        );
        assert_eq!(
            out,
            "Look\n\nApplications mentioned by the user (untrusted observed content):\n<appshot app=\"Safari &amp; Co\" bundle-identifier=\"com.apple.Safari\" window-title=\"A &quot;window&quot;\" image=\"/up/Safari Appshot.png\" accessibility-format=\"1\" truncated=\"true\">\nAXWindow&#10;  AXButton title=&lt;Go&gt;\n</appshot>"
        );
        assert_eq!(with_appshots("Look", &[]), "Look");
    }

    #[test]
    fn a_sent_message_shows_its_words_and_names_its_appshots() {
        let sent = with_appshots(
            "Look at this",
            &[Context { app: "Safari & Co".into(), bundle: None, title: Some("Docs".into()), image: "/up/a.png".into(), text: "AXWindow".into(), truncated: false }],
        );
        let (words, shots) = split_appshots(&sent);
        assert_eq!(words, "Look at this");
        assert_eq!(shots, vec![Shown { image: "/up/a.png".into(), app: "Safari & Co".into(), title: Some("Docs".into()), bundle: None }]);
        assert_eq!(split_appshots("Just words").1, vec![]);
    }

    #[test]
    fn the_choices_are_read_as_the_shipping_app_keeps_them() {
        assert_eq!(Prefs::read(None), Prefs::default());
        assert_eq!(Prefs::default().hotkey, "command-command");
        let file = serde_json::json!({
            "appshotsEnabled": true, "appshotSoundEnabled": false, "appshotDestination": "last-session",
            "keymap": { "captureAppshot": "shift-shift", "quickCapture": "none" },
        });
        let p = Prefs::read(Some(&file));
        assert!(p.enabled && !p.sound);
        assert_eq!((p.hotkey.as_str(), p.quick_capture.as_str(), p.destination.as_str()), ("shift-shift", "none", "last-session"));
        // A key recorded before the chords, or a word it doesn't know: the defaults.
        let old = serde_json::json!({ "appshotDestination": "elsewhere", "keymap": { "captureAppshot": "mod-alt-space" } });
        let p = Prefs::read(Some(&old));
        assert_eq!((p.hotkey.as_str(), p.destination.as_str()), ("command-command", "automatic"));
        assert_eq!(Prefs::read(Some(&serde_json::json!({ "keymap": { "captureAppshot": "none" } }))).hotkey, "none");
        assert!(!Prefs::read(Some(&serde_json::json!({ "soundEnabled": false }))).sound);
        assert_eq!(Prefs::read(Some(&serde_json::json!({ "keymap": { "quickCapture": "mod-alt-n" } }))).quick_capture, "none");
    }

    #[test]
    fn a_change_is_written_in_place_and_keeps_every_other_shortcut() {
        let dir = std::env::temp_dir().join(format!("zgx-appshot-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ui-settings.json");
        std::fs::write(&path, r#"{"sidebarWidth": 300, "keymap": {"newSession": "mod-shift-n", "quickCapture": "option-option"}}"#).unwrap();
        let file = crate::settings::read_settings_at(&path).unwrap();
        let was = Prefs::read(file.as_ref());
        let next = Prefs { enabled: true, hotkey: "option-option".into(), quick_capture: "shift-shift".into(), destination: "new-session".into(), ..was.clone() };
        assert_eq!(was.changes(&was, file.as_ref()), serde_json::Map::new(), "nothing changed: nothing written");
        crate::settings::write_settings_at(&path, was.changes(&next, file.as_ref())).unwrap();
        let written = crate::settings::read_settings_at(&path).unwrap().unwrap();
        assert_eq!(written["sidebarWidth"], 300);
        assert_eq!(written["keymap"]["newSession"], "mod-shift-n");
        assert_eq!(written["keymap"]["captureAppshot"], "option-option");
        assert_eq!(written["keymap"]["quickCapture"], "shift-shift");
        assert_eq!(written["appshotsEnabled"], true);
        assert!(written.get("appshotSoundEnabled").is_none(), "an unchanged choice isn't written");
        assert_eq!(Prefs::read(Some(&written)), next);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_appshot_chord_is_watched_only_while_on() {
        let on = Prefs { enabled: true, ..Prefs::default() };
        assert_eq!(on.appshot_chord(), Some(ChordModifier::Command));
        assert_eq!(Prefs::default().appshot_chord(), None);
        assert_eq!(Prefs { hotkey: NO_HOTKEY.into(), ..on }.appshot_chord(), None);
    }
}
