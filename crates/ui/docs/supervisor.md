The Supervisor is one agent for this Mac's workspace that sits above the leads of your Mission Control missions. You talk with it about architecture and direction, and it watches the missions you leave on. It does not run all the time: Zeron wakes it when something happens in a mission, and it goes back to sleep when it has nothing to do.

## Open the Supervisor

Click **Supervisor**, the shield in the rail under **Tasks**. The first time you open the page, Zeron starts the Supervisor's chat. It uses your **SLP Supervisor** agent profile when you have one, and otherwise the agent and model you last used on New chat. Its permission starts at **Full**.

The Supervisor's chat never shows in the sidebar or under **Archived**, and it never plays the run-finished sound. You reach it only from this page.

## What wakes it and what it can do

Zeron wakes the Supervisor when something happens in a watched mission: a step fails, a step or the lead waits on you, the mission finishes, the mission goes quiet for 30 minutes, the lead stays quiet for 10 minutes after a step failed, the plan is edited after the mission started, the lead escalates, or two missions contend for a shared resource. Events that arrive within a minute of each other come as one wake, and it wakes at most 12 times an hour.

It acts through the missions' leads, within the switches under **Authority**:

- It messages a lead or a step's thread, and answers their questions and tool approvals.
- It re-briefs a step: it rewrites that step's title and spec, and the lead gets one message saying what changed and why. It never changes the plan's overview and never adds or removes a step.
- It pauses a step that blocks a shared resource, and resumes it later. A pause needs a contention Zeron verified on this Mac (see **Resources** below): it stops the step's thread, the lead cannot run that step again until it is resumed, and the lead gets one message each time.
- It starts a new mission. The new lead writes a plan that you review before any step runs.
- It asks you. A change to a mission's goal or spend, or a decision it should not make alone, becomes a **Needs you** card.

A mission's lead can bring a decision to you too. When a decision changes the goal, costs money you have not approved, or cannot be settled from the plan, the lead escalates it as a **Needs you** card and keeps working on what does not depend on it. A lead escalates even when its mission is not watched; only the Supervisor's wake is skipped.

## The header

The header shows the shield and **Supervisor**, the Supervisor's state on the right, and a gear button.

- **Awake · watching 2 missions**, with a green dot that breathes, while the Supervisor works, on a wake or on your message.
- **Asleep · watching 2 missions**, with a grey dot, when it has nothing to do.
- **Stopped · 2 wakes waiting · send a message to resume**, in amber, after you stop its turn. Wakes queue up while it is stopped; send it a message to resume, and the waiting wakes follow.
- **Off** before its chat exists.

Hover the state to see when it last woke and why, for example **Last woke 14:06 for step failed**. When a wake arrives, the dot flashes amber and settles back.

The number on the rail's **Supervisor** item counts the open **Needs you** cards, and it pops when a new one arrives.

## The feed

Under the header, one feed lists everything open, one card per row. Each card starts with its kind: **Needs you**, **Question**, **Intervention** or **Resource**. **Needs you** cards are always on top, newest first; everything else follows by time, newest first.

The chips above the feed count the open rows of each kind: **All**, **Needs you**, **Questions**, **Interventions** and **Resources**. **Needs you** is tinted while anything waits on you. Click a chip to show only that kind; click it again, or **All**, to show everything. A new **Needs you** card brings the feed back to **All**.

When nothing is open, the feed reads **All quiet** and how many missions the Supervisor watches.

Rows leave the feed when they are done, and wait in **Earlier** at the bottom of the feed, closed by default. Click **Earlier** to see them.

### Needs you

A **Needs you** card holds what waits on your decision.

- **A decision** raised by a mission's lead or by the Supervisor: who raised it, the mission, why it comes to you (**Goal**, **Spend** or **Decision**), the details, and one button per option. Click an option to answer, and the mission's lead gets one message with your choice. When the Supervisor raised it, the Supervisor gets your choice too.
- **A question or an approval** a thread of a watched mission is waiting on: the lead's own questions, any step's tool approvals, and the Supervisor's own questions. The card names the thread that asks and its mission, then the questions and their options. Click an option to answer, the same way you answer in the chat; with several questions, pick one option for each and click **Send answers**. **Open thread** takes you to the thread, where you can type an answer instead.

**Discuss** on either kind puts the card into the Supervisor's composer as a quote and moves the cursor there. Nothing is sent until you send it.

Once Zeron has your answer, the card folds away and moves to **Earlier**, with your choice when you answered it on this page. A card answered anywhere else leaves the same way.

### When Zeron tells you

When a new **Needs you** card appears, Zeron checks whether you are there:

