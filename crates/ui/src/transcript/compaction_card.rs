use super::*;
use gpui::Div;
use zeron_doc::{ClassifierRecord, ClassifierVerdict, CompactionRecord, DecidedBy};

const SETTLE_HOLD: Duration = Duration::from_millis(700);
const COUNT: Duration = Duration::from_millis(600);
// The compaction flag drops before its entry lands; hold Compacting across the gap.
const HOLD_AFTER_WAIT: Duration = Duration::from_secs(5);
const DETAIL_MAX: f32 = 96.0;
const INSET: f32 = 34.0;

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
    settled: HashMap<SharedString, Instant>,
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
}

pub(super) fn merge_compaction_rows(rows: &mut Vec<Row>) {
    let mut ix = 0;
    while ix + 1 < rows.len() {
        if let (
            RowKind::Classifier {
                compaction: None, ..
            },
            RowKind::Compaction { record, .. },
        ) = (&rows[ix].kind, &rows[ix + 1].kind)
        {
            let record = record.clone();
            let version = rows.remove(ix + 1).version;
            let row = &mut rows[ix];
            row.version = row.version.rotate_left(7) ^ version;
            if let RowKind::Classifier { compaction, .. } = &mut row.kind {
                *compaction = Some(record);
            }
        }
        ix += 1;
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

fn bar_fill(elapsed: Duration, usual: Option<u64>) -> f32 {
    let secs = elapsed.as_secs_f32();
    match usual.filter(|usual| *usual > 0) {
        Some(usual) => 0.92 * motion::EASE_OUT.eval((secs / usual as f32).min(1.0)),
        None => 0.92 * (1.0 - (-secs / 40.0).exp()),
    }
}

fn carried_body(carried: &str) -> &str {
    let inner = carried
        .trim()
        .trim_start_matches("<carried_over>")
        .trim_end_matches("</carried_over>")
        .trim();
    inner.split_once("\n\n").map_or(inner, |(_, rest)| rest)
}

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

fn verdict_tag(record: &ClassifierRecord, theme: &Theme) -> (SharedString, gpui::Hsla) {
    let color = if !record.decided_by.asked() || record.verdict == ClassifierVerdict::Failed {
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
    fn card_icon(&self, icon: Icon, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let glyph = |path: &'static str, color: gpui::Hsla| {
            crate::icons::icon(path)
                .size(px(13.0))
                .text_color(color)
                .into_any_element()
        };
        let inner = match icon {
            Icon::Spinner => {
                crate::loaders::gradient_spinner("compaction-card", theme, 2.5, cx.entity_id(), cx)
                    .into_any_element()
            }
            Icon::Done => glyph(crate::icons::LUCIDE_CHECK, theme.success),
            Icon::Alert => glyph(crate::icons::LUCIDE_CIRCLE_ALERT, theme.warning),
            Icon::Decision => glyph(crate::icons::LUCIDE_LIST_FILTER, theme.text_muted),
            Icon::Queued => glyph(crate::icons::LUCIDE_CIRCLE_DASHED, theme.text_faint),
            Icon::Kept => div()
                .text_color(theme.text_muted)
                .child("–")
                .into_any_element(),
        };
        div()
            .size(px(14.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(inner)
            .into_any_element()
    }

    // Shared by the trailer and the row so the handoff between them never moves a pixel.
    fn card_head(
        &self,
        head: Head,
        card: f32,
        fade: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let icon = self.card_icon(head.icon, theme, cx);
        let title_color = motion::mix(theme.text_muted, theme.text, card);
        div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(motion::lerp(4.0, 9.0, card)))
            .text_size(crate::typography::ui_rems(12.5))
            .child(icon)
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .opacity(fade)
                    .child(
                        div()
                            .flex_none()
                            .text_color(title_color)
                            .when(card > 0.5, |title| {
                                title.font_weight(gpui::FontWeight::SEMIBOLD)
                            })
                            .child(head.title),
                    )
                    .children(head.chip.map(|chip| {
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .child(chip)
                    }))
                    .children(head.tag.map(|(label, color)| {
                        div()
                            .flex_none()
                            .px(px(8.0))
                            .py(px(1.0))
                            .rounded_full()
                            .border_1()
                            .border_color(color.opacity(0.4))
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(color)
                            .child(label)
                    }))
                    .children(head.meta.map(|meta| {
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text_faint)
                            .child(meta)
                    })),
            )
            .children(head.chevron.map(|open| {
                div().ml_auto().flex_none().child(
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

    fn card_frame(card: f32, theme: &Theme) -> Div {
        div()
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.border.opacity(card))
            .bg(theme.surface_card.opacity(0.85 * card))
    }

    fn card_detail(
        reason: Option<SharedString>,
        bar: Option<f32>,
        reveal: f32,
        theme: &Theme,
    ) -> Option<Div> {
        (reveal > 0.0 && (reason.is_some() || bar.is_some())).then(|| {
            div()
                .w_full()
                .max_h(px(DETAIL_MAX * reveal))
                .overflow_hidden()
                .opacity(reveal)
                .children(reason.map(|reason| {
                    div()
                        .pl(px(INSET))
                        .pr(px(12.0))
                        .pb(px(9.0))
                        .text_size(crate::typography::ui_rems(12.5))
                        .line_height(px(19.0))
                        .text_color(theme.text_muted)
                        .child(reason)
                }))
                .children(bar.map(|fill| {
                    div().pl(px(INSET)).pr(px(12.0)).pb(px(10.0)).child(
                        div()
                            .w_full()
                            .h(px(2.0))
                            .rounded(px(2.0))
                            .bg(theme.border)
                            .child(
                                div()
                                    .h_full()
                                    .w(gpui::relative(fill.clamp(0.0, 1.0)))
                                    .rounded(px(2.0))
                                    .bg(theme.accent),
                            ),
                    )
                }))
        })
    }

    pub(super) fn render_compaction_wait(
        &mut self,
        chat_id: String,
        wait: CompactionWait,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let now = motion::now();
        let since = self.compaction_clock.wait_since(&chat_id, &wait, now);
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
        let (phase, head, bar) = match &wait {
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
                None,
            ),
            CompactionWait::Classifying {
                harness,
                model,
                agent_note,
            } => (
                Phase::Checking,
                Head {
                    icon: Icon::Spinner,
                    title: if *agent_note {
                        "Checking the agent's note"
                    } else {
                        "Checking whether to compact"
                    }
                    .into(),
                    chip: Some(HandoffSide::new(*harness, model.as_deref(), cx).chip(&theme)),
                    tag: None,
                    meta: Some(format!("· {:.1}s", elapsed.as_secs_f32()).into()),
                    chevron: None,
                },
                None,
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
                Some(bar_fill(elapsed, usual)),
            ),
        };
        let changed = self.compaction_clock.phase_since(&chat_id, phase, now);
        let entered = self
            .compaction_clock
            .entered_at(&chat_id)
            .unwrap_or(changed);
        let enter = progress(motion::RESIZE, entered, now, reduce);
        let fade = progress(motion::FADE_QUICK, changed, now, reduce);
        let reveal = progress(motion::RESIZE, changed, now, reduce);
        let head = self.card_head(head, 1.0, fade, &theme, cx);
        Some(
            div()
                .pt(px(Theme::SPACE_LG))
                .child(
                    Self::card_frame(1.0, &theme)
                        .relative()
                        .top(px(8.0 * (1.0 - enter)))
                        .opacity(enter)
                        .child(head)
                        .children(Self::card_detail(None, bar, reveal, &theme)),
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
        let hold = if kept.is_some() {
            SETTLE_HOLD
        } else {
            SETTLE_HOLD + Duration::from_millis(200)
        };
        if let Some(at) = settled_at
            && (reduce || now.saturating_duration_since(at) > hold + motion::RESIZE.total())
        {
            self.compaction_clock.settled.remove(row_id);
            settled_at = None;
        }
        let fold = self.folds.get(row_id).copied().unwrap_or_default();
        let open = fold.open.unwrap_or(false);
        let toggle = fold
            .toggled_at
            .map_or(1.0, |at| progress(motion::RESIZE, at, now, reduce));
        let base = if compacting_since.is_some() {
            1.0
        } else {
            settled_at.map_or(0.0, |at| {
                let held = now.saturating_duration_since(at).saturating_sub(hold);
                1.0 - eased(motion::RESIZE, held, reduce)
            })
        };
        let card = if open {
            motion::lerp(base, 1.0, toggle)
        } else {
            motion::lerp(1.0, base, toggle)
        };
        let phase_at = live_chat
            .as_deref()
            .and_then(|chat_id| {
                self.compaction_clock
                    .phase
                    .as_ref()
                    .filter(|(id, ..)| id == chat_id)
            })
            .map(|(_, _, at)| *at)
            .filter(|_| compacting_since.is_some() || settled_at.is_some());
        let fade = phase_at.map_or(1.0, |at| progress(motion::FADE_QUICK, at, now, reduce));
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
                let post = kept.post_tokens.map(|post| {
                    let t = settled_at.map_or(1.0, |at| {
                        let counted = now
                            .saturating_duration_since(at)
                            .saturating_sub(motion::FADE_QUICK.total());
                        motion::EASE_OUT.eval(counted.as_secs_f32() / COUNT.as_secs_f32())
                    });
                    motion::lerp(context as f32, post as f32, t).round() as u64
                });
                let mut meta = format!("· {}", tokens(context));
                if let Some(post) = post {
                    meta.push_str(&format!(" → {}", tokens(post)));
                }
                if let Some(ms) = kept.duration_ms {
                    meta.push_str(&format!(" · {}", format_elapsed((ms / 1000) as i64)));
                }
                Head {
                    icon: Icon::Done,
                    title: "Compacted".into(),
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
        let bar = match (compacting_since, &kept) {
            (Some(since), _) => Some(bar_fill(
                now.saturating_duration_since(since),
                usual_secs(&self.rows, ix),
            )),
            (None, Some(_)) if settled_at.is_some() => Some(1.0),
            _ => None,
        };
        let reveal = match (compacting_since, phase_at) {
            (Some(_), Some(at)) => progress(motion::RESIZE, at, now, reduce),
            (Some(_), None) => 1.0,
            _ => card,
        };
        let toggle_id = row_id.clone();
        let head = self
            .card_head(head, card, fade, theme, cx)
            .id(SharedString::from(format!("{row_id}-head")))
            .when(settled, |head| {
                head.cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let fold = this.folds.entry(toggle_id.clone()).or_default();
                        fold.open = Some(!fold.open.unwrap_or(false));
                        fold.toggled_at = Some(motion::now());
                        this.diagram_layout_changed(std::slice::from_ref(&toggle_id), cx);
                    }))
            });
        let body = (settled && open).then(|| {
            let shown = progress(
                motion::FADE_QUICK,
                fold.toggled_at.unwrap_or(now),
                now,
                reduce,
            );
            div()
                .w_full()
                .relative()
                .top(px(-4.0 * (1.0 - shown)))
                .opacity(if fold.toggled_at.is_some() {
                    shown
                } else {
                    1.0
                })
                .children(
                    kept.as_ref()
                        .and_then(|kept| self.kept_section(row_id, kept, theme)),
                )
                .child(self.classifier_section(row_id, &record, theme, cx))
        });
        Self::card_frame(card, theme)
            .child(head)
            .children(Self::card_detail(reason(&record), bar, reveal, theme))
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
            .child(link)
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

    pub(super) fn render_compaction_divider(
        &mut self,
        row_id: &SharedString,
        text: SharedString,
        record: Arc<CompactionRecord>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(kept) = self.kept_section(row_id, &record, theme) else {
            return divider_row(row_id, text, None, theme);
        };
        let open = self
            .folds
            .get(row_id)
            .and_then(|fold| fold.open)
            .unwrap_or(false);
        let muted = theme.text_muted.opacity(0.7);
        let bright = theme.text_muted;
        let toggle_id = row_id.clone();
        let rule = || div().flex_1().h(px(1.0)).bg(crate::theme::hairline(0.08));
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .py(px(4.0))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(rule())
                    .child(
                        div()
                            .id(SharedString::from(format!("{row_id}-toggle")))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .cursor_pointer()
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(muted)
                            .hover(move |s| s.text_color(bright))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.folds.entry(toggle_id.clone()).or_default().open = Some(!open);
                                this.diagram_layout_changed(std::slice::from_ref(&toggle_id), cx);
                            }))
                            .child(text)
                            .child(
                                crate::icons::icon(if open {
                                    crate::icons::LUCIDE_CHEVRON_DOWN
                                } else {
                                    crate::icons::LUCIDE_CHEVRON_RIGHT
                                })
                                .size(px(11.0))
                                .text_color(muted.opacity(0.7)),
                            ),
                    )
                    .child(rule()),
            )
            .when(open, |row| {
                row.child(
                    Self::card_frame(1.0, theme)
                        .mt(px(6.0))
                        .overflow_hidden()
                        .child(kept),
                )
            })
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

        let mut later = vec![classifier("k1"), compaction("c1", 10)];
        later[1].version ^= 1;
        merge_compaction_rows(&mut later);
        let mut first = vec![classifier("k1"), compaction("c1", 10)];
        merge_compaction_rows(&mut first);
        assert_ne!(later[0].version, first[0].version);
    }
}
