use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RailMode {
    Full,
    Narrow,
    Strip,
}

impl RailMode {
    pub(super) fn auto(page: f32, pref: Option<bool>) -> Self {
        let window = page + crate::app_rail::WIDTH;
        if page <= 0.0 || window >= 1180.0 {
            return RailMode::Full;
        }
        match pref {
            Some(false) => RailMode::Strip,
            Some(true) => RailMode::Narrow,
            None if window >= 860.0 => RailMode::Narrow,
            None => RailMode::Strip,
        }
    }

    pub(super) fn width(self) -> f32 {
        match self {
            RailMode::Full => 384.0,
            RailMode::Narrow => 300.0,
            RailMode::Strip => 52.0,
        }
    }
}

pub(super) struct RailMotion {
    pub from: RailMode,
    pub to: RailMode,
    pub at: Option<Instant>,
}

pub(super) struct RailFrame {
    pub width: f32,
    pub shown: RailMode,
    pub opacity: f32,
    pub moving: bool,
}

impl RailMotion {
    pub(super) fn frame(&self, now: Instant) -> RailFrame {
        let Some(at) = self.at else {
            return RailFrame {
                width: self.to.width(),
                shown: self.to,
                opacity: 1.0,
                moving: false,
            };
        };
        let scale = motion::speed_scale();
        let ms = |spec: MotionSpec| spec.total().mul_f32(scale).as_secs_f32();
        let (out, width, fade) = (ms(RAIL_OUT), ms(RAIL_WIDTH), ms(RAIL_IN));
        let t = now.saturating_duration_since(at).as_secs_f32();
        let phase = |start: f32, len: f32, spec: MotionSpec| {
            spec.progress(((t - start) / len.max(0.001)).clamp(0.0, 1.0))
        };
        let grow = phase(out, width, RAIL_WIDTH);
        let mix = self.from.width() + (self.to.width() - self.from.width()) * grow;
        if t < out {
            return RailFrame {
                width: mix,
                shown: self.from,
                opacity: 1.0 - phase(0.0, out, RAIL_OUT),
                moving: true,
            };
        }
        RailFrame {
            width: mix,
            shown: self.to,
            opacity: phase(out + width, fade, RAIL_IN),
            moving: t < out + width + fade,
        }
    }
}

pub(super) fn mission_tone(mission: &Mission, theme: &Theme) -> Hsla {
    let tone = |indicator| crate::shell::chat_status_tone(indicator, theme).2;
    match mission.status {
        TaskStatus::NeedsYou => tone(ChatIndicator::AwaitingInput),
        TaskStatus::Working => tone(ChatIndicator::Working),
        _ => tone(ChatIndicator::Completed),
    }
}

fn rail_button(
    theme: &Theme,
    key: &'static str,
    label: &'static str,
    glyph: &'static str,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(key)
        .debug_selector(move || key.into())
        .role(gpui::Role::Button)
        .aria_label(label)
        .tooltip(widgets::text_tooltip(label))
        .flex_none()
        .size(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|s| s.bg(theme.wash(0.08)))
        .child(icon(glyph).size(px(14.0)).text_color(theme.text_muted))
}

fn rail_header(button: impl IntoElement) -> gpui::Div {
    div()
        .flex_none()
        .h(px(26.0))
        .flex()
        .items_center()
        .child(button)
}

