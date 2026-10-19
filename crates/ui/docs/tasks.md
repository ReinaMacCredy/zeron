The Tasks page is a to-do list for your work. Any task can be handed to an agent with one click, and the task then tracks that agent's chat.

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

The **⋯** button next to the close button holds **Mark as done** and **Delete**. Delete does not ask first. Press Escape or click the close button to close the card.

Under **Hand it to**, three tabs pick who does the task: **New chat**, **Thread** or **Mission Control**. Each tab shows only its own settings and its own button, and Cmd+Enter presses that button.

## Start a task with an agent

On the **New chat** tab, pick the agent and model. The line under it reads **in** and the project the chat runs in; click it to pick another, or click **Folder…** to run in any folder. Then click **Start** or press Cmd+Enter. The model picker is the same one the New chat composer shows: effort, fast mode, context and the other model settings, provider instances and profiles. The task's chat runs with exactly what you pick. Picks start out as the ones you last used on New chat, and changing them here leaves New chat as it was.

Zeron creates a chat named after the task, sends the title and notes as the first message, and opens the chat. The card then shows the chat's status and an **Open thread** button.

## Assign a task to a thread

To give a task to a chat you already have, open the **Thread** tab and type **@** and part of the chat's name. The matches float over the card; pick one, or press Enter for the first match. The picked chat shows with its status, and its × lets you pick another. Then click:

- **Send task**, or press Cmd+Enter: the title and notes go to that chat as a message. If the chat is working, the message steers its current turn.
- **Link only**: nothing is sent. The task just follows that chat's status.

A task has one chat at a time. Once it has one, the card shows the chat with **Open thread** and **Unassign**, so you can start or assign the task again.

When a task could not be sent to its chat, the list shows **Not sent**, and the card shows **Not sent:** with the reason and a **Retry** button that sends it to the same chat again.

## Run a big task with Mission Control

For work that needs more than one agent, open the **Mission Control** tab, then click **Start mission**. Its **Reads** row lists chats the lead reads before it plans. A lead agent plans the work into steps, runs each step on its own thread, checks the results and reports back, and the [Supervisor](zeron://docs/supervisor) can watch the mission from above. A Mission Control task opens its own page instead of the card, and in the list it has a progress ring in place of its circle. See [Mission Control](zeron://docs/mission-control).

## Images

A task can carry up to 12 images:

- On the Tasks list, paste an image (Cmd+V) into **Add a task, or press Tab to add notes and start it**, drop an image file on that row, or click the image button at its right. The images wait under the row with a × to take one out, and go with the task when you press Enter or Tab. Escape clears them with the text.
- In a task's card, paste or drop an image on the card, or click the image button next to **Project**. The images show under the note and save on their own; the × on a thumbnail removes it.

Thumbnails show in the task's row and on its board card. Click one to see it large; the left and right arrows, or the ‹ › buttons, move between the task's images, and Escape or a click outside closes it.

The images go to the agent that does the work, the same way an image you attach in a chat does: with the task's first message when you start or send it. Images are stored on this device with your chat attachments, and deleting a task leaves them there. Images on a mission's overview and steps are in [Mission Control](zeron://docs/mission-control).

## Tasks from agents

Agents can write tasks too, with Zeron's `tasks` tool: list them, add and edit them, mark them done, and assign them to a chat on this device. An agent's new task lands in that agent's project. A [Mission Control](zeron://docs/mission-control) lead uses the same tool to write its plan and mark its steps done. See [Agent tools](zeron://docs/agent-tools).

## Kanban columns

| Column | Holds |
| --- | --- |
| **To do** | Tasks with no chat yet |
| **Working** | Tasks whose agent is working |
| **Needs you** | Tasks whose agent stopped: finished, failed, or asking you something. A Mission Control task also lands here while its plan waits for you or a step needs you |
| **Done** | Tasks you marked done |

Drag a card to **Done** to finish it, or from **Done** back to **To do** to reopen it. **Working** and **Needs you** follow the chat, so you cannot drop cards there. A card stays in **Needs you** until you mark it done.

When tasks need you, the **Tasks** button in the rail shows how many.
