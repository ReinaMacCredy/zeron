An agent can hand part of its work to a child agent on any installed provider and model. Zeron shows each child in the transcript, in the **Live state** column next to the chat, and in the agents pill above the composer, so you can follow or stop it.

## Delegated task chips

When an agent delegates, a chip appears in the transcript with the child's agent, title, model, and status: **Running**, **Completed**, **Failed**, or **Stopped**. When the child is done, the chip shows how long it took, and the parent chat gets a row saying the delegated task finished, with **Open subagent**.

Click the chip to open the child chat as a tab in the right pane. It has its own composer, so you can talk to the child directly. A line above that composer reads **Subagent of** the parent chat, with **Open parent** to jump back. Delegated chats do not appear in the sidebar.

Built-in subagents of agents such as Claude Code get a chip too; it opens a read-only transcript tab in the right pane.

## The Live state column

The **Live state** column on the right of a chat lists **Subagents**, with a count of running ones. Each row shows the agent's icon, its task, and its model. A running row shows its elapsed time; a failed one reads **Failed**. Finished agents fold under **Previous agents**. Click any row to open that agent.

The chevron at the top collapses the column to a narrow rail, and the rail's chevron expands it again. When the right pane is open, the rail button opens the same content as a **Live state** popover.

## The agents pill

While child agents run, a pill above the composer shows how many are working. Click it to open the roster. Delegated tasks, and Claude Code's own subagents, have a stop button on their row, and **Stop all** stops them all. Click a row to see its log, then **Open in pane** to read it in the right pane.

## Limits

[Settings > General](zeron://settings/general) has two limits for agents that delegate:

| Setting | Default | Range |
| --- | --- | --- |
| **Delegation depth**: how many levels of delegated tasks an agent may nest | 2 | 1 to 5 |
| **Delegated tasks per chat**: how many delegated tasks one chat may run at once | 8 | 1 to 32 |

**Delegated tasks per chat** also caps how many steps of an [orchestrated task](zeron://docs/tasks) its lead runs at once. Those steps run on ordinary chats, not as delegated tasks, so they show in the sidebar.

An agent that hits a limit is told so and cannot start the task.

> To stop agents from delegating at all, turn off **delegate_task** in the **Delegation** group of [Settings > Tools](zeron://settings/tools).
