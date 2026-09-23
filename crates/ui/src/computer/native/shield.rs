use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSBackingStoreType, NSColor, NSFont,
    NSFontWeightSemibold, NSLineBreakMode, NSPanel, NSScreen, NSStatusWindowLevel, NSTextField,
    NSView, NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
    NSWorkspaceActiveSpaceDidChangeNotification,
};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue, NSPoint, NSRect,
    NSSize, NSString,
};
use tokio::sync::oneshot;
use zeron_proto::computer::{
    CuaComputerTask, CuaShieldFrame, cua_computer_task_key, is_cua_shield_id,
};

use crate::computer::host::{ComputerShieldEngagement, ComputerShieldHost};

pub const SHIELD_TTL: Duration = Duration::from_secs(30);
pub const SHIELD_LIMIT: usize = 8;
const SHIELD_ENGAGE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_ENDED_TASKS: usize = 256;
const DEFAULT_LABEL: &str = "Zeron is activating this window";

pub type ShieldReply = Box<dyn FnOnce(Result<(), String>) + Send>;

pub trait ShieldSurface: Send + Sync + 'static {
    fn engage(
        &self,
        id: String,
        frame: CuaShieldFrame,
        label: Option<String>,
        token: u64,
        reply: ShieldReply,
    );
    fn release(&self, id: String, token: Option<u64>, reason: &'static str);
    fn release_all(&self, reason: &'static str);
}

pub(super) struct PanelRegistry<P> {
    panels: HashMap<String, (P, u64)>,
}

impl<P> PanelRegistry<P> {
    pub(super) fn new() -> Self {
        Self {
            panels: HashMap::new(),
        }
    }

    pub(super) fn engage(
        &mut self,
        id: String,
        token: u64,
        build: impl FnOnce() -> P,
    ) -> Result<Option<&P>, &'static str> {
        if self.panels.contains_key(&id) {
            return Ok(None);
        }
        if self.panels.len() >= SHIELD_LIMIT {
            return Err("shield-limit");
        }
        let (panel, _) = self.panels.entry(id).or_insert((build(), token));
        Ok(Some(panel))
    }

    pub(super) fn release(&mut self, id: &str, token: Option<u64>) -> Option<P> {
        match self.panels.get(id) {
            Some((_, live)) if token.is_none_or(|token| token == *live) => {
                self.panels.remove(id).map(|(panel, _)| panel)
            }
            _ => None,
        }
    }

    pub(super) fn release_all(&mut self) -> Vec<P> {
        self.panels.drain().map(|(_, (panel, _))| panel).collect()
    }
}

pub fn cocoa_rect(frame: &CuaShieldFrame, primary_max_y: f64) -> NSRect {
    NSRect::new(
        NSPoint::new(frame.x, primary_max_y - frame.y - frame.height),
        NSSize::new(frame.width, frame.height),
    )
}

fn valid_geometry(frame: &CuaShieldFrame) -> bool {
    [frame.x, frame.y, frame.width, frame.height]
        .iter()
        .all(|value| value.is_finite())
        && frame.width > 0.0
        && frame.height > 0.0
}

struct LiveShield {
    task_key: Option<String>,
    thread_id: Option<String>,
}

struct Book {
    live: HashMap<String, LiveShield>,
    pending: HashMap<String, (u64, oneshot::Sender<Result<(), String>>)>,
    ended: VecDeque<String>,
    ended_set: HashSet<String>,
    closed: bool,
    next_token: u64,
}

impl Book {
    fn remember_ended(&mut self, key: String) {
        if self.ended_set.insert(key.clone()) {
            self.ended.push_back(key);
        }
        while self.ended.len() > MAX_ENDED_TASKS {
            if let Some(oldest) = self.ended.pop_front() {
                self.ended_set.remove(&oldest);
            }
        }
    }

    fn ended(&self, key: Option<&String>) -> bool {
        key.is_some_and(|key| self.ended_set.contains(key))
    }

