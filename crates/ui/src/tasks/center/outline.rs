use super::*;
use crate::tasks::thread_tree::{NodeKind, TreeNode, tree_rows};
use crate::tasks::tree::{self, Status};
use crate::tasks::{StripTree, StripTreeEvent, TreeHost};

const OUTLINE_WIDTH: f32 = 480.0;

fn clock(at: DateTime<Utc>) -> String {
    let at = at.with_timezone(&chrono::Local);
    if at.date_naive() == chrono::Local::now().date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%b %-d %H:%M").to_string()
    }
}

pub(super) fn role(node: &TreeNode) -> String {
    match node.kind {
        NodeKind::Root => "Lead".into(),
        NodeKind::Step(n) => format!("Step {n}"),
        NodeKind::Agent => "Agent".into(),
        NodeKind::Native => "Sub-agent".into(),
    }
}

impl TaskCenter {
    pub(super) fn outline_for(&mut self, root: &str, cx: &mut Context<Self>) -> Entity<StripTree> {
        if let Some((id, tree, _)) = &self.outline
            && id == root
        {
            return tree.clone();
        }
        let state = self.state.clone();
        let id = root.to_string();
        let tree = cx.new(|cx| StripTree::new(state, id, true, true, TreeHost::Center, cx));
        let events = cx.subscribe(&tree, |center, _, event: &StripTreeEvent, cx| match event {
            StripTreeEvent::Open { selection, .. } => center.select(selection.clone(), cx),
            StripTreeEvent::OpenNative {
                parent_chat_id,
                doc_id,
                title,
                frozen,
            } => cx.emit(TaskCenterEvent::OpenNative {
                parent_chat_id: parent_chat_id.clone(),
                doc_id: doc_id.clone(),
                title: title.clone(),
                frozen: *frozen,
            }),
            StripTreeEvent::Brief(_) => {}
        });
        self.outline = Some((root.to_string(), tree.clone(), events));
        tree
    }

    pub(super) fn picked(&self, nodes: &[TreeNode]) -> usize {
        let id = match &self.selected {
            Selection::Lead => return 0,
            Selection::Step(id) | Selection::Node(id) => id,
        };
        nodes.iter().position(|node| &node.id == id).unwrap_or(0)
    }

    fn tick(&mut self, live: bool, cx: &mut Context<Self>) {
        let now = Instant::now();
        if live && self.clock.as_ref().is_none_or(|(due, _)| now >= *due) {
            let tick = std::time::Duration::from_secs(1);
            self.clock = Some((
                now + tick,
                cx.spawn(async move |center, cx| {
                    cx.background_executor().timer(tick).await;
                    center.update(cx, |_, cx| cx.notify()).ok();
                }),
            ));
        }
    }

    fn root_settled(chat: &zeron_proto::Chat) -> bool {
        chat.settle.is_some_and(|settle| {
            settle.settled && chat.last_message_at.is_none_or(|at| at <= settle.at)
        })
    }

