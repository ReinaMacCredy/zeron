//! Live agent HTML pages inside the transcript. Each card owns a WKWebView
//! kept at the card's exact bounds but fully transparent: WebKit still lays
//! it out, animates it and answers snapshots, and the card paints the latest
//! snapshot, so the page clips, fades and blurs under the transcript chrome
//! like text and GPUI popovers draw over it. The transparent view takes
//! pointer input only while armed and inside the card's input region. Rows
//! scrolled out of the list are never painted, so a window-level sweep
//! disarms any page whose card did not sync during the frame being presented.
use super::model::allowed_navigation;
use gpui::{Bounds, IntoElement, Pixels, Point, RenderImage, SharedString, Styled, Window};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send};
use objc2_app_kit::{
    NSEvent, NSEventMask, NSEventType, NSImage, NSView, NSWindow,
    NSWindowDidChangeOcclusionStateNotification, NSWindowDidDeminiaturizeNotification,
    NSWindowOcclusionState, NSWindowOrderingMode,
};
use objc2_foundation::{
    NSError, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSOperationQueue,
    NSPoint, NSRect, NSSize, NSString,
};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
    WKNavigationType, WKSnapshotConfiguration, WKWebView,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::NonNull;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use wry::{WebView, WebViewBuilderExtMacos, WebViewExtMacOS};

pub(crate) enum InlineEvent {
    Height(f32),
    OpenLink(String),
    Crashed,
    Frame(Arc<RenderImage>, f32),
    Active,
    Pointer,
}

pub(crate) type InlineSender = tokio::sync::mpsc::UnboundedSender<(SharedString, InlineEvent)>;

pub(crate) const FRAME_INTERVAL: Duration = Duration::from_millis(33);
const POINTER_INTERVAL: Duration = Duration::from_millis(100);

struct PolicyState {
    key: SharedString,
    tx: InlineSender,
    loaded: Cell<bool>,
    clicked: Arc<AtomicU64>,
}

/// A page opens links only right after the reader clicks inside its card,
/// never from script alone.
const CLICK_WINDOW_MS: u64 = 1500;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

fn clicked_recently(clicked: &AtomicU64) -> bool {
    now_ms().saturating_sub(clicked.load(Ordering::Relaxed)) <= CLICK_WINDOW_MS
}

fn debug() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ZERON_VISUAL_DEBUG").is_some())
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ZeronInlinePagePolicy"]
    #[ivars = PolicyState]
    struct Policy;
    unsafe impl NSObjectProtocol for Policy {}
    unsafe impl WKNavigationDelegate for Policy {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn policy(
            &self,
            _view: &WKWebView,
            action: &WKNavigationAction,
            decision: &block2::Block<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let url = unsafe { action.request().URL() }
                .and_then(|u| u.absoluteString())
                .map(|u| u.to_string())
                .unwrap_or_default();
            let main =
                unsafe { action.targetFrame() }.is_some_and(|frame| unsafe { frame.isMainFrame() });
            let link = unsafe { action.navigationType() } == WKNavigationType::LinkActivated
                && clicked_recently(&self.ivars().clicked);
            let allow = page_navigation(&url, main, link, &self.ivars().loaded, |url| {
                let _ = self
                    .ivars()
                    .tx
                    .send((self.ivars().key.clone(), InlineEvent::OpenLink(url)));
            });
            decision.call((if allow {
                WKNavigationActionPolicy::Allow
            } else {
                WKNavigationActionPolicy::Cancel
            },));
        }
        #[unsafe(method(webView:didFinishNavigation:))]
        fn finish(&self, _view: &WKWebView, _navigation: Option<&WKNavigation>) {}
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn terminated(&self, _view: &WKWebView) {
            let _ = self
                .ivars()
                .tx
                .send((self.ivars().key.clone(), InlineEvent::Crashed));
        }
    }
);

/// The card's page never leaves the thread: its first load is the page
/// itself; a link the reader clicks opens in their browser; every other
/// top-level navigation is refused. Embedded frames load, never from disk.
fn page_navigation(
    url: &str,
    main_frame: bool,
    link: bool,
    loaded: &Cell<bool>,
    open: impl FnOnce(String),
) -> bool {
    if !main_frame {
        return !url.starts_with("file:");
    }
    if !loaded.get() {
        loaded.set(true);
        return url == "about:blank";
    }
    if url.starts_with("about:blank#") {
        return true;
    }
    if link && allowed_navigation(url) {
        open(url.to_owned());
    }
    false
}

