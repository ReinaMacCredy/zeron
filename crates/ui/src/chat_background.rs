//! The new-thread artwork shown again behind an open conversation, sized and
//! softened by [`ChatBackground`]. Shared by the shell and the Appearance
//! preview so both draw the same layer.
use crate::settings::{ChatBackground, ChatBackgroundPlacement};
use crate::theme::Theme;
use gpui::{
    AnyElement, Background, Bounds, ContentMask, Corners, Hsla, IntoElement, ParentElement, Pixels,
    Point, RenderImage, Styled, Window, div, point, prelude::*, px, size,
};
use std::{cell::Cell, rc::Rc, sync::Arc};

const TAB_FADE_TOP: f32 = 6.0;
const TAB_FADE_BOTTOM: f32 = Theme::TITLEBAR_HEIGHT + 46.0;

pub(crate) type Painted = Rc<Cell<Option<(Bounds<Pixels>, Bounds<Pixels>)>>>;

pub(crate) type Art = Rc<dyn Fn(Option<&Painted>) -> AnyElement>;

/// Content masks are rectangles, so art reaching the inset card's corners rounds itself.
#[derive(Clone, Default)]
pub(crate) struct Card {
    pub(crate) anchored: bool,
    pub(crate) bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl Card {
    pub(crate) fn place(
        &self,
        layer: Bounds<Pixels>,
        window: &Window,
    ) -> (Bounds<Pixels>, Bounds<Pixels>, Corners<Pixels>) {
        let layer = if self.anchored {
            Bounds::from_corners(point(px(0.0), layer.top()), layer.bottom_right())
        } else {
            layer
        };
        let visible = layer.intersect(&window.content_mask().bounds);
        let Some(card) = self.bounds.get() else {
            return (layer, visible, Corners::default());
        };
        let radius = |a: Point<Pixels>, b: Point<Pixels>| {
            if (a.x - b.x).abs() < px(0.5) && (a.y - b.y).abs() < px(0.5) {
                px(crate::shell::CARD_RADIUS)
            } else {
                px(0.0)
            }
        };
        let corners = Corners {
            top_left: radius(visible.origin, card.origin),
            top_right: radius(visible.top_right(), card.top_right()),
            bottom_left: radius(visible.bottom_left(), card.bottom_left()),
            bottom_right: radius(visible.bottom_right(), card.bottom_right()),
        };
        (layer, visible, corners)
    }
}

#[derive(Clone)]
pub(crate) struct Backdrop {
    pub(crate) art: Art,
    pub(crate) tab_shade: f32,
    pub(crate) fade: f32,
    pub(crate) scrim_color: Hsla,
    pub(crate) body: Hsla,
    pub(crate) painted: Painted,
}

impl Backdrop {
    pub(crate) fn chat(
        artwork: Option<Arc<RenderImage>>,
        settings: ChatBackground,
        fade: f32,
        scrim_color: Hsla,
        body: Hsla,
        card: Card,
    ) -> Self {
        Self {
            art: art(artwork, settings, scrim_color, card),
            tab_shade: settings.tab_shade,
            fade,
            scrim_color,
            body,
            painted: Default::default(),
        }
    }

    pub(crate) fn strip(&self) -> gpui::Canvas<()> {
        let painted = self.painted.clone();
        let (color, tab_shade) = (self.scrim_color.opacity(self.fade), self.tab_shade);
        gpui::canvas(
            // Set in prepaint so the frame, painted under the card, reads it.
            move |bounds, window, _| painted.set(Some((bounds, window.content_mask().bounds))),
            move |bounds, _, window, _cx| {
                let strip = Bounds::from_corners(
                    point(bounds.left(), bounds.top() + px(Theme::TITLEBAR_HEIGHT)),
                    point(bounds.right(), bounds.top() + px(TAB_FADE_BOTTOM)),
                );
                window.paint_quad(gpui::fill(
                    strip,
                    tab_fade(color, tab_shade, Theme::TITLEBAR_HEIGHT, TAB_FADE_BOTTOM),
                ));
            },
        )
        .absolute()
        .inset_0()
    }

