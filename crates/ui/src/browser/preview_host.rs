//! `html_preview`: the engine hands the desktop app a page; a windowless
//! WKWebView (the engine that shows it in the thread) lays it out at the
//! requested width and returns a PNG, the content height and the console.
use std::cell::{Cell, RefCell};
use std::time::Duration;

use base64::Engine as _;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSError, NSObject, NSObjectProtocol, NSString};
use objc2_web_kit::{
    WKFrameInfo, WKMediaCaptureType, WKNavigation, WKNavigationAction, WKNavigationActionPolicy,
    WKNavigationDelegate, WKPermissionDecision, WKSecurityOrigin, WKUIDelegate, WKUserScript,
    WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration,
};
use serde_json::json;
use zeron_proto::html_render::{
    ConsoleMessage, HtmlPreview, HtmlPreviewRequest, MAX_CAPTURE_HEIGHT, PreviewAppearance,
    PreviewInspect, PreviewRect, PreviewShot,
};
use zeron_rpc::{RpcError, methods};

use crate::state::EngineHandle;
use crate::theme::Theme;

const RETRY_MIN: Duration = Duration::from_millis(500);
const RETRY_MAX: Duration = Duration::from_secs(30);
const LOAD_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_PNG_WIDTH: u32 = 1600;
/// A real origin for the page: with none, WebKit reports every uncaught
/// error as "Script error." with no message or stack.
const PREVIEW_ORIGIN: &str = "https://preview.zeron.invalid/";

/// Records the page's console from its first script on, for the agent.
const CONSOLE_SCRIPT: &str = r#"(function(){var out=[];window.__zeronConsole=out;function push(l,a){try{out.push({level:l,text:Array.prototype.map.call(a,function(x){try{return typeof x==="string"?x:(x instanceof Error?(String(x)+(x.stack?"\n"+x.stack:"")):JSON.stringify(x))}catch(e){return String(x)}}).join(" ").slice(0,2000)});if(out.length>200)out.shift()}catch(e){}}["log","info","warn","error"].forEach(function(k){var o=console[k];console[k]=function(){push(k==="warn"?"warning":k,arguments);return o.apply(console,arguments)}});window.addEventListener("error",function(e){push("error",[(e.message||"Error")+((e.error&&e.error.stack)?"\n"+e.error.stack:" at "+e.filename+":"+e.lineno)])});window.addEventListener("unhandledrejection",function(e){push("error",["Unhandled rejection: "+((e.reason&&e.reason.stack)||e.reason)])});})();"#;

const MEASURE_SCRIPT: &str = r#"if (document.fonts) { await document.fonts.ready; } await new Promise(function (r) { setTimeout(r, 50); }); var r = document.documentElement; return JSON.stringify({ height: Math.ceil(r.scrollHeight > r.clientHeight ? r.scrollHeight : r.getBoundingClientRect().height), console: window.__zeronConsole || [] });"#;

const SETTLE_SCRIPT: &str =
    r#"await new Promise(function (r) { setTimeout(r, 50); }); return "1";"#;

type Job = (
    HtmlPreviewRequest,
    std::time::Instant,
    tokio::sync::oneshot::Sender<Result<HtmlPreview, String>>,
);
/// The engine stops waiting at 30 seconds; a request queued longer is
/// answered without rendering it.
const STALE_AFTER: Duration = Duration::from_secs(25);

/// Serves previews for as long as the engine is attached.
pub(crate) fn spawn(engine: EngineHandle, cx: &gpui::App) -> gpui::Task<()> {
    let (jobs, mut queue) = tokio::sync::mpsc::unbounded_channel::<Job>();
    cx.spawn(async move |cx| {
        while let Some((request, queued, reply)) = queue.recv().await {
            if queued.elapsed() > STALE_AFTER {
                let _ = reply.send(Err("The preview waited too long behind others.".into()));
                continue;
            }
            let (theme, font_size) = cx.update(|cx| {
                (
                    theme_for(Theme::of(cx), request.appearance),
                    crate::typography::font_size(cx).pixels(),
                )
            });
            let html =
                crate::html_render::inject_bootstrap(&request.html, &theme, font_size, false);
            let executor = cx.background_executor().clone();
            let timeout = executor.timer(LOAD_TIMEOUT);
            let _ = reply
                .send(render(&html, request.width, &request.inspect, &executor, timeout).await);
        }
    })
    .detach();
    let executor = cx.background_executor().clone();
    cx.spawn(async move |_| serve(engine, jobs, executor).await)
}

