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

Under **More options** you find **Mark as done** and **Delete**. Delete does not ask first. Press Escape or click the close button to close the card.

## Start a task with an agent

Under **Hand it to an agent**, pick the agent and model, and choose where it runs in **Run in** or with **Folder…**. Then click **Start** or press Cmd+Enter. The agent and model start out as the ones you last used on New chat.

Zeron creates a chat named after the task, sends the title and notes as the first message, and opens the chat. The card then shows the chat's status and an **Open thread** button.

## Assign a task to a thread

To give a task to a chat you already have, type **@** and part of its name in **or assign to a thread** under **Hand it to an agent**. Pick the chat from the list, or press Enter for the first match. Then choose:

- **Send the task**: the title and notes go to that chat as a message. If the chat is working, the message steers its current turn.
- **Link only**: nothing is sent. The task just follows that chat's status.

Click **Assign** or press Cmd+Enter. A task has one chat at a time. The × next to the chat's status unassigns it, so you can start or assign the task again.

## Tasks from agents

Agents can write tasks too, with Zeron's `tasks` tool: list them, add and edit them, mark them done, and assign them to a chat on this device. An agent's new task lands in that agent's project. See [Agent tools](zeron://docs/agent-tools).

## Kanban columns

| Column | Holds |
| --- | --- |
| **To do** | Tasks with no chat yet |
| **Working** | Tasks whose agent is working |
| **Needs you** | Tasks whose agent stopped: finished, failed, or asking you something |
| **Done** | Tasks you marked done |

Drag a card to **Done** to finish it, or from **Done** back to **To do** to reopen it. **Working** and **Needs you** follow the chat, so you cannot drop cards there. A card stays in **Needs you** until you mark it done.

When tasks need you, the **Tasks** button in the rail shows how many.
