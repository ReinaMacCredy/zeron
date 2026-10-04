use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use gpui::{
    AnyElement, Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px,
};
use zeron_markdown::parser::BlockTree;
use zeron_proto::{
    ChangeRequestActivity, ChangeRequestActivityComment, ChangeRequestActivityCommit,
    ChangeRequestCommentKind, ChangeRequestDetail, ChangeRequestFailure, ChangeRequestReaction,
    ChangeRequestReactionContent, GetChangeRequestRequest, PostChangeRequestCommentRequest,
    SetChangeRequestReactionRequest,
};
use zeron_rpc::methods;

use super::{
    PullRequestKey, PullRequestsPage, Tones, avatar_sized, diff_stat, failure_message,
    relative_time,
};
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::icons::{self, icon};
use crate::popover::Loadable;
use crate::settings::widgets::text_tooltip;
use crate::theme::Theme;

const COMMENT_PAGE: usize = 30;

pub(super) const REACTIONS: [(ChangeRequestReactionContent, &str, &str); 8] = [
    (ChangeRequestReactionContent::ThumbsUp, "👍", "thumbs up"),
    (
        ChangeRequestReactionContent::ThumbsDown,
        "👎",
        "thumbs down",
    ),
    (ChangeRequestReactionContent::Laugh, "😄", "laugh"),
    (ChangeRequestReactionContent::Hooray, "🎉", "hooray"),
    (ChangeRequestReactionContent::Confused, "😕", "confused"),
    (ChangeRequestReactionContent::Heart, "❤️", "heart"),
    (ChangeRequestReactionContent::Rocket, "🚀", "rocket"),
    (ChangeRequestReactionContent::Eyes, "👀", "eyes"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Verdict {
    Approved,
    ChangesRequested,
    Dismissed,
}

pub(super) fn verdict_of(state: Option<&str>) -> Option<Verdict> {
    match state?
        .to_ascii_uppercase()
        .replace(['-', ' '], "_")
        .as_str()
    {
        "APPROVED" => Some(Verdict::Approved),
        "CHANGES_REQUESTED" => Some(Verdict::ChangesRequested),
        "DISMISSED" => Some(Verdict::Dismissed),
        _ => None,
    }
}

/// A body of nothing but HTML comments renders as nothing, so it counts as no body.
pub(super) fn visible_body(body: &str) -> Option<&str> {
    let mut rest = body;
    let mut visible = String::new();
    while let Some(start) = rest.find("<!--") {
        visible.push_str(&rest[..start]);
        rest = match rest[start..].find("-->") {
            Some(end) => &rest[start + end + 3..],
            None => "",
        };
    }
    visible.push_str(rest);
    (!visible.trim().is_empty()).then(|| body.trim())
}

pub(super) fn newest_commit_at(commits: &[ChangeRequestActivityCommit]) -> Option<DateTime<Utc>> {
    commits
        .iter()
        .filter_map(|commit| commit.committed_at)
        .max()
}

pub(super) fn is_stale(at: DateTime<Utc>, newest_commit: Option<DateTime<Utc>>) -> bool {
    newest_commit.is_some_and(|newest| at < newest)
}

/// Approvals that still stand: each reviewer's latest verdict, approved and not older than the
/// newest commit; none at all when the conversation was cut short.
pub(super) fn approval_count(activity: &ChangeRequestActivity) -> usize {
    if activity.truncated {
        return 0;
    }
    let newest = newest_commit_at(&activity.commits);
    let mut latest: HashMap<&str, (DateTime<Utc>, Verdict)> = HashMap::new();
    for comment in &activity.comments {
        let (Some(author), Some(verdict)) = (
            comment.author.as_ref(),
            verdict_of(comment.review_state.as_deref()),
        ) else {
            continue;
        };
        let entry = latest
            .entry(author.login.as_str())
            .or_insert((comment.created_at, verdict));
        if comment.created_at >= entry.0 {
            *entry = (comment.created_at, verdict);
        }
    }
    latest
        .values()
        .filter(|(at, verdict)| *verdict == Verdict::Approved && !is_stale(*at, newest))
        .count()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Event {
    Opened(DateTime<Utc>),
    Commit(usize),
    Comment(usize),
    Merged(DateTime<Utc>),
    Closed(DateTime<Utc>),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum TimelineRow {
    Event(Event),
    Comments(Vec<usize>),
}

/// t3code's six event kinds, newest first. Merged wins over closed: GitHub sets both on a merge.
pub(super) fn timeline_events(
    created_at: DateTime<Utc>,
    merged_at: Option<DateTime<Utc>>,
    closed_at: Option<DateTime<Utc>>,
    activity: &ChangeRequestActivity,
) -> Vec<Event> {
    let mut events: Vec<(DateTime<Utc>, Event)> = vec![(created_at, Event::Opened(created_at))];
    for (index, commit) in activity.commits.iter().enumerate() {
        events.push((
            commit.committed_at.unwrap_or(created_at),
            Event::Commit(index),
        ));
    }
    for (index, comment) in activity.comments.iter().enumerate() {
        events.push((comment.created_at, Event::Comment(index)));
    }
    match (merged_at, closed_at) {
        (Some(at), _) => events.push((at, Event::Merged(at))),
        (None, Some(at)) => events.push((at, Event::Closed(at))),
        (None, None) => {}
    }
    events.sort_by(|left, right| right.0.cmp(&left.0));
    events.into_iter().map(|(_, event)| event).collect()
}

/// Back-to-back comments without a verdict fold into one "N comments" row; commits, lifecycle
/// events and verdicts each stand on their own and split those runs.
pub(super) fn group_timeline(
    events: &[Event],
    comments: &[ChangeRequestActivityComment],
) -> Vec<TimelineRow> {
    let mut rows: Vec<TimelineRow> = Vec::new();
    for event in events {
        let foldable = match event {
            Event::Comment(index) => verdict_of(comments[*index].review_state.as_deref()).is_none(),
            _ => false,
        };
        match (foldable, rows.last_mut(), event) {
            (true, Some(TimelineRow::Comments(run)), Event::Comment(index)) => run.push(*index),
            (true, _, Event::Comment(index)) => rows.push(TimelineRow::Comments(vec![*index])),
            _ => rows.push(TimelineRow::Event(*event)),
        }
    }
    rows
}

/// `You, bob, and 2 others reacted with heart emoji` (t3code `RLogic:73-81`).
pub(super) fn reaction_sentence(
    reaction: &ChangeRequestReaction,
    viewer: &str,
    name: &str,
) -> String {
    let mut names: Vec<String> = Vec::new();
    if reaction.viewer_has_reacted {
        names.push("You".into());
    }
    for login in &reaction.reactors {
        if names.len() == 3 {
            break;
        }
        if !login.eq_ignore_ascii_case(viewer) {
            names.push(login.clone());
        }
    }
    let others = (reaction.count as usize).saturating_sub(names.len());
    let who = if names.is_empty() {
        format!("{others} {}", if others == 1 { "person" } else { "people" })
    } else {
        if others > 0 {
            names.push(format!(
                "{others} {}",
                if others == 1 { "other" } else { "others" }
            ));
        }
        match names.len() {
            1 => names[0].clone(),
            2 => format!("{} and {}", names[0], names[1]),
            n => format!("{}, and {}", names[..n - 1].join(", "), names[n - 1]),
        }
    };
    format!("{who} reacted with {name} emoji")
}

/// Pills in picker order with the reader's unconfirmed clicks applied on top of the host's counts.
pub(super) fn shown_reactions(
    reactions: &[ChangeRequestReaction],
    guesses: &HashMap<ChangeRequestReactionContent, bool>,
) -> Vec<ChangeRequestReaction> {
    REACTIONS
        .iter()
        .filter_map(|(content, _, _)| {
            let base = reactions
                .iter()
                .find(|reaction| reaction.content == *content);
            let (count, viewer) = base
                .map(|reaction| (reaction.count, reaction.viewer_has_reacted))
                .unwrap_or((0, false));
            let wanted = guesses.get(content).copied().unwrap_or(viewer);
            let count = match (viewer, wanted) {
                (false, true) => count + 1,
                (true, false) => count.saturating_sub(1),
                _ => count,
            };
            (count > 0).then(|| ChangeRequestReaction {
                content: *content,
                count,
                viewer_has_reacted: wanted,
                reactors: base
                    .map(|reaction| reaction.reactors.clone())
                    .unwrap_or_default(),
            })
        })
        .collect()
}

pub(super) struct ActivityState {
    pub(super) activity: Loadable<ChangeRequestActivity>,
    bodies: HashMap<String, BlockTree>,
    comments_newest: bool,
    shown: usize,
    pub(super) timeline_newest: bool,
    open_groups: HashSet<usize>,
    open_cards: HashSet<String>,
    picker: Option<String>,
    guesses: HashMap<String, HashMap<ChangeRequestReactionContent, bool>>,
    composer: Entity<ComposerInput>,
    posting: bool,
    _composer_sub: Subscription,
    _task: Option<Task<()>>,
}

impl ActivityState {
    pub(super) fn new(key: PullRequestKey, cx: &mut Context<PullRequestsPage>) -> Self {
        let composer = cx.new(|cx| ComposerInput::new("Leave a comment", cx));
        let sub = cx.subscribe(&composer, move |page, _, event, cx| match event {
            ComposerInputEvent::ModifiedSubmitted => page.post_comment(&key, cx),
            ComposerInputEvent::Edited => cx.notify(),
            _ => {}
        });
        Self {
            activity: Loadable::Idle,
            bodies: HashMap::new(),
            comments_newest: true,
            shown: COMMENT_PAGE,
            timeline_newest: true,
            open_groups: HashSet::new(),
            open_cards: HashSet::new(),
            picker: None,
            guesses: HashMap::new(),
            composer,
            posting: false,
            _composer_sub: sub,
            _task: None,
        }
    }

    pub(super) fn composer(&self) -> Entity<ComposerInput> {
        self.composer.clone()
    }

    pub(super) fn dismiss_picker(&mut self) -> bool {
        self.picker.take().is_some()
    }
}

impl PullRequestsPage {
    pub(super) fn ensure_activity(&mut self, key: &PullRequestKey, cx: &mut Context<Self>) {
        if self
            .details
            .get(key)
            .is_some_and(|state| matches!(state.activity.activity, Loadable::Idle))
        {
            self.load_activity(key, cx);
        }
    }

    pub(super) fn load_activity(&mut self, key: &PullRequestKey, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(state) = self.details.get_mut(key) else {
            return;
        };
        if state.activity.activity.ready().is_none() {
            state.activity.activity = Loadable::Loading;
        }
        let params = serde_json::to_value(GetChangeRequestRequest {
            host: key.host.clone(),
            repository: key.repository.clone(),
            number: key.number,
        })
        .unwrap_or(serde_json::Value::Null);
        let key = key.clone();
        state.activity._task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::GET_CHANGE_REQUEST_ACTIVITY, params)
                .await
                .map_err(|error| rpc_message(&error.to_string()))
                .and_then(|value| {
                    serde_json::from_value::<ChangeRequestActivity>(value)
                        .map_err(|error| error.to_string())
                });
            this.update(cx, |page, cx| {
                let Some(state) = page.details.get_mut(&key) else {
                    return;
                };
                let activity = &mut state.activity;
                match result {
                    Ok(loaded) => {
                        activity.bodies = loaded
                            .comments
                            .iter()
                            .filter_map(|comment| {
                                visible_body(&comment.body).map(|body| {
                                    (
                                        comment.id.clone(),
                                        crate::markdown::parse_full(&super::detail::strip_html(
                                            body,
                                        )),
                                    )
                                })
                            })
                            .collect();
                        activity.guesses.clear();
                        activity.activity = Loadable::Ready(loaded);
                        page.sync_code_review(&key, cx);
                    }
                    Err(error) if activity.activity.ready().is_none() => {
                        activity.activity = Loadable::Error(error);
                    }
                    Err(_) => {}
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub(super) fn post_comment(&mut self, key: &PullRequestKey, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(state) = self.details.get_mut(key) else {
            return;
        };
        let body = state.activity.composer.read(cx).text().to_string();
        if body.trim().is_empty() || state.activity.posting {
            return;
        }
        state.activity.posting = true;
        let params = serde_json::to_value(PostChangeRequestCommentRequest {
            host: key.host.clone(),
            repository: key.repository.clone(),
            number: key.number,
            body,
        })
        .unwrap_or(serde_json::Value::Null);
        let key = key.clone();
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::POST_CHANGE_REQUEST_COMMENT, params)
                .await;
            this.update(cx, |page, cx| {
                let Some(state) = page.details.get_mut(&key) else {
                    return;
                };
                state.activity.posting = false;
                match result {
                    Ok(_) => {
                        let composer = state.activity.composer.clone();
                        composer.update(cx, |input, cx| input.set_text("", cx));
                        page.load_activity(&key, cx);
                    }
                    Err(_) => page.toast("Could not post the comment", cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn set_reaction(
        &mut self,
        key: &PullRequestKey,
        subject: String,
        content: ChangeRequestReactionContent,
        add: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(state) = self.details.get_mut(key) else {
            return;
        };
        state.activity.picker = None;
        state
            .activity
            .guesses
            .entry(subject.clone())
            .or_default()
            .insert(content, add);
        let params = serde_json::to_value(SetChangeRequestReactionRequest {
            host: key.host.clone(),
            repository: key.repository.clone(),
            number: key.number,
            subject_id: (!subject.is_empty()).then(|| subject.clone()),
            content,
            add,
        })
        .unwrap_or(serde_json::Value::Null);
        let key = key.clone();
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::SET_CHANGE_REQUEST_REACTION, params)
                .await;
            this.update(cx, |page, cx| {
                match result {
                    Ok(_) => page.load_activity(&key, cx),
                    Err(_) => {
                        if let Some(guesses) = page
                            .details
                            .get_mut(&key)
                            .and_then(|state| state.activity.guesses.get_mut(&subject))
                        {
                            guesses.remove(&content);
                        }
                        page.toast("The reaction could not be saved", cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The pill bar under a body: one pill per reaction in picker order, then the add button.
    /// `subject` is "" for the pull request itself.
    fn render_reactions(
        &self,
        key: &PullRequestKey,
        subject: &str,
        reactions: &[ChangeRequestReaction],
        viewer: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = &self.details[key].activity;
        let empty = HashMap::new();
        let shown = shown_reactions(reactions, state.guesses.get(subject).unwrap_or(&empty));
        let picker_open = state.picker.as_deref() == Some(subject);
        let id = |part: &str| SharedString::from(format!("pr-reaction-{subject}-{part}"));
        let mut bar = div().flex().flex_wrap().items_center().gap(px(4.0));
        for reaction in &shown {
            let (_, emoji, name) = REACTIONS
                .iter()
                .find(|(content, _, _)| *content == reaction.content)
                .copied()
                .unwrap_or(REACTIONS[0]);
            let key = key.clone();
            let subject = subject.to_owned();
            let content = reaction.content;
            let add = !reaction.viewer_has_reacted;
            bar = bar.child(
                div()
                    .id(id(name))
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from(format!("{name}, {}", reaction.count)))
                    .tooltip(text_tooltip(reaction_sentence(reaction, viewer, name)))
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(8.0))
                    .rounded_full()
                    .border_1()
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .map(|el| {
                        if reaction.viewer_has_reacted {
                            el.border_color(theme.accent.opacity(0.6))
                                .bg(theme.accent.opacity(0.1))
                                .text_color(theme.text)
                        } else {
                            el.border_color(theme.hairline(0.1))
                                .bg(theme.ink(0.03))
                                .text_color(theme.text_muted)
                                .hover(|style| style.border_color(theme.accent.opacity(0.6)))
                        }
                    })
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.set_reaction(&key, subject.clone(), content, add, cx)
                    }))
                    .child(emoji)
                    .child(reaction.count.to_string()),
            );
        }
        let picker_key = key.clone();
        let picker_subject = subject.to_owned();
        let mut add = div()
            .id(id("add"))
            .role(gpui::Role::Button)
            .aria_label("Add a reaction")
            .relative()
            .h(px(24.0))
            .px(px(6.0))
            .flex()
            .items_center()
            .rounded_full()
            .border_1()
            .border_color(theme.hairline(0.1))
            .text_color(theme.text_muted)
            .cursor_pointer()
            .hover(|style| {
                style
                    .border_color(theme.accent.opacity(0.6))
                    .text_color(theme.text)
            })
            .on_click(cx.listener(move |page, _, _, cx| {
                if let Some(state) = page.details.get_mut(&picker_key) {
                    let open = state.activity.picker.as_deref() == Some(picker_subject.as_str());
                    state.activity.picker = (!open).then(|| picker_subject.clone());
                    cx.notify();
                }
            }))
            .child(
                icon(icons::LUCIDE_SMILE_PLUS)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            );
        if picker_open {
            let popup = theme.for_popup();
            let mut row = div().flex().gap(px(2.0));
            for (content, emoji, name) in REACTIONS {
                let used = shown
                    .iter()
                    .any(|reaction| reaction.content == content && reaction.viewer_has_reacted);
                let key = key.clone();
                let subject = subject.to_owned();
                row = row.child(
                    div()
                        .id(id(&format!("pick-{name}")))
                        .role(gpui::Role::Button)
                        .aria_label(name)
                        .size(px(28.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .text_size(px(16.0))
                        .cursor_pointer()
                        .when(used, |el| el.bg(theme.accent.opacity(0.1)))
                        .hover(|style| style.bg(popup.ink(0.08)))
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.set_reaction(&key, subject.clone(), content, !used, cx)
                        }))
                        .child(emoji),
                );
            }
            add = add.child(
                gpui::deferred(
                    gpui::anchored()
                        .anchor(gpui::Anchor::BottomLeft)
                        .snap_to_window_with_margin(px(8.0))
                        .child(
                            crate::popover::popover_card(&popup)
                                .mb(px(6.0))
                                .px(px(4.0))
                                .py(px(8.0))
                                .on_mouse_down_out(cx.listener(|page, _, _, cx| {
                                    page.dismiss_reaction_pickers(cx);
                                }))
                                .child(row),
                        ),
                )
                .with_priority(1),
            );
        }
        Some(bar.child(add).into_any_element())
    }

    pub(super) fn dismiss_reaction_pickers(&mut self, cx: &mut Context<Self>) -> bool {
        let mut closed = false;
        for state in self.details.values_mut() {
            closed |= state.activity.dismiss_picker();
        }
        if closed {
            cx.notify();
        }
        closed
    }

    pub(super) fn description_reactions(
        &self,
        key: &PullRequestKey,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let activity = self.details.get(key)?.activity.activity.ready()?;
        self.render_reactions(key, "", &activity.reactions, &activity.viewer, theme, cx)
    }

    pub(super) fn comments_meta(&self, key: &PullRequestKey) -> String {
        match &self.details[key].activity.activity {
            Loadable::Idle | Loadable::Loading => "Loading conversation…".into(),
            Loadable::Error(_) => "Conversation unavailable".into(),
            Loadable::Ready(activity) => match activity.comment_count {
                1 => "1 comment".into(),
                count => format!("{count} comments"),
            },
        }
    }

    pub(super) fn comment_count(&self, key: &PullRequestKey) -> Option<u64> {
        self.details[key]
            .activity
            .activity
            .ready()
            .map(|activity| activity.comment_count)
    }

    pub(super) fn render_comments_order(
        &self,
        key: &PullRequestKey,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = &self.details[key].activity;
        state
            .activity
            .ready()
            .filter(|activity| !activity.comments.is_empty())?;
        let newest = state.comments_newest;
        let key = key.clone();
        Some(
            order_toggle(
                "pr-comments-order",
                newest,
                if newest {
                    "Show oldest comments first"
                } else {
                    "Show newest comments first"
                },
                theme,
            )
            .on_click(cx.listener(move |page, _, _, cx| {
                cx.stop_propagation();
                if let Some(state) = page.details.get_mut(&key) {
                    state.activity.comments_newest = !state.activity.comments_newest;
                    cx.notify();
                }
            }))
            .into_any_element(),
        )
    }

    pub(super) fn render_comments(
        &self,
        key: &PullRequestKey,
        detail: &ChangeRequestDetail,
        theme: &Theme,
        tones: Tones,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.details[key].activity;
        let muted = theme.text_muted;
        let list: AnyElement = match &state.activity {
            Loadable::Idle | Loadable::Loading => comments_skeleton(theme),
            Loadable::Error(error) => self.render_unavailable(key, error, false, theme, cx),
            Loadable::Ready(activity) if activity.comments.is_empty() => div()
                .py(px(8.0))
                .text_size(px(14.0))
                .text_color(muted)
                .child("No comments yet.")
                .into_any_element(),
            Loadable::Ready(activity) => {
                let total = activity.comments.len();
                let shown = state.shown.min(total);
                let hidden = total - shown;
                let now = Utc::now();
                let mut indices: Vec<usize> = (total - shown..total).collect();
                if state.comments_newest {
                    indices.reverse();
                }
                let more = (hidden > 0).then(|| {
                    let key = key.clone();
                    let count = hidden.min(COMMENT_PAGE);
                    outline_button(
                        "pr-comments-more",
                        format!(
                            "Show {count} oldest {}",
                            if count == 1 { "comment" } else { "comments" }
                        )
                        .into(),
                        theme,
                    )
                    .w_full()
                    .justify_center()
                    .on_click(cx.listener(move |page, _, _, cx| {
                        if let Some(state) = page.details.get_mut(&key) {
                            state.activity.shown += COMMENT_PAGE;
                            cx.notify();
                        }
                    }))
                    .into_any_element()
                });
                let truncated = activity.truncated.then(|| {
                    div()
                        .mb(px(8.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(theme.warning.opacity(0.3))
                        .bg(theme.warning.opacity(0.05))
                        .px(px(8.0))
                        .py(px(6.0))
                        .text_size(px(12.0))
                        .text_color(theme.text)
                        .child(format!(
                            "This conversation is longer than this page reads in one go. The most recent {total} are here; open it on the host to read the rest."
                        ))
                });
                let cards: Vec<AnyElement> = indices
                    .into_iter()
                    .map(|index| {
                        self.render_comment_card(
                            key,
                            &activity.comments[index],
                            &activity.viewer,
                            now,
                            theme,
                            tones,
                            window,
                            cx,
                        )
                    })
                    .collect();
                let mut items: Vec<AnyElement> = cards;
                if let Some(more) = more {
                    if state.comments_newest {
                        items.push(more);
                    } else {
                        items.insert(0, more);
                    }
                }
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .children(truncated)
                    .children(items)
                    .into_any_element()
            }
        };
        div()
            .flex()
            .flex_col()
            .child(list)
            .child(self.render_comment_box(key, detail, theme, cx))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_comment_card(
        &self,
        key: &PullRequestKey,
        comment: &ChangeRequestActivityComment,
        viewer: &str,
        now: DateTime<Utc>,
        theme: &Theme,
        tones: Tones,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.details[key].activity;
        let muted = theme.text_muted;
        let verdict = verdict_of(comment.review_state.as_deref());
        let collapsible = (comment.kind == ChangeRequestCommentKind::ReviewComment
            && comment.thread_resolved)
            || verdict == Some(Verdict::Dismissed);
        let body = state.bodies.get(&comment.id).map(|tree| {
            let options = crate::markdown::render::RenderOptions::settled(SharedString::from(
                format!("pr-comment-{}", comment.id),
            ));
            crate::markdown::render::render_tree(tree, &options, theme, window, &|_| None)
        });
        let reactions =
            self.render_reactions(key, &comment.id, &comment.reactions, viewer, theme, cx);
        let path = comment.path.clone().map(|path| {
            div()
                .id(SharedString::from(format!(
                    "pr-comment-path-{}",
                    comment.id
                )))
                .mt(px(4.0))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(px(12.0))
                .text_color(muted)
                .tooltip(text_tooltip(path.clone()))
                .child(path)
        });
        let author = comment
            .author
            .as_ref()
            .map(|author| author.login.as_str())
            .unwrap_or("ghost");
        if collapsible {
            let open = state.open_cards.contains(&comment.id);
            let toggle_key = key.clone();
            let id = comment.id.clone();
            return div()
                .rounded(px(10.0))
                .border_1()
                .border_color(theme.hairline(0.06))
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "pr-comment-toggle-{}",
                            comment.id
                        )))
                        .role(gpui::Role::Button)
                        .aria_expanded(open)
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .p(px(12.0))
                        .text_size(px(12.0))
                        .text_color(muted)
                        .cursor_pointer()
                        .when(!open, |el| el.opacity(0.65))
                        .hover(|style| style.opacity(1.0))
                        .on_click(cx.listener(move |page, _, _, cx| {
                            if let Some(state) = page.details.get_mut(&toggle_key) {
                                if !state.activity.open_cards.remove(&id) {
                                    state.activity.open_cards.insert(id.clone());
                                }
                                cx.notify();
                            }
                        }))
                        .child(avatar_sized(author, 16.0, theme))
                        .child(relative_time(now, comment.created_at))
                        .child(if verdict == Some(Verdict::Dismissed) {
                            "Approval dismissed"
                        } else {
                            "Resolved"
                        })
                        .child(div().flex_1())
                        .child(chevron(open, theme)),
                )
                .when(open, |el| {
                    el.child(
                        div()
                            .px(px(12.0))
                            .pb(px(12.0))
                            .children(path)
                            .children(body.map(|body| div().mt(px(8.0)).child(body)))
                            .children(reactions.map(|bar| div().mt(px(8.0)).child(bar))),
                    )
                })
                .into_any_element();
        }
        let badge: Option<AnyElement> = match (verdict, comment.review_state.as_deref()) {
            (Some(Verdict::Approved), _) => Some(super::detail::verdict_badge(
                icons::LUCIDE_CIRCLE_CHECK,
                "Approved",
                tones.open,
            )),
            (Some(Verdict::ChangesRequested), _) => Some(super::detail::verdict_badge(
                icons::LUCIDE_CIRCLE_X,
                "Changes requested",
                tones.conflict,
            )),
            (_, Some(state)) => Some(
                div()
                    .child(super::detail::verdict_word(state))
                    .into_any_element(),
            ),
            _ => None,
        };
        let (header_reactions, body_reactions) = if body.is_some() {
            (None, reactions)
        } else {
            (reactions, None)
        };
        div()
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.hairline(0.06))
            .p(px(12.0))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(12.0))
                    .text_color(muted)
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "pr-comment-author-{}",
                                comment.id
                            )))
                            .tooltip(text_tooltip(author.to_owned()))
                            .child(avatar_sized(author, 16.0, theme)),
                    )
                    .child(relative_time(now, comment.created_at))
                    .children(badge)
                    .children(header_reactions),
            )
            .children(path)
            .children(body.map(|body| div().mt(px(8.0)).child(body)))
            .children(body_reactions.map(|bar| div().mt(px(8.0)).child(bar)))
            .into_any_element()
    }

    fn render_comment_box(
        &self,
        key: &PullRequestKey,
        detail: &ChangeRequestDetail,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.details[key].activity;
        let empty = state.composer.read(cx).text().trim().is_empty();
        let posting = state.posting;
        let close = self.comment_box_actions(key, detail.item.state, empty, theme, cx);
        let key = key.clone();
        div()
            .mt(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .id("pr-comment-box")
                    .role(gpui::Role::TextInput)
                    .aria_label("Comment on this pull request")
                    .min_h(px(64.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(theme.hairline(0.12))
                    .bg(theme.ink(0.02))
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_size(px(13.0))
                    .when(posting, |el| el.opacity(0.6))
                    .child(state.composer.clone()),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .children(close)
                    .child(
                        outline_button(
                            "pr-comment-send",
                            if posting {
                                "Posting...".into()
                            } else {
                                "Comment".into()
                            },
                            theme,
                        )
                        .child(
                            icon(icons::LUCIDE_SEND)
                                .size(px(12.0))
                                .text_color(theme.text_muted),
                        )
                        .when(empty || posting, |el| el.opacity(0.64).cursor_default())
                        .when(!empty && !posting, |el| {
                            el.on_click(
                                cx.listener(move |page, _, _, cx| page.post_comment(&key, cx)),
                            )
                        }),
                    ),
            )
            .into_any_element()
    }

    fn render_unavailable(
        &self,
        key: &PullRequestKey,
        error: &str,
        full: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = key.clone();
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(if full { 40.0 } else { 12.0 }))
            .when(full, |el| el.min_h(px(192.0)))
            .child(
                div()
                    .text_size(px(14.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child("Could not load pull request activity"),
            )
            .child(
                div()
                    .max_w(px(448.0))
                    .text_size(px(12.0))
                    .text_color(theme.text_muted)
                    .text_center()
                    .child(error.to_string()),
            )
            .child(
                outline_button("pr-activity-retry", "Retry".into(), theme).on_click(cx.listener(
                    move |page, _, _, cx| {
                        if let Some(state) = page.details.get_mut(&key) {
                            state.activity.activity = Loadable::Idle;
                        }
                        page.ensure_activity(&key, cx);
                    },
                )),
            )
            .into_any_element()
    }

    /// Counts and the order toggle that sit on the tab bar while the Timeline is shown.
    pub(super) fn render_timeline_controls(
        &self,
        key: &PullRequestKey,
        theme: &Theme,
        tones: Tones,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.details[key].activity;
        let activity = state.activity.ready();
        let placeholder = match &state.activity {
            Loadable::Error(_) => "—",
            _ => "…",
        };
        let count = |value: Option<usize>| {
            value
                .map(|value| value.to_string())
                .unwrap_or_else(|| placeholder.to_string())
        };
        let comments = count(activity.map(|activity| activity.comment_count as usize));
        let commits = count(activity.map(|activity| activity.commits.len()));
        let approvals = activity.map(approval_count).unwrap_or(0);
        let newest = state.timeline_newest;
        let toggle_key = key.clone();
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(11.0))
            .text_color(theme.text_muted)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .when(activity.is_none(), |el| el.opacity(0.35))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                icon(icons::LUCIDE_MESSAGE_SQUARE)
                                    .size(px(12.0))
                                    .text_color(theme.text_muted),
                            )
                            .child(comments),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                icon(icons::LUCIDE_GIT_COMMIT_HORIZONTAL)
                                    .size(px(12.0))
                                    .text_color(theme.text_muted),
                            )
                            .child(commits),
                    )
                    .when(approvals > 0, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .child(
                                    icon(icons::LUCIDE_CIRCLE_CHECK)
                                        .size(px(12.0))
                                        .text_color(tones.open),
                                )
                                .child(approvals.to_string()),
                        )
                    }),
            )
            .child(
                order_toggle(
                    "pr-timeline-order",
                    newest,
                    if newest {
                        "Show oldest activity first"
                    } else {
                        "Show newest activity first"
                    },
                    theme,
                )
                .on_click(cx.listener(move |page, _, _, cx| {
                    if let Some(state) = page.details.get_mut(&toggle_key) {
                        state.activity.timeline_newest = !state.activity.timeline_newest;
                        cx.notify();
                    }
                })),
            )
            .into_any_element()
    }

    pub(super) fn render_timeline(
        &self,
        key: &PullRequestKey,
        detail: &ChangeRequestDetail,
        theme: &Theme,
        tones: Tones,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.details[key].activity;
        let activity = match &state.activity {
            Loadable::Idle | Loadable::Loading => return timeline_skeleton(theme),
            Loadable::Error(error) => return self.render_unavailable(key, error, true, theme, cx),
            Loadable::Ready(activity) => activity,
        };
        let mut events = timeline_events(
            detail.item.created_at,
            detail.merged_at,
            detail.closed_at,
            activity,
        );
        if !state.timeline_newest {
            events.reverse();
        }
        let rows = group_timeline(&events, &activity.comments);
        let now = Utc::now();
        let newest = newest_commit_at(&activity.commits);
        let rendered: Vec<AnyElement> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| match row {
                TimelineRow::Comments(run) => self
                    .render_comment_group(key, index, run, activity, now, theme, tones, window, cx),
                TimelineRow::Event(event) => self.render_timeline_event(
                    key, *event, detail, activity, newest, now, theme, tones, window, cx,
                ),
            })
            .collect();
        div()
            .px(px(16.0))
            .py(px(20.0))
            .child(
                div()
                    .mx_auto()
                    .max_w(px(768.0))
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .top(px(4.0))
                            .bottom(px(20.0))
                            .left(px(15.0))
                            .w(px(1.0))
                            .bg(theme.hairline(0.1)),
                    )
                    .children(rendered),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_comment_group(
        &self,
        key: &PullRequestKey,
        index: usize,
        run: &[usize],
        activity: &ChangeRequestActivity,
        now: DateTime<Utc>,
        theme: &Theme,
        tones: Tones,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.details[key].activity;
        let group = run.iter().copied().min().unwrap_or(index);
        let open = state.open_groups.contains(&group);
        let first = &activity.comments[run[0]];
        let authors: HashSet<&str> = run
            .iter()
            .filter_map(|ix| {
                activity.comments[*ix]
                    .author
                    .as_ref()
                    .map(|a| a.login.as_str())
            })
            .collect();
        let toggle_key = key.clone();
        let marker = match first.author.as_ref() {
            Some(author) => avatar_marker(&author.login, !open, theme),
            None => icon_marker(icons::LUCIDE_MESSAGE_SQUARE, theme),
        };
        let cards: Vec<AnyElement> = if open {
            run.iter()
                .map(|ix| {
                    self.render_timeline_comment(
                        key,
                        &activity.comments[*ix],
                        &activity.viewer,
                        now,
                        theme,
                        tones,
                        window,
                        cx,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        timeline_row(marker, 8.0)
            .child(
                div()
                    .id(("pr-timeline-group", index))
                    .role(gpui::Role::Button)
                    .aria_expanded(open)
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .py(px(8.0))
                    .cursor_pointer()
                    .when(!open, |el| el.opacity(0.55).text_color(theme.text_muted))
                    .when(open, |el| el.text_color(theme.text))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        if let Some(state) = page.details.get_mut(&toggle_key) {
                            if !state.activity.open_groups.remove(&group) {
                                state.activity.open_groups.insert(group);
                            }
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(format!(
                                        "{} {}",
                                        run.len(),
                                        if run.len() == 1 {
                                            "comment"
                                        } else {
                                            "comments"
                                        }
                                    )),
                            )
                            .child(
                                div()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .text_size(px(10.0))
                                    .text_color(theme.text_muted)
                                    .child(format!(
                                        "{} {} · {}",
                                        authors.len(),
                                        if authors.len() == 1 {
                                            "author"
                                        } else {
                                            "authors"
                                        },
                                        relative_time(now, first.created_at)
                                    )),
                            ),
                    )
                    .child(chevron_down(open, theme)),
            )
            .when(open, |el| {
                el.child(
                    div()
                        .mt(px(4.0))
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .children(cards),
                )
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_timeline_comment(
        &self,
        key: &PullRequestKey,
        comment: &ChangeRequestActivityComment,
        viewer: &str,
        now: DateTime<Utc>,
        theme: &Theme,
        tones: Tones,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = tones;
        let state = &self.details[key].activity;
        let author = comment
            .author
            .as_ref()
            .map(|a| a.login.as_str())
            .unwrap_or("ghost");
        let action = if comment.kind == ChangeRequestCommentKind::Review {
            "reviewed"
        } else {
            "commented"
        };
        let body = state.bodies.get(&comment.id).map(|tree| {
            let options = crate::markdown::render::RenderOptions::settled(SharedString::from(
                format!("pr-timeline-comment-{}", comment.id),
            ));
            crate::markdown::render::render_tree(tree, &options, theme, window, &|_| None)
        });
        let reactions =
            self.render_reactions(key, &comment.id, &comment.reactions, viewer, theme, cx);
        div()
            .py(px(8.0))
            .child(
                div()
                    .px(px(8.0))
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(px(6.0))
                                    .text_size(px(12.0))
                                    .child(actor_name(author, theme))
                                    .child(div().text_color(theme.text_muted).child(action))
                                    .children(comment.review_state.as_deref().map(|state| {
                                        div()
                                            .text_size(px(10.0))
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(theme.text_muted)
                                            .child(super::detail::verdict_word(state))
                                    })),
                            )
                            .child(
                                div()
                                    .mt(px(4.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .min_w_0()
                                    .text_size(px(11.0))
                                    .text_color(theme.text_muted)
                                    .child(relative_time(now, comment.created_at))
                                    .children(comment.path.clone().map(|path| {
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.0))
                                            .min_w_0()
                                            .child("·")
                                            .child(
                                                icon(icons::LUCIDE_FILE_CODE_2)
                                                    .size(px(12.0))
                                                    .flex_none()
                                                    .text_color(theme.text_muted),
                                            )
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .whitespace_nowrap()
                                                    .child(path),
                                            )
                                    })),
                            ),
                    )
                    .children(
                        comment
                            .url
                            .clone()
                            .map(|url| open_on_host(&comment.id, url, theme)),
                    ),
            )
            .children(body.map(|body| div().px(px(8.0)).pb(px(8.0)).mt(px(12.0)).child(body)))
            .children(reactions.map(|bar| div().px(px(8.0)).pb(px(8.0)).child(bar)))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_timeline_event(
        &self,
        key: &PullRequestKey,
        event: Event,
        detail: &ChangeRequestDetail,
        activity: &ChangeRequestActivity,
        newest_commit: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        theme: &Theme,
        tones: Tones,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = theme.text_muted;
        match event {
            Event::Opened(at) | Event::Merged(at) | Event::Closed(at) => {
                let (glyph, label) = match event {
                    Event::Opened(_) => (icons::LUCIDE_GIT_PULL_REQUEST, "Pull request opened"),
                    Event::Merged(_) => (icons::LUCIDE_GIT_MERGE, "Pull request merged"),
                    _ => (icons::LUCIDE_GIT_PULL_REQUEST_CLOSED, "Pull request closed"),
                };
                let actor = matches!(event, Event::Opened(_))
                    .then(|| detail.item.author.as_ref())
                    .flatten();
                timeline_row(icon_marker(glyph, theme), 2.0)
                    .child(
                        div()
                            .py(px(6.0))
                            .text_size(px(12.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .children(actor.map(|actor| actor_name(&actor.login, theme)))
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(theme.text)
                                            .child(label),
                                    ),
                            )
                            .child(
                                div()
                                    .mt(px(2.0))
                                    .text_size(px(11.0))
                                    .text_color(muted)
                                    .child(relative_time(now, at)),
                            ),
                    )
                    .into_any_element()
            }
            Event::Commit(index) => {
                let commit = &activity.commits[index];
                let marker = match commit.author.as_ref() {
                    Some(author) => avatar_marker(&author.login, false, theme),
                    None => icon_marker(icons::LUCIDE_GIT_COMMIT_HORIZONTAL, theme),
                };
                let short: String = commit.oid.chars().take(7).collect();
                let headline = if commit.headline.trim().is_empty() {
                    "Untitled commit".to_string()
                } else {
                    commit.headline.clone()
                };
                let open_key = key.clone();
                let oid = commit.oid.clone();
                timeline_row(marker, 2.0)
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "pr-timeline-commit-{}",
                                commit.oid
                            )))
                            .role(gpui::Role::Button)
                            .aria_label(SharedString::from(format!("View commit {short}")))
                            .group("pr-timeline-commit")
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .py(px(6.0))
                            .cursor_pointer()
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.open_commit(&open_key, &oid, cx)
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .text_size(px(12.0))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(theme.text)
                                            .hover(|style| style.text_color(theme.accent))
                                            .child(headline),
                                    )
                                    .child(
                                        div()
                                            .mt(px(4.0))
                                            .flex()
                                            .gap(px(6.0))
                                            .text_size(px(10.0))
                                            .text_color(muted)
                                            .child(
                                                div()
                                                    .font_family(theme.font_mono.clone())
                                                    .child(short),
                                            )
                                            .children(
                                                commit
                                                    .committed_at
                                                    .map(|at| relative_time(now, at)),
                                            ),
                                    ),
                            )
                            .children(match (commit.additions, commit.deletions) {
                                (Some(additions), Some(deletions)) if additions + deletions > 0 => {
                                    Some(
                                        div()
                                            .flex_none()
                                            .font_family(theme.font_mono.clone())
                                            .text_size(px(10.0))
                                            .child(diff_stat(additions, deletions, tones, theme)),
                                    )
                                }
                                _ => None,
                            }),
                    )
                    .into_any_element()
            }
            Event::Comment(index) => {
                let comment = &activity.comments[index];
                let verdict =
                    verdict_of(comment.review_state.as_deref()).unwrap_or(Verdict::Dismissed);
                let author = comment
                    .author
                    .as_ref()
                    .map(|a| a.login.as_str())
                    .unwrap_or("ghost");
                let stale =
                    verdict != Verdict::Dismissed && is_stale(comment.created_at, newest_commit);
                let (word, label, tone) = match verdict {
                    Verdict::Approved => ("approved", "Approved", tones.open),
                    Verdict::ChangesRequested => {
                        ("changes requested", "Changes requested", theme.danger)
                    }
                    Verdict::Dismissed => {
                        ("review dismissed", "Review dismissed", muted.opacity(0.7))
                    }
                };
                let state = &self.details[key].activity;
                let body = state.bodies.get(&comment.id).map(|tree| {
                    let options = crate::markdown::render::RenderOptions::settled(
                        SharedString::from(format!("pr-timeline-verdict-{}", comment.id)),
                    );
                    crate::markdown::render::render_tree(tree, &options, theme, window, &|_| None)
                });
                let reactions = self.render_reactions(
                    key,
                    &comment.id,
                    &comment.reactions,
                    &activity.viewer,
                    theme,
                    cx,
                );
                let marker = match comment.author.as_ref() {
                    Some(author) => avatar_marker(&author.login, false, theme),
                    None => icon_marker(
                        match verdict {
                            Verdict::Approved => icons::LUCIDE_CIRCLE_CHECK,
                            Verdict::ChangesRequested => icons::LUCIDE_CIRCLE_X,
                            Verdict::Dismissed => icons::LUCIDE_CIRCLE_SLASH,
                        },
                        theme,
                    ),
                };
                let word_el = div()
                    .id(SharedString::from(format!("pr-verdict-{}", comment.id)))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .map(|el| {
                        if stale {
                            el.text_color(muted)
                                .opacity(0.7)
                                .tooltip(text_tooltip(format!("{label} earlier changes")))
                        } else {
                            el.text_color(tone)
                        }
                    })
                    .child(word);
                timeline_row(marker, 8.0)
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(8.0))
                            .py(px(8.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(6.0))
                                            .text_size(px(12.0))
                                            .child(actor_name(author, theme))
                                            .child(word_el),
                                    )
                                    .child(
                                        div()
                                            .mt(px(2.0))
                                            .flex()
                                            .flex_wrap()
                                            .items_center()
                                            .gap(px(8.0))
                                            .text_size(px(11.0))
                                            .text_color(muted)
                                            .child(relative_time(now, comment.created_at))
                                            .children(comment.path.clone())
                                            .children(reactions),
                                    ),
                            )
                            .children(
                                comment
                                    .url
                                    .clone()
                                    .map(|url| open_on_host(&comment.id, url, theme)),
                            ),
                    )
                    .children(body.map(|body| div().mt(px(12.0)).child(body)))
                    .into_any_element()
            }
        }
    }
}