    fn fail_pending(&mut self, error: &str) {
        for (_, (_, sender)) in self.pending.drain() {
            let _ = sender.send(Err(error.into()));
        }
    }
}

pub struct ComputerShield {
    book: Arc<Mutex<Book>>,
    surface: Arc<dyn ShieldSurface>,
    ttl: Duration,
    engage_timeout: Duration,
}

impl ComputerShield {
    pub fn new(surface: Arc<dyn ShieldSurface>) -> Self {
        Self::with_timing(surface, SHIELD_TTL, SHIELD_ENGAGE_TIMEOUT)
    }

    fn with_timing(
        surface: Arc<dyn ShieldSurface>,
        ttl: Duration,
        engage_timeout: Duration,
    ) -> Self {
        Self {
            book: Arc::new(Mutex::new(Book {
                live: HashMap::new(),
                pending: HashMap::new(),
                ended: VecDeque::new(),
                ended_set: HashSet::new(),
                closed: false,
                next_token: 0,
            })),
            surface,
            ttl,
            engage_timeout,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Book> {
        self.book
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn start_ttl(&self, id: String, token: u64) {
        let surface = self.surface.clone();
        let ttl = self.ttl;
        let _ = std::thread::Builder::new()
            .name("zeron-cua-shield-ttl".into())
            .spawn(move || {
                std::thread::sleep(ttl);
                surface.release(id, Some(token), "ttl-expired");
            });
    }

    fn release_now(&self, id: String) {
        self.lock().live.remove(&id);
        self.surface.release(id, None, "explicit");
    }
}

#[async_trait(?Send)]
impl ComputerShieldHost for ComputerShield {
    async fn engage(
        &self,
        request: ComputerShieldEngagement,
        task: Option<CuaComputerTask>,
    ) -> Result<(), String> {
        let task_key = task.as_ref().map(cua_computer_task_key);
        if !is_cua_shield_id(&request.shield_id) {
            return Err(
                "The activation shield failed: engage requires a printable shield id.".into(),
            );
        }
        if !valid_geometry(&request.frame) {
            return Err(
                "The activation shield failed: engage requires finite positive geometry.".into(),
            );
        }
        let id = request.shield_id;
        let (token, confirmation) = {
            let mut book = self.lock();
            if book.closed {
                return Err("The activation shield host is closed.".into());
            }
            book.next_token += 1;
            let token = book.next_token;
            let (sender, confirmation) = oneshot::channel();
            book.pending.insert(id.clone(), (token, sender));
            (token, confirmation)
        };
        let book = self.book.clone();
        let reply_id = id.clone();
        let reply: ShieldReply = Box::new(move |result| {
            let sender = {
                let mut book = book.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                match book.pending.get(&reply_id) {
                    Some((live, _)) if *live == token => {
                        book.pending.remove(&reply_id).map(|(_, sender)| sender)
                    }
                    _ => None,
                }
            };
            if let Some(sender) = sender {
                let _ = sender.send(result);
            }
        });
        self.surface
            .engage(id.clone(), request.frame, request.label, token, reply);
        self.start_ttl(id.clone(), token);
        let outcome = match tokio::time::timeout(self.engage_timeout, confirmation).await {
            Ok(Ok(result)) => result,
            _ => {
                let mut book = self.lock();
                if book
                    .pending
                    .get(&id)
                    .is_some_and(|(live, _)| *live == token)
                {
                    book.pending.remove(&id);
                }
                Err("The activation shield did not confirm in time.".into())
            }
        };
        if let Err(error) = outcome {
            if self.lock().ended(task_key.as_ref()) {
                self.release_now(id);
            }
            return Err(error);
        }
        let ended = {
            let mut book = self.lock();
            book.live.insert(
                id.clone(),
                LiveShield {
                    task_key: task_key.clone(),
                    thread_id: task.map(|task| task.thread_id),
                },
            );
            book.ended(task_key.as_ref())
        };
        if ended {
            self.release_now(id);
        }
        Ok(())
    }

    async fn release(&self, shield_id: String) -> Result<(), String> {
        self.release_now(shield_id);
        Ok(())
    }

    async fn release_all(&self) -> Result<u64, String> {
        let released = {
            let mut book = self.lock();
            let released = book.live.len() as u64;
            book.live.clear();
            released
        };
        self.surface.release_all("explicit");
        Ok(released)
    }

    async fn end_task(&self, task: CuaComputerTask) -> Result<(), String> {
        let key = cua_computer_task_key(&task);
        let matching: Vec<String> = {
            let mut book = self.lock();
            book.remember_ended(key.clone());
            book.live
                .iter()
                .filter(|(_, shield)| {
                    shield.thread_id.as_deref() == Some(task.thread_id.as_str())
                        && (task.turn_id.is_none() || shield.task_key.as_deref() == Some(&key))
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in matching {
            self.release_now(id);
        }
        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        let released = {
            let mut book = self.lock();
            let released = book.live.len();
            book.live.clear();
            book.fail_pending("The activation shield host stopped.");
            released
        };
        self.surface.release_all("quit");
        if released > 0 {
            tracing::info!(target: "desktop_cua", "activation shield stop released {released} live shield(s)");
        }
        Ok(())
    }

    async fn dispose(&self) -> Result<(), String> {
        self.lock().closed = true;
        self.stop().await
    }
}

struct MainShields {
    registry: PanelRegistry<Retained<NSPanel>>,
    _observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

thread_local! {
    static MAIN_SHIELDS: RefCell<Option<MainShields>> = const { RefCell::new(None) };
}

fn close_panels(panels: Vec<Retained<NSPanel>>) {
    for panel in panels {
        panel.orderOut(None);
        panel.close();
    }
}

fn release_all_on_main(_reason: &'static str) {
    let panels = MAIN_SHIELDS.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut shields| shields.as_mut().map(|s| s.registry.release_all()))
            .unwrap_or_default()
    });
    close_panels(panels);
}

fn observe(
    center: &NSNotificationCenter,
    name: &objc2_foundation::NSNotificationName,
    reason: &'static str,
) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
    let block = block2::RcBlock::new(move |_: NonNull<NSNotification>| release_all_on_main(reason));
    unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(name),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &block,
        )
    }
}

impl MainShields {
    fn start() -> Self {
        let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
        let default = NSNotificationCenter::defaultCenter();
        Self {
            registry: PanelRegistry::new(),
            _observers: vec![
                observe(
                    &workspace,
                    unsafe { NSWorkspaceActiveSpaceDidChangeNotification },
                    "space-changed",
                ),
                observe(
                    &default,
                    unsafe { NSApplicationDidChangeScreenParametersNotification },
                    "display-changed",
                ),
            ],
        }
    }
}

fn on_main(
    work: impl FnOnce(MainThreadMarker, &mut MainShields) -> Vec<Retained<NSPanel>> + Send + 'static,
) {
    DispatchQueue::main().exec_async(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let panels = MAIN_SHIELDS.with(|cell| {
            let mut shields = cell.borrow_mut();
            work(mtm, shields.get_or_insert_with(MainShields::start))
        });
        close_panels(panels);
    });
}

