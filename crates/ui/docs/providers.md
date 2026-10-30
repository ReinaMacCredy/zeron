Providers are the agents Zeron can run: Claude Code, Codex, Cursor, Devin, Grok, Hermes, fx, Pi, OpenCode, Antigravity, and Muse Code, plus any agent you add from the ACP Registry. Open [Settings > Providers](zeron://settings/providers) to choose which ones the composer offers, sign in, and add extra accounts or setups.

## Turn agents on and off

Each row shows the agent, its version, and a switch that decides whether the composer offers it. Turning an agent on needs its CLI installed; a missing CLI dims the row and its hint says what to install, for example **Install the claude CLI to enable**. **Install** sets it up when Zeron can; the row reads **Installing** and the agent's name while it runs, and **Cancel** stops it. You cannot turn off the last enabled agent while it is installed (an agent whose CLI is missing can always be turned off). Turning off the agent the [Supervisor](zeron://docs/supervisor) runs on stops its wakes until you turn it back on.

**Update** appears when a new version is out, and **Check now** in the header looks right away. While an update waits for the agent to be idle, prepares, or downloads, the version line says so (**Waiting for agent to be idle**, **Preparing update…**, **Downloading…**) and **Cancel** replaces **Update**. A failed update reads **Update failed** and offers **Retry update**. A new version known not to work with Zeron is held back, and the version line says so, for example **v1.4.13 held back: not supported yet**. When the version you have is the one that does not work, the line turns red and says so, with the version to use when there is one.

### Manage another device

The device menu at the top of the page, next to **Add provider**, chooses which device the page manages. Your own device is marked **You**. Pick another device to see and change its agents, updates, instances, and accounts; installs and sign-ins then run on that device.

Click an enabled row to open its details:

- **Completion**: **Use $ for skills** and **Separate / commands**.
- **Updates**: **Update policy** is **Notify**, **Auto when idle**, or **Off**.
- **Accounts**: every login you have for that agent.

fx has no accounts in Zeron: once it is installed, its row says **Configure your model provider with fx login or fx setup.** Run one of those in a terminal to pick the model provider fx uses.

## Accounts and sign-in

The **Accounts** section of an expanded row lists every login you have for that agent. The last row adds one: with no account yet it reads **Connect a … account** (for example **Connect a Claude Code account**), and after that **Add account**. OpenCode, Pi, and Hermes keep a login per model provider, so they get one button per login instead: **Connect ChatGPT** (then **Add ChatGPT account**) for all three, **Connect GitHub Copilot** for OpenCode, and **Connect Nous Portal** for Hermes. Antigravity keeps one login per device, so its button goes away once it is connected.

Finish signing in on the page that opens in your browser; **Reopen the sign-in page** opens it again. For most agents the new login is saved next to your current one, and nothing changes until you switch. Cursor instead mints a Zeron-named API key you can revoke from Cursor's dashboard, Antigravity just confirms its one login, and Hermes adds the login to its own credential pool. Grok, GitHub Copilot, and Hermes show a code to approve or enter in the browser.

Each account shows how much of its plan it has used, as a percent, in up to two windows, such as the session and the week. A bar turns to a warning color at 80% used and to danger at 95%; hover the meters to see when each window resets. The refresh button beside **Accounts** checks usage again. Antigravity reports no usage.

The account in use is marked **In use**. Click another account to switch to it, or use its **…** menu for **Switch to this account** and **Remove account**.

> Hermes manages its own credential pool and rotates through it. Accounts added here go through `hermes auth add`; remove one with `hermes auth remove`. Zeron lists Hermes accounts but does not switch or remove them.

**Auto-resume after usage limits** continues a thread that stopped on a usage limit once the limit resets.

## Add provider

**Add provider** sets up another instance of an agent, for example a work login next to a personal one. Each instance has its own label and settings, and its own tab in the model picker.

![The Add provider dialog](providers-add.jpg)

1. **Provider**: pick a tile. For Claude, Codex, and Pi, **Sign in with Claude** or **Continue with ChatGPT** creates a numbered instance with a home folder of its own and starts its sign-in in one step. **Configure manually**, or **Next** for the other agents, walks through the rest.
2. **Identity**: a **Label** shown in the provider list and the model picker, an **Instance ID** (up to 64 letters, digits, `-`, or `_`, starting with a letter; the id of a built-in agent such as `codex` reads as taken), and an optional **Accent color** for the picker.
3. **Config**: a **Binary path**, **Launch arguments**, and **Environment** variables set for this instance only. Claude, Codex, and Pi also take a separate home folder, so the instance keeps its own login and settings. Cursor instances take only environment variables.

## ACP agents

Below the tiles, **Or choose from ACP Registry** searches the registry of agents that speak the Agent Client Protocol. Each result shows its version and how it installs, and **Installed** when it is already set up. Click **Add** to download or prepare it, give it a label, then choose **Continue to sign-in**. The **Sign in** step lists the sign-in methods the agent offers, or links to its docs with **Open docs** when it has none in the app. **Skip for now** finishes without signing in.

- **Enter manually** takes an agent ID from the official ACP Registry, for example `devin`.
- An agent marked **unverified** downloads a program the registry publishes no checksum for, so Zeron cannot check it. Its **Add** turns into **Add anyway** on the first click, and under **Enter manually** **Next** does the same; click again to install it unchecked.
- **Local ACP command** runs an ACP agent that is already installed: set its **Executable** (an absolute path or a command on PATH), **Arguments**, and **Environment**, click **Next**, give it a **Label** on the **Identity** step, then **Add instance**.

## Instances on the Providers page

Instances of a built-in agent show under that agent's row; ACP agents get rows of their own. A Claude, Codex, or Pi instance with a home folder of its own shows its own account, or **Sign in** when it has none. Any other instance of a built-in agent shares your login and reads **Uses the default … login**, for example **Uses the default Claude login**. ACP rows always show **Sign in**, which reopens that agent's sign-in step; Zeron does not check whether it is already signed in. The pen (**Edit**) reopens the setup, and the trash asks **Delete?** first. Deleting an instance whose home folder Zeron made also removes that folder and its saved login. Chats that were using the instance cannot run until you pick another model or instance for them.

Moving a chat that has history to another instance works like switching to another agent: Zeron asks **Switch provider?**, closes the old session, and hands the conversation over as text (the most recent 48,000 characters). See [Models and effort](zeron://docs/models-and-effort).

## Plan usage in the rail

The left rail shows up to two usage rings, one per signed-in provider. When an account reports both limits, the outer ring is the weekly limit and the inner one the session limit. A ring turns to a warning color at 80% used and to danger at 95%. Hover it to see what is left and when it resets; click it to switch accounts or **Open Usage**.

> The [Usage](zeron://usage) page shows every account's limits and the token spend on this device.
