`zeron` is one program: the desktop app, the engine that runs your agents, and a command line for signing in, checking status, updating, and running Zeron on a server without a window.

## Where the command lives

| System | Where `zeron` is |
| --- | --- |
| macOS | Inside the app: `/Applications/Zeron.app/Contents/MacOS/zeron`. Zeron does not add it to your PATH; call it by that path or make your own symlink. |
| Linux | The installer puts it in `~/.local/bin/zeron`. If that folder is not on your PATH, the installer says so: add it. |
| Windows | The installer puts `zeron.exe` in `%LOCALAPPDATA%\Programs\Zeron` and does not add it to your PATH. |

## Opening the app

Running `zeron` with no command opens the desktop app. One engine serves each machine: if one is already running (the background service, `zeron headless`, or another open Zeron window), the app connects to it. Otherwise the app starts its own engine, and other windows and commands on this machine connect to that one.

`zeron <URL>` opens Zeron at a conversation link. See **Conversation links** below.

## Commands

| Command | What it does |
| --- | --- |
| `zeron headless` | Runs the engine with no window. It stays local-only unless you signed in with `zeron login`. |
| `zeron login` | Signs in and turns on sync for the next engine start. |
| `zeron logout` | Removes the saved sign-in. The next engine start uses the local-only workspace. |
| `zeron status` | Shows the data folder, the workspace mode, the sign-in, and whether an engine is running. |
| `zeron sync` | Shows live sync state from the running engine. |
| `zeron update` | Downloads and installs a newer release. `zeron update --check` only reports. |
| `zeron daemon …` | Runs `zeron headless` as a background service. |
| `zeron appshot` | Linux only: takes an Appshot in the running Zeron app. |

`zeron --help` lists them, and `zeron <command> --help` explains one. The list also shows `zeron ui`, a developer tool for driving the app. Zeron has a few more commands it runs for itself, such as the tool server it starts next to each agent; you never need to run those.

### Sign in and out

`zeron login` needs an interactive terminal. It prints `Sign in to Zeron:` with a link; open it, sign in, then paste the code the browser shows and press Enter. If your account has no workspace yet, it asks you to name one; with several, it lists them and asks you to pick (`Pick a workspace [1-N]:`). When it finishes, it prints `Signed in as <email>` and `Sync is ready.` Start or restart Zeron to open the synced workspace; sessions you already had stay in the local workspace.

`zeron login` and `zeron logout` refuse while an engine is running, because the running engine owns the sign-in. Stop it first (`zeron daemon stop`, or quit the Zeron app), or use the account menu in the app instead. If you are already signed in, `zeron login` says so; run `zeron logout` first to switch accounts.

### Status

`zeron status` prints:

- `Mode:` `local only`, `synced`, or `development`.
- `Auth:` for example `signed out (optional in local-only mode)`, `signed in as you@example.com; sync is ready after restart`, or `signed in as you@example.com (workspace …)`.
- `Engine:` `running (pid N)` or `not running`.
- `IPC:` whether something listens on the engine port.

It exits with an error only when a synced workspace has a problem: the saved session is no longer valid, or no workspace is selected. Both tell you to run `zeron login`. Being signed out in local-only mode is normal.

### Sync

`zeron sync` asks the running engine how sync is going: one line for the workspace and one per open chat, each `connected` or `DISCONNECTED`. The workspace line says when changes were last pushed and acknowledged; a chat line shows how far it has caught up and how many of its changes are still waiting to send. A count marked `REJECTED` means the server refused changes. With no engine running it says `no engine listening on 127.0.0.1:<port>`.

### Update

`zeron update` checks for a newer release and installs it:

- On Linux it downloads the new version, switches to it, and restarts the background service.
- On macOS it replaces `Zeron.app`; relaunch Zeron to finish.
- On Windows it replaces `zeron.exe`; relaunch Zeron to finish.
- A copy Zeron did not install (for example, built from source) cannot update itself; the command explains where to get a release.

