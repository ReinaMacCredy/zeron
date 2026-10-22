use std::rc::Rc;
use std::sync::Arc;

use super::*;
use crate::docs::{self, Action, Article, ArticleExt as _};
use crate::markdown::parser::{Block, BlockTree, TopBlock};
use crate::markdown::render::{self as md, LinkOutcome, LinkUi, MediaUi, RenderOptions};
use crate::settings::widgets::{self, ActionTone};
use gpui::{ObjectFit, img, svg};

const TOC_WIDTH: f32 = 232.0;
const CARD_MIN_WIDTH: f32 = 220.0;
const OVERVIEW_MAX_WIDTH: f32 = 960.0;
const OVERVIEW_PAD_X: f32 = 40.0;

fn article_icon(article: &Article, size: f32, color: gpui::Hsla) -> gpui::Svg {
    svg()
        .path(article.icon_path())
        .flex_none()
        .size(px(size))
        .text_color(color)
}

impl Shell {
    pub(super) fn open_docs(&mut self, article: Option<&'static str>, cx: &mut Context<Self>) {
        if matches!(self.route, Route::Docs) && self.docs_article == article {
            return;
        }
        self.enter_docs_route(article, cx);
        self.nav.push(NavEntry::Docs(article));
    }

    pub(super) fn enter_docs_route(
        &mut self,
        article: Option<&'static str>,
        cx: &mut Context<Self>,
    ) {
        self.set_route(Route::Docs, cx);
        self.docs_article = article;
        self.close_user_menu(cx);
        self.close_chat_menu(cx);
        cx.notify();
    }

