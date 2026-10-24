use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use zeron_proto::claude_config::{
    ClaudeConfigSnapshot, LocalCommandOutput, Sources, keep_known_keys,
};

use super::{ClaudeHarness, PROBE_TIMEOUT, config_root, launch_env, managed_mcp_config, model_arg};
use crate::HarnessError;
use crate::process::{Command, Stdio};

fn global_config_path(env: &BTreeMap<String, String>) -> PathBuf {
    let explicit = env
        .get("CLAUDE_CONFIG_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("CLAUDE_CONFIG_DIR")
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
        });
    match explicit {
        Some(dir) => dir.join(".claude.json"),
        None => env
            .get("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(crate::executable::home_or_current_dir)
            .join(".claude.json"),
    }
}

// Verified: a `/config` run in a subfolder writes the repo root's .claude/settings.local.json.
fn project_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

fn read_known(path: &Path) -> serde_json::Map<String, Value> {
    let mut map = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    keep_known_keys(&mut map);
    map
}

pub(super) fn read_config(
    harness: &ClaudeHarness,
    cwd: &Path,
    run_env: &BTreeMap<String, String>,
) -> ClaudeConfigSnapshot {
    let env = launch_env(&harness.launch, run_env);
    let user = config_root(&env).join("settings.json");
    let project = project_root(cwd).join(".claude");
    let global = global_config_path(&env);
    let files = [
        user.clone(),
        project.join("settings.json"),
        project.join("settings.local.json"),
        global.clone(),
    ];
    let sources = Sources {
        user: read_known(&files[0]),
        project: read_known(&files[1]),
        local: read_known(&files[2]),
        global: read_known(&files[3]),
    };
    ClaudeConfigSnapshot {
        values: sources.values(),
        user_settings: files[0].to_string_lossy().into_owned(),
        files: files
            .iter()
            .filter(|path| path.is_file())
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    }
}

pub(super) async fn run(
    harness: &ClaudeHarness,
    request: &zeron_proto::RunRequest,
    command: &str,
    connect_mcp: bool,
) -> Result<LocalCommandOutput, HarnessError> {
    let cwd = Path::new(&request.cwd);
    if !command.starts_with('/') {
        return Err(HarnessError::Protocol(format!(
            "not a slash command: {command}"
        )));
    }
    let exe = harness.resolve_executable()?;
    let mut cmd = Command::new(&exe);
    cmd.current_dir(cwd);
    crate::process::owned::configure(&mut cmd);
    crate::compose_child_path(&mut cmd, &exe);
    harness.launch.apply(&mut cmd, Some("CLAUDE_CONFIG_DIR"));
    crate::apply_run_env(&mut cmd, &request.env);
    cmd.args([
        "--print",
        "--output-format",
        "stream-json",
        "--verbose",
        "--no-session-persistence",
        "--settings",
        r#"{"disableAllHooks":true}"#,
    ]);
    if !connect_mcp && !managed_mcp_config() {
        cmd.arg("--strict-mcp-config");
    }
    if let Some(model) = model_arg(request) {
        cmd.args(["--model", &model]);
    }
    if let Some(session) = &request.resume {
        cmd.args(["--resume", session, "--fork-session"]);
    }
    cmd.args(&harness.launch.args);
    cmd.arg(command);
    cmd.env("CLAUDE_CODE_AUTO_CONNECT_IDE", "0")
        .env("CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL", "1");
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(PROBE_TIMEOUT, cmd.output())
        .await
        .map_err(|_| {
            HarnessError::Protocol(format!(
                "Claude {command} timed out after {}s",
                PROBE_TIMEOUT.as_secs()
            ))
        })?
        .map_err(|e| {
            crate::executable::spawn_error(e, Some(cwd), || crate::executable::binary_hint(&exe))
        })?;
    parse_output(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail = stderr.lines().rev().take(6).collect::<Vec<_>>();
        let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
        HarnessError::Protocol(format!(
            "Claude {command} exited ({}) without an answer{}",
            output.status,
            if tail.is_empty() {
                String::new()
            } else {
                format!(": {}", crate::redact_secrets(&tail))
            }
        ))
    })
}

fn parse_output(stdout: &str) -> Option<LocalCommandOutput> {
    let mut init = None;
    for line in stdout.lines() {
        let Ok(frame) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match (
            frame.get("type").and_then(Value::as_str),
            frame.get("subtype").and_then(Value::as_str),
        ) {
            (Some("system"), Some("init")) => init = Some(frame),
            (Some("result"), _) => {
                return Some(LocalCommandOutput {
                    init,
                    text: frame
                        .get("result")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                });
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_keeps_the_init_and_the_result_text() {
        let stdout = concat!(
            "{\"type\":\"system\",\"subtype\":\"init\",\"claude_code_version\":\"2.1.310\"}\n",
            "not json\n",
            "{\"type\":\"assistant\",\"message\":{}}\n",
            "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"Set Theme to dark\"}\n",
        );
        let out = parse_output(stdout).unwrap();
        assert_eq!(out.text, "Set Theme to dark");
        assert_eq!(out.init.unwrap()["claude_code_version"], "2.1.310");
        assert!(parse_output("{\"type\":\"system\",\"subtype\":\"init\"}\n").is_none());
    }

    #[test]
    fn config_reads_user_project_local_and_global_files() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("cfg");
        let repo = dir.path().join("repo");
        let cwd = repo.join("sub");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join(".claude")).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(
            config.join("settings.json"),
            r#"{"theme":"light","env":{"SECRET":"x"}}"#,
        )
        .unwrap();
        std::fs::write(
            repo.join(".claude/settings.local.json"),
            r#"{"spinnerTipsEnabled":false}"#,
        )
        .unwrap();
        std::fs::write(config.join(".claude.json"), r#"{"copyOnSelect":false}"#).unwrap();
        let harness = ClaudeHarness::new();
        let env = BTreeMap::from([(
            "CLAUDE_CONFIG_DIR".to_owned(),
            config.to_string_lossy().into_owned(),
        )]);
        let snapshot = read_config(&harness, &cwd, &env);
        assert_eq!(snapshot.values["theme"], "light");
        assert_eq!(snapshot.values["tips"], "false");
        assert_eq!(snapshot.values["copyOnSelect"], "false");
        assert_eq!(snapshot.values["verbose"], "false");
        assert_eq!(snapshot.files.len(), 3);
    }
}
