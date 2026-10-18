Zeron can run your chats on more than one machine. Each project belongs to one device and its chats run there, while you follow them from the Zeron window in front of you.

## Connect your devices

Out of the box Zeron uses a local workspace on this device only. To see your other machines, click your account button in the lower part of the left rail and choose **Enable sync**, then finish signing in in your browser. If you already have local sessions, Zeron offers **Bring my work** to move them into the synced workspace, or **Start fresh**. Sign in the same way on each machine you want to connect.

## Settings > Devices

[Settings > Devices](zeron://settings/devices) lists the machines in your workspace:

- **This device** is the machine you are using.
- **Other devices** shows the rest, with their platform, Zeron version, and **Online** or when they were last seen. With no other devices yet, it says "Sign in on another device to see it here." A local workspace has no **Other devices** list.

Each row has **Copy ID** and **Rename**. A new name applies across the workspace.

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
