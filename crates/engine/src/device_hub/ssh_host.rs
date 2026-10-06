use std::collections::HashSet;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;
use zeron_proto::device_hub::{
    DeviceHostCheck, DeviceHostStatus, DeviceHostTest, DeviceHubHost, DevicePlatform,
    DevicePlatformAvailability, DeviceToolVersion, DeviceToolVersions, SshDeviceHostConfig,
};

use super::actions::{CommandOutput, Helpers, Runner};
use super::{hub_process, toolchain};

const PROBE_TIMEOUT: Duration = Duration::from_secs(45);
const START_TIMEOUT: Duration = Duration::from_secs(1300);
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);
const FORWARD_READY: Duration = Duration::from_secs(15);
const HEALTH_EVERY: Duration = Duration::from_secs(10);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);
const RECONNECT_MAX: Duration = Duration::from_secs(30);
const HEALTH_MISSES: u32 = 3;
const RETRY_AFTER: Duration = Duration::from_secs(30);
const SCRIPT_OUTPUT_LIMIT: usize = 1 << 20;
const SCREENSHOT_LIMIT: usize = 64 << 20;
const STDERR_TAIL: usize = 2000;
pub(crate) const HOST_UNREACHABLE: &str =
    "Cannot reach device host. Test its SSH connection in Settings.";
const HOST_REMOVED: &str = "The device host was removed.";
const NODE_MISSING: &str = "Node is missing from the non-interactive SSH PATH";

pub(crate) const REMOTE_ENV: &str = r#"export PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
if [ -z "$ANDROID_HOME" ]; then
  if [ -d "$HOME/Library/Android/sdk" ]; then export ANDROID_HOME="$HOME/Library/Android/sdk";
  elif [ -d "$HOME/Android/Sdk" ]; then export ANDROID_HOME="$HOME/Android/Sdk"; fi
fi
if [ -n "$ANDROID_HOME" ]; then export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH"; fi
if [ -z "$JAVA_HOME" ] && ! command -v java >/dev/null 2>&1; then
  for device_java_home in "$HOME/.local/opt/android-studio/jbr" /opt/android-studio/jbr /Applications/Android\ Studio.app/Contents/jbr "$HOME/Applications/Android Studio.app/Contents/jbr"; do
    if [ -x "$device_java_home/bin/java" ]; then export JAVA_HOME="$device_java_home"; break; fi
  done
fi
if [ -n "$JAVA_HOME" ]; then export PATH="$JAVA_HOME/bin:$PATH"; fi
"#;

const REMOTE_SCRIPT: &str = r#"
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const net = require('node:net');
const { spawn, spawnSync } = require('node:child_process');
const root = path.join(os.homedir(), '.zeron', 'device');
const state = path.join(root, 'hosts', owner);
const run = (command, args, options = {}) => spawnSync(command, args, { encoding: 'utf8', timeout: 30000, ...options });
const read = (file) => { try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return null; } };
const write = (file, value) => { const tmp = file + '.' + process.pid; fs.writeFileSync(tmp, JSON.stringify(value), { mode: 0o600 }); fs.renameSync(tmp, file); };
const toolVersions = (name, requiredVersion, entry, record) => {
  const directory = path.join(root, 'tools');
  const prefix = name + '@';
  let names = [];
  try { names = fs.readdirSync(directory); } catch (error) { if (error.code !== 'ENOENT') return null; }
  let unreadable = false;
  const installedVersions = names.filter(name => name.startsWith(prefix)).map(name => name.slice(prefix.length)).filter(version => {
    if (!/^[0-9]+\.[0-9]+\.[0-9]+(?:-[a-zA-Z0-9.-]+)?$/.test(version)) return false;
    const dir = path.join(directory, prefix + version);
    try { return fs.readFileSync(path.join(dir, '.install-complete'), 'utf8').trim() === version && fs.existsSync(path.join(dir, 'node_modules', name, entry)); } catch (error) { if (error.code !== 'ENOENT') unreadable = true; return false; }
  }).sort();
  if (unreadable) return null;
  let runningVersion = null;
  if (record?.entryPath && record?.pid) {
    const command = run('ps', ['-p', String(record.pid), '-o', 'command=']).stdout || '';
    runningVersion = installedVersions.find(version => {
      const install = path.join(directory, prefix + version);
      return record.entryPath === path.join(install, 'node_modules', name, entry) && command.includes(install + path.sep);
    }) ?? null;
  }
  return { requiredVersion, installedVersions, runningVersion };
};
const versions = () => {
  const result = {
    hub: toolVersions('expo-device-hub', hubVersion, 'dist/server/cli.mjs', read(path.join(state, 'hub.json'))),
    agent: toolVersions('agent-device', agentVersion, 'bin/agent-device.mjs', { ...read(path.join(state, 'agent.json')), ...read(path.join(state, 'daemon.json')) }),
  };
  return result.hub && result.agent ? result : undefined;
};
const ours = hub => {
  const command = run('ps', ['-p', String(hub.pid), '-o', 'command=']).stdout || '';
  return command.includes(hub.entryPath) && command.includes(' --port ' + hub.port + ' ');
};
const stopHub = hub => {
  if (!hub || hub.owner !== owner) return;
  if (ours(hub)) {
    try { process.kill(hub.pid, 'SIGTERM'); } catch {}
  }
};
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const healthy = async (port, route) => { try { return (await fetch('http://127.0.0.1:' + port + route, { signal: AbortSignal.timeout(2000) })).ok; } catch { return false; } };
const port = () => new Promise((resolve, reject) => { const server = net.createServer(); server.once('error', reject); server.listen(0, '127.0.0.1', () => { const value = server.address().port; server.close(() => resolve(value)); }); });
async function acquireLock(lock, complete = () => false) {
  const deadline = Date.now() + 600000;
  const token = process.pid + ':' + require('node:crypto').randomUUID();
  const holder = () => { try { return fs.readlinkSync(lock); } catch (error) { if (error.code === 'ENOENT') return null; throw error; } };
  while (true) {
    try {
      fs.symlinkSync(token, lock);
      return () => { if (holder() === token) fs.unlinkSync(lock); };
    } catch (error) {
      if (error.code !== 'EEXIST') throw error;
      if (complete()) return null;
      const previous = holder();
      if (previous === null) continue;
      const pid = Number(previous.split(':')[0]);
      if (!Number.isSafeInteger(pid) || pid <= 0) throw Error('Invalid device lock at ' + lock);
      try { process.kill(pid, 0); } catch (error) {
        if (error.code === 'ESRCH' && holder() === previous) {
          try { fs.unlinkSync(lock); } catch (error) { if (error.code !== 'ENOENT') throw error; }
          continue;
        }
      }
      if (Date.now() > deadline) throw Error('Device operation is locked at ' + lock + '. Check the other installer before removing the lock.');
      await sleep(500);
    }
  }
}
async function install(name, version, entry) {
  const dir = path.join(root, 'tools', name + '@' + version);
  const file = path.join(dir, 'node_modules', name, entry);
  const complete = () => fs.existsSync(file) && fs.existsSync(path.join(dir, '.install-complete')) && fs.readFileSync(path.join(dir, '.install-complete'), 'utf8').trim() === version;
  if (complete()) return file;
  fs.mkdirSync(path.dirname(dir), { recursive: true });
  const release = await acquireLock(dir + '.lock', complete);
  if (!release) return file;
  let staging;
  try {
    if (complete()) return file;
    staging = fs.mkdtempSync(path.join(path.dirname(dir), '.install-'));
    const result = run('npm', ['install', '--prefix', staging, '--no-fund', '--no-audit', name + '@' + version], { timeout: 600000, maxBuffer: 8 * 1024 * 1024 });
    if (result.status !== 0) throw Error('Installing ' + name + ': ' + (result.error?.message || result.stderr?.slice(-2000)));
    if (!fs.existsSync(path.join(staging, 'node_modules', name, entry))) throw Error('Missing installed entry for ' + name);
    fs.writeFileSync(path.join(staging, '.install-complete'), version);
    fs.rmSync(dir, { recursive: true, force: true });
    fs.renameSync(staging, dir);
    return file;
  } finally {
    if (staging) fs.rmSync(staging, { recursive: true, force: true });
    release();
  }
}
(async () => {
  const ios = process.platform === 'darwin' && run('xcrun', ['simctl', 'help']).status === 0;
  const android = run('adb', ['version']).status === 0;
  const platforms = [
    { platform: 'ios', available: ios, ...(!ios ? { reason: 'iOS needs macOS with Xcode and working xcrun simctl.' } : {}) },
    { platform: 'android', available: android, ...(!android ? { reason: 'Android SDK missing. Set ANDROID_HOME or put adb on the SSH PATH.' } : {}) },
  ];
  if (mode === 'probe') {
    const npm = run('npm', ['--version']);
    console.log(JSON.stringify({ nodePath: process.execPath, nodeVersion: process.versions.node, npmVersion: npm.status === 0 ? npm.stdout.trim() : null, platforms, tools: versions() })); return;
  }
  if (Number(process.versions.node.split('.')[0]) < 22) throw Error('Node 22 or newer is required on the device host.');
  fs.mkdirSync(state, { recursive: true, mode: 0o700 });
  const releaseHost = await acquireLock(path.join(state, 'runtime.lock'));
  try {
  const hubFile = path.join(state, 'hub.json');
  const daemonFile = path.join(state, 'daemon.json');
  const agentFile = path.join(state, 'agent.json');
  if (mode === 'stop' || mode === 'stop-agent') {
    const hub = read(hubFile);
    if (mode === 'stop' && hub && hub.owner === owner) {
      stopHub(hub);
      fs.rmSync(hubFile, { force: true });
    }
    const entry = read(agentFile)?.entryPath || path.join(root, 'tools', 'agent-device@' + agentVersion, 'node_modules', 'agent-device', 'bin', 'agent-device.mjs');
    if (fs.existsSync(entry)) run(process.execPath, [entry, 'daemon', 'stop', '--state-dir', state]);
    return;
  }
  if (!ios && !android) throw Error(platforms.map(p => p.reason).join(' '));
  const hubEntry = await install('expo-device-hub', hubVersion, 'dist/server/cli.mjs');
  const guard = path.join(state, 'hub-guard.mjs');
  fs.writeFileSync(guard, guardSource, { mode: 0o600 });
  let hub = read(hubFile);
  if (!hub || hub.owner !== owner || hub.entryPath !== hubEntry || !ours(hub) || !await healthy(hub.port, '/readyz')) {
    stopHub(hub);
    const env = { ...process.env, FORCE_COLOR: '0', NO_COLOR: '1' };
    for (const key of Object.keys(env)) if (key.startsWith('EXPO_DEVICE_HUB_')) delete env[key];
    if (env.ANDROID_HOME) env.SERVE_EMU_CAMERA_DIR = '/dev/null/zeron-no-camera';
    for (let attempt = 0; attempt < 5; attempt++) {
      const hubPort = await port();
      const log = fs.openSync(path.join(state, 'hub.log'), 'a');
      const child = spawn(process.execPath, ['--import', require('node:url').pathToFileURL(guard).href, hubEntry, '--port', String(hubPort), '--host', '127.0.0.1', '--hide-sidebar', '--hide-boot-device'], {
        cwd: state, detached: true, stdio: ['ignore', log, log], env,
      });
      try { await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject); }); }
      finally { fs.closeSync(log); }
      child.unref();
      hub = { owner, pid: child.pid, port: hubPort, entryPath: hubEntry };
      write(hubFile, hub);
      const deadline = Date.now() + 30000;
      let listening = false;
      while (child.exitCode === null && child.signalCode === null) {
        if (await healthy(hub.port, '/readyz')) { listening = true; break; }
        if (Date.now() > deadline) { stopHub(hub); throw Error('Device hub did not become ready. See ' + path.join(state, 'hub.log')); }
        await sleep(200);
      }
      if (listening) break;
      fs.rmSync(hubFile, { force: true });
      if (attempt === 4) throw Error('Device hub exited before becoming ready. See ' + path.join(state, 'hub.log'));
    }
  }
  let agentResult = {};
  if (mode === 'agent-start') {
    const agentEntry = await install('agent-device', agentVersion, 'bin/agent-device.mjs');
    const previousAgent = read(agentFile)?.entryPath;
    let daemon = read(daemonFile);
    if (daemon && (previousAgent !== agentEntry || !await healthy(daemon.httpPort, '/health'))) {
      const stopped = run(process.execPath, [previousAgent || agentEntry, 'daemon', 'stop', '--state-dir', state]);
      if (stopped.status !== 0) throw Error('Could not stop the previous agent-device version.');
      fs.rmSync(daemonFile, { force: true });
      daemon = null;
    }
    if (!daemon) {
      fs.rmSync(daemonFile, { force: true });
      const env = { ...process.env, AGENT_DEVICE_STATE_DIR: state, AGENT_DEVICE_DAEMON_SERVER_MODE: 'http', AGENT_DEVICE_DAEMON_IDLE_TIMEOUT_MS: '0', AGENT_DEVICE_NO_UPDATE_NOTIFIER: '1' };
      delete env.AGENT_DEVICE_DAEMON_BASE_URL; delete env.AGENT_DEVICE_DAEMON_AUTH_TOKEN; delete env.AGENT_DEVICE_CONFIG;
      run(process.execPath, [agentEntry, 'devices', '--json'], { env });
      daemon = read(daemonFile);
    }
    if (!daemon || !await healthy(daemon.httpPort, '/health')) throw Error('agent-device daemon did not become ready in ' + state);
    write(agentFile, { entryPath: agentEntry });
    agentResult = { daemonPort: daemon.httpPort, token: daemon.token, entryPath: agentEntry };
  }
  const vendor = path.resolve(path.dirname(hubEntry), '../../vendor/serve-sim/dist');
  const optional = file => fs.existsSync(file) ? file : null;
  console.log(JSON.stringify({ nodePath: process.execPath, platforms, tools: versions(), hubPort: hub.port, ...agentResult,
    androidHome: process.env.ANDROID_HOME || null,
    helpers: { serveSimAxSettings: optional(path.join(vendor, 'simax/serve-sim-ax-settings')), serveSimCli: optional(path.join(vendor, 'serve-sim.js')) } }));
  } finally { releaseHost(); }
})().catch(error => { console.error(error.message); process.exitCode = 1; });
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Probe,
    Start,
    AgentStart,
    StopAgent,
    Stop,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Probe => "probe",
            Mode::Start => "start",
            Mode::AgentStart => "agent-start",
            Mode::StopAgent => "stop-agent",
            Mode::Stop => "stop",
        }
    }
}

