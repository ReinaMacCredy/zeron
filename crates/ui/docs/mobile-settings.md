The iPhone's **Settings** has two kinds of settings: the phone's own, and each Mac's. A Mac's settings are the same ones you change in **Settings** on that Mac; a change from the phone applies there.

![A Mac's settings page, and What's New on the phone](ios-settings.jpg)

## A Mac's settings

Open **Settings** > **Devices** and tap a Mac. Its page lists:

- **Accounts and usage**: each provider's accounts, with the active one's meters (such as the 5-hour and weekly windows and when they reset) and its plan. Tap an account to switch to it; swipe it for **Remove account**. **Add account** under a provider opens its sign-in.
- **Usage**: "Plan limits and token spend". **Plan limits** from your accounts, then **Token usage** over the last 24 hours, 7, 30 or 90 days, with **Totals**, **By provider** and **Top models**, estimated from public API prices. See [Usage](zeron://docs/usage).
- **Providers**, **Agent profiles**, **Routing** and **Tools**, below.
- **Limits**: **Auto-resume after usage limits**, so chats on that Mac continue on their own when a limit resets.
- **Stop computer use** stops every chat on that Mac from using apps (see [Devices and computer use on iPhone](zeron://docs/mobile-devices)).

## Providers and instances

**Providers** lists each agent with a switch and its version. When a newer version is out, a row such as "Update Codex to v1.2" installs it; **Check now** looks again. A provider whose CLI is missing says to install it on the Mac.

**Instances** run a provider with their own name, sign-in, binary or environment. Tap one to edit its **Name**, **Accent** (its chip's color in the model picker), binary path and arguments, and **Environment** variables as `KEY=value`; **Sign in** gives it its own login, and **Delete instance** removes it.

![Add provider, and Providers with an instance](ios-providers.jpg)

**Add provider** adds one:

- One of the built-in agents, as a numbered instance such as "Codex 2".
- **Search the ACP Registry** for an agent that speaks ACP. "Adding an agent installs it on the Mac." An agent the registry has not verified asks first, with **Add anyway**.
- **Local ACP command** runs a command of your own that speaks ACP.

See [Providers](zeron://docs/providers).

**Sign in** opens the provider's page in the phone's browser after **Start sign-in**. Come back to Zeron to **Paste the authorization code**, or tap **Check again** once you have signed in.

## Agent profiles

**Agent profiles** lists your [profiles](zeron://docs/agent-profiles) with their agent, model, mode and effort. Tap one to edit its **Name**, **When to use**, **Instructions**, **Agent**, **Model**, **Mode** and **Effort**, and its **Tools**: **Zeron tools** by group, **Claude built-in** tools and **MCP servers**. Swipe a profile to **Move up** or **Move down**. **New profile** adds one; the **System prompt** row sets the Mac's own.

## Routing and tools

**Routing** sets, for each role, the harness, model and reasoning a delegated task runs on, or **Agents pick**. **When out of quota** sets what happens when the routed harness runs out: the fallback list, and what to do on a **Mid-run limit**. See [Routing](zeron://docs/routing).

**Tools** turns Zeron's tools on and off for every chat on that Mac: **Turn all on**, **Turn all off**, or one group or tool at a time. A call to a tool you turn off is refused right away. See [Agent tools](zeron://docs/agent-tools).

## The phone's own settings

- **Account** and **Devices**: who you are signed in as, and your Macs with **Online** or **Offline**.
- **Appearance**: **System**, **Light** or **Dark**; a **Wallpaper**; **Chat background** to show the wallpaper behind open conversations; and **Render Math**, **Math Font** and **Math Size**.
- **Sessions**: **Archived Sessions**.
- **Composer**: **Queue** or **Steer** for messages you send while the agent runs, and **In a chat, Mission Control** (see [The composer on iPhone](zeron://docs/mobile-composer)).
- **Voice** and **Notifications** (see [Zeron on iPhone](zeron://docs/mobile)).
- **Sign Out**.

## What's New and Docs

After an update, a card says what is new, with **See what’s new →** and **Dismiss**. **Settings** > **Help** > **What's New** opens the release any time, with **View changelog →**. See [Updates](zeron://docs/updates).

**Settings** > **Help** > **Docs** has these articles, by section. Links to other articles open on the phone; tap a screenshot to see it full size. Links to settings point at the Mac app, and say "Open this in Zeron on the Mac".
