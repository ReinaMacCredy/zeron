use std::{
    ffi::OsString,
    fs::File,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use super::synctex::normalized;

const MAX_PROBLEMS: usize = 40;
const HEADER_LINES: usize = 400;
const PLACES: [&str; 4] = ["/Library/TeX/texbin", "/opt/homebrew/bin", "/usr/local/bin", "/usr/texbin"];
const SOURCES: [&str; 18] = ["tex", "sty", "cls", "ltx", "bib", "bbl", "aux", "def", "cfg", "clo", "dtx", "fd", "lbx", "bbx", "cbx", "toc", "tikz", "ins"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Program {
    Pdf,
    Xe,
    Lua,
}

impl Program {
    fn engine(self) -> &'static str {
        match self {
            Program::Pdf => "pdflatex",
            Program::Xe => "xelatex",
            Program::Lua => "lualatex",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tool {
    Latexmk(PathBuf),
    Engine(PathBuf),
    Tectonic(PathBuf),
}

impl Tool {
    pub fn name(&self) -> &'static str {
        match self {
            Tool::Latexmk(_) => "latexmk",
            Tool::Engine(_) => "tex",
            Tool::Tectonic(_) => "tectonic",
        }
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(if matches!(self, Tool::Tectonic(_)) { 300 } else { 120 })
    }
}

fn code(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'%' => return &line[..i],
            _ => i += 1,
        }
    }
    line
}

pub fn magic(text: &str, key: &str) -> Option<String> {
    text.lines().take(HEADER_LINES).find_map(|line| {
        let rest = line.trim_start().strip_prefix('%')?.trim_start();
        let rest = rest.get(..4).filter(|m| m.eq_ignore_ascii_case("!TEX")).map(|_| rest[4..].trim_start())?;
        let (name, value) = rest.split_once('=')?;
        let name = name.trim();
        let wanted = name.eq_ignore_ascii_case(key) || (key == "program" && name.eq_ignore_ascii_case("TS-program"));
        wanted.then(|| value.trim().to_string()).filter(|v| !v.is_empty())
    })
}

pub fn is_document(text: &str) -> bool {
    text.lines().any(|line| code(line).contains("\\documentclass"))
}

pub enum Root {
    Itself,
    At(PathBuf),
    Missing(PathBuf),
    None,
}

pub fn root_of(file: &Path, text: &str) -> Root {
    if let Some(named) = magic(text, "root") {
        let dir = file.parent().unwrap_or(Path::new("/"));
        let path = normalized(&dir.join(named));
        return if path.is_file() { if path == normalized(file) { Root::Itself } else { Root::At(path) } } else { Root::Missing(path) };
    }
    if is_document(text) { Root::Itself } else { Root::None }
}

pub fn program_of(text: &str) -> Program {
    if let Some(named) = magic(text, "program") {
        let named = named.to_ascii_lowercase();
        if named.contains("xe") {
            return Program::Xe;
        }
        if named.contains("lua") {
            return Program::Lua;
        }
        return Program::Pdf;
    }
    let mut header = String::new();
    for line in text.lines() {
        let line = code(line);
        header.push_str(line);
        if line.contains("\\begin{document}") {
            break;
        }
    }
    let mut rest = header.as_str();
    while let Some(at) = rest.find("\\usepackage").or_else(|| rest.find("\\RequirePackage")) {
        rest = &rest[at + 1..];
        let Some(open) = rest.find('{') else { break };
        let Some(close) = rest[open..].find('}') else { break };
        let names = &rest[open + 1..open + close];
        if names.split(',').map(str::trim).any(|n| matches!(n, "fontspec" | "unicode-math" | "polyglossia")) {
            return Program::Xe;
        }
    }
    Program::Pdf
}

pub fn places(path: Option<OsString>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = path.map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for place in PLACES {
        let place = PathBuf::from(place);
        if !out.contains(&place) {
            out.push(place);
        }
    }
    out
}

pub fn find(program: Program, places: &[PathBuf], bundled: Option<PathBuf>) -> Option<Tool> {
    let here = |dir: &Path, name: &str| Some(dir.join(name)).filter(|p| p.is_file());
    places
        .iter()
        .find_map(|dir| here(dir, "latexmk").filter(|_| here(dir, program.engine()).is_some()).map(Tool::Latexmk))
        .or_else(|| places.iter().find_map(|dir| here(dir, program.engine())).map(Tool::Engine))
        .or_else(|| bundled.filter(|p| p.is_file()).map(Tool::Tectonic))
        .or_else(|| places.iter().find_map(|dir| here(dir, "tectonic")).map(Tool::Tectonic))
}

pub const PIPED: &str = "texput";

pub fn wrapper(root: &Path) -> Option<String> {
    let name = root.file_name()?.to_str().filter(|n| !n.contains(['"', '%', '#', '{', '}', '\\', '\n']))?;
    Some(format!(
        "\\ifdefined\\pdfgentounicode\\else\\newcount\\pdfgentounicode\\fi\n\\ifdefined\\pdfglyphtounicode\\else\\def\\pdfglyphtounicode#1#2{{}}\\fi\n\\input{{\\detokenize{{{name}}}}}\n"
    ))
}

pub fn command(tool: &Tool, program: Program, root: &Path, out: &Path, piped: Option<File>) -> Command {
    let name = root.file_name().map(OsString::from).unwrap_or_default();
    let mut cmd = match tool {
        Tool::Latexmk(bin) => {
            let mut cmd = Command::new(bin);
            let format = match program {
                Program::Pdf => "-pdf",
                Program::Xe => "-pdfxe",
                Program::Lua => "-pdflua",
            };
            cmd.args(["-norc", "-interaction=nonstopmode", "-file-line-error", "-synctex=1", "-no-shell-escape", format]);
            cmd.arg(format!("-outdir={}", out.display()));
            cmd
        }
        Tool::Engine(bin) => {
            let mut cmd = Command::new(bin);
            cmd.args(["-interaction=nonstopmode", "-file-line-error", "-synctex=1", "-no-shell-escape"]);
            cmd.arg(format!("-output-directory={}", out.display()));
            cmd
        }
        Tool::Tectonic(bin) => {
            let mut cmd = Command::new(bin);
            cmd.args(["--untrusted", "--synctex", "--keep-logs", "-Z", "continue-on-errors", "--outdir"]);
            cmd.arg(out);
            cmd.env("TECTONIC_UNTRUSTED_MODE", "1");
            cmd
        }
    };
    if let Tool::Latexmk(bin) | Tool::Engine(bin) = tool
        && let Some(dir) = bin.parent()
    {
        let mut path = vec![dir.to_path_buf()];
        path.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
        if let Ok(joined) = std::env::join_paths(path) {
            cmd.env("PATH", joined);
        }
    }
    match piped {
        Some(file) => {
            cmd.arg("-");
            cmd.stdin(file);
        }
        None => {
            cmd.arg(name);
            cmd.stdin(Stdio::null());
        }
    }
    cmd.current_dir(root.parent().unwrap_or(Path::new("/")));
    cmd.env("max_print_line", "1000");
    cmd
}

pub struct Ran {
    pub status: Option<ExitStatus>,
    pub timed_out: bool,
}

pub fn run(mut cmd: Command, output: &Path, timeout: Duration, started: impl FnOnce(i32)) -> Result<Ran, String> {
    let log = std::fs::File::create(output).map_err(|e| e.to_string())?;
    cmd.stdout(log.try_clone().map_err(|e| e.to_string())?);
    cmd.stderr(log);
    cmd.process_group(0);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let group = child.id() as i32;
    started(group);
    let late = Arc::new(AtomicBool::new(false));
    let (done, wait) = std::sync::mpsc::channel::<()>();
    let watch = {
        let late = late.clone();
        std::thread::Builder::new()
            .name("zeron-tex-timeout".into())
            .spawn(move || {
                if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = wait.recv_timeout(timeout) {
                    late.store(true, Ordering::SeqCst);
                    stop(group);
                }
            })
            .map_err(|e| e.to_string())?
    };
    let status = child.wait().ok();
    let _ = done.send(());
    let _ = watch.join();
    Ok(Ran { status, timed_out: late.load(Ordering::SeqCst) })
}

pub fn stop(group: i32) {
    unsafe {
        libc::killpg(group, libc::SIGKILL);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub message: String,
    pub line: Option<u32>,
    pub file: Option<PathBuf>,
}

fn located(line: &str) -> Option<(&str, u32, &str)> {
    let mut from = 0;
    while let Some(at) = line[from..].find(':').map(|i| i + from) {
        let rest = &line[at + 1..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && rest[digits..].starts_with(": ") {
            let file = &line[..at];
            let known = Path::new(file).extension().and_then(|e| e.to_str()).is_some_and(|e| SOURCES.contains(&e));
            if !file.is_empty() && !file.starts_with(' ') && (known || file.starts_with('/') || file.starts_with("./") || file.starts_with("../")) {
                return Some((file, rest[..digits].parse().ok()?, rest[digits + 2..].trim()));
            }
        }
        from = at + 1;
    }
    None
}

fn line_mark(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("l.")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0).then(|| rest[..digits].parse().ok()).flatten()
}

fn opened<'a>(line: &'a str, stack: &mut Vec<&'a str>) {
    let mut rest = line;
    while let Some(at) = rest.find(['(', ')']) {
        if rest.as_bytes()[at] == b')' {
            stack.pop();
            rest = &rest[at + 1..];
        } else {
            let name = &rest[at + 1..];
            let end = name.find(|c: char| c.is_whitespace() || c == '(' || c == ')').unwrap_or(name.len());
            stack.push(&name[..end]);
            rest = &name[end..];
        }
    }
}

fn source(name: &str, base: &Path) -> PathBuf {
    let path = Path::new(name);
    let path = normalized(&if path.is_absolute() { path.to_path_buf() } else { base.join(path) });
    let tex = path.with_extension("tex");
    if path.extension().is_none() && tex.is_file() { tex } else { path }
}

pub fn problems(log: &str, base: &Path) -> Vec<Problem> {
    let lines: Vec<&str> = log.lines().collect();
    let mut out: Vec<Problem> = Vec::new();
    let mut stack: Vec<&str> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let found = if let Some(message) = line.strip_prefix("! ") {
            let after = lines[i + 1..].iter().take(40).take_while(|l| !l.starts_with("! ") && located(l).is_none());
            let file = stack.last().filter(|n| !n.is_empty()).map(|n| source(n, base));
            Some(Problem { message: message.trim().to_string(), line: after.filter_map(|l| line_mark(l)).next(), file })
        } else {
            opened(line, &mut stack);
            located(line).map(|(file, n, message)| Problem { message: message.to_string(), line: Some(n), file: Some(source(file, base)) })
        };
        if let Some(p) = found
            && !p.message.is_empty()
            && !out.contains(&p)
        {
            out.push(p);
            if out.len() >= MAX_PROBLEMS {
                break;
            }
        }
    }
    if out.len() > 1 {
        out.retain(|p| p.message != "Emergency stop.");
    }
    out
}