struct InputState {
    armed: Cell<bool>,
    region: Cell<NSRect>,
    scrolls: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "ZeronInlinePageInput"]
    #[ivars = InputState]
    struct PageInput;
    impl PageInput {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut NSView {
            let state = self.ivars();
            if state.armed.get() && contains(state.region.get(), point) {
                unsafe { msg_send![super(self), hitTest: point] }
            } else {
                std::ptr::null_mut()
            }
        }
    }
);

fn contains(rect: NSRect, point: NSPoint) -> bool {
    point.x >= rect.origin.x
        && point.x < rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y < rect.origin.y + rect.size.height
}

fn set_armed(input: &PageInput, key: &str, armed: bool, why: &'static str) {
    if input.ivars().armed.replace(armed) != armed && debug() {
        tracing::info!(page = key, armed, why, "inline page input");
    }
}

struct Host {
    web: WebView,
    view: Retained<WKWebView>,
    _policy: Retained<Policy>,
    input: Retained<PageInput>,
    parent: Retained<NSView>,
    key: SharedString,
    tx: InlineSender,
    placed: Cell<Option<(Bounds<Pixels>, Bounds<Pixels>)>>,
    window: u64,
    stamp: Cell<u64>,
    concealed: Cell<bool>,
    snapshotting: Arc<AtomicBool>,
    last_snapshot: Cell<Option<Instant>>,
    missed: Arc<AtomicBool>,
    monitor: Option<Retained<AnyObject>>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

#[derive(Clone)]
pub(crate) struct InlineWebView(Rc<RefCell<Host>>);

thread_local! {
    static TICKS: RefCell<HashMap<u64, u64>> = RefCell::new(HashMap::new());
    static HOSTS: RefCell<Vec<Weak<RefCell<Host>>>> = const { RefCell::new(Vec::new()) };
}

fn tick(window: u64) -> u64 {
    TICKS.with(|ticks| ticks.borrow().get(&window).copied().unwrap_or(0))
}

/// Painted first in a window's root: opens the frame the sweep checks against.
pub(crate) fn frame_begin() -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        |_, _, window, _| {
            let id = window.window_handle().window_id().as_u64();
            window.on_present(move || {
                TICKS.with(|ticks| *ticks.borrow_mut().entry(id).or_default() += 1);
            });
        },
    )
    .absolute()
    .size_0()
}

/// Painted last in a window's root: disarms every page whose card was not
/// painted in this frame (scrolled out, chat switched, route left, sheet open).
pub(crate) fn frame_end() -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        |_, _, window, _| {
            let id = window.window_handle().window_id().as_u64();
            window.on_present(move || {
                let now = tick(id);
                HOSTS.with(|hosts| {
                    hosts.borrow_mut().retain(|host| {
                        let Some(host) = host.upgrade() else {
                            return false;
                        };
                        if let Ok(host) = host.try_borrow()
                            && host.window == id
                            && host.stamp.get() != now
                        {
                            host.conceal();
                        }
                        true
                    });
                });
            });
        },
    )
    .absolute()
    .size_0()
}

