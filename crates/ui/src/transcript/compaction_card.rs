use super::*;
use gpui::Div;
use zeron_doc::{ClassifierRecord, ClassifierVerdict, CompactionRecord, DecidedBy};

// Long enough for the rules to fill and fade before the label goes static.
const SETTLE: Duration = Duration::from_millis(900);
// The compaction flag drops before its entry lands; hold Compacting across the gap.
const HOLD_AFTER_WAIT: Duration = Duration::from_secs(5);
// How long a compaction takes is unknowable, so the rules sweep instead of filling.
const SWEEP: Duration = Duration::from_millis(1400);
const SWEEP_WIDTH: f32 = 0.4;
const INSET: f32 = 14.0;
const SWEEP_FADE: motion::MotionSpec = motion::MotionSpec::new(300, motion::EASE_OUT);
const FILL: motion::MotionSpec = motion::MotionSpec::new(320, motion::EASE_OUT_QUINT);
const FILL_FADE: motion::MotionSpec = motion::MotionSpec::new(400, motion::EASE).with_delay(450);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Queued,
    Checking,
    Compacting,
    Settled,
}

impl Phase {
    fn live(self) -> bool {
        self != Self::Settled
    }
}

#[derive(Default)]
pub(super) struct CardClock {
    wait: Option<(String, CompactionWait, Instant)>,
    wait_ended: Option<(String, Instant)>,
    phase: Option<(String, Phase, Instant)>,
    entered: Option<(String, Instant)>,
    epoch: u64,
    look: Option<(SharedString, Look, Option<Look>, Instant)>,
    settled: HashMap<SharedString, Instant>,
    tail: Option<SharedString>,
}

impl CardClock {
    fn wait_since(&mut self, chat_id: &str, wait: &CompactionWait, now: Instant) -> Instant {
        match &self.wait {
            Some((id, last, at)) if id == chat_id && last == wait => *at,
            _ => {
                self.wait = Some((chat_id.to_string(), wait.clone(), now));
                now
            }
        }
    }

    pub(super) fn no_wait(&mut self, chat_id: &str, now: Instant) {
        if self.wait.take().is_some() {
            self.wait_ended = Some((chat_id.to_string(), now));
        }
    }

    fn phase_since(&mut self, chat_id: &str, phase: Phase, now: Instant) -> Instant {
        match &self.phase {
            Some((id, last, at)) if id == chat_id && *last == phase => *at,
            previous => {
                let was_live = previous
                    .as_ref()
                    .is_some_and(|(id, last, _)| id == chat_id && last.live());
                if phase.live() && !was_live {
                    self.entered = Some((chat_id.to_string(), now));
                    self.epoch += 1;
                }
                self.phase = Some((chat_id.to_string(), phase, now));
                now
            }
        }
    }

    fn live_phase(&self, chat_id: &str) -> Option<(Phase, Instant)> {
        self.phase
            .as_ref()
            .filter(|(id, phase, _)| id == chat_id && phase.live())
            .map(|(_, phase, at)| (*phase, *at))
    }

    fn entered_at(&self, chat_id: &str) -> Option<Instant> {
        self.entered
            .as_ref()
            .filter(|(id, _)| id == chat_id)
            .map(|(_, at)| *at)
    }

    fn wait_ended_recently(&self, chat_id: &str, now: Instant) -> bool {
        self.wait.as_ref().is_some_and(|(id, _, _)| id == chat_id)
            || self.wait_ended.as_ref().is_some_and(|(id, at)| {
                id == chat_id && now.saturating_duration_since(*at) < HOLD_AFTER_WAIT
            })
    }

    // One key per compaction, shared by the trailer and the row that replaces
    // it, so the label's text rolls across the handoff instead of restarting.
    fn label_key(&self, chat_id: &str) -> SharedString {
        format!("compaction-{chat_id}-{}", self.epoch).into()
    }

    fn look_since(
        &mut self,
        key: &SharedString,
        look: &Look,
        now: Instant,
    ) -> (Option<Look>, Instant) {
        match &mut self.look {
            Some((last_key, last, from, at)) if last_key == key => {
                if last != look {
                    *from = Some(std::mem::replace(last, look.clone()));
                    *at = now;
                }
                (from.clone(), *at)
            }
            _ => {
                self.look = Some((key.clone(), look.clone(), None, now));
                (None, now)
            }
        }
    }
}

pub(super) fn merge_compaction_rows(rows: &mut Vec<Row>) {
    let mut ix = 0;
    while ix + 1 < rows.len() {
        // A message sent while the chat compacts lands between the decision and its result.
        let next = (ix + 1..rows.len()).find(|&at| !matches!(rows[at].kind, RowKind::User { .. }));
        if let (
            RowKind::Classifier {
                compaction: None, ..
            },
            Some(next),
        ) = (&rows[ix].kind, next)
            && let RowKind::Compaction { record, .. } = &rows[next].kind
        {
            let record = record.clone();
            let version = rows.remove(next).version;
            let row = &mut rows[ix];
            row.version = row.version.rotate_left(7) ^ version;
            if let RowKind::Classifier { compaction, .. } = &mut row.kind {
                *compaction = Some(record);
            }
        }
        ix += 1;
    }
}

