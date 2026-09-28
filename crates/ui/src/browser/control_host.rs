use super::automation::{Action, Request};
use crate::{shell::Shell, state::EngineHandle};
use gpui::{AnyWindowHandle, App, WeakEntity};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use zeron_proto::computer::{
    ComputerHostCall, ComputerHostMethod, ComputerSettings, CuaContent, CuaReply, CuaToolResult,
};
use zeron_rpc::{ComputerHost, methods};

pub(crate) struct Envelope {
    pub call: ComputerHostCall,
    pub request: Request,
    pub developer: bool,
    pub monitor: Option<Arc<crate::computer::native::escape_monitor::EscapeKillSwitchMonitor>>,
    pub admission: Option<(
        Arc<super::control_admission::Admission>,
        super::control_admission::Stamp,
    )>,
    pub cancelled: Arc<AtomicBool>,
    pub reply: tokio::sync::oneshot::Sender<CuaReply>,
}
impl Envelope {
    pub fn interrupted(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
            || self.reply.is_closed()
            || self
                .admission
                .as_ref()
                .is_some_and(|(gate, stamp)| !gate.current(stamp))
            || self
                .monitor
                .as_ref()
                .is_some_and(|monitor| !monitor.state().ready)
    }
}

pub(crate) enum Message {
    Call(Envelope),
    Revoke(Option<String>),
    Release(Option<String>),
}
#[derive(Default)]
struct Registry {
    developer_blocked: bool,
    shells: Vec<(WeakEntity<Shell>, AnyWindowHandle)>,
}
impl gpui::Global for Registry {}

pub(crate) fn set_developer_blocked(blocked: bool, cx: &mut App) {
    if !cx.has_global::<Registry>() {
        return;
    }
    cx.global_mut::<Registry>().developer_blocked = blocked;
    if blocked {
        let shells = cx.global::<Registry>().shells.clone();
        for (shell, _) in shells {
            let _ = shell.update(cx, |shell, cx| shell.disable_browser_diagnostics(cx));
        }
    }
}

pub(crate) fn register(shell: WeakEntity<Shell>, window: AnyWindowHandle, cx: &mut App) {
    if !cx.has_global::<Registry>() {
        return;
    }
    let registry = cx.global_mut::<Registry>();
    registry.shells.retain(|(old, _)| old.upgrade().is_some());
    if !registry.shells.iter().any(|(old, _)| old == &shell) {
        registry.shells.push((shell, window));
    }
}

pub(crate) fn install(cx: &mut App) -> tokio::sync::mpsc::UnboundedSender<Message> {
    cx.set_global(Registry::default());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    cx.spawn(async move |cx| {
        while let Some(message) = rx.recv().await {
            let _ = cx.update(|cx| {
                let shells = cx.global::<Registry>().shells.clone();
                match message {
                    Message::Revoke(thread) => for (shell, _) in shells {
                        let _ = shell.update(cx, |shell, cx| shell.revoke_browser_control(thread.as_deref(), cx));
                    },
                    Message::Release(thread) => for (shell, _) in shells {
                        let _ = shell.update(cx, |shell,cx| shell.release_browser_control(thread.as_deref(), cx));
                    },
                    Message::Call(mut envelope) => {
                        envelope.developer &= !cx.global::<Registry>().developer_blocked;
                        let thread = envelope.call.task.as_ref().map(|t| t.thread_id.as_str()).unwrap_or("");
                        if envelope.request.action == Action::List {
                            if envelope.interrupted() {
                                let _ = envelope.reply.send(failure("Browser command was interrupted before dispatch."));
                                return;
                            }
                            let mut tabs = Vec::new();
                            for (shell, window) in &shells {
                                if let Some(shell) = shell.upgrade() {
                                    for mut tab in shell.read(cx).native_browser_tabs(thread, cx) {
                                        tab["window_active"] = json!(cx.active_window() == Some(*window));
                                        tabs.push(tab);
                                    }
                                }
                            }
                            let _ = envelope.reply.send(success(json!({"tabs":tabs})));
                            return;
                        }
                        let matches: Vec<_> = shells.into_iter().filter(|(shell, _)| {
                            shell.upgrade().is_some_and(|shell| shell.read(cx).owns_browser_task(thread, &envelope.request, cx))
                        }).collect();
                        if matches.len() != 1 {
                            let _ = envelope.reply.send(failure("Open the task in exactly one Zeron window before controlling its browser."));
                            return;
                        }
                        let (shell, window) = matches.into_iter().next().unwrap();
                        let _ = window.update(cx, |_, window, cx| {
                            let _ = shell.update(cx, |shell, cx| shell.dispatch_browser_control(envelope, window, cx));
                        });
                    }
                }
            });
        }
    }).detach();
    tx
}

