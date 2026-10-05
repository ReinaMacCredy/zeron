use gpui::{AnyElement, App, EntityId, SharedString, div, prelude::*, px};

use crate::icons;
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadingStage {
    Opening,
    Stream,
}

impl LoadingStage {
    pub fn label(self) -> &'static str {
        match self {
            LoadingStage::Opening => "Step 1 of 2: open device",
            LoadingStage::Stream => "Step 2 of 2: connect video",
        }
    }
}

pub struct LoadingView {
    pub name: SharedString,
    pub description: SharedString,
    pub message: SharedString,
    pub stage: LoadingStage,
    pub error: bool,
}

pub fn render(
    view: LoadingView,
    theme: &Theme,
    owner: EntityId,
    action: Option<AnyElement>,
    cx: &mut App,
) -> AnyElement {
    let segment = |filled: bool| {
        div().h(px(4.0)).flex_1().rounded_full().bg(if filled {
            theme.text.opacity(0.6)
        } else {
            crate::theme::ink(0.08)
        })
    };
    let small = crate::typography::ui_rems(12.0);
    div()
        .id("device-loading")
        .role(if view.error {
            gpui::Role::Alert
        } else {
            gpui::Role::Status
        })
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.bg)
        .px(px(24.0))
        .py(px(40.0))
        .child(
            div()
                .max_w(px(384.0))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.0))
                .child(
                    div()
                        .size(px(48.0))
                        .rounded(px(12.0))
                        .border_1()
                        .border_color(theme.border)
                        .bg(crate::theme::ink(0.03))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            icons::icon(icons::SMARTPHONE)
                                .size(px(24.0))
                                .text_color(theme.text_muted),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(13.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(view.name),
                        )
                        .when(!view.description.is_empty(), |el| {
                            el.child(
                                div()
                                    .text_size(small)
                                    .text_color(theme.text_muted)
                                    .child(view.description),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(small)
                        .text_color(if view.error {
                            theme.danger
                        } else {
                            theme.text_muted
                        })
                        .when(!view.error, |el| {
                            el.child(crate::loaders::mini_mono_spinner(
                                "device-loading-spinner",
                                2.0,
                                theme.text_muted,
                                owner,
                                cx,
                            ))
                        })
                        .child(view.message),
                )
                .children(action)
                .when(!view.error, |el| {
                    el.child(
                        div()
                            .id("device-loading-progress")
                            .debug_selector(move || view.stage.label().into())
                            .role(gpui::Role::ProgressIndicator)
                            .aria_label(view.stage.label())
                            .w(px(96.0))
                            .flex()
                            .gap(px(4.0))
                            .child(segment(true))
                            .child(segment(view.stage == LoadingStage::Stream)),
                    )
                }),
        )
        .into_any_element()
}
