use crate::appshots::CapturedAppshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AnnotationState {
    #[default]
    Idle,
    Annotating {
        was_paused: bool,
    },
    Capturing {
        was_paused: bool,
    },
}

impl AnnotationState {
    pub fn active(self) -> bool {
        self != Self::Idle
    }

    pub fn begin(paused: bool) -> Self {
        Self::Annotating { was_paused: paused }
    }

    pub fn took_over(&mut self) {
        if let Self::Annotating { was_paused } | Self::Capturing { was_paused } = self {
            *was_paused = true;
        }
    }

    pub fn capturing(&mut self) {
        if let Self::Annotating { was_paused } = *self {
            *self = Self::Capturing { was_paused };
        }
    }

    pub fn end(&mut self) -> bool {
        let resume = matches!(
            *self,
            Self::Annotating { was_paused: false } | Self::Capturing { was_paused: false }
        );
        *self = Self::Idle;
        resume
    }
}

#[derive(Clone)]
pub struct AnnotationCapture {
    pub appshot: Option<CapturedAppshot>,
    pub draft: String,
    pub send: bool,
}

impl std::fmt::Debug for AnnotationCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnnotationCapture")
            .field("appshot", &self.appshot.as_ref().map(|shot| &shot.id))
            .field("draft", &self.draft)
            .field("send", &self.send)
            .finish()
    }
}

#[cfg(target_os = "macos")]
mod host {
    use super::{AnnotationCapture, AnnotationState};
    use crate::browser::{BrowserEvent, BrowserSurface};
    use gpui::Context;
    use serde::Deserialize;

