use std::sync::OnceLock;

use serde::Deserialize;

pub mod changelog;
pub mod images;

pub const INDEX: &str = include_str!("../../ui/docs/index.toml");

pub const BODIES: &[(&str, &str)] = &[
    (
        "getting-around",
        include_str!("../../ui/docs/getting-around.md"),
    ),
    ("new-chat", include_str!("../../ui/docs/new-chat.md")),
    ("composer", include_str!("../../ui/docs/composer.md")),
    (
        "conversation",
        include_str!("../../ui/docs/conversation.md"),
    ),
    (
        "commands-and-mentions",
        include_str!("../../ui/docs/commands-and-mentions.md"),
    ),
    (
        "models-and-effort",
        include_str!("../../ui/docs/models-and-effort.md"),
    ),
    (
        "sessions-sidebar",
        include_str!("../../ui/docs/sessions-sidebar.md"),
    ),
    (
        "command-palette",
        include_str!("../../ui/docs/command-palette.md"),
    ),
    (
        "keyboard-shortcuts",
        include_str!("../../ui/docs/keyboard-shortcuts.md"),
    ),
    ("appearance", include_str!("../../ui/docs/appearance.md")),
    (
        "home-widgets",
        include_str!("../../ui/docs/home-widgets.md"),
    ),
    ("updates", include_str!("../../ui/docs/updates.md")),
    ("providers", include_str!("../../ui/docs/providers.md")),
    (
        "agent-profiles",
        include_str!("../../ui/docs/agent-profiles.md"),
    ),
    ("routing", include_str!("../../ui/docs/routing.md")),
    ("delegation", include_str!("../../ui/docs/delegation.md")),
    ("threads", include_str!("../../ui/docs/threads.md")),
    ("side-chat", include_str!("../../ui/docs/side-chat.md")),
    ("handoff", include_str!("../../ui/docs/handoff.md")),
    ("review", include_str!("../../ui/docs/review.md")),
    ("compaction", include_str!("../../ui/docs/compaction.md")),
    (
        "agent-questions",
        include_str!("../../ui/docs/agent-questions.md"),
    ),
    ("agent-tools", include_str!("../../ui/docs/agent-tools.md")),
    ("projects", include_str!("../../ui/docs/projects.md")),
    ("right-pane", include_str!("../../ui/docs/right-pane.md")),
    ("live-state", include_str!("../../ui/docs/live-state.md")),
    ("terminal", include_str!("../../ui/docs/terminal.md")),
    ("git", include_str!("../../ui/docs/git.md")),
    (
        "pull-requests",
        include_str!("../../ui/docs/pull-requests.md"),
    ),
    ("tasks", include_str!("../../ui/docs/tasks.md")),
    (
        "mission-control",
        include_str!("../../ui/docs/mission-control.md"),
    ),
    ("supervisor", include_str!("../../ui/docs/supervisor.md")),
    ("inbox", include_str!("../../ui/docs/inbox.md")),
    ("usage", include_str!("../../ui/docs/usage.md")),
    ("split-view", include_str!("../../ui/docs/split-view.md")),
    (
        "computer-use",
        include_str!("../../ui/docs/computer-use.md"),
    ),
    ("browser", include_str!("../../ui/docs/browser.md")),
    ("appshots", include_str!("../../ui/docs/appshots.md")),
    ("device-hub", include_str!("../../ui/docs/device-hub.md")),
    ("voice", include_str!("../../ui/docs/voice.md")),
    (
        "notifications",
        include_str!("../../ui/docs/notifications.md"),
    ),
    ("devices", include_str!("../../ui/docs/devices.md")),
    (
        "command-line",
        include_str!("../../ui/docs/command-line.md"),
    ),
    ("mobile", include_str!("../../ui/docs/mobile.md")),
    ("mobile-composer", include_str!("../../ui/docs/mobile-composer.md")),
    ("mobile-transcript", include_str!("../../ui/docs/mobile-transcript.md")),
    ("mobile-tasks", include_str!("../../ui/docs/mobile-tasks.md")),
    ("mobile-inbox", include_str!("../../ui/docs/mobile-inbox.md")),
    ("mobile-git", include_str!("../../ui/docs/mobile-git.md")),
    ("mobile-settings", include_str!("../../ui/docs/mobile-settings.md")),
    ("mobile-devices", include_str!("../../ui/docs/mobile-devices.md")),
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
}

