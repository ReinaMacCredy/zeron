Providers are the agents Zeron can run: Claude Code, Codex, Cursor, Devin, Grok, Hermes, Pi, OpenCode, and Antigravity, plus any agent you add from the ACP Registry. Open [Settings > Providers](zeron://settings/providers) to choose which ones the composer offers, sign in, and add extra accounts or setups.

## Turn agents on and off

Each row shows the agent, its version, and a switch that decides whether the composer offers it. Turning an agent on needs its CLI installed; a missing CLI dims the row, and **Install** sets it up when Zeron can. You cannot turn off the last enabled agent. **Update** appears when a new version is out, and **Check now** in the header looks right away.

Click an enabled row to open its details:

- **Completion**: **Use $ for skills** and **Separate / commands**.
- **Updates**: **Update policy** is **Notify**, **Auto when idle**, or **Off**.
- **Accounts**: every login you have for that agent.

## Accounts and sign-in

Click **Add account** and finish signing in on the page that opens in your browser; the new login is saved next to your current one. Each account shows how much of its plan is left, and the one in use is marked **In use**. Click another account to switch to it, or use its menu to **Remove account**.

**Auto-resume after usage limits** continues a thread that stopped on a usage limit once the limit resets.

## Add provider

**Add provider** sets up another instance of an agent, for example a work login next to a personal one. Each instance has its own label and settings, and its own tab in the model picker.

1. **Provider**: pick a tile. For Claude, Codex, and Pi, **Sign in with Claude** or **Continue with ChatGPT** creates a numbered instance with a home folder of its own and starts its sign-in in one step. **Configure manually**, or **Next** for the other agents, walks through the rest.
2. **Identity**: a **Label** shown in the provider list and the model picker, an **Instance ID** (letters, digits, `-`, or `_`), and an optional **Accent color** for the picker.
3. **Config**: a **Binary path**, **Launch arguments**, and **Environment** variables set for this instance only. Claude, Codex, and Pi also take a separate home folder, so the instance keeps its own login and settings. Cursor instances take only environment variables.

## ACP agents

Below the tiles, **Or choose from ACP Registry** searches the registry of agents that speak the Agent Client Protocol. Each result shows its version and how it installs, and **Installed** when it is already set up. Click **Add** to download or prepare it, give it a label, then choose **Continue to sign-in**. The **Sign in** step lists the sign-in methods the agent offers, or links to its docs with **Open docs** when it has none in the app. **Skip for now** finishes without signing in.

- **Enter manually** takes an agent ID from the official ACP Registry, for example `devin`.
- **Local ACP command** runs an ACP agent that is already installed: set its **Executable** (an absolute path or a command on PATH), **Arguments**, and **Environment**, then **Add instance**.

## Instances on the Providers page

Instances of a built-in agent show under that agent's row; ACP agents get rows of their own. A row shows the instance's account, **Uses the default Claude login** (or Codex, or Pi) when it shares yours, or **Sign in** for an ACP agent you have not signed in to yet. The pen (**Edit**) reopens the setup, and the trash asks **Delete?** first. Deleting an instance whose home folder Zeron made also removes that folder and its saved login.

Moving a chat that has history to another instance works like switching to another agent: Zeron asks **Switch provider?**, closes the old session, and hands the conversation over as text. See [Models and effort](zeron://docs/models-and-effort).

## Plan usage in the rail

The left rail shows up to two usage rings, one per signed-in provider. When an account reports both limits, the outer ring is the weekly limit and the inner one the session limit. A ring turns to a warning color at 80% used and to danger at 95%. Hover it to see what is left and when it resets; click it to switch accounts or **Open Usage**.

> The [Usage](zeron://usage) page shows every account's limits and the token spend on this device.