impl InlineWebView {
    pub(crate) fn new(
        window: &Window,
        key: SharedString,
        html: &str,
        tx: InlineSender,
    ) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("Pages must be created on the main thread")?;
        window
            .enable_scene_overlay()
            .map_err(|error| error.to_string())?;
        let ipc_tx = tx.clone();
        let ipc_key = key.clone();
        let link_tx = tx.clone();
        let link_key = key.clone();
        let clicked = Arc::new(AtomicU64::new(0));
        let link_clicked = clicked.clone();
        let configuration = unsafe {
            let configuration = objc2_web_kit::WKWebViewConfiguration::new(mtm);
            configuration
                .preferences()
                .setJavaScriptCanOpenWindowsAutomatically(false);
            configuration
        };
        let web = wry::WebViewBuilder::new()
            .with_webview_configuration(configuration)
            .with_visible(false)
            .with_focused(false)
            .with_incognito(true)
            .with_transparent(true)
            .with_accept_first_mouse(true)
            .with_ipc_handler(move |request| {
                if let Some(event) = page_signal(request.body()) {
                    let _ = ipc_tx.send((ipc_key.clone(), event));
                }
            })
            .with_permission_handler(|_| wry::PermissionResponse::Deny)
            .with_drag_drop_handler(|_| true)
            .with_new_window_req_handler(move |url, _| {
                if allowed_navigation(&url) && clicked_recently(&link_clicked) {
                    let _ = link_tx.send((link_key.clone(), InlineEvent::OpenLink(url)));
                }
                wry::NewWindowResponse::Deny
            })
            .with_download_started_handler(|_, _| false)
            .build_as_child(window)
            .map_err(|e| e.to_string())?;
        let view: Retained<WKWebView> = Retained::into_super(web.webview());
        let parent = unsafe { view.superview() }.ok_or("Page parent is missing")?;
        let input: Retained<PageInput> = unsafe {
            let object = mtm.alloc().set_ivars(InputState {
                armed: Cell::new(false),
                region: Cell::new(NSRect::ZERO),
                scrolls: Cell::new(false),
            });
            msg_send![super(object), initWithFrame: parent.bounds()]
        };
        input.setAutoresizesSubviews(false);
        parent.addSubview(&input);
        // Beneath GPUI's overlay plane, so popovers and menus keep their clicks.
        for sibling in parent.subviews() {
            if sibling.class().name() == c"GPUIOverlayView" {
                parent.addSubview_positioned_relativeTo(
                    &input,
                    NSWindowOrderingMode::Below,
                    Some(&sibling),
                );
                break;
            }
        }
        input.addSubview(&view);
        // Transparent, never hidden: WebKit stops rendering a hidden view.
        view.setAlphaValue(0.0);
        view.setHidden(false);
        let policy: Retained<Policy> = {
            let object = mtm.alloc().set_ivars(PolicyState {
                key: key.clone(),
                tx: tx.clone(),
                loaded: Cell::new(false),
                clicked: clicked.clone(),
            });
            unsafe { msg_send![super(object), init] }
        };
        unsafe { view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*policy))) };
        let monitor = input_monitor(
            input.clone(),
            parent.clone(),
            view.clone(),
            clicked,
            key.clone(),
            tx.clone(),
        );
        let missed = Arc::new(AtomicBool::new(false));
        let observers = shown_observers(parent.window(), &missed, &key, &tx);
        let host = Rc::new(RefCell::new(Host {
            web,
            view,
            _policy: policy,
            input,
            parent,
            key,
            tx,
            placed: Cell::new(None),
            window: window.window_handle().window_id().as_u64(),
            stamp: Cell::new(u64::MAX),
            concealed: Cell::new(true),
            snapshotting: Arc::new(AtomicBool::new(false)),
            last_snapshot: Cell::new(None),
            missed,
            monitor,
            observers,
        }));
        host.borrow()
            .web
            .load_html(html)
            .map_err(|e| e.to_string())?;
        HOSTS.with(|hosts| hosts.borrow_mut().push(Rc::downgrade(&host)));
        Ok(Self(host))
    }

    /// Paint-phase registration: places the page over `bounds`, taking input
    /// only inside `mask`, when this frame is presented.
    pub(crate) fn sync_on_present(
        &self,
        bounds: Bounds<Pixels>,
        mask: Bounds<Pixels>,
        window: &mut Window,
    ) {
        let host = Rc::downgrade(&self.0);
        window.on_present(move || {
            if let Some(host) = host.upgrade() {
                host.borrow().sync(bounds, mask);
            }
        });
    }

    pub(crate) fn set_scrolls(&self, scrolls: bool) {
        self.0.borrow().input.ivars().scrolls.set(scrolls);
    }

    pub(crate) fn eval(&self, script: &str) {
        unsafe {
            self.0
                .borrow()
                .view
                .evaluateJavaScript_completionHandler(&NSString::from_str(script), None);
        }
    }

    pub(crate) fn arm(&self, position: Point<Pixels>) {
        let host = self.0.borrow();
        if host.painted()
            && let Some((_, visible)) = host.placed.get()
            && visible.contains(&position)
        {
            set_armed(&host.input, &host.key, true, "pointer");
        }
    }

    pub(crate) fn painted(&self) -> bool {
        self.0.borrow().painted()
    }

    pub(crate) fn snapshot(&self) {
        let host = self.0.borrow();
        if !host.painted()
            || host.snapshotting.load(Ordering::Acquire)
            || host
                .last_snapshot
                .get()
                .is_some_and(|at| at.elapsed() < FRAME_INTERVAL)
        {
            return;
        }
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        host.snapshotting.store(true, Ordering::Release);
        host.last_snapshot.set(Some(Instant::now()));
        if debug() {
            tracing::info!(page = %host.key, "inline page snapshot");
        }
        let busy = host.snapshotting.clone();
        let key = host.key.clone();
        let tx = host.tx.clone();
        let done = block2::RcBlock::new(move |image: *mut NSImage, _error: *mut NSError| {
            match unsafe { image.as_ref() }.and_then(cg_image) {
                Some((image, points_width)) => convert(FrameJob {
                    image,
                    points_width,
                    key: key.clone(),
                    tx: tx.clone(),
                    busy: busy.clone(),
                }),
                None => busy.store(false, Ordering::Release),
            }
        });
        unsafe {
            let config = WKSnapshotConfiguration::new(mtm);
            host.view
                .takeSnapshotWithConfiguration_completionHandler(Some(&config), &done);
        }
    }
}

