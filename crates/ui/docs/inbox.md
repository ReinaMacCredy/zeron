The Inbox page is a start-of-day view: what is waiting on you, today's tasks, how the day is going, and how much of your plan is left.

![The Inbox page](inbox.jpg)

## Open the Inbox

Click **Inbox** in the rail, or pick **Open inbox** in the Cmd+K palette. The rail button shows a count when chats are waiting for your input.

The line under the title greets you and sums things up, for example "Good morning. Tuesday, October 6. 2 things need you. 3 tasks to do." The day starts at 4 AM, so late-night work still counts toward the day before.

## Needs you

This tile lists every chat that is waiting on you, with what it wants: **approve a command**, **has a question**, or **waiting for you**. Click a row to open that chat.

For local git projects, a GitHub row also counts pull requests that need you, such as review requests, requested changes, or failing CI. Click it to open the [Pull requests](zeron://pull-requests) page. The tile is hidden when nothing needs you.

## Today's tasks

This tile shows open tasks due today or earlier, tasks with a chat, and tasks you finished today. Each row says **Overdue**, **Today**, or which chat it runs in. Tick a row to mark it done, or click it to open the task card, where you can **Start** it with an agent.

Type in the last row, **Add a task for today, or press Tab to hand it to an agent**. Enter adds a task due today. Tab adds it and opens the card ready to start. **All tasks** opens the [Tasks](zeron://docs/tasks) page.

## Your day so far

A short summary of the day: how many prompts you sent across how many chats, how long your agents ran, and how many of today's tasks are done. Below it, **Morning**, **Afternoon**, and **Evening** show your prompts by hour. Before anything runs it says **Nothing ran yet today. This fills in as your agents work.**

## Plan left

The last tile shows your tightest plan limit across your agent accounts, for example **Left on Claude Code, week**, with the percent left and when it resets. It only appears once an account reports usage. See [Usage](zeron://docs/usage) for every limit.

> The sidebar has its own Inbox mode: open the **...** menu above the sidebar (**Sidebar view options**) and pick **Inbox** to sort chats into **Inbox** and **Settled**, plus **Working** when **Show > Working** is on. See [Sessions in the sidebar](zeron://docs/sessions-sidebar).
