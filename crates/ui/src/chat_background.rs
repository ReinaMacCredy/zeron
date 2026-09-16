//! The new-thread artwork shown again behind an open conversation, sized and
//! softened by [`ChatBackground`]. Shared by the shell and the Appearance
//! preview so both draw the same layer.
use crate::settings::{ChatBackground, ChatBackgroundPlacement};
use gpui::{
    AnyElement, Hsla, IntoElement, ParentElement, RenderImage, Styled, div, prelude::*, px,
};
use std::sync::Arc;

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
pub(crate) fn layer(
    artwork: Option<Arc<RenderImage>>,
    settings: ChatBackground,
    width: f32,
    height: f32,
    scrim_color: Hsla,
    fade: f32,
) -> AnyElement {
    let fade = fade.clamp(0.0, 1.0);
    let fade_bottom = settings.placement == ChatBackgroundPlacement::Hero;
    div()
        .absolute()
        .top_0()
        .left_0()
        .w(px(width))
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
        .when(settings.scrim, |layer| {
            layer.child(scrim(scrim_color, fade_bottom, fade))
        })
        .into_any_element()
}

/// Two stacked gradients stand in for a three-stop scrim. The hero placement
/// releases toward the bottom where the artwork has already dissolved, so
/// translucent themes show no edge at the layer's end.
fn scrim(color: Hsla, fade_bottom: bool, fade: f32) -> gpui::Div {
    let stop = |alpha: f32| color.opacity(alpha);
    let lower_end = if fade_bottom { 0.0 } else { 0.75 };
    div()
        .absolute()
        .inset_0()
        .opacity(fade)
        .flex()
        .flex_col()
        .child(
            div()
                .h(gpui::relative(0.45))
                .w_full()
                .bg(gpui::linear_gradient(
                    180.0,
                    gpui::linear_color_stop(stop(0.10), 0.0),
                    gpui::linear_color_stop(stop(0.40), 1.0),
                )),
        )
        .child(div().flex_1().w_full().bg(gpui::linear_gradient(
            180.0,
            gpui::linear_color_stop(stop(0.40), 0.0),
            gpui::linear_color_stop(stop(lower_end), 1.0),
        )))
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
