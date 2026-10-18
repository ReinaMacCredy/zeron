use super::*;
use crate::motion::{CubicBezier, MotionSpec};
use crate::whats_new::{self, CURRENT_VERSION, Entry, Feature, Resolution};
use gpui::{
    Animation, AnimationExt as _, Div, ObjectFit, img, linear_color_stop, linear_gradient, relative,
};

const CARD_RADIUS: f32 = 12.0;
const HERO_HEIGHT: f32 = 96.0;
const CARD_DELAY_MS: u64 = 500;
const CARD_SPRING_MS: u64 = 420;
const CARD_RISE: f32 = 14.0;
const SPRING: CubicBezier = CubicBezier::new(0.34, 1.56, 0.64, 1.0);
const SHINE: MotionSpec =
    MotionSpec::new(900, motion::EASE_IN_OUT).with_delay(CARD_DELAY_MS + CARD_SPRING_MS);
const FEATURE_IN: MotionSpec = MotionSpec::new(360, motion::EASE_OUT_QUINT);
const FEATURE_STAGGER_MS: u64 = 80;
const DIALOG_WIDTH: f32 = 480.0;
const DIALOG_BODY_MAX: f32 = 520.0;
const HELP_RELEASES: usize = 3;
const HELP_MENU_WIDTH: f32 = 264.0;

pub(super) struct WhatsNewCard {
    entry: &'static Entry,
    gradient_hero: bool,
}

pub(super) struct WhatsNewDialog {
    view: WhatsNewView,
    entry: &'static Entry,
    expanded: std::collections::HashSet<&'static str>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WhatsNewView {
    Highlights,
    Changelog,
}

fn anchor_entry() -> Option<&'static Entry> {
    let entries = whats_new::entries();
    whats_new::entry_for(entries, CURRENT_VERSION).or(entries.first())
}

pub(super) fn boot_card(settings: &mut UiSettings, cx: &mut App) -> Option<WhatsNewCard> {
    let entries = whats_new::entries();
    let seen = settings.whats_new_seen_version.as_deref();
    let shown = match whats_new::resolve(entries, CURRENT_VERSION, seen) {
        Resolution::Show(entry) => Some(entry),
        Resolution::Advance => {
            settings.whats_new_seen_version = Some(CURRENT_VERSION.into());
            settings::update(SavePolicy::Debounced, cx, |s| {
                s.whats_new_seen_version = Some(CURRENT_VERSION.into());
            });
            None
        }
        Resolution::Noop => None,
    };
    let knob = std::env::var("ZERON_OPEN_DIALOG").ok();
    let gradient_hero = knob.as_deref() == Some("whats-new-card-gradient");
    let forced = gradient_hero || knob.as_deref() == Some("whats-new-card");
    let entry = shown.or_else(|| forced.then(anchor_entry).flatten())?;
    Some(WhatsNewCard {
        entry,
        gradient_hero,
    })
}

fn card_entrance() -> Animation {
    let total = CARD_DELAY_MS + CARD_SPRING_MS;
    Animation::new(std::time::Duration::from_millis(total).mul_f32(motion::speed_scale()))
}

fn spring_at(raw: f32) -> f32 {
    let total = (CARD_DELAY_MS + CARD_SPRING_MS) as f32;
    ((raw * total - CARD_DELAY_MS as f32) / CARD_SPRING_MS as f32).clamp(0.0, 1.0)
}

fn feature_section(theme: &Theme, feature: &'static Feature) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(15.0))
                        .line_height(px(20.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(SharedString::from(feature.title.as_str())),
                )
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(13.0))
                        .line_height(px(19.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(feature.description.as_str())),
                ),
        )
        .when_some(
            feature.image.as_deref().and_then(whats_new::picture),
            |el, picture| {
                el.child(
                    div()
                        .w_full()
                        .aspect_ratio(picture.aspect)
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(theme.border)
                        .child(
                            img(picture.image.clone())
                                .size_full()
                                .rounded(px(7.0))
                                .object_fit(ObjectFit::Cover),
                        ),
                )
            },
        )
        .when_some(feature.details.as_deref(), |el, details| {
            el.child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .line_height(px(18.0))
                    .text_color(theme.text_faint)
                    .child(SharedString::from(details)),
            )
        })
}

