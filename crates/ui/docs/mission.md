A mission is a chat that hands work out. Each piece it hands out runs on a thread: an agent of its own, on its own chat, that reports back to the chat that started it. The chat that hands the work out is the lead. You keep talking to the lead, and the **Threads** tab beside it shows every thread and where it stands.

A chat becomes a lead the moment its agent delegates a task, writes a plan, or starts another chat. Ask for that in your own words, or turn on **Mission** in the composer.

![A lead chat after its plan was approved, and its Threads tab: four steps working and one planned](mission-threads.jpg)

## Ask for it

Ask in your own words:

- "Split this into steps, show me the plan, then run it."
- "Hand the test fix to a worker and keep going on the export."
- "Run these four changes in parallel and review each one before you report."

The agent hands work out with `delegate_task` and plans with `plan`; see [Delegation and subagents](zeron://docs/delegation) for how a child is started, which agent and model it runs on, and what it may do.

## Mission in the composer

Press **⇧⇥** (Shift+Tab) while you type in the composer, or click the **⇧⇥ Mission** key beside the microphone, to turn Mission on. In a narrow window the key folds to a target button. While Mission is on, a **Mission ×** label sits on the composer's top edge; click it, or press **⇧⇥** again, to turn Mission off. With **Mission** on, every message you type starts a mission turn: the agent writes a plan before it does anything else, and the steps run on threads. The composer takes the accent ring, and until the chat has a plan the send button reads **Start mission**.

A row of options opens above the composer's controls while Mission is on:

- **Agents**: **By role** sends each step where its role says (see [Routing](zeron://docs/routing)); **Same as this chat** runs every step on this chat's agent and model.
- **Approve**: **before it runs** holds the plan until you approve it; **not needed** lets the lead start the steps as soon as the plan is written.

Mission stays on for the chat until you turn it off, and each chat has its own. A `/` command you type is not a mission turn. Messages the lead gets from its threads, and the note it gets when you approve or edit the plan, never ask for a new plan.

Mission and the agent's own **Plan mode** (in the access menu) turn each other off: Plan mode is the agent's native read-only planning, Mission is Zeron's plan with steps on threads. The key, the label and the shortcut are not offered where a chat cannot lead: a side chat, a thread's own composer, and a chat whose `plan` or `delegate_task` tool is off.

## The Threads tab

**Threads** is the first tab of the right panel. Open it with the **Threads** button in the titlebar, or click **N Working** above the composer, which counts the threads that are running.

The tab's head reads the number of threads, and the list groups them by what they need, in this order:

| Group | Holds |
| --- | --- |
| **Ready for review** | Threads that finished a run and wait for someone to look at the result |
| **Waiting on you** | Threads that stopped on a question, and threads that failed |
| **Working** | Threads whose agent is running |
| **Planned** | Steps of the plan that have not started |
| **Done** | Threads marked done, and steps that were skipped or cancelled |

Each row shows the thread's state, its title, its agent's mark and model, and the last thing it said. A row in **Waiting on you** shows the question it asks. A step of the plan carries its number.

A chat the lead watches with `watch_threads` is listed too, as a **Peer** row marked **read-only**. The agent's built-in subagents are not threads: they stay on the composer's agents pill and its roster.

With no threads yet, the tab reads **No threads yet. Hand work out in this chat and each piece becomes a thread.**

## Open a thread

Click a row and the thread opens inside the tab: its transcript, and its own composer, **Message this thread…**, so you can talk to that agent directly. The lead is told what you wrote. A thread that waits on you shows **Waiting on you:** and its question above the transcript.

**Back to threads** returns to the list, and the lead's chat stays beside it the whole time. A thread's chip in the lead's transcript opens the same view.

Right-click a row, or use its **…** button, for:

- **Open thread**
- **Stop**, while it is working
- **Archive**
- **Mark done**, which moves it to **Done**, and **Reopen**, which brings it back
- **Skip**, for a step that has not run
- **Run again**, for a step that failed or was stopped

## A plan

When the work has several parts, the lead writes a plan: an overview and a list of steps, each with a title, a brief, and the role that decides where it runs (see [Routing](zeron://docs/routing)). The plan arrives in the lead's transcript as a card, **Plan · 5 steps**, with the overview and the numbered steps.

![A plan card waiting for approval, and the Plan bar above the Planned steps](mission-plan.jpg)

Nothing runs until you approve. While the plan waits:

- the card has **Edit plan** and **Approve and run**;
- the **Plan** bar at the top of the Threads tab reads **Waiting for your approval. Nothing has started.**, with **Edit** and **Approve**;
- the **Waiting** pill above the composer counts it, here as **Approve the plan** and in every other chat, where its card shows the steps with **Approve**.

After you approve, the card reads **Approved**, the lead starts the steps, and each step moves from **Planned** to **Working** as its thread starts. The **Plan** bar counts the steps that are done, such as **Plan 2/5**.

A step is done only when it is marked done, by the lead or by you; a thread that merely finished a run waits in **Ready for review**. A chat has one plan, and the lead can add steps to it as the work goes on.

## Edit the plan

Click **Edit** on the **Plan** bar, or **Edit plan** on the card. The editor opens in the Threads tab, so the lead's chat stays in view.

![The plan editor in the Threads tab](mission-plan-edit.jpg)

- **Overview** is the text every step is read against.
- Under **Steps**, drag a step by its grip to reorder it, type in its title to rename it, pick its role from the menu under the title, click the chevron to edit its brief, and click **×** to remove it. **Add step** adds one at the end.
- A step that has started shows its state in place of the grip. It keeps its place and cannot be removed, and a change to its brief is sent to its thread.
- Images on the overview and on a step stay with them when you save.

The foot counts your changes. **Save** applies them in one go and tells the lead what changed; **Cancel** asks before it discards them.

## How the lead hears back

- When a thread finishes a run, fails, or is stopped, Zeron tells the lead, with the thread's final reply. The lead does not have to wait or poll.
- When a thread stops on a question, the lead is woken with it and answers when the answer follows from the task. Anything else waits for you: in the thread, in **Waiting on you**, and in the **Waiting** pill.
- When you write to a thread yourself, the lead gets a note that quotes you.

## Limits

One setting caps what a lead has running at once: **Delegated tasks per chat** in [Settings > Agents > Routing](zeron://settings/routing), 8 by default. It counts delegated tasks, steps of the plan, and chats the lead started that are still working. **Delegation depth** caps how many levels of threads may nest. A lead at the limit is told so, and the step it tried to start stays in **Planned** until it tries again.

## Missions and Tasks

A task on the [Tasks](zeron://docs/tasks) page is a to-do. Start it with **Plan first** on its card and its chat writes a plan before it does anything; the task then shows that chat's progress, such as **2/5**, and sits in **Needs you** while the plan waits for approval or a step asks or fails. Marking the task done is still yours.
