use std::collections::BTreeMap;

use crate::app_commands::{ClaudeCardCommand, TERMINAL_ONLY_COMMANDS};
use crate::claude_config::{COMMAND_NOTE_PREFIX, CommandCard, LocalCommandOutput};

pub const NO_SESSION: &str =
    "No turn has run in this chat yet, so these numbers are for a new session.";
pub const LOST_SESSION: &str =
    "The chat's Claude session was not found, so these numbers are for a new session.";
pub const FORKED: &str = "Read from a copy of this chat's saved session; Zeron's own tool server and per-run settings are not counted.";
pub const LOCAL_FOOTNOTE: &str = "Ran locally in Claude Code · no tokens used";
pub const MODEL_FOOTNOTE: &str = "Picking a model here changes this chat's model chip.";
pub const MCP_FOOTNOTE: &str = "Names and status from Claude Code's init for this folder; Zeron's own tool server is not listed.";
pub const MCP_EMPTY: &str = "No MCP servers are configured for this folder.";
pub const GROUP_ROWS: usize = 5;
pub const BAR_SCALE: f32 = 6.0;

pub struct SideRun {
    pub name: &'static str,
    pub connect_mcp: bool,
    pub live: bool,
}

pub fn side_command(name: &str) -> Option<SideRun> {
    let (name, connect_mcp, live) = match name.trim_start_matches('/') {
        "context" => ("context", true, true),
        "usage" | "cost" => ("usage", false, false),
        "mcp" => ("mcp", true, false),
        _ => return None,
    };
    Some(SideRun {
        name,
        connect_mcp,
        live,
    })
}

pub fn side_run(command: &ClaudeCardCommand) -> Option<SideRun> {
    match command {
        ClaudeCardCommand::Context => side_command("context"),
        ClaudeCardCommand::Usage => side_command("usage"),
        ClaudeCardCommand::Mcp => side_command("mcp"),
        ClaudeCardCommand::Model(_) | ClaudeCardCommand::TerminalOnly(_) => None,
    }
}

pub fn pending_card(name: &str, now_ms: i64) -> CommandCard {
    CommandCard {
        command: format!("/{name}"),
        pending: true,
        started_at: Some(now_ms),
        ..Default::default()
    }
}

pub fn side_headline(name: &str) -> String {
    format!("{COMMAND_NOTE_PREFIX}{name}")
}

pub fn lost_session(reply: &Result<LocalCommandOutput, String>) -> bool {
    reply
        .as_ref()
        .is_err_and(|e| e.contains("No conversation found"))
}

pub fn output_card(
    name: &str,
    reply: Result<LocalCommandOutput, String>,
    note: Option<&str>,
) -> CommandCard {
    match reply {
        Ok(out) => CommandCard {
            command: format!("/{name}"),
            mcp_servers: (name == "mcp")
                .then(|| out.init.as_ref()?.get("mcp_servers").cloned())
                .flatten(),
            text: out.text,
            note: note.map(str::to_owned),
            ..Default::default()
        },
        Err(error) => CommandCard {
            command: format!("/{name}"),
            error: Some(error),
            ..Default::default()
        },
    }
}

pub fn terminal_card(name: &str) -> (String, CommandCard) {
    (
        format!("{COMMAND_NOTE_PREFIX}{name} · terminal only"),
        CommandCard {
            command: format!("/{name}"),
            ..Default::default()
        },
    )
}

pub fn model_match<'a>(models: &'a [(String, String)], arg: &str) -> Option<&'a (String, String)> {
    let wanted = arg.to_lowercase();
    models
        .iter()
        .find(|(id, label)| id.to_lowercase() == wanted || label.to_lowercase() == wanted)
        .or_else(|| {
            models.iter().find(|(id, label)| {
                id.to_lowercase().contains(&wanted) || label.to_lowercase().contains(&wanted)
            })
        })
}

