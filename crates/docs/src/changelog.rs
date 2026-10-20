use std::sync::OnceLock;

use serde::Deserialize;
use zeron_update::version_newer;

pub const SOURCE: &str = include_str!("../../ui/whats_new.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Changelog {
    entry: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub version: String,
    pub date: String,
    #[serde(default)]
    pub hero: Option<String>,
    pub features: Vec<Feature>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
}

pub fn parse(source: &str) -> Result<Vec<Entry>, toml::de::Error> {
    let mut entries = toml::from_str::<Changelog>(source)?.entry;
    entries.sort_by(|a, b| {
        if version_newer(&a.version, &b.version) {
            std::cmp::Ordering::Less
        } else if version_newer(&b.version, &a.version) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    Ok(entries)
}

pub fn entries() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        parse(SOURCE).unwrap_or_else(|err| {
            tracing::warn!(error = %err, "whats_new.toml does not parse");
            Vec::new()
        })
    })
}

pub fn entry_for<'a>(entries: &'a [Entry], version: &str) -> Option<&'a Entry> {
    let version = version.trim_start_matches('v');
    entries
        .iter()
        .find(|entry| entry.version.trim_start_matches('v') == version)
}
