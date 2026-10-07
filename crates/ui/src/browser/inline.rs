//! Live agent HTML pages inside the transcript. Each card owns a WKWebView
//! laid over its placeholder every presented frame. Rows scrolled out of the
//! list are never painted, so a window-level sweep hides any page whose card
//! did not sync during the frame being presented.
use super::macos::{BrowserClipView, attach_clip, place};
use super::model::allowed_navigation;
use gpui::{Bounds, IntoElement, Pixels, SharedString, Styled, Window};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSEventMask, NSView};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
    WKNavigationType, WKWebView,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use wry::{WebView, WebViewBuilderExtMacos, WebViewExtMacOS};

pub(crate) enum InlineEvent {
    Height(f32),
    OpenLink(String),
    Crashed,
}

pub(crate) type InlineSender = tokio::sync::mpsc::UnboundedSender<(SharedString, InlineEvent)>;

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

struct Host {
    web: WebView,
    view: Retained<WKWebView>,
    _policy: Retained<Policy>,
    clip: Retained<BrowserClipView>,
    clip_mask: Retained<AnyObject>,
    parent: Retained<NSView>,
    bounds: Option<Bounds<Pixels>>,
    visible: Option<Bounds<Pixels>>,
    window: u64,
    stamp: Cell<u64>,
    scrolls: Rc<Cell<bool>>,
    monitor: Option<Retained<AnyObject>>,
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

/// Painted last in a window's root: hides every page whose card was not
/// painted in this frame (scrolled out, chat switched, route left).
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
                            host.hide();
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
        let height_tx = tx.clone();
        let height_key = key.clone();
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
                if let Some(height) = reported_height(request.body()) {
                    let _ = height_tx.send((height_key.clone(), InlineEvent::Height(height)));
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
        let view = Retained::into_super(web.webview());
        let (parent, clip, clip_mask) = attach_clip(window, &view, mtm)?;
        let policy: Retained<Policy> = {
            let object = mtm.alloc().set_ivars(PolicyState {
                key: key.clone(),
                tx,
                loaded: Cell::new(false),
                clicked: clicked.clone(),
            });
            unsafe { msg_send![super(object), init] }
        };
        unsafe { view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*policy))) };

        let scrolls = Rc::new(Cell::new(false));
        let monitor_clip = clip.clone();
        let monitor_parent = parent.clone();
        let monitor_scrolls = scrolls.clone();
        // A page that fits its frame has nothing to scroll; the wheel belongs
        // to the transcript, or the thread stops scrolling under the pointer.
        let callback = block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| {
            let e = unsafe { event.as_ref() };
            if monitor_clip.isHidden() || e.window(mtm) != monitor_parent.window() {
                return event.as_ptr();
            }
            let point = monitor_parent.convertPoint_fromView(e.locationInWindow(), None);
            let region = monitor_clip.ivars().region.get();
            let inside = point.x >= region.origin.x
                && point.x < region.origin.x + region.size.width
                && point.y >= region.origin.y
                && point.y < region.origin.y + region.size.height;
            if !inside {
                return event.as_ptr();
            }
            if e.r#type() == objc2_app_kit::NSEventType::LeftMouseUp {
                clicked.store(now_ms(), Ordering::Relaxed);
                return event.as_ptr();
            }
            if monitor_scrolls.get() {
                return event.as_ptr();
            }
            unsafe {
                let _: () = msg_send![&*monitor_parent, scrollWheel: e];
            }
            std::ptr::null_mut()
        });
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::ScrollWheel | NSEventMask::LeftMouseUp,
                &callback,
            )
        };
        let host = Rc::new(RefCell::new(Host {
            web,
            view,
            _policy: policy,
            clip,
            clip_mask,
            parent,
            bounds: None,
            visible: None,
            window: window.window_handle().window_id().as_u64(),
            stamp: Cell::new(u64::MAX),
            scrolls,
            monitor,
        }));
        host.borrow()
            .web
            .load_html(html)
            .map_err(|e| e.to_string())?;
        HOSTS.with(|hosts| hosts.borrow_mut().push(Rc::downgrade(&host)));
        Ok(Self(host))
    }

    /// Paint-phase registration: places the page over `bounds`, clipped to
    /// `mask`, when this frame is presented.
    pub(crate) fn sync_on_present(
        &self,
        bounds: Bounds<Pixels>,
        mask: Bounds<Pixels>,
        window: &mut Window,
    ) {
        let host = Rc::downgrade(&self.0);
        window.on_present(move || {
            if let Some(host) = host.upgrade() {
                host.borrow_mut().sync(bounds, mask);
            }
        });
    }

    pub(crate) fn set_scrolls(&self, scrolls: bool) {
        self.0.borrow().scrolls.set(scrolls);
    }

    pub(crate) fn eval(&self, script: &str) {
        unsafe {
            self.0
                .borrow()
                .view
                .evaluateJavaScript_completionHandler(&NSString::from_str(script), None);
        }
    }
}

fn reported_height(body: &str) -> Option<f32> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let height = value.get("height")?.as_f64()? as f32;
    (height.is_finite() && height > 0.0).then(|| height.min(100_000.0))
}

impl Host {
    fn sync(&mut self, bounds: Bounds<Pixels>, mask: Bounds<Pixels>) {
        self.stamp.set(tick(self.window));
        let visible = bounds.intersect(&mask);
        if self.bounds != Some(bounds) || self.visible != Some(visible) {
            self.bounds = Some(bounds);
            self.visible = Some(visible);
            place(
                &self.parent,
                &self.clip,
                &self.clip_mask,
                &self.view,
                bounds,
                visible,
            );
        }
        let shown = f32::from(visible.size.width) > 0. && f32::from(visible.size.height) > 0.;
        if shown {
            self.show();
        } else {
            self.hide();
        }
    }

    fn show(&self) {
        if self.view.isHidden() {
            let _ = self.web.set_visible(true);
        }
        self.clip.setHidden(false);
    }

    fn hide(&self) {
        if self.clip.isHidden() {
            return;
        }
        if has_focus(&self.view) {
            let _ = self.web.focus_parent();
        }
        let _ = self.web.set_visible(false);
        self.clip.setHidden(true);
    }
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
        unsafe {
            if let Some(monitor) = self.monitor.take() {
                NSEvent::removeMonitor(&monitor);
            }
            self.view.setNavigationDelegate(None);
            self.view.stopLoading();
        }
        self.view.removeFromSuperview();
        self.clip.removeFromSuperview();
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
    fn heights_are_finite_positive_numbers() {
        assert_eq!(reported_height(r#"{"height":412}"#), Some(412.0));
        assert_eq!(reported_height(r#"{"height":-3}"#), None);
        assert_eq!(reported_height(r#"{"height":"9"}"#), None);
        assert_eq!(reported_height("nope"), None);
        assert_eq!(reported_height(r#"{"height":1e12}"#), Some(100_000.0));
    }
}
