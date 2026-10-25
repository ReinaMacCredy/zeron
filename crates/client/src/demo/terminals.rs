use std::collections::HashMap;

use serde_json::Value;
use zeron_proto::{
    ProjectAction, ProjectActionIcon, ProjectActionRun, ProjectActionsSnapshot, TerminalColor,
    TerminalInfo, TerminalRun, TerminalScreen, TerminalSession,
};
use zeron_rpc::methods as m;

use crate::error::{ClientError, Result};

const ACTION_RUN_MS: i64 = 1_500;

struct Terminal {
    info: TerminalInfo,
    lines: Vec<Vec<TerminalRun>>,
    started_ms: i64,
    exit_after: Option<i32>,
}

pub(super) struct HostTerminals {
    by_chat: HashMap<String, Vec<Terminal>>,
    seq: u32,
}

fn run(text: &str, fg: Option<u8>) -> TerminalRun {
    TerminalRun {
        text: text.into(),
        fg: fg.map(TerminalColor::Indexed),
        ..TerminalRun::default()
    }
}

fn bold(text: &str, fg: u8) -> TerminalRun {
    TerminalRun {
        bold: true,
        ..run(text, Some(fg))
    }
}

fn dim(text: &str) -> TerminalRun {
    TerminalRun {
        dim: true,
        ..run(text, None)
    }
}

fn prompt(command: &str) -> Vec<TerminalRun> {
    vec![bold("~/zeron", 4), run(" $ ", None), run(command, None)]
}

fn shell_lines() -> Vec<Vec<TerminalRun>> {
    vec![
        prompt("cargo test -p zeron-proto computer"),
        vec![
            bold("   Compiling", 2),
            run(
                " zeron-proto v0.2.114 (/Users/dev/zeron/crates/proto)",
                None,
            ),
        ],
        vec![bold("    Finished", 2), dim(" `test` profile in 4.21s")],
        vec![run("running 3 tests", None)],
        vec![
            run("test computer::cards::tests::left_open ... ", None),
            run("ok", Some(2)),
        ],
        vec![
            run("test computer::cards::tests::risk_lists ... ", None),
            run("ok", Some(2)),
        ],
        vec![
            run("test computer::presentation::tests::js_badge ... ", None),
            bold("FAILED", 1),
        ],
        vec![
            run("warning", Some(3)),
            run(": unused variable `app` ", None),
            TerminalRun {
                text: "--> cards.rs:42:9".into(),
                fg: Some(TerminalColor::Rgb(0x6c, 0x9e, 0xf8)),
                underline: true,
                ..TerminalRun::default()
            },
        ],
        vec![
            run("test result: ", None),
            bold("FAILED", 1),
            run(". 2 passed; 1 failed", None),
        ],
        prompt(""),
    ]
}

fn action_lines(action: &ProjectAction, exit: Option<i32>) -> Vec<Vec<TerminalRun>> {
    let mut lines = vec![prompt(&action.command)];
    match exit {
        None => lines.push(vec![dim("  waiting for changes...")]),
        Some(0) => lines.push(vec![bold("✓", 2), run(" done", None)]),
        Some(code) => lines.push(vec![
            bold("✗", 1),
            run(&format!(" exited with {code}"), None),
        ]),
    }
    lines
}

fn actions() -> Vec<(ProjectAction, Option<i32>)> {
    let action = |id: &str, name: &str, command: &str, icon| ProjectAction {
        id: id.into(),
        name: name.into(),
        command: command.into(),
        icon,
        run_on_worktree_create: false,
    };
    vec![
        (
            action(
                "act-dev",
                "Dev server",
                "bun run dev",
                ProjectActionIcon::Play,
            ),
            None,
        ),
        (
            action(
                "act-test",
                "Test",
                "cargo test -p zeron-proto",
                ProjectActionIcon::Test,
            ),
            Some(0),
        ),
        (
            action(
                "act-lint",
                "Lint",
                "cargo clippy --workspace",
                ProjectActionIcon::Lint,
            ),
            Some(1),
        ),
    ]
}

fn plain(line: &[TerminalRun]) -> String {
    line.iter().map(|run| run.text.as_str()).collect()
}

