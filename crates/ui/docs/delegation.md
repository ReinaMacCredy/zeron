An agent can hand part of its work to a child agent on any agent and model you have installed, not only its own. A Claude Code chat can hand a task to Codex, Grok, OpenCode or an ACP agent, and a Codex chat can hand one to Claude Code. Zeron shows each child in the transcript, in the **Live state** column next to the chat, and in the agents pill above the composer, so you can follow or stop it.

## Ask for a delegated task

Ask in your own words, and name the agent and model you want when it matters:

- "Delegate the security audit to Codex."
- "Have Grok review this diff while you keep going."
- "Run the migration with a Claude Code child on its strongest model, and wait for it."

Without a name, the child runs where [Settings > Routing](zeron://settings/routing) sends its role, or, when that role is **Off**, on this chat's agent, model and reasoning level. The agent gives each child a role: **worker** to write and change code, **validator** to review and test, **research** to read and report, **design** to shape an approach, or **general**. See [Routing](zeron://docs/routing).

The agent looks up what it can use with Zeron's `orchestrator_capabilities` tool: every agent installed and turned on in [Settings > Providers](zeron://settings/providers), with its current models from the same list as the model picker, your agent profiles, and the limits below. An agent that is not installed or is turned off cannot run a child. When the agent picks another agent without naming a model, the child gets that agent's default model; a model that agent does not offer is refused.

For work on its own agent and model, an agent such as Claude Code may still use its built-in subagents. It uses a Zeron child for another agent, for a model its own subagents cannot run, or when you ask for one.

The child starts with only the task the agent writes for it, not this chat's history, and works in this chat's project and folder. It runs at this chat's permission mode or a lower one, never higher, and keeps off every tool this chat has off.

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

Click the chip to open the child chat as a tab in the right pane. It has its own composer, so you can talk to the child directly. A line above that composer reads **Subagent of** the parent chat, with **Open parent** to jump back. Delegated chats do not appear in the sidebar.

Built-in subagents of agents such as Claude Code get a chip too; it opens a read-only transcript tab in the right pane.

## When a child asks

A child can stop on a question or on a tool approval. Zeron then wakes the parent's agent with the question, its options and its request id, even when the parent is waiting for that child.

The parent's agent answers with Zeron's `respond_to_input` tool when the answer follows from the task or from an earlier decision. It can grant a tool approval only when the parent chat runs at **Full access**. Anything else, and anything only you can decide, it asks you or leaves to you. Computer use consent is always yours to give.

The question waits for you as well, so you can answer it yourself. If you answer first, the parent's agent finds nothing left to answer. See [When an agent asks](zeron://docs/agent-questions).

## When you write to a child

You can talk to a child yourself in its tab. When you send a message to a delegated task that is still running, its parent's agent gets a note that names the child and quotes what you wrote, so it knows the child's plan may have changed. The same goes for a step of a [Mission Control](zeron://docs/mission-control) mission: its lead gets the note. The note is only news: it does not change the child's result, and the parent's agent does not need to answer it. A thread another chat merely opened with `create_thread` sends no note.

## Check on or stop a task

With Zeron's `task_status` tool, the agent reads where a task stands: still working, waiting on children of its own, or done, with the child's final result, or the error when it failed. It can read a task another chat delegated too, without affecting that chat.

`task_cancel` stops a running child. A result the child already finished stays readable.

## The Live state column

The **Live state** column on the right of a chat lists **Subagents**, with a count of running ones. Each row shows the agent's icon, its task, and its model. A running row shows its elapsed time; a failed one reads **Failed**, and one that was stopped reads **Stopped**. Finished agents fold under **Previous agents**. Click any row to open that agent.

The chevron at the top collapses the column to a narrow rail, and the rail's chevron expands it again. When the right pane is open, the rail button opens the same content as a **Live state** popover.

## The agents pill

While child agents run, a pill above the composer shows how many are working. Click it to open the roster. Delegated tasks, and Claude Code's own subagents, have a stop button on their row, and **Stop all** stops them all. Click a row to see its log, then **Open in pane** to read it in the right pane.

## Limits

[Settings > General](zeron://settings/general) has two limits for agents that delegate:

| Setting | Default | Range |
| --- | --- | --- |
| **Delegation depth**: how many levels of delegated tasks an agent may nest | 2 | 1 to 5 |
| **Delegated tasks per chat**: how many delegated tasks one chat may run at once | 8 | 1 to 32 |

**Delegated tasks per chat** also caps how many steps of a [Mission Control task](zeron://docs/mission-control) its lead runs at once. Those steps run on ordinary chats, not as delegated tasks, so they show in the sidebar.

An agent that hits a limit is told so and cannot start the task.

To have an agent work with your other chats instead of its own children, see [Agents and other threads](zeron://docs/threads).

> To stop agents from delegating at all, turn off **delegate_task** in the **Delegation** group of [Settings > Tools](zeron://settings/tools).