fn primary_max_y(mtm: MainThreadMarker) -> Option<f64> {
    let screens = NSScreen::screens(mtm);
    let primary = screens
        .iter()
        .find(|screen| screen.frame().origin == NSPoint::new(0.0, 0.0))
        .or_else(|| NSScreen::mainScreen(mtm))
        .or_else(|| screens.firstObject())?;
    let frame = primary.frame();
    Some(frame.origin.y + frame.size.height)
}

fn set_layer(view: &NSView, background: &NSColor, border: Option<&NSColor>, radius: f64) {
    view.setWantsLayer(true);
    let layer: Option<Retained<AnyObject>> = unsafe { msg_send![view, layer] };
    let Some(layer) = layer else {
        return;
    };
    let background = background.CGColor();
    unsafe {
        let _: () = msg_send![&*layer, setBackgroundColor: &*background];
        let _: () = msg_send![&*layer, setCornerRadius: radius];
        let _: () = msg_send![&*layer, setCornerCurve: &*NSString::from_str("continuous")];
        if let Some(border) = border {
            let border = border.CGColor();
            let _: () = msg_send![&*layer, setBorderColor: &*border];
            let _: () = msg_send![&*layer, setBorderWidth: 2.0f64];
        }
    }
}

fn build_panel(
    mtm: MainThreadMarker,
    frame: &CuaShieldFrame,
    label: Option<&str>,
) -> Retained<NSPanel> {
    let rect = cocoa_rect(frame, primary_max_y(mtm).unwrap_or(frame.y + frame.height));
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        rect,
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setFloatingPanel(true);
    panel.setLevel(NSStatusWindowLevel);
    panel.setHidesOnDeactivate(false);
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setIgnoresMouseEvents(true);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::IgnoresCycle | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    unsafe { panel.setReleasedWhenClosed(false) };

    let size = rect.size;
    let content = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), size),
    );
    let accent = NSColor::controlAccentColor();
    set_layer(
        &content,
        &accent.colorWithAlphaComponent(0.14),
        Some(&accent.colorWithAlphaComponent(0.9)),
        8.0,
    );

    let text = label
        .filter(|label| !label.is_empty())
        .unwrap_or(DEFAULT_LABEL);
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, unsafe {
        NSFontWeightSemibold
    })));
    field.setTextColor(Some(&NSColor::whiteColor()));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    let field_size = field.frame().size;
    let padding = 10.0;
    let pill_height = field_size.height + 8.0;
    let pill_width = (field_size.width + padding * 2.0).min((size.width - 24.0).max(0.0));
    let pill = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(
                (size.width - pill_width) / 2.0,
                size.height - pill_height - 10.0,
            ),
            NSSize::new(pill_width, pill_height),
        ),
    );
    set_layer(
        &pill,
        &NSColor::blackColor().colorWithAlphaComponent(0.75),
        None,
        pill_height / 2.0,
    );
    field.setFrame(NSRect::new(
        NSPoint::new(padding, (pill_height - field_size.height) / 2.0),
        NSSize::new((pill_width - padding * 2.0).max(0.0), field_size.height),
    ));
    pill.addSubview(&field);
    content.addSubview(&pill);
    panel.setContentView(Some(&content));
    panel
}

