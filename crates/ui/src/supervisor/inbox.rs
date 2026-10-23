use super::*;

const Q_WIDE: f32 = 190.0;
const Q_NARROW: f32 = 140.0;
const HAIRLINE_LEFT: f32 = 38.0;
const ROWS_FIT: f32 = 860.0;
const COLUMN_PADDING: f32 = 82.0;
const DETAIL_MIN: f32 = 72.0;
const WIDE_ROW: f32 = 124.0;
const WIDE_INTERVENTION: f32 = 330.0;
const WIDE_FOOTER: f32 = 440.0;

pub(super) fn sheet_row(entry: &Entry) -> Option<&NeedsYouRow> {
    match entry {
        Entry::Needs(row @ NeedsYouRow::Item(_)) => Some(row),
        Entry::Needs(row @ NeedsYouRow::Input(input))
            if !input.questions.is_empty()
                && input.questions.iter().all(|q| !q.options.is_empty()) =>
        {
            Some(row)
        }
        _ => None,
    }
}

fn sheet_lines(row: &NeedsYouRow) -> usize {
    match row {
        NeedsYouRow::Item(_) => 1,
        NeedsYouRow::Input(input) => input.questions.len(),
    }
}

pub(super) fn strip_mission(title: &str, mission: Option<&str>) -> String {
    let Some(rest) = mission.and_then(|mission| title.strip_prefix(&format!("{mission}: "))) else {
        return title.to_owned();
    };
    let mut chars = rest.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => title.to_owned(),
    }
}

fn item_quote(item: &SupervisorItem, mission: Option<&str>) -> String {
    let raised = match item.raised_by {
        Raiser::Lead => "Raised by Lead",
        Raiser::Supervisor => "Raised by Supervisor",
    };
    let meta = match mission {
        Some(mission) => format!("{raised} · {mission}"),
        None => raised.to_owned(),
    };
    let mut quote = vec![
        format!("Needs you: {}", item.title),
        format!("{meta} · {}", item.why.label()),
    ];
    if !item.detail.trim().is_empty() {
        quote.push(item.detail.trim().to_owned());
    }
    if !item.options.is_empty() {
        quote.push(format!("Options: {}", item.options.join(" | ")));
    }
    quote.join("\n")
}

fn input_quote(input: &NeedsYouInput, who: &str) -> String {
    let mut quote = vec![
        format!("Needs you: {}", input.title()),
        format!("Asked by {who}"),
    ];
    for question in &input.questions {
        quote.push(match question.options.as_slice() {
            [] => crate::transcript::single_line(&question.question),
            options => format!(
                "{} Options: {}",
                crate::transcript::single_line(&question.question),
                options.join(" | ")
            ),
        });
    }
    quote.join("\n")
}

fn sel_of(key: &str) -> String {
    key.replace(':', "-")
}

fn mark(child: impl IntoElement) -> gpui::Div {
    div()
        .flex_none()
        .w(px(14.0))
        .flex()
        .items_center()
        .justify_center()
        .child(child)
}

fn unread(theme: &Theme) -> gpui::Div {
    div().size(px(7.0)).rounded_full().bg(needs_tone(theme))
}

fn time_cell(theme: &Theme, text: String) -> gpui::Div {
    div()
        .flex_none()
        .w(px(38.0))
        .flex()
        .justify_end()
        .text_size(ui_rems(11.5))
        .text_color(theme.text_faint)
        .child(text)
}

#[derive(Clone)]
enum Pick {
    Stage {
        key: String,
        group: String,
        question: String,
        multi: bool,
    },
    Correct {
        item_id: String,
    },
}

fn faint(theme: &Theme, text: String) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(ui_rems(12.0))
        .text_color(theme.text_faint)
        .child(text)
}

fn hairline(theme: &Theme) -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .left(px(HAIRLINE_LEFT))
        .right_0()
        .h(px(1.0))
        .bg(widgets::row_divider(theme))
}

fn icon_button(
    theme: &Theme,
    key: String,
    label: &'static str,
    name: &'static str,
) -> gpui::Stateful<gpui::Div> {
    let selector = key.clone();
    div()
        .id(SharedString::from(key))
        .debug_selector(move || selector.clone())
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
        .child(icon(name).size(px(14.0)).text_color(theme.text_muted))
}

impl SupervisorPage {
    pub(super) fn compact(&self) -> bool {
        self.rail
            .as_ref()
            .is_some_and(|rail| rail.to == RailMode::Narrow)
            && self.width.get() + crate::app_rail::WIDTH < ROWS_FIT
    }

    fn inbox_width(&self) -> f32 {
        let rail = match &self.rail {
            Some(rail) => rail.to,
            None => RailMode::auto(self.width.get(), self.rail_pref),
        };
        (self.width.get() - rail.width() - COLUMN_PADDING).min(PAGE_WIDTH - COLUMN_PADDING)
    }

    fn fits(&self, options: &[String], beside: f32) -> bool {
        self.inbox_width() >= segments_width(options) + beside
    }

    fn detail_fits(&self, options: &[String]) -> bool {
        self.fits(options, HAIRLINE_LEFT + 30.0 + DETAIL_MIN)
    }

