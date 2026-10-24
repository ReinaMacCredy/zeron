//! The new-thread artwork shown again behind an open conversation, sized and
//! softened by [`ChatBackground`]. Shared by the shell and the Appearance
//! preview so both draw the same layer.
use crate::settings::{ChatBackground, ChatBackgroundPlacement};
use crate::theme::Theme;
use gpui::{
    AnyElement, Background, Bounds, ContentMask, Hsla, IntoElement, ParentElement, Pixels,
    RenderImage, Styled, Window, div, point, prelude::*, px, size,
};
use std::{cell::Cell, rc::Rc, sync::Arc};

const TAB_FADE_TOP: f32 = 6.0;
const TAB_FADE_BOTTOM: f32 = Theme::TITLEBAR_HEIGHT + 46.0;

pub(crate) type Painted = Rc<Cell<Option<(Bounds<Pixels>, Bounds<Pixels>)>>>;

#[derive(Clone)]
pub(crate) struct Backdrop {
    pub(crate) artwork: Option<Arc<RenderImage>>,
    pub(crate) settings: ChatBackground,
    pub(crate) fade: f32,
    pub(crate) scrim_color: Hsla,
    pub(crate) body: Hsla,
    pub(crate) painted: Painted,
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
    painted: Option<Painted>,
) -> AnyElement {
    let fade = fade.clamp(0.0, 1.0);
    let fade_bottom = settings.placement == ChatBackgroundPlacement::Hero;
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
        .children(artwork.map(|artwork| {
            div()
                .absolute()
                .inset_0()
                .opacity(settings.opacity.clamp(0.0, 1.0) * fade)
                .child(
                    gpui::canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _cx| {
                            crate::new_thread_background_mask::paint_chat(
                                artwork.clone(),
                                bounds,
                                fade_bottom,
                                window,
                            );
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
        }))
        .child(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, _cx| {
                    if settings.scrim {
                        paint_scrim(bounds, scrim_color, fade_bottom, fade, window);
                    }
                    if let Some(painted) = &painted {
                        painted.set(Some((bounds, window.content_mask().bounds)));
                        let strip = Bounds::from_corners(
                            point(bounds.left(), bounds.top() + px(Theme::TITLEBAR_HEIGHT)),
                            point(bounds.right(), bounds.top() + px(TAB_FADE_BOTTOM)),
                        );
                        window.paint_quad(gpui::fill(
                            strip,
                            tab_fade(
                                scrim_color.opacity(fade),
                                settings.tab_shade,
                                Theme::TITLEBAR_HEIGHT,
                                TAB_FADE_BOTTOM,
                            ),
                        ));
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
        .into_any_element()
}

pub(crate) fn surface(
    backdrop: Backdrop,
    overlay: impl Fn(Bounds<Pixels>, Bounds<Pixels>, &mut Window) + 'static,
) -> gpui::Div {
    let Backdrop {
        artwork,
        settings,
        fade,
        scrim_color,
        body,
        painted,
    } = backdrop;
    let fade_bottom = settings.placement == ChatBackgroundPlacement::Hero;
    div()
        .absolute()
        .inset_0()
        .opacity(fade.clamp(0.0, 1.0))
        .child(clipped(painted.clone(), move |own, _, window| {
            window.paint_quad(gpui::fill(own, body));
        }))
        .children(artwork.map(|artwork| {
            div()
                .absolute()
                .inset_0()
                .opacity(settings.opacity.clamp(0.0, 1.0))
                .child(clipped(painted.clone(), move |_, layer, window| {
                    crate::new_thread_background_mask::paint_chat(
                        artwork.clone(),
                        layer,
                        fade_bottom,
                        window,
                    );
                }))
        }))
        .child(clipped(painted, move |own, layer, window| {
            if settings.scrim {
                paint_scrim(layer, scrim_color, fade_bottom, 1.0, window);
            }
            overlay(own, layer, window);
        }))
}

fn clipped(
    painted: Painted,
    paint: impl Fn(Bounds<Pixels>, Bounds<Pixels>, &mut Window) + 'static,
) -> gpui::Canvas<()> {
    gpui::canvas(
        |_, _, _| {},
        move |own, _, window, _cx| {
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
    color: Hsla,
    fade_bottom: bool,
    fade: f32,
    window: &mut Window,
) {
    let stop = |alpha: f32| color.opacity(alpha * fade);
    let lower_end = if fade_bottom { 0.0 } else { 0.75 };
    let split = bounds.top() + bounds.size.height * 0.45;
    window.paint_quad(gpui::fill(
        Bounds::from_corners(bounds.origin, point(bounds.right(), split)),
        gpui::linear_gradient(
            180.0,
            gpui::linear_color_stop(stop(0.10), 0.0),
            gpui::linear_color_stop(stop(0.40), 1.0),
        ),
    ));
    window.paint_quad(gpui::fill(
        Bounds::from_corners(point(bounds.left(), split), bounds.bottom_right()),
        gpui::linear_gradient(
            180.0,
            gpui::linear_color_stop(stop(0.40), 0.0),
            gpui::linear_color_stop(stop(lower_end), 1.0),
        ),
    ));
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