pub fn model_card(
    models: Vec<(String, String)>,
    model: Option<String>,
    note: Option<String>,
) -> CommandCard {
    CommandCard {
        command: "/model".into(),
        models,
        model,
        note,
        ..Default::default()
    }
}

pub fn switched_note(label: &str) -> String {
    format!("Switched this chat to {label}.")
}

pub fn no_match_note(arg: &str) -> String {
    format!("No model matches \u{201c}{arg}\u{201d}.")
}

pub fn model_headline(card: &CommandCard) -> String {
    format!(
        "{COMMAND_NOTE_PREFIX}model{}",
        card.model
            .as_deref()
            .map(|m| format!(" · {m}"))
            .unwrap_or_default()
    )
}

pub fn terminal_only(card: &CommandCard) -> Option<&str> {
    (card.text.is_empty() && card.error.is_none() && card.models.is_empty())
        .then(|| card.command.strip_prefix('/'))
        .flatten()
        .filter(|name| TERMINAL_ONLY_COMMANDS.contains(name))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalAction {
    Status,
    Permissions,
    Help,
    Hooks,
    Export,
}

pub fn terminal_action(name: &str) -> Option<(&'static str, TerminalAction)> {
    match name {
        "status" => Some(("Open Status", TerminalAction::Status)),
        "permissions" => Some(("Open permission mode", TerminalAction::Permissions)),
        "help" => Some(("Open Docs", TerminalAction::Help)),
        "hooks" => Some(("Open settings.json", TerminalAction::Hooks)),
        "export" => Some(("Export chat…", TerminalAction::Export)),
        _ => None,
    }
}

pub fn selected_model(models: &[(String, String)], current: Option<&str>) -> Option<usize> {
    let current = current?;
    let alias = current.to_lowercase();
    models.iter().position(|(id, _)| id == current).or_else(|| {
        models
            .iter()
            .position(|(_, label)| label.to_lowercase().starts_with(&alias))
    })
}

pub fn mcp_servers(card: &CommandCard) -> Option<Vec<(String, String)>> {
    Some(
        card.mcp_servers
            .as_ref()?
            .as_array()?
            .iter()
            .filter_map(|s| {
                Some((
                    s.get("name")?.as_str()?.to_string(),
                    s.get("status")?.as_str().unwrap_or("unknown").to_string(),
                ))
            })
            .collect(),
    )
}

pub fn usage_subtitle(note: Option<&str>) -> String {
    note.map(|n| {
        if n.contains("subscription") {
            "Claude Code · subscription".to_string()
        } else {
            format!("Claude Code · {n}")
        }
    })
    .unwrap_or_else(|| "Claude Code".into())
}

#[derive(Debug, PartialEq)]
pub struct Group {
    pub title: String,
    pub total: u64,
    pub rows: Vec<(String, u64)>,
}

#[derive(Debug, PartialEq)]
pub struct ContextReport {
    pub model: String,
    pub used: u64,
    pub total: u64,
    pub categories: Vec<(String, u64)>,
    pub free: u64,
    pub buffer: u64,
    pub groups: Vec<Group>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Series {
    System,
    Mcp,
    Agents,
    Memory,
    Skills,
    Messages,
    Other,
}

pub fn series(name: &str) -> Series {
    match name {
        n if n.starts_with("System") => Series::System,
        n if n.starts_with("MCP") => Series::Mcp,
        "Custom agents" => Series::Agents,
        "Memory files" => Series::Memory,
        "Skills" => Series::Skills,
        "Messages" => Series::Messages,
        _ => Series::Other,
    }
}

impl ContextReport {
    pub fn pill(&self) -> String {
        format!(
            "{} / {} · {}",
            crate::usage::format_tokens(self.used),
            crate::usage::format_tokens(self.total),
            percent(self.used, self.total)
        )
    }

    // Drawn up to BAR_SCALE times so the used part reads; free space is not drawn.
    pub fn bar_scale(&self) -> f32 {
        let total = self.total.max(1);
        let drawn = self.used + self.buffer;
        (0.98 * total as f32 / drawn.max(1) as f32).min(BAR_SCALE)
    }

    pub fn bar_fraction(&self, n: u64) -> f32 {
        (n as f32 / self.total.max(1) as f32 * self.bar_scale()).max(0.004)
    }

    pub fn bar_caption(&self) -> String {
        format!(
            "Bar drawn {:.0}× so the used part reads; free space is not drawn.",
            self.bar_scale()
        )
    }

    pub fn tip(&self) -> Option<String> {
        let g = self.groups.iter().max_by_key(|g| g.total)?;
        let top = g.rows.first()?;
        Some(format!(
            "{} take {}. Largest: {} {}.",
            g.title,
            crate::usage::format_tokens(g.total),
            top.0,
            crate::usage::format_tokens(top.1)
        ))
    }
}

pub fn tokens(cell: &str) -> Option<u64> {
    let cell = cell.trim().trim_start_matches('~').to_lowercase();
    let (number, scale) = match cell.chars().last()? {
        'k' => (&cell[..cell.len() - 1], 1_000.0),
        'm' => (&cell[..cell.len() - 1], 1_000_000.0),
        _ => (cell.as_str(), 1.0),
    };
    let value: f64 = number.trim().replace(',', "").parse().ok()?;
    Some((value * scale).round() as u64)
}

fn cells(line: &str) -> Option<Vec<String>> {
    let line = line.trim();
    let inner = line.strip_prefix('|')?.strip_suffix('|')?;
    if inner.chars().all(|c| matches!(c, '-' | '|' | ' ' | ':')) {
        return None;
    }
    Some(inner.split('|').map(|c| c.trim().to_string()).collect())
}

fn sections(text: &str) -> Vec<(String, Vec<Vec<String>>)> {
    let mut out: Vec<(String, Vec<Vec<String>>)> = Vec::new();
    let mut header_seen = false;
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("### ") {
            out.push((title.trim().to_string(), Vec::new()));
            header_seen = false;
        } else if let Some(row) = cells(line)
            && let Some((_, rows)) = out.last_mut()
        {
            if header_seen {
                rows.push(row);
            } else {
                header_seen = true;
            }
        }
    }
    out
}

pub fn group_of(title: &str, rows: &[Vec<String>]) -> Option<Group> {
    let last = |row: &Vec<String>| row.last().and_then(|c| tokens(c));
    let mut items: Vec<(String, u64)> = match title {
        "MCP Tools" => {
            let mut servers: BTreeMap<String, u64> = BTreeMap::new();
            for row in rows {
                if let (Some(server), Some(n)) = (row.get(1), last(row)) {
                    *servers.entry(server.clone()).or_default() += n;
                }
            }
            servers.into_iter().collect()
        }
        "Memory Files" => rows
            .iter()
            .filter_map(|row| {
                let path = row.get(1)?;
                let name = path.rsplit('/').next().unwrap_or(path);
                Some((format!("{name} · {}", row.first()?), last(row)?))
            })
            .collect(),
        "Custom Agents" => rows
            .iter()
            .filter_map(|row| Some((format!("{} · {}", row.first()?, row.get(1)?), last(row)?)))
            .collect(),
        "Skills" => rows
            .iter()
            .filter_map(|row| Some((row.first()?.clone(), last(row)?)))
            .collect(),
        _ => return None,
    };
    if items.is_empty() {
        return None;
    }
    items.sort_by(|a, b| b.1.cmp(&a.1));
    let total = items.iter().map(|(_, n)| n).sum();
    if items.len() > GROUP_ROWS {
        let rest: Vec<_> = items.split_off(GROUP_ROWS - 1);
        let label = match title {
            "MCP Tools" => "servers",
            "Memory Files" => "files",
            "Custom Agents" => "agents",
            _ => "skills",
        };
        items.push((
            format!("+ {} {label}", rest.len()),
            rest.iter().map(|(_, n)| n).sum(),
        ));
    }
    let title = match title {
        "MCP Tools" => "MCP tools",
        "Memory Files" => "Memory files",
        "Custom Agents" => "Custom agents",
        other => other,
    };
    Some(Group {
        title: title.to_string(),
        total,
        rows: items,
    })
}

pub fn parse_context(text: &str) -> Option<ContextReport> {
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(&format!("**{name}:**")))
            .map(|v| v.trim().to_string())
    };
    let tokens_line = field("Tokens")?;
    let (used, rest) = tokens_line.split_once('/')?;
    let total = rest.split_whitespace().next().and_then(tokens)?;
    let used = tokens(used)?;
    let mut report = ContextReport {
        model: field("Model").unwrap_or_default(),
        used,
        total,
        categories: Vec::new(),
        free: 0,
        buffer: 0,
        groups: Vec::new(),
    };
    for (title, rows) in sections(text) {
        if title.starts_with("Estimated usage by category") {
            for row in rows {
                let (Some(name), Some(n)) = (row.first(), row.get(1).and_then(|c| tokens(c)))
                else {
                    continue;
                };
                match name.as_str() {
                    "Free space" => report.free = n,
                    "Autocompact buffer" => report.buffer = n,
                    _ => report.categories.push((name.clone(), n)),
                }
            }
        } else if let Some(group) = group_of(&title, &rows) {
            report.groups.push(group);
        }
    }
    (!report.categories.is_empty()).then_some(report)
}

