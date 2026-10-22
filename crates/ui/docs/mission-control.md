Mission Control follows every chat that started other agents, and every mission. A chat that hands work to other threads becomes the root of a tree: its children, their children, and the sub-agents they run. A mission is the same tree with a plan: a lead agent plans the work into steps, runs each step on its own thread, checks the results and reports back. You can add a plan to any chat at any time, read and change it, approve it before anything runs, and watch every thread live. A mission is a task on your [Tasks](zeron://docs/tasks) list, so it shows there with the rest of your work.

Mission Control introduces itself once, in a four-page sheet that appears after the update unless you have already started a mission: **Start with a goal**, **Approve the plan**, **Watch every step**, and **A lead and a Supervisor watch**. **Try Mission Control** on its last page opens New chat in Mission Control mode. To see the sheet again, choose **Mission Control intro** in the rail's **Help** menu.

## The Mission Control page

Click **Mission Control** in the rail, between **Tasks** and **Supervisor**. The page lists, newest activity first:

- **Running**: every mission that is not done, and every tree with a thread still working or waiting on you.
- **Done today**: missions finished today, and trees you marked done today or whose threads all went idle today.

A chat gets a row when it leads a plan or started another thread: a delegated task, a thread it created with `create_thread`, or a step. A sidechat, a fork, or Claude Code's own built-in sub-agents never make a row by themselves. A thread that belongs to a tree shows inside that tree's row, not as its own.

Each row shows a status mark, the title, a **Mission** or **Tree** badge, the lead's agent and model, steps done out of all steps, how many agents it started and how many of them are nested, one small bar per thread in its state's color, what it is doing (**Watching 2 steps, 1 agent**, **Idle**, or **Needs you** and who asks, from any depth), the tokens and time it has used, and a green shield with **Watched** when the [Supervisor](zeron://docs/supervisor) watches it.

The chips above the list filter it: **All**, **Missions**, **Trees** and **Needs you**, each with its count. On the right, **Supervisor watching 2 of 5 running** says how many running rows the Supervisor watches. **New mission** opens New chat in Mission Control mode.

Click a row to unfold its whole tree under it, and again to fold it. **Open mission** or **Open tree** at the bottom opens its page.

![The Mission Control page with a mission unfolded](mission-control.jpg)

## How threads link

Zeron reads how each thread started and links it to its parent:

- **Delegated**: a task a chat handed off with `delegate_task`, or a step thread the lead runs. The parent owns it: it fills a step, its parent hears when it finishes, and it counts in the tree's progress.
- **Opened from**: a thread a chat or an agent created with `create_thread`. It shows in the tree under its parent, but it is only a label: it does not fill a step or wake its parent. Matching it to a step with Add plan (below) makes it a step.
- **Forked from** and **sidechat of**: labels only. A fork or a sidechat starts its own tree.

## A tree's page

Click **Open tree** on a row, or a thread in the Supervisor's rail. The header shows the root chat's title with **Tree**, the lead's agent and model, when the tree started and **No plan**. On the right: **Watch** (see **Watched by the Supervisor** below), the tree's state, **Add plan**, **Mark as done** and **⋯**. **Mark as done** sets the root chat done without stopping anything; **Reopen** undoes it. A bar under them has one segment per thread, with agents done out of all agents, how many work, and the tokens and time.

![A tree's page in Outline, with the lead live on the right](mission-tree.jpg)

The titlebar switches the page between three views, for every tree and mission alike. Zeron remembers your choice; a new install starts on **Outline**.

- **Outline**: the whole tree as rows, at any depth, with **Steps** and **Agents** as two small groups under the lead when there is a plan. Click a row to see that thread on the right: who started it and when, which step it fills, its spec, its own children, its live transcript, **Open thread** and a composer. A built-in sub-agent's row opens its read-only transcript.
- **Spine**: the cards described under **A mission's page**. Without a plan, the lead card says **No plan** with **Add plan**, and every child is an **Agent** card. A card whose thread started others lists them inside it, **Started 2 agents**. With a plan, agents that fill no step sit after **Add a step**, under **Agents · started by the Lead, not in the plan**.
- **Timeline**: the outline on the left and, on the right, one bar per turn of each thread: blue while it runs, green when done, amber while it waited on an answer, labelled **Needs you** or **Waited 35s**. A dotted line drops from the parent's bar at the moment it started the child. Planned steps show as dashed pills after now. A finished tree fits its whole run; a live one follows the clock.

Switching views moves each card to its row and back; with reduced motion the view changes at once.

When a thread starts a new child, it rises into the tree on the page, the Mission Control list, the Supervisor's rail and the lead's strip at the same time, and a card that grows pushes the cards under it down. With reduced motion, everything appears at once.

## Add a plan to any chat

**Add plan** on a tree's page, on its lead card, or in a chat's right-click menu turns the chat into a mission's lead in place. Zeron makes a task named after the chat, and the chat gets the lead's instructions as a message: write a plan for the work it is already doing, do a step itself or with its own sub-agents and mark it done, start a new thread only when a step needs one, and match a thread it already started to a step. Then the plan waits for your review. The chat's own conversation continues where it was.

While the lead writes, its line reads **Planning**. The planned steps rise in above the threads it had already started, which move under **Agents**, and the badge turns from **Tree** to **Mission**. **Approve and run** starts the steps. A chat leads one unfinished plan at a time. Add plan does not show for sidechats, pull request chats, the Supervisor, or chats on another device.

## Watched by the Supervisor

Every mission is watched by the [Supervisor](zeron://docs/supervisor) unless you turn it off. A tree without a plan is watched only when you pick it: click **Watch** in its header, read what the Supervisor will do, set its two **Authority** switches if you want, and click **Start watching**. The button turns into **Watched** with a green dot. Click it again for **Stop watching**. With **Watch everything** on, every tree is watched.

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

In an open chat, press Shift+Tab in the composer, or click **Mission Control** under it, to turn the composer into a mission. Pick **plan.md** or **Steps only**, leave **Review plan first** on or turn it off, type the mission and click **Start mission**. The first line becomes the task's title and the rest its notes; a first line too long for a title is cut for the title, and the whole text goes to the notes. Chats you mention in the text with **@** become the chats the lead reads, up to 5, as on the card; past 5, a notice says the later ones were left out. The lead's own chat is left out. A mission started from a composer has no **Steps** choice: the lead picks per step. Shift+Tab again goes back to the chat, and so does starting the mission. What happens next follows **In a chat, Mission Control** in [Settings > General](zeron://settings/general):

- **This chat leads**, the default: the chat becomes the mission's lead. It keeps its name and its agent, and the task with the lead's instructions arrives in it as a message, so the agent plans from the conversation you already had. If the agent is working, the message steers its turn instead of stopping it. The step threads show in the sidebar under this chat. A chat leads one unfinished mission at a time: while it still leads one, Zeron refuses a second and gives your text back.
- **New mission reads this chat**: Zeron starts a new lead with the chat's agent, model, permissions and project, and this chat is the first of the chats the lead reads before it plans. The chat shows **Started mission** with the task's name until the mission is done or deleted. Click **Open** for its Mission Control page, or **Dismiss** to hide the notice.

## A mission's page

A Mission Control task opens its own page instead of the card, from the Tasks list, the board, the Inbox, the **Missions** widget on Home, the Supervisor's **Missions** list, or the strip at the top of its chats. The back button in the titlebar takes you back: to the chat you came from, named after it, or else to **Tasks**.

![A mission's plan as a tree](mission-control-tree.jpg)

The header shows the task's title with **Mission**, and under it the lead's agent and model and when the mission started, or when it finished. On the right:

- **Watched** with a green dot while the Supervisor watches the mission, or **Watch**.
- A status pill: **Needs you** with what waits (**plan review**, **1 question**), **1 step failed**, **✓ Done**, or the lead's state (**Planning**, **Working**, **Idle**). Click **Needs you** or **failed** for a small window that lists those steps, with **Open thread** and **Answer**, **Approve and run** for a plan that waits for you, or **Skip** and **Retry**.
- The next thing to do, when there is one: **Answer** (or **Approve and run**) for the first thing that waits on you, or **Retry step N** for the first failed step.
- **Mark as done**, or **Reopen** once the mission is done, and **⋯** with **Open lead thread** and **Fold the steps**.

Under them a bar has one segment per step in its state's color: green when done, amber when it needs you, red when it failed, blue with a moving shine while it works, faint when planned. Hover a segment to name its step; click it to open that step's thread on the right. Next to the bar: steps done out of all steps, how many are working, the tokens the whole mission has used so far, and how long it has run (**took** and the time it took, once done).

In **Spine**, under the header the plan is drawn as a tree:

- The lead's card on top: its agent and model, its state (**Not started**, **Planning**, **Working**, **Needs you**, **Done**), what it is doing (**The lead is planning**, **Watching** its running steps, **Working**, what waits on you such as **Plan review, 1 question**, or **Idle**), and, with **plan.md**, the **Overview** the lead wrote. The overview shows one line; click **Overview** to read all of it. **Reads**, when the mission has chats to read, shows one chip per chat with its status, read-only. The bottom line has the tokens the lead and its steps have used, and a button with the number of steps that folds them away and brings them back.
- The steps, from the top down, alternating left and right of a line that runs down the middle, each joined to it. Each card shows the step's number with its agent and model (or **Not started**) above its title, its state (**Planned**, **Working**, **Needs you**, **Failed**, **Stopped**, **Interrupted**, **Done**, or **Skipped**), and at the bottom the tokens it has used and its time: the time so far while it works, then the time its agent spent working. A step that needs you shows its question with **Answer**, which opens its thread; a failed step shows what went wrong with **Retry**. Click **Spec** at the bottom right, which counts the spec's images, to open what the lead wrote for the step in a box under it, with **Edit**, **Add image** and **Open thread**. The spec slides open and closed, and the cards below move with it.
- **Add a step**, a dashed card at the end.

As the plan arrives, the lines draw themselves from the lead and the cards rise in one after another; when cards move, they glide to their new place. Running steps shimmer. With reduced motion, everything appears and moves at once.

Click a card to watch its thread live in a panel that slides in from the right, with a composer to write to it. The tree makes room for it. **Open thread** opens the full chat; × or Escape closes the panel. The lead, and any step that has not started, show the lead's chat.

The page keeps its place while you are away: go back to it from a chat, and the specs you opened, the edit you were writing, your unsent changes and the scroll position are as you left them.

## Approve the plan

While the lead writes the plan, its card reads **The lead is planning**. Then **Plan ready** on the lead's card shows how many steps there are, and the lead waits. Until you approve, the lead cannot start any step. Click **Approve and run** when it looks right. If you changed the plan and have not sent the changes yet, approving sends them first. Each step has a role, and [Routing](zeron://docs/routing) decides which agent and model runs it. The lead's chat shows the plan as a card with that estimate, the cost against running every step on the lead's model, and its own **Start mission** button.

With **Decide for me** on, the [Supervisor](zeron://docs/supervisor) may approve the plan for you. The lead card then reads **Approved by the Supervisor · starts in 0:24** for 30 seconds before the lead hears of it, with **Take back**, which returns the plan to review and brings **Approve and run** back. **Take back** stays until the first step starts. After that the card reads **Approved by the Supervisor · steps already started**: change course by editing the plan here or messaging the lead.

## Change the plan

You can change the plan once the lead has started, before you approve it and while the steps run, in both **plan.md** and **Steps only**. A task marked done cannot be edited.

- Open a step's **Spec** and click **Edit**, or right-click the step and pick **Edit spec**, to change its title and its spec. The title turns into a field in place and the spec's box turns into the editor, which grows as you type. Press Cmd+Enter or click **Save** to keep the change, or press Escape or click **Cancel** to drop it. Enter in the title saves too.
- With **plan.md**, **Edit** on the lead card's **Overview** changes the overview in an edit card in its place, with the same keys.
- Type a new step in **Add a step** and press Enter.
- **Remove** takes out a step that has not started. **Restore** brings it back.
- Drag a step that has not started by the grip that takes the place of its agent's mark when you hover the card, to change the order; a bar shows where it will land. Or right-click it and pick **Move up** or **Move down**. A step can only land below the last step that has started; steps that are done or running have no grip, and keep their place. Because the cards alternate sides, the steps after a moved one glide to the other side. Steps you added and have not sent stay at the end, in the order you put them.

Right-click a step for **Edit spec**, **Open thread** (when the step has a thread), **Add image**, **Move up**, **Move down**, **Skip** or **Reopen**, and **Remove**. An item that does not apply to that step is greyed out. Escape or a click outside closes the menu.

Your changes are tagged **Edited**, **New**, **Removed**, or **New order**, and stay on this page until you send them: a bar at the bottom reads how many changes are unsent. **Undo** drops them all. **Send to lead** saves them all at once and sends the lead one message that lists what changed, including the new order of the steps, so the lead never works from a half-edited plan. When a step whose thread is already running changed, the lead tells that thread. Opening another task's Mission Control page, or the task being done or deleted, drops changes you have not sent.

## Images

A mission's overview and a step's spec can carry images, up to 12 each:

- While you edit a step's spec or the overview, paste or drop an image on the card, or click **＋ Add image** under the text. **Save** keeps the images and **Cancel** drops the ones you added.
- **Add image** in a step's open spec or its right-click menu adds the picked images to its spec at once.
- Like any edit, they reach the lead when you click **Send to lead**.

Thumbnails show under the overview once you click **Overview** to open it, and in a step's open spec. Click one to see it large; the left and right arrows, or the ‹ › buttons, move between the images of that overview or step, and Escape or a click outside closes it.

The images go to the agent that does the work, the same way an image you attach in a chat does: the overview's images with the lead's first message (images added to the overview later go with the **Send to lead** message), and a step's images with the first message of the step's thread. An image added to a step whose thread has already started does not reach that thread; the edit card says so. The overview's images are the task's own, so images the task had before you started the mission show under the overview.

When the lead rewrites the plan, a step that has not started keeps its images if the new plan has exactly one step with the same title. Otherwise its images move to the overview, up to 12, and the lead is told which images moved. A rewrite whose images would not fit on the overview is refused, so no image is lost.

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

![The mission strip at the top of a lead's chat](mission-control-strip.jpg)

Click **Mission Control** in the strip to open the task's Mission Control page with that thread live on the right. Click a step's line in the tree to open that step's chat; a step with no thread yet opens Mission Control on that step. The lead's own line does nothing there, since you are already in its chat. From a step's chat, clicking the strip opens the page with that step selected. The page slides in from the right over the chat. Back, or the chat's name in the titlebar, slides it out to the right and returns you to the chat where you left it, scrolled where it was. Opening the page from anywhere else in a chat, or going back and forward between them, moves the same way. With reduced motion, the page fades in and out instead. The strip goes away when the task is deleted.

## In the sidebar

A lead shows its step threads indented under its row. Hover the lead and click the chevron at its left edge, or use the right-click menu, to fold them away. A collapsed lead sums up its steps, for example **4 · 1 running**: the number of steps, then the most pressing of asking you, failed, or running. See [Sessions in the sidebar](zeron://docs/sessions-sidebar).

## On the Tasks page

In the list, a Mission Control task has a progress ring in place of its circle, and its line reads **Mission Control** and shows the agents on it, steps done out of the steps that are not skipped, and how many are running or need you. On the board, its card has a bar with one segment per step that is not skipped. A mission lands in **Needs you** while its plan waits for you, a step needs you or has failed, its lead asks you something or has failed, a message to the lead did not get through, or its lead is idle with no step running.

## The Supervisor

The [Supervisor](zeron://docs/supervisor) is one agent above the leads of all your missions. Open its page once, from **Supervisor** in the rail, to start it. From then on, every mission whose lead runs on this Mac is watched: Zeron wakes the Supervisor when a step fails, a thread waits on you, a mission finishes or goes quiet, the plan is edited after the mission started, or the lead escalates.

The Supervisor acts through the leads, within the switches you set under **Authority**: it messages a lead or a step's thread, answers their questions, re-briefs a step through its lead, pauses a step that blocks a shared resource, or starts a new mission, whose plan you review before any step runs. It never adds or removes a step and never changes the overview. A change to a mission's goal or spend comes to you as a **Needs you** card on the Supervisor page, and so does a decision a lead escalates. When you turn on **Decide for me** under **Authority**, the Supervisor decides the leads' decisions and approves plans itself. **Change a mission's goal or spend** is a separate switch: on its own it lets the Supervisor decide goal and spend decisions and change a mission's title and notes. Each decision shows on the Supervisor page, where you can change it.

The Supervisor's rail shows every mission that is not done and every tree it watches, who works on it, and **Open Mission Control** for its page. Turn off **Watched by the Supervisor** there to keep a mission or tree from waking it. A mission whose lead runs on another device is not watched.