/// What the composer's chip says while `chat_id` is checking or compacting:
/// the same words as the divider in the chat.
pub(crate) fn composer_label(
    state: &crate::state::AppState,
    chat_id: &str,
) -> Option<&'static str> {
    if state.chat_compacting(chat_id) {
        return Some("Compacting");
    }
    let session = state.session_for(chat_id)?;
    session.classifying.as_ref()?;
    Some(checking_title(
        session.compact_plan == Some(zeron_proto::CompactPlan::Agent),
    ))
}

fn checking_title(agent_note: bool) -> &'static str {
    if agent_note {
        "Checking the agent's note"
    } else {
        "Checking whether to compact"
    }
}

fn eased(spec: motion::MotionSpec, elapsed: Duration, reduce: bool) -> f32 {
    if reduce {
        return 1.0;
    }
    spec.progress(elapsed.as_secs_f32() / spec.total().as_secs_f32())
}

fn progress(spec: motion::MotionSpec, since: Instant, now: Instant, reduce: bool) -> f32 {
    eased(spec, now.saturating_duration_since(since), reduce)
}

fn boundary_row(rows: &[Row]) -> Option<usize> {
    rows.iter()
        .rposition(|row| !matches!(row.kind, RowKind::User { .. }))
        .filter(|ix| matches!(rows[*ix].kind, RowKind::Classifier { .. }))
}

fn usual_secs(rows: &[Row], skip: usize) -> Option<u64> {
    let mut durations: Vec<u64> = rows
        .iter()
        .enumerate()
        .filter(|(ix, _)| *ix != skip)
        .filter_map(|(_, row)| match &row.kind {
            RowKind::Classifier {
                compaction: Some(record),
                ..
            }
            | RowKind::Compaction { record, .. } => record.duration_ms,
            _ => None,
        })
        .collect();
    if durations.is_empty() {
        return None;
    }
    durations.sort_unstable();
    Some((durations[durations.len() / 2] + 500) / 1000)
}

fn tokens(count: u64) -> String {
    crate::composer::background::compact_count(count)
}

fn wait_meta(elapsed: Duration, usual: Option<u64>) -> String {
    let elapsed = format_elapsed(elapsed.as_secs() as i64);
    match usual {
        Some(usual) => format!("{elapsed}, usually ~{}", format_elapsed(usual as i64)),
        None => elapsed,
    }
}

fn carried_turns(carried: Option<&str>) -> usize {
    carried.map_or(0, |carried| {
        carried_body(carried)
            .split("\n\n")
            .filter(|block| !block.starts_with("Still running when the chat compacted:"))
            .count()
    })
}

