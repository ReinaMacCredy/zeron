use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use gpui::{
    Context, Entity, EventEmitter, Focusable as _, MouseButton, SharedString, Task, Window, div,
    prelude::*, px,
};
use zeron_proto::app_commands::{COMPACT_COMMAND, terminal_only_command};
use zeron_proto::command_catalog::{
    CatalogFailure, InvocationCandidate, catalog_empty_message, catalog_error_message,
    inserts_message_text, invocation_detail, invocation_insertion, skill_display_name,
    with_workspace_commands, with_zeron_commands, zeron_owned_command,
};
use zeron_proto::{HarnessId, SlashCommand};
use zeron_rpc::{RpcError, methods};

use super::{ComposerInput, MentionToken, completion_trigger};
use crate::state::AppState;
use crate::theme::Theme;

pub(super) const PREFETCH_DELAY: Duration = Duration::from_millis(400);

/// Slash-command completion state: like [`super::FileMentionState`] but the
/// candidate list is scoped to device, harness and workspace, then filtered
/// locally per keystroke. Commands and skills share focus and keyboard handling.
#[derive(Debug, Clone, Default)]
pub(super) struct SlashState {
    pub(super) token: Option<MentionToken>,
    /// Indices into the cached command list, filter-ranked for the query.
    pub(super) filtered: Vec<usize>,
    pub(super) active: Option<usize>,
    /// Harness the popup is showing commands for (cache key).
    pub(super) harness: Option<HarnessId>,
    /// Shared identity for the command and skill catalogs. Switching trigger
    /// characters should not invalidate another catalog for the same provider.
    pub(super) catalog_context: String,
    pub(super) context: String,
    pub(super) skill: bool,
    pub(super) supported: bool,
    pub(super) request: u64,
    pub(super) loading: bool,
    pub(super) error: Option<SharedString>,
    pub(super) dismissed: Option<(Range<usize>, String)>,
}

/// Where the picked text goes. A chat composer also offers Zeron's own
/// workspace commands; a task's text only reaches an agent later, as a new
/// chat's first message, so it keeps just the entries that insert text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlashScope {
    Chat { in_chat: bool },
    Task,
}

pub(crate) struct SlashTarget {
    pub(crate) harness: Option<HarnessId>,
    /// `ListCommands` / `ListSkills` params, `harness` included.
    pub(crate) params: serde_json::Value,
    /// Engine, connection and device reachability: a change re-fetches.
    pub(crate) connection: String,
    pub(crate) scope: SlashScope,
    pub(crate) blocked_compact: Option<SharedString>,
}

pub(crate) enum SlashMenuEvent {
    Changed,
    /// A row was clicked: the host runs the same accept as Enter.
    Accept,
}

pub(crate) struct SlashMenu {
    app: Entity<AppState>,
    input: Entity<ComposerInput>,
    pub(super) state: SlashState,
    /// Advertised invocations for the current device/harness/workspace.
    /// Invalidated on context changes; filtering stays local while typing.
    pub(super) cache: HashMap<String, Vec<InvocationCandidate>>,
    fetch: Option<Task<()>>,
    prefetch: Option<(String, Task<()>)>,
    scope: SlashScope,
    blocked_compact: Option<SharedString>,
    scroll: gpui::ScrollHandle,
    bar: crate::popover::MenuScrollbarState,
}

impl EventEmitter<SlashMenuEvent> for SlashMenu {}

fn slash_context(
    skill: bool,
    include_skills: bool,
    commands_allowed: bool,
    catalog_context: &str,
) -> String {
    format!(
        "{}:{include_skills}:{commands_allowed}:{catalog_context}",
        if skill { "skill" } else { "command" },
    )
}

pub(crate) fn connection_context(app: &AppState, device: Option<&str>) -> String {
    let engine = app
        .engine()
        .map(|engine| engine.client() as *const _ as usize);
    let online = device.is_none_or(|device| app.device_online(device, chrono::Utc::now()));
    format!(
        "{engine:?}:{:?}:{:?}:{online}",
        app.connection, app.connectivity.state
    )
}