fn help_row(
    theme: &Theme,
    id: &'static str,
    glyph: &'static str,
    label: &'static str,
) -> gpui::Stateful<Div> {
    popover::menu_row(theme, false, id)
        .id(id)
        .child(icon(glyph).size(px(16.0)).text_color(theme.text_muted))
        .child(SharedString::from(label))
}

fn staggered(id: SharedString, index: usize, el: Div) -> AnyElement {
    let spec = FEATURE_IN.with_delay(index as u64 * FEATURE_STAGGER_MS);
    el.with_animation(id, spec.animation(), |el, t| {
        el.relative().opacity(t).top(px(8.0 * (1.0 - t)))
    })
    .into_any_element()
}

impl Shell {
    fn mark_whats_new_seen(&mut self, cx: &mut Context<Self>) {
        let seen = self.settings.whats_new_seen_version.as_deref();
        if seen.is_none_or(|seen| zeron_update::version_newer(CURRENT_VERSION, seen)) {
            self.settings.whats_new_seen_version = Some(CURRENT_VERSION.into());
            self.schedule_save(cx);
        }
    }

    fn dismiss_whats_new_card(&mut self, cx: &mut Context<Self>) {
        self.whats_new_card = None;
        self.mark_whats_new_seen(cx);
        cx.notify();
    }

    fn open_whats_new(
        &mut self,
        entry: &'static Entry,
        view: WhatsNewView,
        cx: &mut Context<Self>,
    ) {
        self.whats_new_dialog = Some(WhatsNewDialog {
            view,
            entry,
            expanded: std::collections::HashSet::from([entry.version.as_str()]),
        });
        cx.notify();
    }

    pub(super) fn open_whats_new_changelog(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = whats_new::entries().first() {
            self.open_whats_new(entry, WhatsNewView::Changelog, cx);
        }
    }

    pub(super) fn open_whats_new_highlights(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = anchor_entry() {
            self.open_whats_new(entry, WhatsNewView::Highlights, cx);
        }
    }

    pub(super) fn close_whats_new(&mut self, cx: &mut Context<Self>) {
        self.whats_new_dialog = None;
        self.whats_new_card = None;
        self.mark_whats_new_seen(cx);
        cx.notify();
    }

    fn set_whats_new_view(&mut self, view: WhatsNewView, cx: &mut Context<Self>) {
        if let Some(dialog) = self.whats_new_dialog.as_mut() {
            dialog.view = view;
            cx.notify();
        }
    }

