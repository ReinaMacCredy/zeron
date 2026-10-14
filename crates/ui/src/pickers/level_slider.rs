use super::*;

#[derive(Default)]
pub(crate) struct LevelSlider {
    bounds: Option<gpui::Bounds<gpui::Pixels>>,
    dragging: bool,
    snap: Option<EffortTween>,
    press: Option<EffortTween>,
}

fn fraction_of(index: usize, len: usize) -> f32 {
    if len > 1 {
        index as f32 / (len - 1) as f32
    } else {
        0.0
    }
}

fn inset(glass: bool) -> f32 {
    if glass {
        compact::THUMB_INSET
    } else {
        EFFORT_THUMB_SIZE / 2.0
    }
}

impl LevelSlider {
    fn pick<V: 'static>(
        view: &mut V,
        access: fn(&mut V) -> &mut LevelSlider,
        x: gpui::Pixels,
        len: usize,
        selected: usize,
        glass: bool,
        on_pick: &dyn Fn(&mut V, usize, &mut Context<V>),
        cx: &mut Context<V>,
    ) {
        let slider = access(view);
        let Some(index) = slider
            .bounds
            .and_then(|bounds| stop_at(bounds, x, len, inset(glass)))
        else {
            return;
        };
        if index == selected {
            return;
        }
        let (from, _) = EffortTween::glide(slider.snap, fraction_of(selected, len), cx);
        slider.snap = Some(EffortTween::new(from, fraction_of(index, len)));
        on_pick(view, index, cx);
        cx.notify();
    }

    fn set_pressed(&mut self, pressed: bool) {
        if self.dragging == pressed {
            return;
        }
        let from = self
            .press
            .map(|press| press.eval(motion::PRESS, std::time::Instant::now()).0)
            .unwrap_or(0.0);
        self.dragging = pressed;
        self.press = Some(EffortTween::new(from, if pressed { 1.0 } else { 0.0 }));
    }

    pub(crate) fn render<V: 'static>(
        &self,
        id: &'static str,
        aria_label: &'static str,
        labels: &[SharedString],
        selected: usize,
        access: fn(&mut V) -> &mut LevelSlider,
        on_pick: impl Fn(&mut V, usize, &mut Context<V>) + 'static,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> AnyElement {
        let len = labels.len();
        let selected = selected.min(len.saturating_sub(1));
        let glass =
            crate::settings::compact_effort_slider(cx) == crate::settings::EffortSliderStyle::Glass;
        let (fraction, gliding) = EffortTween::glide(self.snap, fraction_of(selected, len), cx);
        let (press, pressing) = self
            .press
            .map(|press| press.eval(motion::PRESS, std::time::Instant::now()))
            .unwrap_or((0.0, false));
        if gliding || pressing {
            window.request_animation_frame();
        }
        let track = if glass {
            compact::glass_track(
                theme,
                Some(fraction),
                len,
                press,
                false,
                compact::Shimmer::STILL,
            )
        } else {
            stepped_track(theme, len, Some((selected, fraction)), press, false, id)
        };
        let on_pick: std::rc::Rc<dyn Fn(&mut V, usize, &mut Context<V>)> =
            std::rc::Rc::new(on_pick);
        let down_pick = on_pick.clone();
        let move_pick = on_pick.clone();
        let entity = cx.entity().downgrade();
        let follow = entity.clone();
        let dragging = self.dragging;
        track
            .id(id)
            .debug_selector(move || id.to_string())
            .role(gpui::Role::Slider)
            .aria_label(aria_label)
            .aria_value(labels.get(selected).cloned().unwrap_or_default())
            .aria_numeric_value(selected as f64)
            .aria_min_numeric_value(0.0)
            .aria_max_numeric_value(len.saturating_sub(1) as f64)
            .aria_numeric_value_step(1.0)
            .cursor_pointer()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    Self::pick(
                        this,
                        access,
                        event.position.x,
                        len,
                        selected,
                        glass,
                        &*down_pick,
                        cx,
                    );
                    access(this).set_pressed(true);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    access(this).set_pressed(false);
                    cx.notify();
                }),
            )
            .child(
                gpui::canvas(
                    move |bounds, _, cx| {
                        let _ = entity.update(cx, |this, _| access(this).bounds = Some(bounds));
                    },
                    move |_, _, window, _| {
                        if !dragging {
                            return;
                        }
                        let on_move = follow.clone();
                        let move_pick = move_pick.clone();
                        window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                            if phase != gpui::DispatchPhase::Bubble {
                                return;
                            }
                            let _ = on_move.update(cx, |this, cx| {
                                if access(this).dragging {
                                    Self::pick(
                                        this,
                                        access,
                                        event.position.x,
                                        len,
                                        selected,
                                        glass,
                                        &*move_pick,
                                        cx,
                                    );
                                }
                            });
                        });
                        let on_up = follow.clone();
                        window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                            if phase != gpui::DispatchPhase::Bubble {
                                return;
                            }
                            let _ = on_up.update(cx, |this, cx| {
                                access(this).set_pressed(false);
                                cx.notify();
                            });
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }
}
