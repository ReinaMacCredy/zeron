use std::{cell::Cell, rc::Rc};

use gpui::{
    AnimationExt as _, AnyElement, App, Bounds, EntityId, Hsla, InteractiveElement, IntoElement,
    MouseDownEvent, ParentElement, Pixels, SharedString, StatefulInteractiveElement, Styled,
    Window, div, prelude::*, px,
};

use crate::theme::Theme;

pub(crate) const SIZE: f32 = 38.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Marks {
    pub working: bool,
    pub count: usize,
    pub dot: bool,
}

#[derive(Clone, Default)]
pub(crate) struct Float {
    orb: Rc<Cell<Option<Bounds<Pixels>>>>,
    height: Rc<Cell<f32>>,
}

impl Float {
    pub(crate) fn height(&self) -> f32 {
        self.height.get()
    }

    fn on_orb(&self, position: gpui::Point<Pixels>) -> bool {
        self.orb
            .get()
            .is_some_and(|bounds| bounds.contains(&position))
    }
}

pub(crate) struct DockOrb {
    pub id: &'static str,
    pub glyph: &'static str,
    pub tint: Hsla,
    pub label: SharedString,
    pub tooltip: &'static str,
    pub marks: Marks,
    pub open: bool,
}

impl DockOrb {
    pub(crate) fn render(
        self,
        theme: &Theme,
        float: &Float,
        on_open: impl Fn(&mut Window, &mut App) + 'static,
        on_fold: impl Fn(&mut Window, &mut App) + 'static,
        cx: &App,
    ) -> AnyElement {
        let id = self.id;
        let open = self.open;
        let button = div()
            .id(id)
            .debug_selector(move || id.into())
            .role(gpui::Role::Button)
            .aria_label(self.label)
            .tab_index(0)
            .size(px(SIZE))
            .rounded_full()
            .border_1()
            .border_color(theme.composer_surface_border())
            .bg(theme.composer_surface_bg())
            .when(!theme.is_frost(), |el| el.shadow_lg())
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .focus_visible(|style| style.border_2().border_color(theme.accent))
            .tooltip(crate::settings::widgets::text_tooltip(self.tooltip))
            .on_click(move |_, window, cx| {
                if open {
                    on_fold(window, cx)
                } else {
                    on_open(window, cx)
                }
            })
            .child(
                crate::icons::icon(self.glyph)
                    .size(px(16.0))
                    .text_color(self.tint),
            );
        let accent = theme.accent;
        let ring = self.marks.working.then(|| {
            let diameter = SIZE + 6.0;
            let ring = div()
                .debug_selector(move || format!("{id}-working"))
                .absolute()
                .top(px(-3.0))
                .left(px(-3.0))
                .size(px(diameter));
            if crate::motion::reduced_motion(cx) {
                ring.child(crate::loaders::turning_arc(diameter, 0.0, 0.3, accent))
                    .into_any_element()
            } else {
                ring.with_animation(
                    SharedString::from(format!("{id}-ring")),
                    gpui::Animation::new(std::time::Duration::from_millis(1000)).repeat(),
                    move |ring, turn| {
                        ring.child(crate::loaders::turning_arc(diameter, turn, 0.3, accent))
                    },
                )
                .into_any_element()
            }
        });
        let count = (self.marks.count > 0).then(|| {
            div()
                .debug_selector(move || format!("{id}-count"))
                .absolute()
                .top(px(-3.0))
                .right(px(-3.0))
                .min_w(px(16.0))
                .h(px(16.0))
                .px(px(4.0))
                .rounded_full()
                .bg(accent)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(gpui::white())
                .child(self.marks.count.to_string())
        });
        let dot = self.marks.dot.then(|| {
            div()
                .debug_selector(move || format!("{id}-dot"))
                .absolute()
                .bottom(px(1.0))
                .right(px(1.0))
                .size(px(8.0))
                .rounded_full()
                .bg(accent)
        });
        div()
            .relative()
            .flex_none()
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(crate::frost::frosted(
                SIZE / 2.0,
                crate::frost::MENU_BLUR,
                button,
            ))
            .children(ring)
            .children(count)
            .children(dot)
            .child({
                let orb = float.orb.clone();
                gpui::canvas(move |bounds, _, _| orb.set(Some(bounds)), |_, _, _, _| {})
                    .absolute()
                    .inset_0()
            })
            .into_any_element()
    }
}

pub(crate) fn float(
    id: &'static str,
    float: &Float,
    owner: EntityId,
    on_outside: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    composer: impl IntoElement,
) -> gpui::Div {
    let orb = float.clone();
    let height = float.height.clone();
    div()
        .relative()
        .occlude()
        .on_mouse_down_out(move |event, window, cx| {
            if !orb.on_orb(event.position) {
                on_outside(event, window, cx)
            }
        })
        .child(crate::motion::menu_in_from(id, 8.0, div().child(composer)))
        .child(
            gpui::canvas(
                move |bounds, window, _| {
                    let measured = f32::from(bounds.size.height);
                    if (height.get() - measured).abs() > 0.5 {
                        height.set(measured);
                        window.on_next_frame(move |_, cx| cx.notify(owner));
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
}