    pub(super) fn close_help_menu(&mut self, cx: &mut Context<Self>) {
        if self.help_menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.help_menu);
            cx.notify();
        }
    }

    pub(super) fn render_help_menu(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let open = self.help_menu.is_open();
        let trigger = app_rail::button(
            "rail-help",
            "Help",
            icons::LUCIDE_CIRCLE_HELP,
            open,
            self.rail_wash(theme),
            theme,
        )
        .relative()
        .aria_expanded(open)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.help_menu.note_trigger_press()),
        )
        .on_click(cx.listener(|this, _, _, cx| {
            if this.help_menu.take_press_was_open() {
                this.close_help_menu(cx);
            } else {
                this.help_menu.open(());
            }
            cx.notify();
        }))
        .when(!open, |el| {
            el.tooltip(crate::settings::widgets::text_tooltip("Help"))
        });
        if self.help_menu.get().is_none() {
            return trigger.into_any_element();
        }
        let theme = &theme.for_popup();
        let releases: Vec<AnyElement> = whats_new::entries()
            .iter()
            .take(HELP_RELEASES)
            .map(|entry| {
                let id = SharedString::from(format!("help-release-{}", entry.version));
                let title = entry.features.first().map_or_else(
                    || format!("Version {}", entry.version),
                    |feature| feature.title.clone(),
                );
                popover::menu_row(theme, false, id.clone())
                    .id(id)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_help_menu(cx);
                        this.open_whats_new(entry, WhatsNewView::Changelog, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(title)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.text_muted)
                            .child(SharedString::from(entry.date.as_str())),
                    )
                    .into_any_element()
            })
            .collect();
        let menu = popover::popover_card(theme)
            .w(px(HELP_MENU_WIDTH))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_help_menu(cx)))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                div()
                    .px(px(8.0))
                    .pt(px(6.0))
                    .pb(px(4.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from("What\u{2019}s new")),
            )
            .children(releases)
            .child(
                help_row(
                    theme,
                    "help-changelog",
                    icons::LUCIDE_GIFT,
                    "Full changelog",
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_help_menu(cx);
                    this.open_whats_new_changelog(cx);
                })),
            )
            .child(popover::menu_separator())
            .child(
                help_row(
                    theme,
                    "help-keybindings",
                    icons::LUCIDE_KEYBOARD,
                    "Keybindings",
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_help_menu(cx);
                    this.open_settings(SettingsSection::Shortcuts, cx);
                })),
            )
            .child(
                help_row(theme, "help-docs", icons::LUCIDE_BOOK_OPEN, "Docs").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.close_help_menu(cx);
                        this.open_docs(None, cx);
                    }),
                ),
            )
            .into_any_element();
        trigger
            .child(popover::anchored_menu_right(
                "help-menu-popover",
                menu,
                self.help_menu.closing_since(),
            ))
            .into_any_element()
    }

    fn toggle_whats_new_release(&mut self, version: &'static str, cx: &mut Context<Self>) {
        if let Some(dialog) = self.whats_new_dialog.as_mut() {
            if !dialog.expanded.remove(version) {
                dialog.expanded.insert(version);
            }
            cx.notify();
        }
    }

    pub(super) fn render_whats_new_card(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let card = self.whats_new_card.as_ref()?;
        if self.whats_new_dialog.is_some() {
            return None;
        }
        let entry = card.entry;
        let surface = theme.surface_raised;
        let picture = entry
            .hero
            .as_deref()
            .and_then(whats_new::picture)
            .filter(|_| !card.gradient_hero);
        let title = entry
            .features
            .first()
            .map(|feature| SharedString::from(feature.title.as_str()))
            .unwrap_or_else(|| format!("What\u{2019}s new in v{}", entry.version).into());
        let glint = gpui::white().opacity(0.30);
        let glint_band = |from: gpui::Hsla, to: gpui::Hsla| {
            div().h_full().flex_1().bg(linear_gradient(
                90.0,
                linear_color_stop(from, 0.0),
                linear_color_stop(to, 1.0),
            ))
        };
        let shine = div()
            .absolute()
            .top_0()
            .h_full()
            .w(relative(0.45))
            .flex()
            .flex_row()
            .child(glint_band(glint.opacity(0.0), glint))
            .child(glint_band(glint, glint.opacity(0.0)))
            .with_animation("whats-new-card-shine", SHINE.animation(), |el, t| {
                el.left(relative(-0.5 + 1.6 * t))
            });
        let hero = div()
            .relative()
            .h(px(HERO_HEIGHT))
            .w_full()
            .overflow_hidden()
            .map(|el| match picture {
                Some(picture) => el.child(
                    img(picture.image.clone())
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                ),
                None => el
                    .bg(linear_gradient(
                        135.0,
                        linear_color_stop(theme.accent.opacity(0.42), 0.0),
                        linear_color_stop(theme.accent.opacity(0.06), 1.0),
                    ))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        icon(icons::ZERON_LOGO)
                            .size(px(36.0))
                            .text_color(theme.text),
                    ),
            })
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(40.0))
                    .bg(linear_gradient(
                        180.0,
                        linear_color_stop(surface.opacity(0.0), 0.0),
                        linear_color_stop(surface, 1.0),
                    )),
            )
            .child(shine);
        let body = div()
            .px(px(14.0))
            .pt(px(6.0))
            .pb(px(14.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.accent)
                    .child(SharedString::from(format!(
                        "New \u{00B7} v{}",
                        entry.version
                    ))),
            )
            .child(
                div()
                    .mt(px(4.0))
                    .text_size(crate::typography::ui_rems(14.0))
                    .line_height(px(19.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .line_clamp(2)
                    .child(title),
            )
            .child(
                div()
                    .mt(px(10.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(motion::hover_blend(
                        "whats-new-card",
                        theme.text_muted,
                        theme.text,
                    ))
                    .child("See what\u{2019}s new \u{2192}"),
            );
        let dismiss = div()
            .id("whats-new-card-dismiss")
            .absolute()
            .top(px(8.0))
            .right(px(8.0))
            .size(px(22.0))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.35))
            .hover(|s| s.bg(gpui::black().opacity(0.55)))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.dismiss_whats_new_card(cx);
            }))
            .tooltip(crate::settings::widgets::text_tooltip("Dismiss"))
            .child(icon(icons::CLOSE).size(px(10.0)).text_color(gpui::white()));
        Some(
            div()
                .id("whats-new-card")
                .debug_selector(|| "whats-new-card".into())
                .role(gpui::Role::Button)
                .aria_label(format!("Open What\u{2019}s new in v{}", entry.version))
                .relative()
                .mx(px(Theme::SPACE_SM))
                .mb(px(Theme::SPACE_SM))
                .rounded(px(CARD_RADIUS))
                .border_1()
                .border_color(motion::hover_blend(
                    "whats-new-card",
                    theme.border,
                    theme.accent.opacity(0.45),
                ))
                .bg(surface)
                .when(!theme.is_frost(), |el| el.shadow_md())
                .cursor_pointer()
                .on_hover(motion::hover_listener("whats-new-card"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_whats_new(entry, WhatsNewView::Highlights, cx);
                }))
                .child(crate::rounded_clip::rounded_clip(
                    CARD_RADIUS - 1.0,
                    div().flex().flex_col().child(hero).child(body),
                ))
                .child(dismiss)
                .with_animation("whats-new-card-in", card_entrance(), |el, raw| {
                    let t = spring_at(raw);
                    el.opacity(motion::EASE_OUT.eval(t))
                        .top(px(CARD_RISE * (1.0 - SPRING.eval_unclamped(t))))
                })
                .into_any_element(),
        )
    }

    pub(super) fn render_whats_new_dialog(
        &self,
        viewport: gpui::Size<Pixels>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let dialog = self.whats_new_dialog.as_ref()?;
        if self.sync_flow.has_visible_overlay() {
            return None;
        }
        let entry = dialog.entry;
        let caption = |copy: SharedString| {
            div()
                .text_size(crate::typography::ui_rems(12.0))
                .line_height(px(17.0))
                .text_color(theme.text_muted)
                .child(copy)
        };
        let heading = |leading: AnyElement, title: &str, subtitle: SharedString| {
            div()
                .px(px(20.0))
                .pt(px(18.0))
                .pb(px(12.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(leading)
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(popover::dialog_title(theme, title))
                        .child(caption(subtitle)),
                )
        };
        let header = match dialog.view {
            WhatsNewView::Highlights => heading(
                icon(icons::ZERON_LOGO)
                    .size(px(28.0))
                    .flex_none()
                    .text_color(theme.text)
                    .into_any_element(),
                "What\u{2019}s new?",
                format!("v{} \u{00B7} {}", entry.version, entry.date).into(),
            ),
            WhatsNewView::Changelog => heading(
                div()
                    .id("whats-new-back")
                    .role(gpui::Role::Button)
                    .aria_label("Back to What\u{2019}s new")
                    .size(px(28.0))
                    .flex_none()
                    .rounded(px(8.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(crate::theme::ink(0.06)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_whats_new_view(WhatsNewView::Highlights, cx);
                    }))
                    .child(
                        icon(icons::ARROW_LEFT)
                            .size(px(16.0))
                            .text_color(theme.text_muted),
                    )
                    .into_any_element(),
                "Changelog",
                "Every release, newest first.".into(),
            ),
        };
        let content = match dialog.view {
            WhatsNewView::Highlights => div().py(px(4.0)).flex().flex_col().gap(px(28.0)).children(
                entry.features.iter().enumerate().map(|(ix, feature)| {
                    staggered(
                        format!("whats-new-feature-{}-{ix}", entry.version).into(),
                        ix,
                        feature_section(theme, feature),
                    )
                }),
            ),
            WhatsNewView::Changelog => {
                let entries = whats_new::entries();
                div()
                    .flex()
                    .flex_col()
                    .children(entries.iter().enumerate().map(|(row, release)| {
                        let open = dialog.expanded.contains(release.version.as_str());
                        let count = release.features.len();
                        let version: &'static str = release.version.as_str();
                        div()
                            .flex()
                            .flex_col()
                            .when(row + 1 < entries.len(), |el| {
                                el.border_b_1().border_color(theme.border)
                            })
                            .child(
                                div()
                                    .id(("whats-new-release", row))
                                    .role(gpui::Role::Button)
                                    .aria_expanded(open)
                                    .py(px(10.0))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(10.0))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_whats_new_release(version, cx);
                                    }))
                                    .child(
                                        icon(if open {
                                            icons::LUCIDE_CHEVRON_DOWN
                                        } else {
                                            icons::LUCIDE_CHEVRON_RIGHT
                                        })
                                        .size(px(14.0))
                                        .flex_none()
                                        .text_color(theme.text_muted),
                                    )
                                    .child(caption(SharedString::from(release.date.as_str())))
                                    .child(
                                        div()
                                            .text_size(crate::typography::ui_rems(14.0))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(theme.text)
                                            .child(SharedString::from(format!(
                                                "Version {}",
                                                release.version
                                            ))),
                                    )
                                    .child(
                                        div()
                                            .text_size(crate::typography::ui_rems(12.0))
                                            .text_color(theme.text_faint)
                                            .child(SharedString::from(format!(
                                                "({count} {})",
                                                if count == 1 { "update" } else { "updates" }
                                            ))),
                                    ),
                            )
                            .when(open, |el| {
                                el.child(
                                    div()
                                        .pl(px(24.0))
                                        .pb(px(16.0))
                                        .flex()
                                        .flex_col()
                                        .gap(px(24.0))
                                        .children(release.features.iter().enumerate().map(
                                            |(ix, feature)| {
                                                staggered(
                                                    format!(
                                                        "whats-new-release-{}-{ix}",
                                                        release.version
                                                    )
                                                    .into(),
                                                    ix,
                                                    feature_section(theme, feature),
                                                )
                                            },
                                        )),
                                )
                            })
                    }))
            }
        };
        let body = div()
            .id(match dialog.view {
                WhatsNewView::Highlights => "whats-new-highlights",
                WhatsNewView::Changelog => "whats-new-changelog",
            })
            .max_h(px((f32::from(viewport.height) * 0.62).min(DIALOG_BODY_MAX)))
            .overflow_y_scroll()
            .px(px(20.0))
            .py(px(8.0))
            .child(content);
        let footer = div()
            .px(px(20.0))
            .py(px(14.0))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(match dialog.view {
                WhatsNewView::Highlights => {
                    popover::btn_ghost(theme, "View changelog \u{2192}", "whats-new-view-changelog")
                        .id("whats-new-view-changelog")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_whats_new_view(WhatsNewView::Changelog, cx);
                        }))
                        .into_any_element()
                }
                WhatsNewView::Changelog => div().into_any_element(),
            })
            .child(
                popover::btn_primary(theme, "Got it")
                    .id("whats-new-got-it")
                    .on_click(cx.listener(|this, _, _, cx| this.close_whats_new(cx))),
            );
        let card = popover::dialog_card(theme)
            .w(px(DIALOG_WIDTH))
            .p(px(0.0))
            .child(header)
            .child(body)
            .child(footer)
            .into_any_element();
        Some(popover::modal_glass(
            "whats-new-dialog",
            viewport,
            card,
            16.0,
        ))
    }
}