pub fn last_words(output: &str) -> Option<String> {
    let lines: Vec<&str> = output.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines.iter().rev().find(|l| l.starts_with("error:")).or(lines.last()).map(|l| l.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tex_program_follows_the_magic_comment_then_fontspec() {
        assert_eq!(program_of("\\documentclass{article}\n\\usepackage{tabularx}\n"), Program::Pdf);
        assert_eq!(program_of("% !TEX program = xelatex\n\\documentclass{article}\n"), Program::Xe);
        assert_eq!(program_of("%!TeX TS-program = LuaLaTeX\n\\documentclass{article}\n"), Program::Lua);
        assert_eq!(program_of("% !TEX program = pdflatex\n\\usepackage{fontspec}\n"), Program::Pdf);
        assert_eq!(program_of("\\documentclass{article}\n\\usepackage[no-math]{ amsmath, fontspec }\n"), Program::Xe);
        assert_eq!(program_of("\\documentclass{article}\n% \\usepackage{fontspec}\n"), Program::Pdf);
        assert_eq!(program_of("\\begin{document}\n\\usepackage{fontspec}\n"), Program::Pdf);
    }

    #[test]
    fn tex_root_is_the_file_itself_or_the_one_it_names() {
        let dir = std::env::temp_dir().join(format!("zgx-tex-root-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("parts")).unwrap();
        let dir = dir.canonicalize().unwrap();
        let main = dir.join("main.tex");
        std::fs::write(&main, "\\documentclass{article}").unwrap();
        let part = dir.join("parts/a.tex");
        assert!(matches!(root_of(&main, "\\documentclass{article}"), Root::Itself));
        assert!(matches!(root_of(&part, "% !TEX root = ../main.tex\n\\section{A}"), Root::At(p) if p == main));
        assert!(matches!(root_of(&part, "% !TEX root = ../gone.tex"), Root::Missing(_)));
        assert!(matches!(root_of(&part, "\\section{A} % \\documentclass"), Root::None));
        assert!(matches!(root_of(&part, "\\section{A} 50\\% \\documentclass{x}"), Root::Itself));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tex_finds_latexmk_beside_its_engine_then_the_engine_then_tectonic() {
        let dir = std::env::temp_dir().join(format!("zgx-tex-find-{}", std::process::id()));
        let (a, b, t) = (dir.join("a"), dir.join("b"), dir.join("t"));
        for d in [&a, &b, &t] {
            std::fs::create_dir_all(d).unwrap();
        }
        let touch = |p: PathBuf| {
            std::fs::write(&p, "").unwrap();
            p
        };
        let tectonic = touch(t.join("tectonic"));
        let places = vec![a.clone(), b.clone()];
        assert_eq!(find(Program::Pdf, &places, None), None);
        assert_eq!(find(Program::Pdf, &places, Some(tectonic.clone())), Some(Tool::Tectonic(tectonic.clone())));
        touch(a.join("latexmk"));
        let engine = touch(b.join("pdflatex"));
        assert_eq!(find(Program::Pdf, &places, Some(tectonic.clone())), Some(Tool::Engine(engine)));
        touch(b.join("latexmk"));
        assert_eq!(find(Program::Pdf, &places, Some(tectonic.clone())), Some(Tool::Latexmk(b.join("latexmk"))));
        assert_eq!(find(Program::Xe, &places, Some(tectonic.clone())), Some(Tool::Tectonic(tectonic)));
        assert!(super::places(None).iter().any(|p| p == Path::new("/Library/TeX/texbin")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tex_tectonic_reads_the_root_through_a_wrapper_that_stands_in_for_pdftex_primitives() {
        let text = wrapper(Path::new("/my docs/cv~1.tex")).unwrap();
        assert!(text.contains("\\newcount\\pdfgentounicode") && text.contains("\\def\\pdfglyphtounicode#1#2{}"));
        assert!(text.ends_with("\\input{\\detokenize{cv~1.tex}}\n"));
        assert_eq!(wrapper(Path::new("/doc/50%.tex")), None);
        assert_eq!(wrapper(Path::new("/doc/a{b}.tex")), None);
        let file = File::open("/dev/null").unwrap();
        let cmd = command(&Tool::Tectonic("/t/tectonic".into()), Program::Pdf, Path::new("/doc/cv.tex"), Path::new("/out"), Some(file));
        assert_eq!(cmd.get_args().last().and_then(|a| a.to_str()), Some("-"));
        let cmd = command(&Tool::Engine("/b/pdflatex".into()), Program::Pdf, Path::new("/doc/cv.tex"), Path::new("/out"), None);
        assert_eq!(cmd.get_args().last().and_then(|a| a.to_str()), Some("cv.tex"));
    }

    #[test]
    fn tex_log_problems_name_their_message_line_and_file() {
        let base = Path::new("/doc");
        let pdflatex = "(./cv.tex\n./cv.tex:58: Undefined control sequence.\nl.58 \\undefinedmacro\n                     {x}\nThe control sequence at the end of the top line\n";
        assert_eq!(problems(pdflatex, base), vec![Problem { message: "Undefined control sequence.".into(), line: Some(58), file: Some(PathBuf::from("/doc/cv.tex")) }]);
        let plain = "(glyphtounicode\n! Undefined control sequence.\nl.7 \\pdfglyphtounicode\n                      {A}{0041}\n! LaTeX Error: Missing \\begin{document}.\n\nSee the LaTeX manual\nl.7 \\pdfglyphtounicode{A\n! Emergency stop.\n<*> cv.tex\n";
        assert_eq!(
            problems(plain, base),
            vec![
                Problem { message: "Undefined control sequence.".into(), line: Some(7), file: Some(PathBuf::from("/doc/glyphtounicode")) },
                Problem { message: "LaTeX Error: Missing \\begin{document}.".into(), line: Some(7), file: Some(PathBuf::from("/doc/glyphtounicode")) },
            ]
        );
        let system = "/opt/texmf-dist/tex/generic/pdftex/glyphtounicode.tex:7: Undefined control sequence.\nl.7 \\pdfglyphtounicode\n";
        assert_eq!(problems(system, base)[0].file, Some(PathBuf::from("/opt/texmf-dist/tex/generic/pdftex/glyphtounicode.tex")));
        assert!(problems("Package hyperref Warning: Token not allowed on input line 12.\nLaTeX Font Info: at 10:30: fine\n", base).is_empty());
        assert_eq!(problems(&"! Too many }'s.\nl.3 }\n".repeat(80), base).len(), 1);
        let nested = "(./main.tex (/tex/article.cls (/tex/size10.clo)) (./parts/a.tex\n(help (e.g., `\\hobx')) text\n! Missing $ inserted.\nl.4 x^\n";
        assert_eq!(problems(nested, base)[0].file, Some(PathBuf::from("/doc/parts/a.tex")));
        assert_eq!(problems("! Undefined control sequence.\nl.2 \\x\n", base)[0].file, None);
    }

    #[test]
    fn tex_run_past_its_time_stops_the_whole_group() {
        let dir = std::env::temp_dir().join(format!("zgx-tex-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("child");
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(format!("sleep 300 & echo $! > '{}'; wait", marker.display()));
        let started = std::time::Instant::now();
        // (Long enough for the shell to have named its child on a busy machine.)
        let ran = run(cmd, &dir.join("output.txt"), Duration::from_secs(3), |_| {}).unwrap();
        assert!(ran.timed_out && started.elapsed() < Duration::from_secs(60));
        let child: i32 = std::fs::read_to_string(&marker).unwrap().trim().parse().unwrap();
        // Killed, it is gone once it has died and been reaped: soon, not at once.
        let killed = std::time::Instant::now();
        while unsafe { libc::kill(child, 0) } == 0 && killed.elapsed() < Duration::from_secs(30) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_ne!(unsafe { libc::kill(child, 0) }, 0, "the grandchild went with it");
        let ran = run(Command::new("/usr/bin/true"), &dir.join("output.txt"), Duration::from_secs(5), |_| {}).unwrap();
        assert!(!ran.timed_out && ran.status.is_some_and(|s| s.success()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tex_last_words_prefer_an_error_line() {
        assert_eq!(last_words("note: Running TeX ...\nerror: no network\nnote: done\n").as_deref(), Some("error: no network"));
        assert_eq!(last_words("Latexmk: gave up\n\n").as_deref(), Some("Latexmk: gave up"));
        assert_eq!(last_words("  \n"), None);
    }
}