    pub(super) fn render_tabs(
        &self,
        feed: &Feed,
        now: Instant,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let total = feed.open.len();
        let mut tabs = vec![(None, total)];
        tabs.extend(
            Kind::CHIPS
                .iter()
                .map(|&kind| (Some(kind), feed.count(kind))),
        );
        let origin = self.tabs.clone();
        let underline = self.underline_rect(now);
        div()
            .id("supervisor-chips")
            .debug_selector(|| "supervisor-chips".into())
            .role(gpui::Role::TabList)
            .relative()
            .mt(px(16.0))
            .flex()
            .gap(px(20.0))
            .border_b_1()
            .border_color(widgets::row_divider(&theme))
            .children(
                tabs.into_iter()
                    .enumerate()
                    .map(|(ix, (kind, count))| self.render_tab(ix, kind, count, now, &theme, cx)),
            )
            .children(underline.map(|(x, w)| {
                div()
                    .absolute()
                    .bottom(px(-1.0))
                    .left(px(x))
                    .w(px(w))
                    .h(px(2.0))
                    .bg(theme.text)
            }))
            .child(
                canvas(
                    move |bounds, window, _| {
                        let x = f32::from(bounds.origin.x);
                        let mut probe = origin.borrow_mut();
                        if (probe.origin - x).abs() > 0.5 {
                            probe.origin = x;
                            window.request_animation_frame();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }

    fn render_tab(
        &self,
        ix: usize,
        kind: Option<Kind>,
        count: usize,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = match kind {
            None => self.filter == Filter::All,
            Some(kind) => self.filter == Filter::Only(kind),
        };
        let label = kind.map_or("All", Kind::chip);
        let key = format!("supervisor-chip-{}", label.to_lowercase().replace(' ', "-"));
        let selector = key.clone();
        let tick = self
            .ticks
            .get(&kind)
            .map_or(1.0, |tick| tick.tween.value_at(now));
        let probe = self.tabs.clone();
        let tone = if kind == Some(Kind::Needs) && count > 0 {
            needs_tone(theme)
        } else {
            theme.text
        };
        div()
            .id(SharedString::from(key))
            .debug_selector(move || selector.clone())
            .role(gpui::Role::Tab)
            .aria_label(format!("{label}, {count}"))
            .aria_selected(selected)
            .relative()
            .flex()
            .items_center()
            .gap(px(5.0))
            .pb(px(8.0))
            .cursor_pointer()
            .text_size(ui_rems(12.5))
            .text_color(if selected {
                theme.text
            } else if count == 0 {
                theme.text_faint
            } else {
                theme.text_muted
            })
            .hover(|s| s.text_color(theme.text))
            .child(label)
            .when(count > 0, |el| {
                el.child(
                    div()
                        .h(ui_rems(14.0))
                        .line_height(ui_rems(14.0))
                        .overflow_hidden()
                        .text_size(ui_rems(11.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(tone)
                        .child(
                            div()
                                .relative()
                                .top(ui_rems(14.0 * (1.0 - tick)))
                                .opacity(tick)
                                .child(count.to_string()),
                        ),
                )
            })
            .child(
                canvas(
                    move |bounds, window, _| {
                        let rect = (f32::from(bounds.origin.x), f32::from(bounds.size.width));
                        let mut probe = probe.borrow_mut();
                        if probe.rects.len() <= ix {
                            probe.rects.resize(ix + 1, (0.0, 0.0));
                        }
                        let old = probe.rects[ix];
                        if (old.0 - rect.0).abs() > 0.5 || (old.1 - rect.1).abs() > 0.5 {
                            probe.rects[ix] = rect;
                            window.request_animation_frame();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| page.pick_filter(kind, cx)))
            .into_any_element()
    }

    pub(super) fn render_inbox(
        &self,
        feed: &Feed,
        missions: &[Mission],
        calm: (String, String),
        now: Instant,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let motions: Vec<&RowMotion> = self
            .rows
            .values()
            .filter(|motion| !motion.fold.gone(now))
            .collect();
        let groups = feed::grouped(motions.iter().map(|motion| &*motion.row));
        let narrow = self
            .rail
            .as_ref()
            .is_some_and(|rail| rail.to == RailMode::Narrow);
        let blocks: Vec<AnyElement> = groups
            .into_iter()
            .filter_map(|group| {
                let key = group.key();
                let fold = self.groups.get(&key)?;
                (!fold.fold.gone(now)).then(|| {
                    let body = self.render_group(&group, feed, missions, narrow, now, &theme, cx);
                    folded(
                        format!("supervisor-group-fold-{key}"),
                        &fold.fold,
                        now,
                        div().pb(px(12.0)).child(body),
                    )
                })
            })
            .collect();
        let calm = (!self.calm.gone(now)).then(|| {
            folded(
                "supervisor-calm-fold".into(),
                &self.calm,
                now,
                div()
                    .pb(px(12.0))
                    .child(render_calm(&theme, calm.0, calm.1)),
            )
        });
        div()
            .id("supervisor-feed")
            .debug_selector(|| "supervisor-feed".into())
            .mt(px(14.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .top(px(2.0 * (1.0 - self.list_opacity(now)).min(1.0)))
                    .opacity(self.list_opacity(now))
                    .flex()
                    .flex_col()
                    .children(blocks)
                    .children(calm),
            )
            .children(self.render_earlier(&feed.earlier, now, &theme, cx))
            .into_any_element()
    }

    fn list_opacity(&self, now: Instant) -> f32 {
        match self.list_switch {
            Some(at) => {
                let total = LIST_OUT
                    .total()
                    .mul_f32(motion::speed_scale())
                    .as_secs_f32();
                let raw = now.saturating_duration_since(at).as_secs_f32() / total.max(0.001);
                1.0 - LIST_OUT.progress(raw)
            }
            None => self.list_in.value_at(now),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_group(
        &self,
        group: &feed::Group,
        feed: &Feed,
        missions: &[Mission],
        narrow: bool,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = group.key();
        let mission = group
            .task
            .and_then(|task| missions.iter().find(|m| m.key == task));
        let title = match (group.task, mission) {
            (None, _) => "Supervisor and shared".to_owned(),
            (Some(_), Some(mission)) => mission.title.clone(),
            (Some(key), None) => {
                let state = self.state.read(cx);
                mission_title(state, Some(key))
                    .or_else(|| {
                        let root = state.chats.iter().find(|chat| chat.id == key)?;
                        Some(crate::transcript::single_line(&crate::home::chat_title(
                            root,
                        )))
                    })
                    .unwrap_or_else(|| "A mission".into())
            }
        };
        let open_needs = feed
            .open
            .iter()
            .filter(|row| row.kind == Kind::Needs && row.group.as_deref() == group.task)
            .count();
        let updates = feed
            .open
            .iter()
            .filter(|row| row.kind != Kind::Needs && row.group.as_deref() == group.task)
            .count();
        let tick = self
            .groups
            .get(&key)
            .map_or(1.0, |motion| motion.tick.tween.value_at(now));
        let glyph = match mission {
            Some(mission) => div()
                .flex_none()
                .child(crate::loaders::upload_progress_arc(
                    (mission.fraction() * 100.0).round() as u8,
                    13.0,
                    theme.text_faint.opacity(0.35),
                    mission_tone(mission, theme),
                ))
                .into_any_element(),
            None => icon(if group.task.is_none() {
                icons::LUCIDE_SHIELD
            } else {
                icons::LUCIDE_CIRCLE_DASHED
            })
            .size(px(13.0))
            .text_color(theme.text_muted)
            .into_any_element(),
        };
        let header = div()
            .debug_selector({
                let key = key.clone();
                move || format!("supervisor-group-{key}")
            })
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(34.0))
            .pl(px(14.0))
            .pr(px(12.0))
            .text_size(ui_rems(12.0))
            .text_color(theme.text_faint)
            .child(glyph)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_rems(12.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(title.clone()),
            )
            .when(open_needs > 0, |el| {
                el.child(
                    div()
                        .flex_none()
                        .flex()
                        .gap(px(4.0))
                        .text_color(needs_tone(theme))
                        .child(
                            div().h(ui_rems(16.0)).overflow_hidden().child(
                                div()
                                    .relative()
                                    .top(ui_rems(16.0 * (1.0 - tick)))
                                    .opacity(tick)
                                    .child(open_needs.to_string()),
                            ),
                        )
                        .child("to answer"),
                )
            })
            .when(updates > 0, |el| {
                el.child(div().flex_none().child(format!(
                    "{}{updates} {}",
                    if open_needs > 0 { "· " } else { "" },
                    plural(updates, "update", "updates")
                )))
            });
        let (needs, others): (Vec<&Row>, Vec<&Row>) =
            group.rows.iter().partition(|row| row.kind == Kind::Needs);
        let q_width = if narrow { Q_NARROW } else { Q_WIDE };
        let mut body = div()
            .rounded(px(12.0))
            .border_1()
            .border_color(theme.border)
            .bg(widgets::block_fill(theme))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(header);
        for (ix, row) in needs.iter().enumerate() {
            body = body.child(self.fold_row(row, ix > 0, q_width, &title, now, theme, cx));
        }
        let sheet: Vec<&NeedsYouRow> = needs
            .iter()
            .filter_map(|row| sheet_row(&row.entry))
            .filter(|row| {
                !self
                    .replies
                    .get(&row.key())
                    .is_some_and(|reply| !reply.failed())
            })
            .collect();
        if !sheet.is_empty() {
            body = body.child(self.render_footer(&key, &sheet, &title, theme, cx));
        }
        if !others.is_empty() {
            let mut list = div().flex().flex_col();
            if !needs.is_empty() {
                list = list.border_t_1().border_color(widgets::row_divider(theme));
            }
            for (ix, row) in others.iter().enumerate() {
                list = list.child(self.fold_row(row, ix > 0, q_width, &title, now, theme, cx));
            }
            body = body.child(list);
        }
        let task = group.task.map(str::to_owned);
        div()
            .id(SharedString::from(format!("supervisor-sheet-{key}")))
            .on_hover(cx.listener(move |page, hovered: &bool, _, cx| {
                page.hover_task(task.clone(), *hovered, cx)
            }))
            .child(body)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn fold_row(
        &self,
        row: &Row,
        line: bool,
        q_width: f32,
        mission: &str,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(motion) = self.rows.get(&row.key) else {
            return div().into_any_element();
        };
        let slide = motion
            .arrival
            .as_ref()
            .map_or(1.0, |arrival| arrival.slide.value_at(now));
        let body = self.render_row(row, false, q_width, Some(mission), now, theme, cx);
        folded(
            format!("supervisor-fold-{}", row.key),
            &motion.fold,
            now,
            div()
                .relative()
                .top(px(-4.0 * (1.0 - slide)))
                .when(line, |el| el.child(hairline(theme)))
                .child(body),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_row(
        &self,
        row: &Row,
        done: bool,
        q_width: f32,
        mission: Option<&str>,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match &row.entry {
            Entry::Needs(needs) => match self.replies.get(&row.key) {
                Some(reply) if !reply.failed() => {
                    self.render_answer(row, needs, reply, mission, now, theme, cx)
                }
                _ => match needs {
                    NeedsYouRow::Item(item) => self.render_item(row, item, q_width, theme, cx),
                    NeedsYouRow::Input(input) => {
                        self.render_input(row, input, q_width, mission, theme, cx)
                    }
                },
            },
            Entry::Question(question) => self.render_question(&row.key, question, done, theme, cx),
            Entry::Intervention(intervention) => {
                self.render_intervention(&row.key, intervention, done, now, theme, cx)
            }
            Entry::Resource(resource) => self.render_resource(&row.key, resource, done, theme, cx),
            Entry::Answered(answered) => self.render_answered(answered, theme, cx),
        }
    }

    fn row_shell(
        &self,
        key: &str,
        selector: String,
        label: String,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(SharedString::from(format!("supervisor-card-{key}")))
            .debug_selector(move || selector.clone())
            .group(SharedString::from(format!("supervisor-row-{key}")))
            .role(gpui::Role::Group)
            .aria_label(label)
            .relative()
            .flex()
            .items_center()
            .gap(px(12.0))
            .min_h(px(40.0))
            .pl(px(14.0))
            .pr(px(10.0))
            .hover(|s| s.bg(theme.wash(0.04)))
    }

    #[allow(clippy::too_many_arguments)]
    fn render_segments(
        &self,
        selector: &str,
        options: &[String],
        picked: &[String],
        pick: Pick,
        wrap: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let count = options.len();
        div()
            .flex_none()
            .flex()
            .items_center()
            .when(wrap, |el| el.flex_wrap().max_w_full().min_h(px(24.0)))
            .when(!wrap, |el| el.h(px(24.0)))
            .p(px(2.0))
            .rounded(px(7.0))
            .bg(theme.wash(0.06))
            .border_1()
            .border_color(theme.border)
            .children(options.iter().enumerate().map(|(ix, option)| {
                let chosen = picked.contains(option);
                let next_chosen = options
                    .get(ix + 1)
                    .is_some_and(|next| picked.contains(next));
                let (pick, option_value) = (pick.clone(), option.clone());
                let id = format!("{selector}-option-{ix}");
                let sel = id.clone();
                div()
                    .id(SharedString::from(id))
                    .debug_selector(move || sel.clone())
                    .role(gpui::Role::Button)
                    .aria_label(option.clone())
                    .aria_selected(chosen)
                    .relative()
                    .flex()
                    .items_center()
                    .when(wrap, |el| el.flex_none().max_w_full().min_h(px(18.0)))
                    .when(!wrap, |el| el.h(px(18.0)).whitespace_nowrap())
                    .px(px(9.0))
                    .rounded(px(5.0))
                    .cursor_pointer()
                    .text_size(ui_rems(12.0))
                    .when(chosen, |el| {
                        el.bg(theme.solid)
                            .text_color(theme.on_solid)
                            .font_weight(gpui::FontWeight::MEDIUM)
                    })
                    .when(!chosen, |el| {
                        el.text_color(theme.text).hover(|s| s.bg(theme.wash(0.10)))
                    })
                    .when(ix + 1 < count && !chosen && !next_chosen, |el| {
                        el.child(
                            div()
                                .absolute()
                                .right(px(0.0))
                                .top(px(4.0))
                                .bottom(px(4.0))
                                .w(px(1.0))
                                .bg(theme.border),
                        )
                    })
                    .child(option.clone())
                    .on_click(
                        cx.listener(move |page, _: &ClickEvent, window, cx| match &pick {
                            Pick::Stage {
                                key,
                                group,
                                question,
                                multi,
                            } => page.stage(
                                key.clone(),
                                group.clone(),
                                question.clone(),
                                option_value.clone(),
                                *multi,
                                window,
                                cx,
                            ),
                            Pick::Correct { item_id } if !chosen => {
                                page.correct(item_id.clone(), option_value.clone(), cx)
                            }
                            Pick::Correct { .. } => {}
                        }),
                    )
            }))
    }

    fn hover_tools(
        &self,
        key: &str,
        selector: &str,
        quote: String,
        thread: Option<String>,
        time: String,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let group = SharedString::from(format!("supervisor-row-{key}"));
        let reply = self.pane.is_some().then(|| {
            icon_button(
                theme,
                format!("{selector}-discuss"),
                "Reply (Discuss)",
                icons::LUCIDE_MESSAGE_SQUARE,
            )
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.emit(SupervisorEvent::Discuss(quote.clone()))
            }))
        });
        let open = thread.map(|chat_id| {
            icon_button(
                theme,
                format!("{selector}-open"),
                "Open thread",
                icons::LUCIDE_ARROW_UP_RIGHT,
            )
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.emit(SupervisorEvent::OpenChat(chat_id.clone()))
            }))
        });
        div()
            .flex_none()
            .relative()
            .w(px(38.0))
            .h(px(24.0))
            .child(
                time_cell(theme, time)
                    .absolute()
                    .right_0()
                    .top(px(4.0))
                    .group_hover(group.clone(), |s| s.opacity(0.0)),
            )
            .child(
                div()
                    .absolute()
                    .right_0()
                    .top_0()
                    .flex()
                    .gap(px(2.0))
                    .opacity(0.0)
                    .group_hover(group, |s| s.opacity(1.0))
                    .children(open)
                    .children(reply),
            )
    }

    fn render_item(
        &self,
        row: &Row,
        item: &SupervisorItem,
        q_width: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sel = format!("supervisor-needs-item-{}", item.id);
        let key = row.key.as_str();
        let group = row.group.clone().unwrap_or_default();
        let picked = self.staged(key, "");
        let mission_name = mission_title(self.state.read(cx), item.task_id.as_deref());
        let title = strip_mission(&item.title, mission_name.as_deref());
        let quote = item_quote(item, mission_name.as_deref());
        let pick = Pick::Stage {
            key: key.to_owned(),
            group,
            question: String::new(),
            multi: false,
        };
        let compact = self.compact() || !self.fits(&item.options, q_width + WIDE_ROW);
        let segments = self.render_segments(&sel, &item.options, &picked, pick, compact, theme, cx);
        let tools = self.hover_tools(key, &sel, quote, None, clock(item.at), theme, cx);
        let failed = self.failure(key);
        let detail = crate::transcript::single_line(item.detail.trim());
        let shell = self.row_shell(key, sel, item.title.clone(), theme);
        let row = if compact {
            let detail = self
                .detail_fits(&item.options)
                .then(|| faint(theme, detail));
            two_lines(shell)
                .child(mark(unread(theme)))
                .child(question_line(theme, title, None))
                .child(tools)
                .child(second_line().child(segments).children(detail))
        } else {
            shell
                .child(mark(unread(theme)))
                .child(question_cell(theme, title, q_width, None))
                .child(segments)
                .child(faint(theme, detail))
                .child(tools)
        };
        div()
            .flex()
            .flex_col()
            .child(row)
            .children(failed.map(|error| self.failure_line(error, theme)))
            .into_any_element()
    }

    fn render_input(
        &self,
        shown: &Row,
        input: &NeedsYouInput,
        q_width: f32,
        mission: Option<&str>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = shown.key.as_str();
        let sel = format!(
            "supervisor-needs-input-{}",
            input.request_id.replace(':', "-")
        );
        let who = asker_name(input, self.state.read(cx), Utc::now());
        let quote = input_quote(input, &who);
        let thread = (input.asker != InputAsker::Supervisor).then(|| input.chat_id.clone());
        let title = strip_mission(&input.title(), mission);
        let approval = input.questions.iter().any(|question| question.approval);
        if sheet_row(&shown.entry).is_none() {
            let chat_id = input.chat_id.clone();
            let supervisor = input.asker == InputAsker::Supervisor;
            let reply_quote = quote.clone();
            let link = (!supervisor || self.pane.is_some()).then(|| {
                div()
                    .id(SharedString::from(format!("{sel}-reply")))
                    .debug_selector({
                        let sel = sel.clone();
                        move || format!("{sel}-reply")
                    })
                    .role(gpui::Role::Button)
                    .flex_none()
                    .cursor_pointer()
                    .text_size(ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .hover(|s| s.text_color(theme.text))
                    .child("Reply in conversation")
                    .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                        if supervisor {
                            page.open_conversation(cx);
                            cx.emit(SupervisorEvent::Discuss(reply_quote.clone()));
                        } else {
                            cx.emit(SupervisorEvent::OpenChat(chat_id.clone()));
                        }
                    }))
            });
            return self
                .row_shell(key, sel, title.clone(), theme)
                .child(mark(unread(theme)))
                .child(question_cell(theme, title, q_width, None))
                .children(link)
                .child(faint(theme, "Typed answer".into()))
                .child(time_cell(theme, clock(input.at)))
                .into_any_element();
        }
        let group = shown.group.clone().unwrap_or_default();
        let multi = input.questions.len() > 1;
        let mut lines = div().flex().flex_col();
        for (qx, question) in input.questions.iter().enumerate() {
            let picked = self.staged(key, &question.id);
            let header = question.header.trim();
            let asked = crate::transcript::single_line(question.question.trim());
            let (name, detail) = match (multi, header.is_empty()) {
                (false, _) => (strip_mission(&asked, mission), who.clone()),
                (true, _) if qx == 0 && input.task_id.is_none() => (who.clone(), asked),
                (true, false) => (header.to_owned(), asked),
                (true, true) => (format!("Question {}", qx + 1), asked),
            };
            let pick = Pick::Stage {
                key: key.to_owned(),
                group: group.clone(),
                question: question.id.clone(),
                multi: question.multi_select,
            };
            let compact = self.compact() || !self.fits(&question.options, q_width + WIDE_ROW);
            let segments = self.render_segments(
                &format!("{sel}-q{qx}"),
                &question.options,
                &picked,
                pick,
                compact,
                theme,
                cx,
            );
            let tools = if qx == 0 {
                self.hover_tools(
                    key,
                    &sel,
                    quote.clone(),
                    thread.clone(),
                    clock(input.at),
                    theme,
                    cx,
                )
            } else {
                div().flex_none().w(px(38.0))
            };
            lines = lines.child(
                div()
                    .relative()
                    .when(qx > 0, |el| el.child(hairline(theme)))
                    .child(
                        self.row_shell(
                            &if qx == 0 {
                                key.to_owned()
                            } else {
                                format!("{key}-q{qx}")
                            },
                            if qx == 0 {
                                sel.clone()
                            } else {
                                format!("{sel}-q{qx}-row")
                            },
                            name.clone(),
                            theme,
                        )
                        .map(|shell| {
                            let glyph = mark(if qx == 0 {
                                unread(theme).into_any_element()
                            } else {
                                div().into_any_element()
                            });
                            let word = (qx == 0 && approval).then_some("Approval");
                            if compact {
                                let detail = self
                                    .detail_fits(&question.options)
                                    .then(|| faint(theme, detail));
                                two_lines(shell)
                                    .child(glyph)
                                    .child(question_line(theme, name, word))
                                    .child(tools)
                                    .child(second_line().child(segments).children(detail))
                            } else {
                                shell
                                    .child(glyph)
                                    .child(question_cell(theme, name, q_width, word))
                                    .child(segments)
                                    .child(faint(theme, detail))
                                    .child(tools)
                            }
                        }),
                    ),
            );
        }
        lines
            .children(
                self.failure(key)
                    .map(|error| self.failure_line(error, theme)),
            )
            .into_any_element()
    }

    fn failure(&self, key: &str) -> Option<String> {
        match self.replies.get(key) {
            Some(Reply {
                stage: Stage::Failed(error),
                ..
            }) => Some(error.clone()),
            _ => None,
        }
    }

    fn failure_line(&self, error: String, theme: &Theme) -> gpui::Div {
        div()
            .pl(px(HAIRLINE_LEFT))
            .pr(px(10.0))
            .pb(px(8.0))
            .text_size(ui_rems(11.5))
            .text_color(theme.danger)
            .child(format!("Could not send: {error}"))
    }

    #[allow(clippy::too_many_arguments)]
    fn render_answer(
        &self,
        row: &Row,
        needs: &NeedsYouRow,
        reply: &Reply,
        mission: Option<&str>,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = row.key.clone();
        let sel = format!("supervisor-answered-{}", sel_of(&key));
        let title = strip_mission(&needs.title(), mission);
        let to = needs.dest().word();
        let fade = {
            let total = CONFIRM.total().mul_f32(motion::speed_scale()).as_secs_f32();
            let raw = now.saturating_duration_since(reply.flipped).as_secs_f32() / total.max(0.001);
            if motion::reduced_motion(cx) {
                1.0
            } else {
                CONFIRM.progress(raw)
            }
        };
        let end = match &reply.stage {
            Stage::Held => format!("Sending to {to}"),
            Stage::Sending => format!("Sending to {to}"),
            _ => format!("Sent to {to}"),
        };
        let undo = (reply.stage == Stage::Held).then(|| {
            let key = key.clone();
            let id = format!("{sel}-undo");
            let selector = id.clone();
            div()
                .id(SharedString::from(id))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label("Undo")
                .cursor_pointer()
                .text_color(theme.text_muted)
                .hover(|s| s.text_color(theme.text))
                .child("Undo")
                .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| page.undo(&key, cx)))
        });
        div()
            .id(SharedString::from(format!("supervisor-card-{key}")))
            .debug_selector(move || sel.clone())
            .role(gpui::Role::Status)
            .aria_label(format!("{title} → {}. {end}", reply.choice))
            .opacity(0.8 * fade)
            .flex()
            .items_center()
            .gap(px(10.0))
            .h(px(30.0))
            .pl(px(14.0))
            .pr(px(10.0))
            .overflow_hidden()
            .text_size(ui_rems(12.5))
            .text_color(theme.text_faint)
            .child(mark(
                icon(icons::LUCIDE_CHECK)
                    .size(px(13.0))
                    .text_color(ok_tone(theme)),
            ))
            .child(
                div()
                    .flex_none()
                    .max_w(px(260.0))
                    .truncate()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(title),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(format!("→ {}", reply.choice)),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .gap(px(6.0))
                    .text_size(ui_rems(11.5))
                    .child(end)
                    .when_some(undo, |el, undo| el.child("·").child(undo)),
            )
            .into_any_element()
    }

    fn render_footer(
        &self,
        group: &str,
        sheet: &[&NeedsYouRow],
        title: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let total: usize = sheet.iter().map(|row| sheet_lines(row)).sum();
        let picked: usize = sheet
            .iter()
            .map(|row| {
                let key = row.key();
                match row {
                    NeedsYouRow::Item(_) => usize::from(!self.staged(&key, "").is_empty()),
                    NeedsYouRow::Input(input) => input
                        .questions
                        .iter()
                        .filter(|question| !self.staged(&key, &question.id).is_empty())
                        .count(),
                }
            })
            .sum();
        let ready: Vec<&&NeedsYouRow> = sheet
            .iter()
            .filter(|row| self.ready(row).is_some())
            .collect();
        let dests: Vec<SendDest> = if ready.is_empty() {
            sheet.iter().map(|row| row.dest()).collect()
        } else {
            ready.iter().map(|row| row.dest()).collect()
        };
        let to = dests
            .first()
            .filter(|first| dests.iter().all(|other| other == *first))
            .map(|dest| dest.word());
        let label = match (ready.len(), to) {
            (0, Some(to)) => format!("Send to {to}"),
            (0, None) => "Send".to_owned(),
            (n, Some(to)) => format!("Send {n} to {to}"),
            (n, None) => format!("Send {n}"),
        };
        let quote = {
            let state = self.state.read(cx);
            let mut lines = vec![format!("Open decisions on {title}:")];
            for row in sheet {
                lines.push(match row {
                    NeedsYouRow::Item(item) => item_quote(item, Some(title)),
                    NeedsYouRow::Input(input) => {
                        input_quote(input, &asker_name(input, state, Utc::now()))
                    }
                });
            }
            lines.join("\n\n")
        };
        let selector = format!(
            "supervisor-send-{}",
            if group.is_empty() { "shared" } else { group }
        );
        let reply_sel = format!(
            "supervisor-sheet-reply-{}",
            if group.is_empty() { "shared" } else { group }
        );
        let enabled = !ready.is_empty();
        let send_group = group.to_owned();
        let compact = self.compact() || self.inbox_width() < WIDE_FOOTER;
        div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .when(compact, |el| {
                el.flex_wrap().gap_y(px(6.0)).min_h(px(40.0)).py(px(7.0))
            })
            .when(!compact, |el| el.h(px(40.0)))
            .pl(px(HAIRLINE_LEFT))
            .pr(px(10.0))
            .border_t_1()
            .border_color(widgets::row_divider(theme))
            .text_size(ui_rems(12.0))
            .text_color(theme.text_faint)
            .child(
                div()
                    .debug_selector({
                        let group = group.to_owned();
                        move || format!("supervisor-picked-{group}")
                    })
                    .child(format!("{picked} of {total} picked")),
            )
            .when(self.pane.is_some(), |el| {
                el.child("·").child(
                    div()
                        .id(SharedString::from(reply_sel.clone()))
                        .debug_selector(move || reply_sel.clone())
                        .role(gpui::Role::Button)
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme.text))
                        .child("Reply with a note")
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(SupervisorEvent::Discuss(quote.clone()))
                        })),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .role(gpui::Role::Button)
                    .aria_label(label.clone())
                    .aria_disabled(!enabled)
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .h(px(26.0))
                    .px(px(11.0))
                    .rounded(px(7.0))
                    .bg(theme.solid)
                    .text_color(theme.on_solid)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .when(!enabled, |el| el.opacity(0.35))
                    .when(enabled, |el| el.cursor_pointer().hover(|s| s.opacity(0.9)))
                    .child(label)
                    .when(
                        !compact && self.last_sheet.as_deref() == Some(group),
                        |el| {
                            el.child(
                                div()
                                    .px(px(4.0))
                                    .rounded(px(4.0))
                                    .border_1()
                                    .border_color(theme.on_solid.opacity(0.5))
                                    .font_family(theme.font_mono.clone())
                                    .text_size(ui_rems(10.5))
                                    .line_height(ui_rems(15.0))
                                    .opacity(0.7)
                                    .child("⌘↵"),
                            )
                        },
                    )
                    .when(enabled, |el| {
                        el.on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                            page.send_sheet(&send_group, cx)
                        }))
                    }),
            )
            .into_any_element()
    }

    fn other_shell(
        &self,
        key: &str,
        selector: String,
        label: String,
        glyph: &'static str,
        tone: Option<Hsla>,
        done: bool,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        let selector = if done {
            format!("earlier-{selector}")
        } else {
            selector
        };
        self.row_shell(&done_key(key, done), selector, label, theme)
            .gap(px(10.0))
            .min_h(px(38.0))
            .child(mark(
                icon(glyph)
                    .size(px(13.0))
                    .text_color(tone.unwrap_or(theme.text_muted)),
            ))
    }

    fn render_question(
        &self,
        key: &str,
        question: &LeadQuestion,
        done: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let asked = {
            let state = self.state.read(cx);
            let step = step_name(state, &question.task_id, &question.step_id, Utc::now())
                .and_then(|name| name.split(" · ").next().map(str::to_owned))
                .unwrap_or_else(|| "A step".into());
            format!("{step} asked the Lead")
        };
        let (label, detail) = question_state(&question.state);
        let text = question_text(&question.questions);
        let line = match (done, detail) {
            (true, Some(detail)) => format!("{label} {detail} · {text}"),
            _ => text.clone(),
        };
        let selector = format!(
            "supervisor-question-{}",
            question.request_id.replace(':', "-")
        );
        let chat_id = question.chat_id.clone();
        let open = chat_id.clone();
        let group = SharedString::from(format!("supervisor-row-{}", done_key(key, done)));
        self.other_shell(
            key,
            selector.clone(),
            format!("{label}: {text}"),
            icons::LUCIDE_CIRCLE_HELP,
            kind_tone(theme, Kind::Question),
            done,
            theme,
        )
        .role(gpui::Role::Button)
        .cursor_pointer()
        .child(dim(theme, asked))
        .child(faint(theme, clip(&line)))
        .child(
            div()
                .flex_none()
                .opacity(0.0)
                .group_hover(group, |s| s.opacity(1.0))
                .child(
                    icon(icons::LUCIDE_ARROW_UP_RIGHT)
                        .size(px(14.0))
                        .text_color(theme.text),
                ),
        )
        .child(time_cell(theme, clock(question.at)))
        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
            cx.emit(SupervisorEvent::OpenChat(open.clone()))
        }))
        .into_any_element()
    }

    fn render_intervention(
        &self,
        key: &str,
        row: &SupervisorIntervention,
        done: bool,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let utc = Utc::now();
        let action = {
            let state = self.state.read(cx);
            let text = action_text(row, state, utc);
            let mut chars = text.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => text,
            }
        };
        let text = if row.text.trim().is_empty() {
            intervention_kind_label(row.kind).to_owned()
        } else {
            crate::transcript::single_line(row.text.trim())
        };
        let detail = if done {
            let chips: Vec<String> = row.chips.iter().map(chip_label).collect();
            if chips.is_empty() {
                text.clone()
            } else {
                format!("{text} · {}", chips.join(" · "))
            }
        } else {
            text.clone()
        };
        let decision = row
            .item
            .as_ref()
            .filter(|_| row.kind == InterventionKind::Decide)
            .and_then(|item| Some((item, item.answer.as_ref()?)));
        let detail = match decision {
            Some((item, answer)) => {
                let chose = match &answer.supervisor {
                    Some(decided) if answer.corrected() => format!(
                        "{} → {} (you changed it from {})",
                        item.title, answer.choice, decided.choice
                    ),
                    _ => format!("{} → {}", item.title, answer.choice),
                };
                format!("{chose} · {detail}")
            }
            None => detail,
        };
        let compact = self.compact()
            || decision.is_some_and(|(item, _)| !self.fits(&item.options, WIDE_INTERVENTION));
        let correction = decision.map(|(item, answer)| {
            let sel = format!("supervisor-intervention-{}-choice", row.id);
            let pick = Pick::Correct {
                item_id: item.id.clone(),
            };
            let picked = std::slice::from_ref(&answer.choice);
            self.render_segments(&sel, &item.options, picked, pick, compact, theme, cx)
        });
        let failed = decision
            .and_then(|(item, _)| self.sends.get(&item.id).cloned().flatten())
            .map(|error| format!("Could not send the correction: {error}"));
        let face = (!done).then(|| chip_face(row, utc)).flatten();
        let label = match &face {
            Some(face) => format!("{action}: {detail} · {}", face.label),
            None => format!("{action}: {detail}"),
        };
        let shell = self.other_shell(
            key,
            format!("supervisor-intervention-{}", row.id),
            label,
            icons::LUCIDE_CORNER_DOWN_RIGHT,
            kind_tone(theme, Kind::Intervention),
            done,
            theme,
        );
        let chip = face.map(|face| self.render_chip(key, &face, now, theme));
        let shell = if compact {
            two_lines(shell)
                .child(div().flex_1().min_w_0().flex().child(dim(theme, action)))
                .child(time_cell(theme, clock(row.at)))
                .child(
                    second_line()
                        .children(correction)
                        .children(chip)
                        .child(faint(theme, clip(&detail))),
                )
        } else {
            shell
                .child(dim(theme, action))
                .child(faint(theme, clip(&detail)))
                .children(correction)
                .children(chip)
                .child(time_cell(theme, clock(row.at)))
        };
        let approval = (row.kind == InterventionKind::Approve)
            .then(|| approval_state(row, self.state.read(cx), utc))
            .flatten()
            .map(|approval| self.approval_line(row, approval, theme, cx));
        let resume = row
            .step_ids
            .first()
            .filter(|_| !done && row.chips.contains(&InterventionChip::StepPaused))
            .map(|step_id| self.resume_line(row, step_id.clone(), theme, cx));
        div()
            .flex()
            .flex_col()
            .child(shell)
            .children(approval)
            .children(resume)
            .children(failed.map(|error| self.failure_line(error, theme)))
            .into_any_element()
    }

    fn resume_line(
        &self,
        row: &SupervisorIntervention,
        step_id: String,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let key = format!("resume:{}", row.id);
        let sent = self.sends.get(&key);
        let sending = matches!(sent, Some(None));
        let error = sent.cloned().flatten();
        let selector = format!("supervisor-resume-{}", row.id);
        let resume = widgets::text_action(
            theme,
            widgets::ActionTone::Outlined,
            if sending { "Resuming…" } else { "Resume" },
        )
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .role(gpui::Role::Button)
        .aria_label("Resume the paused step")
        .flex_none()
        .min_h(px(22.0))
        .rounded_full()
        .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            let params = serde_json::json!({ "op": "resumeSupervisorStep", "stepId": step_id });
            page.send_mutation(key.clone(), params, cx)
        }));
        div()
            .pl(px(HAIRLINE_LEFT))
            .pr(px(10.0))
            .pb(px(8.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .text_size(ui_rems(11.5))
            .child(div().flex().child(resume))
            .children(error.map(|error| {
                div()
                    .text_color(theme.danger)
                    .child(format!("Could not resume the step: {error}"))
            }))
    }

    fn approval_line(
        &self,
        row: &SupervisorIntervention,
        approval: ApprovalState,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let task_id = row.task_id.clone().unwrap_or_default();
        let sent = self.sends.get(&take_back_key(&task_id));
        let sending = matches!(sent, Some(None));
        let error = sent.cloned().flatten();
        let selector = format!("supervisor-take-back-{}", row.id);
        let take_back = approval.can_take_back().then(|| {
            widgets::text_action(
                theme,
                widgets::ActionTone::Outlined,
                if sending {
                    "Taking back…"
                } else {
                    "Take back"
                },
            )
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .role(gpui::Role::Button)
            .aria_label("Take back")
            .flex_none()
            .min_h(px(22.0))
            .rounded_full()
            .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                page.take_back(task_id.clone(), cx)
            }))
        });
        div()
            .pl(px(HAIRLINE_LEFT))
            .pr(px(10.0))
            .pb(px(8.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .text_size(ui_rems(11.5))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .min_w_0()
                    .text_color(theme.text_muted)
                    .child(div().min_w_0().child(approval.line()))
                    .children(take_back),
            )
            .children(error.map(|error| {
                div()
                    .text_color(theme.danger)
                    .child(format!("Could not take it back: {error}"))
            }))
    }

    fn render_resource(
        &self,
        key: &str,
        resource: &SharedResource,
        done: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (declared, held, verified, listed) = {
            let state = self.state.read(cx);
            let listed = state.supervisor.as_ref().is_some_and(|snapshot| {
                snapshot
                    .resources
                    .iter()
                    .any(|declared| declared.key == resource.key)
            });
            let title = |id: &str| {
                state
                    .tasks
                    .iter()
                    .find(|task| task.id == id)
                    .map_or_else(|| format!("task {id}"), |task| task.title.clone())
            };
            let names = |ids: &[String]| {
                ids.iter()
                    .map(|id| title(id))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            (
                names(&resource.declared_by),
                match resource.holders.as_slice() {
                    [] => "held by none".to_string(),
                    holders => format!("held by {}", names(holders)),
                },
                verified_line(resource, &title),
                listed,
            )
        };
        let tone = match resource.state {
            _ if !listed => theme.text_muted,
            ResourceState::Contended => theme.danger,
            ResourceState::Clear => ok_tone(theme),
            _ if feed::resource_open(resource.state) => needs_tone(theme),
            _ => theme.text_muted,
        };
        let state = if listed {
            resource_label(resource.state)
        } else {
            "No longer shared by watched missions"
        };
        let name = resource.key.clone();
        let mut detail = format!("{held} · declared by {declared}");
        if let Some(line) = verified {
            detail = format!("{detail} · {line}");
        }
        self.other_shell(
            key,
            format!("supervisor-resource-{name}"),
            format!("{name} · {state}"),
            icons::LUCIDE_LAYERS,
            kind_tone(theme, Kind::Resource),
            done,
            theme,
        )
        .child(
            div()
                .flex_none()
                .font_family(theme.font_mono.clone())
                .text_size(ui_rems(12.0))
                .text_color(theme.text)
                .child(name),
        )
        .child(
            div()
                .flex_none()
                .text_size(ui_rems(11.5))
                .text_color(tone)
                .child(state),
        )
        .child(faint(theme, detail))
        .children(
            resource
                .check
                .as_ref()
                .map(|check| time_cell(theme, clock(check.at))),
        )
        .into_any_element()
    }

    fn render_answered(
        &self,
        answered: &feed::Answered,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = answered.row.key();
        let (task_id, _) = answered.row.origin();
        let state = self.state.read(cx);
        let mission = mission_title(state, task_id);
        let title = strip_mission(&answered.row.title(), mission.as_deref());
        let decided = |id: &str| {
            state
                .supervisor
                .as_ref()?
                .interventions
                .iter()
                .filter_map(|row| row.item.as_ref())
                .find(|item| item.id == id)
                .and_then(|item| Some(item.answer.as_ref()?.choice.clone()))
        };
        let choice = match (&answered.choice, &answered.row) {
            (Some(choice), _) => format!("→ {choice}"),
            (None, NeedsYouRow::Item(item)) => decided(&item.id)
                .map_or_else(|| "Answered".to_owned(), |choice| format!("→ {choice}")),
            (None, NeedsYouRow::Input(_)) => "Closed".to_owned(),
        };
        self.other_shell(
            &key,
            format!("supervisor-needs-{}", sel_of(&key)),
            format!("{title} {choice}"),
            icons::LUCIDE_CHECK,
            Some(ok_tone(theme)),
            true,
            theme,
        )
        .child(dim(theme, title))
        .child(faint(
            theme,
            match mission {
                Some(mission) => format!("{choice} · {mission}"),
                None => choice,
            },
        ))
        .child(time_cell(theme, clock(answered.at)))
        .into_any_element()
    }

    fn render_earlier(
        &self,
        rows: &[Rc<Row>],
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if rows.is_empty() {
            return None;
        }
        let header = div()
            .id("supervisor-earlier-toggle")
            .debug_selector(|| "supervisor-earlier-toggle".into())
            .role(gpui::Role::Button)
            .aria_label(format!("Earlier, {}", rows.len()))
            .aria_expanded(self.earlier_open)
            .flex()
            .items_center()
            .gap(px(6.0))
            .mx(px(2.0))
            .py(px(2.0))
            .cursor_pointer()
            .text_size(ui_rems(11.5))
            .text_color(theme.text_faint)
            .hover(|s| s.text_color(theme.text_muted))
            .child(chevron(theme, self.earlier_chevron.value_at(now) * 90.0))
            .child(format!("Earlier · {}", rows.len()))
            .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.toggle_earlier(cx)));
        let body = (!self.earlier.gone(now)).then(|| {
            let hidden = rows.len().saturating_sub(EARLIER_ROWS);
            let list = div()
                .id("supervisor-earlier")
                .debug_selector(|| "supervisor-earlier".into())
                .mt(px(6.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(theme.border)
                .overflow_hidden()
                .opacity(0.7)
                .flex()
                .flex_col()
                .children(rows.iter().take(EARLIER_ROWS).enumerate().map(|(ix, row)| {
                    div()
                        .relative()
                        .when(ix > 0, |el| el.child(hairline(theme)))
                        .child(self.render_row(row, true, Q_NARROW, None, now, theme, cx))
                }))
                .when(hidden > 0, |el| {
                    el.child(
                        div()
                            .px(px(14.0))
                            .py(px(8.0))
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_faint)
                            .child(format!(
                                "{hidden} older {} not shown",
                                plural(hidden, "row", "rows")
                            )),
                    )
                });
            folded("supervisor-earlier-fold".into(), &self.earlier, now, list)
        });
        Some(
            div()
                .flex()
                .flex_col()
                .child(header)
                .children(body)
                .into_any_element(),
        )
    }
}