async fn fetch_invocation_catalog(
    engine: crate::state::EngineHandle,
    params: serde_json::Value,
    harness: Option<HarnessId>,
    skill: bool,
    include_skills: bool,
    commands_allowed: bool,
) -> Result<(Vec<InvocationCandidate>, bool, Option<SharedString>), RpcError> {
    let commands = async {
        if skill || !commands_allowed {
            return Ok(Vec::new());
        }
        let value = engine
            .client()
            .call(methods::LIST_COMMANDS, params.clone())
            .await?;
        serde_json::from_value::<Vec<SlashCommand>>(value)
            .map(|commands| with_zeron_commands(harness, commands))
            .map_err(|e| RpcError::Failed(e.to_string()))
    };
    let skills = async {
        // Even separated slash menus need this metadata to exclude
        // provider commands that are actually skill aliases.
        let value = engine
            .client()
            .call(methods::LIST_SKILLS, params.clone())
            .await?;
        serde_json::from_value::<Option<Vec<zeron_proto::invocation::Skill>>>(value)
            .map_err(|e| RpcError::Failed(e.to_string()))
    };
    let (commands, skills) = futures::join!(commands, skills);
    merge_invocation_results(commands, skills, skill).map(|(mut rows, supported, warning)| {
        if !include_skills {
            rows.retain(|row| row.invocation.prefix() == '/');
        }
        (rows, supported, warning)
    })
}

fn merge_invocation_results(
    commands: Result<Vec<SlashCommand>, RpcError>,
    skills: Result<Option<Vec<zeron_proto::invocation::Skill>>, RpcError>,
    skill_only: bool,
) -> Result<(Vec<InvocationCandidate>, bool, Option<SharedString>), RpcError> {
    zeron_proto::command_catalog::merge_invocation_results(commands, skills, skill_only).map(
        |(rows, supported, warning)| {
            (
                rows,
                supported,
                warning.map(|(error, skill)| slash_error_message(&error, skill)),
            )
        },
    )
}

fn slash_error_message(err: &RpcError, skill: bool) -> SharedString {
    let failure = match err {
        RpcError::UnknownMethod(_) => CatalogFailure::OutdatedEngine,
        RpcError::Transport(_) | RpcError::Closed => CatalogFailure::Unreachable,
        RpcError::BadParams(_) | RpcError::Capability(_) | RpcError::Failed(_) => {
            CatalogFailure::Failed
        }
    };
    catalog_error_message(failure, skill).into()
}

fn scoped(scope: SlashScope, candidates: Vec<InvocationCandidate>) -> Vec<InvocationCandidate> {
    match scope {
        SlashScope::Chat { in_chat } => with_workspace_commands(candidates, in_chat),
        SlashScope::Task => candidates
            .into_iter()
            .filter(inserts_message_text)
            .collect(),
    }
}