pub fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "0%".into();
    }
    format!("{:.1}%", part as f64 / whole as f64 * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTEXT: &str = "## Context Usage\n\n**Model:** claude-haiku-5-5  \n**Tokens:** 59.9k / 1m (6%)\n\n### Estimated usage by category\n\n| Category | Tokens | Percentage |\n|----------|--------|------------|\n| System prompt | 3.3k | 0.3% |\n| Memory files | 25.1k | 2.5% |\n| Messages | 18.8k | 1.9% |\n| Free space | 907.1k | 90.7% |\n| Autocompact buffer | 33k | 3.3% |\n\n### MCP Tools\n\n| Tool | Server | Tokens |\n|------|--------|--------|\n| mcp__xcode__A | xcode | 1.4k |\n| mcp__xcode__B | xcode | 1.2k |\n| mcp__semble__search | semble | 519 |\n\n### Memory Files\n\n| Type | Path | Tokens |\n|------|------|--------|\n| User | /Users/r/.claude/CLAUDE.md | 3.5k |\n| Project | /Users/r/AGENTS.md | 3.4k |\n";

    #[test]
    fn context_markdown_becomes_categories_and_groups() {
        let report = parse_context(CONTEXT).unwrap();
        assert_eq!(report.model, "claude-haiku-5-5");
        assert_eq!((report.used, report.total), (59_900, 1_000_000));
        assert_eq!(report.free, 907_100);
        assert_eq!(report.buffer, 33_000);
        assert_eq!(
            report.categories,
            [
                ("System prompt".to_string(), 3_300),
                ("Memory files".to_string(), 25_100),
                ("Messages".to_string(), 18_800)
            ]
        );
        assert_eq!(report.groups[0].title, "MCP tools");
        assert_eq!(
            report.groups[0].rows,
            [("xcode".to_string(), 2_600), ("semble".to_string(), 519)]
        );
        assert_eq!(
            report.groups[1].rows[0],
            ("CLAUDE.md · User".to_string(), 3_500)
        );
        assert!(parse_context("Not enough messages to compact.").is_none());
    }

    #[test]
    fn long_groups_fold_their_tail_into_one_row() {
        let rows: Vec<Vec<String>> = (1..=8)
            .map(|n| vec![format!("skill-{n}"), "User".into(), format!("{}", n * 100)])
            .collect();
        let group = group_of("Skills", &rows).unwrap();
        assert_eq!(group.rows.len(), GROUP_ROWS);
        assert_eq!(
            group.rows.last().unwrap(),
            &("+ 4 skills".to_string(), 1_000)
        );
        assert_eq!(group.total, 3_600);
    }
}
