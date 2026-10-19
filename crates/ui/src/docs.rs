use std::sync::OnceLock;

use gpui::SharedString;
use serde::Deserialize;

use crate::markdown::{BlockTree, parse_full};
use crate::shell::SettingsSection;

const SOURCE: &str = include_str!("../docs/index.toml");

const BODIES: &[(&str, &str)] = &[
    ("getting-around", include_str!("../docs/getting-around.md")),
    ("new-chat", include_str!("../docs/new-chat.md")),
    ("composer", include_str!("../docs/composer.md")),
    (
        "commands-and-mentions",
        include_str!("../docs/commands-and-mentions.md"),
    ),
    (
        "models-and-effort",
        include_str!("../docs/models-and-effort.md"),
    ),
    (
        "sessions-sidebar",
        include_str!("../docs/sessions-sidebar.md"),
    ),
    (
        "command-palette",
        include_str!("../docs/command-palette.md"),
    ),
    (
        "keyboard-shortcuts",
        include_str!("../docs/keyboard-shortcuts.md"),
    ),
    ("appearance", include_str!("../docs/appearance.md")),
    ("home-widgets", include_str!("../docs/home-widgets.md")),
    ("updates", include_str!("../docs/updates.md")),
    ("providers", include_str!("../docs/providers.md")),
    ("agent-profiles", include_str!("../docs/agent-profiles.md")),
    ("delegation", include_str!("../docs/delegation.md")),
    ("side-chat", include_str!("../docs/side-chat.md")),
    ("handoff", include_str!("../docs/handoff.md")),
    ("review", include_str!("../docs/review.md")),
    ("compaction", include_str!("../docs/compaction.md")),
    (
        "agent-questions",
        include_str!("../docs/agent-questions.md"),
    ),
    ("agent-tools", include_str!("../docs/agent-tools.md")),
    ("projects", include_str!("../docs/projects.md")),
    ("right-pane", include_str!("../docs/right-pane.md")),
    ("terminal", include_str!("../docs/terminal.md")),
    ("git", include_str!("../docs/git.md")),
    ("pull-requests", include_str!("../docs/pull-requests.md")),
    ("tasks", include_str!("../docs/tasks.md")),
    ("mission-control", include_str!("../docs/mission-control.md")),
    ("supervisor", include_str!("../docs/supervisor.md")),
    ("inbox", include_str!("../docs/inbox.md")),
    ("usage", include_str!("../docs/usage.md")),
    ("split-view", include_str!("../docs/split-view.md")),
    ("computer-use", include_str!("../docs/computer-use.md")),
    ("browser", include_str!("../docs/browser.md")),
    ("appshots", include_str!("../docs/appshots.md")),
    ("device-hub", include_str!("../docs/device-hub.md")),
    ("voice", include_str!("../docs/voice.md")),
    ("notifications", include_str!("../docs/notifications.md")),
    ("devices", include_str!("../docs/devices.md")),
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    section: Vec<Section>,
    #[serde(default)]
    article: Vec<Article>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Article {
    pub id: String,
    pub section: String,
    pub title: String,
    pub icon: String,
    pub blurb: String,
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(skip)]
    haystack: String,
    #[serde(skip)]
    tree: OnceLock<BlockTree>,
}

#[derive(Debug, Default)]
pub struct Docs {
    pub sections: Vec<Section>,
    pub articles: Vec<Article>,
}

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

fn parse(source: &str) -> Result<Docs, toml::de::Error> {
    let Index {
        section: sections,
        article: mut articles,
    } = toml::from_str(source)?;
    articles.sort_by_key(|article| {
        sections
            .iter()
            .position(|section| section.id == article.section)
            .unwrap_or(usize::MAX)
    });
    for article in &mut articles {
        article.haystack =
            format!("{}\n{}\n{}", article.title, article.blurb, article.body()).to_lowercase();
    }
    Ok(Docs { sections, articles })
}

pub fn docs() -> &'static Docs {
    static DOCS: OnceLock<Docs> = OnceLock::new();
    DOCS.get_or_init(|| {
        parse(SOURCE).unwrap_or_else(|err| {
            tracing::warn!(error = %err, "docs/index.toml does not parse");
            Docs::default()
        })
    })
}

pub fn article(id: &str) -> Option<&'static Article> {
    docs().articles.iter().find(|article| article.id == id)
}

impl Docs {
    pub fn section(&self, id: &str) -> Option<&Section> {
        self.sections.iter().find(|section| section.id == id)
    }

    pub fn in_section<'a>(&'a self, section: &'a Section) -> impl Iterator<Item = &'a Article> {
        self.articles
            .iter()
            .filter(move |article| article.section == section.id)
    }

    pub fn neighbors(&self, id: &str) -> (Option<&Article>, Option<&Article>) {
        let Some(ix) = self.articles.iter().position(|article| article.id == id) else {
            return (None, None);
        };
        (
            ix.checked_sub(1).and_then(|ix| self.articles.get(ix)),
            self.articles.get(ix + 1),
        )
    }
}

impl Article {
    pub fn body(&self) -> &'static str {
        BODIES
            .iter()
            .find(|(id, _)| *id == self.id)
            .map_or("", |(_, body)| body)
    }

    pub fn tree(&self) -> &BlockTree {
        self.tree.get_or_init(|| parse_full(self.body()))
    }

    pub fn icon_path(&self) -> SharedString {
        format!("icons/{}.svg", self.icon).into()
    }

    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty() || self.haystack.contains(&query)
    }

    pub fn actions(&self) -> impl Iterator<Item = Action> + '_ {
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
    fn bundled_index_parses_with_every_body_icon_action_link_and_image() {
        let docs = parse(SOURCE).unwrap();
        assert!(!docs.sections.is_empty());
        for (ix, article) in docs.articles.iter().enumerate() {
            let id = article.id.as_str();
            assert!(docs.section(&article.section).is_some(), "{id}: no section");
            assert!(
                docs.articles[..ix].iter().all(|other| other.id != id),
                "{id} is listed twice"
            );
            assert!(!article.body().trim().is_empty(), "{id} has no body");
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
        for (id, _) in BODIES {
            assert!(
                docs.articles.iter().any(|article| article.id == *id),
                "{id}.md is not in the index"
            );
        }
        for section in &docs.sections {
            assert!(
                docs.in_section(section).next().is_some(),
                "{} has no articles",
                section.id
            );
        }
    }

    #[test]
    fn search_matches_title_and_body_ignoring_case() {
        let docs = parse(SOURCE).unwrap();
        let Some(article) = docs.articles.first() else {
            return;
        };
        assert!(article.matches(""));
        assert!(article.matches(&article.title.to_uppercase()));
        let word = article
            .body()
            .split_whitespace()
            .find(|word| word.len() > 6 && !article.title.contains(*word))
            .unwrap();
        assert!(article.matches(word));
        assert!(!article.matches("no article mentions this phrase"));
    }
}