fn page_signal(body: &str) -> Option<InlineEvent> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if value.get("active").is_some() {
        return Some(InlineEvent::Active);
    }
    reported_height(&value).map(InlineEvent::Height)
}

fn reported_height(value: &serde_json::Value) -> Option<f32> {
    let height = value.get("height")?.as_f64()? as f32;
    (height.is_finite() && height > 0.0).then(|| height.min(100_000.0))
}

impl Host {
    fn painted(&self) -> bool {
        if self.concealed.get() || self.stamp.get() != tick(self.window) {
            return false;
        }
        // A hidden window presents nothing, so its last stamp never goes stale.
        let shown = self
            .parent
            .window()
            .is_some_and(|window| on_screen(&window));
        if !shown {
            self.missed.store(true, Ordering::Release);
        }
        shown
    }

    fn sync(&self, bounds: Bounds<Pixels>, mask: Bounds<Pixels>) {
        self.stamp.set(tick(self.window));
        let visible = bounds.intersect(&mask);
        if self.placed.get() != Some((bounds, visible)) {
            self.placed.set(Some((bounds, visible)));
            self.place(bounds, visible);
        }
        if !(f32::from(visible.size.width) > 0. && f32::from(visible.size.height) > 0.) {
            set_armed(&self.input, &self.key, false, "covered");
        } else if self.pointer_rests_inside() {
            set_armed(&self.input, &self.key, true, "rest");
        }
        if self.concealed.replace(false) {
            let _ = self.tx.send((self.key.clone(), InlineEvent::Active));
        }
    }

    fn pointer_rests_inside(&self) -> bool {
        let Some(window) = self.parent.window() else {
            return false;
        };
        if !window.isKeyWindow() || NSEvent::pressedMouseButtons() != 0 {
            return false;
        }
        let in_window = window.convertPointFromScreen(NSEvent::mouseLocation());
        let point = self.parent.convertPoint_fromView(in_window, None);
        contains(self.input.ivars().region.get(), point)
    }

    fn place(&self, bounds: Bounds<Pixels>, visible: Bounds<Pixels>) {
        let actions_disabled: bool = unsafe {
            let previous = msg_send![class!(CATransaction), disableActions];
            let _: () = msg_send![class!(CATransaction), setDisableActions: true];
            previous
        };
        self.input.setFrame(self.parent.bounds());
        self.input.ivars().region.set(self.rect(visible));
        self.view.setFrame(self.rect(bounds));
        unsafe {
            // Left open for the matching Metal presentation, like browser tabs.
            let _: () = msg_send![class!(CATransaction), setDisableActions: actions_disabled];
        }
        if debug() {
            self.debug_hit(visible);
        }
    }

    fn conceal(&self) {
        if self.concealed.replace(true) {
            return;
        }
        if has_focus(&self.view) {
            let _ = self.web.focus_parent();
        }
        set_armed(&self.input, &self.key, false, "unpainted");
        self.input.ivars().region.set(NSRect::ZERO);
        self.placed.set(None);
    }