impl HostTerminals {
    pub(super) fn new(now: i64) -> Self {
        let shell = Terminal {
            info: TerminalInfo {
                id: "term-demo-1".into(),
                cwd: "/Users/dev/zeron".into(),
                shell: "zsh".into(),
                title: None,
                exit_code: None,
                selection: None,
            },
            lines: shell_lines(),
            started_ms: now,
            exit_after: None,
        };
        Self {
            by_chat: HashMap::from([("chat-computer".to_owned(), vec![shell])]),
            seq: 1,
        }
    }

    pub(super) fn handles(method: &str) -> bool {
        matches!(
            method,
            m::LIST_TERMINALS | m::READ_TERMINAL | m::LIST_PROJECT_ACTIONS | m::RUN_PROJECT_ACTION
        )
    }

    fn settle(&mut self, now: i64) {
        for terminal in self.by_chat.values_mut().flatten() {
            if terminal.info.exit_code.is_none()
                && let Some(code) = terminal.exit_after
                && now - terminal.started_ms >= ACTION_RUN_MS
            {
                terminal.info.exit_code = Some(code);
                if let Some((action, _)) = actions()
                    .into_iter()
                    .find(|(action, _)| terminal.info.title.as_deref() == Some(&action.name))
                {
                    terminal.lines = action_lines(&action, Some(code));
                }
            }
        }
    }

    pub(super) fn call(&mut self, method: &str, params: &Value, now: i64) -> Result<Value> {
        self.settle(now);
        let chat_id = params["chatId"].as_str().unwrap_or_default().to_owned();
        let value = match method {
            m::LIST_TERMINALS => {
                let mut infos: Vec<TerminalInfo> = self
                    .by_chat
                    .get(&chat_id)
                    .into_iter()
                    .flatten()
                    .map(|terminal| terminal.info.clone())
                    .collect();
                infos.reverse();
                serde_json::to_value(infos)
            }
            m::READ_TERMINAL => {
                let id = params["terminalId"].as_str().unwrap_or_default();
                let terminal = self
                    .by_chat
                    .get(&chat_id)
                    .into_iter()
                    .flatten()
                    .find(|terminal| terminal.info.id == id)
                    .ok_or_else(|| {
                        ClientError::HostError("Terminal does not belong to this chat".into())
                    })?;
                let styled = params["styled"].as_bool().unwrap_or(false);
                serde_json::to_value(TerminalScreen {
                    terminal: terminal.info.clone(),
                    cols: 80,
                    rows: 24,
                    alt_screen: false,
                    cursor_row: terminal.lines.len().saturating_sub(1),
                    cursor_col: plain(terminal.lines.last().map_or(&[][..], Vec::as_slice))
                        .chars()
                        .count(),
                    lines: terminal.lines.iter().map(|line| plain(line)).collect(),
                    truncated: false,
                    styled_lines: if styled {
                        terminal.lines.clone()
                    } else {
                        Vec::new()
                    },
                })
            }
            m::LIST_PROJECT_ACTIONS => serde_json::to_value(ProjectActionsSnapshot {
                space_id: params["spaceId"].as_str().unwrap_or_default().into(),
                actions: actions().into_iter().map(|(action, _)| action).collect(),
                importable_actions: Vec::new(),
                project_file_issue: None,
            }),
            m::RUN_PROJECT_ACTION => {
                let action_id = params["actionId"].as_str().unwrap_or_default();
                let (action, exit) = actions()
                    .into_iter()
                    .find(|(action, _)| action.id == action_id)
                    .ok_or_else(|| ClientError::HostError("Project action not found".into()))?;
                self.seq += 1;
                let session = TerminalSession {
                    id: format!("term-demo-{}", self.seq),
                    cwd: "/Users/dev/zeron".into(),
                    shell: "zsh".into(),
                    host_replies: true,
                };
                self.by_chat.entry(chat_id).or_default().push(Terminal {
                    info: TerminalInfo {
                        id: session.id.clone(),
                        cwd: session.cwd.clone(),
                        shell: session.shell.clone(),
                        title: Some(action.name.clone()),
                        exit_code: None,
                        selection: None,
                    },
                    lines: action_lines(&action, None),
                    started_ms: now,
                    exit_after: exit,
                });
                serde_json::to_value(ProjectActionRun {
                    action_id: action.id,
                    action_name: action.name,
                    terminal: session,
                })
            }
            other => return Err(ClientError::Unsupported(other.to_owned())),
        };
        value.map_err(|e| ClientError::Internal(e.to_string()))
    }
}
