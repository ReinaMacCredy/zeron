use super::*;
use crate::browser::{
    automation::{Action, Request},
    control_host::{Envelope, failure, success},
};
use serde_json::json;

impl Shell {
    pub(crate) fn native_browser_tabs(&self, thread: &str, cx: &App) -> Vec<serde_json::Value> {
        let selected = self.resolved_right_active(cx);
        self.right_tabs
            .get(thread)
            .into_iter()
            .flatten()
            .filter_map(|tab| {
                let RightSurface::Browser(id) = tab else {
                    return None;
                };
                let browser = self.browsers.get(id)?;
                let mut value = browser.read(cx).automation_description();
                value["selected"] =
                    json!(self.active_chat == thread && selected == RightSurface::Browser(*id));
                Some(value)
            })
            .collect()
    }

    pub(crate) fn disable_browser_diagnostics(&mut self, cx: &mut Context<Self>) {
        for browser in self.browsers.values() {
            browser.update(cx, |browser, _| browser.disable_diagnostics());
        }
    }

    pub(crate) fn owns_browser_task(&self, thread: &str, request: &Request, cx: &App) -> bool {
        if thread.is_empty() {
            return false;
        }
        if request.action == Action::Open {
            return self.active_chat == thread;
        }
        if request.action == Action::List && self.active_chat == thread {
            return true;
        }
        self.right_tabs.get(thread).is_some_and(|tabs| {
            tabs.iter().any(|tab| match tab {
                RightSurface::Browser(id) => {
                    request.action == Action::List
                        || self.browsers.get(id).is_some_and(|browser| {
                            Some(browser.read(cx).control.id.as_str()) == request.tab_id.as_deref()
                        })
                }
                _ => false,
            })
        })
    }

    pub(crate) fn release_browser_control(&mut self, thread: Option<&str>, cx: &mut Context<Self>) {
        self.change_browser_control(thread, false, cx);
    }

    pub(crate) fn revoke_browser_control(&mut self, thread: Option<&str>, cx: &mut Context<Self>) {
        self.change_browser_control(thread, true, cx);
    }

    fn change_browser_control(
        &mut self,
        thread: Option<&str>,
        takeover: bool,
        cx: &mut Context<Self>,
    ) {
        let ids: Vec<_> = self
            .right_tabs
            .iter()
            .filter(|(owner, _)| thread.is_none_or(|thread| thread == owner.as_str()))
            .flat_map(|(_, tabs)| {
                tabs.iter().filter_map(|tab| {
                    if let RightSurface::Browser(id) = tab {
                        Some(*id)
                    } else {
                        None
                    }
                })
            })
            .collect();
        for id in ids {
            if let Some(browser) = self.browsers.get(&id) {
                browser.update(cx, |browser, cx| {
                    if takeover {
                        if browser.control.used {
                            browser.take_over(cx);
                        }
                    } else {
                        browser.release_control(cx);
                    }
                });
            }
        }
    }