    /// GPUI window pixels (top-left origin) to the parent's coordinates.
    fn rect(&self, bounds: Bounds<Pixels>) -> NSRect {
        let x = f64::from(f32::from(bounds.origin.x));
        let y = f64::from(f32::from(bounds.origin.y));
        let width = f64::from(f32::from(bounds.size.width)).max(0.0);
        let height = f64::from(f32::from(bounds.size.height)).max(0.0);
        let y = if self.parent.isFlipped() {
            y
        } else {
            self.parent.bounds().size.height - y - height
        };
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }

    fn debug_hit(&self, visible: Bounds<Pixels>) {
        let region = self.input.ivars().region.get();
        let center = NSPoint::new(
            region.origin.x + region.size.width / 2.0,
            region.origin.y + region.size.height / 2.0,
        );
        let armed = self.input.ivars().armed.replace(true);
        let hit = self.parent.window().and_then(|window| {
            let in_window = self.parent.convertPoint_toView(center, None);
            let root = window
                .contentView()
                .and_then(|c| unsafe { c.superview() })?;
            let point = root.convertPoint_fromView(in_window, None);
            root.hitTest(point)
                .map(|v| v.class().name().to_string_lossy().into_owned())
        });
        self.input.ivars().armed.set(armed);
        tracing::info!(
            page = %self.key,
            ?visible,
            ?region,
            parent_bounds = ?self.parent.bounds(),
            flipped = self.parent.isFlipped(),
            web_frame = ?self.view.frame(),
            ?hit,
            "inline page placement"
        );
    }
}

// Runs before AppKit hit-tests the event, so disarming here routes it to GPUI.
fn input_monitor(
    input: Retained<PageInput>,
    parent: Retained<NSView>,
    view: Retained<WKWebView>,
    clicked: Arc<AtomicU64>,
    key: SharedString,
    tx: InlineSender,
) -> Option<Retained<AnyObject>> {
    let last_pointer: Cell<Option<Instant>> = Cell::new(None);
    let callback = block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| {
        let e = unsafe { event.as_ref() };
        let kind = e.r#type();
        let state = input.ivars();
        if kind == NSEventType::MouseMoved && !state.armed.get() {
            return event.as_ptr();
        }
        let Some(window) = parent.window() else {
            return event.as_ptr();
        };
        // Mouse-moved events often carry no window; trust the screen location.
        let in_window = window.convertPointFromScreen(NSEvent::mouseLocation());
        let point = parent.convertPoint_fromView(in_window, None);
        let inside = contains(state.region.get(), point);
        let same_window = e.windowNumber() == window.windowNumber();
        let pointer = || {
            if last_pointer
                .get()
                .is_none_or(|at| at.elapsed() >= POINTER_INTERVAL)
            {
                last_pointer.set(Some(Instant::now()));
                let _ = tx.send((key.clone(), InlineEvent::Pointer));
            }
        };
        match kind {
            NSEventType::MouseMoved if inside => pointer(),
            NSEventType::MouseMoved => set_armed(&input, &key, false, "left"),
            NSEventType::ScrollWheel if state.armed.get() => {
                if inside && state.scrolls.get() && same_window {
                    pointer();
                } else {
                    set_armed(&input, &key, false, "wheel");
                }
            }
            NSEventType::LeftMouseDown
            | NSEventType::RightMouseDown
            | NSEventType::OtherMouseDown => {
                // GPUI's view never takes first responder on click, so the page
                // would keep eating Space and arrows typed into the composer.
                if !(state.armed.get() && inside) {
                    set_armed(&input, &key, false, "press");
                    if same_window && has_focus(&view) {
                        window.makeFirstResponder(Some(&parent));
                    }
                }
            }
            NSEventType::LeftMouseUp if same_window && state.armed.get() && inside => {
                clicked.store(now_ms(), Ordering::Relaxed);
            }
            _ => {}
        }
        event.as_ptr()
    });
    unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::MouseMoved
                | NSEventMask::ScrollWheel
                | NSEventMask::LeftMouseDown
                | NSEventMask::RightMouseDown
                | NSEventMask::OtherMouseDown
                | NSEventMask::LeftMouseUp,
            &callback,
        )
    }
}

