Home widgets are small cards under the composer on the new chat canvas. They show what is running, what needs review, and how your machine is doing before you start a chat.

## The widget grid

The grid appears only on the new chat canvas, below the composer. It hides once you are in a chat, and when every widget is turned off. A summary pill at the top of the grid reads something like "2 running · 1 to review · 3 commits" (or just **Home** when there is nothing to report). Click it to collapse or expand the grid; Zeron remembers which.

![Home widgets under the composer](home-widgets-grid.jpg)

Widgets refresh only while the grid is visible.

## The widgets

| Widget | What it shows |
| --- | --- |
| **Working now** | Chats with a running turn, and chats whose turn is done while their subagents, delegated chats or shells still run (**Turn done · background still running**). Under each chat, a line per subagent, delegated chat and shell, with its elapsed time; the header totals them, such as "2 subagents · 1 shell". Click a chat to open it, a subagent line to open it with its agents showing, or a shell line to open it with its shells. |
| **Missions** | Missions you have not marked done, with a progress ring and what each is doing: **Planning**, **Step 2/5 · Working**, **Needs you · plan review**, **Plan approved · can take back** while the [Supervisor](zeron://docs/supervisor)'s approval waits out its 30 seconds, **Needs you · step 3** for the step that stopped, or just **Needs you** when the lead waits on you or has stopped and no step is stuck. Tasks that need you come first. Up to four show; **+2 more · Open Tasks** opens Tasks. With none, it reads **No missions running**. Click a row to open the mission's page; see [Mission Control](zeron://docs/mission-control). |
| **Pull requests** | Pull requests waiting on your review, and your own open ones with their status and CI. |
| **Git activity** | Your commits over the last 14 days, across all projects or one at a time. |
| **This Mac** | CPU, memory, and disk use of this machine. |
| **Running services** | Ports listening on this machine. Click a row to open `http://localhost:<port>`. |
| **Recent areas** | Projects and worktrees you worked in most recently. Click to open the latest chat there. |
| **CI failures** | Failing checks on your open pull requests. |

The **Pull requests** and **Git activity** widgets have a project filter. **All projects** is one of the choices.

## Customize

Click **Customize** at the right of the grid header to switch widgets on or off, and to choose what **Running services** lists:

- **By Zeron** (the default) lists servers running inside your Zeron projects and worktrees.
- **All ports** adds every other listening process on this Mac.

Changes save right away.

All eight widgets are on by default. Their order is fixed. You can hide a widget, but you cannot move it.

> The canvas background image is not set here. Find it under **New thread background** in [Settings > Appearance](zeron://settings/appearance).
