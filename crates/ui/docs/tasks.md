The Tasks page is a to-do list for your work. Any task can be handed to an agent with one click, and the task then tracks that agent's chat.

Mission Control introduces itself once, in a three-page sheet that appears after the update unless you have already started a mission. **Try Mission Control** on its last page opens New chat in Mission Control mode.

![Tasks on the Kanban board](kanban.jpg)

## Open Tasks

Click **Tasks** in the rail, or pick **Tasks** in the Cmd+K palette. The Inbox's **All tasks** link opens it too. In the titlebar, switch between **List** and **Kanban**. Zeron remembers your choice.

## Add a task

In **List**, type in the last row, **Add a task, or press Tab to add notes and start it**.

- Press Enter to add the task.
- Press Tab to add it and open its card, with the cursor in the notes.
- Press Escape to clear the row.

A new task belongs to the project you have selected for New chat. Drag open tasks up and down to reorder them, and tick the circle to mark one done. Finished tasks collect under **Done**.

## The task card

Click a task to open its card beside the list. Edits save on their own. You can set:

- the title and a note (**Add a note**)
- a due date: **No date**, **Today**, or **Tomorrow**
- the project, or **No project**

Under **More options** you find **Mark as done** and **Delete**. Delete does not ask first. Press Escape or click the close button to close the card.

## Start a task with an agent

Under **Hand it to an agent**, pick the agent and model, and choose where it runs in **Run in** or with **Folder…**. Then click **Start** or press Cmd+Enter. The model picker is the same one the New chat composer shows: effort, fast mode, context and the other model settings, provider instances and profiles. The task's chat runs with exactly what you pick. Picks start out as the ones you last used on New chat, and changing them here leaves New chat as it was.

Zeron creates a chat named after the task, sends the title and notes as the first message, and opens the chat. The card then shows the chat's status and an **Open thread** button.

## Assign a task to a thread

To give a task to a chat you already have, type **@** and part of its name in **or assign to a thread** under **Hand it to an agent**. Pick the chat from the list, or press Enter for the first match. Then choose:

- **Send the task**: the title and notes go to that chat as a message. If the chat is working, the message steers its current turn.
- **Link only**: nothing is sent. The task just follows that chat's status.

Click **Assign** or press Cmd+Enter. A task has one chat at a time. The × next to the chat's status unassigns it, so you can start or assign the task again.

When a task could not be sent to its chat, the list shows **Not sent**, and the card shows **Not sent:** with the reason and a **Retry** button that sends it to the same chat again.

## Run a big task with Mission Control

For work that needs more than one agent, switch **Hand it to an agent** from **One agent** to **Mission Control**. A lead agent plans the work into steps, runs each step on its own thread, checks the results and reports back.

- **Plan** picks what the lead writes:
  - **plan.md** (the default): a plan you can read and edit, with an overview (the goal, the approach, and how the result will be checked) and a spec for each step (what it builds, where, and how to tell it is done). The plan is stored in Zeron with the task, so tasks with **No project** get one too.
  - **Steps only**: just the steps, as a list of what to do.
- Pick the lead's agent and model, and where it runs, the same way as for one agent.
- **Step agents**: **Lead picks per step** lets the lead choose an agent and model for each step. **Same as the lead** runs every step on the lead's agent and model.
- **Let me review the plan before it runs** (with **Steps only**: **Show me the plan before it runs**) is on by default: the lead writes the plan and waits for you. Turn it off to let the lead start the steps as soon as the plan is written.

Click **Start mission** or press Cmd+Enter. Zeron starts the lead's chat and opens the task's Mission Control page.

## Start a mission from a composer