    pub(super) fn kind_badge(mission: bool, theme: &Theme) -> AnyElement {
        let (mark, label, fg, bg) = if mission {
            (
                icons::MISSION_CONTROL,
                "Mission",
                motion::mix(theme.accent, theme.text, 0.2),
                theme.accent.opacity(0.14),
            )
        } else {
            (
                icons::LUCIDE_LIST_TREE,
                "Tree",
                theme.text_muted,
                theme.wash(0.07),
            )
        };
        div()
            .debug_selector(move || format!("task-center-kind-{label}"))
            .flex_none()
            .h(px(18.0))
            .pl(px(5.0))
            .pr(px(6.0))
            .rounded(px(5.0))
            .bg(bg)
            .flex()
            .items_center()
            .gap(px(4.0))
            .text_size(ui_rems(11.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(fg)
            .child(icon(mark).size(px(11.0)).text_color(fg))
            .child(label)
            .into_any_element()
    }

    fn pill(status: Status, label: String, reduced: bool, theme: &Theme) -> AnyElement {
        let tone = status.tone(theme);
        let (bg, fg) = if status.muted() {
            (theme.wash(0.08), theme.text_muted)
        } else {
            (tone.opacity(0.15), tone)
        };
        div()
            .debug_selector(|| "task-center-status".into())
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(28.0))
            .pl(px(10.0))
            .pr(px(12.0))
            .rounded_full()
            .bg(bg)
            .text_size(ui_rems(12.5))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(fg)
            .child(tree::glyph(status, 13.0, fg, reduced))
            .child(label)
            .into_any_element()
    }

    fn branch_tokens(&self, nodes: &[TreeNode], cx: &App) -> Option<u64> {
        let state = self.state.read(cx);
        tree::amount::branch(
            None,
            nodes
                .iter()
                .filter_map(|node| node.chat_id.as_deref())
                .map(|id| tree::amount::of(self.stats.get(state, id).as_ref())),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_tree_header(
        &mut self,
        root: &zeron_proto::Chat,
        nodes: &[TreeNode],
        reduced: bool,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settled = Self::root_settled(root);
        let kids = &nodes[1..];
        let count = |status: Status| kids.iter().filter(|node| node.status == status).count();
        let (done, working) = (count(Status::Done), count(Status::Running));
        let asking = nodes.iter().find(|node| node.status == Status::Waiting);
        let (status, word) = match asking {
            Some(node) if node.kind == NodeKind::Root => (Status::Waiting, "Needs you".into()),
            Some(node) => (Status::Waiting, format!("Needs you \u{b7} {}", node.title)),
            None if settled => (Status::Done, "Done".into()),
            None => (nodes[0].status, nodes[0].word.to_string()),
        };
        let button = |label: &'static str, selector: &'static str, tone: widgets::ActionTone| {
            widgets::text_action(theme, tone, label)
                .id(selector)
                .debug_selector(move || selector.into())
                .role(gpui::Role::Button)
                .aria_label(label)
                .flex_none()
                .min_h(px(28.0))
                .rounded_full()
        };
        let can_plan = crate::tasks::can_add_plan(self.state.read(cx), root);
        let root_id = root.id.clone();
        let add_plan = can_plan.then(|| {
            let id = root_id.clone();
            widgets::action_button(theme, widgets::ActionTone::Filled)
                .id("task-center-add-plan")
                .debug_selector(|| "task-center-add-plan".into())
                .role(gpui::Role::Button)
                .aria_label("Add plan")
                .flex_none()
                .min_h(px(28.0))
                .rounded_full()
                .gap(px(6.0))
                .child(icon(icons::PLUS).size(px(13.0)).text_color(theme.text))
                .child("Add plan")
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.emit(TaskCenterEvent::AddPlan(id.clone()))
                }))
        });
        let id = root_id.clone();
        let toggle = button(
            if settled { "Reopen" } else { "Mark as done" },
            "task-center-toggle-done",
            widgets::ActionTone::Quiet,
        )
        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
            cx.emit(TaskCenterEvent::Settle {
                chat_id: id.clone(),
                settled: !settled,
            })
        }));
        let lead = Some(root_id.clone());
        let more = div()
            .id("task-center-more")
            .debug_selector(|| "task-center-more".into())
            .role(gpui::Role::Button)
            .aria_label("More")
            .flex_none()
            .size(px(28.0))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(theme.wash(0.08)))
            .child(
                icon(icons::LUCIDE_ELLIPSIS)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |center, event: &ClickEvent, window, cx| {
                cx.stop_propagation();
                center.open_menu(
                    MenuFor::Mission { lead: lead.clone() },
                    event.position(),
                    window,
                    cx,
                );
            }));
        let started = kids
            .iter()
            .filter_map(|node| node.chat_id.as_deref())
            .filter_map(|id| self.state.read(cx).chats.iter().find(|chat| chat.id == id))
            .map(|chat| chat.created_at)
            .min()
            .unwrap_or(root.created_at);
        let mut sub = vec!["Lead".to_string()];
        sub.extend(agent_of(root, cx));
        sub.push(format!("started {}", clock(started)));
        sub.push("No plan".into());
        let live = nodes.iter().any(|node| node.status.live());
        let ended = (!live)
            .then(|| {
                nodes
                    .iter()
                    .filter_map(|node| node.chat_id.as_deref())
                    .filter_map(|id| self.state.read(cx).chats.iter().find(|chat| chat.id == id))
                    .filter_map(|chat| chat.last_message_at)
                    .max()
            })
            .flatten();
        let elapsed = tree::fmt_time(
            (ended.unwrap_or_else(Utc::now) - started)
                .num_milliseconds()
                .max(0) as u64,
        );
        let segments = kids.iter().map(|node| {
            let color = match node.status {
                Status::Queued => theme.wash(0.10),
                Status::Stopped => theme.text_faint.opacity(0.35),
                Status::Running => node.status.tone(theme).opacity(0.45),
                status => status.tone(theme),
            };
            let selection = match (&node.kind, &node.step_id) {
                (NodeKind::Step(_), Some(step)) => Selection::Step(step.clone()),
                _ => Selection::Node(node.id.clone()),
            };
            let selector = format!("task-center-segment-{}", node.id);
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label(format!("{}: {}", node.title, node.word))
                .flex_1()
                .min_w_0()
                .py(px(6.0))
                .cursor_pointer()
                .tooltip(widgets::text_tooltip(format!(
                    "{} \u{b7} {}",
                    node.title, node.word
                )))
                .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                    center.select(selection.clone(), cx)
                }))
                .child(div().h(px(6.0)).rounded(px(3.0)).bg(color))
        });
        let phase = (live && !reduced).then(|| tree::amount::phase(self.shimmer_since, now));
        let agents = kids.len();
        let mut totals = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .text_size(ui_rems(12.0))
            .text_color(theme.text_muted)
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(format!("{done}/{agents}")),
            )
            .child(format!("{} done", plural(agents, "agent", "agents")));
        if working > 0 {
            totals = totals.child("\u{b7}").child(
                div()
                    .text_color(Status::Running.tone(theme))
                    .child(format!("{working} working")),
            );
        }
        if let Some(label) =
            tree::amount::label(self.branch_tokens(nodes, cx), phase, ui_rems(12.0), theme)
        {
            totals = totals.child("\u{b7}").child(label);
        }
        totals = totals.child("\u{b7}").child(elapsed);
        div()
            .id("task-center-header")
            .debug_selector(|| "task-center-header".into())
            .flex_none()
            .px(px(20.0))
            .py(px(12.0))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_none()
                            .size(px(28.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(theme.border)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                icon(icons::LUCIDE_LIST_TREE)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .min_w_0()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(ui_rems(18.0))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(theme.text)
                                            .child(crate::transcript::single_line(
                                                &crate::home::chat_title(root),
                                            )),
                                    )
                                    .child(Self::kind_badge(false, theme)),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child(sub.join(" \u{b7} ")),
                            ),
                    )
                    .child(Self::pill(status, word, reduced, theme))
                    .children(add_plan)
                    .child(toggle)
                    .child(more),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .debug_selector(|| "task-center-progress".into())
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .gap(px(3.0))
                            .children(segments),
                    )
                    .child(totals),
            )
            .into_any_element()
    }

    pub(super) fn render_outline_column(
        &mut self,
        root: &str,
        picked: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let outline = self.outline_for(root, cx);
        outline.update(cx, |tree, cx| tree.set_picked(Some(picked.to_string()), cx));
        div()
            .id("task-center-outline")
            .debug_selector(|| "task-center-outline".into())
            .flex_none()
            .w(px(OUTLINE_WIDTH))
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.border)
            .pt(px(14.0))
            .pb(px(20.0))
            .pl(px(12.0))
            .pr(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .pl(px(16.0))
                    .pr(px(10.0))
                    .pb(px(8.0))
                    .text_size(ui_rems(11.5))
                    .text_color(theme.text_faint)
                    .child(div().flex_1().child("Tree"))
                    .child("Click a row to see it here"),
            )
            .child(outline)
            .into_any_element()
    }

    fn detail_box(caps: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
        div()
            .flex_none()
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.border)
            .bg(widgets::block_fill(theme))
            .px(px(12.0))
            .py(px(10.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(ui_rems(10.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text_faint)
                    .child(caps.into()),
            )
    }

    pub(super) fn render_detail(
        &mut self,
        nodes: &[TreeNode],
        ix: usize,
        reduced: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let node = &nodes[ix];
        let (chat, step) = {
            let state = self.state.read(cx);
            (
                node.chat_id
                    .as_deref()
                    .and_then(|id| state.chats.iter().find(|chat| chat.id == id))
                    .cloned(),
                node.step_id
                    .as_deref()
                    .and_then(|id| state.task_steps.iter().find(|step| step.id == id))
                    .cloned(),
            )
        };
        let agent = chat.as_ref().and_then(|chat| agent_of(chat, cx));
        let (mark, tint) = node.brand.unwrap_or((icons::BOT, None));
        let kicker = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(ui_rems(11.5))
            .text_color(theme.text_muted)
            .child(
                icon(mark)
                    .size(px(13.0))
                    .text_color(tint.unwrap_or(theme.text_muted)),
            )
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(role(node)),
            )
            .children(agent.map(|agent| div().child(format!("\u{b7} {agent}"))));
        let title = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(ui_rems(16.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(match node.kind {
                        NodeKind::Root => chat
                            .as_ref()
                            .map(|chat| {
                                crate::transcript::single_line(&crate::home::chat_title(chat))
                            })
                            .unwrap_or_default(),
                        _ => node.title.clone(),
                    }),
            )
            .child(Self::chip(node.status, node.word, reduced, theme));
        let mut from = Vec::new();
        if let Some(parent) = node.parent {
            from.push(match parent {
                0 => "Started by the Lead".to_string(),
                parent => format!("Started by {}", nodes[parent].title),
            });
        }
        if let Some(at) = chat.as_ref().map(|chat| chat.created_at).or(node.since) {
            from.push(match node.parent {
                Some(_) => format!("at {}", clock(at)),
                None => format!("Started at {}", clock(at)),
            });
        }
        if let NodeKind::Step(n) = node.kind {
            from.push(format!("\u{b7} fills step {n}"));
        }
        let from = (!from.is_empty()).then(|| {
            div()
                .text_size(ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(from.join(" "))
        });
        let spec = step
            .filter(|step| !step.notes.trim().is_empty())
            .map(|step| {
                Self::detail_box("SPEC", theme).child(
                    div()
                        .text_size(ui_rems(12.5))
                        .text_color(theme.text)
                        .child(step.notes.clone()),
                )
            });
        let kids: Vec<&TreeNode> = nodes.iter().filter(|kid| kid.parent == Some(ix)).collect();
        let started = (!kids.is_empty()).then(|| {
            Self::detail_box(format!("STARTED {}", kids.len()), theme).children(kids.iter().map(
                |kid| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .h(px(26.0))
                        .text_size(ui_rems(13.0))
                        .child(tree::glyph(
                            kid.status,
                            13.0,
                            kid.status.tone(theme),
                            reduced,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(kid.title.clone()),
                        )
                        .child(
                            div()
                                .text_size(ui_rems(12.0))
                                .text_color(kid.status.tone(theme))
                                .child(kid.word),
                        )
                },
            ))
        });
        let live = self
            .live
            .as_ref()
            .filter(|live| node.chat_id.as_deref() == Some(live.chat_id.as_str()));
        let tokens = node
            .chat_id
            .as_deref()
            .and_then(|id| tree::amount::of(self.stats.get(self.state.read(cx), id).as_ref()));
        let time = {
            let state = self.state.read(cx);
            let stats = node
                .chat_id
                .as_deref()
                .and_then(|id| self.stats.get(state, id));
            tree::thread_time(node.status, node.since, stats.as_ref())
        };
        let open = match (&node.chat_id, node.kind) {
            (Some(id), _) => {
                let id = id.clone();
                Some(
                    widgets::text_action(theme, widgets::ActionTone::Filled, "Open thread")
                        .id("task-center-open-thread")
                        .debug_selector(|| "task-center-open-thread".into())
                        .role(gpui::Role::Button)
                        .aria_label("Open thread")
                        .flex_none()
                        .rounded_full()
                        .gap(px(6.0))
                        .child(
                            icon(icons::LUCIDE_ARROW_UP_RIGHT)
                                .size(px(12.0))
                                .text_color(theme.text),
                        )
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(TaskCenterEvent::OpenChat(id.clone()))
                        })),
                )
            }
            (None, NodeKind::Native) => {
                let event = node
                    .parent
                    .and_then(|parent| nodes[parent].chat_id.clone())
                    .map(|parent| (parent, node.id.clone(), node.title.clone()));
                let frozen = node.status != Status::Running;
                event.map(|(parent, doc, title)| {
                    widgets::text_action(theme, widgets::ActionTone::Filled, "Open")
                        .id("task-center-open-native")
                        .debug_selector(|| "task-center-open-native".into())
                        .role(gpui::Role::Button)
                        .aria_label("Open")
                        .flex_none()
                        .rounded_full()
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(TaskCenterEvent::OpenNative {
                                parent_chat_id: parent.clone(),
                                doc_id: doc.clone(),
                                title: title.clone(),
                                frozen,
                            })
                        }))
                })
            }
            _ => None,
        };
        let footer = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(self.amount(tokens, node.status.live(), time, reduced, theme))
            .children(open);
        let empty = match node.kind {
            NodeKind::Native => "A sub-agent of its thread. Open it to read it.",
            NodeKind::Step(_) if node.chat_id.is_none() => {
                "This step has not started. Messages go to the lead."
            }
            _ => "Loading the thread",
        };
        let body = match live {
            Some(live) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(theme.border)
                        .overflow_hidden()
                        .child(live.transcript.clone()),
                )
                .into_any_element(),
            None => div()
                .flex_1()
                .text_size(ui_rems(12.5))
                .text_color(theme.text_muted)
                .child(empty)
                .into_any_element(),
        };
        let composer = live.map(|live| div().flex_none().child(live.composer.clone()));
        div()
            .id("task-center-detail")
            .debug_selector(|| "task-center-detail".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .px(px(24.0))
            .pt(px(18.0))
            .pb(px(12.0))
            .child(kicker)
            .child(title)
            .children(from)
            .children(spec)
            .children(started)
            .child(body)
            .child(footer)
            .children(composer)
            .into_any_element()
    }

    pub(super) fn render_split(
        &mut self,
        root: &str,
        nodes: &[TreeNode],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reduced = motion::reduced_motion(cx);
        let ix = self.picked(nodes);
        let outline = self.render_outline_column(root, &nodes[ix].id, theme, cx);
        let detail = self.render_detail(nodes, ix, reduced, theme, cx);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(outline)
            .child(detail)
            .into_any_element()
    }

    pub(super) fn render_tree_page(
        &mut self,
        root: &str,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = Instant::now();
        let reduced = motion::reduced_motion(cx);
        let (chat, nodes) = {
            let state = self.state.read(cx);
            (
                state.chats.iter().find(|chat| chat.id == root).cloned(),
                tree_rows(state, root, true, Utc::now()),
            )
        };
        let Some(chat) = chat.filter(|_| !nodes.is_empty()) else {
            return self.render_missing(theme, cx);
        };
        let ids: Vec<String> = nodes
            .iter()
            .filter_map(|node| node.chat_id.clone())
            .collect();
        let state = self.state.clone();
        self.stats
            .refresh(&state, ids, |center: &mut Self| &mut center.stats, cx);
        let live = nodes.iter().any(|node| node.status.live());
        self.tick(live, cx);
        if live && !reduced {
            motion::pulse_lease(cx.entity_id(), cx);
        }
        let header = self.render_tree_header(&chat, &nodes, reduced, now, theme, cx);
        let body = match crate::settings::current(cx).tree_view {
            crate::settings::TreeView::Spine => {
                self.render_spine_body(None, &nodes, reduced, now, theme, window, cx)
            }
            _ => self.render_split(root, &nodes, theme, cx),
        };
        div()
            .id("task-center")
            .debug_selector(|| "task-center".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|center, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key != "escape" {
                    return;
                }
                if center.menu.is_open() {
                    cx.stop_propagation();
                    center.close_menu(cx);
                } else if center.drawer_open {
                    cx.stop_propagation();
                    center.close_drawer(cx);
                }
            }))
            .relative()
            .size_full()
            .pt(px(Theme::TITLEBAR_HEIGHT))
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .children(self.render_menu(theme, cx))
            .into_any_element()
    }
}