pub(crate) fn failure(message: &str) -> CuaReply {
    CuaReply {
        ok: true,
        result: Some(CuaToolResult {
            is_error: Some(true),
            content: Some(vec![CuaContent {
                kind: "text".into(),
                text: Some(message.into()),
                data: None,
                mime_type: None,
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}
pub(crate) fn success(mut value: Value) -> CuaReply {
    let mut content = vec![];
    if let Some(image) = value
        .as_object_mut()
        .and_then(|v| v.remove("image"))
        .and_then(|v| v.as_str().map(str::to_owned))
    {
        content.push(CuaContent {
            kind: "image".into(),
            text: None,
            data: Some(image),
            mime_type: Some("image/png".into()),
        });
    }
    content.push(CuaContent {
        kind: "text".into(),
        text: Some(value.to_string()),
        data: None,
        mime_type: None,
    });
    CuaReply {
        ok: true,
        result: Some(CuaToolResult {
            content: Some(content),
            structured_content: value.as_object().cloned(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub(crate) struct Host {
    pub driver: Arc<dyn ComputerHost>,
    pub admission: Arc<super::control_admission::Admission>,
    pub monitor: Arc<crate::computer::native::escape_monitor::EscapeKillSwitchMonitor>,
    pub sender: tokio::sync::mpsc::UnboundedSender<Message>,
    pub engine: Arc<Mutex<Option<EngineHandle>>>,
    pub pending: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub cancelled: Mutex<VecDeque<String>>,
}
#[async_trait::async_trait]
impl ComputerHost for Host {
    async fn request(&self, call: ComputerHostCall) -> CuaReply {
        if matches!(
            call.method,
            ComputerHostMethod::Stop
                | ComputerHostMethod::EndTask
                | ComputerHostMethod::EndBrowserThread
        ) {
            self.admission
                .stop(call.task.as_ref().map(|task| task.thread_id.as_str()));
        }
        if call.method == ComputerHostMethod::Stop {
            let _ = self.sender.send(Message::Revoke(
                call.task.as_ref().map(|task| task.thread_id.clone()),
            ));
        } else if matches!(
            call.method,
            ComputerHostMethod::EndTask | ComputerHostMethod::EndBrowserThread
        ) {
            let _ = self.sender.send(Message::Release(
                call.task.as_ref().map(|t| t.thread_id.clone()),
            ));
        }
        if call.method != ComputerHostMethod::Call || call.name.as_deref() != Some("zeron_browser")
        {
            return self.driver.request(call).await;
        }
        let request: Request = match serde_json::from_value(call.args.clone()) {
            Ok(request) => request,
            Err(error) => return failure(&format!("Invalid browser request: {error}")),
        };
        if call.task.as_ref().is_none_or(|t| t.thread_id.is_empty()) {
            return failure("A live task identity is required.");
        }
        if let Err(error) = request.validate() {
            return failure(&error);
        }
        let stamp = match self.admission.begin(&call.task.as_ref().unwrap().thread_id) {
            Ok(stamp) => stamp,
            Err(error) => return failure(error),
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let id = call.correlation_id.clone();
        self.pending
            .lock()
            .unwrap()
            .insert(id.clone(), cancelled.clone());
        if self.cancelled.lock().unwrap().contains(&id) {
            cancelled.store(true, Ordering::SeqCst);
        }
        self.monitor.activate().await;
        if !self.monitor.state().ready {
            self.pending.lock().unwrap().remove(&id);
            self.admission
                .stop(call.task.as_ref().map(|task| task.thread_id.as_str()));
            return failure(
                "Input Monitoring must be available so Escape can stop native browser control.",
            );
        }
        let engine = self.engine.lock().unwrap().clone();
        let developer = if let Some(engine) = engine {
            engine
                .client()
                .call_as::<ComputerSettings>(methods::GET_COMPUTER_SETTINGS, json!({}))
                .await
                .is_ok_and(|s| s.browser_developer_access)
        } else {
            false
        };
        let result = if request.action.developer() && !developer {
            failure("Developer access is off in Settings > Computer.")
        } else {
            let (reply, answer) = tokio::sync::oneshot::channel();
            if self
                .sender
                .send(Message::Call(Envelope {
                    call,
                    request,
                    developer,
                    monitor: Some(self.monitor.clone()),
                    admission: Some((self.admission.clone(), stamp)),
                    cancelled: cancelled.clone(),
                    reply,
                }))
                .is_err()
            {
                failure("Zeron browser host is unavailable.")
            } else {
                match tokio::time::timeout(std::time::Duration::from_secs(20), answer).await {
                    Ok(Ok(reply)) => reply,
                    _ => {
                        cancelled.store(true, Ordering::SeqCst);
                        failure(
                            "Browser response unavailable; an action may have been dispatched. Inspect before retrying.",
                        )
                    }
                }
            }
        };
        self.pending.lock().unwrap().remove(&id);
        result
    }
    fn cancel(&self, correlation_id: &str) {
        {
            let mut cancelled = self.cancelled.lock().unwrap();
            cancelled.push_back(correlation_id.to_owned());
            while cancelled.len() > 256 {
                cancelled.pop_front();
            }
        }
        if let Some(cancelled) = self.pending.lock().unwrap().get(correlation_id) {
            cancelled.store(true, Ordering::SeqCst);
        }
        self.driver.cancel(correlation_id);
    }
}