fn theme_for(current: &Theme, appearance: PreviewAppearance) -> Theme {
    let dark = appearance == PreviewAppearance::Dark;
    if current.appearance.is_dark() == dark {
        current.clone()
    } else if dark {
        Theme::dark()
    } else {
        Theme::light()
    }
}

async fn serve(
    engine: EngineHandle,
    jobs: tokio::sync::mpsc::UnboundedSender<Job>,
    executor: gpui::BackgroundExecutor,
) {
    let mut backoff = RETRY_MIN;
    loop {
        match engine
            .client()
            .subscribe_checked(methods::WATCH_HTML_PREVIEW_HOST, json!({}))
            .await
        {
            Ok(mut items) => {
                backoff = RETRY_MIN;
                while let Some(value) = items.recv().await {
                    let Ok(request) = serde_json::from_value::<HtmlPreviewRequest>(value) else {
                        continue;
                    };
                    let (reply, answer) = tokio::sync::oneshot::channel();
                    let id = request.id.clone();
                    if jobs
                        .send((request, std::time::Instant::now(), reply))
                        .is_err()
                    {
                        return;
                    }
                    let engine = engine.clone();
                    executor
                        .spawn(async move {
                            let result = answer
                                .await
                                .unwrap_or_else(|_| Err("The preview was dropped.".into()));
                            let params = match result {
                                Ok(preview) => json!({ "id": id, "preview": preview }),
                                Err(error) => json!({ "id": id, "error": error }),
                            };
                            if let Err(error) = engine
                                .client()
                                .call(methods::RESPOND_HTML_PREVIEW, params)
                                .await
                            {
                                tracing::debug!(%error, "html preview reply dropped");
                            }
                        })
                        .detach();
                }
            }
            Err(RpcError::UnknownMethod(_) | RpcError::Closed) => return,
            Err(error) => tracing::debug!(%error, "html preview host registration failed"),
        }
        executor.timer(backoff).await;
        backoff = (backoff * 2).min(RETRY_MAX);
    }
}

struct PolicyState {
    loaded: Cell<bool>,
    done: RefCell<Option<futures::channel::oneshot::Sender<Result<(), String>>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ZeronHtmlPreviewPolicy"]
    #[ivars = PolicyState]
    struct Policy;
    unsafe impl NSObjectProtocol for Policy {}
    unsafe impl WKUIDelegate for Policy {
        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn media(
            &self,
            _view: &WKWebView,
            _origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            _kind: WKMediaCaptureType,
            decision: &block2::Block<dyn Fn(WKPermissionDecision)>,
        ) {
            decision.call((WKPermissionDecision::Deny,));
        }
    }
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
            let allow = if main {
                !self.ivars().loaded.replace(true) && url == PREVIEW_ORIGIN
            } else {
                !url.starts_with("file:")
            };
            decision.call((if allow {
                WKNavigationActionPolicy::Allow
            } else {
                WKNavigationActionPolicy::Cancel
            },));
        }
        #[unsafe(method(webView:didFinishNavigation:))]
        fn finish(&self, _view: &WKWebView, _navigation: Option<&WKNavigation>) {
            self.settle(Ok(()));
        }
        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn provisional_error(
            &self,
            _view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            self.settle(Err(error.localizedDescription().to_string()));
        }
        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn navigation_error(
            &self,
            _view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            self.settle(Err(error.localizedDescription().to_string()));
        }
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn terminated(&self, _view: &WKWebView) {
            self.settle(Err("The page's web process stopped while loading.".into()));
        }
    }
);

impl Policy {
    fn settle(&self, result: Result<(), String>) {
        if let Some(done) = self.ivars().done.borrow_mut().take() {
            let _ = done.send(result);
        }
    }
}

fn frame(width: u32, height: u32) -> objc2_foundation::NSRect {
    objc2_foundation::NSRect::new(
        objc2_foundation::NSPoint::new(0.0, 0.0),
        objc2_foundation::NSSize::new(f64::from(width), f64::from(height)),
    )
}