    pub(crate) fn frame(&self, shade: f32, blur: f32, content_left: f32, inset: f32) -> gpui::Div {
        let frame_painted = Painted::default();
        let (painted, target) = (self.painted.clone(), frame_painted.clone());
        let color = self.scrim_color;
        div()
            .absolute()
            .inset_0()
            .opacity(self.fade.clamp(0.0, 1.0))
            .child(
                gpui::canvas(
                    |_, _, _| {},
                    move |own, _, _, _| {
                        target.set(painted.get().map(|(layer, _)| (layer, own)));
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child((self.art)(Some(&frame_painted)))
            .child(
                gpui::canvas(
                    |_, _, _| {},
                    move |own, _, window, _| {
                        if blur > 0.0 {
                            window.paint_layer(own, |window| {
                                for piece in frame_pieces(own, content_left, inset) {
                                    window.paint_backdrop_blur(piece, Default::default(), px(blur));
                                }
                            });
                        }
                        window.paint_quad(gpui::fill(own, color.opacity(shade)));
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}

/// Strips reach under the card by its radius so its rounded corners show frame, not raw art.
fn frame_pieces(own: Bounds<Pixels>, content_left: f32, inset: f32) -> [Bounds<Pixels>; 4] {
    let reach = px(crate::shell::CARD_RADIUS);
    let card_top = own.top() + px(Theme::TITLEBAR_HEIGHT);
    let card_left = own.left() + px(content_left);
    let card_right = own.right() - px(inset);
    let card_bottom = own.bottom() - px(inset);
    [
        Bounds::from_corners(own.origin, point(own.right(), card_top + reach)),
        Bounds::from_corners(
            point(own.left(), card_top),
            point(card_left + reach, own.bottom()),
        ),
        Bounds::from_corners(point(card_right - reach, card_top), own.bottom_right()),
        Bounds::from_corners(point(own.left(), card_bottom - reach), own.bottom_right()),
    ]
}

pub(crate) fn layer_height(
    placement: ChatBackgroundPlacement,
    viewport_height: f32,
    hero_height: f32,
) -> f32 {
    match placement {
        ChatBackgroundPlacement::Hero => hero_height.min(viewport_height.max(0.0)),
        ChatBackgroundPlacement::Full => viewport_height.max(0.0),
    }
}

/// `fade` is the route and readiness progress shared by artwork and scrim;
/// the user's opacity scales only the artwork so the scrim keeps text legible
/// at every slider position.
/// `width` is the shell's full conversation canvas; `None` fills the parent,
/// which is what a miniature preview wants.
pub(crate) fn layer(
    artwork: Option<Arc<RenderImage>>,
    settings: ChatBackground,
    width: Option<f32>,
    height: f32,
    scrim_color: Hsla,
    fade: f32,
    backdrop: Option<&Backdrop>,
    card: Card,
) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .map(|layer| match width {
            Some(width) => layer.w(px(width)),
            None => layer.w_full(),
        })
        .h(px(height))
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .inset_0()
                .opacity(fade.clamp(0.0, 1.0))
                .child(art(artwork, settings, scrim_color, card)(None)),
        )
        .when_some(backdrop, |layer, backdrop| layer.child(backdrop.strip()))
        .into_any_element()
}

fn art(
    artwork: Option<Arc<RenderImage>>,
    settings: ChatBackground,
    scrim_color: Hsla,
    card: Card,
) -> Art {
    let fade_bottom = settings.placement == ChatBackgroundPlacement::Hero;
    Rc::new(move |painted| {
        let card = card.clone();
        div()
            .absolute()
            .inset_0()
            .children(artwork.clone().map(|artwork| {
                let card = card.clone();
                div()
                    .absolute()
                    .inset_0()
                    .opacity(settings.opacity.clamp(0.0, 1.0))
                    .child(clipped(painted, move |_, layer, window| {
                        let (layer, visible, corners) = card.place(layer, window);
                        crate::new_thread_background_mask::paint_chat(
                            artwork.clone(),
                            layer,
                            visible,
                            corners,
                            fade_bottom,
                            window,
                        );
                    }))
            }))
            .when(settings.scrim, |el| {
                el.child(clipped(painted, move |_, layer, window| {
                    let (layer, visible, corners) = card.place(layer, window);
                    paint_scrim(layer, visible, corners, scrim_color, fade_bottom, window);
                }))
            })
            .into_any_element()
    })
}

pub(crate) fn surface(
    backdrop: Backdrop,
    overlay: impl Fn(Bounds<Pixels>, Bounds<Pixels>, &mut Window) + 'static,
) -> gpui::Div {
    let Backdrop {
        art,
        fade,
        body,
        painted,
        ..
    } = backdrop;
    div()
        .absolute()
        .inset_0()
        .opacity(fade.clamp(0.0, 1.0))
        .child(clipped(Some(&painted), move |own, _, window| {
            window.paint_quad(gpui::fill(own, body));
        }))
        .child(art(Some(&painted)))
        .child(clipped(Some(&painted), overlay))
}

pub(crate) fn clipped(
    painted: Option<&Painted>,
    paint: impl Fn(Bounds<Pixels>, Bounds<Pixels>, &mut Window) + 'static,
) -> gpui::Canvas<()> {
    let painted = painted.cloned();
    gpui::canvas(
        |_, _, _| {},
        move |own, _, window, _cx| {
            let Some(painted) = &painted else {
                return paint(own, own, window);
            };
            let Some((layer, mask)) = painted.get() else {
                return;
            };
            let clip = own.intersect(&Bounds::new(
                point(mask.left(), own.top()),
                size(mask.size.width, own.size.height),
            ));
            if clip.size.width > px(0.0) && clip.size.height > px(0.0) {
                window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                    paint(own, layer, window)
                });
            }
        },
    )
    .absolute()
    .inset_0()
}

pub(crate) fn tab_fade(color: Hsla, tab_shade: f32, top: f32, bottom: f32) -> Background {
    let alpha = |y: f32| {
        tab_shade
            * 0.55
            * ((TAB_FADE_BOTTOM - y) / (TAB_FADE_BOTTOM - TAB_FADE_TOP)).clamp(0.0, 1.0)
    };
    gpui::linear_gradient(
        180.0,
        gpui::linear_color_stop(color.opacity(alpha(top)), 0.0),
        gpui::linear_color_stop(color.opacity(alpha(bottom)), 1.0),
    )
}

/// Two stacked gradients stand in for a three-stop scrim. The hero placement
/// releases toward the bottom where the artwork has already dissolved, so
/// translucent themes show no edge at the layer's end.
fn paint_scrim(
    bounds: Bounds<Pixels>,
    visible: Bounds<Pixels>,
    corners: Corners<Pixels>,
    color: Hsla,
    fade_bottom: bool,
    window: &mut Window,
) {
    let lower_end = if fade_bottom { 0.0 } else { 0.75 };
    let split = bounds.top() + bounds.size.height * 0.45;
    let mut band = |top: Pixels, bottom: Pixels, from: f32, to: f32, corners: Corners<Pixels>| {
        let shown = Bounds::from_corners(
            point(visible.left(), top.max(visible.top())),
            point(visible.right(), bottom.min(visible.bottom())),
        );
        if shown.size.height <= px(0.0) || bottom <= top {
            return;
        }
        let alpha = |y: Pixels| from + (to - from) * f32::from(y - top) / f32::from(bottom - top);
        window.paint_quad(
            gpui::fill(
                shown,
                gpui::linear_gradient(
                    180.0,
                    gpui::linear_color_stop(color.opacity(alpha(shown.top())), 0.0),
                    gpui::linear_color_stop(color.opacity(alpha(shown.bottom())), 1.0),
                ),
            )
            .corner_radii(corners),
        );
    };
    band(
        bounds.top(),
        split,
        0.10,
        0.40,
        Corners {
            top_left: corners.top_left,
            top_right: corners.top_right,
            ..Default::default()
        },
    );
    band(
        split,
        bounds.bottom(),
        0.40,
        lower_end,
        Corners {
            bottom_left: corners.bottom_left,
            bottom_right: corners.bottom_right,
            ..Default::default()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hero_placement_reuses_the_hero_height_and_full_covers_the_column() {
        assert_eq!(
            layer_height(ChatBackgroundPlacement::Hero, 1000.0, 720.0),
            720.0
        );
        assert_eq!(
            layer_height(ChatBackgroundPlacement::Hero, 200.0, 720.0),
            200.0
        );
        assert_eq!(
            layer_height(ChatBackgroundPlacement::Full, 1000.0, 720.0),
            1000.0
        );
        assert_eq!(
            layer_height(ChatBackgroundPlacement::Full, -5.0, 720.0),
            0.0
        );
    }
}
