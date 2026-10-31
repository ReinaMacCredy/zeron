An agent can hand part of its work to a child agent on any agent and model you have installed, not only its own. A Claude Code chat can hand a task to Codex, Grok, OpenCode or an ACP agent, and a Codex chat can hand one to Claude Code. Zeron shows each child in the transcript and in the **Live state** column next to the chat, under **Beside this thread**, so you can follow or stop it.

## Ask for a delegated task

Ask in your own words, and name the agent and model you want when it matters:

- "Delegate the security audit to Codex."
- "Have Grok review this diff while you keep going."
- "Run the migration with a Claude Code child on its strongest model, and wait for it."

Without a name, the child runs where [Settings > Agents > Routing](zeron://settings/routing) sends its role, or, when that role is **Off**, on this chat's agent, model and reasoning level. The agent gives each child a role: **worker** to write and change code, **validator** to review and test, **research** to read and report, **design** to shape an approach, or **general**. See [Routing](zeron://docs/routing).

The agent looks up what it can use with Zeron's `orchestrator_capabilities` tool: every agent installed and turned on in [Settings > Providers](zeron://settings/providers), with its models from the same list as the model picker, your agent profiles, and the limits below. For each agent it lists the models in use and the first eight, says how many it left out, and searches the rest when the agent asks for one by name; any model in the list can be used. An agent that is not installed or is turned off cannot run a child. When the agent picks another agent without naming a model, the child gets that agent's default model; a model that agent does not offer is refused.

For work on its own agent and model, an agent such as Claude Code may still use its built-in subagents. It uses a Zeron child for another agent, for a model its own subagents cannot run, or when you ask for one.

The child starts with only the task the agent writes for it, not this chat's history, and works in this chat's project and folder. It runs at this chat's access mode or a lower one, never higher, and keeps off every tool this chat has off.

## Pick a profile

When you have [agent profiles](zeron://docs/agent-profiles), the agent reads each one's **When to use** note and can start the child on the profile that fits. The child then runs on that profile's agent, model, reasoning, model options and instructions, with the profile's tools turned off as well. The profile's mode applies only when it is no higher than this chat's. Without a profile, the child never gets this chat's instructions.

A clear **When to use** note, such as "Security reviews of Rust code", is what lets an agent pick the right profile without being told.

## In the background or waiting

By default a child runs in the background. The agent gets the task's id and carries on, or ends its turn, and Zeron wakes it when the child is done. If you stop the turn that started the child, the child keeps running, but Zeron does not wake the agent for it; the child's chip and `task_status` still show its result.

When the agent needs the result before it can go on, it can wait for the child instead: up to 10 minutes by default, 30 at most. If that time runs out, the child keeps running and the parent's agent is woken when the result is ready.

## Delegated task chips

When an agent delegates, a chip appears in the transcript with the child's agent, title, model, and status: **Running**, **Completed**, **Failed**, or **Stopped**. When the child is done, the chip shows how long it took, and the parent chat gets a row such as **Delegated task “Audit” finished**, or **failed** or **stopped**, with **Open subagent**. Children that finish within half a second of each other share one row, such as **2 delegated tasks finished: Audit, Lint**, without the **Open subagent** button. The parent's agent gets each child's final reply in that message, cut short after 4,000 characters.

![A delegated task's finished row with Open subagent](delegation-finished.jpg)

If the parent's agent is in the middle of a turn and its agent takes messages mid-turn, the message reaches it inside that turn. Otherwise it waits for the turn to end and then starts a new one. Archiving the parent chat drops the message.

Click the chip to open the child chat as a tab in the right pane, with an **Agent** badge; clicking the chip again, or the child's row in the **Live state** column, brings back the same tab. The chip of a child that is open in the pane has an accent border. The tab has its own composer, so you can talk to the child directly. A line above that composer reads **Subagent of** the parent chat, with **Open parent** to jump back. A delegated chat has no sidebar row while its parent chat has one; find it in the parent's **Live state** column.

Built-in subagents of agents such as Claude Code get a chip too; it opens a read-only transcript tab in the right pane.

## When a child asks

A child can stop on a question or on a tool approval. Zeron then wakes the parent's agent with the question, its options and its request id, even when the parent is waiting for that child.

The parent's agent answers with Zeron's `respond_to_input` tool when the answer follows from the task or from an earlier decision. It can grant a tool approval only when the parent chat runs at **Full access**. Anything else, and anything only you can decide, it asks you or leaves to you. Computer use consent is always yours to give.

The question waits for you as well, so you can answer it yourself. If you answer first, the parent's agent finds nothing left to answer. See [When an agent asks](zeron://docs/agent-questions).

## When you write to a child

You can talk to a child yourself in its tab. When you send a message to a delegated task that is still running, its parent's agent gets a note that names the child and quotes what you wrote, so it knows the child's plan may have changed. The same goes for a step of a Task with a plan (see [Agent trees and plans](zeron://docs/mission-control)): its lead gets the note. The note is only news: it does not change the child's result, and the parent's agent does not need to answer it. A thread another chat merely opened with `create_thread` sends no note.

## Check on or stop a task

With Zeron's `task_status` tool, the agent reads where a task stands: still working, waiting on children of its own, or done, with the child's final result, or the error when it failed. It can read a task another chat delegated too, without affecting that chat.

`task_cancel` stops a running child. A result the child already finished stays readable.

## The Live state column

The **Live state** column on the right of a chat has three sections: **Workspace**, **Beside this thread** and **Actions**. **Beside this thread** lists the chat's children, with a count of running agents in its header, such as **Beside this thread · 2 agents running**. Running agents come first. Each row shows the agent's icon and its task. A running row shows its elapsed time; a failed one reads **Failed**, and one that was stopped reads **Stopped**. Finished agents fold under **Previous agents**. Side chats, forks and threads an agent started follow, then background shells. Click any row to open that child as a tab in the right pane; the row of the open one reads **Open**.

The chevron at the top folds the column to a narrow rail, and the rail's top button expands it again. While a tab is open in the right pane, the column shows as the rail: its **Workspace**, **Beside this thread** and **Actions** buttons each open that section as a popover, and the **Beside this thread** button counts the running agents.

## Stop agents

While child agents or background shells run, **Stop all** at the foot of **Beside this thread** stops them all: delegated tasks, Claude Code's own subagents and shells. Hover a background shell's row for its own **Stop**.

## Limits

The **Delegation** section at the bottom of [Settings > Agents > Routing](zeron://settings/routing) has two limits for agents that delegate:

| Setting | Default | Range |
| --- | --- | --- |
| **Delegation depth**: how many levels of delegated tasks an agent may nest | 2 | 1 to 5 |
| **Delegated tasks per chat**: how many delegated tasks one chat may run at once | 8 | 1 to 32 |

**Delegated tasks per chat** also caps how many steps of a Task's plan (see [Agent trees and plans](zeron://docs/mission-control)) its lead runs at once. Those steps run on ordinary chats, not as delegated tasks, so they show in the sidebar.

An agent that hits a limit is told so and cannot start the task.

To have an agent work with your other chats instead of its own children, see [Agents and other threads](zeron://docs/threads).

> To stop agents from delegating at all, turn off **delegate_task** in the **Delegation** group of [Settings > Context & tools > Tools](zeron://settings/tools).