fn question_cell(theme: &Theme, text: String, width: f32, word: Option<&'static str>) -> gpui::Div {
    div()
        .flex_none()
        .w(px(width))
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.0))
        .overflow_hidden()
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(ui_rems(13.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text)
                .child(text),
        )
        .children(word.map(|word| {
            div()
                .flex_none()
                .text_size(ui_rems(11.5))
                .text_color(needs_tone(theme).opacity(0.9))
                .child(word)
        }))
}

fn segments_width(options: &[String]) -> f32 {
    options
        .iter()
        .map(|option| option.chars().count() as f32 * 7.0 + 18.0)
        .sum::<f32>()
        + 4.0
}

fn question_line(theme: &Theme, text: String, word: Option<&'static str>) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(6.0))
        .child(
            div()
                .min_w_0()
                .text_size(ui_rems(13.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text)
                .child(text),
        )
        .children(word.map(|word| {
            div()
                .flex_none()
                .text_size(ui_rems(11.5))
                .text_color(needs_tone(theme).opacity(0.9))
                .child(word)
        }))
}

fn two_lines(shell: gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div> {
    shell.flex_wrap().gap_x(px(12.0)).gap_y(px(6.0)).py(px(8.0))
}

fn second_line() -> gpui::Div {
    div()
        .w_full()
        .pl(px(HAIRLINE_LEFT - 14.0))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(10.0))
}

fn dim(theme: &Theme, text: String) -> gpui::Div {
    div()
        .flex_none()
        .max_w(relative(0.45))
        .truncate()
        .text_size(ui_rems(12.5))
        .text_color(theme.text_muted)
        .child(text)
}

pub(super) fn render_calm(theme: &Theme, head: String, line: String) -> AnyElement {
    div()
        .id("supervisor-calm")
        .debug_selector(|| "supervisor-calm".into())
        .role(gpui::Role::Group)
        .aria_label(format!("{head}. {line}"))
        .flex()
        .items_center()
        .gap(px(9.0))
        .h(px(44.0))
        .px(px(14.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(theme.border)
        .bg(widgets::block_fill(theme))
        .text_size(ui_rems(12.5))
        .child(
            icon(icons::LUCIDE_CIRCLE_CHECK)
                .size(px(15.0))
                .text_color(theme.text_muted),
        )
        .child(div().flex_none().text_color(theme.text).child(head))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(theme.text_faint)
                .child(format!("· {line}")),
        )
        .into_any_element()
}