    pub(crate) fn dispatch_browser_control(
        &mut self,
        envelope: Envelope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if envelope.interrupted() {
            let _ = envelope
                .reply
                .send(failure("Browser command cancelled before dispatch."));
            return;
        }
        if crate::computer::protected_surfaces::any_showing() {
            let _ = envelope.reply.send(failure(
                "A protected Zeron dialog is open; browser input is paused.",
            ));
            return;
        }
        let thread = envelope.call.task.as_ref().unwrap().thread_id.clone();
        let request = &envelope.request;
        let owned: Vec<_> = self
            .right_tabs
            .get(&thread)
            .into_iter()
            .flatten()
            .filter_map(|tab| {
                if let RightSurface::Browser(id) = tab {
                    self.browsers.get(id).map(|browser| (*id, browser.clone()))
                } else {
                    None
                }
            })
            .collect();
        if request.action == Action::List {
            let _ = envelope.reply.send(success(
                json!({"tabs":self.native_browser_tabs(&thread,cx)}),
            ));
            return;
        }
        if request.action == Action::Open {
            if self.active_chat != thread
                || self.state.read(cx).selected_chat_row().is_none_or(|chat| {
                    Some(chat.device_id.as_str()) != self.state.read(cx).local_device_id.as_deref()
                })
            {
                let _ = envelope.reply.send(failure(
                    "Open the local task before creating its browser tab.",
                ));
                return;
            }
            let url = request
                .url
                .as_deref()
                .and_then(|url| crate::browser::model::normalize_address(url).ok());
            let Some(url) = url.filter(|url| {
                url::Url::parse(url).is_ok_and(|url| crate::browser::model::loopback(&url))
            }) else {
                let _ = envelope.reply.send(failure(
                    "open requires an http(s) localhost or loopback URL.",
                ));
                return;
            };
            let Some(browser) = self.new_browser_surface(window, cx) else {
                let _ = envelope
                    .reply
                    .send(failure("No local browser surface available."));
                return;
            };
            self.set_surfaces_open(true, cx);
            browser.update(cx, |browser, cx| {
                browser.control.used = true;
                browser.navigate(&url, window, cx)
            });
            let _ = envelope
                .reply
                .send(success(browser.read(cx).automation_description()));
            return;
        }
        let browser = owned.into_iter().find_map(|(_, browser)| {
            (Some(browser.read(cx).control.id.as_str()) == request.tab_id.as_deref())
                .then_some(browser)
        });
        let Some(browser) = browser else {
            let _ = envelope.reply.send(failure(
                "Tab does not belong to this task or was closed. List this task's tabs.",
            ));
            return;
        };
        if let Err(error) = browser
            .read(cx)
            .control
            .check(request.control_token.as_deref())
        {
            let _ = envelope.reply.send(failure(&error));
            return;
        }
        let developer = envelope.developer;
        if request.action == Action::Navigate {
            let url = request
                .url
                .as_deref()
                .and_then(|url| crate::browser::model::normalize_address(url).ok());
            let Some(url) = url else {
                let _ = envelope
                    .reply
                    .send(failure("navigate requires an http(s) URL."));
                return;
            };
            browser.update(cx, |browser, cx| {
                browser.control.used = true;
                browser.navigate(&url, window, cx)
            });
            let mut value = browser.read(cx).automation_description();
            value["effect"] = json!("dispatched-unknown");
            let _ = envelope.reply.send(success(value));
            return;
        }
        let receiver = browser.update(cx, |browser, cx| {
            browser.dispatch_automation(request, developer, cx)
        });
        match receiver {
            Err(error) => {
                let _ = envelope.reply.send(failure(&error));
            }
            Ok(receiver) => {
                let tab = browser.read(cx).automation_description();
                cx.spawn(async move |_, _| {
                    let reply = match receiver.await {
                        Ok(Ok(mut value)) => {
                            if let Some(object) = value.as_object_mut() {
                                object.insert("tab".into(), tab);
                            } else {
                                value = json!({"value":value,"tab":tab});
                            }
                            success(value)
                        }
                        Ok(Err(error)) => failure(&format!(
                            "Browser action returned an error; inspect before retrying: {error}"
                        )),
                        Err(_) => failure(
                            "Browser page closed before completion; inspect before retrying.",
                        ),
                    };
                    let _ = envelope.reply.send(reply);
                })
                .detach();
            }
        }
    }
}

#[cfg(feature = "browser-fixture")]
impl Shell {
    pub fn fixture_native_browser_call(
        &mut self,
        args: serde_json::Value,
        developer: bool,
        cancelled: bool,
        thread: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> tokio::sync::oneshot::Receiver<zeron_proto::computer::CuaReply> {
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.dispatch_browser_control(
            Envelope {
                call: zeron_proto::computer::ComputerHostCall {
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                    method: zeron_proto::computer::ComputerHostMethod::Call,
                    name: Some("zeron_browser".into()),
                    args: args.clone(),
                    task: Some(zeron_proto::computer::CuaComputerTask {
                        thread_id: thread.unwrap_or_else(|| self.active_chat.clone()),
                        turn_id: None,
                        label: None,
                    }),
                    delivery_mode: None,
                    model_observation: None,
                },
                request: serde_json::from_value(args).unwrap(),
                developer,
                monitor: None,
                admission: None,
                cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(cancelled)),
                reply,
            },
            window,
            cx,
        );
        receiver
    }
    pub fn fixture_browser_takeover(&mut self, cx: &mut Context<Self>) {
        if let Some((_, browser)) = self.fixture_active_browser(cx) {
            browser.update(cx, |b, cx| b.take_over(cx));
        }
    }
    pub fn fixture_browser_resume(&mut self, cx: &mut Context<Self>) {
        if let Some((_, browser)) = self.fixture_active_browser(cx) {
            browser.update(cx, |b, cx| b.resume_control(cx));
        }
    }
}