impl SlashMenu {
    pub(crate) fn new(app: Entity<AppState>, input: Entity<ComposerInput>) -> Self {
        Self {
            app,
            input,
            state: SlashState::default(),
            cache: HashMap::new(),
            fetch: None,
            prefetch: None,
            scope: SlashScope::Chat { in_chat: false },
            blocked_compact: None,
            scroll: gpui::ScrollHandle::new(),
            bar: crate::popover::MenuScrollbarState::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn seed_rows(&mut self, rows: Vec<InvocationCandidate>, cx: &mut Context<Self>) {
        let rows = rows.into_iter().filter(|row| match self.scope {
            SlashScope::Task => inserts_message_text(row),
            SlashScope::Chat { .. } => true,
        });
        self.cache
            .insert(self.state.context.clone(), rows.collect());
        self.refilter(cx);
    }

    pub(crate) fn is_open(&self) -> bool {
        self.state.token.is_some()
    }

    pub(crate) fn is_fetching(&self) -> bool {
        self.fetch.is_some()
    }

    pub(crate) fn has_selection(&self) -> bool {
        self.state.active.is_some()
    }

    /// The rows the open (or last open) trigger lists.
    pub(crate) fn rows(&self) -> &[InvocationCandidate] {
        self.cache
            .get(&self.state.context)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(crate) fn selected(&self) -> Option<(Range<usize>, InvocationCandidate)> {
        let token = self.state.token.as_ref()?;
        let ix = *self.state.filtered.get(self.state.active?)?;
        Some((token.range.clone(), self.rows().get(ix)?.clone()))
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(SlashMenuEvent::Changed);
        cx.notify();
    }

    pub(crate) fn set_blocked_compact(
        &mut self,
        reason: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        if self.blocked_compact != reason {
            self.blocked_compact = reason;
            cx.notify();
        }
    }

    fn empty_scope_rows(
        &self,
        skill: bool,
        commands_allowed: bool,
    ) -> Option<Vec<InvocationCandidate>> {
        (!skill && commands_allowed).then(|| scoped(self.scope, Vec::new()))
    }

    /// Track the `/` or `$` token on every edit: open/refresh the popup, fetch
    /// the harness's catalog on each open, filter locally per keystroke.
    pub(crate) fn refresh(
        &mut self,
        text: &str,
        cursor: usize,
        target: SlashTarget,
        cx: &mut Context<Self>,
    ) {
        let SlashTarget {
            harness,
            params,
            connection,
            scope,
            blocked_compact,
        } = target;
        self.scope = scope;
        self.blocked_compact = blocked_compact;
        let preferences =
            crate::settings::current(cx).skill_completion(harness.unwrap_or(HarnessId::Codex));
        let (token, skill, include_skills, commands_allowed) =
            completion_trigger(text, cursor, preferences);
        if token.is_none() {
            // Leaving a token must not replace the catalog identity with the
            // idle (commands_allowed=false) context and evict the warm cache.
            self.reset(None, cx);
            return;
        }
        let catalog_context = format!("{preferences:?}:{scope:?}:{connection}:{params}");
        let context = slash_context(skill, include_skills, commands_allowed, &catalog_context);
        let context_changed = self.state.context != context;
        if !context_changed
            && token.as_ref().is_some_and(|token| {
                self.state.dismissed.as_ref().is_some_and(|(range, value)| {
                    token.range == *range && text.get(range.clone()) == Some(value.as_str())
                })
            })
        {
            return;
        }
        if !context_changed && token == self.state.token {
            return;
        }
        let refresh_catalog = context_changed || self.state.token.is_none();
        self.state.dismissed = None;
        if context_changed {
            self.state.request = self.state.request.wrapping_add(1);
            self.fetch = None;
            self.state.loading = false;
            if self.state.catalog_context != catalog_context {
                self.cache.clear();
            } else if self.state.error.is_some() || !self.state.supported {
                self.cache.remove(&self.state.context);
            }
        }
        self.state.catalog_context = catalog_context;
        self.state.context = context.clone();
        self.state.harness = harness;
        self.state.skill = skill;
        if context_changed {
            self.state.supported = true;
        }
        self.state.token = token;
        if context_changed {
            self.state.error = None;
        }
        if harness.is_none()
            && let Some(rows) = self.empty_scope_rows(skill, commands_allowed)
        {
            self.cache.insert(context.clone(), rows);
        }
        if harness.is_none()
            || (self.cache.contains_key(&context) && !refresh_catalog)
            || self.state.loading
        {
            self.refilter(cx);
            return;
        }
        let Some(engine) = self.app.read(cx).engine().cloned() else {
            if let Some(rows) = self.empty_scope_rows(skill, commands_allowed) {
                self.cache.insert(context, rows);
                self.state.error = Some("Agent command discovery requires a connection".into());
                self.refilter(cx);
            }
            return;
        };
        self.state.request = self.state.request.wrapping_add(1);
        let request = self.state.request;
        self.state.loading = true;
        self.state.error = None;
        self.refilter(cx);
        self.fetch = Some(cx.spawn(async move |this, cx| {
            let result = fetch_invocation_catalog(
                engine,
                params,
                harness,
                skill,
                include_skills,
                commands_allowed,
            )
            .await;
            this.update(cx, |menu, cx| {
                if menu.state.request != request || menu.state.context != context {
                    return;
                }
                menu.state.loading = false;
                let decoded = result.map(|(candidates, supported, warning)| {
                    menu.state.supported = supported;
                    menu.state.error = warning;
                    candidates
                });
                match decoded {
                    Ok(candidates) => {
                        let candidates = if !skill && commands_allowed {
                            scoped(scope, candidates)
                        } else {
                            candidates
                        };
                        menu.cache.insert(context, candidates);
                    }
                    Err(err) => {
                        menu.cache.remove(&context);
                        menu.state.error = Some(slash_error_message(&err, skill));
                        if let Some(rows) = menu.empty_scope_rows(skill, commands_allowed) {
                            menu.cache.insert(context, rows);
                        }
                    }
                }
                menu.refilter(cx);
            })
            .ok();
        }));
        cx.notify();
    }

    /// Warm both triggers' catalogs while nothing is open, so the first `/` or
    /// `$` opens on rows instead of a skeleton.
    pub(crate) fn prefetch(&mut self, target: SlashTarget, cx: &mut Context<Self>) {
        if self.state.token.is_some() {
            return;
        }
        let SlashTarget {
            harness,
            params,
            connection,
            scope,
            ..
        } = target;
        let Some(harness) = harness else {
            return;
        };
        let preferences = crate::settings::current(cx).skill_completion(harness);
        let catalog_context = format!("{preferences:?}:{scope:?}:{connection}:{params}");
        if self.state.catalog_context == catalog_context
            || self
                .prefetch
                .as_ref()
                .is_some_and(|(pending, _)| *pending == catalog_context)
        {
            return;
        }
        let Some(engine) = self.app.read(cx).engine().cloned() else {
            return;
        };
        let triggers: Vec<_> = ["/", "$"]
            .into_iter()
            .filter_map(|trigger| {
                let (token, skill, include_skills, commands_allowed) =
                    completion_trigger(trigger, 1, preferences);
                token.map(|_| (skill, include_skills, commands_allowed))
            })
            .collect();
        let context = catalog_context.clone();
        let task = cx.spawn(async move |this, cx| {
            // Clicking through chats replaces this task before any agent is spawned.
            cx.background_executor().timer(PREFETCH_DELAY).await;
            let results = futures::future::join_all(triggers.iter().map(
                |&(skill, include_skills, commands_allowed)| {
                    fetch_invocation_catalog(
                        engine.clone(),
                        params.clone(),
                        Some(harness),
                        skill,
                        include_skills,
                        commands_allowed,
                    )
                },
            ))
            .await;
            this.update(cx, |menu, _| {
                if menu
                    .prefetch
                    .take_if(|(pending, _)| *pending == context)
                    .is_none()
                    || menu.state.token.is_some()
                {
                    return;
                }
                if menu.state.catalog_context != context {
                    menu.cache.clear();
                    menu.state.catalog_context = context.clone();
                }
                for ((skill, include_skills, commands_allowed), result) in
                    triggers.into_iter().zip(results)
                {
                    let Ok((candidates, _, _)) = result else {
                        continue;
                    };
                    let candidates = if !skill && commands_allowed {
                        scoped(scope, candidates)
                    } else {
                        candidates
                    };
                    menu.cache.insert(
                        slash_context(skill, include_skills, commands_allowed, &context),
                        candidates,
                    );
                }
            })
            .ok();
        });
        self.prefetch = Some((catalog_context, task));
    }

    /// Re-rank the cached list for the current query (pure local filter).
    pub(crate) fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self
            .state
            .token
            .as_ref()
            .map(|t| t.query.clone())
            .unwrap_or_default();
        let commands = self
            .cache
            .get(&self.state.context)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
        self.state.filtered = crate::popover::filter_indices(&query, &names);
        let disabled = &crate::settings::current(cx).disabled_skills;
        if !disabled.is_empty() {
            self.state.filtered.retain(|&ix| {
                !commands
                    .get(ix)
                    .is_some_and(|command| match &command.invocation {
                        zeron_proto::invocation::Invocation::Skill { path, .. } => {
                            disabled.contains(path)
                        }
                        zeron_proto::invocation::Invocation::Command { .. } => false,
                    })
            });
        }
        if self.state.harness == Some(HarnessId::ClaudeCode) {
            self.state
                .filtered
                .sort_by_key(|&ix| commands.get(ix).is_some_and(terminal_only_command));
        }
        self.state.active = (!self.state.filtered.is_empty()).then_some(0);
        // A fresh query/reopen restarts the row stack at the top.
        crate::popover::reset_menu_scroll(&self.scroll, &mut self.bar);
        self.changed(cx);
    }

    pub(crate) fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.state.active =
            crate::popover::menu_step(self.state.active, self.state.filtered.len(), delta);
        if let Some(active) = self.state.active {
            // Keep the keyboard cursor visible in the scrolled row stack.
            self.scroll.scroll_to_item(active);
        }
        self.changed(cx);
    }

    pub(crate) fn dismiss(&mut self, cx: &mut Context<Self>) {
        let dismissed = self.state.token.as_ref().and_then(|token| {
            self.input
                .read(cx)
                .text()
                .get(token.range.clone())
                .map(|text| (token.range.clone(), text.to_string()))
        });
        self.reset(dismissed, cx);
    }

    /// Tear down the completion; the catalog and its identity stay warm.
    pub(crate) fn reset(
        &mut self,
        dismissed: Option<(Range<usize>, String)>,
        cx: &mut Context<Self>,
    ) {
        // Partial discovery is useful now, but reopening must retry the failed provider call.
        if self.state.error.is_some() {
            self.cache.remove(&self.state.context);
        }
        let request = self.state.request.wrapping_add(1);
        self.fetch = None;
        self.state = SlashState {
            request,
            dismissed,
            harness: self.state.harness,
            catalog_context: self.state.catalog_context.clone(),
            context: self.state.context.clone(),
            supported: self.state.supported,
            ..SlashState::default()
        };
        self.changed(cx);
    }

    /// Put the picked entry into the draft in place of its token: a canonical
    /// reference link, or the plain command when the device cannot deliver links.
    pub(crate) fn insert(
        &mut self,
        range: Range<usize>,
        candidate: &InvocationCandidate,
        delivery_supported: bool,
        cx: &mut Context<Self>,
    ) {
        let insertion = if zeron_owned_command(&candidate.name) {
            format!("/{}", candidate.name)
        } else {
            invocation_insertion(&candidate.invocation, delivery_supported)
        };
        if super::reference_refused(&self.input, &range, &insertion, cx) {
            self.dismiss(cx);
            return;
        }
        self.input.update(cx, |input, cx| {
            input.replace_plain_token(range, &insertion, cx)
        });
        self.reset(None, cx);
    }

    fn on_list_hover(&mut self, hovered: &bool, _window: &mut Window, cx: &mut Context<Self>) {
        if self.bar.set_list_hovered(*hovered) {
            cx.notify();
        }
    }
}

impl crate::popover::ScrollRailHost for SlashMenu {
    fn rail_bar(&mut self) -> &mut crate::popover::MenuScrollbarState {
        &mut self.bar
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.state.token.is_some().then(|| self.scroll.clone())
    }
}

impl Render for SlashMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &Theme::of(cx).for_popup();
        if self.state.token.is_none() {
            return div().into_any_element();
        }
        let commands = self
            .cache
            .get(&self.state.context)
            .map(Vec::as_slice)
            .unwrap_or_default();
        // Full pill width at the mention card's height budget — both composer
        // completions share the same surface shape.
        let mut card = crate::popover::completion_card(theme)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.input.focus_handle(cx), cx);
                }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss(cx)));
        if let Some(error) = self.state.error.clone() {
            card = card.child(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.danger_muted)
                    .child(error),
            );
        }
        if self.state.loading && commands.is_empty() {
            card = card.child(crate::popover::skeleton_rows(
                "slash-loading",
                theme,
                3,
                cx.entity_id(),
                cx,
            ));
        } else if self.state.filtered.is_empty() && self.state.error.is_none() {
            card = card.child(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(catalog_empty_message(
                        self.state.skill,
                        self.state.supported,
                        commands.is_empty(),
                        crate::settings::current(cx)
                            .skill_completion(self.state.harness.unwrap_or(HarnessId::Codex))
                            .separate_from_slash,
                    )),
            );
        } else {
            let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(self.state.filtered.len());
            let unsupported = self.blocked_compact.clone();
            let claude = self.state.harness == Some(HarnessId::ClaudeCode);
            for (row_ix, &cmd_ix) in self.state.filtered.iter().enumerate() {
                let Some(command) = commands.get(cmd_ix) else {
                    continue;
                };
                let selected = self.state.active == Some(row_ix);
                let name: SharedString = if command.invocation.prefix() == '$' {
                    skill_display_name(&command.name)
                } else {
                    format!("/{}", command.name)
                }
                .into();
                // The reason replaces the description rather than hiding the
                // row: the entry exists on every harness, so its absence
                // would read as a bug.
                let blocked = (command.name == COMPACT_COMMAND)
                    .then(|| unsupported.clone())
                    .flatten();
                let description: SharedString = match blocked {
                    Some(reason) => reason,
                    None => invocation_detail(&command.description, command.input_hint.as_deref())
                        .into(),
                };
                let disabled = command.name == COMPACT_COMMAND && unsupported.is_some();
                let terminal = claude && terminal_only_command(command);
                rows.push(
                    crate::popover::menu_row(theme, selected, format!("slash-result-{row_ix}"))
                        .id(("slash-result", row_ix))
                        .debug_selector(move || format!("slash-result-{row_ix}"))
                        .when(disabled, |row| row.opacity(0.45))
                        .when(terminal, |row| row.opacity(0.55))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.state.active = Some(row_ix);
                            cx.emit(SlashMenuEvent::Accept);
                        }))
                        .child(crate::popover::completion_row_content(
                            theme,
                            crate::icons::icon(if command.invocation.prefix() == '$' {
                                crate::icons::MAGIC_STICK_3
                            } else {
                                crate::icons::COMMAND
                            })
                            .size(px(16.0))
                            .text_color(theme.text_muted)
                            .into_any_element(),
                            name,
                            description,
                        ))
                        .when(terminal, |row| {
                            row.child(
                                div()
                                    .flex_none()
                                    .px(px(6.0))
                                    .rounded(px(4.0))
                                    .bg(crate::theme::ink(0.06))
                                    .text_size(crate::typography::ui_rems(10.5))
                                    .text_color(theme.text_muted)
                                    .child("Terminal"),
                            )
                        })
                        .into_any_element(),
                );
            }
            // Overflowing rows wheel-scroll inside a bounded viewport; the
            // floating rail mirrors the model-list scrollbar treatment.
            card = card.child(
                crate::popover::menu_scroll_host("slash-scroll-host")
                    .on_hover(cx.listener(Self::on_list_hover))
                    .child(crate::popover::completion_list(
                        "slash-list",
                        &self.scroll,
                        rows,
                    ))
                    .children(crate::popover::rail(self, "slash-scrollbar", theme, cx)),
            );
        }
        card.into_any_element()
    }
}
