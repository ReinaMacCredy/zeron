use gpui::{
    AnyElement, App, Context, Entity, Focusable as _, Subscription, Window, div, prelude::*, px,
};
use zeron_proto::{HarnessId, capabilities};

use super::mentions::{MentionMenu, MentionMenuEvent, MentionTarget};
use super::slash::{self, SlashMenu, SlashMenuEvent, SlashScope, SlashTarget};
use super::{ComposerInput, ComposerInputEvent, invocation_token, slash_token};
use crate::pickers::Pickers;
use crate::popover;
use crate::state::AppState;

const WIDTH: f32 = 420.0;
const GAP: f32 = 4.0;
/// Task inputs sit in cards and popovers that are deferred layers themselves.
const LAYER: usize = 2;

pub(crate) struct TaskScope {
    pub(crate) space_id: Option<String>,
    pub(crate) agent: Option<Entity<Pickers>>,
}

pub(crate) struct TaskCompletions {
    app: Entity<AppState>,
    input: Entity<ComposerInput>,
    mentions: Entity<MentionMenu>,
    slash: Entity<SlashMenu>,
    scope: Box<dyn Fn(&App) -> TaskScope>,
    /// Made when a `/` or `$` token first opens; it fetches the harness list.
    default_agent: Option<(Entity<Pickers>, Subscription)>,
    _subscriptions: [Subscription; 3],
}

fn workspace_params(app: &AppState, space_id: Option<&str>) -> Option<serde_json::Value> {
    let space = space_id.and_then(|id| app.space_row(id))?;
    Some(serde_json::json!({
        "spaceId": space.id,
        "targetDeviceId": space.device_id,
        "cwd": space.path,
    }))
}

fn target_device(app: &AppState, space_id: Option<&str>) -> Option<String> {
    space_id
        .and_then(|id| app.space_row(id))
        .map(|space| space.device_id.clone())
        .or_else(|| app.selected_device.clone())
        .or_else(|| app.local_device_id.clone())
}

impl TaskCompletions {
    pub(crate) fn new(
        app: Entity<AppState>,
        input: Entity<ComposerInput>,
        scope: impl Fn(&App) -> TaskScope + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let mentions = cx.new(|_| MentionMenu::new(app.clone(), input.clone()));
        let slash = cx.new(|_| SlashMenu::new(app.clone(), input.clone()));
        let input_events = cx.subscribe(&input, |this, _, event, cx| match event {
            ComposerInputEvent::Edited | ComposerInputEvent::CursorMoved => this.refresh(cx),
            ComposerInputEvent::MentionNavigate(delta) => {
                if this.slash.read(cx).is_open() {
                    this.slash.update(cx, |menu, cx| menu.step(*delta, cx))
                } else {
                    this.mentions.update(cx, |menu, cx| menu.step(*delta, cx))
                }
            }
            ComposerInputEvent::MentionAccept => {
                if this.slash.read(cx).is_open() {
                    this.accept_slash(cx)
                } else {
                    this.mentions.update(cx, |menu, cx| menu.accept(cx))
                }
            }
            ComposerInputEvent::MentionDismiss => {
                if this.slash.read(cx).is_open() {
                    this.slash.update(cx, |menu, cx| menu.dismiss(cx))
                } else {
                    this.mentions.update(cx, |menu, cx| menu.dismiss(cx))
                }
            }
            _ => {}
        });
        let mention_events = cx.subscribe(&mentions, |this, _, _: &MentionMenuEvent, cx| {
            this.changed(cx)
        });
        let slash_events = cx.subscribe(&slash, |this, _, event, cx| match event {
            SlashMenuEvent::Changed => this.changed(cx),
            SlashMenuEvent::Accept => this.accept_slash(cx),
        });
        Self {
            app,
            input,
            mentions,
            slash,
            scope: Box::new(scope),
            default_agent: None,
            _subscriptions: [input_events, mention_events, slash_events],
        }
    }

    pub(crate) fn in_selected_space(
        app: Entity<AppState>,
        input: Entity<ComposerInput>,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = app.clone();
        Self::new(
            app,
            input,
            move |cx| TaskScope {
                space_id: state
                    .read(cx)
                    .selected_space_row()
                    .map(|space| space.id.clone()),
                agent: None,
            },
            cx,
        )
    }

    pub(crate) fn is_open(&self, cx: &App) -> bool {
        self.mentions.read(cx).is_open() || self.slash.read(cx).is_open()
    }

    pub(crate) fn has_selection(&self, cx: &App) -> bool {
        let slash = self.slash.read(cx);
        if slash.is_open() {
            slash.has_selection()
        } else {
            self.mentions.read(cx).has_selection()
        }
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.mentions.update(cx, |menu, cx| menu.reset(None, cx));
        self.slash.update(cx, |menu, cx| menu.reset(None, cx));
    }

