//! Paint-time source-alpha feather. Resizing changes only GPU parameters, not
//! the image identity, pixels, atlas entry, or an asynchronous raster job.
use gpui::{Bounds, ImageAlphaMask, Pixels, RenderImage, Window, point, px, size};
use std::{cell::Cell, rc::Rc, sync::Arc};

// Reveal half-strength artwork through the cutout's darkest area. The main
// masked pass preserves its contour, while this underlay softens its contrast.
pub(crate) const CUTOUT_REVEAL_OPACITY: f32 = 0.5;

pub(crate) type SurfaceBounds = Rc<Cell<Option<Bounds<Pixels>>>>;

fn mask(bounds: Bounds<Pixels>, composer: Bounds<Pixels>, cutout: bool) -> ImageAlphaMask {
    let height = f32::from(bounds.size.height);
    // Keep the cleared area open through the hero's bottom. A taller image
    // must not fade back in beneath the composer's rounded lower edge.
    let cleared = Bounds::new(
        composer.origin,
        size(
            composer.size.width,
            composer.bottom().max(bounds.bottom()) - composer.top(),
        ),
    );
    ImageAlphaMask {
        // The reveal pass has only the shared bottom fade. Its exclusion sits
        // below the image, so it fills the cutout without changing its shape.
        bounds: if cutout {
            cleared
        } else {
            Bounds::new(point(bounds.left(), bounds.bottom() + px(1.0)), bounds.size)
        },
        radius: if cutout {
            px(crate::composer::COMPOSER_RADIUS)
        } else {
            px(0.0)
        },
        feather: if cutout {
            px((height * 0.52).clamp(120.0, 280.0))
        } else {
            px(1.0)
        },
        clearance: if cutout { px(8.0) } else { px(0.0) },
        // Start fading at the image's top, rather than holding full opacity
        // through its first 40% and compressing the transition near the bottom.
        // Both passes use the full height, independently of the softer cutout.
        bottom_fade: Some((bounds.bottom(), px(height.max(1.0)))),
    }
}

/// All elements have finished prepaint before this reads the measured surface,
/// so the first visible frame uses the current composer, including on sidebar
/// resize and right-panel handoffs. Object-fit cropping is independent.
pub(crate) fn paint(
    source: Arc<RenderImage>,
    bounds: Bounds<Pixels>,
    composer: Bounds<Pixels>,
    cutout: bool,
    window: &mut Window,
) {
    let Some(fitted) = cover_fit(&source, bounds) else {
        return;
    };
    let _ = window.paint_image_fitted_masked(
        bounds,
        fitted,
        Default::default(),
        source,
        0,
        false,
        Some(mask(bounds, composer, cutout)),
    );
}

/// The chat variant: no composer cutout and no reveal pass. `fade_bottom`
/// keeps the hero placement's full-height dissolve; the full placement paints
/// the plain cover-fitted image.
pub(crate) fn paint_chat(
    source: Arc<RenderImage>,
    bounds: Bounds<Pixels>,
    fade_bottom: bool,
    window: &mut Window,
) {
    let Some(fitted) = cover_fit(&source, bounds) else {
        return;
    };
    let _ = window.paint_image_fitted_masked(
        bounds,
        fitted,
        Default::default(),
        source,
        0,
        false,
        fade_bottom.then(|| bottom_fade_mask(bounds)),
    );
}

fn bottom_fade_mask(bounds: Bounds<Pixels>) -> ImageAlphaMask {
    let height = f32::from(bounds.size.height);
    ImageAlphaMask {
        bounds: Bounds::new(point(bounds.left(), bounds.bottom() + px(1.0)), bounds.size),
        radius: px(0.0),
        feather: px(1.0),
        clearance: px(0.0),
        bottom_fade: Some((bounds.bottom(), px(height.max(1.0)))),
    }
}