pub struct AppKitShieldSurface;

impl ShieldSurface for AppKitShieldSurface {
    fn engage(
        &self,
        id: String,
        frame: CuaShieldFrame,
        label: Option<String>,
        token: u64,
        reply: ShieldReply,
    ) {
        on_main(move |mtm, shields| {
            let outcome = shields
                .registry
                .engage(id, token, || build_panel(mtm, &frame, label.as_deref()));
            match outcome {
                Ok(Some(panel)) => {
                    panel.orderFrontRegardless();
                    reply(Ok(()));
                }
                Ok(None) => reply(Ok(())),
                Err(_) => reply(Err("The activation shield was refused (refused).".into())),
            }
            Vec::new()
        });
    }

    fn release(&self, id: String, token: Option<u64>, _reason: &'static str) {
        on_main(move |_, shields| shields.registry.release(&id, token).into_iter().collect());
    }

    fn release_all(&self, _reason: &'static str) {
        on_main(|_, shields| shields.registry.release_all());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Surface {
        registry: Mutex<PanelRegistry<u64>>,
        log: Mutex<Vec<String>>,
        silent: bool,
    }

    impl Surface {
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Default for PanelRegistry<u64> {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ShieldSurface for Surface {
        fn engage(
            &self,
            id: String,
            _frame: CuaShieldFrame,
            _label: Option<String>,
            token: u64,
            reply: ShieldReply,
        ) {
            self.log.lock().unwrap().push(format!("engage {id}"));
            if self.silent {
                std::mem::forget(reply);
                return;
            }
            let outcome = self
                .registry
                .lock()
                .unwrap()
                .engage(id, token, || token)
                .map(|_| ());
            reply(outcome.map_err(|_| "The activation shield was refused (refused).".into()));
        }

        fn release(&self, id: String, token: Option<u64>, reason: &'static str) {
            let released = self.registry.lock().unwrap().release(&id, token).is_some();
            self.log
                .lock()
                .unwrap()
                .push(format!("release {id} {reason} {released}"));
        }

        fn release_all(&self, reason: &'static str) {
            let released = self.registry.lock().unwrap().release_all().len();
            self.log
                .lock()
                .unwrap()
                .push(format!("release-all {reason} {released}"));
        }
    }

    fn frame() -> CuaShieldFrame {
        CuaShieldFrame {
            x: 10.0,
            y: 20.0,
            width: 300.0,
            height: 200.0,
        }
    }

    fn engagement(id: &str) -> ComputerShieldEngagement {
        ComputerShieldEngagement {
            shield_id: id.into(),
            frame: frame(),
            window_id: 7,
            pid: 42,
            label: None,
        }
    }

    fn task(thread: &str, turn: Option<&str>) -> CuaComputerTask {
        CuaComputerTask {
            thread_id: thread.into(),
            turn_id: turn.map(Into::into),
            label: None,
        }
    }

    fn shield(surface: &Arc<Surface>) -> ComputerShield {
        ComputerShield::with_timing(
            surface.clone(),
            Duration::from_secs(60),
            Duration::from_millis(50),
        )
    }

    #[test]
    fn the_registry_is_idempotent_bounded_and_token_scoped() {
        let mut registry = PanelRegistry::new();
        assert_eq!(registry.engage("a".into(), 1, || 1).unwrap(), Some(&1));
        assert_eq!(registry.engage("a".into(), 2, || 2).unwrap(), None);
        assert_eq!(
            registry.release("a", Some(2)),
            None,
            "stale TTL leaves a re-engaged id"
        );
        assert_eq!(registry.release("a", Some(1)), Some(1));
        for n in 0..SHIELD_LIMIT as u64 {
            registry.engage(format!("s{n}"), n, || n).unwrap();
        }
        assert_eq!(
            registry.engage("over".into(), 99, || 99),
            Err("shield-limit")
        );
        assert_eq!(registry.release("s0", None), Some(0));
        assert_eq!(registry.release_all().len(), SHIELD_LIMIT - 1);
        assert_eq!(registry.release("s1", None), None);
    }

    #[test]
    fn cocoa_frames_flip_from_the_primary_screen_top() {
        let rect = cocoa_rect(&frame(), 900.0);
        assert_eq!((rect.origin.x, rect.origin.y), (10.0, 680.0));
        assert_eq!((rect.size.width, rect.size.height), (300.0, 200.0));
    }

    #[tokio::test]
    async fn engage_confirms_and_rejects_bad_geometry_before_the_surface() {
        let surface = Arc::new(Surface::default());
        let shield = shield(&surface);
        shield.engage(engagement("one"), None).await.unwrap();
        let mut bad = engagement("two");
        bad.frame.width = 0.0;
        assert_eq!(
            shield.engage(bad, None).await.unwrap_err(),
            "The activation shield failed: engage requires finite positive geometry."
        );
        let mut nan = engagement("three");
        nan.frame.x = f64::NAN;
        assert!(shield.engage(nan, None).await.is_err());
        assert!(shield.engage(engagement("bad id!"), None).await.is_err());
        assert_eq!(surface.log(), vec!["engage one"]);
        assert_eq!(shield.release_all().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_silent_surface_times_out_and_stop_fails_pending() {
        let surface = Arc::new(Surface {
            silent: true,
            ..Surface::default()
        });
        let shield = shield(&surface);
        assert_eq!(
            shield.engage(engagement("one"), None).await.unwrap_err(),
            "The activation shield did not confirm in time."
        );
        let slow = ComputerShield::with_timing(
            surface.clone(),
            Duration::from_secs(60),
            Duration::from_secs(5),
        );
        let (engaged, ()) = tokio::join!(slow.engage(engagement("two"), None), async {
            tokio::task::yield_now().await;
            slow.stop().await.unwrap();
        });
        assert_eq!(engaged.unwrap_err(), "The activation shield host stopped.");
    }

    #[tokio::test]
    async fn refused_engages_surface_the_refusal() {
        let surface = Arc::new(Surface::default());
        let shield = shield(&surface);
        for n in 0..SHIELD_LIMIT {
            shield
                .engage(engagement(&format!("s{n}")), None)
                .await
                .unwrap();
        }
        assert_eq!(
            shield.engage(engagement("over"), None).await.unwrap_err(),
            "The activation shield was refused (refused)."
        );
    }

    #[tokio::test]
    async fn end_task_releases_the_thread_or_the_exact_turn() {
        let surface = Arc::new(Surface::default());
        let shield = shield(&surface);
        shield
            .engage(engagement("a1"), Some(task("a", Some("1"))))
            .await
            .unwrap();
        shield
            .engage(engagement("a2"), Some(task("a", Some("2"))))
            .await
            .unwrap();
        shield
            .engage(engagement("b1"), Some(task("b", Some("1"))))
            .await
            .unwrap();
        shield.end_task(task("a", Some("1"))).await.unwrap();
        assert!(
            surface
                .log()
                .contains(&"release a1 explicit true".to_string())
        );
        assert!(
            !surface
                .log()
                .iter()
                .any(|line| line.starts_with("release a2"))
        );
        shield.end_task(task("a", None)).await.unwrap();
        assert!(
            surface
                .log()
                .contains(&"release a2 explicit true".to_string())
        );
        assert_eq!(shield.release_all().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_shield_confirmed_after_its_task_ended_is_released_at_once() {
        let surface = Arc::new(Surface::default());
        let shield = shield(&surface);
        shield.end_task(task("a", Some("1"))).await.unwrap();
        shield
            .engage(engagement("late"), Some(task("a", Some("1"))))
            .await
            .unwrap();
        assert_eq!(
            surface.log(),
            vec!["engage late", "release late explicit true"]
        );
        assert_eq!(shield.release_all().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn the_ttl_runs_off_the_caller_and_spares_a_re_engaged_id() {
        let surface = Arc::new(Surface::default());
        let shield = ComputerShield::with_timing(
            surface.clone(),
            Duration::from_millis(30),
            Duration::from_millis(500),
        );
        shield.engage(engagement("one"), None).await.unwrap();
        std::thread::sleep(Duration::from_millis(120));
        assert!(
            surface
                .log()
                .contains(&"release one ttl-expired true".to_string())
        );
        shield.engage(engagement("two"), None).await.unwrap();
        shield.release("two".into()).await.unwrap();
        shield.engage(engagement("two"), None).await.unwrap();
        std::thread::sleep(Duration::from_millis(120));
        let log = surface.log();
        assert_eq!(
            log.iter()
                .filter(|line| line.starts_with("release two ttl-expired true"))
                .count(),
            1,
            "{log:?}"
        );
    }

    #[tokio::test]
    async fn dispose_closes_the_host_and_drops_every_shield() {
        let surface = Arc::new(Surface::default());
        let shield = shield(&surface);
        shield.engage(engagement("one"), None).await.unwrap();
        shield.dispose().await.unwrap();
        assert_eq!(surface.log().last().unwrap(), "release-all quit 1");
        assert_eq!(
            shield.engage(engagement("two"), None).await.unwrap_err(),
            "The activation shield host is closed."
        );
    }
}
