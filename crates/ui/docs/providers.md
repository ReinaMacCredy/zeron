Providers are the agents Zeron can run: Claude Code, Codex, Cursor, Devin, Grok, Hermes, Pi, OpenCode, and Antigravity. Open [Settings > Providers](zeron://settings/providers) to choose which ones the composer offers, sign in, and add extra accounts or setups.

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

**Add provider** sets up a second instance of an agent, for example a work login next to a personal one.

1. Pick a provider tile. **Sign in with Claude** or **Continue with ChatGPT** creates a numbered instance with its own login in one step. **Configure manually** walks through the rest.
2. Give it a **Label**, an **Instance ID**, and an optional **Accent color** for the picker.
3. Set a **Binary path**, a separate home folder, **Launch arguments**, or **Environment** variables.

Below the tiles, **Or choose from ACP Registry** lets you search for agents that speak the Agent Client Protocol and **Add** them. **Local ACP command** runs an ACP agent from a command on your Mac.

Instances appear under their provider, with a pen to edit and a trash button that asks **Delete?** first.

## Plan usage in the rail

The left rail shows up to two usage rings, one per signed-in provider. When an account reports both limits, the outer ring is the weekly limit and the inner one the session limit. A ring turns to a warning color at 80% used and to danger at 95%. Hover it to see what is left and when it resets; click it to switch accounts or **Open Usage**.

> The [Usage](zeron://usage) page shows every account's limits and the token spend on this device.
