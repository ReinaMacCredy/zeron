Home widgets are small cards on the new chat canvas, below the **Needs you**, **Running** and **Recent** cards under the composer. They show your pull requests, your git activity, and how your machine is doing before you start a chat.

## The widget grid

The grid appears only on the new chat canvas, below the composer and its cards. It hides once you are in a chat, and when every widget is turned off. A summary pill at the top of the grid reads something like "1 to review · 3 commits" (or just **Home** when there is nothing to report). Click it to collapse or expand the grid; Zeron remembers which. A new install starts with the grid collapsed.

![Home widgets under the composer](home-widgets-grid.jpg)

Widgets refresh only while the grid is visible.

## The widgets

| Widget | What it shows |
| --- | --- |
| **Pull requests** | Pull requests waiting on your review, and your own open ones with their status and CI. |
| **Git activity** | Your commits over the last 14 days, across all projects or one at a time. |
| **This Mac** | CPU, memory, and disk use of this machine. |
| **Running services** | Ports listening on this machine. Click a row to open `http://localhost:<port>`. |
| **CI failures** | Failing checks on your open pull requests. |

Running chats and their agents are in the **Running** card and the Cmd+I tray. Tasks with a plan are on the [Tasks](zeron://docs/tasks) page under **Plans**. Recent chats are in the **Recent** card.

The **Pull requests** and **Git activity** widgets have a project filter. **All projects** is one of the choices.

## Customize

Click **Customize** at the right of the grid header to switch widgets on or off, and to choose what **Running services** lists:

- **By Zeron** (the default) lists servers running inside your Zeron projects and worktrees.
- **All ports** adds every other listening process on this Mac.

Changes save right away.

All five widgets are on by default. Their order is fixed. You can hide a widget, but you cannot move it.

> The canvas background image is not set here. Find it under **New thread background** in [Settings > Appearance](zeron://settings/appearance).