    pub(super) fn run_docs_action(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::Tasks => self.open_tasks(cx),
            Action::Supervisor => self.open_supervisor(cx),
            Action::Inbox => self.open_inbox(cx),
            Action::Usage => self.open_usage(cx),
            Action::PullRequests => self.open_pull_requests(cx),
            Action::Changelog => self.open_whats_new_changelog(cx),
            Action::Settings(section) => self.open_settings(section, cx),
            Action::Article(id) => self.open_docs(Some(id), cx),
        }
    }

    pub(super) fn render_docs_page(
        &mut self,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let article = self.docs_article.and_then(docs::article);
        let toc = self.render_docs_toc(article, &theme, window, cx);
        let content = match article {
            Some(article) => self.render_docs_article(article, &theme, window, cx),
            None => self.render_docs_overview(width - TOC_WIDTH, &theme, cx),
        };
        let key = article.map_or("overview", |article| article.id.as_str());
        div()
            .id("docs-page")
            .debug_selector(|| "docs-page".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_row()
            .child(toc)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(widgets::scroll_faded(
                        format!("docs-scroll-{key}"),
                        div()
                            .id(SharedString::from(format!("docs-content-{key}")))
                            .size_full()
                            .overflow_y_scroll()
                            .child(content),
                    )),
            )
            .into_any_element()
    }

    fn render_docs_toc(
        &mut self,
        current: Option<&'static Article>,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let query = self.docs_search.read(cx).text().to_string();
        let reduced = motion::reduced_motion(cx);
        let mut tab = |key: String,
                       selected: bool,
                       glyph: gpui::Svg,
                       label: SharedString,
                       target: Option<&'static str>| {
            let hover_key = format!("{key}-hover");
            let t = widgets::tab_selection_t(window, format!("{key}-selection"), selected, reduced);
            let tint = motion::hover_blend(
                &hover_key,
                motion::mix(theme.text_muted, theme.text, t),
                theme.text,
            );
            widgets::section_tab(theme, selected, t, key, hover_key)
                .role(gpui::Role::Tab)
                .aria_label(label.clone())
                .aria_selected(selected)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.open_docs(target, cx)))
                .child(glyph.size(px(16.0)).text_color(tint))
                .child(div().min_w_0().truncate().child(label))
                .into_any_element()
        };
        let all = docs::docs();
        let mut rows = vec![tab(
            "docs-toc-overview".into(),
            current.is_none(),
            icon(icons::LUCIDE_BOOK_OPEN),
            "Overview".into(),
            None,
        )];
        let mut found = false;
        for section in &all.sections {
            let hits: Vec<&'static Article> = all
                .in_section(section)
                .filter(|article| article.matches(&query))
                .collect();
            if hits.is_empty() {
                continue;
            }
            found = true;
            rows.push(
                div()
                    .mt(px(Theme::SPACE_MD))
                    .px(px(Theme::SPACE_SM))
                    .py(px(4.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_faint)
                    .child(SharedString::from(section.title.as_str()))
                    .into_any_element(),
            );
            for article in hits {
                rows.push(tab(
                    format!("docs-toc-{}", article.id),
                    current.is_some_and(|current| std::ptr::eq(current, article)),
                    svg().path(article.icon_path()).flex_none(),
                    article.title.clone().into(),
                    Some(article.id.as_str()),
                ));
            }
        }
        if !found {
            rows.push(
                div()
                    .mt(px(Theme::SPACE_MD))
                    .px(px(Theme::SPACE_SM))
                    .text_size(crate::typography::ui_rems(13.0))
                    .text_color(theme.text_muted)
                    .child("No articles match.")
                    .into_any_element(),
            );
        }
        div()
            .w(px(TOC_WIDTH))
            .flex_none()
            .h_full()
            .pt(px(Theme::TITLEBAR_HEIGHT))
            .border_r_1()
            .border_color(widgets::row_divider(theme))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(px(Theme::SPACE_SM))
                    .pt(px(Theme::SPACE_MD))
                    .child(
                        popover::search_input_frame(
                            theme,
                            self.docs_search.clone().into_any_element(),
                        )
                        .mb(px(6.0)),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(widgets::scroll_faded(
                    "docs-toc-scroll",
                    div()
                        .id("docs-toc")
                        .role(gpui::Role::TabList)
                        .aria_label("Docs articles")
                        .size_full()
                        .overflow_y_scroll()
                        .px(px(Theme::SPACE_SM))
                        .pb(px(Theme::SPACE_LG))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .children(rows),
                )),
            )
            .into_any_element()
    }

    fn render_docs_overview(
        &mut self,
        width: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let columns = ((width.min(OVERVIEW_MAX_WIDTH) - 2.0 * OVERVIEW_PAD_X) / CARD_MIN_WIDTH)
            .floor()
            .clamp(1.0, 3.0) as u16;
        let all = docs::docs();
        let mut column = widgets::page_column()
            .max_w(px(OVERVIEW_MAX_WIDTH))
            .px(px(OVERVIEW_PAD_X))
            .gap(px(28.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(widgets::page_header(theme, "Zeron docs", None))
                    .child(widgets::page_subtitle(
                        theme,
                        "How each part of Zeron works and where to find it. Pick a card, or search the list on the left.",
                    )),
            );
        for section in &all.sections {
            let cards = all.in_section(section).map(|article| {
                let hover = format!("docs-card-{}", article.id);
                let id = article.id.as_str();
                div()
                    .id(SharedString::from(hover.clone()))
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from(article.title.as_str()))
                    .min_w_0()
                    .p(px(14.0))
                    .rounded(px(12.0))
                    .bg(motion::hover_blend(
                        &hover,
                        widgets::block_fill(theme),
                        theme.wash(0.08),
                    ))
                    .cursor_pointer()
                    .on_hover(motion::hover_listener(hover))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_docs(Some(id), cx)))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(article_icon(article, 18.0, theme.text_muted))
                    .child(
                        div()
                            .mt(px(4.0))
                            .text_size(crate::typography::ui_rems(14.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(SharedString::from(article.title.as_str())),
                    )
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(12.5))
                            .line_height(crate::typography::ui_rems(17.0))
                            .text_color(theme.text_muted)
                            .line_clamp(2)
                            .child(SharedString::from(article.blurb.as_str())),
                    )
            });
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(widgets::section_label(theme, section.title.as_str()))
                    .child(
                        div()
                            .grid()
                            .grid_cols(columns)
                            .gap(px(12.0))
                            .children(cards),
                    ),
            );
        }
        column.into_any_element()
    }

    fn render_docs_article(
        &mut self,
        article: &'static Article,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let all = docs::docs();
        let section = all
            .section(&article.section)
            .map_or("", |section| section.title.as_str());
        let breadcrumb = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .text_size(crate::typography::ui_rems(12.5))
            .text_color(theme.text_muted)
            .child(
                div()
                    .id("docs-breadcrumb-home")
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme.text))
                    .on_click(cx.listener(|this, _, _, cx| this.open_docs(None, cx)))
                    .child("Docs"),
            )
            .child("\u{203A}")
            .child(SharedString::from(section));
        let actions: Vec<AnyElement> = article
            .actions()
            .enumerate()
            .map(|(ix, action)| {
                widgets::action_button(theme, ActionTone::Outlined)
                    .id(("docs-action", ix))
                    .on_click(cx.listener(move |this, _, _, cx| this.run_docs_action(action, cx)))
                    .child(
                        icon(action.icon())
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    )
                    .child(action.label())
                    .into_any_element()
            })
            .collect();
        let (previous, next) = all.neighbors(&article.id);
        let neighbor = |target: Option<&'static Article>, caption: &'static str, end: bool| {
            let Some(target) = target else {
                return div().flex_1().into_any_element();
            };
            let id = target.id.as_str();
            let hover = format!("docs-neighbor-{caption}");
            div()
                .id(SharedString::from(hover.clone()))
                .flex_1()
                .min_w_0()
                .p(px(12.0))
                .rounded(px(10.0))
                .bg(motion::hover_blend(
                    &hover,
                    widgets::block_fill(theme),
                    theme.wash(0.08),
                ))
                .cursor_pointer()
                .on_hover(motion::hover_listener(hover))
                .on_click(cx.listener(move |this, _, _, cx| this.open_docs(Some(id), cx)))
                .flex()
                .flex_col()
                .when(end, |el| el.items_end())
                .gap(px(2.0))
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(caption),
                )
                .child(
                    div()
                        .max_w_full()
                        .truncate()
                        .text_size(crate::typography::ui_rems(14.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(SharedString::from(target.title.as_str())),
                )
                .into_any_element()
        };
        let footer = div()
            .mt(px(16.0))
            .pt(px(16.0))
            .border_t_1()
            .border_color(widgets::row_divider(theme))
            .flex()
            .flex_row()
            .gap(px(12.0))
            .child(neighbor(previous, "Previous", false))
            .child(neighbor(next, "Next", true));
        widgets::page_column()
            .gap(px(20.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(breadcrumb)
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(24.0))
                            .line_height(crate::typography::ui_rems(30.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(SharedString::from(article.title.as_str())),
                    )
                    .when(!actions.is_empty(), |el| {
                        el.child(
                            div()
                                .mt(px(6.0))
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap(px(8.0))
                                .children(actions),
                        )
                    }),
            )
            .child(self.render_docs_body(article, theme, window, cx))
            .child(footer)
            .into_any_element()
    }

    fn render_docs_body(
        &self,
        article: &'static Article,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shell = cx.weak_entity();
        let mut options = RenderOptions::settled(format!("docs-{}", article.id).into());
        options.link = Some(LinkUi {
            source_session: None,
            source_local: false,
            file_roots: None,
            handler: Rc::new(move |activation, _, cx| {
                match Action::from_link(&activation.target.original) {
                    Some(action) => {
                        let _ = shell.update(cx, |this, cx| this.run_docs_action(action, cx));
                        LinkOutcome::Internal
                    }
                    None => activation.web_outcome(false),
                }
            }),
        });
        options.media = Some(MediaUi {
            diagram: None,
            image: Some(Rc::new(
                |image, id, theme| match crate::whats_new::picture(&image.source) {
                    Some(picture) => div()
                        .id(id)
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
                        )
                        .into_any_element(),
                    None => div()
                        .text_color(theme.text_muted)
                        .child(SharedString::from(image.alt.clone()))
                        .into_any_element(),
                },
            )),
        });
        let tree = |blocks: Vec<Arc<TopBlock>>, key: String| {
            let mut scoped = options.clone();
            scoped.row_key = key.into();
            md::render_tree(&BlockTree { blocks }, &scoped, theme, window, &|_| None)
        };
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(md::MD_BLOCK_GAP))
            .children(article.tree().blocks.iter().enumerate().map(|(ix, top)| {
                let key = format!("{}-{ix}", options.row_key);
                let Block::BlockQuote { children } = &top.block else {
                    return tree(vec![top.clone()], key);
                };
                let children = children
                    .iter()
                    .map(|block| {
                        Arc::new(TopBlock {
                            range: top.range.clone(),
                            block: block.clone(),
                        })
                    })
                    .collect();
                div()
                    .flex()
                    .flex_row()
                    .gap(px(10.0))
                    .p(px(12.0))
                    .rounded(px(10.0))
                    .bg(theme.accent.opacity(0.08))
                    .border_1()
                    .border_color(theme.accent.opacity(0.22))
                    .child(
                        icon(icons::SPARK)
                            .mt(px(3.0))
                            .size(px(16.0))
                            .text_color(theme.accent),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(12.5))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(theme.accent)
                                    .child("Tip"),
                            )
                            .child(tree(children, key)),
                    )
                    .into_any_element()
            }))
            .into_any_element()
    }
}