async fn render(
    html: &str,
    width: u32,
    inspect: &PreviewInspect,
    executor: &gpui::BackgroundExecutor,
    timeout: impl std::future::Future<Output = ()>,
) -> Result<HtmlPreview, String> {
    let mtm = MainThreadMarker::new().ok_or("Previews run on the main thread")?;
    let (done, loaded) = futures::channel::oneshot::channel();
    let policy: Retained<Policy> = {
        let object = mtm.alloc().set_ivars(PolicyState {
            loaded: Cell::new(false),
            done: RefCell::new(Some(done)),
        });
        unsafe { msg_send![super(object), init] }
    };
    let view = unsafe {
        let configuration = WKWebViewConfiguration::new(mtm);
        configuration
            .preferences()
            .setJavaScriptCanOpenWindowsAutomatically(false);
        configuration.setWebsiteDataStore(
            &objc2_web_kit::WKWebsiteDataStore::nonPersistentDataStore(mtm),
        );
        let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
            mtm.alloc(),
            &NSString::from_str(CONSOLE_SCRIPT),
            WKUserScriptInjectionTime::AtDocumentStart,
            true,
        );
        configuration.userContentController().addUserScript(&script);
        let view =
            WKWebView::initWithFrame_configuration(mtm.alloc(), frame(width, 600), &configuration);
        view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*policy)));
        view.setUIDelegate(Some(ProtocolObject::from_ref(&*policy)));
        let origin = objc2_foundation::NSURL::URLWithString(&NSString::from_str(PREVIEW_ORIGIN));
        view.loadHTMLString_baseURL(&NSString::from_str(html), origin.as_deref());
        view
    };
    let work = async {
        loaded
            .await
            .map_err(|_| "The page did not load.".to_string())??;
        let measured = eval(&view, MEASURE_SCRIPT).await?;
        let content_height = measured["height"].as_f64().unwrap_or(0.0).max(1.0).round() as u32;
        let console_messages: Vec<ConsoleMessage> =
            serde_json::from_value(measured["console"].clone()).unwrap_or_default();
        let captured_height = content_height.clamp(1, MAX_CAPTURE_HEIGHT);
        view.setFrame(frame(width, captured_height));
        eval(&view, SETTLE_SCRIPT).await?;
        let page = crate::ui_driver::web::Preview {
            view: &view,
            size: (width as f32, captured_height as f32),
            executor,
        };
        let crop = match (inspect.rect, &inspect.select) {
            (Some(rect), _) => Some(rect),
            (None, Some(select)) => {
                let [x, y, w, h] = page.bounds(select, inspect.nth).await?;
                Some(PreviewRect { x, y, w, h })
            }
            (None, None) => None,
        }
        .map(|rect| clip(rect, width, captured_height))
        .transpose()?;
        let tree = match inspect.tree {
            true => Some(page.tree().await?),
            false => None,
        };
        let png = snapshot(&view, width, crop).await?;
        let mut steps = Vec::new();
        for step in &inspect.steps {
            let shot = match page.step(step).await {
                Ok(()) => {
                    eval(&view, SETTLE_SCRIPT).await?;
                    PreviewShot {
                        step: step.clone(),
                        png: Some(snapshot(&view, width, crop).await?),
                        tree: Some(page.tree().await?),
                        error: None,
                    }
                }
                Err(error) => PreviewShot {
                    step: step.clone(),
                    png: None,
                    tree: None,
                    error: Some(error),
                },
            };
            let failed = shot.error.is_some();
            steps.push(shot);
            if failed {
                break;
            }
        }
        Ok(HtmlPreview {
            width,
            content_height,
            captured_height,
            console_messages,
            missing_images: Vec::new(),
            png,
            tree,
            crop,
            steps,
        })
    };
    futures::pin_mut!(work, timeout);
    let result = match futures::future::select(work, timeout).await {
        futures::future::Either::Left((result, _)) => result,
        futures::future::Either::Right(_) => {
            Err("The page did not finish within 20 seconds; check its scripts.".into())
        }
    };
    unsafe {
        view.setNavigationDelegate(None);
        view.setUIDelegate(None);
        view.stopLoading();
    }
    result
}

