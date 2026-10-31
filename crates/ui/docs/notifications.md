Zeron tells you when a chat needs you: a short sound, a desktop banner, or both. Open [Settings > Notifications](zeron://settings/notifications) to choose which.

![Settings > Notifications](notifications-settings.jpg)

## What triggers a notification

A notification fires when a chat you started changes state, when Zeron loses its connection, or when something new is available:

| Event | Banner |
| --- | --- |
| A run finishes | Chat title, "Run finished" |
| The agent asks you something | Chat title, "Waiting on your input" |
| A run fails | Chat title, "Run failed" |
| The connection drops | "Connection unavailable", with "Your device is offline" or "Zeron is trying to reconnect" |
| The Supervisor needs your decision while you are away | "Supervisor needs you", the decision's title |
| Agent CLI updates are ready | "1 agent update available", or "3 agent updates available" for several |
| An agent offers a model it did not list before | "New model available" or "New models available", with the agent and model names |

On macOS, click a chat's banner to open that chat. When the chat still needs you, the banner opens the Cmd+I tray with that chat's row open instead, so you can answer it where you are. The Supervisor's banner opens the [Supervisor](zeron://docs/supervisor) page. On Linux, banners go through `notify-send` and clicking them does not open anything in Zeron. Windows shows no banners yet; the sounds still play there.

Only chats you opened yourself notify you. Side chats, delegated child threads, and the voice session stay quiet. A side chat's or child thread's status shows beside its parent chat, in the **Live state** column under **Beside this thread**.

## Desktop

- **Desktop notifications** turns banners on or off.
- **Agent updates** shows a banner when monitored agent CLIs have updates. Clicking it opens [Settings > Providers](zeron://settings/providers).
- **Only when in the background** skips the banner while any Zeron window is focused. The sound still plays, so you hear it while you look at another chat. The Supervisor's banner ignores this switch: it shows only after you have been away from Zeron for 3 minutes. New model banners ignore it too, and show whenever **Desktop notifications** is on.

The two rows under **Desktop notifications** are dimmed and inactive while banners are off.

## Sounds

**Session sounds** is the master switch for chimes. Under it, each event has its own switch:

- **Task completed**
- **Input required**
- **Errors and disconnections**

The per-event rows are dimmed and inactive while **Session sounds** is off. Errors and disconnections share one chime, and failures that arrive at the same moment play it once.

> All of these are on by default, and **Only when in the background** is on too, so you get banners only when Zeron is behind another app.

## Other sounds

Two features have their own sound settings: the shutter cue for Appshots (**Capture sound** in [Settings > This Mac > Appshots](zeron://settings/appshots)) and the click sound when an agent uses your Mac (**Click sounds** in [Settings > This Mac > Computer use](zeron://settings/computer)).

A [voice call](zeron://docs/voice) plays a short chime when it starts and when it ends. **Session sounds** does not turn these off.

To silence Zeron on a machine regardless of these settings, start it with `ZERON_DISABLE_SOUND` (no sounds) or `ZERON_DISABLE_NOTIFICATIONS` (no banners) set. See [Command line and servers](zeron://docs/command-line).

On your iPhone, notifications are set in the phone app. See [Zeron on iPhone](zeron://docs/mobile).
