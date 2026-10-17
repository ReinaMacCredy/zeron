use super::BrowserSurface;
use gpui::Context;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub action: Action,
    pub tab_id: Option<String>,
    pub control_token: Option<String>,
    pub url: Option<String>,
    #[serde(rename = "ref")]
    pub element: Option<String>,
    pub text: Option<String>,
    pub script: Option<String>,
    pub delta_y: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    List,
    Open,
    State,
    Navigate,
    Click,
    Type,
    Scroll,
    Screenshot,
    Evaluate,
    Diagnostics,
}
impl Action {
    pub fn developer(self) -> bool {
        matches!(self, Self::Evaluate | Self::Diagnostics)
    }
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.text.as_ref().is_some_and(|s| s.len() > 16384)
            || self.script.as_ref().is_some_and(|s| s.len() > 16384)
            || self
                .delta_y
                .is_some_and(|n| !n.is_finite() || n.abs() > 4096.0)
        {
            return Err("Browser input exceeds its bounds.".into());
        }
        Ok(())
    }
}

pub(crate) struct Control {
    pub id: String,
    token: String,
    pub paused: bool,
    pub used: bool,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            token: uuid::Uuid::new_v4().to_string(),
            paused: false,
            used: false,
        }
    }
}
impl Control {
    pub fn revoke(&mut self) {
        self.paused = true;
        self.token = uuid::Uuid::new_v4().to_string();
    }
    pub fn resume(&mut self) {
        self.paused = false;
        self.token = uuid::Uuid::new_v4().to_string();
    }
    pub fn check(&self, token: Option<&str>) -> Result<(), String> {
        if self.paused {
            return Err(
                "The user took over this tab. Only the user can resume agent control.".into(),
            );
        }
        if token != Some(self.token.as_str()) {
            return Err("Stale or missing control_token; list this task's tabs again.".into());
        }
        Ok(())
    }
}
impl BrowserSurface {
    pub(crate) fn automation_description(&self) -> Value {
        json!({"tab_id":self.control.id, "control_token":if self.control.paused {None} else {Some(&self.control.token)},
            "url":self.page.url,"title":self.page.title,"paused":self.control.paused,"loading":self.page.loading})
    }
    pub(crate) fn release_control(&mut self, cx: &mut Context<Self>) {
        self.disable_diagnostics();
        self.control.token = uuid::Uuid::new_v4().to_string();
        self.control.used = false;
        cx.notify();
    }
    pub(crate) fn take_over(&mut self, cx: &mut Context<Self>) {
        self.disable_diagnostics();
        self.control.revoke();
        self.annotation.took_over();
        cx.notify();
    }
    pub(crate) fn resume_control(&mut self, cx: &mut Context<Self>) {
        self.control.resume();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn takeover_invalidates_queued_commands_even_after_resume() {
        let mut control = Control::default();
        let old = control.token.clone();
        assert!(control.check(Some(&old)).is_ok());
        control.revoke();
        assert!(control.check(Some(&old)).is_err());
        control.resume();
        assert!(control.check(Some(&old)).is_err());
        assert!(control.check(Some(&control.token)).is_ok());
    }
    #[test]
    fn caller_cannot_supply_authority_fields() {
        assert!(
            serde_json::from_value::<Request>(json!({"action":"evaluate","developer_access":true}))
                .is_err()
        );
    }
}

impl BrowserSurface {
    pub(crate) fn disable_diagnostics(&self) {
        if let Some(native) = &self.native {
            drop(native.automation_eval(
                "(() => { globalThis.__zeronStopDiagnostics?.(); return JSON.stringify(null); })()",
                true,
            ));
        }
    }

    pub(crate) fn dispatch_automation(
        &mut self,
        request: &Request,
        developer: bool,
        cx: &mut Context<Self>,
    ) -> Result<futures::channel::oneshot::Receiver<Result<Value, String>>, String> {
        self.control.check(request.control_token.as_deref())?;
        request.validate()?;
        if request.action.developer() && !developer {
            return Err("Developer access is off in Settings > Computer.".into());
        }
        let native = self.native.as_ref().ok_or("Browser page is not ready")?;
        if native.state().loading && !matches!(request.action, Action::State | Action::Screenshot) {
            return Err("Navigation is still loading; observe the new page before acting.".into());
        }
        if request.action == Action::State {
            native.automation_snapshot();
        }
        if matches!(request.action, Action::Click | Action::Type)
            && !native.automation_refs_current()
        {
            return Err("Navigation invalidated refs; read state again.".into());
        }
        self.control.used = true;
        let result = match request.action {
            Action::Screenshot => native.automation_screenshot(),
            Action::Evaluate => {
                let script = request
                    .script
                    .as_deref()
                    .ok_or("evaluate requires script")?;
                let script = format!(
                    "(async () => {{ const value = await (0,eval)({}); return JSON.stringify(value === undefined ? null : value); }})()",
                    serde_json::to_string(script).unwrap()
                );
                native.automation_eval(&script, true)
            }
            Action::Diagnostics => native.automation_eval(include_str!("diagnostics.js"), true),
            Action::State | Action::Click | Action::Type | Action::Scroll => {
                if matches!(request.action, Action::Click | Action::Type)
                    && request.element.is_none()
                {
                    return Err("Action requires a current element ref".into());
                }
                if request.action == Action::Type && request.text.is_none() {
                    return Err("type requires text".into());
                }
                if request.action == Action::Scroll && request.delta_y.is_none() {
                    return Err("scroll requires delta_y".into());
                }
                let action = match request.action {
                    Action::State => "state",
                    Action::Click => "click",
                    Action::Type => "type",
                    _ => "scroll",
                };
                let args = json!({"action":action,"ref":request.element,"text":request.text,"delta_y":request.delta_y,"snapshot_id":uuid::Uuid::new_v4().to_string()});
                native.automation_eval(
                    &include_str!("automation.js").replace("__REQUEST__", &args.to_string()),
                    false,
                )
            }
            _ => return Err("Unsupported page action".into()),
        };
        cx.notify();
        Ok(result)
    }
}
