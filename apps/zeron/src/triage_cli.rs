use std::io::{BufRead as _, Write as _};
use std::path::Path;

use zeron_engine::triage;

pub fn run(data_dir: &Path, dry_run: bool) -> anyhow::Result<()> {
    let prepared = triage::prepare(data_dir)?;
    triage::write_context(data_dir, &prepared, triage::Entry::Cli, None, !dry_run)?;
    println!("Triage playbook: {}", prepared.playbook.display());
    println!("Machine facts:   {}", prepared.context.display());
    let prompt = triage::launch_prompt(&prepared);
    let agents = triage::terminal_agents();
    if agents.is_empty() {
        println!(
            "\nNeither Claude Code (`claude`) nor Codex (`codex`) is on PATH. Start any coding agent in {} and give it this message:\n\n{prompt}",
            prepared.dir.display()
        );
        return Ok(());
    }
    if dry_run {
        println!("\nWould start, in {}:", prepared.dir.display());
        for (name, exe) in &agents {
            println!("  {name}: {} {}", exe.display(), shell_quote(&prompt));
        }
        return Ok(());
    }
    let (name, exe) = pick(&agents)?;
    triage::spawn_source_clone(data_dir, &prepared.dir);
    println!("\nStarting {name}…");
    let status = std::process::Command::new(&exe)
        .arg(&prompt)
        .current_dir(&prepared.dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("{name} exited with {status}");
    }
    Ok(())
}

fn pick(
    agents: &[(&'static str, std::path::PathBuf)],
) -> anyhow::Result<(&'static str, std::path::PathBuf)> {
    if agents.len() < 2 {
        return Ok(agents[0].clone());
    }
    print!("\nBoth Claude Code and Codex are installed. Start which? [1] Claude Code  [2] Codex: ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(match answer.trim() {
        "2" | "codex" | "Codex" => agents[1].clone(),
        _ => agents[0].clone(),
    })
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}
