Mission Control runs a big task with more than one agent. A lead agent plans the work into steps, runs each step on its own thread, checks the results and reports back. You can read and change the plan, approve it before anything runs, and watch every step live. A mission is a task on your [Tasks](zeron://docs/tasks) list, so it shows there with the rest of your work.

Mission Control introduces itself once, in a four-page sheet that appears after the update unless you have already started a mission: **Start with a goal**, **Approve the plan**, **Watch every step**, and **A lead and a Supervisor watch**. **Try Mission Control** on its last page opens New chat in Mission Control mode. To see the sheet again, choose **Mission Control intro** in the rail's **Help** menu.

## Start a mission from a task

Open a task's card on the Tasks page and pick the **Mission Control** tab under **Hand it to**. The card then shows:

- **Plan** picks what the lead writes:
  - **plan.md** (the default): a plan you can read and edit, with an overview (the goal, the approach, and how the result will be checked) and a spec for each step (what it builds, where, and how to tell it is done). The plan is stored in Zeron with the task, so tasks with **No project** get one too.
  - **Steps only**: just the steps, as a list of what to do.
- **Lead**: pick the lead's agent and model, and under it the project it runs **in**, or **Folder…**, the same way as on the **New chat** tab.
- **Reads**: chats the lead reads before it plans. See **Chats the lead reads** below.
- **Steps**: **Lead picks** (the default) lets the lead choose an agent and model for each step. **Same as lead** runs every step on the lead's agent and model.
- **Let me review the plan before it runs** (with **Steps only**: **Show me the plan before it runs**) is on by default: the lead writes the plan and waits for you. Turn it off to let the lead start the steps as soon as the plan is written.

A mission from the card always starts a new lead. To make a chat you already have the lead, start the mission from that chat's composer.

Click **Start mission** or press Cmd+Enter. Zeron starts the lead's chat and opens the task's Mission Control page.

## Chats the lead reads

A mission can read up to 5 chats you already have before it plans: a review that found the bugs, the chat where you worked out the design. In the **Reads** row, type **@** and part of a chat's name, and pick it from the list that opens, or press Enter for the first match. Each chat becomes a chip with its status and a ×; Backspace in the empty field removes the last one. A chat from another project carries that project's name. The list offers chats on this Mac only, because the lead can only read those, and leaves out drafts.

These chats are context, not assignments. Nothing is sent to them, and the task does not follow their status. The lead's first message lists each one by title and id, and the lead reads them with Zeron's `read_thread` tool before it writes the plan. The message tells it that they are context only: it does not write to them or answer their questions. A chat that is still working is fine; the lead is told to read it again before it sends the plan. When a step needs one of them, the lead names it in that step's instructions, so the step's thread can read it too.

If the lead's tools leave out `read_thread`, in its profile or in [Settings > Tools](zeron://settings/tools), the field turns off and says why. With chats already picked, **Start mission** waits until you remove them, turn `read_thread` back on, or pick another lead.

The list is fixed once the mission starts, and the Mission Control page shows it as read-only chips under the lead. To point the lead at another chat later, mention it with **@** in the lead's composer.

## Start a mission from a composer

You do not need a task card to start a mission. On the New chat canvas, the **Mission Control** tab starts one from what you type: see [Start a new chat](zeron://docs/new-chat).

In an open chat, press Shift+Tab in the composer, or click **Mission Control** under it, to turn the composer into a mission. Pick **plan.md** or **Steps only**, leave **Review plan first** on or turn it off, type the mission and click **Start mission**. The first line becomes the task's title and the rest its notes. Chats you mention in the text with **@** become the chats the lead reads, up to 5, as on the card; the lead's own chat is left out. A mission started from a composer has no **Steps** choice: the lead picks per step. Shift+Tab again goes back to the chat, and so does starting the mission. What happens next follows **In a chat, Mission Control** in [Settings > General](zeron://settings/general):

- **This chat leads**, the default: the chat becomes the mission's lead. It keeps its name and its agent, and the task with the lead's instructions arrives in it as a message, so the agent plans from the conversation you already had. If the agent is working, the message steers its turn instead of stopping it. The step threads show in the sidebar under this chat. A chat leads one unfinished mission at a time: while it still leads one, Zeron refuses a second and gives your text back.
- **New mission reads this chat**: Zeron starts a new lead with the chat's agent, model, permissions and project, and this chat is the first of the chats the lead reads before it plans. The chat shows **Started mission** with the task's name. Click **Open** for its Mission Control page, or **Dismiss** to hide the notice.

## The Mission Control page

A Mission Control task opens its own page instead of the card, from the Tasks list, the board, the Inbox, the **Missions** widget on Home, the Supervisor's **Missions** list, or the strip at the top of its chats. The back button in the titlebar takes you back: to the chat you came from, named after it, or else to **Tasks**.

The header reads **MISSION CONTROL** above the task's title, and shows the task's status (**Planning**, **Plan ready**, or its Kanban column), **Mark as done** or **Reopen**, and how many steps are done. Under it the plan is drawn as a tree:

- **Steps** at the top sums it up: how many steps, how many are working, a **needs you** and a **failed** button when any step needs you or failed, and the tokens the whole mission has used so far. Click **needs you** or **failed** for a small window that lists those steps, with **Open thread** and **Answer**, **Approve and run** for a plan that waits for you, or **Skip** and **Retry**.
- The lead's card on top: its agent and model, its state (**Not started**, **Planning**, **Working**, **Needs you**, **Done**), what it is doing (**The lead is planning**, **Watching** its running steps, **Working**, **Waiting on you**), and, with **plan.md**, the **Overview** the lead wrote. The overview shows one line; click **Overview** to read all of it. **Reads**, when the mission has chats to read, shows one chip per chat with its status, read-only. The bottom line has the tokens the lead and its steps have used, and a button with the number of steps that folds them away and brings them back.
- The steps, from the top down, alternating left and right of a line that runs down the middle, each joined to it. Each card shows the step's number, its agent and model (or **Not started**), its state (**Planned**, **Working**, **Needs you**, **Failed**, **Stopped**, **Interrupted**, **Done**, or **Skipped**), its title, and the tokens it has used and its time: the time so far while it works or waits on you, then the time its agent spent working. A failed step shows what went wrong. Click **spec** on a card to read what the lead wrote for it.
- **Add a step**, a dashed card at the end.

As the plan arrives, the lines draw themselves from the lead and the cards rise in one after another; when cards move, they glide to their new place. Running steps shimmer. With reduced motion, everything appears and moves at once.

Click a card to watch its thread live in a panel that slides in from the right, with a composer to write to it. The tree makes room for it. **Open thread** opens the full chat; × or Escape closes the panel. The lead, and any step that has not started, show the lead's chat.

The page keeps its place while you are away: go back to it from a chat, and the specs you opened, the edit you were writing, your unsent changes and the scroll position are as you left them.

## Approve the plan

While the lead writes the plan, its card reads **The lead is planning**. Then **Plan ready** on the lead's card shows how many steps there are, and the lead waits. Until you approve, the lead cannot start any step. Click **Approve and run** when it looks right. If you changed the plan and have not sent the changes yet, approving sends them first.

## Change the plan

You can change the plan once the lead has started, before you approve it and while the steps run, in both **plan.md** and **Steps only**. A task marked done cannot be edited.

- Open a step's **spec** and click **Edit**, or right-click the step and pick **Edit spec**, to change its title and its spec. The step's card opens for editing in its place and grows taller. Press Cmd+Enter or click **Save** to keep the change, or press Escape or click **Cancel** to drop it. Enter in the title saves too.
- With **plan.md**, **Edit** on the lead card's **Overview** changes the overview in the same kind of card.
- Type a new step in **Add a step** and press Enter.
- **Remove** takes out a step that has not started. **Restore** brings it back.
- Drag a step that has not started by the grip that takes the place of its agent's mark when you hover the card, to change the order; a bar shows where it will land. Or right-click it and pick **Move up** or **Move down**. A step can only land below the last step that has started; steps that are done or running have no grip, and keep their place. Because the cards alternate sides, the steps after a moved one glide to the other side. Steps you added and have not sent stay at the end, in the order you put them.

Right-click a step for **Edit spec**, **Open thread** (when the step has a thread), **Add image**, **Move up**, **Move down**, **Skip** or **Reopen**, and **Remove**. An item that does not apply to that step is greyed out. Escape or a click outside closes the menu.

Your changes are tagged **Edited**, **New**, **Removed**, or **New order**, and stay on this page until you send them: a bar at the bottom reads how many changes are unsent. **Undo** drops them all. **Send to lead** saves them all at once and sends the lead one message that lists what changed, including the new order of the steps, so the lead never works from a half-edited plan. When a step whose thread is already running changed, the lead tells that thread. Opening another task's Mission Control page, or the task being done or deleted, drops changes you have not sent.

## Images

A mission's overview and a step's spec can carry images, up to 12 each:

- While you edit a step's spec or the overview, paste or drop an image on the card, or click **＋ Add image** under the text. **Save** keeps the images and **Cancel** drops the ones you added.
- **Add image** in a step's right-click menu adds the picked images to its spec at once.
- Like any edit, they reach the lead when you click **Send to lead**.

Thumbnails show under the overview once you click **Overview** to open it, and in a step's open spec. Click one to see it large; the left and right arrows, or the ‹ › buttons, move between the images of that overview or step, and Escape or a click outside closes it.

The images go to the agent that does the work, the same way an image you attach in a chat does: the overview's images with the lead's first message (images added to the overview later go with the **Send to lead** message), and a step's images with the first message of the step's thread. An image added to a step whose thread has already started does not reach that thread; the edit card says so. The overview's images are the task's own, so images the task had before you started the mission show under the overview.

When the lead rewrites the plan, a step that has not started keeps its images if the new plan has exactly one step with the same title. Otherwise its images move to the overview, up to 12; the lead is told which images moved, and which did not fit.

## While the steps run

Steps that do not depend on each other run at the same time. The lead opens a new chat for each step. These are ordinary chats: each shows in the sidebar indented under the lead's chat, and clicking it, or **Open thread** on the task page, opens it like any other chat, where you can write to it. In the lead's chat, each one reads **Started thread** with its name and an **Open** link. **Delegated tasks per chat** in Settings > General caps how many steps one lead runs at once.

A step turns **Done** when its chat finishes a turn, and **Failed** when the chat fails. Zeron tells the lead either way and quotes the chat's last reply, cut short after 4,000 characters. Steps that finish within half a second of each other, or while that message still waits for the lead, share one message. In the lead's chat, the message starts with **From** and the name of each step thread it reports on; click a name to open that thread. Each reply shows as a quote under it. The lead reads the chat, then marks the step done or skipped, sends the chat what is still missing, or runs the step again on a new chat. Hover a step that is planned, needs you, or failed, and that the lead has not marked yet, to **Skip** it; hover one the lead has marked done or skipped to **Reopen** it. When a step's chat failed or was stopped, **Retry** asks the lead to look at what happened and fix it or run the step again.

When every step is done, the lead checks the whole result, tells you what was built, and marks the task done.

If a message to the lead, such as an approval or a retry, does not get through, the page shows **Not sent to the lead:** with the reason. Write to the lead in the composer instead, then click **Dismiss**.

The lead writes its plan, with the overview when you picked **plan.md**, and marks steps done or skipped with Zeron's `tasks` tool, and runs each step on a new chat with `create_thread`. See [Zeron tools for agents](zeron://docs/agent-tools).

Whether the lead's chat and the step chats compact on their own follows **In Mission Control** in [Settings > General](zeron://settings/general). See [Compaction and context](zeron://docs/compaction).

## Open Mission Control from a chat

The lead's chat and the chat of every step show a strip at the top of the conversation. It stays in place while you scroll, and shows:

- the task's progress ring and its title;
- **Lead**, or which step this chat runs, such as **Step 3 of 5**, numbered as on the Mission Control page;
- a bar with the steps done;
- **Mission Control**.

In the lead's chat, the strip also shows the mission as a tree: the lead with what it is doing, and each step on its own line under it with its state, its agent's mark and its time: the time so far while it works or waits on you, then the time its agent spent working. Running steps shimmer; the lines to each step draw themselves in as the plan arrives. The button at the end of the lead's line (**6** with a chevron) folds the steps away; click it again to bring them back. Click the strip's top row to fold the whole tree into the strip, and again to open it. With reduced motion, the tree appears and folds at once.

Click **Mission Control** in the strip, or any line of the tree, to open the task's Mission Control page with that thread live on the right. From a step's chat, clicking the strip opens the page with that step selected. The page slides in from the right over the chat. Back, or the chat's name in the titlebar, slides it out to the right and returns you to the chat where you left it, scrolled where it was. Opening the page from anywhere else in a chat, or going back and forward between them, moves the same way. With reduced motion, the page fades in and out instead. The strip goes away when the task is deleted.

## In the sidebar

A lead shows its step threads indented under its row. Hover the lead and click the chevron at its left edge, or use the right-click menu, to fold them away. A collapsed lead sums up its steps, for example **4 · 1 running**: the number of steps, then the most pressing of asking you, failed, or running. See [Sessions in the sidebar](zeron://docs/sessions-sidebar).

## On the Tasks page

In the list, a Mission Control task has a progress ring in place of its circle, and its line reads **Mission Control** and shows the agents on it, steps done out of the steps that are not skipped, and how many are running or need you. On the board, its card has a bar with one segment per step that is not skipped. A mission lands in **Needs you** while its plan waits for you, a step needs you or has failed, its lead asks you something or has failed, a message to the lead did not get through, or its lead is idle with no step running.

## The Supervisor

The [Supervisor](zeron://docs/supervisor) is one agent above the leads of all your missions. Open its page once, from **Supervisor** in the rail, to start it. From then on, every mission whose lead runs on this Mac is watched: Zeron wakes the Supervisor when a step fails, a thread waits on you, a mission finishes or goes quiet, the plan is edited after the mission started, or the lead escalates.

The Supervisor acts through the leads, within the switches you set under **Authority**: it messages a lead or a step's thread, answers their questions, re-briefs a step through its lead, pauses a step that blocks a shared resource, or starts a new mission, whose plan you review before any step runs. It never adds or removes a step and never changes the overview. A change to a mission's goal or spend comes to you as a **Needs you** card on the Supervisor page, and so does a decision a lead escalates.

The Supervisor's **Missions** list shows every mission that is not done, who works on it, and **Open Mission Control** for its page. Turn off **Watched by the Supervisor** there to keep a mission from waking it. A mission whose lead runs on another device is not watched.
