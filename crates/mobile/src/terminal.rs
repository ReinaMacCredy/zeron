use zeron_proto::{
    ProjectAction, ProjectActionIcon, TerminalColor, TerminalInfo, TerminalRun, TerminalScreen,
    terminal_extended_rgb,
};
use zeron_theme::{Color, TerminalPalette, builtin_registry};

const DIM_ALPHA: f32 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RunState {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TerminalTab {
    pub id: String,
    pub title: String,
    pub state: RunState,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TerminalSpan {
    pub text: String,
    /// 0xRRGGBBAA.
    pub fg: u32,
    pub bg: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TerminalView {
    pub tab: TerminalTab,
    pub lines: Vec<Vec<TerminalSpan>>,
    pub background: u32,
    pub foreground: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProjectActionItem {
    pub id: String,
    pub name: String,
    pub command: String,
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProjectActions {
    pub actions: Vec<ProjectActionItem>,
    pub issue: Option<String>,
}

pub fn run_state(exit_code: Option<i32>) -> RunState {
    match exit_code {
        None => RunState::Running,
        Some(0) => RunState::Done,
        Some(_) => RunState::Failed,
    }
}

pub fn run_status(exit_code: Option<i32>) -> String {
    match exit_code {
        None => "Running".into(),
        Some(0) => "Done".into(),
        Some(code) => format!("Failed (exit {code})"),
    }
}

pub fn tab(info: &TerminalInfo) -> TerminalTab {
    TerminalTab {
        id: info.id.clone(),
        title: info
            .title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| info.shell.clone()),
        state: run_state(info.exit_code),
        status: run_status(info.exit_code),
    }
}

pub fn action_item(action: &ProjectAction) -> ProjectActionItem {
    let icon = match action.icon {
        ProjectActionIcon::Play => "play.fill",
        ProjectActionIcon::Test => "checkmark.seal",
        ProjectActionIcon::Lint => "wand.and.stars",
        ProjectActionIcon::Configure => "gearshape",
        ProjectActionIcon::Build => "hammer",
        ProjectActionIcon::Debug => "ladybug",
    };
    ProjectActionItem {
        id: action.id.clone(),
        name: action.name.clone(),
        command: action.command.clone(),
        icon: icon.into(),
    }
}

fn palette(dark: bool) -> &'static TerminalPalette {
    let id = if dark { "zeron-dark" } else { "zeron-light" };
    &builtin_registry()
        .variant(id)
        .expect("the Zeron terminal palette is built in")
        .terminal
}

fn rgba(color: Color, alpha: f32) -> u32 {
    let a = (color.a as f32 * alpha).round() as u32;
    (color.r as u32) << 24 | (color.g as u32) << 16 | (color.b as u32) << 8 | a
}

fn resolve(color: TerminalColor, palette: &TerminalPalette, dark: bool) -> Color {
    match color {
        TerminalColor::Rgb(r, g, b) => Color { r, g, b, a: 255 },
        TerminalColor::Indexed(index) => match terminal_extended_rgb(dark, index) {
            Some((r, g, b)) => Color { r, g, b, a: 255 },
            None => palette.ansi[index as usize],
        },
    }
}

fn span(run: &TerminalRun, palette: &TerminalPalette, dark: bool) -> TerminalSpan {
    let fg = run.fg.map(|c| resolve(c, palette, dark));
    let bg = run.bg.map(|c| resolve(c, palette, dark));
    let (fg, bg) = if run.inverse {
        (
            bg.unwrap_or(palette.background),
            Some(fg.unwrap_or(palette.foreground)),
        )
    } else {
        (fg.unwrap_or(palette.foreground), bg)
    };
    TerminalSpan {
        text: run.text.clone(),
        fg: rgba(fg, if run.dim { DIM_ALPHA } else { 1.0 }),
        bg: bg.map(|bg| rgba(bg, 1.0)),
        bold: run.bold,
        italic: run.italic,
        underline: run.underline,
    }
}

pub fn view(screen: &TerminalScreen, dark: bool) -> TerminalView {
    let palette = palette(dark);
    let lines = if screen.styled_lines.is_empty() {
        screen
            .lines
            .iter()
            .map(|line| {
                vec![span(
                    &TerminalRun {
                        text: line.clone(),
                        ..TerminalRun::default()
                    },
                    palette,
                    dark,
                )]
            })
            .collect()
    } else {
        screen
            .styled_lines
            .iter()
            .map(|line| line.iter().map(|run| span(run, palette, dark)).collect())
            .collect()
    };
    TerminalView {
        tab: tab(&screen.terminal),
        lines,
        background: rgba(palette.background, 1.0),
        foreground: rgba(palette.foreground, 1.0),
        truncated: screen.truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(styled_lines: Vec<Vec<TerminalRun>>) -> TerminalScreen {
        TerminalScreen {
            terminal: TerminalInfo {
                id: "t".into(),
                cwd: "/".into(),
                shell: "zsh".into(),
                title: None,
                exit_code: Some(1),
                selection: None,
            },
            cols: 80,
            rows: 24,
            alt_screen: false,
            cursor_row: 0,
            cursor_col: 0,
            lines: vec!["ok plain".into()],
            truncated: false,
            styled_lines,
        }
    }

    #[test]
    fn styled_runs_paint_in_the_desktop_palette() {
        let dark = palette(true);
        let view = view(
            &screen(vec![vec![
                TerminalRun {
                    text: "ok".into(),
                    fg: Some(TerminalColor::Indexed(2)),
                    bold: true,
                    ..TerminalRun::default()
                },
                TerminalRun {
                    text: " hint".into(),
                    dim: true,
                    ..TerminalRun::default()
                },
                TerminalRun {
                    text: "sel".into(),
                    inverse: true,
                    ..TerminalRun::default()
                },
                TerminalRun {
                    text: "red".into(),
                    fg: Some(TerminalColor::Indexed(196)),
                    ..TerminalRun::default()
                },
            ]]),
            true,
        );
        let line = &view.lines[0];
        assert_eq!(line[0].fg, rgba(dark.ansi[2], 1.0));
        assert!(line[0].bold);
        assert_eq!(
            line[1].fg & 0xff,
            (dark.foreground.a as f32 * DIM_ALPHA).round() as u32
        );
        assert_eq!(line[2].fg, rgba(dark.background, 1.0));
        assert_eq!(line[2].bg, Some(rgba(dark.foreground, 1.0)));
        assert_eq!(line[3].fg, 0xff0000ff);
        assert_eq!(view.tab.title, "zsh");
        assert_eq!(
            (view.tab.state, view.tab.status.as_str()),
            (RunState::Failed, "Failed (exit 1)")
        );
    }

    #[test]
    fn an_old_host_without_runs_paints_plain_lines() {
        let view = view(&screen(Vec::new()), false);
        assert_eq!(view.lines.len(), 1);
        assert_eq!(view.lines[0][0].text, "ok plain");
        assert_eq!(view.lines[0][0].fg, rgba(palette(false).foreground, 1.0));
    }
}
