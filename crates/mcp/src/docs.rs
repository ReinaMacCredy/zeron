use std::fmt::Write as _;

use serde_json::{Value, json};
use zeron_docs::changelog;

pub const TOOL: &str = "docs";

const RECENT_RELEASES: usize = 3;
const MAX_QUERY_CHARS: usize = 200;

pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Read Zeron's own documentation: the in-app Docs shipped with this build. Use it whenever the person asks how something in Zeron itself works (a feature, a page, a setting, a keyboard shortcut, a zeron tool) and answer from the article instead of from memory. `search` with a few words returns the matching article ids, each with a short snippet; `read` with an `id` returns that article's markdown; `list` returns every article by section with its blurb; `whatsNew` returns the release notes newest first, or one release with `version`. Articles link with zeron://docs/<id> (another article) and zeron://settings/<slug> (a Settings page): quote those links so the person can open them, and read a linked article when the answer continues there.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "read", "search", "whatsNew"] },
                "id": { "type": "string", "description": "Article id, for read." },
                "query": { "type": "string", "maxLength": MAX_QUERY_CHARS, "description": "Words to look for, for search. An article matches when it contains every word." },
                "version": { "type": "string", "description": "Release version such as 0.2.114, for whatsNew. Defaults to the newest releases." }
            },
            "required": ["action"],
            "additionalProperties": false
        },
        "annotations": { "title": "Zeron docs", "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
    })
}

pub fn mcp_result(args: &Value) -> Value {
    let (text, is_error) = match run(args) {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn run(args: &Value) -> Result<String, String> {
    let docs = zeron_docs::docs();
    match args["action"].as_str() {
        Some("list") => Ok(list(docs)),
        Some("read") => {
            let id = args["id"]
                .as_str()
                .ok_or("read needs an `id` from list or search")?;
            let id = id.trim().trim_start_matches("zeron://docs/");
            docs.article(id)
                .map(|article| read(docs, article))
                .ok_or_else(|| format!("No article `{id}`. Use search, or list for every id."))
        }
        Some("search") => {
            let query = args["query"].as_str().unwrap_or_default();
            if query.trim().is_empty() {
                return Err("search needs a `query`".into());
            }
            if query.chars().count() > MAX_QUERY_CHARS {
                return Err(format!(
                    "`query` must be at most {MAX_QUERY_CHARS} characters"
                ));
            }
            Ok(search(docs, query))
        }
        Some("whatsNew") => whats_new(args["version"].as_str()),
        _ => Err("action must be list, read, search or whatsNew".into()),
    }
}

fn list(docs: &zeron_docs::Docs) -> String {
    let mut out = format!(
        "Zeron Docs, {} articles. Read one with action read and its id.\n",
        docs.articles.len()
    );
    for section in &docs.sections {
        let _ = write!(out, "\n## {}\n", section.title);
        for article in docs.in_section(section) {
            let _ = writeln!(
                out,
                "- {}: {}. {}",
                article.id, article.title, article.blurb
            );
        }
    }
    out
}

fn read(docs: &zeron_docs::Docs, article: &zeron_docs::Article) -> String {
    let section = docs
        .section(&article.section)
        .map_or(article.section.as_str(), |section| section.title.as_str());
    let mut out = format!(
        "# {}\n\nzeron://docs/{} ({section})\n{}\n",
        article.title, article.id, article.blurb
    );
    if !article.actions.is_empty() {
        let links: Vec<String> = article
            .actions
            .iter()
            .map(|target| format!("zeron://{target}"))
            .collect();
        let _ = writeln!(out, "Opens: {}", links.join(", "));
    }
    let _ = write!(out, "\n{}", article.body().trim_end());
    out
}

fn search(docs: &zeron_docs::Docs, query: &str) -> String {
    let hits = docs.search(query);
    if hits.is_empty() {
        return format!(
            "No article contains every word of \"{}\". Try fewer or other words, or list for every article.",
            query.trim()
        );
    }
    let mut out = format!(
        "{} article(s) match \"{}\". Read one with action read and its id.\n",
        hits.len(),
        query.trim()
    );
    for hit in hits {
        let _ = write!(
            out,
            "\n- {} ({}): {}\n  {}\n",
            hit.article.id, hit.article.title, hit.article.blurb, hit.snippet
        );
    }
    out
}

fn whats_new(version: Option<&str>) -> Result<String, String> {
    let entries = changelog::entries();
    let shown: Vec<&changelog::Entry> = match version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(version) => vec![changelog::entry_for(entries, version).ok_or_else(|| {
            let known: Vec<&str> = entries.iter().map(|entry| entry.version.as_str()).collect();
            format!(
                "No release notes for {version}. Releases with notes: {}.",
                known.join(", ")
            )
        })?],
        None => entries.iter().take(RECENT_RELEASES).collect(),
    };
    let mut out = format!("This build is {}.\n", env!("CARGO_PKG_VERSION"));
    for entry in shown {
        let _ = write!(out, "\n## {} ({})\n", entry.version, entry.date);
        for feature in &entry.features {
            let _ = write!(out, "- {}: {}", feature.title, feature.description);
            if let Some(details) = &feature.details {
                let _ = write!(out, " {details}");
            }
            out.push('\n');
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(result: &Value) -> &str {
        result["content"][0]["text"].as_str().unwrap()
    }

    #[test]
    fn list_read_and_search_answer_from_the_bundled_docs() {
        let docs = zeron_docs::docs();
        let listed = mcp_result(&json!({ "action": "list" }));
        assert_eq!(listed["isError"], false);
        for article in &docs.articles {
            assert!(text(&listed).contains(&format!("- {}: ", article.id)));
        }

        let article = docs
            .articles
            .iter()
            .find(|article| !article.actions.is_empty())
            .unwrap();
        for id in [article.id.clone(), format!("zeron://docs/{}", article.id)] {
            let read = mcp_result(&json!({ "action": "read", "id": id }));
            assert_eq!(read["isError"], false);
            assert!(text(&read).starts_with(&format!("# {}\n", article.title)));
            assert!(text(&read).contains(article.body().trim_end()));
            assert!(text(&read).contains(&format!("zeron://{}", article.actions[0])));
        }
        let missing = mcp_result(&json!({ "action": "read", "id": "no-such-article" }));
        assert_eq!(missing["isError"], true);

        let found = mcp_result(&json!({ "action": "search", "query": article.title }));
        assert!(text(&found).contains(&format!("- {} (", article.id)));
        let none = mcp_result(&json!({ "action": "search", "query": "zzqx nothing" }));
        assert_eq!(none["isError"], false);
        assert!(text(&none).starts_with("No article"));
        assert_eq!(mcp_result(&json!({ "action": "search" }))["isError"], true);
        let long = "word ".repeat(MAX_QUERY_CHARS);
        assert_eq!(
            mcp_result(&json!({ "action": "search", "query": long }))["isError"],
            true
        );
    }

    #[test]
    fn whats_new_shows_the_newest_releases_or_one_by_version() {
        let entries = changelog::entries();
        let newest = mcp_result(&json!({ "action": "whatsNew" }));
        assert!(text(&newest).contains(&format!("## {} (", entries[0].version)));
        let oldest = entries.last().unwrap();
        let one =
            mcp_result(&json!({ "action": "whatsNew", "version": format!("v{}", oldest.version) }));
        assert!(text(&one).contains(&oldest.features[0].title));
        let missing = mcp_result(&json!({ "action": "whatsNew", "version": "0.0.1" }));
        assert_eq!(missing["isError"], true);
    }
}