fn carried_body(carried: &str) -> &str {
    let inner = carried
        .trim()
        .trim_start_matches("<carried_over>")
        .trim_end_matches("</carried_over>")
        .trim();
    inner.split_once("\n\n").map_or(inner, |(_, rest)| rest)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Icon {
    Spinner,
    Done,
    Kept,
    Alert,
    Decision,
    Queued,
}

struct Head {
    icon: Icon,
    title: SharedString,
    chip: Option<AnyElement>,
    tag: Option<(SharedString, gpui::Hsla)>,
    meta: Option<SharedString>,
    chevron: Option<bool>,
}

#[derive(Clone, PartialEq)]
struct Look {
    icon: Icon,
    chip: bool,
    tag: Option<SharedString>,
    chevron: bool,
}

impl Head {
    fn look(&self) -> Look {
        Look {
            icon: self.icon,
            chip: self.chip.is_some(),
            tag: self.tag.as_ref().map(|(label, _)| label.clone()),
            chevron: self.chevron.is_some(),
        }
    }
}

#[derive(Default)]
struct Rules {
    sweep: Option<(f32, f32)>,
    fill: Option<(f32, f32)>,
}

fn rules(
    live_since: Option<Instant>,
    settled_at: Option<Instant>,
    filled: bool,
    now: Instant,
    reduce: bool,
) -> Rules {
    if reduce {
        return Rules::default();
    }
    let glow = settled_at.map_or(1.0, |at| 1.0 - progress(SWEEP_FADE, at, now, false));
    Rules {
        sweep: live_since.filter(|_| glow > 0.0).map(|since| {
            let cycle =
                (now.saturating_duration_since(since).as_secs_f32() / SWEEP.as_secs_f32()).fract();
            (motion::EASE_IN_OUT.eval(cycle), glow)
        }),
        fill: settled_at.filter(|_| filled).map(|at| {
            (
                progress(FILL, at, now, false),
                1.0 - progress(FILL_FADE, at, now, false),
            )
        }),
    }
}

fn rule(left: bool, rules: &Rules, theme: &Theme) -> Div {
    let accent = theme.accent;
    div()
        .flex_1()
        .min_w(px(24.0))
        .h(px(1.0))
        .relative()
        .overflow_hidden()
        .bg(crate::theme::hairline(0.08))
        .children(rules.sweep.map(|(t, alpha)| {
            let (near, far) = (accent.opacity(alpha), accent.opacity(0.0));
            let at = gpui::relative(motion::lerp(-SWEEP_WIDTH, 1.0, t));
            let streak = div()
                .absolute()
                .top_0()
                .h_full()
                .w(gpui::relative(SWEEP_WIDTH));
            if left {
                streak.right(at).bg(gpui::linear_gradient(
                    90.0,
                    gpui::linear_color_stop(far, 0.0),
                    gpui::linear_color_stop(near, 1.0),
                ))
            } else {
                streak.left(at).bg(gpui::linear_gradient(
                    90.0,
                    gpui::linear_color_stop(near, 0.0),
                    gpui::linear_color_stop(far, 1.0),
                ))
            }
        }))
        .children(rules.fill.map(|(width, alpha)| {
            let fill = div()
                .absolute()
                .top_0()
                .h_full()
                .w(gpui::relative(width))
                .bg(theme.success.opacity(0.7 * alpha));
            if left { fill.right_0() } else { fill.left_0() }
        }))
}

fn verdict_tag(record: &ClassifierRecord, theme: &Theme) -> (SharedString, gpui::Hsla) {
    if record.decided_by == DecidedBy::BackgroundCopy {
        return ("Swapped in".into(), theme.warning);
    }
    let routine = record.decided_by.asked() || record.decided_by == DecidedBy::TurnEnd;
    let color = if !routine || record.verdict == ClassifierVerdict::Failed {
        theme.warning
    } else {
        match record.verdict {
            ClassifierVerdict::New => theme.success,
            ClassifierVerdict::Carry => theme.accent,
            _ => theme.text_muted,
        }
    };
    (record.verdict.label().into(), color)
}

fn reason(record: &ClassifierRecord) -> Option<SharedString> {
    record
        .reason
        .clone()
        .or_else(|| (!record.decided_by.asked()).then(|| record.decided_by.label().to_string()))
        .map(SharedString::from)
}

impl Transcript {
    fn glyph(&self, icon: Icon, scale: f32, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let glyph = |path: &'static str, color: gpui::Hsla| {
            crate::icons::icon(path)
                .size(px(13.0 * scale))
                .text_color(color)
                .into_any_element()
        };
        match icon {
            Icon::Spinner => crate::loaders::mini_mono_spinner(
                "compaction-spinner",
                2.0 * scale,
                theme.text_muted,
                cx.entity_id(),
                cx,
            )
            .into_any_element(),
            Icon::Done => glyph(crate::icons::LUCIDE_CHECK, theme.success),
            Icon::Alert => glyph(crate::icons::LUCIDE_CIRCLE_ALERT, theme.warning),
            Icon::Decision => glyph(crate::icons::LUCIDE_LIST_FILTER, theme.text_muted),
            Icon::Queued => glyph(crate::icons::LUCIDE_CIRCLE_DASHED, theme.text_faint),
            Icon::Kept => div()
                .text_color(theme.text_muted)
                .child("–")
                .into_any_element(),
        }
    }

    fn label_icon(
        &self,
        icon: Icon,
        from: Option<Icon>,
        p: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let layer = |glyph: AnyElement, opacity: f32| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .opacity(opacity)
                .child(glyph)
        };
        let from = from.filter(|from| *from != icon && p < 1.0);
        let (scale, shown) = match from {
            Some(_) => (motion::lerp(0.6, 1.0, p), p),
            None => (1.0, 1.0),
        };
        div()
            .relative()
            .size(px(14.0))
            .flex_none()
            .children(from.map(|from| {
                layer(
                    self.glyph(from, motion::lerp(1.0, 0.6, p), theme, cx),
                    1.0 - p,
                )
            }))
            .child(layer(self.glyph(icon, scale, theme, cx), shown))
            .into_any_element()
    }

    // `morph` is the look this label had before its last change and how far
    // the change has run; new pieces fade in, the chevron's slot grows.
    fn divider_label(
        &self,
        key: &SharedString,
        head: Head,
        morph: Option<(Option<Look>, f32)>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let reduce = cx.reduce_motion();
        let (from, p) = morph.unwrap_or((None, 1.0));
        let shown = |had: bool| if from.is_none() || had { 1.0 } else { p };
        let chip_in = shown(from.as_ref().is_some_and(|from| from.chip));
        let tag_in =
            shown(from.as_ref().is_some_and(|from| {
                from.tag.as_ref() == head.tag.as_ref().map(|(label, _)| label)
            }));
        let chevron_in = shown(from.as_ref().is_some_and(|from| from.chevron));
        let icon = self.label_icon(head.icon, from.as_ref().map(|from| from.icon), p, theme, cx);
        let small = crate::typography::ui_rems(11.5);
        div()
            .flex_none()
            .max_w(gpui::relative(0.8))
            .min_w_0()
            .flex()
            .items_center()
            .px(px(6.0))
            .py(px(2.0))
            .rounded(px(6.0))
            .overflow_hidden()
            .text_size(crate::typography::ui_rems(12.5))
            .child(icon)
            .child(
                div()
                    .ml(px(7.0))
                    .flex_none()
                    .text_color(theme.text_muted)
                    .child(crate::roll_text::roll_text(
                        format!("{key}-title"),
                        head.title,
                        reduce,
                    )),
            )
            .children(head.chip.map(|chip| {
                div()
                    .ml(px(7.0))
                    .flex_none()
                    .opacity(chip_in)
                    .text_size(small)
                    .text_color(theme.text_muted)
                    .child(chip)
            }))
            .children(head.tag.map(|(label, color)| {
                div()
                    .ml(px(7.0))
                    .flex_none()
                    .opacity(tag_in)
                    .px(px(8.0))
                    .py(px(1.0))
                    .rounded_full()
                    .border_1()
                    .border_color(color.opacity(0.4))
                    .text_size(small)
                    .text_color(color)
                    .child(label)
            }))
            .children(head.meta.map(|meta| {
                div()
                    .ml(px(7.0))
                    .min_w_0()
                    .text_size(small)
                    .text_color(theme.text_faint)
                    .child(crate::roll_text::roll_text(
                        format!("{key}-meta"),
                        meta,
                        reduce,
                    ))
            }))
            .children(head.chevron.map(|open| {
                div()
                    .flex_none()
                    .w(px(16.0 * chevron_in))
                    .opacity(chevron_in)
                    .overflow_hidden()
                    .flex()
                    .justify_end()
                    .child(
                        crate::icons::icon(if open {
                            crate::icons::LUCIDE_CHEVRON_DOWN
                        } else {
                            crate::icons::LUCIDE_CHEVRON_RIGHT
                        })
                        .size(px(12.0))
                        .text_color(theme.text_faint),
                    )
            }))
    }

    fn divider_line(label: impl IntoElement, rules: Rules, theme: &Theme) -> Div {
        div()
            .w_full()
            .h(px(24.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(rule(true, &rules, theme))
            .child(label)
            .child(rule(false, &rules, theme))
    }

    // The detail opens in a frame under the divider; its first section's top
    // border sits on the frame's.
    fn panel(content: impl IntoElement, theme: &Theme) -> Div {
        div()
            .w_full()
            .mt(px(6.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface_card.opacity(0.85))
            .overflow_hidden()
            .child(div().mt(px(-1.0)).flex().flex_col().child(content))
    }

    pub(super) fn render_compaction_wait(
        &mut self,
        chat_id: String,
        wait: CompactionWait,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let now = motion::now();
        if !self
            .compaction_clock
            .wait
            .as_ref()
            .is_some_and(|(id, ..)| *id == chat_id)
        {
            self.compaction_clock.tail = self.rows.last().map(|row| row.id.clone());
        }
        let since = self.compaction_clock.wait_since(&chat_id, &wait, now);
        if self.fresh_compaction(&chat_id, now).is_some() {
            return None;
        }
        if matches!(wait, CompactionWait::Compacting(Some(_)))
            && boundary_row(&self.rows).is_some_and(|ix| {
                matches!(
                    self.rows[ix].kind,
                    RowKind::Classifier {
                        compaction: None,
                        ..
                    }
                )
            })
        {
            return None;
        }
        let reduce = cx.reduce_motion();
        motion::pulse_lease(cx.entity_id(), cx);
        let theme = Theme::of(cx).clone();
        let elapsed = now.saturating_duration_since(since);
        let usual = usual_secs(&self.rows, usize::MAX);
        let (phase, head) = match &wait {
            CompactionWait::Queued => (
                Phase::Queued,
                Head {
                    icon: Icon::Queued,
                    title: "Compaction queued".into(),
                    chip: None,
                    tag: None,
                    meta: Some("· starts when the turn settles".into()),
                    chevron: None,
                },
            ),
            CompactionWait::Classifying {
                harness,
                model,
                agent_note,
            } => (
                Phase::Checking,
                Head {
                    icon: Icon::Spinner,
                    title: checking_title(*agent_note).into(),
                    chip: Some(HandoffSide::new(*harness, model.as_deref(), cx).chip(&theme)),
                    tag: None,
                    meta: Some(format!("· {}", format_elapsed(elapsed.as_secs() as i64)).into()),
                    chevron: None,
                },
            ),
            CompactionWait::Compacting(boundary) => (
                Phase::Compacting,
                Head {
                    icon: Icon::Spinner,
                    title: "Compacting".into(),
                    chip: None,
                    tag: boundary.map(|boundary| {
                        (
                            SharedString::from(if boundary.carry {
                                "Carry over"
                            } else {
                                "New task"
                            }),
                            theme.accent,
                        )
                    }),
                    meta: Some(
                        match boundary {
                            Some(boundary) => {
                                format!(
                                    "· {} · {}",
                                    tokens(boundary.tokens),
                                    wait_meta(elapsed, usual)
                                )
                            }
                            None => format!("· {}", wait_meta(elapsed, usual)),
                        }
                        .into(),
                    ),
                    chevron: None,
                },
            ),
        };
        let changed = self.compaction_clock.phase_since(&chat_id, phase, now);
        let entered = self
            .compaction_clock
            .entered_at(&chat_id)
            .unwrap_or(changed);
        let enter = progress(motion::RESIZE, entered, now, reduce);
        let key = self.compaction_clock.label_key(&chat_id);
        let (from, at) = self.compaction_clock.look_since(&key, &head.look(), now);
        let morph = (from, progress(motion::RESIZE, at, now, reduce));
        let rules = rules(
            (phase != Phase::Queued).then_some(entered),
            None,
            false,
            now,
            reduce,
        );
        let label = self.divider_label(&key, head, Some(morph), &theme, cx);
        Some(
            div()
                .pt(px(Theme::SPACE_LG))
                .child(
                    Self::divider_line(label, rules, &theme)
                        .relative()
                        .top(px(8.0 * (1.0 - enter)))
                        .opacity(enter),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_compaction_card(
        &mut self,
        ix: usize,
        row_id: &SharedString,
        record: Arc<ClassifierRecord>,
        kept: Option<Arc<CompactionRecord>>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = motion::now();
        let reduce = cx.reduce_motion();
        let live_chat = self.chat_id.clone().filter(|_| self.doc_override.is_none());
        let boundary = boundary_row(&self.rows) == Some(ix);
        let compacts = matches!(
            record.verdict,
            ClassifierVerdict::New | ClassifierVerdict::Carry
        );
        let mut compacting_since = None;
        let mut settled_at = self.compaction_clock.settled.get(row_id).copied();
        if let Some(chat_id) = live_chat.as_deref().filter(|_| boundary) {
            let wait = self.compaction_wait(chat_id, chrono::Utc::now(), cx);
            if kept.is_none() && compacts {
                if let Some(wait @ CompactionWait::Compacting(Some(_))) = &wait {
                    compacting_since = Some(self.compaction_clock.wait_since(chat_id, wait, now));
                } else if let Some((phase, at)) = self.compaction_clock.live_phase(chat_id)
                    && self.compaction_clock.wait_ended_recently(chat_id, now)
                {
                    compacting_since = Some(if phase == Phase::Compacting { at } else { now });
                }
            } else if self.compaction_clock.live_phase(chat_id).is_some() {
                self.compaction_clock.settled.insert(row_id.clone(), now);
                settled_at = Some(now);
            }
            if compacting_since.is_some() {
                self.compaction_clock
                    .phase_since(chat_id, Phase::Compacting, now);
            } else if settled_at.is_some() {
                self.compaction_clock
                    .phase_since(chat_id, Phase::Settled, now);
            }
        }
        if let Some(at) = settled_at
            && (reduce || now.saturating_duration_since(at) > SETTLE)
        {
            self.compaction_clock.settled.remove(row_id);
            settled_at = None;
        }
        let fold = self.folds.get(row_id).copied().unwrap_or_default();
        let open = fold.open.unwrap_or(false);
        let animating = compacting_since.is_some()
            || settled_at.is_some()
            || fold
                .toggled_at
                .is_some_and(|at| now.saturating_duration_since(at) < motion::RESIZE.total());
        if animating {
            motion::pulse_lease(cx.entity_id(), cx);
        }

        let context = kept
            .as_ref()
            .and_then(|kept| kept.pre_tokens)
            .unwrap_or(record.context_tokens);
        let tag = Some(verdict_tag(&record, theme));
        let settled = compacting_since.is_none();
        let head = match (&kept, compacting_since) {
            (_, Some(since)) => {
                let elapsed = now.saturating_duration_since(since);
                Head {
                    icon: Icon::Spinner,
                    title: "Compacting".into(),
                    chip: None,
                    tag,
                    meta: Some(
                        format!(
                            "· {} · {}",
                            tokens(context),
                            wait_meta(elapsed, usual_secs(&self.rows, ix))
                        )
                        .into(),
                    ),
                    chevron: None,
                }
            }
            (Some(kept), None) => {
                let mut meta = format!("· {}", tokens(context));
                if let Some(post) = kept.post_tokens {
                    meta.push_str(&format!(" → {}", tokens(post)));
                }
                let copy = record.decided_by == DecidedBy::BackgroundCopy;
                if copy {
                    meta.push_str(&match carried_turns(kept.carried.as_deref()) {
                        0 => " · nothing carried".to_string(),
                        1 => " · 1 turn as text".to_string(),
                        turns => format!(" · {turns} turns as text"),
                    });
                } else if let Some(ms) = kept.duration_ms {
                    meta.push_str(&format!(" · {}", format_elapsed((ms / 1000) as i64)));
                }
                Head {
                    icon: Icon::Done,
                    title: if copy {
                        "Compacted in the background"
                    } else {
                        "Compacted"
                    }
                    .into(),
                    chip: None,
                    tag,
                    meta: Some(meta.into()),
                    chevron: Some(open),
                }
            }
            (None, None) => {
                let (icon, title, meta) = match record.verdict {
                    ClassifierVerdict::Continue => (
                        Icon::Kept,
                        "Not compacted",
                        format!("· {} · context kept", tokens(context)),
                    ),
                    ClassifierVerdict::Failed => (
                        Icon::Alert,
                        "Not compacted",
                        format!("· {} · the classifier gave no answer", tokens(context)),
                    ),
                    _ => (
                        Icon::Decision,
                        "Classifier",
                        format!(
                            "· {} · {:.1}s",
                            tokens(context),
                            record.duration_ms as f64 / 1000.0
                        ),
                    ),
                };
                Head {
                    icon,
                    title: title.into(),
                    chip: None,
                    tag,
                    meta: Some(meta.into()),
                    chevron: Some(open),
                }
            }
        };
        let live_key = live_chat
            .as_deref()
            .filter(|_| boundary)
            .map(|chat_id| self.compaction_clock.label_key(chat_id));
        let morph = live_key.as_ref().map(|key| {
            let (from, at) = self.compaction_clock.look_since(key, &head.look(), now);
            (from, progress(motion::RESIZE, at, now, reduce))
        });
        let live_since = live_chat
            .as_deref()
            .and_then(|chat_id| self.compaction_clock.entered_at(chat_id))
            .or(compacting_since)
            .filter(|_| compacting_since.is_some() || settled_at.is_some());
        let rules = rules(live_since, settled_at, kept.is_some(), now, reduce);
        let key = live_key.unwrap_or_else(|| row_id.clone());
        let toggle_id = row_id.clone();
        let hover = theme.wash(0.08);
        let label = self
            .divider_label(&key, head, morph, theme, cx)
            .id(SharedString::from(format!("{row_id}-head")))
            .when(settled, |label| {
                label
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let fold = this.folds.entry(toggle_id.clone()).or_default();
                        fold.open = Some(!fold.open.unwrap_or(false));
                        fold.toggled_at = Some(motion::now());
                        this.diagram_layout_changed(std::slice::from_ref(&toggle_id), cx);
                    }))
            });
        let body = (settled && open).then(|| {
            let shown = fold
                .toggled_at
                .map_or(1.0, |at| progress(motion::FADE_QUICK, at, now, reduce));
            Self::panel(
                div()
                    .flex()
                    .flex_col()
                    .children(
                        kept.as_ref()
                            .and_then(|kept| self.kept_section(row_id, kept, theme)),
                    )
                    .child(self.classifier_section(row_id, &record, theme, cx)),
                theme,
            )
            .relative()
            .top(px(-4.0 * (1.0 - shown)))
            .opacity(shown)
        });
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(Self::divider_line(label, rules, theme))
            .children(body)
            .into_any_element()
    }

    fn section(title: &'static str, theme: &Theme) -> Div {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .pl(px(INSET))
            .pr(px(12.0))
            .pt(px(8.0))
            .pb(px(10.0))
            .border_t_1()
            .border_color(theme.border)
            .text_size(crate::typography::ui_rems(12.5))
            .child(
                div()
                    .text_size(crate::typography::ui_rems(11.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text_muted)
                    .child(title),
            )
    }

    fn text_box(id: SharedString, text: &str, theme: &Theme) -> impl IntoElement {
        div()
            .id(id)
            .max_h(px(240.0))
            .overflow_y_scroll()
            .pl(px(10.0))
            .border_l_2()
            .border_color(theme.border)
            .line_height(px(19.0))
            .text_color(theme.text)
            .children(text.lines().map(|line| {
                div()
                    .w_full()
                    .min_h(px(19.0))
                    .child(SharedString::from(line.to_string()))
            }))
    }

    fn kept_section(
        &self,
        row_id: &SharedString,
        kept: &CompactionRecord,
        theme: &Theme,
    ) -> Option<Div> {
        if kept.summary.is_none() && kept.carried.is_none() {
            return None;
        }
        let sub = |text: &'static str| div().pt(px(4.0)).text_color(theme.text_muted).child(text);
        Some(
            Self::section("WHAT THE AGENT KEPT", theme)
                .children(kept.summary.as_deref().map(|summary| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(sub("Summary the agent wrote"))
                        .child(Self::text_box(
                            format!("{row_id}-summary").into(),
                            summary,
                            theme,
                        ))
                }))
                .child(match kept.carried.as_deref() {
                    Some(carried) => div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(sub("Carried over word for word"))
                        .child(Self::text_box(
                            format!("{row_id}-carried").into(),
                            carried_body(carried),
                            theme,
                        )),
                    None => {
                        sub("Nothing carried word for word: the next turn starts from the summary")
                    }
                }),
        )
    }

    fn classifier_section(
        &self,
        row_id: &SharedString,
        record: &ClassifierRecord,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let field = |key: &'static str, value: String| {
            div()
                .flex()
                .gap(px(10.0))
                .child(
                    div()
                        .w(px(96.0))
                        .flex_none()
                        .text_color(theme.text_muted)
                        .child(key),
                )
                .child(
                    div()
                        .min_w_0()
                        .text_color(theme.text)
                        .child(SharedString::from(value)),
                )
        };
        let label = ClassifierVerdict::label;
        let decision = match record.classifier_verdict {
            Some(said) => format!("{} (said {})", label(record.verdict), label(said)),
            None => label(record.verdict).to_string(),
        };
        let asked = record.decided_by.asked();
        let model = record
            .harness
            .map(|harness| HandoffSide::new(harness, record.model.as_deref(), cx).detail())
            .or_else(|| record.model.clone());
        let decided = match (asked, model) {
            (true, Some(model)) => format!(
                "{model} · {} · {:.1}s",
                record.decided_by.label(),
                record.duration_ms as f64 / 1000.0
            ),
            _ => record.decided_by.label().to_string(),
        };
        let usage = match (record.input_tokens, record.output_tokens) {
            (Some(input), Some(output)) => {
                Some(format!("{} in · {} out", tokens(input), tokens(output)))
            }
            _ => None,
        };
        let input_id: SharedString = format!("{row_id}-input").into();
        let input_open = self
            .folds
            .get(&input_id)
            .and_then(|fold| fold.open)
            .unwrap_or(false);
        let toggle_id = row_id.clone();
        let link = div()
            .id(SharedString::from(format!("{row_id}-input-link")))
            .pt(px(4.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .cursor_pointer()
            .text_color(theme.text_muted)
            .hover(|s| s.text_color(theme.text))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.folds.entry(input_id.clone()).or_default().open = Some(!input_open);
                this.diagram_layout_changed(std::slice::from_ref(&toggle_id), cx);
            }))
            .child(
                crate::icons::icon(if input_open {
                    crate::icons::LUCIDE_CHEVRON_DOWN
                } else {
                    crate::icons::LUCIDE_CHEVRON_RIGHT
                })
                .size(px(11.0))
                .text_color(theme.text_muted),
            )
            .child(if asked {
                "What the classifier saw"
            } else {
                "What it would have seen (not sent)"
            });
        Self::section("CLASSIFIER", theme)
            .child(field("Decision", decision))
            .children(reason(record).map(|reason| field("Reason", reason.to_string())))
            .child(field("Decided by", decided))
            .child(field(
                "Context",
                format!("{} tokens", tokens(record.context_tokens)),
            ))
            .children(usage.map(|usage| field("Tokens", usage)))
            .when(!record.input.is_empty(), |section| section.child(link))
            .when(input_open, |section| {
                section.child(self.classifier_io(row_id, record, theme))
            })
    }

    fn classifier_io(
        &self,
        row_id: &SharedString,
        record: &ClassifierRecord,
        theme: &Theme,
    ) -> Div {
        let mono_box = |id: &'static str, text: &str, border: gpui::Hsla| {
            div()
                .id(SharedString::from(format!("{row_id}-{id}")))
                .max_h(px(224.0))
                .overflow_y_scroll()
                .p(px(10.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(border)
                .font_family(theme.font_mono.clone())
                .text_size(px(12.0))
                .line_height(px(18.0))
                .text_color(theme.text_faint)
                .children(text.lines().map(|line| {
                    div()
                        .w_full()
                        .min_h(px(18.0))
                        .child(SharedString::from(line.to_string()))
                }))
        };
        let titled = |title: SharedString, id: &'static str, text: &str, border: gpui::Hsla| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(div().text_color(theme.text_muted).child(title))
                .child(mono_box(id, text, border))
        };
        let asked = record.decided_by.asked();
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(mono_box("input", &record.input, theme.border))
            .when(asked, |io| {
                io.child(match record.stage2_reply.as_deref() {
                    None => titled("Reply".into(), "reply", &record.reply, theme.border),
                    Some(stage2) => {
                        let decides = |stage: DecidedBy| {
                            if record.decided_by == stage {
                                theme.accent
                            } else {
                                theme.border
                            }
                        };
                        div()
                            .flex()
                            .gap(px(10.0))
                            .child(titled(
                                "First look".into(),
                                "reply",
                                &record.reply,
                                decides(DecidedBy::Stage1),
                            ))
                            .child(titled(
                                "Second look".into(),
                                "stage2",
                                stage2,
                                decides(DecidedBy::Stage2),
                            ))
                    }
                })
            })
    }

    fn fresh_compaction(&self, chat_id: &str, now: Instant) -> Option<usize> {
        let ix = self
            .rows
            .iter()
            .rposition(|row| !matches!(row.kind, RowKind::User { .. }))?;
        let row = &self.rows[ix];
        let clock = &self.compaction_clock;
        let settling = clock.settled.contains_key(&row.id)
            || clock
                .live_phase(chat_id)
                .is_some_and(|(phase, _)| phase == Phase::Compacting)
                && clock.wait_ended_recently(chat_id, now);
        (matches!(row.kind, RowKind::Compaction { .. })
            && clock.tail.as_ref() != Some(&row.id)
            && settling)
            .then_some(ix)
    }
    pub(super) fn render_compaction_divider(
        &mut self,
        ix: usize,
        row_id: &SharedString,
        text: SharedString,
        record: Arc<CompactionRecord>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = motion::now();
        let reduce = cx.reduce_motion();
        let live_chat = self.chat_id.clone().filter(|_| self.doc_override.is_none());
        if let Some(chat_id) = live_chat.as_deref()
            && !self.compaction_clock.settled.contains_key(row_id)
            && self.fresh_compaction(chat_id, now) == Some(ix)
        {
            self.compaction_clock.settled.insert(row_id.clone(), now);
            self.compaction_clock
                .phase_since(chat_id, Phase::Settled, now);
        }
        let mut settled_at = self.compaction_clock.settled.get(row_id).copied();
        if let Some(at) = settled_at
            && (reduce || now.saturating_duration_since(at) > SETTLE)
        {
            self.compaction_clock.settled.remove(row_id);
            settled_at = None;
        }
        if settled_at.is_some() {
            motion::pulse_lease(cx.entity_id(), cx);
        }
        let kept = self.kept_section(row_id, &record, theme);
        let open = self
            .folds
            .get(row_id)
            .and_then(|fold| fold.open)
            .unwrap_or(false);
        let meta = record.pre_tokens.map(|pre| {
            let mut meta = format!("· {}", tokens(pre));
            if let Some(post) = record.post_tokens {
                meta.push_str(&format!(" → {}", tokens(post)));
            }
            if let Some(ms) = record.duration_ms {
                meta.push_str(&format!(" · {}", format_elapsed((ms / 1000) as i64)));
            }
            SharedString::from(meta)
        });
        let head = Head {
            icon: Icon::Done,
            title: if meta.is_none() && !text.is_empty() {
                text
            } else {
                "Compacted".into()
            },
            chip: None,
            tag: None,
            meta,
            chevron: kept.is_some().then_some(open),
        };
        let live_key = live_chat
            .as_deref()
            .filter(|_| settled_at.is_some())
            .map(|chat_id| self.compaction_clock.label_key(chat_id));
        let morph = live_key.as_ref().map(|key| {
            let (from, at) = self.compaction_clock.look_since(key, &head.look(), now);
            (from, progress(motion::RESIZE, at, now, reduce))
        });
        let live_since = live_chat
            .as_deref()
            .and_then(|chat_id| self.compaction_clock.entered_at(chat_id))
            .filter(|_| settled_at.is_some());
        let rules = rules(live_since, settled_at, true, now, reduce);
        let key = live_key.unwrap_or_else(|| row_id.clone());
        let toggle_id = row_id.clone();
        let hover = theme.wash(0.08);
        let label = self
            .divider_label(&key, head, morph, theme, cx)
            .id(SharedString::from(format!("{row_id}-toggle")))
            .when(kept.is_some(), |label| {
                label
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.folds.entry(toggle_id.clone()).or_default().open = Some(!open);
                        this.diagram_layout_changed(std::slice::from_ref(&toggle_id), cx);
                    }))
            });
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(Self::divider_line(label, rules, theme))
            .children(kept.filter(|_| open).map(|kept| Self::panel(kept, theme)))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, kind: RowKind) -> Row {
        Row {
            id: id.to_string().into(),
            version: fnv1a(id.as_bytes()),
            turn_start: true,
            kind,
            entry_id: id.to_string().into(),
            timestamp: None,
            copy_text: None,
            compact_fold: None,
        }
    }

    fn classifier(id: &str) -> Row {
        row(
            id,
            RowKind::Classifier {
                record: Arc::new(ClassifierRecord::default()),
                compaction: None,
            },
        )
    }

    fn compaction(id: &str, post: u64) -> Row {
        row(
            id,
            RowKind::Compaction {
                text: "Context compacted".into(),
                record: Arc::new(CompactionRecord {
                    post_tokens: Some(post),
                    ..Default::default()
                }),
            },
        )
    }

    fn user(id: &str) -> Row {
        row(
            id,
            RowKind::User {
                text: "how is it going".into(),
                mentions: Arc::default(),
                markdown: None,
                attachments: Arc::default(),
                badges: Arc::default(),
                sender: None,
                pending: false,
                from_chat_id: None,
            },
        )
    }

    fn divider(id: &str) -> Row {
        row(
            id,
            RowKind::Divider {
                text: "Host restarted".into(),
                hint: None,
            },
        )
    }

    #[test]
    fn a_swapped_copy_counts_the_turns_it_carried_as_text() {
        let head = "<carried_over>\nThe last turns of this conversation.\n\n";
        assert_eq!(carried_turns(None), 0);
        assert_eq!(
            carried_turns(Some(&format!(
                "{head}person: refund\nagent: done\n</carried_over>"
            ))),
            1
        );
        assert_eq!(
            carried_turns(Some(&format!(
                "{head}person: a\nagent: b\n\nperson: c\nagent: d\n\nStill running when the chat compacted:\n- background shell x: y\n</carried_over>"
            ))),
            2
        );
    }

    #[test]
    fn a_compaction_folds_into_the_decision_right_before_it() {
        let mut rows = vec![
            classifier("k1"),
            compaction("c1", 10),
            classifier("k2"),
            divider("d"),
            compaction("c2", 20),
        ];
        let decided = rows[0].version;
        merge_compaction_rows(&mut rows);
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_ref()).collect();
        assert_eq!(ids, ["k1", "k2", "d", "c2"]);
        assert!(matches!(
            &rows[0].kind,
            RowKind::Classifier { compaction: Some(record), .. } if record.post_tokens == Some(10)
        ));
        assert!(matches!(
            &rows[1].kind,
            RowKind::Classifier {
                compaction: None,
                ..
            }
        ));
        assert_ne!(rows[0].version, decided);

        let mut sent_meanwhile = vec![classifier("k1"), user("u1"), compaction("c1", 10)];
        merge_compaction_rows(&mut sent_meanwhile);
        let ids: Vec<&str> = sent_meanwhile.iter().map(|row| row.id.as_ref()).collect();
        assert_eq!(ids, ["k1", "u1"]);
        assert!(matches!(
            &sent_meanwhile[0].kind,
            RowKind::Classifier { compaction: Some(record), .. } if record.post_tokens == Some(10)
        ));

        let mut later = vec![classifier("k1"), compaction("c1", 10)];
        later[1].version ^= 1;
        merge_compaction_rows(&mut later);
        let mut first = vec![classifier("k1"), compaction("c1", 10)];
        merge_compaction_rows(&mut first);
        assert_ne!(later[0].version, first[0].version);
    }
}