    const SPIKE: &str = r#"(() => {
  const g = globalThis;
  if (!g.__zeronAnnotate) {
    let done = null, box = null, off = null;
    const finish = (value) => { off?.(); off = null; const d = done; done = null; d?.(JSON.stringify(value)); };
    g.__zeronAnnotate = {
      start() {
        finish(null);
        return new Promise((resolve) => {
          done = resolve;
          const down = (event) => {
            event.preventDefault();
            event.stopPropagation();
            const target = document.elementFromPoint(event.clientX, event.clientY);
            if (!target) return;
            const r = target.getBoundingClientRect();
            box = document.createElement("div");
            box.style.cssText = `position:fixed;left:${r.left}px;top:${r.top}px;width:${r.width}px;height:${r.height}px;border:2px solid #2563eb;box-sizing:border-box;pointer-events:none;z-index:2147483646`;
            document.documentElement.appendChild(box);
            const x = Math.max(0, r.left - 20), y = Math.max(0, r.top - 20);
            finish({
              annotation: { pageUrl: location.href, pageTitle: document.title || null, comment: "" },
              cropRect: { x, y, width: Math.min(innerWidth - x, r.width + 40), height: Math.min(innerHeight - y, r.height + 40) },
              submission: "attach",
            });
          };
          window.addEventListener("pointerdown", down, true);
          off = () => window.removeEventListener("pointerdown", down, true);
        });
      },
      cancel() { finish(null); this.teardown(); },
      teardown() { box?.remove(); box = null; },
    };
  }
  return g.__zeronAnnotate.start();
})()"#;
    const CANCEL: &str =
        "(() => { globalThis.__zeronAnnotate?.cancel(); return JSON.stringify(null); })()";
    const TEARDOWN: &str =
        "(() => { globalThis.__zeronAnnotate?.teardown(); return JSON.stringify(null); })()";

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Picked {
        annotation: PickedAnnotation,
        crop_rect: Rect,
        submission: Submission,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PickedAnnotation {
        page_url: String,
        page_title: Option<String>,
        comment: String,
    }

    #[derive(Debug, Clone, Copy, Deserialize)]
    struct Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum Submission {
        Attach,
        Send,
    }

    impl BrowserSurface {
        pub(crate) fn annotation_available(&self) -> bool {
            self.native.is_some() && self.page.url.is_some() && self.page.error.is_none()
        }

        pub(crate) fn toggle_annotation(&mut self, cx: &mut Context<Self>) {
            if self.annotation.active() {
                self.cancel_annotation(cx);
            } else {
                self.start_annotation(cx);
            }
        }

        pub(crate) fn start_annotation(&mut self, cx: &mut Context<Self>) {
            if self.annotation.active() || !self.annotation_available() {
                return;
            }
            let Some(native) = &self.native else {
                return;
            };
            let picked = native.automation_eval(SPIKE, true);
            native.focus_page();
            let was_paused = self.control.paused;
            self.take_over(cx);
            self.annotation = AnnotationState::begin(was_paused);
            self.annotation_task = Some(cx.spawn(async move |this, cx| {
                let picked = picked
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .and_then(|value| serde_json::from_value::<Option<Picked>>(value).ok())
                    .flatten();
                let Some(picked) = picked else {
                    let _ = this.update(cx, |this, cx| this.end_annotation(cx));
                    return;
                };
                let rect = picked.crop_rect;
                let Ok(Some(snapshot)) = this.update(cx, |this, cx| {
                    this.annotation.capturing();
                    cx.notify();
                    this.native.as_ref().map(|native| {
                        native.annotation_snapshot([rect.x, rect.y, rect.width, rect.height])
                    })
                }) else {
                    return;
                };
                let timeout = cx
                    .background_executor()
                    .timer(std::time::Duration::from_secs(5));
                let png = match futures::future::select(snapshot, std::pin::pin!(timeout)).await {
                    futures::future::Either::Left((Ok(Ok(png)), _)) => Some(png),
                    _ => None,
                };
                let _ = this.update(cx, |this, cx| {
                    if let Some(native) = &this.native {
                        drop(native.automation_eval(TEARDOWN, true));
                    }
                    let capture = this.annotation_capture(picked, png);
                    this.end_annotation(cx);
                    cx.emit(BrowserEvent::Annotation(capture));
                });
            }));
            cx.notify();
        }

        pub(crate) fn cancel_annotation(&mut self, cx: &mut Context<Self>) {
            if !self.annotation.active() {
                return;
            }
            if let Some(native) = &self.native {
                drop(native.automation_eval(CANCEL, true));
            }
            self.annotation_task = None;
            self.end_annotation(cx);
        }

        fn end_annotation(&mut self, cx: &mut Context<Self>) {
            if self.annotation.end() && self.control.paused {
                self.resume_control(cx);
            }
            cx.notify();
        }

        fn annotation_capture(&self, picked: Picked, png: Option<Vec<u8>>) -> AnnotationCapture {
            let title = picked
                .annotation
                .page_title
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| picked.annotation.page_url.clone());
            let appshot = png
                .and_then(|png| crate::appshots::stage_appshot_png("Browser", png).ok())
                .map(
                    |(screenshot, dimensions)| crate::appshots::CapturedAppshot {
                        id: uuid::Uuid::new_v4().to_string(),
                        app_name: "Browser".into(),
                        bundle_identifier: None,
                        window_title: Some(title),
                        accessibility: crate::appshots::AccessibilitySnapshot {
                            format_version: 1,
                            content: format!("url: {}", picked.annotation.page_url),
                            truncated: false,
                        },
                        screenshot,
                        screenshot_dimensions: Some(dimensions),
                        app_icon: self.favicon.clone(),
                        window_bounds: None,
                        captured_at: chrono::Utc::now(),
                    },
                );
            AnnotationCapture {
                appshot,
                draft: picked.annotation.comment,
                send: picked.submission == Submission::Send,
            }
        }

        // Synthetic input never reaches a Dev bundle; drive the overlay through page events.
        pub(crate) fn run_annotate_demo(&mut self, cx: &mut Context<Self>) {
            cx.spawn(async move |this, cx| {
                let executor = cx.background_executor().clone();
                let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
                for _ in 0..100 {
                    wait(200).await;
                    let Ok(ready) = this.update(cx, |this, _| {
                        this.annotation_available() && !this.page.loading
                    }) else {
                        return;
                    };
                    if ready {
                        break;
                    }
                }
                tracing::info!("annotate-demo: idle");
                wait(6000).await;
                let _ = this.update(cx, |this, cx| this.start_annotation(cx));
                wait(1000).await;
                let _ = this.update(cx, |this, _| {
                    if let Some(native) = &this.native {
                        drop(native.automation_eval(DEMO_PICK, true));
                    }
                });
                tracing::info!("annotate-demo: picked");
            })
            .detach();
        }
    }

    const DEMO_PICK: &str = r#"(() => {
  const target = document.querySelector("[data-annotate-demo]");
  const r = target.getBoundingClientRect();
  target.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, composed: true, button: 0, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 }));
  return JSON.stringify(null);
})()"#;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_restores_only_the_pause_it_took() {
        let mut state = AnnotationState::begin(false);
        assert!(state.end());
        let mut state = AnnotationState::begin(true);
        assert!(!state.end());
        let mut state = AnnotationState::begin(false);
        state.capturing();
        assert!(state.end());
    }

    #[test]
    fn takeover_during_annotation_is_never_resumed() {
        let mut state = AnnotationState::begin(false);
        state.took_over();
        assert!(!state.end());
        let mut state = AnnotationState::begin(false);
        state.capturing();
        state.took_over();
        assert!(!state.end());
        assert_eq!(state, AnnotationState::Idle);
    }
}
