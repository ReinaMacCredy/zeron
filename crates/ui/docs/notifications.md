Zeron tells you when a chat needs you: a short sound, a desktop banner, or both. Open [Settings > Notifications](zeron://settings/notifications) to choose which.

## What triggers a notification

A notification fires when a chat you started changes state, or when Zeron loses its connection:

| Event | Banner |
| --- | --- |
| A run finishes | Chat title, "Run finished" |
| The agent asks you something | Chat title, "Waiting on your input" |
| A run fails | Chat title, "Run failed" |
| The connection drops | "Connection unavailable" |
| The Supervisor needs your decision while you are away | "Supervisor needs you", the decision's title |

On macOS, click a chat's banner to open that chat, and the Supervisor's banner to open the [Supervisor](zeron://docs/supervisor) page. Windows shows no banners yet; the sounds still play there.

Only chats you opened yourself notify you. Side chats, delegated child threads, and the voice session stay quiet, and their status still shows in the sidebar.

## Desktop

- **Desktop notifications** turns banners on or off.
- **Agent updates** shows a banner when monitored agent CLIs have updates. Clicking it opens [Settings > Providers](zeron://settings/providers).
- **Only when in the background** skips the banner while any Zeron window is focused. The sound still plays, so you hear it while you look at another chat. The Supervisor's banner ignores this switch: it shows only after you have been away from Zeron for 3 minutes.

The two rows under **Desktop notifications** are dimmed and inactive while banners are off.

## Sounds

**Session sounds** is the master switch for chimes. Under it, each event has its own switch:

- **Task completed**
- **Input required**
- **Errors and disconnections**

The per-event rows are dimmed and inactive while **Session sounds** is off. Errors and disconnections share one chime, and failures that arrive at the same moment play it once.

> All of these are on by default, and **Only when in the background** is on too, so you get banners only when Zeron is behind another app.

## Other sounds

Two features have their own sound settings: the shutter cue for Appshots (**Capture sound** in [Settings > Appshots](zeron://settings/appshots)) and the click sound when an agent uses your Mac (**Click sounds** in [Settings > Computer](zeron://settings/computer)).