You do not need a task card to start a mission. On the New chat canvas, the **Mission Control** tab starts one from what you type: see [Start a new chat](zeron://docs/new-chat).

In an open chat, press Shift+Tab in the composer, or click **Mission Control** under it, to turn the composer into a mission. Pick **plan.md** or **Steps only**, leave **Review plan first** on or turn it off, type the mission and click **Start mission**. The first line becomes the task's title and the rest its notes. Shift+Tab again goes back to the chat, and so does starting the mission. What happens next follows **In a chat, Mission Control** in [Settings > General](zeron://settings/general):

- **This chat leads**, the default: the chat becomes the mission's lead. It keeps its name and its agent, and the task with the lead's instructions arrives in it as a message, so the agent plans from the conversation you already had. If the agent is working, the message steers its turn instead of stopping it. The step threads show in the sidebar under this chat. A chat leads one unfinished mission at a time: while it still leads one, Zeron refuses a second and gives your text back.
- **New mission reads this chat**: Zeron starts a new lead with the chat's agent, model, permissions and project, and the task's notes ask the lead to read this chat before it plans. The chat shows **Started mission** with the task's name. Click **Open** for its Mission Control page, or **Dismiss** to hide the notice.

## The Mission Control page

A Mission Control task opens its own page instead of the card, from the list, the board, the Inbox, or the strip at the top of its chats. The back button in the titlebar takes you back: to the chat you came from, named after it, or else to **Tasks**.

The header reads **MISSION CONTROL** above the task's title, and shows the task's status, **Mark as done** or **Reopen**, and how many steps are done. On the left is the plan:

- The lead, with its model and what it is doing: **Planning**, **Watching** its running steps, **Waiting on you**, or **Done**.
- With **plan.md**, the **Overview** the lead wrote.
- The steps in order. Each one shows its thread and its state: **Planned**, **Working** with the time so far, **Asked you a question**, **Failed**, **Stopped**, **Interrupted**, **Done**, or **Skipped**. Click **spec** under a step to read what the lead wrote for it.

Click a step to watch its thread live on the right, with a composer to write to it. **Open thread** opens the full chat. The lead, and any step that has not started, show the lead's chat.

## Open Mission Control from a chat

The lead's chat and the chat of every step show a strip at the top of the conversation. It stays in place while you scroll, and shows:

- the task's progress ring and its title;
- **Lead**, or which step this chat runs, such as **Step 3 of 5**, numbered as on the Mission Control page;
- a bar with the steps done;
- **Mission Control**.

Click the strip to open the task's Mission Control page with this chat live on the right. From a step's chat, that step is selected. On the way in, the chat shrinks into the right-hand pane while the steps slide in. Back, or the chat's name in the titlebar, returns you to the chat where you left it, and the pane grows back into the chat. With reduced motion, the page fades in instead. The strip goes away when the task is deleted.

## Approve the plan

While the lead writes the plan, the page reads **The lead is planning**. Then **Plan ready** shows how many steps there are, and the lead waits. Until you approve, the lead cannot start any step. Click **Approve and run** when it looks right. If you changed the plan and have not sent the changes yet, approving sends them first.

## Change the plan

You can change the plan before you approve it and while the steps run, in both **plan.md** and **Steps only**:

- Open a step's **spec** and click **Edit** to change its title and its spec. Press Cmd+Enter or click **Done** to keep the change, or Escape to drop it.
- With **plan.md**, **Edit** on the **Overview** changes the overview.
- Type a new step in **Add a step** and press Enter.
- **Remove** takes out a step that has not started. **Restore** brings it back.

Your changes are tagged **Edited**, **New**, or **Removed**, and stay on this page until you send them: a bar at the bottom reads how many changes are unsent. **Undo** drops them all. **Send to lead** saves them all at once and sends the lead one message that lists what changed, so the lead never works from a half-edited plan. When a step whose thread is already running changed, the lead tells that thread. Leaving the page drops changes you have not sent.

## While the steps run

Steps that do not depend on each other run at the same time. The lead opens a new chat for each step. These are ordinary chats: each shows in the sidebar indented under the lead's chat, and clicking it, or **Open thread** on the task page, opens it like any other chat, where you can write to it. In the lead's chat, each one reads **Started thread** with its name and an **Open** link. **Delegated tasks per chat** in Settings > General caps how many steps one lead runs at once.

A step turns **Done** when its chat finishes a turn, and **Failed** when the chat fails. Zeron tells the lead either way and quotes the chat's last reply, cut short after 4,000 characters. Steps that finish within half a second of each other, or while that message still waits for the lead, share one message. In the lead's chat, the message starts with **From** and the name of each step thread it reports on; click a name to open that thread. Each reply shows as a quote under it. The lead reads the chat, then marks the step done or skipped, sends the chat what is still missing, or runs the step again on a new chat. Hover a step to **Skip** it, or to **Reopen** one that is done or skipped. When a step's chat failed or was stopped, **Retry** asks the lead to look at what happened and fix it or run the step again.

When every step is done, the lead checks the whole result, tells you what was built, and marks the task done.

If a message to the lead, such as an approval or a retry, does not get through, the page shows **Not sent to the lead:** with the reason. Write to the lead in the composer instead, then click **Dismiss**.

Whether the lead's chat and the step chats compact on their own follows **In Mission Control** in [Settings > General](zeron://settings/general). See [Compaction and context](zeron://docs/compaction).

In the list, a Mission Control task has a progress ring in place of its circle, and its line reads **Mission Control** and shows the agents on it, steps done out of all steps, and how many are running or need you. On the board, its card has a bar with one segment per step.

## Tasks from agents

Agents can write tasks too, with Zeron's `tasks` tool: list them, add and edit them, mark them done, and assign them to a chat on this device. An agent's new task lands in that agent's project. A Mission Control lead uses the same tool to write its plan, with the overview when you picked **plan.md**, and to mark steps done or skipped, and runs each step on a new chat with `create_thread`. See [Agent tools](zeron://docs/agent-tools).

## Kanban columns

| Column | Holds |
| --- | --- |
| **To do** | Tasks with no chat yet |
| **Working** | Tasks whose agent is working |
| **Needs you** | Tasks whose agent stopped: finished, failed, or asking you something. A Mission Control task also lands here while its plan waits for you or a step needs you |
| **Done** | Tasks you marked done |

Drag a card to **Done** to finish it, or from **Done** back to **To do** to reopen it. **Working** and **Needs you** follow the chat, so you cannot drop cards there. A card stays in **Needs you** until you mark it done.

When tasks need you, the **Tasks** button in the rail shows how many.