fn on_screen(window: &NSWindow) -> bool {
    window
        .occlusionState()
        .contains(NSWindowOcclusionState::Visible)
        && !window.isMiniaturized()
}

// A window can show again without presenting, so no sync would re-stamp its pages.
fn shown_observers(
    window: Option<Retained<NSWindow>>,
    missed: &Arc<AtomicBool>,
    key: &SharedString,
    tx: &InlineSender,
) -> Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>> {
    let Some(window) = window else {
        return Vec::new();
    };
    let center = NSNotificationCenter::defaultCenter();
    let names = unsafe {
        [
            NSWindowDidChangeOcclusionStateNotification,
            NSWindowDidDeminiaturizeNotification,
        ]
    };
    names
        .into_iter()
        .map(|name| {
            let (missed, key, tx) = (missed.clone(), key.clone(), tx.clone());
            let block = block2::RcBlock::new(move |notification: NonNull<NSNotification>| {
                let shown = unsafe { notification.as_ref() }
                    .object()
                    .and_then(|object| object.downcast::<NSWindow>().ok())
                    .is_some_and(|window| on_screen(&window));
                if shown && missed.swap(false, Ordering::AcqRel) {
                    let _ = tx.send((key.clone(), InlineEvent::Active));
                }
            });
            let object: &AnyObject = &window;
            unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(name),
                    Some(object),
                    Some(&NSOperationQueue::mainQueue()),
                    &block,
                )
            }
        })
        .collect()
}

fn has_focus(view: &NSView) -> bool {
    view.window()
        .and_then(|w| w.firstResponder())
        .and_then(|r| r.downcast::<NSView>().ok())
        .is_some_and(|v| v.isDescendantOf(view))
}

impl Drop for Host {
    fn drop(&mut self) {
        if has_focus(&self.view) {
            let _ = self.web.focus_parent();
        }
        self.input.ivars().armed.set(false);
        unsafe {
            if let Some(monitor) = self.monitor.take() {
                NSEvent::removeMonitor(&monitor);
            }
            let center = NSNotificationCenter::defaultCenter();
            for observer in self.observers.drain(..) {
                center.removeObserver(observer.as_ref());
            }
            self.view.setNavigationDelegate(None);
            self.view.stopLoading();
        }
        self.view.removeFromSuperview();
        self.input.removeFromSuperview();
    }
}

struct FrameJob {
    image: core_graphics::image::CGImage,
    points_width: f32,
    key: SharedString,
    tx: InlineSender,
    busy: Arc<AtomicBool>,
}

// SAFETY: a CGImage is immutable and safe to read from any thread.
unsafe impl Send for FrameJob {}

fn cg_image(image: &NSImage) -> Option<(core_graphics::image::CGImage, f32)> {
    use foreign_types::ForeignType;
    let points = image.size();
    let cg: *mut std::ffi::c_void = unsafe {
        msg_send![
            image,
            CGImageForProposedRect: std::ptr::null_mut::<NSRect>(),
            context: std::ptr::null_mut::<AnyObject>(),
            hints: std::ptr::null_mut::<AnyObject>()
        ]
    };
    if cg.is_null() || points.width <= 0.0 {
        return None;
    }
    // The NSImage owns its CGImage; the conversion holds its own reference.
    Some(unsafe {
        core_foundation::base::CFRetain(cg as *const _);
        (
            core_graphics::image::CGImage::from_ptr(cg.cast()),
            points.width as f32,
        )
    })
}

fn convert(job: FrameJob) {
    static WORKER: OnceLock<Option<std::sync::mpsc::Sender<FrameJob>>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (jobs, queue) = std::sync::mpsc::channel::<FrameJob>();
        std::thread::Builder::new()
            .name("inline-page-frames".into())
            .spawn(move || {
                for job in queue {
                    if let Some((image, scale)) = bgra_frame(&job.image, job.points_width) {
                        let _ = job.tx.send((job.key, InlineEvent::Frame(image, scale)));
                    }
                    job.busy.store(false, Ordering::Release);
                }
            })
            .ok()
            .map(|_| jobs)
    });
    let refused = match worker {
        Some(worker) => worker.send(job).err().map(|error| error.0),
        None => Some(job),
    };
    if let Some(job) = refused {
        job.busy.store(false, Ordering::Release);
    }
}

