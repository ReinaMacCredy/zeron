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

    const BUNDLE: &str = include_str!("annotate.generated.js");
    const TEARDOWN: &str =
        "(() => { globalThis.__zeronAnnotate?.teardown(); return JSON.stringify(null); })()";

    fn start_script(theme: &crate::theme::Theme) -> String {
        let css = |color: gpui::Hsla| {
            let c = gpui::Rgba::from(color);
            format!(
                "rgba({}, {}, {}, {})",
                (c.r * 255.0).round(),
                (c.g * 255.0).round(),
                (c.b * 255.0).round(),
                c.a
            )
        };
        let opaque = |color: gpui::Hsla| css(gpui::Hsla { a: 1.0, ..color });
        let theme = serde_json::json!({
            "colorScheme": if theme.appearance.is_dark() { "dark" } else { "light" },
            "radius": "10px",
            "background": opaque(theme.bg),
            "foreground": css(theme.text),
            "popover": opaque(theme.surface_dialog),
            "popoverForeground": css(theme.text),
            "primary": opaque(theme.accent),
            "primaryForeground": css(theme.on_accent),
            "muted": css(theme.element_hover),
            "mutedForeground": css(theme.text_muted),
            "accent": css(theme.element_active),
            "accentForeground": css(theme.text),
            "border": css(theme.border),
            "input": css(theme.border_strong),
            "ring": opaque(theme.accent),
            "fontSans": "-apple-system, BlinkMacSystemFont, system-ui, sans-serif",
            "fontMono": "ui-monospace, \"SF Mono\", Menlo, monospace",
        });
        format!(
            "(() => {{ if (!globalThis.__zeronAnnotate) {{ {BUNDLE} }} return globalThis.__zeronAnnotate.start({theme}); }})()"
        )
    }

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
            let picked = native.automation_eval(&start_script(crate::theme::Theme::of(cx)), true);
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
                drop(native.automation_eval(TEARDOWN, true));
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
                for (label, script) in DEMO_STEPS {
                    wait(1500).await;
                    let _ = this.update(cx, |this, _| {
                        if let Some(native) = &this.native {
                            drop(native.automation_eval(script, true));
                        }
                    });
                    tracing::info!("annotate-demo: {label}");
                    wait(6000).await;
                }
            })
            .detach();
        }
    }

    const DEMO_STEPS: [(&str, &str); 3] = [
        (
            "overlay",
            r##"(() => {
  const target = document.querySelector("[data-annotate-demo]");
  const r = target.getBoundingClientRect();
  const at = { bubbles: true, composed: true, button: 0, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 };
  target.dispatchEvent(new PointerEvent("pointermove", at));
  target.dispatchEvent(new PointerEvent("pointerdown", at));
  target.dispatchEvent(new PointerEvent("pointerup", at));
  const ui = [...document.documentElement.children].find((node) => node.shadowRoot).shadowRoot;
  const comment = ui.querySelector("textarea");
  comment.value = "Make this the primary call to action";
  comment.dispatchEvent(new Event("input", { bubbles: true }));
  const next = document.querySelector("#share").getBoundingClientRect();
  document.querySelector("#share").dispatchEvent(new PointerEvent("pointermove", { ...at, clientX: next.left + 10, clientY: next.top + 10 }));
  return JSON.stringify(null);
})()"##,
        ),
        (
            "styled",
            r##"(() => {
  const ui = [...document.documentElement.children].find((node) => node.shadowRoot).shadowRoot;
  ui.querySelector('[aria-label="Expand annotation editor"]').click();
  const size = [...ui.querySelectorAll("label")].find((label) => label.textContent.startsWith("Font size")).querySelector("input");
  size.value = "20";
  size.dispatchEvent(new Event("input", { bubbles: true }));
  return JSON.stringify(null);
})()"##,
        ),
        (
            "attached",
            r##"(() => {
  const ui = [...document.documentElement.children].find((node) => node.shadowRoot).shadowRoot;
  [...ui.querySelectorAll("button")].find((button) => button.textContent === "Attach").click();
  return JSON.stringify(null);
})()"##,
        ),
    ];
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