`zeron update --check` only prints `zeron X is up to date` or `zeron X → Y available`, and exits with an error code when an update is available, so scripts can test for one. For updates inside the app, see [Updates and what's new](zeron://docs/updates).

### Background service

`zeron daemon` manages `zeron headless` as a service that starts on its own: a launchd agent (`sh.zeron.app`) on macOS, a systemd user unit (`zeron.service`) on Linux. Windows is not supported.

| Command | What it does |
| --- | --- |
| `zeron daemon install` | Installs, enables, and starts the service. |
| `zeron daemon uninstall` | Stops and removes it. |
| `zeron daemon start` / `stop` / `restart` | Controls the installed service. |
| `zeron daemon status` | Shows whether it is loaded and running. |

`install` copies your `PATH` into the service, along with these variables when they are set in your shell: `ZERON_DATA_DIR`, `ZERON_IPC_PORT`, `ZERON_HARNESS`, `ZERON_DEVICE_NAME`, `RUST_LOG`, and the developer variables for a custom sync server and sign-in (`ZERON_EDGE_URL`, `ZERON_EDGE_TOKEN`, `ZERON_ORG_ID`, `ZERON_WORKOS_CLIENT_ID`, `ZERON_WORKOS_API_BASE`, `ZERON_CALLBACK_PORT`). Set them before you install. Other variables reach the service only through `~/.zeron/env`, which the Linux service reads. Logs go to `daemon.log` in the data folder on macOS, and to `journalctl --user -u zeron.service` on Linux.

When the service is running, the desktop app on the same machine connects to it instead of starting its own engine.

## Run Zeron on a server

A Linux server can run your agents while you drive them from your computer or phone.

1. Install: `curl -fsSL https://zeron.sh/install.sh | sh`. This installs `zeron`, sets up `zeron.service` as a systemd user service, starts it, and turns on lingering so the engine keeps running after you log out. If it cannot turn on lingering, run `sudo loginctl enable-linger $USER`. Without systemd it tells you to run `zeron headless` yourself.
2. Install the agents you want on the server (Claude Code, Codex, and so on) and sign in to them there. The installer reminds you when the Claude Code CLI is missing.
3. Sign in so the server joins your synced workspace. The service starts local-only, so stop it, sign in, and start it again (the commands are below).

```
systemctl --user stop zeron
zeron login
systemctl --user restart zeron
```

If you set up the service with `zeron daemon install`, use `zeron daemon stop` and `zeron daemon restart` instead.

Use the same account and workspace as your desktop. The server then appears in [Settings > Devices](zeron://settings/devices) under **Other devices**. To work there, add a project and pick the server as its device, or pick the server in the device chip on the new chat canvas. See [Devices and sync](zeron://docs/devices). The desktop app reaches a server only through the synced workspace; there is no direct connection to type in.

To keep the server current, run `zeron update`, or set `ZERON_AUTO_UPDATE=1` (in `~/.zeron/env`) to let the engine install new releases on its own when no run or terminal is active.

## Environment variables

| Variable | What it does |
| --- | --- |
| `ZERON_DATA_DIR` | Where Zeron keeps its data. See below. |
| `ZERON_IPC_PORT` | The local port the engine listens on (default 27654). The app, commands, and engine on one machine must agree on it. |
| `ZERON_DEVICE_NAME` | The name a new device registers with. Otherwise Zeron uses the system's computer name or hostname. A name you set with **Rename** in Settings > Devices is kept. |
| `ZERON_HARNESS` | For `zeron headless`: the agent for chats that have none set: `codex`, `cursor`, `devin`, `grok`, `hermes`, `pi`, or `antigravity`. Unset means Claude Code. |
| `ZERON_AUTO_UPDATE` | `1`, `true`, or `yes` turns on automatic updates. A Linux engine installed with `install.sh` installs them when idle (set it in `~/.zeron/env`). The desktop app downloads them in the background and installs a downloaded update when you quit. Off by default. |
| `ZERON_WORKTREES_DIR` | Where new worktrees go. Overrides the location set in Settings. |
| `ZERON_DISABLE_NOTIFICATIONS` | Any value turns off desktop banners, whatever Settings say. |
| `ZERON_DISABLE_SOUND` | Any value turns off every Zeron sound, whatever Settings say. |
| `ZERON_NO_LOGIN_SHELL` | Any non-empty value stops Zeron from reading `PATH` and other variables from your login shell when it starts agents. |
| `RUST_LOG` | Log detail, for example `RUST_LOG=debug`. |

These apply to the process they are set for. The desktop app sees them when you start it as `zeron` from a terminal that sets them; the background service sees only the variables `zeron daemon install` captured (listed above), plus `~/.zeron/env` on Linux.

## The data folder

Zeron keeps sessions, settings, the saved sign-in (`session.json`), and logs in one folder:

- macOS and Linux: `~/.zeron`
- Windows: `%LOCALAPPDATA%\Zeron`

`ZERON_DATA_DIR` moves it. Logs are in its `logs` folder: `zeron-headed.log` for the app and `zeron-headless.log` for `zeron headless`, each with the previous run kept as `.log.old`.

## Conversation links

Right-click a session in the sidebar and choose **Copy** > **Zeron conversation link** to copy a link like `zeron://open/chat/<id>?workspace=<code>`. Zeron confirms with "Zeron conversation link copied". For a Codex session, **Codex conversation link** copies a `codex://threads/...` link that opens the conversation in Codex.

Clicking a Zeron link anywhere on your computer opens Zeron and selects that conversation; Zeron registers itself for `zeron://` links on macOS, Linux, and Windows. You can also pass one on the command line: `zeron "zeron://open/chat/..."`.

A link only opens in the workspace it came from. In a local workspace that means the same computer; in a synced workspace, the same account and workspace on any of your devices. Otherwise Zeron shows "This conversation link belongs to another workspace". If the conversation no longer exists, it shows "The linked conversation was not found". The iPhone app does not open these links.