fn rpc_message(message: &str) -> String {
    serde_json::from_str::<ChangeRequestFailure>(message)
        .map(|failure| failure_message(failure).to_string())
        .unwrap_or_else(|_| message.to_string())
}

fn actor_name(login: &str, theme: &Theme) -> AnyElement {
    div()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.text)
        .child(login.to_owned())
        .into_any_element()
}

fn timeline_row(marker: AnyElement, marker_top: f32) -> gpui::Div {
    div()
        .relative()
        .mb(px(20.0))
        .pl(px(48.0))
        .child(div().absolute().left_0().top(px(marker_top)).child(marker))
}

fn marker_frame(theme: &Theme) -> gpui::Div {
    div()
        .size(px(32.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.bg)
}

fn icon_marker(glyph: &'static str, theme: &Theme) -> AnyElement {
    marker_frame(theme)
        .child(icon(glyph).size(px(14.0)).text_color(theme.text_muted))
        .into_any_element()
}

fn avatar_marker(login: &str, faded: bool, theme: &Theme) -> AnyElement {
    marker_frame(theme)
        .child(
            div()
                .when(faded, |el| el.opacity(0.45))
                .child(avatar_sized(login, 28.0, theme)),
        )
        .into_any_element()
}

fn chevron(open: bool, theme: &Theme) -> AnyElement {
    icon(icons::LUCIDE_CHEVRON_DOWN)
        .size(px(14.0))
        .text_color(theme.text_muted)
        .when(open, |svg| {
            svg.with_transformation(gpui::Transformation::rotate(gpui::percentage(0.5)))
        })
        .into_any_element()
}

fn chevron_down(open: bool, theme: &Theme) -> AnyElement {
    chevron(open, theme)
}

fn open_on_host(id: &str, url: String, theme: &Theme) -> AnyElement {
    div()
        .id(SharedString::from(format!("pr-open-on-host-{id}")))
        .role(gpui::Role::Button)
        .aria_label("Open activity on host")
        .tooltip(text_tooltip("Open activity on host"))
        .size(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|style| style.bg(theme.ink(0.06)))
        .on_click(move |_, _, cx| cx.open_url(&url))
        .child(
            icon(icons::LUCIDE_EXTERNAL_LINK)
                .size(px(12.0))
                .text_color(theme.text_muted),
        )
        .into_any_element()
}

fn order_toggle(
    id: &'static str,
    newest: bool,
    label: &'static str,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label)
        .h(px(28.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded(px(6.0))
        .text_size(px(10.0))
        .font_weight(gpui::FontWeight::NORMAL)
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(|style| style.bg(theme.ink(0.06)))
        .child(
            icon(icons::LUCIDE_ARROW_DOWN_UP)
                .size(px(12.0))
                .text_color(theme.text_muted),
        )
        .child(if newest {
            "Newest first"
        } else {
            "Oldest first"
        })
}

pub(super) fn outline_button(
    id: &'static str,
    label: SharedString,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .h(px(26.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.hairline(0.12))
        .text_size(px(12.0))
        .text_color(theme.text)
        .cursor_pointer()
        .hover(|style| style.bg(theme.ink(0.05)))
        .child(label)
}

fn comments_skeleton(theme: &Theme) -> AnyElement {
    let bar = theme.text_muted.opacity(0.15);
    div()
        .id("pr-comments-loading")
        .role(gpui::Role::Status)
        .aria_label("Loading pull request conversation")
        .flex()
        .flex_col()
        .gap(px(16.0))
        .py(px(8.0))
        .children([0.6, 0.45, 0.7].map(|width| {
            div()
                .flex()
                .gap(px(8.0))
                .child(div().size(px(20.0)).rounded_full().bg(bar))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(div().h(px(12.0)).w(px(96.0)).rounded(px(4.0)).bg(bar))
                        .child(
                            div()
                                .h(px(12.0))
                                .w(gpui::relative(width))
                                .rounded(px(4.0))
                                .bg(bar),
                        )
                        .child(
                            div()
                                .h(px(12.0))
                                .w(gpui::relative(width * 0.7))
                                .rounded(px(4.0))
                                .bg(bar),
                        ),
                )
        }))
        .into_any_element()
}

fn timeline_skeleton(theme: &Theme) -> AnyElement {
    let bar = theme.text_muted.opacity(0.15);
    div()
        .id("pr-timeline-loading")
        .role(gpui::Role::Status)
        .aria_label("Loading timeline")
        .px(px(16.0))
        .py(px(20.0))
        .child(
            div()
                .ml(px(8.0))
                .border_l_1()
                .border_color(theme.hairline(0.1))
                .pl(px(20.0))
                .flex()
                .flex_col()
                .gap(px(20.0))
                .children([0.5, 0.35, 0.6, 0.4, 0.55, 0.3].map(|width| {
                    div()
                        .relative()
                        .child(
                            div()
                                .absolute()
                                .left(px(-24.8))
                                .top(px(4.0))
                                .size(px(8.0))
                                .rounded_full()
                                .bg(bar),
                        )
                        .child(
                            div()
                                .h(px(12.0))
                                .w(gpui::relative(width))
                                .rounded(px(4.0))
                                .bg(bar),
                        )
                        .child(
                            div()
                                .mt(px(6.0))
                                .h(px(10.0))
                                .w(px(80.0))
                                .rounded(px(4.0))
                                .bg(bar),
                        )
                })),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use zeron_proto::ChangeRequestActor;

    use super::*;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, hour, 0, 0).unwrap()
    }

    fn comment(id: &str, hour: u32, state: Option<&str>) -> ChangeRequestActivityComment {
        ChangeRequestActivityComment {
            id: id.into(),
            kind: if state.is_some() {
                ChangeRequestCommentKind::Review
            } else {
                ChangeRequestCommentKind::Comment
            },
            author: Some(ChangeRequestActor {
                login: format!("{id}-author"),
                avatar_url: None,
            }),
            body: "hi".into(),
            created_at: at(hour),
            url: None,
            review_state: state.map(str::to_owned),
            path: None,
            line: None,
            thread_id: None,
            thread_resolved: false,
            outdated: false,
            left_side: false,
            reactions: Vec::new(),
        }
    }

    fn commit(oid: &str, hour: u32) -> ChangeRequestActivityCommit {
        ChangeRequestActivityCommit {
            oid: oid.into(),
            headline: oid.into(),
            committed_at: Some(at(hour)),
            additions: None,
            deletions: None,
            author: None,
        }
    }

    #[test]
    fn timeline_runs_newest_first_and_folds_plain_comments_between_verdicts() {
        let activity = ChangeRequestActivity {
            comments: vec![
                comment("a", 2, None),
                comment("b", 3, Some("COMMENTED")),
                comment("c", 4, Some("APPROVED")),
                comment("d", 6, None),
                comment("e", 7, None),
            ],
            commits: vec![commit("x", 5)],
            ..Default::default()
        };
        let events = timeline_events(at(1), Some(at(8)), Some(at(8)), &activity);
        assert_eq!(
            events,
            [
                Event::Merged(at(8)),
                Event::Comment(4),
                Event::Comment(3),
                Event::Commit(0),
                Event::Comment(2),
                Event::Comment(1),
                Event::Comment(0),
                Event::Opened(at(1)),
            ]
        );
        assert_eq!(
            group_timeline(&events, &activity.comments),
            [
                TimelineRow::Event(Event::Merged(at(8))),
                TimelineRow::Comments(vec![4, 3]),
                TimelineRow::Event(Event::Commit(0)),
                TimelineRow::Event(Event::Comment(2)),
                TimelineRow::Comments(vec![1, 0]),
                TimelineRow::Event(Event::Opened(at(1))),
            ]
        );
        let closed = timeline_events(at(1), None, Some(at(9)), &ChangeRequestActivity::default());
        assert_eq!(closed[0], Event::Closed(at(9)));
    }

    #[test]
    fn approvals_count_only_standing_fresh_verdicts() {
        let mut later = comment("c", 9, Some("CHANGES_REQUESTED"));
        later.author = comment("a", 0, None).author.map(|mut actor| {
            actor.login = "c-author".into();
            actor
        });
        let activity = ChangeRequestActivity {
            comments: vec![
                comment("a", 6, Some("APPROVED")),
                comment("b", 2, Some("APPROVED")),
                comment("c", 7, Some("APPROVED")),
                later,
            ],
            commits: vec![commit("x", 5)],
            ..Default::default()
        };
        assert_eq!(approval_count(&activity), 1);
        let truncated = ChangeRequestActivity {
            truncated: true,
            ..activity
        };
        assert_eq!(approval_count(&truncated), 0);
    }

    #[test]
    fn reaction_sentences_and_optimistic_counts_follow_t3code() {
        let reaction = ChangeRequestReaction {
            content: ChangeRequestReactionContent::Heart,
            count: 5,
            viewer_has_reacted: true,
            reactors: vec!["alice".into(), "bob".into(), "carol".into(), "dave".into()],
        };
        assert_eq!(
            reaction_sentence(&reaction, "alice", "heart"),
            "You, bob, carol, and 2 others reacted with heart emoji"
        );
        let anonymous = ChangeRequestReaction {
            reactors: Vec::new(),
            viewer_has_reacted: false,
            count: 1,
            ..reaction.clone()
        };
        assert_eq!(
            reaction_sentence(&anonymous, "alice", "heart"),
            "1 person reacted with heart emoji"
        );
        let pair = ChangeRequestReaction {
            reactors: vec!["bob".into()],
            count: 2,
            ..reaction.clone()
        };
        assert_eq!(
            reaction_sentence(&pair, "alice", "heart"),
            "You and bob reacted with heart emoji"
        );
        let guesses = HashMap::from([
            (ChangeRequestReactionContent::Heart, false),
            (ChangeRequestReactionContent::Rocket, true),
        ]);
        let shown: Vec<(ChangeRequestReactionContent, u64, bool)> =
            shown_reactions(&[reaction], &guesses)
                .into_iter()
                .map(|reaction| {
                    (
                        reaction.content,
                        reaction.count,
                        reaction.viewer_has_reacted,
                    )
                })
                .collect();
        assert_eq!(
            shown,
            [
                (ChangeRequestReactionContent::Heart, 4, false),
                (ChangeRequestReactionContent::Rocket, 1, true),
            ]
        );
    }

    #[test]
    fn bodies_of_only_html_comments_are_empty_and_verdicts_parse() {
        assert_eq!(visible_body("<!-- bot -->\n  "), None);
        assert_eq!(visible_body(" hi <!-- x --> "), Some("hi <!-- x -->"));
        assert_eq!(
            verdict_of(Some("changes-requested")),
            Some(Verdict::ChangesRequested)
        );
        assert_eq!(verdict_of(Some("COMMENTED")), None);
    }
}
