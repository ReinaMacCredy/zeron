use std::sync::OnceLock;

use serde::Deserialize;

pub mod changelog;
pub mod images;

pub const INDEX: &str = include_str!("../articles/index.toml");

pub const BODIES: &[(&str, &str)] = &[
    (
        "getting-around",
        include_str!("../articles/getting-around.md"),
    ),
    ("new-chat", include_str!("../articles/new-chat.md")),
    ("playbooks", include_str!("../articles/playbooks.md")),
    ("composer", include_str!("../articles/composer.md")),
    (
        "conversation",
        include_str!("../articles/conversation.md"),
    ),
    (
        "commands-and-mentions",
        include_str!("../articles/commands-and-mentions.md"),
    ),
    (
        "models-and-effort",
        include_str!("../articles/models-and-effort.md"),
    ),
    (
        "sessions-sidebar",
        include_str!("../articles/sessions-sidebar.md"),
    ),
    (
        "command-palette",
        include_str!("../articles/command-palette.md"),
    ),
    (
        "keyboard-shortcuts",
        include_str!("../articles/keyboard-shortcuts.md"),
    ),
    ("appearance", include_str!("../articles/appearance.md")),
    (
        "home-widgets",
        include_str!("../articles/home-widgets.md"),
    ),
    ("updates", include_str!("../articles/updates.md")),
    ("providers", include_str!("../articles/providers.md")),
    (
        "agent-profiles",
        include_str!("../articles/agent-profiles.md"),
    ),
    ("routing", include_str!("../articles/routing.md")),
    ("delegation", include_str!("../articles/delegation.md")),
    ("threads", include_str!("../articles/threads.md")),
    ("side-chat", include_str!("../articles/side-chat.md")),
    ("handoff", include_str!("../articles/handoff.md")),
    ("review", include_str!("../articles/review.md")),
    ("compaction", include_str!("../articles/compaction.md")),
    (
        "agent-questions",
        include_str!("../articles/agent-questions.md"),
    ),
    ("skills", include_str!("../articles/skills.md")),
    ("agent-tools", include_str!("../articles/agent-tools.md")),
    ("projects", include_str!("../articles/projects.md")),
    ("right-pane", include_str!("../articles/right-pane.md")),
    ("live-state", include_str!("../articles/live-state.md")),
    ("terminal", include_str!("../articles/terminal.md")),
    ("git", include_str!("../articles/git.md")),
    (
        "pull-requests",
        include_str!("../articles/pull-requests.md"),
    ),
    ("tasks", include_str!("../articles/tasks.md")),
    ("mission", include_str!("../articles/mission.md")),
    ("inbox", include_str!("../articles/inbox.md")),
    ("usage", include_str!("../articles/usage.md")),
    ("split-view", include_str!("../articles/split-view.md")),
    (
        "computer-use",
        include_str!("../articles/computer-use.md"),
    ),
    ("browser", include_str!("../articles/browser.md")),
    ("appshots", include_str!("../articles/appshots.md")),
    ("device-hub", include_str!("../articles/device-hub.md")),
    ("voice", include_str!("../articles/voice.md")),
    (
        "notifications",
        include_str!("../articles/notifications.md"),
    ),
    ("devices", include_str!("../articles/devices.md")),
    (
        "command-line",
        include_str!("../articles/command-line.md"),
    ),
    ("mobile", include_str!("../articles/mobile.md")),
    ("mobile-composer", include_str!("../articles/mobile-composer.md")),
    ("mobile-transcript", include_str!("../articles/mobile-transcript.md")),
    ("mobile-tasks", include_str!("../articles/mobile-tasks.md")),
    ("mobile-inbox", include_str!("../articles/mobile-inbox.md")),
    ("mobile-git", include_str!("../articles/mobile-git.md")),
    ("mobile-settings", include_str!("../articles/mobile-settings.md")),
    ("mobile-devices", include_str!("../articles/mobile-devices.md")),
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