pub(crate) fn quote(value: &str) -> String {
    super::agent::sh_quote(value)
}

pub(crate) fn remote_script(owner: &str, mode: Mode) -> String {
    let js = |value: &str| serde_json::to_string(value).unwrap_or_default();
    format!(
        "const owner = {};\nconst mode = {};\nconst hubVersion = {};\nconst agentVersion = {};\nconst guardSource = {};\n{REMOTE_SCRIPT}",
        js(owner),
        js(mode.name()),
        js(toolchain::hub_version()),
        js(toolchain::agent_version()),
        js(hub_process::GUARD),
    )
}

pub(crate) fn owner_for(data_dir: &Path, host_id: &str) -> String {
    let digest = Sha256::digest(format!("{}\0{host_id}", data_dir.display()).as_bytes());
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()[..24]
        .to_string()
}

pub(crate) fn ssh_args(config: &SshDeviceHostConfig, options: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
    ]
    .map(String::from)
    .to_vec();
    if let Some(port) = config.port {
        args.extend(["-p".into(), port.to_string()]);
    }
    if let Some(identity) = &config.identity_file {
        args.extend(["-i".into(), identity.clone()]);
    }
    args.extend(options.iter().map(|option| option.to_string()));
    args.extend(["--".into(), config.target.clone()]);
    args
}

// The login shell only parses `sh -s`, so csh or fish never sees the script.
const REMOTE_SHELL: [&str; 2] = ["sh", "-s"];
const SCRIPT_END: &str = "ZERON_DEVICE_SCRIPT_END";

fn remote_input(script: &str) -> String {
    format!("{REMOTE_ENV}{script}\n")
}

pub(crate) fn bootstrap_input(owner: &str, mode: Mode) -> String {
    remote_input(&format!(
        "command -v node >/dev/null 2>&1 || {{ echo \"{NODE_MISSING}\" >&2; exit 1; }}\nexec node <<'{SCRIPT_END}'\n{}\n{SCRIPT_END}",
        remote_script(owner, mode)
    ))
}

async fn run_remote(
    ssh: &Path,
    config: &SshDeviceHostConfig,
    input: String,
    timeout: Duration,
    limit: usize,
) -> Result<Ran, String> {
    let mut args = ssh_args(config, &[]);
    args.extend(REMOTE_SHELL.map(String::from));
    run_ssh(ssh, args, Some(input), timeout, limit).await
}

pub(crate) fn forward_args(
    config: &SshDeviceHostConfig,
    hub: (u16, u16),
    daemon: Option<(u16, u16)>,
) -> Vec<String> {
    let mut options = vec![
        "-o".to_string(),
        "ExitOnForwardFailure=yes".into(),
        "-o".into(),
        "ServerAliveInterval=10".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        "-N".into(),
    ];
    for (local, remote) in std::iter::once(hub).chain(daemon) {
        options.extend(["-L".into(), format!("127.0.0.1:{local}:127.0.0.1:{remote}")]);
    }
    let options: Vec<&str> = options.iter().map(String::as_str).collect();
    ssh_args(config, &options)
}

