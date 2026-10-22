use std::collections::HashMap;
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gpui::{
    AnyElement, App, Context, Entity, Hsla, MouseButton, SharedString, Subscription, Task, Window,
    div, prelude::*, px,
};
use zeron_engine::routing::{MidRun, NativePolicy, OutOfQuota, Role, RoleRoute, RoutingSettings};
use zeron_proto::{HarnessId, Model, ReasoningLevel, RouteTarget};
use zeron_rpc::methods;

use crate::popover::{self, Loadable};
use crate::review::ReviewerOptions;
use crate::settings::{accounts, widgets};
use crate::state::{AppState, EngineHandle};
use crate::theme::Theme;

const PAGE_WIDTH: f32 = 940.0;
const HARNESS_WIDTH: f32 = 168.0;
const MODEL_WIDTH: f32 = 148.0;
const MULTIPLIER_WIDTH: f32 = 36.0;
const REASONING_WIDTH: f32 = 100.0;
const QUOTA_WIDTH: f32 = 140.0;
const GAP: f32 = 8.0;

use zeron_engine::routing::preview::{Preview, Row as PreviewRow};
use zeron_proto::routing::{multiplier_text, short_name};

struct Quota {
    fraction: f32,
    reset: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum HarnessPick {
    Off,
    SameAsParent,
    Harness(HarnessId),
}

#[derive(Default)]
struct RowSelects {
    harness: widgets::SelectState,
    model: widgets::SelectState,
    reasoning: widgets::SelectState,
}

struct MenuEntry {
    label: SharedString,
    detail: Option<SharedString>,
    badge: Option<&'static str>,
    harness: Option<HarnessId>,
    selected: bool,
}

type MenuReach = fn(&mut RoutingPage) -> &mut popover::Popup<()>;

pub struct RoutingPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    settings: Loadable<RoutingSettings>,
    options: Option<ReviewerOptions>,
    preview: Option<Preview>,
    error: Option<String>,
    models: HashMap<HarnessId, Vec<Model>>,
    rows: [RowSelects; 5],
    policy_menu: popover::Popup<()>,
    add_menu: popover::Popup<()>,
    task: Option<Task<()>>,
    save_task: Option<Task<()>>,
    model_tasks: HashMap<HarnessId, Task<()>>,
    _state: Subscription,
    _accounts: Subscription,
}

