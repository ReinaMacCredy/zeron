use zeron_docs::changelog;

#[derive(Debug, Clone, uniffi::Record)]
pub struct ChangelogEntry {
    pub version: String,
    pub date: String,
    pub features: Vec<ChangelogFeature>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ChangelogFeature {
    pub title: String,
    pub description: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DocsSection {
    pub title: String,
    pub articles: Vec<DocsArticle>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DocsArticle {
    pub id: String,
    pub title: String,
    pub blurb: String,
}

fn entry(entry: &changelog::Entry) -> ChangelogEntry {
    ChangelogEntry {
        version: entry.version.clone(),
        date: entry.date.clone(),
        features: entry
            .features
            .iter()
            .map(|f| ChangelogFeature {
                title: f.title.clone(),
                description: f.description.clone(),
                details: f.details.clone(),
            })
            .collect(),
    }
}

#[uniffi::export]
pub fn whats_new() -> Option<ChangelogEntry> {
    let entries = changelog::entries();
    changelog::entry_for(entries, env!("CARGO_PKG_VERSION"))
        .or(entries.first())
        .map(entry)
}

#[uniffi::export]
pub fn changelog() -> Vec<ChangelogEntry> {
    changelog::entries().iter().map(entry).collect()
}

#[uniffi::export]
pub fn docs_sections() -> Vec<DocsSection> {
    let docs = zeron_docs::docs();
    docs.sections
        .iter()
        .map(|section| DocsSection {
            title: section.title.clone(),
            articles: docs
                .in_section(section)
                .map(|a| DocsArticle {
                    id: a.id.clone(),
                    title: a.title.clone(),
                    blurb: a.blurb.clone(),
                })
                .collect(),
        })
        .filter(|section| !section.articles.is_empty())
        .collect()
}

/// An article's markdown without its screenshots, which ship only with the
/// desktop app.
#[uniffi::export]
pub fn docs_body(id: String) -> Option<String> {
    let body = zeron_docs::article(&id)?.body();
    Some(
        body.lines()
            .filter(|line| !line.trim_start().starts_with("!["))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[uniffi::export]
pub fn docs_link(url: String) -> Option<String> {
    let id = url.strip_prefix("zeron://docs/")?;
    let id = id.split(['#', '?']).next().unwrap_or(id);
    zeron_docs::article(id).map(|a| a.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docs_reach_every_article_and_links_resolve() {
        let sections = docs_sections();
        let count: usize = sections.iter().map(|s| s.articles.len()).sum();
        assert_eq!(count, zeron_docs::docs().articles.len());
        let id = &sections[0].articles[0].id;
        assert_eq!(
            docs_link(format!("zeron://docs/{id}#top")).as_ref(),
            Some(id)
        );
        assert_eq!(docs_link("zeron://settings/tools".into()), None);
        assert!(!docs_body("agent-profiles".into()).unwrap().contains("!["));
    }

    #[test]
    fn whats_new_has_features() {
        assert!(!whats_new().unwrap().features.is_empty());
        assert!(changelog().len() >= 1);
    }
}