- **You are looking at it**: Zeron is the active app and you are on this page, on Mission Control for that mission, or in the chat that raised it. Only the card appears.
- **You used Zeron in the last 3 minutes**: a notice in the window reads **Supervisor needs you** and the card's title.
- **You have been away longer**: a desktop banner titled **Supervisor needs you**. Click it to open this page. It follows **Desktop notifications** in [Settings > Notifications](zeron://settings/notifications), and it shows even when **Only when in the background** is on.

### Questions

A **Question** card is a question a step of a watched mission asked its lead and that no one has answered yet. It shows the question, the step as Mission Control numbers it, the mission, and the time. A step's lead hears about each of its questions, and so do you. Click the card to open the step's thread. Tool approvals are not listed here; they come as **Needs you** cards.

A question moves to **Earlier** once it is settled, with its answer: **by the Lead**, **by the Supervisor**, or **answered outside the Lead** when you or another thread answered it. A question is **Dropped** when the step's turn ended before anyone answered.

### Interventions

An **Intervention** card is something the Supervisor did through its tools: a message to a lead or a step, an answer to a thread's question, a re-brief, a step it paused or resumed, or a mission it started. It shows the Supervisor's own words, the kind of action and the thread it went to, what led to it (**after the 14:05 wake** or **after your message**), and the time.

The chip on the right shows only what has happened since, and changes as it happens:

- **Sent**: the Supervisor's message went to the thread.
- **Lead ran a turn**: the lead answered after it.
- **Step paused**: the step is still paused and its thread is not running.
- **No effect yet**: **Sent** is still all that happened ten minutes later.

The chips never judge whether the action helped; the Supervisor's own words are its judgment.

An intervention moves to **Earlier** once the work changed after it, or after 24 hours: **Plan edited on steps 2, 3** (those steps' titles or specs changed since), **Step 2 thread completed a turn**, **Resource freed** (a check after a pause found the process gone or the port closed), or the paused step was resumed. An intervention that shows **No effect yet** stays in the feed.

### Resources

A mission's lead can declare, in its plan, what the mission needs to itself while it runs: a port its servers listen on (**port:5173**), a quiet CPU for timing runs (**slot:benchmark**), or a folder it rewrites (**path:src/checkout**). A mission holds what it declared while any of its threads works or has written in the last 30 minutes.

A **Resource** card appears when a declared resource needs attention:

- **Contended**: two running missions share it, and Zeron verified it on the Mac. For a port, it checks which process listens on it (**Verified by lsof** and the time); for a CPU slot, how much CPU each other mission's processes used in two samples ten seconds apart (**Verified by ps**). Another mission using more than half a core in both samples makes the slot contended. When the missions share one folder, Zeron cannot tell them apart and names **a process in** that folder.
- **Declared by both, not verified**: two missions in the same checkout declared overlapping folders. Zeron does not check files and does not wake the Supervisor for it.
- **Not verified**: the check could not run.

Click the **Resources** chip to see every declared resource, with the ones above first and then those that need nothing: **Free** or **Held** when nobody else wants it, **Checking…** while Zeron checks, and **Clear** when the check found no conflict. Each shows the missions that declared it and the ones holding it now. A resource that stops needing attention moves to **Earlier**.

A contention wakes the Supervisor once, and again only after it has cleared. Zeron checks again when you open this page (at most once a minute), half a minute and two minutes after a pause, and when a thread of the missions sharing it starts or stops work (at most every two minutes). Resources are not shown on Windows.

## Missions

**Missions**, under the feed, lists every Mission Control task that is not done. Each row shows its progress ring, its name, the step it is on (**Step 2 of 5** and its title) and a thin progress bar. A mission the Supervisor does not watch reads **Not watched**; a mission whose lead runs on another device reads **on** that device's name, **not watched**: the Supervisor can only reach threads on its own Mac.

Click a mission to see who works on it: the **Lead**, then each step and its title, in the order Mission Control numbers them. Each one shows its agent's mark and model, and the agent profile's name when its chat runs one. **Open Mission Control** opens the mission's page. **Watched by the Supervisor** at the bottom has a switch: turn it off and that mission stops waking the Supervisor; turn it on again to bring it back.

## Read a brief

Hover the lead or a step and click **Brief** to see what that agent was told, in a drawer that slides in from the right. Its header names the thread, the mission and the model, and the branch and commit the thread started on. Then:

- **Task**: the mission's title and notes.
- **Spec**: the plan's overview, and for a step, what the lead wrote for that step.
- **Messages it got**: every message the thread received, oldest first, each tagged by who sent it: **From you**, **From Lead**, **From Supervisor**, or the step that sent it.

Click the × to close the drawer.

## Authority and the wake log

Click the gear in the header to open **Authority** and the **Wake log**. Click outside it or press Esc to close it.

**Authority** sets what the Supervisor may do on its own:

- **Message a Lead or Peer directly**: send messages to leads and step threads, and answer their questions. Answering a tool approval also needs the Supervisor's chat at **Full** access, its default.
- **Re-brief a step, through its Lead**
- **Pause a step that blocks a shared resource**: on by default, and only usable when Zeron has verified a contention. Not shown on Windows.
- **Start a new mission**: off by default. While it is off, the Supervisor cannot start new threads either.

**Change a mission's goal or spend** has no switch: the Supervisor always asks you. **Check in on its own** wakes it every 30 or 60 minutes while a watched mission runs; it is **Off** by default. Your choices are saved on this Mac and stay after a restart.

The **Wake log** lists the recent wakes, newest first. Each shows its time, the events it carried, and how it reached the Supervisor: **Started a turn**, **Steered its turn**, **Held for turn end**, **Held by your Stop**, or **Failed** with the reason. Events that went stale before they were sent are listed as **Dropped**.

## Talk to the Supervisor

The composer at the bottom of the page has the same model picker as New chat. Type in **Talk to the Supervisor about architecture and direction…** and send.

Above it, **Conversation** shows the last message in one line: the last wake, the Supervisor's last reply, or what you last wrote. Click it to open the conversation above the composer; click it again to close it. In the conversation, a wake reads **Wake** and its number of events, so you can tell it from what you wrote.

The page has motion: the blocks rise in when you open it, cards fold in and out as you filter or answer them, and a new **Needs you** card slides in with a short amber glow. With **Reduce motion** on in [Settings > Appearance](zeron://settings/appearance), all of it snaps instead.