impl RoutingPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let state_sub = cx.observe(&state, |this: &mut Self, _, cx| {
            if matches!(this.settings, Loadable::Idle) {
                this.load(cx);
            }
        });
        let accounts_sub =
            cx.observe_global::<accounts::AccountsSnapshotCache>(|_: &mut Self, cx| cx.notify());
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            settings: Loadable::Idle,
            options: None,
            preview: None,
            error: None,
            models: HashMap::new(),
            rows: Default::default(),
            policy_menu: Default::default(),
            add_menu: Default::default(),
            task: None,
            save_task: None,
            model_tasks: HashMap::new(),
            _state: state_sub,
            _accounts: accounts_sub,
        };
        page.load(cx);
        page
    }

    pub(crate) fn load(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if matches!(self.settings, Loadable::Idle) {
            self.settings = Loadable::Loading;
        }
        self.task = Some(cx.spawn(async move |this, cx| {
            let settings = engine
                .client()
                .call_as::<RoutingSettings>(methods::GET_ROUTING_SETTINGS, serde_json::json!({}))
                .await
                .map_err(|e| e.to_string());
            let options = crate::review::reviewer_options(&engine).await;
            let preview = fetch_preview(&engine).await;
            this.update(cx, |page, cx| {
                match settings {
                    Ok(settings) => page.settings = Loadable::Ready(settings),
                    Err(error) if page.settings.ready().is_some() => page.error = Some(error),
                    Err(error) => page.settings = Loadable::Error(error),
                }
                page.options = Some(options);
                page.preview = preview;
                page.load_models(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn offered(&self) -> Vec<HarnessId> {
        self.options
            .iter()
            .flat_map(|options| &options.offered)
            .copied()
            .filter(|harness| *harness != HarnessId::Acp)
            .collect()
    }

    fn load_models(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        for harness in self.offered() {
            if self.models.contains_key(&harness) || self.model_tasks.contains_key(&harness) {
                continue;
            }
            let engine = engine.clone();
            let task = cx.spawn(async move |this, cx| {
                let models = crate::review::list_models(&engine, harness).await;
                this.update(cx, |page, cx| {
                    page.models.insert(harness, models);
                    page.model_tasks.remove(&harness);
                    cx.notify();
                })
                .ok();
            });
            self.model_tasks.insert(harness, task);
        }
    }

    fn save(&mut self, edit: impl FnOnce(&mut RoutingSettings), cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(previous) = self.settings.ready().cloned() else {
            return;
        };
        let mut next = previous.clone();
        edit(&mut next);
        if next == previous {
            return;
        }
        self.settings = Loadable::Ready(next.clone());
        cx.notify();
        self.save_task = Some(cx.spawn(async move |this, cx| {
            let saved = engine
                .client()
                .call_as::<RoutingSettings>(
                    methods::SET_ROUTING_SETTINGS,
                    serde_json::to_value(&next).unwrap_or_default(),
                )
                .await
                .map_err(|e| e.to_string());
            let preview = fetch_preview(&engine).await;
            this.update(cx, |page, cx| {
                match saved {
                    Ok(saved) => {
                        page.settings = Loadable::Ready(saved);
                        page.error = None;
                    }
                    Err(error) => {
                        page.settings = Loadable::Ready(previous);
                        page.error = Some(error);
                    }
                }
                if preview.is_some() {
                    page.preview = preview;
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn set_route(&mut self, role: Role, route: RoleRoute, cx: &mut Context<Self>) {
        self.save(
            move |settings| {
                settings.roles.insert(role, route);
            },
            cx,
        );
    }

    fn preview_row(&self, role: Role) -> Option<&PreviewRow> {
        self.preview
            .as_ref()?
            .rows
            .iter()
            .find(|row| row.role == role.name())
    }

    fn base_label(&self) -> Option<String> {
        let base = self.preview.as_ref()?.base.as_ref()?;
        Some(
            base.model
                .clone()
                .unwrap_or_else(|| base.harness.display_name().to_string()),
        )
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    fn close_menu(&mut self, reach: MenuReach, cx: &mut Context<Self>) {
        if reach(self).begin_close() {
            popover::reap_popup(cx, reach);
        }
        cx.notify();
    }
}

async fn fetch_preview(engine: &EngineHandle) -> Option<Preview> {
    engine
        .client()
        .call_as::<Preview>(methods::PREVIEW_ROUTES, serde_json::json!({}))
        .await
        .ok()
}

fn quota(preview: Option<&Preview>, harness: HarnessId, cx: &App) -> Option<Quota> {
    let engine = preview
        .into_iter()
        .flat_map(|preview| &preview.rows)
        .find(|row| {
            row.quota.known
                && row
                    .target
                    .as_ref()
                    .is_some_and(|target| target.harness == harness)
        });
    if let Some(row) = engine {
        let fraction = if row.quota.exhausted {
            1.0
        } else {
            row.quota.used_fraction.unwrap_or_default() as f32
        };
        let resets_at = row
            .quota
            .resets_at
            .and_then(|at| Utc.timestamp_millis_opt(at).single());
        return Some(Quota {
            fraction,
            reset: accounts::format_reset(resets_at, Utc::now()),
        });
    }
    if !accounts::reports_usage(harness) {
        return None;
    }
    let snapshot = crate::account_usage::local_snapshot(cx)?;
    let account = crate::account_usage::active_account(snapshot, harness)?;
    let fraction = crate::account_usage::used_fraction(account)?;
    let window = account
        .usage_windows
        .iter()
        .max_by(|a, b| a.used_fraction.total_cmp(&b.used_fraction))?;
    Some(Quota {
        fraction,
        reset: accounts::format_reset(window.resets_at, Utc::now()),
    })
}

fn exhausted(preview: Option<&Preview>, harness: HarnessId, cx: &App) -> bool {
    quota(preview, harness, cx).is_some_and(|quota| quota.fraction >= 1.0)
}

fn brand_icon(harness: HarnessId, size: f32, theme: &Theme) -> AnyElement {
    let (mark, tint) = crate::pickers::harness_brand_icon(harness);
    crate::icons::icon(mark)
        .size(px(size))
        .flex_none()
        .text_color(tint.unwrap_or(theme.text_muted))
        .into_any_element()
}

fn small(color: Hsla, text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(11.5))
        .line_height(crate::typography::ui_rems(15.0))
        .text_color(color)
        .child(text.into())
}

fn placeholder(theme: &Theme, label: &'static str, width: f32, chevron: bool) -> gpui::Div {
    div()
        .flex_none()
        .w(px(width))
        .h(px(widgets::SELECT_HEIGHT))
        .pl(px(10.0))
        .pr(px(8.0))
        .rounded(px(8.0))
        .bg(widgets::select_fill(theme, false).opacity(0.6))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .text_size(crate::typography::ui_rems(12.5))
        .text_color(theme.text_muted.opacity(0.8))
        .child(widgets::select_value(label))
        .when(chevron, |el| {
            el.child(
                crate::icons::icon(crate::icons::ALT_ARROW_DOWN)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(theme.text_muted.opacity(0.5)),
            )
        })
}

fn table_row(theme: &Theme, first: bool) -> gpui::Div {
    div()
        .mx(px(16.0))
        .py(px(10.0))
        .when(!first, |el| {
            el.border_t_1().border_color(widgets::row_divider(theme))
        })
        .flex()
        .flex_row()
        .items_center()
        .gap(px(GAP))
}

fn settings_row(
    theme: &Theme,
    first: bool,
    title: impl Into<SharedString>,
    description: &'static str,
) -> gpui::Div {
    widgets::card_row(theme, first).child(
        div()
            .flex_1()
            .min_w(px(220.0))
            .child(widgets::row_title(theme, title))
            .child(widgets::meta_line(
                theme,
                vec![div().child(description).into_any_element()],
            )),
    )
}

impl RoutingPage {
    #[allow(clippy::too_many_arguments)]
    fn with_menu(
        &mut self,
        trigger: gpui::Stateful<gpui::Div>,
        id: &'static str,
        reach: MenuReach,
        width: f32,
        entries: Vec<MenuEntry>,
        pick: impl Fn(&mut Self, usize, &mut Context<Self>) + 'static,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let pick = Rc::new(pick);
        let popup = reach(self);
        let (mounted, closing) = (popup.get().is_some(), popup.closing_since());
        let mut trigger = trigger
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |page, _, _, _| reach(page).note_trigger_press()),
            )
            .on_click(cx.listener(move |page, _, _, cx| {
                let menu = reach(page);
                if menu.take_press_was_open() || menu.is_open() {
                    page.close_menu(reach, cx);
                } else {
                    menu.open(());
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(move |page, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && reach(page).is_open() {
                    cx.stop_propagation();
                    page.close_menu(reach, cx);
                }
            }));
        if !mounted {
            return trigger;
        }
        let rows = entries.into_iter().enumerate().map(|(ix, entry)| {
            let row_id: SharedString = format!("{id}-option-{ix}").into();
            let pick = pick.clone();
            popover::menu_row(theme, entry.selected, row_id.clone())
                .id(row_id.clone())
                .debug_selector(move || row_id.to_string())
                .role(gpui::Role::MenuItemRadio)
                .aria_label(entry.label.clone())
                .aria_toggled(if entry.selected {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .items_start()
                .on_click(cx.listener(move |page, _, _, cx| {
                    cx.stop_propagation();
                    page.close_menu(reach, cx);
                    pick(page, ix, cx);
                }))
                .children(
                    entry
                        .harness
                        .map(|harness| div().pt(px(2.0)).child(brand_icon(harness, 13.0, theme))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(div().min_w_0().truncate().child(entry.label))
                                .children(
                                    entry.badge.map(|badge| widgets::badge_active(theme, badge)),
                                ),
                        )
                        .children(entry.detail.map(|detail| small(theme.text_muted, detail))),
                )
                .child(widgets::select_check(theme, entry.selected))
                .into_any_element()
        });
        let menu = widgets::select_menu(theme)
            .w(px(width))
            .on_mouse_down_out(cx.listener(move |page, _, _, cx| page.close_menu(reach, cx)))
            .child(widgets::dropdown_rows(
                format!("{id}-list"),
                rows,
                widgets::SELECT_HEIGHT,
                8.0,
            ));
        trigger = trigger.child(widgets::dropdown(
            SharedString::from(format!("{id}-menu")),
            menu,
            closing,
            widgets::SELECT_HEIGHT,
        ));
        trigger
    }

    fn render_roles(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let header = |label: SharedString, width: Option<f32>| {
            div()
                .when_some(width, |el, width| el.w(px(width)).flex_none())
                .when(width.is_none(), |el| el.flex_1().min_w(px(120.0)))
                .child(small(theme.text_muted, label))
        };
        let model_header = match self.base_label() {
            Some(base) => format!("Model · vs {base}"),
            None => "Model".to_string(),
        };
        let mut card = widgets::section_card(theme).mt(px(8.0)).child(
            table_row(theme, true)
                .py(px(8.0))
                .child(header("Role".into(), None))
                .child(header("Harness".into(), Some(HARNESS_WIDTH)))
                .child(header(
                    model_header.into(),
                    Some(MODEL_WIDTH + GAP + MULTIPLIER_WIDTH),
                ))
                .child(header("Reasoning".into(), Some(REASONING_WIDTH)))
                .child(header("Quota".into(), Some(QUOTA_WIDTH))),
        );
        for (ix, role) in Role::ALL.into_iter().enumerate() {
            card = card.child(self.render_role(ix, role, role.summary(), theme, cx));
        }
        div()
            .mt(px(24.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_end()
                    .justify_between()
                    .child(widgets::section_label(theme, "Roles"))
                    .child(
                        small(theme.text_muted, "Quota refreshes with the Usage page").pr(px(4.0)),
                    ),
            )
            .child(card)
    }

    fn render_role(
        &mut self,
        ix: usize,
        role: Role,
        meaning: &'static str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let route = self
            .settings
            .ready()
            .map(|settings| settings.route(role).clone())
            .unwrap_or_default();
        let preview = self.preview_row(role).cloned();
        let target_quota = match &route {
            RoleRoute::Target(target) => Some((
                target.harness,
                quota(self.preview.as_ref(), target.harness, cx),
            )),
            _ => None,
        };
        let fell_back = preview.as_ref().is_some_and(PreviewRow::fell_back);
        let fallback_line = preview.as_ref().and_then(PreviewRow::fallback_line);
        let description = match (&fallback_line, &target_quota) {
            (Some(line), _) => small(theme.accent, line.clone()),
            (None, Some((_, None))) => small(
                theme.text_muted,
                "No quota data, falls back after the first limit error",
            ),
            _ => small(theme.text_muted, meaning),
        };
        let role_cell = div()
            .flex_1()
            .min_w(px(120.0))
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .font_family(theme.font_mono.clone())
                            .text_size(crate::typography::ui_rems(13.0))
                            .text_color(theme.text)
                            .child(role.name()),
                    )
                    .when(fell_back, |el| {
                        el.child(
                            div()
                                .px(px(6.0))
                                .rounded(px(4.0))
                                .bg(theme.accent_wash)
                                .text_size(crate::typography::ui_rems(10.5))
                                .text_color(theme.accent)
                                .child("fell back"),
                        )
                    }),
            )
            .child(description);
        let harness = self.render_harness_select(ix, role, &route, theme, cx);
        let mut row = table_row(theme, false)
            .min_h(px(56.0))
            .child(role_cell)
            .child(harness);
        match &route {
            RoleRoute::Off => {
                row = row.child(placeholder(
                    theme,
                    "Agent picks harness and model for this role",
                    MODEL_WIDTH + MULTIPLIER_WIDTH + REASONING_WIDTH + 2.0 * GAP,
                    false,
                ));
            }
            RoleRoute::SameAsParent => {
                row = row
                    .child(placeholder(theme, "Parent's model", MODEL_WIDTH, true))
                    .child(div().w(px(MULTIPLIER_WIDTH)).flex_none())
                    .child(placeholder(theme, "Parent's", REASONING_WIDTH, true));
            }
            RoleRoute::Target(target) => {
                let multiplier = preview
                    .as_ref()
                    .filter(|row| row.source == "routed")
                    .and_then(|row| row.multiplier);
                row = row
                    .child(self.render_model_select(ix, role, target, theme, cx))
                    .child(
                        div()
                            .w(px(MULTIPLIER_WIDTH))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .children(multiplier.map(|multiplier| {
                                small(theme.text_muted, multiplier_text(multiplier))
                            })),
                    )
                    .child(self.render_reasoning_select(ix, role, target, theme, cx));
            }
        }
        row.child(Self::render_quota(theme, target_quota))
    }

    fn render_quota(theme: &Theme, quota: Option<(HarnessId, Option<Quota>)>) -> AnyElement {
        let cell = div()
            .w(px(QUOTA_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(5.0));
        let Some((harness, quota)) = quota else {
            return cell.into_any_element();
        };
        let name = short_name(harness);
        match quota {
            Some(quota) => {
                let percent = (quota.fraction * 100.0).round() as u32;
                let level = accounts::usage_level(quota.fraction);
                let tint = match level {
                    accounts::UsageLevel::Normal => theme.text,
                    _ => accounts::usage_color(level, theme),
                };
                cell.child(accounts::usage_bar(quota.fraction, theme))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(4.0))
                            .min_w_0()
                            .child(small(tint, format!("{name} {percent}%")).flex_none())
                            .children(quota.reset.map(|reset| {
                                small(theme.text_muted, format!("· {reset}"))
                                    .min_w_0()
                                    .truncate()
                            })),
                    )
                    .into_any_element()
            }
            None => cell
                .child(
                    div()
                        .mt(px(2.0))
                        .h(px(3.0))
                        .border_t_1()
                        .border_dashed()
                        .border_color(theme.text_muted.opacity(0.5)),
                )
                .child(small(theme.text_muted, format!("{name}: no quota data")))
                .into_any_element(),
        }
    }

    fn render_harness_select(
        &mut self,
        ix: usize,
        role: Role,
        route: &RoleRoute,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut picks = vec![HarnessPick::Off, HarnessPick::SameAsParent];
        picks.extend(self.offered().into_iter().map(HarnessPick::Harness));
        let current = match route {
            RoleRoute::Off => HarnessPick::Off,
            RoleRoute::SameAsParent => HarnessPick::SameAsParent,
            RoleRoute::Target(target) => HarnessPick::Harness(target.harness),
        };
        if !picks.contains(&current) {
            picks.push(current);
        }
        let selected = picks.iter().position(|pick| *pick == current).unwrap_or(0);
        let theme_icon = theme.clone();
        let options = picks.iter().map(|pick| match pick {
            HarnessPick::Off => widgets::SelectOption::new("Off (agent decides)"),
            HarnessPick::SameAsParent => widgets::SelectOption::new("Same as parent"),
            HarnessPick::Harness(harness) => {
                let harness = *harness;
                let theme = theme_icon.clone();
                widgets::SelectOption::new(harness.display_name())
                    .leading(move || brand_icon(harness, 13.0, &theme))
            }
        });
        let route = route.clone();
        widgets::select(
            format!("routing-{}-harness", role.name()),
            format!("{} harness", role.name()),
            theme,
            move |page: &mut Self| &mut page.rows[ix].harness,
        )
        .options(options, selected)
        .width(HARNESS_WIDTH)
        .menu_width(200.0)
        .open_rightward()
        .on_select(move |page, pick, _, cx| {
            let next = match picks.get(pick) {
                Some(HarnessPick::Off) => RoleRoute::Off,
                Some(HarnessPick::SameAsParent) => RoleRoute::SameAsParent,
                Some(HarnessPick::Harness(harness)) => match &route {
                    RoleRoute::Target(target) if target.harness == *harness => return,
                    _ => RoleRoute::Target(RouteTarget {
                        harness: *harness,
                        model: None,
                        reasoning: None,
                    }),
                },
                None => return,
            };
            page.set_route(role, next, cx);
        })
        .render(&self.rows[ix].harness, cx)
        .into_any_element()
    }

    fn render_model_select(
        &mut self,
        ix: usize,
        role: Role,
        target: &RouteTarget,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut models: Vec<(Option<String>, String)> = vec![(None, "Default".to_string())];
        models.extend(
            self.models
                .get(&target.harness)
                .into_iter()
                .flatten()
                .map(|model| (Some(model.id.clone()), model.label.clone())),
        );
        if let Some(model) = &target.model
            && !models.iter().any(|(id, _)| id.as_ref() == Some(model))
        {
            models.push((Some(model.clone()), model.clone()));
        }
        let selected = models
            .iter()
            .position(|(id, _)| *id == target.model)
            .unwrap_or(0);
        let options = models
            .iter()
            .map(|(_, label)| widgets::SelectOption::new(label.clone()));
        let target = target.clone();
        widgets::select(
            format!("routing-{}-model", role.name()),
            format!("{} model", role.name()),
            theme,
            move |page: &mut Self| &mut page.rows[ix].model,
        )
        .options(options, selected)
        .width(MODEL_WIDTH)
        .menu_width(220.0)
        .on_select(move |page, pick, _, cx| {
            let Some((model, _)) = models.get(pick) else {
                return;
            };
            page.set_route(
                role,
                RoleRoute::Target(RouteTarget {
                    model: model.clone(),
                    ..target.clone()
                }),
                cx,
            );
        })
        .render(&self.rows[ix].model, cx)
        .into_any_element()
    }

    fn render_reasoning_select(
        &mut self,
        ix: usize,
        role: Role,
        target: &RouteTarget,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let model = target.model.as_deref().and_then(|id| {
            self.models
                .get(&target.harness)?
                .iter()
                .find(|model| model.id == id)
        });
        let levels: Vec<ReasoningLevel> = match &self.options {
            Some(options) => crate::review::reasoning_stops(options, target.harness, model),
            None => Vec::new(),
        };
        if levels.is_empty() {
            return placeholder(theme, "Default", REASONING_WIDTH, true).into_any_element();
        }
        let selected = target
            .reasoning
            .and_then(|level| levels.iter().position(|l| *l == level))
            .map_or(0, |ix| ix + 1);
        let options = std::iter::once(widgets::SelectOption::new("Default")).chain(
            levels
                .iter()
                .map(|level| widgets::SelectOption::new(crate::pickers::reasoning_label(*level))),
        );
        let target = target.clone();
        widgets::select(
            format!("routing-{}-reasoning", role.name()),
            format!("{} reasoning", role.name()),
            theme,
            move |page: &mut Self| &mut page.rows[ix].reasoning,
        )
        .options(options, selected)
        .width(REASONING_WIDTH)
        .on_select(move |page, pick, _, cx| {
            page.set_route(
                role,
                RoleRoute::Target(RouteTarget {
                    reasoning: pick.checked_sub(1).and_then(|ix| levels.get(ix).copied()),
                    ..target.clone()
                }),
                cx,
            );
        })
        .render(&self.rows[ix].reasoning, cx)
        .into_any_element()
    }

    fn render_out_of_quota(
        &mut self,
        settings: &RoutingSettings,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let current = settings.out_of_quota;
        let label = current.label();
        let value = if current == OutOfQuota::Native {
            format!("{label} (Recommended)")
        } else {
            label.to_string()
        };
        let open = self.policy_menu.is_open();
        let trigger = widgets::select_trigger(theme, "routing-policy", open)
            .debug_selector(|| "routing-policy".into())
            .w(px(240.0))
            .aria_label(format!("When out of quota: {value}"))
            .child(widgets::select_value(value))
            .child(widgets::select_chevron(theme, open));
        let entries = OutOfQuota::ALL
            .iter()
            .map(|policy| MenuEntry {
                label: policy.label().into(),
                detail: Some(policy.detail().into()),
                badge: (*policy == OutOfQuota::Native).then_some("Recommended"),
                harness: None,
                selected: *policy == current,
            })
            .collect();
        let policy = self.with_menu(
            trigger,
            "routing-policy",
            |page| &mut page.policy_menu,
            320.0,
            entries,
            |page, ix, cx| {
                if let Some(policy) = OutOfQuota::ALL.get(ix).copied() {
                    page.save(move |settings| settings.out_of_quota = policy, cx);
                }
            },
            theme,
            cx,
        );

        let this = cx.entity().downgrade();
        let mid_run = widgets::segmented(
            theme,
            "routing-mid-run",
            &MidRun::ALL,
            settings.mid_run,
            move |mid_run, _, cx| {
                this.update(cx, |page, cx| {
                    page.save(move |settings| settings.mid_run = mid_run, cx)
                })
                .ok();
            },
        );

        widgets::section(
            theme,
            "When out of quota",
            widgets::section_card(theme)
                .mt(px(0.0))
                .child(
                    settings_row(
                        theme,
                        true,
                        "When the routed harness is out of quota",
                        "Checked before start for harnesses that report usage. The others report no \
                         quota, so this applies after their first limit error.",
                    )
                    .child(policy),
                )
                .child(
                    settings_row(
                        theme,
                        false,
                        "Fallback list",
                        "Used by \"Next in fallback list\". Harnesses without quota are skipped.",
                    )
                    .child(self.render_fallback(settings, theme, cx)),
                )
                .child(
                    settings_row(
                        theme,
                        false,
                        "Mid-run limit",
                        "When a running child hits its limit, act on the choice above after a \
                         countdown, or always ask.",
                    )
                    .child(mid_run),
                ),
        )
        .mt(px(28.0))
    }

    fn render_fallback(
        &mut self,
        settings: &RoutingSettings,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let spent: Vec<bool> = settings
            .fallback
            .iter()
            .map(|target| exhausted(self.preview.as_ref(), target.harness, cx))
            .collect();
        let chips: Vec<AnyElement> = settings
            .fallback
            .iter()
            .enumerate()
            .map(|(ix, target)| {
                let spent = spent[ix];
                let label = target
                    .model
                    .clone()
                    .unwrap_or_else(|| target.harness.display_name().to_string());
                div()
                    .id(("routing-fallback", ix))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .h(px(28.0))
                    .pl(px(8.0))
                    .pr(px(4.0))
                    .rounded(px(8.0))
                    .bg(widgets::select_fill(theme, false))
                    .text_size(crate::typography::ui_rems(12.0))
                    .child(small(theme.text_muted, format!("{}", ix + 1)))
                    .child(brand_icon(target.harness, 12.0, theme))
                    .child(
                        div()
                            .text_color(if spent { theme.text_muted } else { theme.text })
                            .when(spent, |el| el.line_through())
                            .child(label.clone()),
                    )
                    .child(
                        div()
                            .id(("routing-fallback-remove", ix))
                            .debug_selector(move || format!("routing-fallback-remove-{ix}"))
                            .role(gpui::Role::Button)
                            .aria_label(format!("Remove {label}"))
                            .size(px(18.0))
                            .rounded(px(4.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|el| el.bg(theme.wash(0.08)))
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.save(
                                    move |settings| {
                                        if ix < settings.fallback.len() {
                                            settings.fallback.remove(ix);
                                        }
                                    },
                                    cx,
                                );
                            }))
                            .child(
                                crate::icons::icon(crate::icons::CLOSE)
                                    .size(px(10.0))
                                    .text_color(theme.text_muted),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        let mut candidates: Vec<RouteTarget> = Vec::new();
        for harness in self.offered() {
            let models = self.models.get(&harness).cloned().unwrap_or_default();
            if models.is_empty() {
                candidates.push(RouteTarget {
                    harness,
                    model: None,
                    reasoning: None,
                });
            }
            candidates.extend(models.into_iter().map(|model| RouteTarget {
                harness,
                model: Some(model.id),
                reasoning: None,
            }));
        }
        candidates.retain(|target| !settings.fallback.contains(target));
        let entries = candidates
            .iter()
            .map(|target| MenuEntry {
                label: target
                    .model
                    .clone()
                    .unwrap_or_else(|| target.harness.display_name().to_string())
                    .into(),
                detail: target
                    .model
                    .is_some()
                    .then(|| target.harness.display_name().into()),
                badge: None,
                harness: Some(target.harness),
                selected: false,
            })
            .collect();
        let add_trigger = div()
            .id("routing-fallback-add")
            .tab_index(0)
            .debug_selector(|| "routing-fallback-add".into())
            .role(gpui::Role::Button)
            .aria_label("Add fallback")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .h(px(28.0))
            .px(px(8.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text_muted)
            .hover(|el| el.text_color(theme.text).bg(theme.wash(0.06)))
            .child(
                crate::icons::icon(crate::icons::PLUS)
                    .size(px(11.0))
                    .text_color(theme.text_muted),
            )
            .child("Add");
        let add = self.with_menu(
            add_trigger,
            "routing-fallback-add",
            |page| &mut page.add_menu,
            280.0,
            entries,
            move |page, ix, cx| {
                if let Some(target) = candidates.get(ix).cloned() {
                    page.save(move |settings| settings.fallback.push(target), cx);
                }
            },
            theme,
            cx,
        );
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .justify_end()
            .items_center()
            .gap(px(6.0))
            .max_w(px(480.0))
            .children(chips)
            .child(add)
            .into_any_element()
    }

    fn render_native(
        &self,
        settings: &RoutingSettings,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let this = cx.entity().downgrade();
        let native = widgets::segmented(
            theme,
            "routing-native",
            &[
                (NativePolicy::SameModelOnly, "Only when same model"),
                (NativePolicy::Always, "Always allowed"),
                (NativePolicy::Off, "Off"),
            ],
            settings.native,
            move |native, _, cx| {
                this.update(cx, |page, cx| {
                    page.save(move |settings| settings.native = native, cx)
                })
                .ok();
            },
        );
        let guidance = self
            .preview
            .as_ref()
            .map(|preview| preview.guidance.clone())
            .unwrap_or_default();
        let title = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(brand_icon(HarnessId::ClaudeCode, 13.0, theme))
            .child(widgets::row_title(theme, "Claude Code native sub-agents"));
        widgets::section(
            theme,
            "Native sub-agents",
            widgets::section_card(theme)
                .mt(px(0.0))
                .child(
                    widgets::card_row(theme, true)
                        .child(div().flex_1().min_w(px(220.0)).child(title).child(
                            widgets::meta_line(
                                theme,
                                vec![
                                        div()
                                            .child(
                                                "The Task tool runs on this chat's own model, so \
                                                 Zeron cannot route it.",
                                            )
                                            .into_any_element(),
                                    ],
                            ),
                        ))
                        .child(native),
                )
                .child(widgets::card_row(theme, false).min_h(px(0.0)).child(small(
                    theme.text_muted,
                    "\"Only when same model\" relaxes for a quota fallback. \"Off\" stays \
                         off: the fallback then opens a Zeron child on the chat's own model.",
                )))
                .child(
                    widgets::card_row(theme, false)
                        .flex_col()
                        .items_stretch()
                        .gap(px(8.0))
                        .child(
                            div()
                                .child(widgets::row_title(theme, "What agents are told"))
                                .child(widgets::meta_line(
                                    theme,
                                    vec![
                                        div()
                                            .child(
                                                "Written from this table into each chat's \
                                                 delegation guidance. Updates on the next turn.",
                                            )
                                            .into_any_element(),
                                    ],
                                )),
                        )
                        .child(
                            div()
                                .id("routing-guidance")
                                .debug_selector(|| "routing-guidance".into())
                                .max_h(px(240.0))
                                .overflow_y_scroll()
                                .p(px(12.0))
                                .rounded(px(8.0))
                                .bg(theme.wash(0.04))
                                .border_1()
                                .border_color(widgets::row_divider(theme))
                                .font_family(theme.font_mono.clone())
                                .text_size(crate::typography::ui_rems(11.5))
                                .line_height(crate::typography::ui_rems(17.0))
                                .text_color(theme.text_muted)
                                .child(if guidance.is_empty() {
                                    "Loading…".to_string()
                                } else {
                                    guidance
                                }),
                        ),
                ),
        )
        .mt(px(28.0))
    }
}

impl popover::ScrollRailHost for RoutingPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for RoutingPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let mut column = widgets::page_column()
            .max_w(px(PAGE_WIDTH))
            .child(widgets::page_header(&theme, "Routing", None))
            .child(widgets::page_subtitle(
                &theme,
                "Which harness and model a delegated child runs on when the agent passes a role \
                 and no model. An explicit model in delegate_task always wins. Quota comes from \
                 the Usage page.",
            ))
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .mt(px(12.0))
                        .child(widgets::error_strip(&theme, error)),
                )
            });
        match self.settings.ready().cloned() {
            Some(settings) => {
                column = column
                    .child(self.render_roles(&theme, cx))
                    .child(self.render_out_of_quota(&settings, &theme, cx))
                    .child(self.render_native(&settings, &theme, cx));
            }
            None => {
                let note = match &self.settings {
                    Loadable::Error(error) => error.clone(),
                    _ => "Loading…".to_string(),
                };
                column = column.child(
                    widgets::section_card(&theme).child(
                        widgets::card_row(&theme, true).child(
                            div()
                                .text_color(theme.text_muted)
                                .child(SharedString::from(note)),
                        ),
                    ),
                );
            }
        }
        let scrollbar = popover::rail(self, "routing-scrollbar", &theme, cx);
        div()
            .id("routing-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("routing-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(column),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}
