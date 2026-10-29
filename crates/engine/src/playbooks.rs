use std::path::{Path, PathBuf};

pub const TRIAGE_ID: &str = "triage";
const APP_DIR: &str = "playbooks";
const PROJECT_DIR: [&str; 2] = [".zeron", "playbooks"];
const TEMPLATE: &str = "---
title: New playbook
question: What should we do?
subtitle:
---

# New playbook

Write the steps the agent should follow. The person's first message comes
after the link to this file, so the first step can use what they wrote.

1. Read what the person asked for.
2. ...
";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybookSource {
    BuiltIn,
    App,
    Project { root: PathBuf },
}

impl PlaybookSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::BuiltIn => "Built in",
            Self::App => "All projects",
            Self::Project { .. } => "Project",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playbook {
    pub id: String,
    pub title: String,
    pub question: String,
    pub subtitle: Option<String>,
    pub source: PlaybookSource,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub title: Option<String>,
    pub question: Option<String>,
    pub subtitle: Option<String>,
}
pub fn parse(text: &str) -> (Frontmatter, &str) {
    let mut front = Frontmatter::default();
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (front, text);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        let line = line.trim_end();
        if line == "---" {
            return (front, &rest[offset..]);
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim();
        let value = (!value.is_empty()).then(|| value.to_string());
        match key.trim() {
            "title" => front.title = value,
            "question" => front.question = value,
            "subtitle" => front.subtitle = value,
            _ => {}
        }
    }
    (Frontmatter::default(), text)
}

pub fn app_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(APP_DIR)
}

pub fn project_dir(root: &Path) -> PathBuf {
    PROJECT_DIR
        .iter()
        .fold(root.to_path_buf(), |dir, part| dir.join(part))
}

pub fn triage() -> Playbook {
    let (front, _) = parse(crate::triage::PLAYBOOK);
    Playbook {
        id: TRIAGE_ID.into(),
        title: front.title.unwrap_or_else(|| "Triage an issue".into()),
        question: front.question.unwrap_or_else(|| "What went wrong?".into()),
        subtitle: Some(format!(
            "Zeron v{} · or run `zeron triage`",
            zeron_update::current_version()
        )),
        source: PlaybookSource::BuiltIn,
        path: None,
    }
}
pub fn list(data_dir: &Path, project_roots: &[PathBuf]) -> Vec<Playbook> {
    let mut all = vec![triage()];
    all.extend(read_dir(&app_dir(data_dir), PlaybookSource::App));
    let mut seen = Vec::new();
    for root in project_roots {
        if seen.contains(root) {
            continue;
        }
        seen.push(root.clone());
        all.extend(read_dir(
            &project_dir(root),
            PlaybookSource::Project { root: root.clone() },
        ));
    }
    all
}

fn read_dir(dir: &Path, source: PlaybookSource) -> Vec<Playbook> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<Playbook> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md") && path.is_file())
        .filter_map(|path| read_file(&path, source.clone()))
        .collect();
    found.sort_by_key(|playbook| playbook.title.to_lowercase());
    found
}

pub fn read_file(path: &Path, source: PlaybookSource) -> Option<Playbook> {
    let text = std::fs::read_to_string(path).ok()?;
    let (front, _) = parse(&text);
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    Some(Playbook {
        id: path.to_string_lossy().into_owned(),
        title: front.title.unwrap_or(stem),
        question: front
            .question
            .unwrap_or_else(|| "What should we work on?".into()),
        subtitle: front.subtitle,
        source,
        path: Some(path.to_path_buf()),
    })
}
pub fn create(dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = (1..)
        .map(|n| match n {
            1 => dir.join("new-playbook.md"),
            n => dir.join(format!("new-playbook-{n}.md")),
        })
        .find(|path| !path.exists())
        .expect("an unused name");
    std::fs::write(&path, TEMPLATE)?;
    Ok(path)
}
pub fn prompt(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!(
        "Read [{name}]({}) and follow it.\n\n",
        path.to_string_lossy()
    )
}
pub fn linked_path(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("Read [")?;
    let (name, rest) = rest.split_once("](")?;
    let (path, _) = rest.split_once(") and follow it.")?;
    (name.ends_with(".md") && path.ends_with(name)).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_and_body_split() {
        let (front, body) = parse("---\ntitle: Ship it\nquestion: \"What ships?\"\n---\n# Body\n");
        assert_eq!(front.title.as_deref(), Some("Ship it"));
        assert_eq!(front.question.as_deref(), Some("What ships?"));
        assert_eq!(front.subtitle, None);
        assert_eq!(body, "# Body\n");
        let (front, body) = parse("# No frontmatter\n");
        assert_eq!(front, Frontmatter::default());
        assert_eq!(body, "# No frontmatter\n");
    }

    #[test]
    fn list_reads_app_and_project_files_after_triage() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let app = create(&app_dir(data.path())).unwrap();
        std::fs::create_dir_all(project_dir(project.path())).unwrap();
        std::fs::write(project_dir(project.path()).join("notes.txt"), "ignored").unwrap();
        std::fs::write(
            project_dir(project.path()).join("release.md"),
            "---\ntitle: Cut a release\nquestion: Which version?\n---\nSteps",
        )
        .unwrap();
        let roots = vec![project.path().to_path_buf(), project.path().to_path_buf()];
        let all = list(data.path(), &roots);
        let titles: Vec<_> = all.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["Triage an issue", "New playbook", "Cut a release"]);
        assert_eq!(all[0].question, "What went wrong?");
        assert_eq!(all[1].path.as_deref(), Some(app.as_path()));
        assert_eq!(all[2].question, "Which version?");
        assert!(matches!(all[2].source, PlaybookSource::Project { .. }));
    }

    #[test]
    fn prompt_links_the_file_and_round_trips() {
        let path = Path::new("/tmp/t/triage-playbook.md");
        let text = prompt(path);
        assert_eq!(
            text,
            "Read [triage-playbook.md](/tmp/t/triage-playbook.md) and follow it.\n\n"
        );
        assert_eq!(
            linked_path(&format!("{text}It crashes")),
            Some("/tmp/t/triage-playbook.md")
        );
        assert_eq!(linked_path("Read the docs"), None);
    }
}
