use std::cell::Cell;
use std::rc::Rc;

use gpui::Bounds;

use super::*;
use crate::settings::TreeView;
use crate::tasks::morph::{Morph, Rects};
use crate::tasks::thread_tree::{NodeKind, TreeNode};
use crate::tasks::tree::{self, Status};

pub(super) const SLIDE: f32 = 48.0;

#[derive(Default)]
pub(super) struct ViewMorph {
    shown: Option<TreeView>,
    from_view: Option<TreeView>,
    morph: Option<Morph>,
    pub cards: Painted,
    body: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl TaskCenter {
    fn view_rects(&self, view: TreeView, cx: &App) -> Rects {
        match view {
            TreeView::Spine => self.views.cards.borrow().clone(),
            TreeView::Outline | TreeView::Timeline => self
                .outline
                .as_ref()
                .map(|(_, tree, _)| {
                    tree.read(cx)
                        .rects()
                        .borrow()
                        .iter()
                        .map(|(id, rect)| (Key::Node(id.clone()), *rect))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn clear_view_rects(&self, cx: &App) {
        self.views.cards.borrow_mut().clear();
        if let Some((_, tree, _)) = &self.outline {
            tree.read(cx).rects().borrow_mut().clear();
        }
    }

    fn rects_at(&self, morph: &Morph, targets: &Rects, elapsed: f32) -> Rects {
        let mut rects = targets.clone();
        for (index, (key, from)) in morph.from.iter().enumerate() {
            if let Some(to) = targets.get(key) {
                let t = morph::task_frame(elapsed, index).rect;
                rects.insert(key.clone(), motion::lerp_bounds(*from, *to, t));
            }
        }
        rects
    }

    pub(super) fn advance_view_morph(&mut self, view: TreeView, reduced: bool) {
        if let Some(shown) = self.views.shown.replace(view)
            && shown != view
        {
            self.views.morph = None;
            self.views.from_view = (!reduced).then_some(shown);
        }
    }

    pub(super) fn slide_from(&self, from: TreeView) -> f32 {
        match (&self.views.morph, self.views.from_view) {
            (Some(morph), Some(view)) if view == from => {
                morph::header_t(morph.elapsed_ms(Instant::now()))
            }
            (None, Some(view)) if view == from => 0.0,
            _ => 1.0,
        }
    }

    pub(super) fn morph_body(
        &mut self,
        body: AnyElement,
        nodes: &[TreeNode],
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = Instant::now();
        let view = self.views.shown.unwrap_or_default();
        if let Some(from_view) = self.views.from_view
            && self.views.morph.is_none()
        {
            let from = self.view_rects(from_view, cx);
            self.clear_view_rects(cx);
            self.views.morph = Some(Morph {
                from,
                started: None,
            });
        }
        let targets = self.view_rects(view, cx);
        let mut elapsed = None;
        if let Some(morph) = self.views.morph.as_mut() {
            if morph.started.is_none() && !targets.is_empty() {
                morph.started = Some(now);
            }
            let ms = morph.elapsed_ms(now);
            if ms >= morph::end_ms(morph.from.len(), 0) {
                self.views.morph = None;
                self.views.from_view = None;
            } else {
                elapsed = Some(ms);
            }
        }
        let measured = self.views.body.clone();
        let probe = gpui::canvas(
            move |bounds, _, _| measured.set(Some(bounds)),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        let travels = view == TreeView::Spine || self.views.from_view == Some(TreeView::Spine);
        if elapsed.is_some() && !travels {
            window.request_animation_frame();
        }
        let elapsed = elapsed.filter(|_| travels);
        let body_t = elapsed.map_or(1.0, |ms| morph::extra_t(ms, true));
        let ghosts = elapsed.zip(self.views.body.get()).map(|(ms, frame)| {
            window.request_animation_frame();
            let morph = self.views.morph.as_ref();
            let rects = morph.map_or_else(Rects::default, |m| self.rects_at(m, &targets, ms));
            let to_spine = view == TreeView::Spine;
            let from_spine = self.views.from_view == Some(TreeView::Spine);
            let started = morph.is_some_and(|m| m.started.is_some());
            let items: Vec<AnyElement> = nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| {
                    let key = Key::Node(node.id.clone());
                    let from = morph.and_then(|m| m.from.get(&key)).copied();
                    let to = targets.get(&key).copied();
                    let rect = if started {
                        rects.get(&key).copied().or(from)?
                    } else {
                        from?
                    };
                    let step = morph::task_frame(ms, index);
                    let card = match (from_spine, to_spine) {
                        (true, true) => 1.0,
                        (true, false) => step.outgoing,
                        (false, true) => step.incoming,
                        (false, false) => 0.0,
                    };
                    let fade = match (from, to) {
                        (Some(_), Some(_)) => 1.0,
                        (Some(_), None) => step.outgoing,
                        (None, _) => step.incoming,
                    };
                    Some(Self::ghost(
                        node,
                        rect,
                        frame,
                        card,
                        fade * (1.0 - body_t),
                        theme,
                    ))
                })
                .collect();
            div()
                .absolute()
                .inset_0()
                .overflow_hidden()
                .children(items)
                .into_any_element()
        });
        let shown = body_t;
        div()
            .flex_1()
            .min_h_0()
            .relative()
            .flex()
            .flex_col()
            .child(probe)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .opacity(shown)
                    .child(body),
            )
            .children(ghosts)
            .into_any_element()
    }

    fn ghost(
        node: &TreeNode,
        rect: Bounds<Pixels>,
        frame: Bounds<Pixels>,
        card: f32,
        fade: f32,
        theme: &Theme,
    ) -> AnyElement {
        let muted = node.status.muted() || node.status == Status::Running;
        let title = match node.kind {
            NodeKind::Root => format!("Lead \u{b7} {}", node.title),
            _ => node.title.clone(),
        };
        div()
            .absolute()
            .left(rect.left() - frame.left())
            .top(rect.top() - frame.top())
            .w(rect.size.width)
            .h(rect.size.height)
            .opacity(fade)
            .rounded(px(motion::lerp(6.0, 10.0, card)))
            .border_1()
            .border_color(theme.border.opacity(card))
            .bg(motion::mix(theme.bg.opacity(0.0), theme.surface_card, card))
            .overflow_hidden()
            .flex()
            .when(card < 0.5, |el| el.items_center())
            .when(card >= 0.5, |el| el.items_start().pt(px(10.0)))
            .gap(px(7.0))
            .px(px(motion::lerp(6.0, 10.0, card)))
            .child(tree::glyph(
                node.status,
                14.0,
                node.status.tone(theme),
                true,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_rems(motion::lerp(14.0, 13.0, card)))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(if muted { theme.text_muted } else { theme.text })
                    .child(title),
            )
            .into_any_element()
    }
}