    pub(crate) fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.slash.read(cx).is_open() {
            self.slash.update(cx, |menu, cx| menu.dismiss(cx));
        }
        if self.mentions.read(cx).is_open() {
            self.mentions.update(cx, |menu, cx| menu.dismiss(cx));
        }
    }

    pub(crate) fn close_if_blurred(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.is_open(cx) && !self.input.focus_handle(cx).is_focused(window) {
            self.close(cx);
        }
    }

    pub(crate) fn card(&self, cx: &App) -> Option<AnyElement> {
        if self.slash.read(cx).is_open() {
            Some(self.slash.clone().into_any_element())
        } else if self.mentions.read(cx).is_open() {
            Some(self.mentions.clone().into_any_element())
        } else {
            None
        }
    }

    /// The open menu under its input; the input's wrapper must be `relative`.
    pub(crate) fn popup_below(&self, id: &'static str, cx: &App) -> Option<AnyElement> {
        let card = self.card(cx)?;
        Some(popover::anchored_menu_below_layer(
            id,
            div().w(px(WIDTH)).child(card).into_any_element(),
            None,
            GAP,
            LAYER,
        ))
    }

    #[cfg(test)]
    pub(crate) fn seed_slash_rows(
        &self,
        rows: Vec<zeron_proto::command_catalog::InvocationCandidate>,
        cx: &mut App,
    ) {
        self.slash.update(cx, |menu, cx| menu.seed_rows(rows, cx));
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        let (open, selected) = (self.is_open(cx), self.has_selection(cx));
        self.input.update(cx, |input, cx| {
            input.set_mention_controls(open, selected, cx)
        });
        cx.notify();
    }

    fn harness(&mut self, scope: &TaskScope, cx: &mut Context<Self>) -> Option<HarnessId> {
        let agent = match &scope.agent {
            Some(agent) => agent.clone(),
            None => self.default_agent(cx),
        };
        agent.read(cx).resolved(cx).harness
    }

    fn default_agent(&mut self, cx: &mut Context<Self>) -> Entity<Pickers> {
        if let Some((agent, _)) = &self.default_agent {
            return agent.clone();
        }
        let app = self.app.clone();
        let agent = cx.new(|cx| {
            let mut agent = Pickers::new_for_tasks(app, cx);
            agent.ensure_harnesses(false, cx);
            agent
        });
        let changes = cx.observe(&agent, |this, _, cx| {
            if this.slash.read(cx).is_open() {
                this.refresh(cx);
            }
        });
        self.default_agent = Some((agent.clone(), changes));
        agent
    }

    fn slash_target(&mut self, scope: &TaskScope, cx: &mut Context<Self>) -> SlashTarget {
        let harness = self.harness(scope, cx);
        let app = self.app.read(cx);
        let space_id = scope.space_id.as_deref();
        let device = target_device(app, space_id);
        let mut params = workspace_params(app, space_id).unwrap_or_else(|| {
            let mut params = serde_json::json!({});
            if let Some(device) = &device {
                params["targetDeviceId"] = device.as_str().into();
            }
            params
        });
        params["harness"] = serde_json::json!(harness);
        SlashTarget {
            harness,
            params,
            connection: slash::connection_context(app, device.as_deref()),
            scope: SlashScope::Task,
            blocked_compact: None,
        }
    }

    fn mention_target(&self, scope: &TaskScope, cx: &App) -> MentionTarget {
        let app = self.app.read(cx);
        let space_id = scope.space_id.as_deref();
        let workspace = workspace_params(app, space_id);
        MentionTarget {
            context: workspace
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            device: target_device(app, space_id).map(|device| (device, None)),
            space: scope.space_id.clone(),
            workspace,
            attachments: Vec::new(),
            done_tasks: false,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let input = self.input.read(cx);
        if input.selecting() {
            if self.is_open(cx) {
                self.close(cx);
            }
            return;
        }
        let (text, cursor) = (input.text().to_string(), input.cursor_offset());
        let scope = (self.scope)(cx);
        if invocation_token(&text, cursor, '$').is_some() || slash_token(&text, cursor).is_some() {
            let target = self.slash_target(&scope, cx);
            self.slash
                .update(cx, |menu, cx| menu.refresh(&text, cursor, target, cx));
        } else {
            let slash = self.slash.read(cx);
            if slash.is_open() || slash.is_fetching() {
                self.slash.update(cx, |menu, cx| menu.reset(None, cx));
            }
        }
        let target = self.mention_target(&scope, cx);
        self.mentions
            .update(cx, |menu, cx| menu.refresh(&text, cursor, target, cx));
    }

    fn accept_slash(&mut self, cx: &mut Context<Self>) {
        let Some((range, candidate)) = self.slash.read(cx).selected() else {
            return;
        };
        let scope = (self.scope)(cx);
        let app = self.app.read(cx);
        let supported = target_device(app, scope.space_id.as_deref()).is_some_and(|device| {
            app.device_supports(&device, capabilities::COMPOSER_REFERENCES_V1)
        });
        self.slash
            .update(cx, |menu, cx| menu.insert(range, &candidate, supported, cx));
    }
}