fn cover_fit(source: &RenderImage, bounds: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let source_size = source.size(0);
    if width <= 0.0 || height <= 0.0 || source_size.width.0 <= 0 || source_size.height.0 <= 0 {
        return None;
    }
    let scale = (width / source_size.width.0 as f32).max(height / source_size.height.0 as f32);
    let fitted_size = size(
        px(source_size.width.0 as f32 * scale),
        px(source_size.height.0 as f32 * scale),
    );
    Some(Bounds::new(
        bounds.center() - point(fitted_size.width * 0.5, fitted_size.height * 0.5),
        fitted_size,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, canvas, div, prelude::*};

    #[gpui::test]
    fn background_paint_sees_same_frame_composer_bounds_even_when_painted_first(
        cx: &mut gpui::TestAppContext,
    ) {
        struct Fixture {
            surface: SurfaceBounds,
            painted: SurfaceBounds,
            source: Arc<RenderImage>,
            left: f32,
            width: f32,
        }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let surface = self.surface.clone();
                let painted = self.painted.clone();
                let measured = self.surface.clone();
                let source = self.source.clone();
                div()
                    .relative()
                    .size_full()
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                painted.set(surface.get());
                                if let Some(composer) = surface.get() {
                                    paint(source, bounds, composer, true, window);
                                }
                            },
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(self.left))
                            .top(px(360.25))
                            .w(px(self.width))
                            .h(px(124.0))
                            .child(
                                canvas(
                                    move |bounds, _, _| measured.set(Some(bounds)),
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0(),
                            ),
                    )
            }
        }
        let surface: SurfaceBounds = Default::default();
        let painted: SurfaceBounds = Default::default();
        let handle = cx.add_window(|_, _| Fixture {
            surface: surface.clone(),
            painted: painted.clone(),
            source: Arc::new(RenderImage::new([image::Frame::new(
                image::RgbaImage::from_pixel(2, 2, image::Rgba([79, 151, 233, 180])),
            )])),
            left: 40.0,
            width: 768.0,
        });
        for (left, width) in [
            (40.0, 768.0),
            (264.0, 544.0),
            (152.25, 656.0),
            (40.0, 408.0),
            (40.0, 768.0),
        ] {
            handle
                .update(cx, |fixture, _, cx| {
                    fixture.left = left;
                    fixture.width = width;
                    cx.notify();
                })
                .unwrap();
            cx.update_window(handle.into(), |_, window, cx| {
                window.draw(cx).clear();
            })
            .unwrap();
            let actual = painted.get().expect("no cold first-frame geometry");
            assert_eq!(Some(actual), surface.get());
            // GPUI layout snaps to physical pixels; the mask must consume
            // that exact measured surface, not the unrounded layout request.
            assert!((f32::from(actual.left()) - left).abs() <= 0.5);
            assert_eq!(actual.size.width, px(width));
        }
    }

    #[test]
    fn mask_tracks_current_surface_in_window_space_without_rounding() {
        for sidebar in [0.0, 112.25, 224.0] {
            for right_panel in [0.0, 360.0] {
                let hero = Bounds::new(
                    point(px(sidebar), px(40.0)),
                    size(px(1200.0 - sidebar), px(440.0)),
                );
                let composer = Bounds::new(
                    point(px(sidebar + 40.5), px(360.25)),
                    size(px(900.0 - sidebar - right_panel), px(124.0)),
                );
                let mask = mask(hero, composer, true);
                assert_eq!(mask.bounds, composer);
                assert_eq!(mask.bottom_fade, Some((px(480.0), px(440.0))));
                assert_eq!(mask.feather, px(440.0 * 0.52));
                assert_eq!(mask.clearance, px(8.0));
            }
        }
    }

    #[test]
    fn taller_background_stays_cleared_below_the_composer() {
        let hero = Bounds::new(point(px(0.0), px(0.0)), size(px(1440.0), px(691.2)));
        let composer = Bounds::new(point(px(352.0), px(406.0)), size(px(736.0), px(124.0)));
        let mask = mask(hero, composer, true);
        assert_eq!(mask.bounds.origin, composer.origin);
        assert_eq!(mask.bounds.size.width, composer.size.width);
        assert_eq!(mask.bounds.bottom(), hero.bottom());
        assert_eq!(mask.feather, px(280.0));
    }
    #[test]
    fn new_thread_cutout_reveal_preserves_the_bottom_fade_and_image_extent() {
        let hero = Bounds::new(point(px(0.0), px(0.0)), size(px(1440.0), px(691.2)));
        let composer = Bounds::new(point(px(352.0), px(406.0)), size(px(736.0), px(124.0)));
        let cutout = mask(hero, composer, true);
        let reveal = mask(hero, composer, false);
        assert_eq!(cutout.bottom_fade, reveal.bottom_fade);
        assert!(reveal.bounds.top() - reveal.feather >= hero.bottom());
        assert_eq!(reveal.radius, px(0.0));
        assert_eq!(reveal.clearance, px(0.0));
        assert_eq!(CUTOUT_REVEAL_OPACITY, 0.5);
    }

    #[test]
    fn chat_bottom_fade_matches_the_hero_reveal_fade_without_any_cutout() {
        let hero = Bounds::new(point(px(0.0), px(0.0)), size(px(1440.0), px(691.2)));
        let composer = Bounds::new(point(px(352.0), px(406.0)), size(px(736.0), px(124.0)));
        let chat = bottom_fade_mask(hero);
        let reveal = mask(hero, composer, false);
        assert_eq!(chat.bottom_fade, reveal.bottom_fade);
        assert_eq!(chat.bounds, reveal.bounds);
        assert_eq!(chat.radius, px(0.0));
        assert_eq!(chat.clearance, px(0.0));
        let source = RenderImage::new([image::Frame::new(image::RgbaImage::new(200, 100))]);
        let fitted = cover_fit(&source, hero).unwrap();
        assert_eq!(fitted.size.width, px(1440.0));
        assert_eq!(fitted.size.height, px(720.0));
        assert!(cover_fit(&source, Bounds::new(hero.origin, size(px(0.0), px(10.0)))).is_none());
    }

    #[test]
    fn new_thread_main_fade_uses_the_full_height_at_every_window_size() {
        for height in [288.0, 489.6, 691.2, 760.0] {
            let hero = Bounds::new(point(px(224.25), px(40.5)), size(px(1000.0), px(height)));
            let composer = Bounds::new(point(px(352.0), px(406.0)), size(px(736.0), px(124.0)));
            for cutout in [false, true] {
                let (end, feather) = mask(hero, composer, cutout).bottom_fade.unwrap();
                assert!((f32::from(end - feather - hero.top())).abs() < 0.0001);
                assert_eq!(end, hero.bottom());
            }
        }
    }
}
