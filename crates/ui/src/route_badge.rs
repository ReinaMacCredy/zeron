use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gpui::{
    AnyElement, App, Context, ElementId, Hsla, Render, SharedString, Window, div, prelude::*, px,
};
use zeron_proto::{Chat, ChildRoute, RouteTarget};

use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    Routed,
    ChosenByAgent,
    FellBack,
}

impl Source {
    pub(crate) fn of(route: &ChildRoute) -> Option<Self> {
        match route.source.as_str() {
            ChildRoute::ROUTED => Some(Self::Routed),
            ChildRoute::CHOSEN_BY_AGENT => Some(Self::ChosenByAgent),
            ChildRoute::FELL_BACK => Some(Self::FellBack),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteLink {
    Routing,
    Usage,
}

pub(crate) type OnRouteLink = Rc<dyn Fn(RouteLink, &mut Window, &mut App)>;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RouteInfo {
    pub route: ChildRoute,
    pub ran: Option<RouteTarget>,
    pub parent: Option<RouteTarget>,
    pub native: bool,
}

impl RouteInfo {
    pub(crate) fn for_child(chats: &[Chat], child_id: &str, route: ChildRoute) -> Self {
        let child = chats.iter().find(|chat| chat.id == child_id);
        let parent_id =
            child.and_then(|chat| chat.delegated_by().or(chat.created_by_chat_id.as_deref()));
        Self {
            route,
            ran: child.and_then(|chat| chat.config.as_ref()).map(target_of),
            parent: parent_id
                .and_then(|id| chats.iter().find(|chat| chat.id == id))
                .and_then(|chat| chat.config.as_ref())
                .map(target_of),
            native: false,
        }
    }
}

pub(crate) fn target_of(config: &zeron_proto::ChatConfig) -> RouteTarget {
    RouteTarget {
        harness: config.harness,
        model: config.model.clone(),
        reasoning: config.reasoning,
    }
}

pub(crate) fn fell_back_tint(theme: &Theme) -> Hsla {
    theme.terminal.ansi[6]
}

pub(crate) fn same_model(a: &RouteTarget, b: &RouteTarget) -> bool {
    a.harness == b.harness && a.model == b.model
}

pub(crate) fn clock(ms: i64) -> Option<String> {
    let at = Utc.timestamp_millis_opt(ms).single()?;
    crate::settings::accounts::format_reset(Some(at), Utc::now())
        .map(|reset| reset.trim_start_matches("resets ").to_string())
}

fn short_reason(route: &ChildRoute) -> String {
    let reason = route.reason.as_deref().unwrap_or("route unavailable");
    let short = match reason.strip_suffix(" out of quota") {
        Some(harness) => format!("{harness} quota"),
        None => match reason.strip_suffix(" is not available") {
            Some(harness) => format!("{harness} unavailable"),
            None => reason.to_string(),
        },
    };
    match route.resets_at.and_then(clock) {
        Some(at) => format!("{short}, resets {at}"),
        None => short,
    }
}

pub(crate) fn badge_label(route: &ChildRoute) -> Option<SharedString> {
    Some(match Source::of(route)? {
        Source::Routed => "routed".into(),
        Source::ChosenByAgent => "chosen by agent".into(),
        Source::FellBack => format!("fell back ({})", short_reason(route)).into(),
    })
}

fn badge_tint(source: Source, theme: &Theme) -> Hsla {
    match source {
        Source::Routed => theme.accent,
        Source::ChosenByAgent => theme.warning,
        Source::FellBack => fell_back_tint(theme),
    }
}

pub(crate) fn target_text(target: &RouteTarget, cx: &App) -> String {
    let mut parts = vec![target.harness.display_name().to_string()];
    if let Some(model) = &target.model {
        parts.push(crate::pickers::model_display_label(
            target.harness,
            model,
            cx,
        ));
    }
    if let Some(reasoning) = target.reasoning {
        parts.push(crate::pickers::reasoning_label(reasoning).to_string());
    }
    parts.join(" · ")
}

pub(crate) fn brand_mark(target: &RouteTarget, size: f32, theme: &Theme) -> AnyElement {
    let (glyph, tint) =
        crate::pickers::agent_brand_icon(Some(target.harness), target.model.as_deref());
    crate::icons::icon(glyph)
        .size(px(size))
        .flex_none()
        .text_color(tint.unwrap_or(theme.text_muted))
        .into_any_element()
}

pub(crate) fn route_badge(
    id: impl Into<ElementId>,
    info: RouteInfo,
    on_link: OnRouteLink,
    theme: &Theme,
) -> Option<AnyElement> {
    let source = Source::of(&info.route)?;
    let label = badge_label(&info.route)?;
    let tint = badge_tint(source, theme);
    Some(
        div()
            .id(id)
            .debug_selector(|| "route-badge".into())
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .flex_none()
            .min_w_0()
            .truncate()
            .px(px(6.0))
            .rounded(px(4.0))
            .bg(tint.opacity(0.14))
            .text_size(crate::typography::ui_rems(10.5))
            .text_color(tint)
            .child(label)
            .on_click(|_, _, cx| cx.stop_propagation())
            .hoverable_tooltip(move |_, cx| {
                let info = info.clone();
                let on_link = on_link.clone();
                cx.new(|_| RoutePopover { info, on_link }).into()
            })
            .into_any_element(),
    )
}

struct RoutePopover {
    info: RouteInfo,
    on_link: OnRouteLink,
}

fn info_row(label: &'static str, value: AnyElement, theme: &Theme) -> AnyElement {
    div()
        .px(px(8.0))
        .py(px(3.0))
        .flex()
        .flex_row()
        .items_start()
        .gap(px(12.0))
        .text_size(crate::typography::ui_rems(12.0))
        .child(
            div()
                .w(px(64.0))
                .flex_none()
                .text_color(theme.text_muted)
                .child(label),
        )
        .child(div().flex_1().min_w_0().text_color(theme.text).child(value))
        .into_any_element()
}

fn text(value: impl Into<SharedString>) -> AnyElement {
    div().child(value.into()).into_any_element()
}

fn branded(target: &RouteTarget, value: String, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(5.0))
        .child(div().pt(px(2.0)).child(brand_mark(target, 12.0, theme)))
        .child(div().min_w_0().child(value))
        .into_any_element()
}

fn note(value: impl Into<SharedString>, theme: &Theme) -> AnyElement {
    div()
        .px(px(8.0))
        .pt(px(4.0))
        .pb(px(2.0))
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted)
        .child(value.into())
        .into_any_element()
}

impl RoutePopover {
    fn link(
        &self,
        id: &'static str,
        label: &'static str,
        which: RouteLink,
        theme: &Theme,
    ) -> AnyElement {
        let on_link = self.on_link.clone();
        div()
            .id(id)
            .role(gpui::Role::Link)
            .aria_label(label)
            .cursor_pointer()
            .text_color(theme.text_muted)
            .underline()
            .hover(|s| s.text_color(theme.text))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_link(which, window, cx);
            })
            .child(label)
            .into_any_element()
    }

    fn ran_as(&self, theme: &Theme, cx: &App) -> Option<AnyElement> {
        let ran = self.info.ran.as_ref()?;
        let on_parent = self
            .info
            .parent
            .as_ref()
            .is_some_and(|parent| same_model(parent, ran));
        let mut value = if self.info.native {
            let model = ran
                .model
                .as_deref()
                .map(|model| crate::pickers::model_display_label(ran.harness, model, cx));
            match model {
                Some(model) => format!("{} native sub-agent, {model}", ran.harness.display_name()),
                None => format!("{} native sub-agent", ran.harness.display_name()),
            }
        } else {
            target_text(ran, cx)
        };
        if Source::of(&self.info.route) == Some(Source::FellBack) && (on_parent || self.info.native)
        {
            value.push_str(" (this chat's model)");
        }
        Some(branded(ran, value, theme))
    }
}

