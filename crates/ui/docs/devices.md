Zeron can run your chats on more than one machine. Each project belongs to one device and its chats run there, while you follow them from the Zeron window in front of you.

## Connect your devices

Out of the box Zeron uses a local workspace on this device only. To see your other machines, click your account button in the lower part of the left rail and choose **Enable sync**. Your browser opens to sign in, and an **Enable sync** dialog waits for you; **Open browser again** reopens the page, and **Cancel** drops the half-finished sign-in and keeps you in the local workspace.

If your account has no workspace yet, Zeron asks you to **Create your workspace**: type a name and click **Create**. If you already belong to workspaces, pick one under **Or continue in a workspace you belong to**.

Once you are signed in, a **Sync is ready** dialog offers the switch:

- **Bring my work** moves this device's sessions and projects into the synced workspace.
- **Start fresh** opens the synced workspace without them. They stay on this device.
- **Switch now** appears instead of those two when this device has nothing to bring.
- **Later** closes the dialog. Choose **Finish sync setup** in the account menu to come back to it.

Zeron switches in place, without a restart. With **Bring my work**, **Bringing your work over** shows each session as it is copied, then **You're all set** says how many moved; click **Continue**. If the copy stops, **Import didn't finish** keeps what was already copied: **Retry import** copies only what is missing, and **Later** leaves it for **Finish sync setup** in the account menu.

If the switch cannot finish in place, **Sync needs a restart** asks you to quit and reopen Zeron (**Quit Zeron**). When the app is connected to a background service, the button reads **Stop daemon and quit**. Sessions that stayed local remain on this device and are not uploaded.

Sign in the same way on each machine you want to connect. On a server without a window, use `zeron login`; see [Command line and servers](zeron://docs/command-line). On your phone, see [Zeron on iPhone](zeron://docs/mobile).

### Signing out

With sync on, the account menu shows **Sign out**. **Sign out?** explains that Zeron removes your credentials, closes the synced workspace, and continues in local mode. Click **Sign out** to confirm.

### Connection status

When Zeron cannot reach the sync server for a while, or changes are waiting to upload, a caption appears at the bottom of the sidebar:

| Caption | Meaning |
| --- | --- |
| **Reconnecting…** | Zeron is trying to reach the server again. |
| **Offline — sends are saved** | This device is offline. What you send goes out when it reconnects. |
| **Sync queued — changes are saved** | Changes in the open chat are waiting to upload. |
| **Syncing…** | The open chat is catching up. |
| **Offline — changes are saved** | The open chat's connection is down; its changes are kept. |
| **Changes could not be saved** | Zeron could not write the open chat's changes to disk. |

To dig deeper, `zeron sync` in a terminal shows the live sync state.

## Settings > Devices

[Settings > Devices](zeron://settings/devices) lists the machines in your workspace:

- **This device** is the machine you are using.
- **Other devices** shows the rest, with their platform, Zeron version, and **Online** or when they were last seen. With no other devices yet, it says "Sign in on another device to see it here." A local workspace has no **Other devices** list.

![Settings > Devices](devices-settings.jpg)

Each row has **Copy ID** (it reads **Copied** for a moment) and **Rename**. A new name applies across the workspace.

A new device takes its name from the computer's name or hostname. To pick the name a server registers with, set `ZERON_DEVICE_NAME` before it first starts; a name you set with **Rename** is kept after that. Phones show up with the platform **iOS**.

## Run a chat on another device

- **New project**: the add-project picker first asks for a device (**Search devices…**), then a location and a folder on it. A dot shows whether each device is online. The project stays tied to that device.
- **New chat**: on the new chat canvas, the device chip next to the project chip shows where the agent runs (**This device** by default). Click it to pick another machine. The chip turns to a warning color while that device is offline.

In the sidebar, a server icon marks chats that run on another device (in compact rows, and in full rows when project labels are hidden).

## What stays on this device

Some features work against the machine in front of you:

- Device tabs (simulators and emulators) need the chat to run on this computer.
- A browser tab in a remote chat opens localhost on this device. To reach a dev server on the other machine, open the preview Zeron detects for it from a new tab.
- Voice mode keeps audio here. **Codex voice device** in [Settings > Voice](zeron://settings/voice) chooses which device runs Codex for the call.

> A device shows as **Online** when it has checked in within roughly the last minute; a running Zeron checks in every 15 seconds.