fn bgra_frame(
    image: &core_graphics::image::CGImage,
    points_width: f32,
) -> Option<(Arc<RenderImage>, f32)> {
    use core_graphics::base::{kCGBitmapByteOrder32Little, kCGImageAlphaPremultipliedFirst};
    use core_graphics::color_space::CGColorSpace;
    use core_graphics::context::CGContext;
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};

    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return None;
    }
    let mut context = CGContext::create_bitmap_context(
        None,
        width,
        height,
        8,
        width * 4,
        &CGColorSpace::create_device_rgb(),
        kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little,
    );
    context.draw_image(
        CGRect::new(
            &CGPoint::new(0.0, 0.0),
            &CGSize::new(width as f64, height as f64),
        ),
        image,
    );
    let stride = context.bytes_per_row();
    let mut pixels = Vec::with_capacity(width * height * 4);
    for row in context.data().chunks_exact(stride).take(height) {
        pixels.extend_from_slice(&row[..width * 4]);
    }
    unpremultiply(&mut pixels);
    let buffer = image::RgbaImage::from_raw(width as u32, height as u32, pixels)?;
    Some((
        Arc::new(RenderImage::new([image::Frame::new(buffer)])),
        width as f32 / points_width,
    ))
}

// CoreGraphics only draws premultiplied; GPUI blends straight-alpha BGRA.
fn unpremultiply(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decide(url: &str, main: bool, link: bool, loaded: &Cell<bool>) -> (bool, Option<String>) {
        let mut opened = None;
        let allow = page_navigation(url, main, link, loaded, |url| opened = Some(url));
        (allow, opened)
    }

    #[test]
    fn the_page_loads_once_and_links_leave_for_the_browser() {
        let loaded = Cell::new(false);
        assert_eq!(decide("about:blank", true, false, &loaded), (true, None));
        assert_eq!(
            decide("about:blank#top", true, false, &loaded),
            (true, None)
        );
        assert_eq!(
            decide("https://example.com/", true, true, &loaded),
            (false, Some("https://example.com/".into()))
        );
        assert_eq!(
            decide("https://example.com/", true, false, &loaded),
            (false, None)
        );
        assert_eq!(decide("about:blank", true, false, &loaded), (false, None));
        assert_eq!(
            decide("file:///etc/passwd", true, true, &loaded),
            (false, None)
        );
    }

    #[test]
    fn a_first_load_that_is_not_the_page_is_refused() {
        let loaded = Cell::new(false);
        assert_eq!(
            decide("https://evil.test/", true, false, &loaded),
            (false, None)
        );
        assert_eq!(decide("about:blank", true, false, &loaded), (false, None));
    }

    #[test]
    fn embedded_frames_load_but_never_from_disk() {
        let loaded = Cell::new(true);
        assert_eq!(
            decide("https://www.youtube.com/embed/x", false, false, &loaded),
            (true, None)
        );
        assert_eq!(decide("about:srcdoc", false, false, &loaded), (true, None));
        assert_eq!(
            decide("file:///Users/x/secret", false, false, &loaded),
            (false, None)
        );
    }

    #[test]
    fn heights_are_finite_positive_numbers_and_activity_is_its_own_signal() {
        let height = |body: &str| match page_signal(body) {
            Some(InlineEvent::Height(height)) => Some(height),
            _ => None,
        };
        assert_eq!(height(r#"{"height":412}"#), Some(412.0));
        assert_eq!(height(r#"{"height":-3}"#), None);
        assert_eq!(height(r#"{"height":"9"}"#), None);
        assert_eq!(height("nope"), None);
        assert_eq!(height(r#"{"height":1e12}"#), Some(100_000.0));
        assert!(matches!(
            page_signal(r#"{"active":1}"#),
            Some(InlineEvent::Active)
        ));
    }

    #[test]
    fn frames_lose_their_premultiplied_alpha() {
        let mut pixels = [64, 32, 0, 128, 10, 20, 30, 255, 9, 9, 9, 0];
        unpremultiply(&mut pixels);
        assert_eq!(pixels, [128, 64, 0, 128, 10, 20, 30, 255, 9, 9, 9, 0]);
    }
}