impl SupervisorPage {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_rail(
        &self,
        frame: &RailFrame,
        feed: &Feed,
        missions: &[Mission],
        watched: usize,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let inner = match frame.shown {
            RailMode::Strip => self.render_strip(feed, missions, watched, now, theme, cx),
            mode => div()
                .id("supervisor-rail-scroll")
                .size_full()
                .overflow_y_scroll()
                .child(
                    div()
                        .w(px(mode.width()))
                        .pt(px(Theme::TITLEBAR_HEIGHT + 16.0))
                        .px(px(16.0))
                        .pb(px(20.0))
                        .flex()
                        .flex_col()
                        .when(mode == RailMode::Narrow, |el| {
                            el.child(
                                rail_header(
                                    rail_button(
                                        theme,
                                        "supervisor-rail-collapse",
                                        "Collapse the missions rail",
                                        icons::LUCIDE_CHEVRON_RIGHT,
                                    )
                                    .on_click(cx.listener(
                                        |page, _: &ClickEvent, _, cx| page.toggle_rail(cx),
                                    )),
                                )
                                .ml(px(-2.0))
                                .mb(px(14.0)),
                            )
                        })
                        .child(self.render_status(
                            watched,
                            mode == RailMode::Narrow,
                            now,
                            theme,
                            cx,
                        ))
                        .children(self.render_missions(
                            missions,
                            mode == RailMode::Narrow,
                            now,
                            theme,
                            cx,
                        )),
                )
                .into_any_element(),
        };
        div()
            .id("supervisor-rail")
            .debug_selector(|| "supervisor-rail".into())
            .relative()
            .flex_none()
            .h_full()
            .w(px(frame.width))
            .border_l_1()
            .border_color(widgets::row_divider(theme))
            .child(
                div()
                    .size_full()
                    .overflow_hidden()
                    .opacity(frame.opacity)
                    .child(inner),
            )
            .into_any_element()
    }