pub(crate) fn check_identity(config: &SshDeviceHostConfig) -> Result<(), String> {
    config.validate()?;
    match &config.identity_file {
        Some(file) if !Path::new(file).is_file() => {
            Err(format!("The identity file {file} does not exist."))
        }
        _ => Ok(()),
    }
}

struct Ran {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: String,
}

async fn run_ssh(
    ssh: &Path,
    args: Vec<String>,
    stdin: Option<String>,
    timeout: Duration,
    limit: usize,
) -> Result<Ran, String> {
    let mut command = tokio::process::Command::new(ssh);
    command
        .args(&args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let run = async {
        let mut child = command.spawn()?;
        let input = child.stdin.take();
        let write = async {
            if let (Some(mut input), Some(text)) = (input, stdin) {
                let _ = input.write_all(text.as_bytes()).await;
            }
            Ok::<_, std::io::Error>(())
        };
        let (_, stdout, stderr) = tokio::try_join!(
            write,
            super::actions::read_pipe(child.stdout.take(), limit),
            super::actions::read_pipe(child.stderr.take(), limit)
        )?;
        let status = child.wait().await?;
        Ok::<_, std::io::Error>(Ran {
            code: status.code(),
            stdout,
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    };
    match tokio::time::timeout(timeout, run).await {
        Err(_) => Err(format!("SSH timed out after {} s.", timeout.as_secs())),
        Ok(Err(error)) => Err(format!("Could not run ssh: {error}")),
        Ok(Ok(ran)) => Ok(ran),
    }
}

fn failure(ran: &Ran) -> String {
    let detail = ran.stderr.trim();
    let detail = if detail.is_empty() {
        String::from_utf8_lossy(&ran.stdout).trim().to_string()
    } else {
        detail.to_string()
    };
    if detail.is_empty() {
        format!("SSH exited with {}.", ran.code.unwrap_or(-1))
    } else {
        redact(&tail(&detail, STDERR_TAIL))
    }
}

fn redact(text: &str) -> String {
    static SECRET: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(_authToken=)\S+|://[^/\s@]+@").expect("valid pattern")
    });
    SECRET
        .replace_all(text, |caps: &regex::Captures| match caps.get(1) {
            Some(key) => format!("{}<redacted>", key.as_str()),
            None => "://<redacted>@".into(),
        })
        .into_owned()
}

fn tail(text: &str, limit: usize) -> String {
    let skip = text.chars().count().saturating_sub(limit);
    text.chars().skip(skip).collect()
}

async fn bootstrap(
    ssh: &Path,
    config: &SshDeviceHostConfig,
    owner: &str,
    mode: Mode,
) -> Result<Ran, String> {
    let timeout = match mode {
        Mode::Start | Mode::AgentStart => START_TIMEOUT,
        _ => PROBE_TIMEOUT,
    };
    run_remote(
        ssh,
        config,
        bootstrap_input(owner, mode),
        timeout,
        SCRIPT_OUTPUT_LIMIT,
    )
    .await
}

async fn bootstrap_json<T: serde::de::DeserializeOwned>(
    ssh: &Path,
    config: &SshDeviceHostConfig,
    owner: &str,
    mode: Mode,
) -> Result<T, String> {
    let ran = bootstrap(ssh, config, owner, mode).await?;
    if ran.code != Some(0) {
        return Err(failure(&ran));
    }
    serde_json::from_slice(String::from_utf8_lossy(&ran.stdout).trim().as_bytes())
        .map_err(|e| format!("Reading the device host reply: {e}"))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteTool {
    #[serde(default)]
    required_version: String,
    #[serde(default)]
    installed_versions: Vec<String>,
    #[serde(default)]
    running_version: Option<String>,
}

impl RemoteTool {
    fn installed(&self) -> bool {
        self.installed_versions.contains(&self.required_version)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RemoteTools {
    #[serde(default)]
    hub: RemoteTool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Probe {
    #[serde(default)]
    node_version: String,
    #[serde(default)]
    npm_version: Option<String>,
    #[serde(default)]
    platforms: Vec<DevicePlatformAvailability>,
    #[serde(default)]
    tools: Option<RemoteTools>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteHelpers {
    #[serde(default)]
    serve_sim_ax_settings: Option<String>,
    #[serde(default)]
    serve_sim_cli: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Started {
    node_path: String,
    #[serde(default)]
    platforms: Vec<DevicePlatformAvailability>,
    #[serde(default)]
    tools: Option<RemoteTools>,
    hub_port: u16,
    #[serde(default)]
    daemon_port: Option<u16>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    android_home: Option<String>,
    #[serde(default)]
    helpers: RemoteHelpers,
}

fn tool_versions(tools: &RemoteTools) -> DeviceToolVersions {
    DeviceToolVersions {
        hub: DeviceToolVersion {
            required: tools.hub.required_version.clone(),
            installed: tools.hub.installed_versions.clone(),
            running: tools.hub.running_version.clone(),
        },
    }
}

fn node_major(version: &str) -> u32 {
    version
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|major| major.parse().ok())
        .unwrap_or(0)
}

fn probe_checks(probe: &Probe) -> Vec<DeviceHostCheck> {
    let node_ok = node_major(&probe.node_version) >= 22;
    let platform_ok = probe.platforms.iter().any(|platform| platform.available);
    vec![
        DeviceHostCheck {
            name: "SSH".into(),
            ok: true,
            detail: None,
        },
        DeviceHostCheck {
            name: "Node 22 or newer".into(),
            ok: node_ok,
            detail: Some(if node_ok {
                format!("Node {}", probe.node_version)
            } else {
                format!(
                    "Node {} is installed. Node 22 or newer is required on the device host.",
                    probe.node_version
                )
            }),
        },
        DeviceHostCheck {
            name: "npm".into(),
            ok: probe.npm_version.is_some(),
            detail: Some(match &probe.npm_version {
                Some(version) => format!("npm {version}"),
                None => "npm is missing from the non-interactive SSH PATH.".into(),
            }),
        },
        DeviceHostCheck {
            name: "Platform tools".into(),
            ok: platform_ok,
            detail: (!platform_ok).then(|| {
                probe
                    .platforms
                    .iter()
                    .filter_map(|platform| platform.reason.clone())
                    .collect::<Vec<_>>()
                    .join(" ")
            }),
        },
    ]
}

fn ready_failure(probe: &Probe) -> Option<String> {
    probe_checks(probe)
        .into_iter()
        .find(|check| !check.ok)
        .map(|check| check.detail.unwrap_or(check.name))
}

pub(crate) async fn test_host(
    ssh: &Path,
    config: &SshDeviceHostConfig,
    owner: &str,
    local: &HashSet<IpAddr>,
) -> DeviceHostTest {
    let failed = |name: &str, detail: String| DeviceHostTest {
        checks: vec![DeviceHostCheck {
            name: name.into(),
            ok: false,
            detail: Some(detail),
        }],
        ..Default::default()
    };
    if let Err(error) = check_identity(config) {
        return failed("Settings", error);
    }
    if is_local(ssh, config, local).await {
        return DeviceHostTest {
            local: true,
            ..Default::default()
        };
    }
    let ran = match bootstrap(ssh, config, owner, Mode::Probe).await {
        Ok(ran) => ran,
        Err(error) => return failed("SSH", error),
    };
    if ran.code == Some(255) || ran.code.is_none() {
        return failed("SSH", failure(&ran));
    }
    let probe = (ran.code == Some(0))
        .then(|| {
            serde_json::from_slice::<Probe>(String::from_utf8_lossy(&ran.stdout).trim().as_bytes())
                .ok()
        })
        .flatten();
    let Some(probe) = probe else {
        let mut test = failed("Node 22 or newer", failure(&ran));
        test.checks.insert(
            0,
            DeviceHostCheck {
                name: "SSH".into(),
                ok: true,
                detail: None,
            },
        );
        return test;
    };
    DeviceHostTest {
        local: false,
        checks: probe_checks(&probe),
        tools: probe.tools.as_ref().map(tool_versions),
        platforms: probe.platforms,
    }
}

pub(crate) fn parse_ssh_config(stdout: &str) -> Vec<(String, String)> {
    stdout
        .lines()
        .filter_map(|line| {
            let (key, value) = line.trim().split_once(' ')?;
            Some((key.to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect()
}

pub(crate) async fn is_local(
    ssh: &Path,
    config: &SshDeviceHostConfig,
    local: &HashSet<IpAddr>,
) -> bool {
    let Ok(ran) = run_ssh(
        ssh,
        ssh_args(config, &["-G"]),
        None,
        RESOLVE_TIMEOUT,
        1 << 20,
    )
    .await
    else {
        return false;
    };
    if ran.code != Some(0) {
        return false;
    }
    let resolved = parse_ssh_config(&String::from_utf8_lossy(&ran.stdout));
    let get = |key: &str| {
        resolved
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    // A local forwarded port or a proxy can lead to a different machine.
    if get("port") != Some("22")
        || ["proxycommand", "proxyjump"]
            .iter()
            .any(|key| get(key).is_some_and(|value| value != "none"))
    {
        return false;
    }
    let Some(hostname) = get("hostname").map(|name| name.trim_matches(['[', ']'])) else {
        return false;
    };
    let addresses: Vec<IpAddr> = match hostname.parse::<IpAddr>() {
        Ok(address) => vec![address],
        Err(_) => tokio::time::timeout(
            LOOKUP_TIMEOUT,
            tokio::net::lookup_host((hostname.to_string(), 22)),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .map(|found| found.map(|address| address.ip()).collect())
        .unwrap_or_default(),
    };
    !addresses.is_empty()
        && addresses
            .iter()
            .all(|address| address.is_loopback() || local.contains(address))
}

#[cfg(unix)]
pub(crate) fn local_addresses() -> HashSet<IpAddr> {
    let mut found = HashSet::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list`, which is walked read-only and released with freeifaddrs.
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return found;
        }
        let mut cursor = list;
        while !cursor.is_null() {
            let address = (*cursor).ifa_addr;
            if !address.is_null() {
                match i32::from((*address).sa_family) {
                    libc::AF_INET => {
                        let v4 = &*(address as *const libc::sockaddr_in);
                        found.insert(IpAddr::from(u32::from_be(v4.sin_addr.s_addr).to_be_bytes()));
                    }
                    libc::AF_INET6 => {
                        let v6 = &*(address as *const libc::sockaddr_in6);
                        found.insert(IpAddr::from(v6.sin6_addr.s6_addr));
                    }
                    _ => {}
                }
            }
            cursor = (*cursor).ifa_next;
        }
        libc::freeifaddrs(list);
    }
    found
}

#[cfg(not(unix))]
pub(crate) fn local_addresses() -> HashSet<IpAddr> {
    HashSet::new()
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteReady {
    pub origin: String,
    pub agent: Option<(String, String)>,
    pub helpers: Helpers,
    generation: u64,
}

pub(crate) enum HostEvent {
    Status(DeviceHostStatus, Option<String>),
    Lost,
    Ready(String),
}

type Events = Arc<dyn Fn(&SshHost, HostEvent) + Send + Sync>;

#[derive(Default)]
struct Inner {
    ready: Option<RemoteReady>,
    watcher: Option<tokio::task::JoinHandle<()>>,
    generation: u64,
    stopped: bool,
    retired: bool,
    activated: bool,
    wants_agent: bool,
    cancel: tokio_util::sync::CancellationToken,
    failed_at: Option<tokio::time::Instant>,
    summary: DeviceHubHost,
}

pub(crate) struct SshHost {
    pub config: SshDeviceHostConfig,
    owner: String,
    ssh: PathBuf,
    http: reqwest::Client,
    lock: tokio::sync::Mutex<()>,
    inner: Mutex<Inner>,
    events: Events,
    me: Weak<SshHost>,
}

impl SshHost {
    pub(crate) fn new(
        config: SshDeviceHostConfig,
        owner: String,
        ssh: PathBuf,
        http: reqwest::Client,
        events: Events,
    ) -> Arc<Self> {
        let summary = DeviceHubHost {
            id: config.id.clone(),
            kind: "ssh".into(),
            label: config.label.clone(),
            ..Default::default()
        };
        Arc::new_cyclic(|me| Self {
            config,
            owner,
            ssh,
            http,
            lock: tokio::sync::Mutex::new(()),
            inner: Mutex::new(Inner {
                summary,
                ..Default::default()
            }),
            events,
            me: me.clone(),
        })
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, event: HostEvent) {
        (self.events)(self, event);
    }

    pub(crate) fn summary(&self) -> DeviceHubHost {
        self.inner().summary.clone()
    }

    pub(crate) fn current(&self) -> Option<RemoteReady> {
        self.inner().ready.clone()
    }

    pub(crate) fn platform(&self, platform: DevicePlatform) -> DevicePlatformAvailability {
        self.summary()
            .platforms
            .into_iter()
            .find(|candidate| candidate.platform == platform)
            .unwrap_or(DevicePlatformAvailability {
                platform,
                available: false,
                reason: Some(HOST_UNREACHABLE.into()),
            })
    }

    fn remember(&self, platforms: Vec<DevicePlatformAvailability>, tools: Option<&RemoteTools>) {
        let mut inner = self.inner();
        inner.summary.platforms = platforms;
        if let Some(tools) = tools {
            inner.summary.tools = Some(tool_versions(tools));
            inner.summary.hub_installed = tools.hub.installed();
        }
    }

    pub(crate) async fn probe(&self) -> Result<DeviceHubHost, String> {
        let probe: Probe =
            bootstrap_json(&self.ssh, &self.config, &self.owner, Mode::Probe).await?;
        self.remember(probe.platforms.clone(), probe.tools.as_ref());
        if let Some(error) = ready_failure(&probe) {
            return Err(error);
        }
        Ok(self.summary())
    }

    async fn run_mode(&self, mode: Mode) -> Result<(), String> {
        let ran = bootstrap(&self.ssh, &self.config, &self.owner, mode).await?;
        if ran.code == Some(0) {
            Ok(())
        } else {
            Err(failure(&ran))
        }
    }

    fn teardown(&self) {
        let (watcher, lost) = {
            let mut inner = self.inner();
            (inner.watcher.take(), inner.ready.take().is_some())
        };
        if let Some(watcher) = watcher {
            watcher.abort();
        }
        if lost {
            self.emit(HostEvent::Lost);
        }
    }

    fn halted(&self) -> Result<(), String> {
        Self::halted_in(&self.inner())
    }

    fn halted_in(inner: &Inner) -> Result<(), String> {
        if inner.retired {
            Err(HOST_REMOVED.into())
        } else if inner.stopped {
            Err(super::DEVICE_SUPPORT_OFF.into())
        } else {
            Ok(())
        }
    }

    async fn cancellable<T>(
        &self,
        work: impl std::future::Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        let cancel = {
            let inner = self.inner();
            Self::halted_in(&inner)?;
            inner.cancel.clone()
        };
        tokio::select! {
            result = work => result,
            () = cancel.cancelled() => Err(self.halted().err().unwrap_or_else(|| HOST_REMOVED.into())),
        }
    }

    pub(crate) fn wants_connect(&self) -> bool {
        let inner = self.inner();
        !inner.stopped
            && !inner.retired
            && inner.ready.is_none()
            && inner.watcher.is_none()
            && inner
                .failed_at
                .is_none_or(|failed| failed.elapsed() >= RETRY_AFTER)
            && self.lock.try_lock().is_ok()
    }

    pub(crate) fn resume(&self) {
        let mut inner = self.inner();
        if !inner.retired {
            inner.stopped = false;
            inner.failed_at = None;
        }
    }

    pub(crate) async fn ensure_ready(&self) -> Result<RemoteReady, String> {
        let _lock = self.lock.lock().await;
        self.halted()?;
        if let Some(ready) = self.current() {
            return Ok(ready);
        }
        let result = self
            .cancellable(async {
                check_identity(&self.config)?;
                let summary = self.probe().await?;
                let installing = !summary.hub_installed;
                self.emit(HostEvent::Status(
                    if installing {
                        DeviceHostStatus::Installing
                    } else {
                        DeviceHostStatus::Starting
                    },
                    installing.then(|| "Installing device support...".to_string()),
                ));
                self.connect().await
            })
            .await;
        if let Err(error) = &result
            && self.halted().is_ok()
        {
            self.inner().failed_at = Some(tokio::time::Instant::now());
            self.emit(HostEvent::Status(
                DeviceHostStatus::Failed,
                Some(error.clone()),
            ));
        }
        result
    }

    pub(crate) async fn ensure_agent(&self) -> Result<(String, String), String> {
        self.ensure_ready().await?;
        let _lock = self.lock.lock().await;
        self.halted()?;
        self.inner().wants_agent = true;
        if let Some(agent) = self.current().and_then(|ready| ready.agent) {
            return Ok(agent);
        }
        self.cancellable(async {
            self.teardown();
            let connected = self.connect().await.and_then(|ready| {
                ready
                    .agent
                    .ok_or_else(|| "Starting agent tools: daemon endpoint missing.".to_string())
            });
            if connected.is_err() {
                self.teardown();
                self.inner().wants_agent = false;
                let _ = self.run_mode(Mode::StopAgent).await;
                if let Err(error) = self.connect().await {
                    self.emit(HostEvent::Status(DeviceHostStatus::Failed, Some(error)));
                }
            }
            connected
        })
        .await
    }

    pub(crate) async fn stop_agent(&self) {
        let _lock = self.lock.lock().await;
        self.inner().wants_agent = false;
        if self.halted().is_err() || self.current().is_none_or(|ready| ready.agent.is_none()) {
            return;
        }
        let _ = self
            .cancellable(async {
                self.teardown();
                if let Err(error) = self.run_mode(Mode::StopAgent).await {
                    tracing::warn!(%error, "stopping remote agent tools failed");
                }
                if let Err(error) = self.connect().await {
                    self.emit(HostEvent::Status(DeviceHostStatus::Failed, Some(error)));
                }
                Ok(())
            })
            .await;
    }

    pub(crate) async fn retire(&self) {
        self.inner().retired = true;
        self.stop().await;
    }

    pub(crate) fn signal_stop(&self) {
        {
            let mut inner = self.inner();
            inner.stopped = true;
            std::mem::take(&mut inner.cancel).cancel();
        }
        self.teardown();
    }

    pub(crate) async fn stop(&self) {
        self.signal_stop();
        let _lock = self.lock.lock().await;
        let (activated, retired) = {
            let mut inner = self.inner();
            if !inner.stopped {
                return;
            }
            inner.wants_agent = false;
            (std::mem::take(&mut inner.activated), inner.retired)
        };
        self.teardown();
        // A removed host may have started its hub under an earlier engine run.
        if (activated || retired)
            && let Err(error) = self.run_mode(Mode::Stop).await
        {
            tracing::warn!(%error, "stopping the remote device hub failed");
        }
    }

    async fn connect(&self) -> Result<RemoteReady, String> {
        let mut attempt = 0;
        loop {
            match self.connect_once().await {
                Ok(ready) => return Ok(ready),
                // ssh binds after the reservation is released, so a competing bind needs fresh ports.
                Err((true, _)) if attempt < 2 => attempt += 1,
                Err((_, error)) => return Err(error),
            }
        }
    }

    async fn connect_once(&self) -> Result<RemoteReady, (bool, String)> {
        self.halted().map_err(|error| (false, error))?;
        let mode = {
            let mut inner = self.inner();
            inner.activated = true;
            if inner.wants_agent {
                Mode::AgentStart
            } else {
                Mode::Start
            }
        };
        let started: Started = bootstrap_json(&self.ssh, &self.config, &self.owner, mode)
            .await
            .map_err(|error| (false, error))?;
        self.emit(HostEvent::Status(DeviceHostStatus::Starting, None));
        self.remember(started.platforms.clone(), started.tools.as_ref());
        self.inner().summary.hub_installed = true;
        let reserve =
            || hub_process::reserve_port().map_err(|e| (false, format!("Reserving a port: {e}")));
        let hub_port = reserve()?;
        let daemon = match (started.daemon_port, &started.token) {
            (Some(remote), Some(token)) => Some((reserve()?, remote, token.clone())),
            _ => None,
        };
        let args = forward_args(
            &self.config,
            (hub_port, started.hub_port),
            daemon.as_ref().map(|(local, remote, _)| (*local, *remote)),
        );
        let mut command = hub_process::lifeline_command(&self.ssh);
        command.args(&args).stdout(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|e| (true, format!("Forwarding ports: {e}")))?;
        let lifeline = child.stdin.take();
        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(pipe) = child.stderr.take() {
            let stderr = stderr.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt as _;
                let mut lines = tokio::io::BufReader::new(pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut text = stderr.lock().unwrap_or_else(PoisonError::into_inner);
                    text.push_str(&line);
                    text.push('\n');
                    *text = tail(&text, STDERR_TAIL);
                }
            });
        }
        let origin = format!("http://127.0.0.1:{hub_port}");
        let agent = daemon
            .as_ref()
            .map(|(local, _, token)| (format!("http://127.0.0.1:{local}"), token.clone()));
        let mut endpoints = vec![format!("{origin}/readyz")];
        endpoints.extend(agent.iter().map(|(base, _)| format!("{base}/health")));
        for url in endpoints {
            if !self.wait_http(&url, &mut child).await {
                let detail = stderr
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .trim()
                    .to_string();
                return Err((
                    true,
                    if detail.is_empty() {
                        "Waiting for SSH forward: Forwarded endpoint did not answer.".into()
                    } else {
                        format!("Waiting for SSH forward: {detail}")
                    },
                ));
            }
        }
        let helpers = Helpers {
            ax_settings: started.helpers.serve_sim_ax_settings,
            serve_sim: started
                .helpers
                .serve_sim_cli
                .map(|cli| (started.node_path.clone(), cli)),
            android_sdk: started.android_home.map(PathBuf::from),
        };
        self.halted().map_err(|error| (false, error))?;
        let ready = {
            let mut inner = self.inner();
            inner.generation += 1;
            let ready = RemoteReady {
                origin: origin.clone(),
                agent,
                helpers,
                generation: inner.generation,
            };
            inner.ready = Some(ready.clone());
            inner.watcher = Some(self.watch(child, lifeline, ready.clone()));
            ready
        };
        self.emit(HostEvent::Status(DeviceHostStatus::Ready, None));
        self.emit(HostEvent::Ready(origin));
        Ok(ready)
    }

    async fn wait_http(&self, url: &str, child: &mut tokio::process::Child) -> bool {
        let deadline = tokio::time::Instant::now() + FORWARD_READY;
        while tokio::time::Instant::now() < deadline {
            if !matches!(child.try_wait(), Ok(None)) {
                return false;
            }
            if self.healthy(url).await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    async fn healthy(&self, url: &str) -> bool {
        self.http
            .get(url)
            .timeout(HEALTH_TIMEOUT)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    fn watch(
        &self,
        mut child: tokio::process::Child,
        lifeline: Option<tokio::process::ChildStdin>,
        ready: RemoteReady,
    ) -> tokio::task::JoinHandle<()> {
        let me = self.me.clone();
        tokio::spawn(async move {
            let health = async {
                let mut misses = 0;
                loop {
                    tokio::time::sleep(HEALTH_EVERY).await;
                    let Some(host) = me.upgrade() else {
                        return;
                    };
                    let hub = host.healthy(&format!("{}/readyz", ready.origin)).await;
                    let daemon = match &ready.agent {
                        Some((base, _)) => host.healthy(&format!("{base}/health")).await,
                        None => true,
                    };
                    misses = if hub && daemon { 0 } else { misses + 1 };
                    if misses >= HEALTH_MISSES {
                        return;
                    }
                }
            };
            tokio::select! {
                _ = child.wait() => {}
                _ = health => {}
            }
            drop((child, lifeline));
            if let Some(host) = me.upgrade() {
                host.reconnect(ready.generation).await;
            }
        })
    }

    async fn reconnect(&self, generation: u64) {
        {
            let mut inner = self.inner();
            if inner.stopped || inner.generation != generation {
                return;
            }
            inner.ready = None;
        }
        self.emit(HostEvent::Lost);
        self.emit(HostEvent::Status(
            DeviceHostStatus::Starting,
            Some("Reconnecting to device host...".into()),
        ));
        let mut delay = Duration::from_secs(1);
        loop {
            tokio::time::sleep(delay).await;
            let _lock = self.lock.lock().await;
            {
                let inner = self.inner();
                if inner.stopped || inner.ready.is_some() || inner.generation != generation {
                    return;
                }
            }
            match self.cancellable(self.connect()).await {
                Ok(_) => return,
                Err(_) if self.halted().is_err() => return,
                Err(error) => {
                    self.emit(HostEvent::Status(DeviceHostStatus::Failed, Some(error)));
                    delay = (delay * 2).min(RECONNECT_MAX);
                }
            }
        }
    }

    pub(crate) fn runner(&self) -> Runner {
        let (ssh, config) = (self.ssh.clone(), self.config.clone());
        Arc::new(move |argv: Vec<String>| {
            let (ssh, config) = (ssh.clone(), config.clone());
            Box::pin(async move {
                let words: Vec<String> = argv.iter().map(|word| quote(word)).collect();
                match run_remote(
                    &ssh,
                    &config,
                    remote_input(&format!("exec {}", words.join(" "))),
                    super::actions::ACTION_TIMEOUT,
                    super::actions::OUTPUT_LIMIT,
                )
                .await
                {
                    Ok(ran) => CommandOutput {
                        code: ran.code.unwrap_or(-1),
                        stdout: String::from_utf8_lossy(&ran.stdout).into_owned(),
                        stderr: ran.stderr,
                    },
                    Err(error) => CommandOutput {
                        code: 127,
                        stdout: String::new(),
                        stderr: error,
                    },
                }
            }) as futures::future::BoxFuture<'static, CommandOutput>
        })
    }

    pub(crate) async fn screenshot(
        &self,
        platform: DevicePlatform,
        device_id: &str,
    ) -> Result<Vec<u8>, String> {
        let id = quote(device_id);
        let script = match platform {
            DevicePlatform::Ios => format!(
                "d=$(mktemp -d) || exit 1; xcrun simctl io {id} screenshot \"$d/screen.png\" >&2 && cat \"$d/screen.png\"; s=$?; rm -f \"$d/screen.png\"; rmdir \"$d\"; exit $s"
            ),
            DevicePlatform::Android => format!("exec adb -s {id} exec-out screencap -p"),
        };
        let ran = run_remote(
            &self.ssh,
            &self.config,
            remote_input(&script),
            Duration::from_secs(30),
            SCREENSHOT_LIMIT,
        )
        .await?;
        if ran.code != Some(0) || !ran.stdout.starts_with(b"\x89PNG") {
            return Err(format!("The device screenshot failed: {}", failure(&ran)));
        }
        Ok(ran.stdout)
    }
}

impl Drop for SshHost {
    fn drop(&mut self) {
        if let Some(watcher) = self.inner().watcher.take() {
            watcher.abort();
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
pub(super) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    const FAKE_SSH: &str = r#"
const fs = require('node:fs');
const net = require('node:net');
const [dir, ...args] = process.argv.slice(2);
fs.appendFileSync(dir + '/calls', JSON.stringify(args) + '\n');
const target = args[args.indexOf('--') + 1];
const configs = {
  'mac-mini': 'hostname 100.65.180.100\nport 22\n',
  remote: 'hostname 192.0.2.1\nport 22\n',
  loopback: 'hostname 127.0.1.1\nport 22\n',
  ipv6: 'hostname ::1\nport 22\n',
  forwarded: 'hostname 127.0.0.1\nport 2222\n',
  proxy: 'hostname 127.0.0.1\nport 22\nproxyjump bastion\n',
  command: 'hostname 127.0.0.1\nport 22\nproxycommand nc remote 22\n',
  unresolved: 'hostname example.invalid\nport 22\n',
};
if (args.includes('-G')) { process.stdout.write(configs[target] ?? 'hostname 192.0.2.7\nport 22\n'); process.exit(0); }
if (target === 'hang') {
  let input = '';
  process.stdin.on('data', chunk => { input += chunk; });
  process.stdin.on('end', () => fs.writeFileSync(dir + '/hang-' + /const mode = "([^"]+)"/.exec(input)?.[1] + '.pid', String(process.pid)));
  setInterval(() => {}, 1 << 30);
  return;
}
if (target === 'unreachable') { console.error('ssh: connect to host unreachable port 22: Connection refused'); process.exit(255); }
if (target === 'nonode') { console.error('Node is missing from the non-interactive SSH PATH'); process.exit(1); }
if (args.includes('-N')) {
  if (fs.existsSync(dir + '/fail-forward-once')) { fs.unlinkSync(dir + '/fail-forward-once'); console.error('bind [127.0.0.1]: Address already in use'); process.exit(255); }
  fs.writeFileSync(dir + '/forward.pid', String(process.pid));
  args.forEach((arg, index) => {
    if (arg !== '-L') return;
    const [, local, , remote] = args[index + 1].split(':');
    net.createServer(socket => { const up = net.connect(Number(remote), '127.0.0.1'); socket.pipe(up).pipe(socket); up.on('error', () => socket.destroy()); socket.on('error', () => up.destroy()); }).listen(Number(local), '127.0.0.1');
  });
  return;
}
if (args.slice(args.indexOf('--') + 2).join(' ') !== 'sh -s') { console.error('expected sh -s'); process.exit(2); }
let input = '';
process.stdin.on('data', chunk => { input += chunk; });
process.stdin.on('end', () => {
  const script = /\nexec node <<'ZERON_DEVICE_SCRIPT_END'\n([\s\S]*)\nZERON_DEVICE_SCRIPT_END\n$/.exec(input)?.[1];
  if (script === undefined) {
    const ran = require('node:child_process').spawnSync('/bin/sh', ['-s'], { input, stdio: ['pipe', 'inherit', 'inherit'] });
    process.exit(ran.status ?? 1);
  }
  const mode = /const mode = "([^"]+)"/.exec(script)?.[1];
  fs.appendFileSync(dir + '/modes', mode + ' ' + /const owner = "([^"]+)"/.exec(script)?.[1] + ' ' + target + '\n');
  const platforms = [{ platform: 'ios', available: true }, { platform: 'android', available: false, reason: 'Android SDK missing. Set ANDROID_HOME or put adb on the SSH PATH.' }];
  const tool = { requiredVersion: '0.12.0', installedVersions: [], runningVersion: null };
  const portFile = fs.existsSync(dir + '/hub-port-' + target) ? dir + '/hub-port-' + target : dir + '/hub-port';
  const hubPort = Number(fs.readFileSync(portFile, 'utf8'));
  if (mode === 'agent-start' && fs.existsSync(dir + '/fail-agent-once')) { fs.unlinkSync(dir + '/fail-agent-once'); console.error('agent-device daemon did not become ready'); process.exit(1); }
  if (mode === 'probe') console.log(JSON.stringify({ nodePath: '/node', nodeVersion: target === 'oldnode' ? '20.11.0' : '22.22.3', npmVersion: '10.9.0', platforms, tools: { hub: tool, agent: tool } }));
  else if (mode === 'start' || mode === 'agent-start') console.log(JSON.stringify({ nodePath: '/node', platforms, hubPort, helpers: { serveSimAxSettings: null, serveSimCli: '/remote/serve-sim.js' }, androidHome: null, ...(mode === 'agent-start' ? { daemonPort: hubPort, token: 'fixture', entryPath: '/agent.mjs' } : {}) }));
});
"#;

    pub(crate) fn host(target: &str) -> SshDeviceHostConfig {
        SshDeviceHostConfig {
            id: target.into(),
            label: target.into(),
            target: target.into(),
            ..Default::default()
        }
    }

    pub(crate) struct Fake {
        pub dir: tempfile::TempDir,
        pub ssh: PathBuf,
    }

    impl Fake {
        pub(crate) fn lines(&self, name: &str) -> Vec<String> {
            std::fs::read_to_string(self.dir.path().join(name))
                .unwrap_or_default()
                .lines()
                .map(String::from)
                .collect()
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.lines("calls")
                .iter()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }

        pub(crate) fn modes(&self) -> Vec<String> {
            self.lines("modes")
                .iter()
                .map(|line| line.split(' ').next().unwrap().to_string())
                .collect()
        }
    }

    pub(crate) async fn fake_ssh() -> Option<Fake> {
        let Some(node) = super::super::agent::tests::node() else {
            eprintln!("node not found; skipping");
            return None;
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-ssh.js");
        std::fs::write(&script, FAKE_SSH).unwrap();
        let ssh = dir.path().join("ssh");
        std::fs::write(
            &ssh,
            format!(
                "#!/bin/sh\nexec {} {} {} \"$@\"\n",
                quote(&node.to_string_lossy()),
                quote(&script.to_string_lossy()),
                quote(&dir.path().to_string_lossy())
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        std::fs::write(
            dir.path().join("hub-port"),
            listener.local_addr().unwrap().port().to_string(),
        )
        .unwrap();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .await;
                });
            }
        });
        Some(Fake { dir, ssh })
    }

    fn http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    async fn eventually(mut done: impl FnMut() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("condition never held");
    }

    pub(crate) fn exited(pid: i32) -> bool {
        let out = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let stat = String::from_utf8_lossy(&out.stdout);
        stat.trim().is_empty() || stat.trim_start().starts_with('Z')
    }

    #[test]
    fn ssh_argv_is_batch_mode_with_the_destination_after_a_double_dash() {
        let config = SshDeviceHostConfig {
            identity_file: Some("/Users/me/.ssh/id ed25519".into()),
            port: Some(2222),
            ..host("me@mini.local")
        };
        assert_eq!(
            ssh_args(&config, &["-G"]),
            [
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "ControlMaster=no",
                "-o",
                "ControlPath=none",
                "-p",
                "2222",
                "-i",
                "/Users/me/.ssh/id ed25519",
                "-G",
                "--",
                "me@mini.local"
            ]
        );
        assert_eq!(
            forward_args(&host("mini"), (4100, 5100), Some((4101, 5101))),
            [
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "ControlMaster=no",
                "-o",
                "ControlPath=none",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "ServerAliveInterval=10",
                "-o",
                "ServerAliveCountMax=3",
                "-N",
                "-L",
                "127.0.0.1:4100:127.0.0.1:5100",
                "-L",
                "127.0.0.1:4101:127.0.0.1:5101",
                "--",
                "mini"
            ]
        );
        let input = bootstrap_input("o", Mode::Probe);
        assert!(input.starts_with(REMOTE_ENV));
        assert!(input.contains("\nexec node <<'ZERON_DEVICE_SCRIPT_END'\nconst owner = \"o\";"));
        assert!(input.ends_with("\nZERON_DEVICE_SCRIPT_END\n"));
        assert_eq!(
            input.lines().filter(|line| *line == SCRIPT_END).count(),
            1,
            "the script never ends its own heredoc early"
        );
        assert_eq!(
            redact(
                "npm error 401 https://me:hunter2@registry.example/x //registry/:_authToken=npm_abc next"
            ),
            "npm error 401 https://<redacted>@registry.example/x //registry/:_authToken=<redacted> next"
        );
        assert!(check_identity(&host("-oProxyCommand=x")).is_err());
        let missing = SshDeviceHostConfig {
            identity_file: Some("/nonexistent/zeron-key".into()),
            ..host("mini")
        };
        assert_eq!(
            check_identity(&missing),
            Err("The identity file /nonexistent/zeron-key does not exist.".into())
        );
    }

    #[test]
    fn the_script_carries_the_owner_mode_pins_and_guard_as_json_literals() {
        let script = remote_script("o\"wner", Mode::AgentStart);
        assert!(
            script.starts_with("const owner = \"o\\\"wner\";\nconst mode = \"agent-start\";\n")
        );
        assert!(script.contains(&format!(
            "const hubVersion = \"{}\";",
            toolchain::hub_version()
        )));
        assert!(script.contains(&format!(
            "const agentVersion = \"{}\";",
            toolchain::agent_version()
        )));
        assert!(script.contains(&format!(
            "const guardSource = {};",
            serde_json::to_string(hub_process::GUARD).unwrap()
        )));
        assert!(script.contains("path.join(os.homedir(), '.zeron', 'device')"));
        assert_eq!(owner_for(Path::new("/data"), "mini").len(), 24);
        assert_ne!(
            owner_for(Path::new("/data"), "mini"),
            owner_for(Path::new("/data2"), "mini")
        );
    }

    #[tokio::test]
    async fn remote_arguments_keep_shell_metacharacters_and_the_env_finds_studio_java() {
        let value = "quotes ' \" ; $(echo expanded) $HOME\nnext line";
        let out = tokio::process::Command::new("sh")
            .args(["-c", &format!("printf %s {}", quote(value))])
            .output()
            .await
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), value);
        let home = tempfile::tempdir().unwrap();
        let java = home.path().join(".local/opt/android-studio/jbr/bin/java");
        std::fs::create_dir_all(java.parent().unwrap()).unwrap();
        std::fs::write(&java, "#!/bin/sh\necho test-java\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = tokio::process::Command::new("/bin/sh")
            .args(["-c", &format!("{REMOTE_ENV}\njava")])
            .env("HOME", home.path())
            .env("PATH", "/nonexistent")
            .env("JAVA_HOME", "")
            .output()
            .await
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "test-java");
    }

    #[tokio::test]
    async fn aliases_resolving_to_this_machine_are_local_and_proxies_or_forwards_are_not() {
        let Some(fake) = fake_ssh().await else {
            return;
        };
        let local = HashSet::from(["100.65.180.100".parse().unwrap()]);
        for target in ["mac-mini", "loopback", "ipv6"] {
            assert!(is_local(&fake.ssh, &host(target), &local).await, "{target}");
        }
        for target in ["remote", "forwarded", "proxy", "command", "unresolved"] {
            assert!(
                !is_local(&fake.ssh, &host(target), &local).await,
                "{target}"
            );
        }
        assert!(
            fake.calls().iter().all(|args| args.contains(&"-G".into())),
            "resolving never connects"
        );
        assert!(!local_addresses().is_empty());
    }

    #[tokio::test]
    async fn test_connection_reports_each_check_and_skips_this_machine() {
        let Some(fake) = fake_ssh().await else {
            return;
        };
        let local = HashSet::new();
        let names = |test: &DeviceHostTest| -> Vec<(String, bool)> {
            test.checks
                .iter()
                .map(|check| (check.name.clone(), check.ok))
                .collect()
        };
        let skipped = test_host(&fake.ssh, &host("loopback"), "o", &local).await;
        assert!(skipped.local && skipped.passed() && skipped.checks.is_empty());
        assert!(fake.modes().is_empty(), "a local target is never probed");

        let down = test_host(&fake.ssh, &host("unreachable"), "o", &local).await;
        assert_eq!(names(&down), [("SSH".to_string(), false)]);
        assert!(
            down.checks[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("Connection refused")
        );

        let no_node = test_host(&fake.ssh, &host("nonode"), "o", &local).await;
        assert_eq!(
            names(&no_node),
            [
                ("SSH".to_string(), true),
                ("Node 22 or newer".to_string(), false)
            ]
        );
        assert_eq!(no_node.checks[1].detail.as_deref(), Some(NODE_MISSING));

        let old = test_host(&fake.ssh, &host("oldnode"), "o", &local).await;
        assert!(!old.passed());
        assert_eq!(names(&old)[1], ("Node 22 or newer".to_string(), false));

        let good = test_host(&fake.ssh, &host("mini"), "o", &local).await;
        assert!(good.passed(), "{good:?}");
        assert_eq!(
            names(&good),
            [
                ("SSH".to_string(), true),
                ("Node 22 or newer".to_string(), true),
                ("npm".to_string(), true),
                ("Platform tools".to_string(), true)
            ]
        );
        assert_eq!(good.platforms.len(), 2);
        assert_eq!(good.tools.unwrap().hub.required, "0.12.0");
        assert!(
            fake.modes().iter().all(|mode| mode == "probe"),
            "testing installs nothing"
        );

        let bad_key = SshDeviceHostConfig {
            identity_file: Some("/nonexistent/key".into()),
            ..host("mini")
        };
        assert!(!test_host(&fake.ssh, &bad_key, "o", &local).await.passed());
    }

    #[tokio::test]
    async fn host_commands_reach_the_remote_shell_as_quoted_words() {
        let Some(fake) = fake_ssh().await else {
            return;
        };
        let ssh_host = SshHost::new(
            host("mini"),
            "o".into(),
            fake.ssh.clone(),
            http(),
            Arc::new(|_: &SshHost, _| {}),
        );
        let word = "a b'c $(touch /tmp/zeron-pwned) ;";
        let output = ssh_host.runner()(vec![
            "printf".into(),
            "%s|%s".into(),
            word.into(),
            "$HOME".into(),
        ])
        .await;
        assert_eq!(output.code, 0, "{}", output.stderr);
        assert_eq!(output.stdout, format!("{word}|$HOME"));
        let failed = ssh_host.runner()(vec![
            "sh".into(),
            "-c".into(),
            "echo nope >&2; exit 3".into(),
        ])
        .await;
        assert_eq!((failed.code, failed.stderr.trim()), (3, "nope"));
        let last = fake.calls().pop().unwrap();
        assert_eq!(
            last[..10],
            [
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "ControlMaster=no",
                "-o",
                "ControlPath=none",
                "--",
                "mini"
            ]
        );
        assert!(
            fake.calls().iter().all(|args| args
                .windows(2)
                .any(|pair| pair == ["-o", "ControlPath=none"])),
            "no call reuses or creates a shared connection"
        );
        assert_eq!(
            last[10..],
            ["sh", "-s"],
            "the command words travel on stdin"
        );
    }

    #[tokio::test]
    async fn the_forward_retries_reconnects_after_it_dies_and_stops_the_remote_helpers() {
        let Some(fake) = fake_ssh().await else {
            return;
        };
        std::fs::write(fake.dir.path().join("fail-forward-once"), "").unwrap();
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        let seen = events.clone();
        let ssh_host = SshHost::new(
            host("mini"),
            "owner-1".into(),
            fake.ssh.clone(),
            http(),
            Arc::new(move |host: &SshHost, event| {
                let id = &host.config.id;
                let text = match event {
                    HostEvent::Status(status, _) => format!("{id} {status:?}"),
                    HostEvent::Lost => format!("{id} lost"),
                    HostEvent::Ready(origin) => format!("{id} ready {origin}"),
                };
                seen.lock().unwrap().push(text);
            }),
        );
        let ready = ssh_host.ensure_ready().await.unwrap();
        let hub_port = std::fs::read_to_string(fake.dir.path().join("hub-port")).unwrap();
        assert_ne!(ready.origin, format!("http://127.0.0.1:{hub_port}"));
        assert_eq!(
            http()
                .get(format!("{}/readyz", ready.origin))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            fake.modes(),
            ["probe", "start", "start"],
            "a failed bind reruns the idempotent start, as t3code does"
        );
        let forwards: Vec<Vec<String>> = fake
            .calls()
            .into_iter()
            .filter(|args| args.contains(&"-N".into()))
            .collect();
        assert_eq!(forwards.len(), 2, "a failed bind retries with fresh ports");
        assert_ne!(forwards[0], forwards[1]);
        assert!(forwards[1].contains(&format!(
            "127.0.0.1:{}:127.0.0.1:{hub_port}",
            ready.origin.rsplit(':').next().unwrap()
        )));
        assert_eq!(forwards[1].last().map(String::as_str), Some("mini"));
        assert_eq!(
            ready.helpers.serve_sim.as_ref().unwrap().1,
            "/remote/serve-sim.js"
        );

        let pid: i32 = std::fs::read_to_string(fake.dir.path().join("forward.pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            unsafe { libc::getpgid(pid) },
            pid,
            "the forward runs in its own lifeline group"
        );
        unsafe { libc::kill(pid, libc::SIGKILL) };
        let host_ref = ssh_host.clone();
        let first = ready.origin.clone();
        eventually(move || {
            host_ref
                .current()
                .is_some_and(|ready| ready.origin != first)
        })
        .await;
        let again = ssh_host.current().unwrap();
        assert_eq!(
            http()
                .get(format!("{}/readyz", again.origin))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        {
            let log = events.lock().unwrap();
            let lost = log.iter().position(|e| e == "mini lost").unwrap();
            assert!(log[lost..].contains(&format!("mini ready {}", again.origin)));
            assert!(log[lost..].contains(&"mini Starting".to_string()));
        }
        assert_eq!(fake.modes(), ["probe", "start", "start", "start"]);

        let (url, token) = ssh_host.ensure_agent().await.unwrap();
        assert_eq!(token, "fixture");
        assert_eq!(fake.modes().last().unwrap(), "agent-start");
        assert_eq!(
            http()
                .get(format!("{url}/health"))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        ssh_host.stop_agent().await;
        assert_eq!(
            fake.modes()[fake.modes().len() - 2..],
            ["stop-agent", "start"]
        );
        assert!(ssh_host.current().unwrap().agent.is_none());

        let last = ssh_host.current().unwrap().origin;
        ssh_host.retire().await;
        assert_eq!(fake.modes().last().unwrap(), "stop");
        assert!(ssh_host.current().is_none());
        let refused = AtomicBool::new(false);
        for _ in 0..40 {
            if http().get(format!("{last}/readyz")).send().await.is_err() {
                refused.store(true, Ordering::SeqCst);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            refused.load(Ordering::SeqCst),
            "the forward dies with the host"
        );
        assert_eq!(
            ssh_host.ensure_ready().await.err().as_deref(),
            Some(HOST_REMOVED)
        );
        assert!(
            fake.lines("modes")
                .iter()
                .all(|line| line.split(' ').nth(1) == Some("owner-1")),
            "every call runs under this engine's owner"
        );
    }

    #[tokio::test]
    async fn a_failed_agent_start_restores_the_plain_hub_and_a_stopped_host_never_restarts() {
        let Some(fake) = fake_ssh().await else {
            return;
        };
        let ssh_host = SshHost::new(
            host("mini"),
            "o".into(),
            fake.ssh.clone(),
            http(),
            Arc::new(|_: &SshHost, _| {}),
        );
        ssh_host.ensure_ready().await.unwrap();
        std::fs::write(fake.dir.path().join("fail-agent-once"), "").unwrap();
        assert!(ssh_host.ensure_agent().await.is_err());
        assert!(
            ssh_host
                .current()
                .is_some_and(|ready| ready.agent.is_none()),
            "the host stays connected without agent tools"
        );
        assert_eq!(
            fake.modes()[fake.modes().len() - 3..],
            ["agent-start", "stop-agent", "start"]
        );
        ssh_host.ensure_agent().await.unwrap();
        ssh_host.signal_stop();
        let before = fake.modes().len();
        ssh_host.stop_agent().await;
        assert!(ssh_host.ensure_ready().await.is_err());
        assert_eq!(
            fake.modes().len(),
            before,
            "a stopped host neither reconnects nor restarts its hub"
        );
        ssh_host.stop().await;
        assert_eq!(fake.modes().last().unwrap(), "stop");

        let fresh = SshHost::new(
            host("studio"),
            "o".into(),
            fake.ssh.clone(),
            http(),
            Arc::new(|_: &SshHost, _| {}),
        );
        fresh.retire().await;
        assert_eq!(
            fake.lines("modes").last().unwrap(),
            "stop o studio",
            "removing a host stops a hub an earlier engine run started"
        );
    }

    #[tokio::test]
    async fn stopping_cancels_an_install_in_flight_and_kills_its_ssh() {
        let Some(fake) = fake_ssh().await else {
            return;
        };
        let ssh_host = SshHost::new(
            host("hang"),
            "o".into(),
            fake.ssh.clone(),
            http(),
            Arc::new(|_: &SshHost, _| {}),
        );
        let connecting = tokio::spawn({
            let ssh_host = ssh_host.clone();
            async move { ssh_host.ensure_ready().await }
        });
        let pid_file = fake.dir.path().join("hang-probe.pid");
        eventually(|| pid_file.exists()).await;
        let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
        assert!(!ssh_host.wants_connect(), "a connect is already in flight");
        let started = std::time::Instant::now();
        ssh_host.signal_stop();
        assert!(connecting.await.unwrap().is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
        eventually(|| exited(pid)).await;
        assert!(!ssh_host.wants_connect(), "a stopped host waits for resume");
        ssh_host.resume();
        assert!(ssh_host.wants_connect());
    }

    #[tokio::test]
    async fn the_remote_script_runs_a_guarded_hub_per_owner_and_stops_only_its_own() {
        let Some(node) = super::super::agent::tests::node() else {
            return;
        };
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(bin.join("adb"), "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(bin.join("adb"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let version = toolchain::hub_version();
        let tool = home
            .path()
            .join(format!(".zeron/device/tools/expo-device-hub@{version}"));
        let entry = tool.join("node_modules/expo-device-hub/dist/server/cli.mjs");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(tool.join(".install-complete"), version).unwrap();
        std::fs::write(
            &entry,
            "import http from 'node:http';\nconst args = process.argv.slice(2);\nhttp.createServer((req, res) => res.end('ok')).listen(Number(args[args.indexOf('--port') + 1]), '127.0.0.1');\n",
        )
        .unwrap();
        let path = format!(
            "{}:{}:{}",
            bin.display(),
            node.parent().unwrap().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let invoke = |owner: &'static str, mode: Mode| {
            let (home, path) = (home.path().to_path_buf(), path.clone());
            async move {
                let mut child = tokio::process::Command::new("/bin/sh")
                    .arg("-s")
                    .env("HOME", home)
                    .env("PATH", path)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                let mut stdin = child.stdin.take().unwrap();
                stdin
                    .write_all(bootstrap_input(owner, mode).as_bytes())
                    .await
                    .unwrap();
                drop(stdin);
                let out = child.wait_with_output().await.unwrap();
                assert!(
                    out.status.success(),
                    "{mode:?}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap_or_default()
            }
        };
        let probe = invoke("one", Mode::Probe).await;
        assert_eq!(
            probe["tools"]["hub"]["installedVersions"],
            serde_json::json!([version])
        );
        assert_eq!(probe["platforms"][1]["available"], true);
        assert!(probe["nodeVersion"].as_str().is_some());
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for owner in ["one", "two"] {
                    let file = self.0.join(format!(".zeron/device/hosts/{owner}/hub.json"));
                    if let Some(pid) = std::fs::read_to_string(file)
                        .ok()
                        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                        .and_then(|hub| hub["pid"].as_i64())
                    {
                        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
                    }
                }
            }
        }
        let _cleanup = Cleanup(home.path().to_path_buf());
        let decoy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let decoy_port = decoy.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = decoy.accept().await {
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
            }
        });
        let stranger = tokio::process::Command::new("sleep")
            .arg("60")
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let state = home.path().join(".zeron/device/hosts/one");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("hub.json"),
            serde_json::json!({"owner": "one", "pid": stranger.id(),
                "port": decoy_port, "entryPath": entry})
            .to_string(),
        )
        .unwrap();
        {
            let (first, concurrent) =
                tokio::join!(invoke("one", Mode::Start), invoke("one", Mode::Start));
            assert_eq!(first["hubPort"], concurrent["hubPort"]);
            assert_ne!(
                first["hubPort"], decoy_port,
                "a healthy listener that is not this owner's hub is never reused"
            );
            let one = format!("http://127.0.0.1:{}/readyz", first["hubPort"]);
            assert_eq!(http().get(&one).send().await.unwrap().status(), 200);
            assert_eq!(
                http()
                    .get(&one)
                    .header("Origin", "http://evil.example")
                    .send()
                    .await
                    .unwrap()
                    .status(),
                403,
                "the remote hub runs behind the same guard"
            );
            let guard = home.path().join(".zeron/device/hosts/one/hub-guard.mjs");
            assert_eq!(std::fs::read_to_string(&guard).unwrap(), hub_process::GUARD);
            assert_eq!(
                std::fs::metadata(&guard).unwrap().permissions().mode() & 0o777,
                0o600
            );
            let second = invoke("two", Mode::Start).await;
            assert_ne!(second["hubPort"], first["hubPort"]);
            let running = invoke("one", Mode::Probe).await;
            assert_eq!(running["tools"]["hub"]["runningVersion"], version);
            invoke("one", Mode::Stop).await;
            let two = format!("http://127.0.0.1:{}/readyz", second["hubPort"]);
            let mut gone = false;
            for _ in 0..40 {
                if http().get(&one).send().await.is_err() {
                    gone = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert!(gone, "stop ends this owner's hub");
            assert_eq!(http().get(&two).send().await.unwrap().status(), 200);
        }
        invoke("two", Mode::Stop).await;
    }
}
