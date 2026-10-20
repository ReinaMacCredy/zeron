The Supervisor is one agent for this Mac's workspace that sits above the leads of your [Mission Control](zeron://docs/mission-control) missions. You talk with it about architecture and direction, and it watches the missions you leave on. It does not run all the time: Zeron wakes it when something happens in a mission, and it goes back to sleep when it has nothing to do.

## Open the Supervisor

Click **Supervisor**, the shield in the rail under **Tasks**. The first time you open the page, Zeron starts the Supervisor's chat. It uses your **SLP Supervisor** agent profile when you have one. Otherwise it uses the agent and model you last used on New chat, with permission at **Full**. When it cannot start, the status card reads **Couldn't start the Supervisor** with the reason: the agent the profile names is turned off in [Settings > Providers](zeron://settings/providers) or not installed on this Mac, or no agent is installed and turned on. Turn the agent on or give the profile another agent, then click **Try again**; it reads the profile again first.

The Supervisor's chat never shows in the sidebar or under **Archived**, and it never plays the run-finished sound. You reach it only from this page.

## What wakes it and what it can do

Zeron wakes the Supervisor when something happens in a watched mission: a step fails, a step or the lead waits on you, the mission finishes, the mission goes quiet for 30 minutes, the lead stays quiet for 10 minutes after a step failed, the plan is edited after the mission started, the lead escalates, or two missions contend for a shared resource. With **Decide for me** on, a plan that waits for review wakes it too. Events that arrive within a minute of each other come as one wake, and it wakes at most 12 times an hour.

It acts through the missions' leads, within the switches under **Authority**:

- It messages a lead or a step's thread, and answers their questions and tool approvals.
- It re-briefs a step: it rewrites that step's title and spec, and the lead gets one message saying what changed and why. It never changes the plan's overview and never adds or removes a step.
- It pauses a step that blocks a shared resource, and resumes it later. A pause needs a contention Zeron verified on this Mac (see **Resources** below): it stops the step's thread, the lead cannot run that step again until it is resumed, and the lead gets one message each time.
- It starts a new mission. The new lead writes a plan that you review before any step runs, or that the Supervisor approves when **Decide for me** is on.
- It decides for you, when you turn on **Decide for me** under **Authority**: it answers the leads' decisions and approves plans that wait for review. With **Change a mission's goal or spend** on too, it also decides goal and spend decisions and can change a mission's title and notes.
- It asks you. A change to a mission's goal or spend, or a decision it should not make alone, becomes a decision in the inbox.

A mission's lead can bring a decision to you too. When a decision changes the goal, costs money you have not approved, or cannot be settled from the plan, the lead escalates it as a decision in the inbox and keeps working on what does not depend on it. A lead escalates even when its mission is not watched; only the Supervisor's wake is skipped.

## The page

The page has two columns. On the left are the header, the inbox and the composer. On the right, the missions rail holds the Supervisor's status card and the tree of every mission.

The header shows the shield, **Supervisor**, a gear button, and the filter tabs. While either full-authority switch is on, **Full authority** shows next to **Supervisor**. The rail's status card says whether the Supervisor is on, what it last did, and what it checks next:

- **Watching 2 missions**, with a green dot that breathes, when it has nothing to do but still watches. **Last** names the last wake, its reason and what the Supervisor did about it, for example **14:06 · step failed → messaged the Lead · Lead ran a turn**. **Next** names the next scheduled check, for example **silence check on Checkout at 14:36 if it stays quiet**, a check-in, or held events and when they wake it. On the right, **3 of 12 wakes this hour**.
- **Working on a wake**, with a spinner and how long the turn has run, while it handles a wake (**Woke** names the time, the reason and the mission), or **Working on a message** while it answers you. **Open the conversation** opens the conversation above the composer.
- **Stopped by you**, in amber, after you stop its turn. Wakes queue up while it is stopped: **Held** lists their reasons and since when, and the right side counts them. **Resume** sends the waiting wakes; sending it a message resumes it too.
- **Off** when it has no chat. **Start the Supervisor** starts it.
- **Couldn't start the Supervisor** with the reason, and **Try again**.

Only **Stopped**, **Off** and **Couldn't start** tint the card, because they need you to act. When a wake arrives, the dot flashes amber and settles back, and when the state changes the card fades to the new one.

The number on the app rail's **Supervisor** item counts the open decisions, and it pops when a new one arrives.

### Window width

- **Wide windows** (1180 points and more): the rail is 384 points wide.
- **Between 860 and 1180 points**: the rail is 300 points wide. The trees drop each agent's mark and time, the mission rows drop the current step, the status card drops the wake count, and each mission's footer is one row: **Open Mission Control ›** and **Watched**. The chevron at the top of the rail, level with the gear, collapses it.
- **Below 860 points, or collapsed**: the rail is a 52-point strip with a chevron at the top that expands it again, the status dot, and one ring per mission with an amber count of its open decisions. Clicking a ring expands it too. While the strip shows, a status card that needs you (**Stopped**, **Off**, **Couldn't start**) moves under the header.

The widths count the window without the sidebar, so an open sidebar narrows the page as if the window were smaller. When the inbox is too narrow for a decision on one line, as when you open the rail on a narrow window, each question takes its own line with its time and its options wrap on the line below it.

## The inbox

The inbox groups everything open by mission. Each mission gets one surface with a header row: its progress ring, its name, how many decisions wait (**5 to answer**, in amber) and how many updates it has. A mission's decisions come first, newest first, then its updates. Missions with decisions come first, newest first; rows with no mission, and resources, gather under **Supervisor and shared**.

The tabs above the inbox count the open rows of each kind: **All**, **Needs you**, **Questions**, **Interventions** and **Resources**. A tab with nothing open is faint, and the **Needs you** count is amber while anything waits on you. Click a tab to show only that kind; click it again, or **All**, to show everything. A new decision brings the inbox back to **All**.

When nothing is open, the inbox is one line: **Nothing needs you**, and how many missions the Supervisor watches.

Rows leave the inbox when they are done, and wait in **Earlier** under it, closed by default. Click **Earlier** to see them.

### Needs you: the decision sheet

A mission's open decisions form one sheet, one row per decision: the question, a segmented choice with one segment per option, and the detail in faint text. They are decisions its lead or the Supervisor raised, and the questions and tool approvals its threads wait on. A question with several parts takes one line per part.

![The Supervisor's feed](supervisor-feed.jpg)

Picking an option does not send it. The pick is staged: its segment fills, and the row keeps its amber dot because nothing has been sent. Click another option to change it, or the same one to clear it. The footer counts the picks, **2 of 5 picked**, and **Send 2 to the Lead ⌘↵** sends every finished pick of that mission at once. ⌘↵ works after a click on the sheet. The lead gets all of the mission's decisions in one message, so it wakes once, not once per decision. A thread's question or approval is answered in that thread, the same way you answer it in the chat, so the button reads **to the step** when it answers a step.

Staged picks stay while Zeron runs, also when you leave the page and come back. They are not saved when Zeron quits: a decision can be answered elsewhere in the meantime, and an old pick would then send an answer you no longer meant.

After **Send**, each row turns into **✓ Who reviews? → Codex · Sending to the Lead · Undo**. For two seconds, **Undo** takes that row back to its staged pick. Then the answers go out, the row reads **Sent to the Lead**, and it folds away, with the rows below sliding up. It moves to **Earlier** with your choice. A decision answered anywhere else leaves the same way. Until it is sent, a decision counts as open in the mission's header, the tabs and the rail.

Hover a row for its buttons. **Reply** puts the decision into the Supervisor's composer as a quote and moves the cursor there; nothing is sent until you send it. **Open thread** takes you to the thread that asks. **Reply with a note** in the footer quotes all of the mission's open decisions. A question that takes a typed answer has **Reply in conversation** instead of options.

### When Zeron tells you

When a new decision appears, Zeron checks whether you are there:

- **You are looking at it**: Zeron is the active app and you are on this page, on Mission Control for that mission, or in the chat that raised it. Only the row appears.
- **You used Zeron in the last 3 minutes**: a notice in the window reads **Supervisor needs you** and the decision's title.
- **You have been away longer**: a desktop banner titled **Supervisor needs you**. Click it to open this page. It follows **Desktop notifications** in [Settings > Notifications](zeron://settings/notifications), and it shows even when **Only when in the background** is on.

### Questions

A question a step of a watched mission asked its lead and that no one has answered yet shows as a row under its mission: **Step 2 asked the Lead**, the question, and the time. A step's lead hears about each of its questions, and so do you. Click the row to open the step's thread. Tool approvals are not listed here; they are decisions in the sheet.

A question moves to **Earlier** once it is settled, with its answer: **by the Lead**, **by the Supervisor**, or **answered outside the Lead** when you or another thread answered it. A question is **Dropped** when the step's turn ended before anyone answered.

### Interventions

An intervention is something the Supervisor did through its tools: a message to a lead or a step, an answer to a thread's question, a re-brief, a step it paused or resumed, a mission it started, a decision it made for you, a plan it approved, or a goal it changed. Its row says what it did (**Messaged the Lead**), the Supervisor's own words, and the time.

A decision the Supervisor made reads **Decided for the Lead**, then the question, the option it chose and its reason, for example **Who reviews hello.txt? → Codex · Codex reviewed the last diffs**, with the options beside it and its choice filled. Click another option to change it: the lead gets your answer as a correction, the same way your own answer reaches it, and the Supervisor hears that you changed it. If the lead already acted on the Supervisor's choice, the correction asks it to undo or adjust what depends on it where it can and to tell you what could not be undone; Zeron undoes nothing by itself. The row then reads **(you changed it from Codex)**. An approved plan reads **Approved the plan of the Lead** with the Supervisor's reason; to take an approval back, edit the plan in Mission Control or message the lead.

The colored words on the right show only what has happened since, and change as it happens:

- **Sent**: the Supervisor's message went to the thread.
- **Lead ran a turn**: the lead answered after it.
- **Step paused**: the step is still paused and its thread is not running.
- **No effect yet**: **Sent** is still all that happened ten minutes later.

They never judge whether the action helped; the Supervisor's own words are its judgment.

An intervention moves to **Earlier** once the work changed after it, or after 24 hours: **Plan edited on steps 2, 3** (those steps' titles or specs changed since), **Step 2 thread completed a turn**, **Resource freed** (a check after a pause found the process gone or the port closed), or the paused step was resumed. An intervention that shows **No effect yet** stays in the inbox.

### Resources

A mission's lead can declare, in its plan, what the mission needs to itself while it runs: a port its servers listen on (**port:5173**), a quiet CPU for timing runs (**slot:benchmark**), or a folder it rewrites (**path:src/checkout**). A mission holds what it declared while any of its threads works or has written in the last 30 minutes.

A resource row appears under **Supervisor and shared** when a declared resource needs attention:

- **Contended**: two running missions share it, and Zeron verified it on the Mac. For a port, it checks which process listens on it (**Verified by lsof** and the time); for a CPU slot, how much CPU each other mission's processes used in two samples ten seconds apart (**Verified by ps**). Another mission using more than half a core in both samples makes the slot contended. When the missions share one folder, Zeron cannot tell them apart and names **a process in** that folder.
- **Declared by both, not verified**: two missions in the same checkout declared overlapping folders. Zeron does not check files and does not wake the Supervisor for it.
- **Not verified**: the check could not run.

Click the **Resources** tab to see every declared resource, with the ones above first and then those that need nothing: **Free** or **Held** when nobody else wants it, **Checking…** while Zeron checks, and **Clear** when the check found no conflict. Each shows the missions that declared it and the ones holding it now. A resource that stops needing attention moves to **Earlier**.

A contention wakes the Supervisor once, and again only after it has cleared. Zeron checks again when you open this page (at most once a minute), half a minute and two minutes after a pause, and when a thread of the missions sharing it starts or stops work (at most every two minutes). Resources are not shown on Windows.

## Missions

The rail lists every Mission Control task that is not done. Missions finished today stay below under **Done today**, folded, with a full ring and **Done**, the step count and the time it finished, and **Done · the Supervisor stopped watching it** in place of the switch. Each mission shows its progress ring, its name, the step it is on (**Step 2 of 5** and its title) and a thin progress bar. A mission the Supervisor does not watch reads **Not watched**; a mission whose lead runs on another device reads **on** that device's name, **not watched**: the Supervisor can only reach threads on its own Mac.

Each running mission is open, showing who works on it as the same tree the strip at the top of a lead's chat shows: the **Lead** with what it is doing, then each step under it in the order Mission Control numbers them, each with its state, its agent's mark and, while it works, the time so far. Click a mission's name to fold it. The lead says what waits on you, such as **Plan review, 1 question**, and reads **Idle** when nothing does. The fold button on the lead folds the steps away. Hover a row to see ↗, and click it to open that thread; a step with no thread yet shows › and opens Mission Control on that step. **Open Mission Control** opens the mission's page too, described in [Mission Control](zeron://docs/mission-control). **Watched by the Supervisor** at the bottom has a switch: turn it off and that mission stops waking the Supervisor; turn it on again to bring it back.

Hover a row in the inbox and its mission lights up in the rail: an amber border, and its lead row highlighted. When a new decision arrives, its mission glows briefly.

## Read a brief

Right-click the lead or a step and choose **Brief** to see what that agent was told, in a drawer that slides in from the right. Its header names the thread, the mission and the model, and the branch and commit the thread started on. Then:

- **Task**: the mission's title and notes.
- **Spec**: the plan's overview, and for a step, what the lead wrote for that step.
- **Messages it got**: every message the thread received, oldest first, each tagged by who sent it: **From you**, **From Lead**, **From Supervisor**, or the step that sent it.

Click the × to close the drawer.

## Authority and the wake log

Click the gear in the header to open **Authority** and the **Wake log**. Click outside it or press Esc to close it.

![The Supervisor's Authority popover with Decide for me](supervisor-authority.jpg)

**Authority** sets what the Supervisor may do on its own:

- **Message a Lead or Peer directly**: send messages to leads and step threads, and answer their questions. Answering a tool approval also needs the Supervisor's chat at **Full** access, its default.
- **Re-brief a step, through its Lead**
- **Pause a step that blocks a shared resource**: on by default, and only usable when Zeron has verified a contention. Not shown on Windows.
- **Start a new mission**: off by default. While it is off, the Supervisor cannot start new threads either.
- **Decide for me**: off by default. The Supervisor decides the leads' decisions whose reason is **Decision**, choosing one of the options and saying why, and approves a plan that waits for review, the way your **Approve** does. Each one shows under **Interventions**, where you can change it. Turning it on wakes the Supervisor for the decisions already waiting that it now covers.
- **Change a mission's goal or spend**: off by default, separate from **Decide for me**. The Supervisor also decides the leads' **Goal** and **Spend** decisions, and can change a mission's title and notes; the lead gets one message about it. It never deletes or reassigns a mission.

With both off, the Supervisor brings these to you as before. It never decides a question it raised itself, or anything in a mission it does not watch. **Check in on its own** wakes it every 30 or 60 minutes while a watched mission runs; it is **Off** by default. Your choices are saved on this Mac and stay after a restart.

The **Wake log** lists the recent wakes, newest first. Each shows its time, the events it carried, and how it reached the Supervisor: **Started a turn**, **Steered its turn**, **Held for turn end**, **Held by your Stop**, or **Failed** with the reason. Events that went stale before they were sent are listed as **Dropped**.

## Talk to the Supervisor

The composer sits under the inbox and has the same model picker as New chat. Type in **Talk to the Supervisor about architecture and direction…** and send.

Once the conversation has a message, a **Conversation** tab sits on the composer's top edge, left of the sidechat tab, with the last message in one line: the last wake, the Supervisor's last reply, or what you last wrote. Click it to open the conversation above the composer; click it again to close it. In the conversation, a wake reads **Wake** and its number of events, so you can tell it from what you wrote.

The page has motion: the blocks rise in when you open it, a new decision slides in and its counts roll, the tab underline slides and the inbox crossfades when you change the filter, an answer confirms and folds away, the status card fades between states, and the rail slides between its widths. Hovering is instant. With **Reduce motion** on in [Settings > Appearance](zeron://settings/appearance), all of it snaps instead.