#[derive(Debug, Default)]
pub struct Docs {
    pub sections: Vec<Section>,
    pub articles: Vec<Article>,
}

#[derive(Debug, Clone)]
pub struct Hit<'a> {
    pub article: &'a Article,
    pub snippet: String,
}

pub fn parse(source: &str) -> Result<Docs, toml::de::Error> {
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
        parse(INDEX).unwrap_or_else(|err| {
            tracing::warn!(error = %err, "docs/index.toml does not parse");
            Docs::default()
        })
    })
}

pub fn article(id: &str) -> Option<&'static Article> {
    docs().article(id)
}

impl Docs {
    pub fn article(&self, id: &str) -> Option<&Article> {
        self.articles.iter().find(|article| article.id == id)
    }

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

    pub fn search(&self, query: &str) -> Vec<Hit<'_>> {
        let query = query.to_lowercase();
        let terms: Vec<&str> = query.split_whitespace().collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<(usize, Hit)> = self
            .articles
            .iter()
            .filter(|article| terms.iter().all(|term| article.haystack.contains(term)))
            .map(|article| {
                let heading = format!("{}\n{}", article.title, article.blurb).to_lowercase();
                let rank = terms.iter().filter(|term| heading.contains(*term)).count();
                let snippet =
                    snippet(article.body(), &terms).unwrap_or_else(|| article.blurb.clone());
                (rank, Hit { article, snippet })
            })
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0));
        hits.into_iter().map(|(_, hit)| hit).collect()
    }
}

fn snippet(body: &str, terms: &[&str]) -> Option<String> {
    const BEFORE: usize = 12;
    const AFTER: usize = 24;
    let (line, _) = body
        .lines()
        .map(|line| {
            let lower = line.to_lowercase();
            (
                line,
                terms.iter().filter(|term| lower.contains(*term)).count(),
            )
        })
        .filter(|(_, count)| *count > 0)
        .max_by_key(|(_, count)| *count)?;
    let words: Vec<&str> = line.split_whitespace().collect();
    let at = words.iter().position(|word| {
        let word = word.to_lowercase();
        terms.iter().any(|term| word.contains(term))
    });
    let start = at.unwrap_or(0).saturating_sub(BEFORE);
    let end = (start + BEFORE + AFTER).min(words.len());
    let mut text = words[start..end].join(" ");
    if start > 0 {
        text.insert_str(0, "... ");
    }
    if end < words.len() {
        text.push_str(" ...");
    }
    Some(text)
}

impl Article {
    pub fn body(&self) -> &'static str {
        BODIES
            .iter()
            .find(|(id, _)| *id == self.id)
            .map_or("", |(_, body)| body)
    }

    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty() || self.haystack.contains(&query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_index_lists_every_body_once_in_a_known_section() {
        let docs = parse(INDEX).unwrap();
        assert!(!docs.sections.is_empty());
        for (ix, article) in docs.articles.iter().enumerate() {
            let id = article.id.as_str();
            assert!(docs.section(&article.section).is_some(), "{id}: no section");
            assert!(
                docs.articles[..ix].iter().all(|other| other.id != id),
                "{id} is listed twice"
            );
            assert!(!article.body().trim().is_empty(), "{id} has no body");
        }
        for (id, _) in BODIES {
            assert!(docs.article(id).is_some(), "{id}.md is not in the index");
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
    fn matches_title_and_body_ignoring_case() {
        let docs = parse(INDEX).unwrap();
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

    #[test]
    fn search_needs_every_term_and_quotes_the_body() {
        let docs = parse(INDEX).unwrap();
        assert!(docs.search("  ").is_empty());
        assert!(docs.search("no article mentions zzqx").is_empty());
        let article = docs.articles.first().unwrap();
        let word = article
            .body()
            .split_whitespace()
            .find(|word| word.len() > 6 && word.chars().all(char::is_alphabetic))
            .unwrap()
            .to_lowercase();
        let title = article
            .title
            .split_whitespace()
            .next()
            .unwrap()
            .to_lowercase();
        let hits = docs.search(&format!("{} {title}", word.to_uppercase()));
        let hit = hits
            .iter()
            .find(|hit| hit.article.id == article.id)
            .unwrap();
        let snippet = hit.snippet.to_lowercase();
        assert!(snippet.contains(&word) || snippet.contains(&title));
        for hit in &hits {
            assert!(hit.article.matches(&word) && hit.article.matches(&title));
        }
    }
}