    fn render_strip(
        &self,
        feed: &Feed,
        missions: &[Mission],
        watched: usize,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (mode, tone) = {
            let state = self.state.read(cx);
            let mode = mode(state.supervisor.as_ref(), &self.ensure);
            let tone = status_tone(&mode, theme);
            (mode, tone)
        };
        let sep = || div().w(px(20.0)).h(px(1.0)).bg(theme.border_strong);
        let rings = missions
            .iter()
            .filter(|mission| mission.done_at.is_none())
            .map(|mission| {
                let id = mission.key.clone();
                let open = feed
                    .open
                    .iter()
                    .filter(|row| {
                        row.kind == Kind::Needs && feed::task_of(&row.entry) == Some(id.as_str())
                    })
                    .count();
                let tick = self
                    .groups
                    .get(&id)
                    .map_or(1.0, |motion| motion.tick.tween.value_at(now));
                let selector = format!("supervisor-strip-{id}");
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .role(gpui::Role::Button)
                    .aria_label(format!("{}, {open} to answer", mission.title))
                    .tooltip(widgets::text_tooltip(mission.title.clone()))
                    .relative()
                    .cursor_pointer()
                    .child(crate::loaders::upload_progress_arc(
                        (mission.fraction() * 100.0).round() as u8,
                        20.0,
                        theme.text_faint.opacity(0.35),
                        mission_tone(mission, theme),
                    ))
                    .when(open > 0, |el| {
                        el.child(
                            div()
                                .absolute()
                                .right(px(-6.0))
                                .top(px(-5.0))
                                .min_w(px(14.0))
                                .h(px(14.0))
                                .px(px(3.0))
                                .rounded(px(7.0))
                                .bg(needs_tone(theme))
                                .flex()
                                .items_center()
                                .justify_center()
                                .overflow_hidden()
                                .text_size(px(9.5))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(gpui::rgb(0x111111))
                                .child(
                                    div()
                                        .relative()
                                        .top(px(10.0 * (1.0 - tick)))
                                        .child(open.to_string()),
                                ),
                        )
                    })
                    .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.toggle_rail(cx)))
            })
            .collect::<Vec<_>>();
        div()
            .id("supervisor-rail-strip")
            .debug_selector(|| "supervisor-rail-strip".into())
            .w(px(RailMode::Strip.width()))
            .pt(px(Theme::TITLEBAR_HEIGHT + 16.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.0))
            .child(rail_header(
                rail_button(
                    theme,
                    "supervisor-rail-expand",
                    "Show missions",
                    icons::LUCIDE_CHEVRON_LEFT,
                )
                .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.toggle_rail(cx))),
            ))
            .child(
                div()
                    .id("supervisor-strip-status")
                    .tooltip(widgets::text_tooltip(format!(
                        "Watching {watched} {}",
                        plural(watched, "tree", "trees")
                    )))
                    .child(self.render_dot(&mode, tone, theme, now, cx)),
            )
            .child(sep())
            .children(rings)
            .into_any_element()
    }

    fn render_owners(
        &self,
        mission: &Mission,
        narrow: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let task_id = mission.key.as_str();
        let tree = self.trees.get(task_id).map(|(tree, _)| tree.clone());
        let open_id = task_id.to_owned();
        let open_key = format!("supervisor-open-center-{task_id}");
        let open = div()
            .id(SharedString::from(open_key.clone()))
            .debug_selector(move || open_key.clone())
            .role(gpui::Role::Button)
            .aria_label("Open Mission Control")
            .flex_none()
            .cursor_pointer()
            .text_size(ui_rems(11.5))
            .text_color(theme.text_muted)
            .hover(|s| s.text_color(theme.text))
            .child("Open Mission Control ›")
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.emit(SupervisorEvent::OpenTaskCenter(open_id.clone()))
            }));
        let footer = |el: gpui::Div| {
            el.flex()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .pl(px(37.0))
                .pr(px(12.0))
                .py(px(7.0))
                .border_t_1()
                .border_color(widgets::row_divider(theme))
                .text_size(ui_rems(11.5))
                .text_color(theme.text_muted)
        };
        let watch = match &mission.watch {
            _ if mission.plan && mission.done_at.is_some() => div()
                .min_w_0()
                .truncate()
                .child("Done · the Supervisor stopped watching it"),
            Watch::Elsewhere(device) => {
                let name = self
                    .state
                    .read(cx)
                    .device_name(device)
                    .unwrap_or("another device")
                    .to_owned();
                div().child(format!("on {name}, not watched"))
            }
            Watch::Watched if !mission.plan && self.watched_by_everything(mission, cx) => div()
                .debug_selector(|| "supervisor-watch-everything-note".into())
                .min_w_0()
                .truncate()
                .child("Watched \u{b7} Watch everything is on"),
            watch => {
                let on = *watch == Watch::Watched;
                let params = watch_params(mission, !on);
                let key = mission.key.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(match (on, narrow) {
                        (true, true) => "Watched",
                        (true, false) => "Watched by the Supervisor",
                        (false, _) => "Not watched",
                    })
                    .child(
                        Self::render_switch(
                            theme,
                            format!("supervisor-watch-{task_id}"),
                            format!("Watch {}", mission.title).into(),
                            on,
                        )
                        .on_click(cx.listener(
                            move |page, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                page.send_watch(key.clone(), params.clone(), cx);
                            },
                        )),
                    )
            }
        };
        let tail = if narrow {
            footer(div()).child(open).child(watch).into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .child(div().pl(px(37.0)).pt(px(3.0)).pb(px(6.0)).child(open))
                .child(footer(div()).child(watch))
                .into_any_element()
        };
        div()
            .border_t_1()
            .border_color(widgets::row_divider(theme))
            .pt(px(4.0))
            .flex()
            .flex_col()
            .children(tree)
            .child(tail)
            .children(
                self.watch_error_line(task_id, theme)
                    .map(|line| line.pl(px(37.0)).pr(px(12.0)).pb(px(7.0))),
            )
            .into_any_element()
    }

    fn render_mission(
        &self,
        mission: &Mission,
        narrow: bool,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = &mission.key;
        let arc = mission_tone(mission, theme);
        let motion = self.missions.get(key);
        let fill = motion.map_or(mission.fraction(), |motion| motion.fill.value_at(now));
        let turn = motion.map_or(0.0, |motion| motion.chevron.value_at(now));
        let expanded = self.mission_open(mission);
        let lit = self.hovered.as_deref() == Some(key.as_str());
        let glow = self
            .glows
            .get(key)
            .filter(|tween| !tween.settled_at(now))
            .map(|tween| tween.value_at(now));
        let unwatched = match &mission.watch {
            _ if mission.done_at.is_some() => None,
            Watch::Watched => None,
            Watch::Unwatched => Some("Not watched".to_owned()),
            Watch::Elsewhere(device) => Some(format!(
                "on {}, not watched",
                self.state
                    .read(cx)
                    .device_name(device)
                    .unwrap_or("another device")
            )),
        };
        let header_key = format!("supervisor-mission-{}", key);
        let toggle_id = key.clone();
        let header = div()
            .id(SharedString::from(header_key.clone()))
            .debug_selector(move || header_key.clone())
            .role(gpui::Role::Button)
            .aria_label(mission.title.clone())
            .aria_expanded(expanded)
            .flex()
            .items_center()
            .gap(px(9.0))
            .px(px(12.0))
            .py(px(9.0))
            .cursor_pointer()
            .hover(|s| s.bg(theme.wash(0.03)))
            .child(div().flex_none().child(crate::loaders::upload_progress_arc(
                (fill * 100.0).round() as u8,
                16.0,
                theme.text_faint.opacity(0.35),
                arc,
            )))
            .child(
                div()
                    .min_w_0()
                    .when(narrow, |el| el.flex_1())
                    .truncate()
                    .text_size(ui_rems(13.0))
                    .text_color(theme.text)
                    .child(mission.title.clone()),
            )
            .child(crate::tasks::kind_badge(mission.plan, theme))
            .when(!narrow, |el| {
                el.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(ui_rems(11.5))
                        .text_color(theme.text_muted)
                        .child(mission.current.clone()),
                )
            })
            .children(unwatched.map(|label| {
                div()
                    .flex_none()
                    .text_size(ui_rems(11.0))
                    .text_color(theme.text_faint)
                    .child(label)
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(if narrow { 40.0 } else { 64.0 }))
                    .h(px(3.0))
                    .rounded(px(2.0))
                    .bg(theme.wash(0.1))
                    .overflow_hidden()
                    .child(div().h_full().w(relative(fill)).bg(arc)),
            )
            .child(chevron(theme, turn * 90.0))
            .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                page.set_mission_open(&toggle_id, !expanded, cx)
            }));
        let body = motion
            .filter(|motion| !motion.fold.gone(now))
            .map(|motion| {
                folded(
                    format!("supervisor-mission-fold-{}", key),
                    &motion.fold,
                    now,
                    div()
                        .opacity(motion.fold.value(now))
                        .child(self.render_owners(mission, narrow, theme, cx)),
                )
            });
        let amber = needs_tone(theme);
        let selector = format!("supervisor-mission-card-{}", key);
        div()
            .debug_selector(move || selector.clone())
            .relative()
            .rounded(px(11.0))
            .border_1()
            .border_color(if lit {
                amber.opacity(0.45)
            } else {
                theme.border
            })
            .bg(widgets::block_fill(theme))
            .flex()
            .flex_col()
            .child(
                div()
                    .rounded(px(11.0))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(header)
                    .children(body),
            )
            .children(glow.map(|t| {
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let spread = px(4.0);
                        let ring = gpui::Bounds::new(
                            gpui::point(bounds.origin.x - spread, bounds.origin.y - spread),
                            gpui::size(
                                bounds.size.width + spread * 2.0,
                                bounds.size.height + spread * 2.0,
                            ),
                        );
                        window.paint_quad(gpui::quad(
                            ring,
                            px(11.0) + spread,
                            gpui::transparent_black(),
                            spread,
                            amber.opacity(0.45 * (1.0 - t)),
                            gpui::BorderStyle::default(),
                        ));
                    },
                )
                .absolute()
                .inset_0()
            }))
            .into_any_element()
    }

    fn render_missions(
        &self,
        missions: &[Mission],
        narrow: bool,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if missions.is_empty() {
            return None;
        }
        let (done, active): (Vec<&Mission>, Vec<&Mission>) = missions
            .iter()
            .partition(|mission| mission.done_at.is_some());
        let mut block = div()
            .id("supervisor-missions")
            .debug_selector(|| "supervisor-missions".into())
            .mt(px(16.0))
            .flex()
            .flex_col()
            .gap(px(7.0));
        if !active.is_empty() {
            block = block
                .child(caps_label(theme, format!("Trees · {}", active.len())))
                .children(
                    active
                        .iter()
                        .map(|mission| self.render_mission(mission, narrow, now, theme, cx)),
                );
        }
        if !done.is_empty() {
            block = block
                .child(
                    div()
                        .debug_selector(|| "supervisor-done-today".into())
                        .when(!active.is_empty(), |el| el.mt(px(11.0)))
                        .child(caps_label(theme, format!("Done today · {}", done.len()))),
                )
                .children(
                    done.iter()
                        .map(|mission| self.render_mission(mission, narrow, now, theme, cx)),
                );
        }
        Some(block.into_any_element())
    }
}