async fn eval(view: &WKWebView, script: &str) -> Result<serde_json::Value, String> {
    crate::ui_driver::web::evaluate(view, script, true)
        .await
        .map_err(|_| "The page did not answer.".to_string())?
}

fn clip(rect: PreviewRect, width: u32, height: u32) -> Result<PreviewRect, String> {
    let x = rect.x.min(f64::from(width));
    let y = rect.y.min(f64::from(height));
    let w = (rect.x + rect.w).min(f64::from(width)) - x;
    let h = (rect.y + rect.h).min(f64::from(height)) - y;
    if w < 1.0 || h < 1.0 {
        return Err(format!(
            "the crop lies outside the captured {width}x{height} page"
        ));
    }
    Ok(PreviewRect { x, y, w, h })
}

async fn snapshot(
    view: &WKWebView,
    width: u32,
    crop: Option<PreviewRect>,
) -> Result<String, String> {
    let (tx, rx) = futures::channel::oneshot::channel();
    let tx = RefCell::new(Some(tx));
    let completion = block2::RcBlock::new(
        move |image: *mut objc2_app_kit::NSImage, error: *mut NSError| {
            let Some(tx) = tx.borrow_mut().take() else {
                return;
            };
            let tiff = (|| {
                if let Some(error) = unsafe { error.as_ref() } {
                    return Err(error.localizedDescription().to_string());
                }
                let image = unsafe { image.as_ref() }.ok_or("The screenshot came back empty.")?;
                let data = image
                    .TIFFRepresentation()
                    .ok_or("The screenshot came back empty.")?;
                Ok(unsafe { data.as_bytes_unchecked() }.to_vec())
            })();
            let _ = tx.send(tiff);
        },
    );
    let configuration = crop.and_then(|rect| {
        let mtm = MainThreadMarker::new()?;
        let configuration = unsafe { objc2_web_kit::WKSnapshotConfiguration::new(mtm) };
        unsafe {
            configuration.setRect(objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(rect.x, rect.y),
                objc2_foundation::NSSize::new(rect.w, rect.h),
            ))
        };
        Some(configuration)
    });
    unsafe {
        view.takeSnapshotWithConfiguration_completionHandler(configuration.as_deref(), &completion)
    };
    let tiff = rx
        .await
        .map_err(|_| "The screenshot was dropped.".to_string())??;
    let (tx, rx) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(encode_png(&tiff, width));
    });
    rx.await
        .map_err(|_| "The screenshot was dropped.".to_string())?
}

/// Retina snapshots come back at 2x; anything wider than 1600px is scaled
/// down so the image stays within what agents read well.
fn encode_png(tiff: &[u8], width: u32) -> Result<String, String> {
    let image = image::load_from_memory(tiff).map_err(|e| e.to_string())?;
    let limit = MAX_PNG_WIDTH.max(width);
    let image = if image.width() > limit {
        image.resize(limit, u32::MAX, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let mut png = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(png.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_retina_captures_scale_down_and_stay_png() {
        let wide = image::DynamicImage::new_rgba8(3200, 100);
        let mut tiff = std::io::Cursor::new(Vec::new());
        wide.write_to(&mut tiff, image::ImageFormat::Tiff).unwrap();
        let png = base64::engine::general_purpose::STANDARD
            .decode(encode_png(tiff.get_ref(), 1600).unwrap())
            .unwrap();
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!(decoded.width(), 1600);
        assert!(png.starts_with(b"\x89PNG"));
    }

    #[test]
    fn crops_stay_inside_the_captured_page() {
        let rect = |x, y, w, h| PreviewRect { x, y, w, h };
        assert_eq!(
            clip(rect(700.0, 10.0, 200.0, 50.0), 800, 600).unwrap(),
            rect(700.0, 10.0, 100.0, 50.0)
        );
        assert!(clip(rect(10.0, 700.0, 20.0, 20.0), 800, 600).is_err());
    }

    #[test]
    fn previews_follow_the_requested_appearance() {
        let dark = Theme::dark();
        assert!(
            theme_for(&dark, PreviewAppearance::Dark)
                .appearance
                .is_dark()
        );
        assert!(
            !theme_for(&dark, PreviewAppearance::Light)
                .appearance
                .is_dark()
        );
    }
}
