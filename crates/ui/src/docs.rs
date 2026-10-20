use std::sync::OnceLock;

use gpui::SharedString;

use crate::markdown::{BlockTree, parse_full};
use crate::shell::SettingsSection;

pub use zeron_docs::{Article, Docs, Section, article, docs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Tasks,
    Supervisor,
    Inbox,
    Usage,
    PullRequests,
    Changelog,
    Settings(SettingsSection),
    Article(&'static str),
}

pub trait ArticleExt {
    fn tree(&self) -> &'static BlockTree;
    fn icon_path(&self) -> SharedString;
    fn actions(&self) -> impl Iterator<Item = Action> + '_;
}

impl ArticleExt for Article {
    fn tree(&self) -> &'static BlockTree {
        static TREES: OnceLock<Vec<OnceLock<BlockTree>>> = OnceLock::new();
        static EMPTY: OnceLock<BlockTree> = OnceLock::new();
        let trees =
            TREES.get_or_init(|| zeron_docs::BODIES.iter().map(|_| OnceLock::new()).collect());
        match zeron_docs::BODIES.iter().position(|(id, _)| *id == self.id) {
            Some(ix) => trees[ix].get_or_init(|| parse_full(self.body())),
            None => EMPTY.get_or_init(|| parse_full("")),
        }
    }

    fn icon_path(&self) -> SharedString {
        format!("icons/{}.svg", self.icon).into()
    }

    fn actions(&self) -> impl Iterator<Item = Action> + '_ {
        self.actions
            .iter()
            .filter_map(|target| Action::parse(target))
    }
}

impl Action {
    pub fn parse(target: &str) -> Option<Self> {
        Some(match target {
            "tasks" => Self::Tasks,
            "supervisor" => Self::Supervisor,
            "inbox" => Self::Inbox,
            "usage" => Self::Usage,
            "pull-requests" => Self::PullRequests,
            "changelog" => Self::Changelog,
            _ => match target.strip_prefix("settings/") {
                Some(slug) => Self::Settings(SettingsSection::from_slug(slug)?),
                None => Self::Article(article(target.strip_prefix("docs/")?)?.id.as_str()),
            },
        })
    }

    pub fn from_link(url: &str) -> Option<Self> {
        Self::parse(url.strip_prefix("zeron://")?)
    }

    pub fn label(self) -> SharedString {
        match self {
            Self::Tasks => "Open Tasks".into(),
            Self::Supervisor => "Open Supervisor".into(),
            Self::Inbox => "Open Inbox".into(),
            Self::Usage => "Open Usage".into(),
            Self::PullRequests => "Open Pull requests".into(),
            Self::Changelog => "Open changelog".into(),
            Self::Settings(section) => format!("Settings \u{203A} {}", section.label()).into(),
            Self::Article(id) => article(id).map_or_else(
                || SharedString::from(id),
                |article| article.title.clone().into(),
            ),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Tasks => crate::icons::TASKS,
            Self::Supervisor => crate::icons::LUCIDE_SHIELD,
            Self::Inbox => crate::icons::BELL,
            Self::Usage => crate::icons::LUCIDE_CHART_NO_AXES_COLUMN,
            Self::PullRequests => crate::icons::LUCIDE_GIT_PULL_REQUEST,
            Self::Changelog => crate::icons::LUCIDE_GIFT,
            Self::Settings(_) => crate::icons::LUCIDE_SETTINGS,
            Self::Article(_) => crate::icons::LUCIDE_BOOK_OPEN,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{Block, InlineRun};
    use gpui::AssetSource as _;

    fn runs(block: &Block) -> Vec<&InlineRun> {
        match block {
            Block::Paragraph { runs } | Block::Heading { runs, .. } => runs.iter().collect(),
            Block::BlockQuote { children } => children.iter().flat_map(runs).collect(),
            Block::List { items, .. } => items.iter().flatten().flat_map(runs).collect(),
            Block::Table { header, rows, .. } => header
                .iter()
                .chain(rows.iter().flatten())
                .flatten()
                .collect(),
            Block::CodeBlock { .. } | Block::Math { .. } | Block::Rule => Vec::new(),
        }
    }

    #[test]
    fn every_article_icon_action_link_and_image_resolves() {
        let docs = docs();
        assert!(!docs.articles.is_empty());
        for article in &docs.articles {
            let id = article.id.as_str();
            assert!(
                crate::icons::Assets
                    .load(&article.icon_path())
                    .unwrap()
                    .is_some(),
                "{id}: icon {} is not bundled",
                article.icon
            );
            for target in &article.actions {
                assert!(Action::parse(target).is_some(), "{id}: action {target}");
            }
            for top in &article.tree().blocks {
                assert!(
                    !matches!(top.block, Block::Heading { level: 1, .. }),
                    "{id}: the title comes from the index, not an H1"
                );
                for run in runs(&top.block) {
                    if let Some(link) = run
                        .style
                        .link
                        .as_deref()
                        .filter(|l| l.starts_with("zeron:"))
                    {
                        assert!(Action::from_link(link).is_some(), "{id}: link {link}");
                    }
                    if let Some(image) = &run.style.image {
                        assert!(
                            crate::whats_new::picture(&image.source).is_some(),
                            "{id}: image {} is not bundled",
                            image.source
                        );
                    }
                }
            }
        }
    }
}
