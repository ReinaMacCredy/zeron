use gpui::{AnyElement, SharedString, div, prelude::*, px};
use zeron_proto::{ChatWorkTree, WorkTreeState};

use crate::change_requests::{ChangeRequestBadgeSurface, ChangeRequestBadgeTone};
use crate::theme::Theme;

pub(crate) fn tooltip(state: WorkTreeState, dirty_files: u32, base: &str) -> Option<String> {
    match state {
        WorkTreeState::Ahead { commits: 1 } => Some(format!("1 commit not in {base}")),
        WorkTreeState::Ahead { commits } => Some(format!("{commits} commits not in {base}")),
        WorkTreeState::Merged if dirty_files == 1 => Some(format!("In {base}, 1 uncommitted file")),
        WorkTreeState::Merged if dirty_files > 0 => {
            Some(format!("In {base}, {dirty_files} uncommitted files"))
        }
        WorkTreeState::Merged => Some(format!("In {base}")),
        WorkTreeState::Fresh | WorkTreeState::Detached => None,
    }
}

pub(crate) fn chat_badge(
    id: impl Into<SharedString>,
    work_tree: Option<&ChatWorkTree>,
    base: Option<&str>,
    surface: ChangeRequestBadgeSurface,
    theme: &Theme,
) -> Option<AnyElement> {
    let work_tree = work_tree?;
    badge(
        id,
        work_tree.state,
        work_tree.dirty_files,
        base,
        surface,
        theme,
    )
}

pub(crate) fn badge(
    id: impl Into<SharedString>,
    state: WorkTreeState,
    dirty_files: u32,
    base: Option<&str>,
    surface: ChangeRequestBadgeSurface,
    theme: &Theme,
) -> Option<AnyElement> {
    let tip = tooltip(state, dirty_files, base.unwrap_or("the base branch"))?;
    let composer = surface == ChangeRequestBadgeSurface::Composer;
    let (color, glyph, count) = match state {
        WorkTreeState::Ahead { commits } => (
            theme.text_muted,
            crate::icons::ARROW_UP,
            Some(commits.to_string()),
        ),
        _ => (
            ChangeRequestBadgeTone::Merged.color(theme),
            crate::icons::CHECK,
            None,
        ),
    };
    let dirty = matches!(state, WorkTreeState::Merged) && dirty_files > 0;
    Some(
        div()
            .id(id.into())
            .h(px(if composer { 20.0 } else { 16.0 }))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(3.0))
            .px(px(if composer { 6.0 } else { 4.0 }))
            .rounded(px(if composer { 6.0 } else { 4.0 }))
            .bg(color.opacity(0.08))
            .text_size(px(if composer { 11.0 } else { 10.0 }))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(color.opacity(0.85))
            .tooltip(crate::settings::widgets::text_tooltip(tip))
            .child(
                crate::icons::icon(glyph)
                    .size(px(if composer { 11.0 } else { 10.0 }))
                    .flex_none()
                    .text_color(color.opacity(0.85)),
            )
            .children(count.map(|count| div().font_family(theme.font_mono.clone()).child(count)))
            .when(dirty, |el| {
                el.child(
                    div()
                        .size(px(5.0))
                        .flex_none()
                        .rounded_full()
                        .bg(theme.warning),
                )
            })
            .into_any_element(),
    )
}
