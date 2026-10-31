The Sidecar, titled **Live state**, sits on the right of a chat's conversation. It sums up what the chat has around it in three sections: **Workspace**, **Beside this thread** and **Actions**.

![The Sidecar: Workspace, Beside this thread with three agents and a shell, and Actions](live-state-column.jpg)

## Show and hide it

The chevron at the top (**Fold the Sidecar**) folds it to a 40 px rail, and the rail's top button (**Expand the Sidecar**) brings it back. The Sidecar folds to the rail by itself when the conversation is narrower than about 640 pixels, and whenever the right pane is open.

The rail has a button per section: **Workspace**, **Beside this thread**, with a badge counting the agents that run, and **Actions**. Click one to open that section as a popover; every row in it does what it does in the full column.

The Sidecar is not shown on New chat or while a [split view](zeron://docs/split-view) is open.

## Workspace

The device the chat runs on (or **Local**) and its folder (or **No workspace**). In a git project, a row shows the branch (or **No branch**), and the next one the short commit and how far the branch is ahead of main, such as **2 ahead of main**. On main itself it shows the commit alone, or **up to date**. While an agent uses [computer use](zeron://docs/computer-use), a **Computer** row names the app it is in; click it to show or hide the preview. Each device open for this chat gets a row too, marked **You** or **Agent** for who opened it. See [Device hub](zeron://docs/device-hub).

## Beside this thread

Everything this chat started that runs beside it, under a header that counts the agents running, such as **3 agents running**:

- its agents, each with how long it has run, and the finished ones folded under **Previous agents (N)**;
- its side chats, forks and the threads an agent started, each with its status or **Expired**;
- background shells, with **Stop** on hover;
- the Task row, **Task:** and its title with the step it is on, when this chat leads a [Task](zeron://docs/mission-control);
- **Stop all**, while anything here can be stopped.

Click a row to open it as a tab in the right pane; the row you have open reads **Open**. Past eight children the rest fold under a **N more** row. With nothing beside the chat the section reads **Nothing beside this thread**. See [Delegation](zeron://docs/delegation) and [Side chat](zeron://docs/side-chat).

## Actions

The project's saved Actions. Click a row to run it: it opens as a tab in the dock under the chat, named after the action, and the row reads **Running**, then **Done** or **Failed** when the command ends. The shell stays open in the tab. Hover a row for **Edit action**, or click **Add action**. See [Projects](zeron://docs/projects).