impl Render for RoutePopover {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_popup();
        let route = &self.info.route;
        let source = Source::of(route).unwrap_or(Source::Routed);
        let role = route.role.clone().unwrap_or_else(|| "general".into());
        let quota = route
            .reason
            .as_deref()
            .is_some_and(|reason| reason.ends_with(" out of quota"));
        let (icon, title): (&str, String) = match source {
            Source::Routed => (crate::icons::FORK, "Routed by Settings > Routing".into()),
            Source::ChosenByAgent => (crate::icons::LUCIDE_USER_ROUND, "Chosen by agent".into()),
            Source::FellBack => (
                crate::icons::FORK,
                format!(
                    "Fell back: {}",
                    route.reason.as_deref().unwrap_or("route unavailable")
                ),
            ),
        };
        let mut body: Vec<AnyElement> = Vec::new();
        match source {
            Source::Routed => {
                body.push(info_row("Role", text(role.clone()), &theme));
                if let Some(ran) = self.ran_as(&theme, cx) {
                    body.push(info_row("Runs on", ran, &theme));
                }
                body.push(info_row("Scope", text("Global default"), &theme));
                body.push(note("Override: pass model in delegate_task.", &theme));
            }
            Source::ChosenByAgent => {
                body.push(info_row("Role", text(role.clone()), &theme));
                if let Some(ran) = self.ran_as(&theme, cx) {
                    body.push(info_row("Runs on", ran, &theme));
                }
                if let Some(table) = &route.table {
                    body.push(info_row(
                        "Route was",
                        branded(table, target_text(table, cx), &theme),
                        &theme,
                    ));
                }
                let reason = route.reason.as_deref().unwrap_or("The lead passed model");
                body.push(note(
                    format!("{reason}, which wins over the route."),
                    &theme,
                ));
            }
            Source::FellBack => {
                if let Some(table) = &route.table {
                    body.push(info_row(
                        "Route",
                        branded(table, target_text(table, cx), &theme),
                        &theme,
                    ));
                }
                if quota {
                    let value = match route.resets_at.and_then(clock) {
                        Some(at) => format!("Used up · resets {at}"),
                        None => "Used up".to_string(),
                    };
                    body.push(info_row("Quota", text(value), &theme));
                }
                if let Some(ran) = self.ran_as(&theme, cx) {
                    body.push(info_row("Ran as", ran, &theme));
                }
                let on_parent = match (&self.info.ran, &self.info.parent) {
                    (Some(ran), Some(parent)) => same_model(ran, parent),
                    _ => false,
                };
                let rule = if !quota {
                    "Harness not available: runs on this chat's model"
                } else if self.info.native {
                    "When out of quota: Native sub-agent"
                } else if on_parent {
                    "When out of quota: Native sub-agent, on this chat's model"
                } else {
                    "When out of quota: Next in fallback list"
                };
                body.push(info_row("Rule", text(rule), &theme));
                if quota
                    && let (Some(at), Some(table)) =
                        (route.resets_at.and_then(clock), route.table.as_ref())
                {
                    body.push(note(
                        format!(
                            "After {at}, {role} routes to {} again.",
                            table.harness.display_name()
                        ),
                        &theme,
                    ));
                }
            }
        }
        let links = div()
            .px(px(8.0))
            .pt(px(6.0))
            .pb(px(4.0))
            .flex()
            .flex_row()
            .gap(px(12.0))
            .text_size(crate::typography::ui_rems(12.0))
            .child(self.link(
                "route-open-routing",
                "Open Routing",
                RouteLink::Routing,
                &theme,
            ))
            .when(source == Source::FellBack, |el| {
                el.child(self.link("route-open-usage", "Usage", RouteLink::Usage, &theme))
            });
        let card = crate::popover::popover_card(&theme)
            .w(px(300.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(8.0))
                    .pt(px(2.0))
                    .pb(px(6.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(crate::typography::ui_rems(12.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(
                        crate::icons::icon(icon)
                            .size(px(13.0))
                            .flex_none()
                            .text_color(theme.text_muted),
                    )
                    .child(div().min_w_0().child(title)),
            )
            .children(body)
            .child(div().mt(px(6.0)).child(crate::popover::menu_separator()))
            .child(links);
        crate::frost::frosted(crate::popover::CARD_RADIUS, crate::frost::MENU_BLUR, card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(source: &str, reason: Option<&str>) -> ChildRoute {
        ChildRoute {
            source: source.into(),
            role: Some("worker".into()),
            reason: reason.map(Into::into),
            ..ChildRoute::default()
        }
    }

    #[test]
    fn badge_reads_the_source_and_drops_unknown_values() {
        assert_eq!(
            badge_label(&route("routed", None)).as_deref(),
            Some("routed")
        );
        assert_eq!(
            badge_label(&route("chosen_by_agent", None)).as_deref(),
            Some("chosen by agent")
        );
        assert_eq!(
            badge_label(&route("fell_back", Some("Codex out of quota"))).as_deref(),
            Some("fell back (Codex quota)")
        );
        assert_eq!(
            badge_label(&route("fell_back", Some("Grok is not available"))).as_deref(),
            Some("fell back (Grok unavailable)")
        );
        assert_eq!(badge_label(&route("future_value", None)), None);
    }
}
