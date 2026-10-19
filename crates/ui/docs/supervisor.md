The Supervisor is one agent for this Mac's workspace that sits above the leads of your Mission Control missions. You talk with it about architecture and direction, and it watches the missions you leave on. It does not run all the time: Zeron wakes it when something happens in a mission, and it goes back to sleep when it has nothing to do.

## Open the Supervisor

Click **Supervisor**, the shield in the rail under **Tasks**. The first time you open the page, Zeron starts the Supervisor's chat. It uses your **SLP Supervisor** agent profile when you have one, and otherwise the agent and model you last used on New chat. Its permission starts at **Full**.

The Supervisor's chat never shows in the sidebar or under **Archived**, and it never plays the run-finished sound. You reach it only from this page.

## What wakes it and what it can do

Zeron wakes the Supervisor when something happens in a watched mission: a step fails, a step or the lead waits on you, the mission finishes, the mission goes quiet for 30 minutes, the lead stays quiet for 10 minutes after a step failed, the plan is edited after the mission started, the lead escalates, or two missions contend for a shared resource. Events that arrive within a minute of each other come as one wake, and it wakes at most 12 times an hour.

It acts through the missions' leads, within the switches under **Authority**:

- It messages a lead or a step's thread, and answers their questions and tool approvals.
- It re-briefs a step: it rewrites that step's title and spec, and the lead gets one message saying what changed and why. It never changes the plan's overview and never adds or removes a step.
- It pauses a step that blocks a shared resource, and resumes it later. A pause needs a contention Zeron verified on this Mac (see **Shared resources**): it stops the step's thread, the lead cannot run that step again until it is resumed, and the lead gets one message each time.
- It starts a new mission. The new lead writes a plan that you review before any step runs.
- It asks you. A change to a mission's goal or spend, or a decision it should not make alone, becomes a card under **Needs you**.

A mission's lead can bring a decision to you too. When a decision changes the goal, costs money you have not approved, or cannot be settled from the plan, the lead escalates it as a card under **Needs you** and keeps working on what does not depend on it. A lead escalates even when its mission is not watched; only the Supervisor's wake is skipped.

Every action the Supervisor takes is listed under **Interventions**.

## The header

Under the title, one line says what the Supervisor is for. The line below it reads **Watching** and the number of missions it watches, then the Supervisor's model, then its state:

- **Asleep**, with the time it last woke and why, for example **woke 14:06 for step failed**.
- **Awake** while it works on a wake.
- **Stopped** after you stop its turn, with the number of wakes waiting, for example **Stopped · 2 wakes waiting**. Wakes queue up while it is stopped; send it a message to resume, and the waiting wakes follow.
- **Off** before its chat exists.

The dot pulses only while the Supervisor is working. The number on the rail's **Supervisor** item counts the cards under **Needs you**.

## Needs you

**Needs you**, the first section under the header, holds what waits on your decision. Each card leaves the page once it is answered.

- **A decision** raised by a mission's lead or by the Supervisor: who raised it, the mission, why it comes to you (**Goal**, **Spend** or **Decision**), the details, and one button per option. Click an option to answer: the card reads **Answered** and leaves, and the mission's lead gets one message with your choice. When the Supervisor raised it, the Supervisor gets your choice too.
- **A question or an approval** a thread of a watched mission is waiting on: the lead's own questions, any step's tool approvals, and the Supervisor's own questions. The card names the thread that asks and its mission, then the questions and their options. Click an option to answer, the same way you answer in the chat; with several questions, pick one option for each and click **Send answers**. **Open thread** takes you to the thread, where you can type an answer instead.

**Discuss** on either kind puts the card into the Supervisor's composer as a quote and moves the cursor there. Nothing is sent until you send it.

### When Zeron tells you

When a new card appears, Zeron checks whether you are there:

- **You are looking at it**: Zeron is the active app and you are on this page, on Mission Control for that mission, or in the chat that raised it. Only the card appears.
- **You used Zeron in the last 3 minutes**: a notice in the window reads **Supervisor needs you** and the card's title.
- **You have been away longer**: a desktop banner titled **Supervisor needs you**. Click it to open this page. It follows **Desktop notifications** in [Settings > Notifications](zeron://settings/notifications), and it shows even when **Only when in the background** is on.

## Missions

**Missions** lists every Mission Control task that is not done, with the same ring, status and progress as the Missions widget on Home. Each row has a **Watched** switch. Turn it off and that mission stops waking the Supervisor; turn it on again to bring it back. A mission whose lead runs on another device reads **on** that device's name, **not watched**: the Supervisor can only reach threads on its own Mac.

Click a mission to see who works on it: the **Lead**, then each step as **Step 2 of 5** and its title, in the order Mission Control numbers them. Each one shows its agent's mark and model, and the agent profile's name when its chat runs one. **Open Mission Control** opens the mission's page.

## Open questions to the Lead

**Open questions to the Lead** lists the questions the steps of your watched missions asked, newest first. A step's lead hears about each of its questions, and so do you. Each row shows the question, the step as Mission Control numbers it (**Step 2 of 4** and its title), the mission, and where it stands:

- **Open**: no one has answered yet.
- **Settled**, with the answer: **by the Lead**, **by the Supervisor**, or **answered outside the Lead** when you or another thread answered it.
- **Dropped**: the step's turn ended before anyone answered.

Click a row to open the step's thread. Tool approvals are not listed here; they wait under **Needs you**.

## Interventions

**Interventions** lists what the Supervisor did through its tools, newest first: a message to a lead or a step, an answer to a thread's question, a re-brief, a step it paused or resumed, or a mission it started. Each row shows the time, the kind of action, the thread it went to, the Supervisor's own words, and what led to it: **after the 14:05 wake** or **after your message**.

Under it, chips show only what has happened since:

- **Sent**: the Supervisor's message went to the thread.
- **Lead ran a turn**: the lead answered after it.
- **Plan edited on steps 2, 3**: those steps' titles or specs changed since.
- **Step 2 thread completed a turn**: that step's thread finished a turn since.
- **Step paused**: the step is still paused and its thread is not running.
- **Resource freed**: a check after the pause found the process gone or the port closed.

**No effect yet** appears when **Sent** is still the only chip ten minutes later. The chips never judge whether the action helped; the Supervisor's own words are its judgment.

## Shared resources

A mission's lead can declare, in its plan, what the mission needs to itself while it runs: a port its servers listen on (**port:5173**), a quiet CPU for timing runs (**slot:benchmark**), or a folder it rewrites (**path:src/checkout**). A mission holds what it declared while any of its threads works or has written in the last 30 minutes.

**Shared resources** lists each declared resource with the missions that declared it, the ones holding it now, and its state:

- **Free** or **Held**: nobody else wants it, so Zeron does not check it.
- **Checking…**, then **Clear** or **Contended**, when two running missions share it. Zeron checks the Mac itself, and only then: for a port, which process listens on it (**Verified by lsof** and the time); for a CPU slot, how much CPU each other mission's processes used in two samples ten seconds apart (**Verified by ps**). Another mission using more than half a core in both samples makes the slot contended. When the missions share one folder, Zeron cannot tell them apart and names **a process in** that folder.
- **Declared by both, not verified**: two missions in the same checkout declared overlapping folders. Zeron does not check files and does not wake the Supervisor for it.
- **Not verified**: the check could not run.

A contention wakes the Supervisor once, and again only after it has cleared. Zeron checks again when you open this page (at most once a minute), half a minute and two minutes after a pause, and when a thread of the missions sharing it starts or stops work (at most every two minutes). This section is not shown on Windows.

## Read a brief

Click **Brief** on the lead or a step to see what that agent was told, in a drawer on the right. Its header names the thread, the mission and the model, and the branch and commit the thread started on. Then:

- **Task**: the mission's title and notes.
- **Spec**: the plan's overview, and for a step, what the lead wrote for that step.
- **Messages it got**: every message the thread received, oldest first, each tagged by who sent it: **From you**, **From Lead**, **From Supervisor**, or the step that sent it.

Click the × to close the drawer.

## Authority

**Authority** sets what the Supervisor may do on its own:

- **Message a Lead or Peer directly**: send messages to leads and step threads, and answer their questions. Answering a tool approval also needs the Supervisor's chat at **Full** access, its default.
- **Re-brief a step, through its Lead**
- **Pause a step that blocks a shared resource**: on by default, and only usable when Zeron has verified a contention. Not shown on Windows.
- **Start a new mission**: off by default. While it is off, the Supervisor cannot start new threads either.

**Change a mission's goal or spend** has no switch: the Supervisor always asks you. **Check in on its own** wakes it every 30 or 60 minutes while a watched mission runs; it is **Off** by default. Your choices are saved on this Mac and stay after a restart.

## Wake log

**Wake log** at the end of the page, closed by default, lists the recent wakes, newest first. Each shows its time, how many events it carried, and how it reached the Supervisor: **Started a turn**, **Steered its turn**, **Held for turn end**, **Held by your Stop**, or **Failed** with the reason. Under it are the events themselves. Events that went stale before they were sent are listed as **Dropped**.

## Talk to the Supervisor

The Supervisor's conversation is on the right, with a composer under it that has the same model picker as New chat. Type in **Talk to the Supervisor about architecture and direction…** and send. In the conversation, a wake reads **Wake** and its number of events, so you can tell it from what you wrote. **Hide conversation** in the header closes the conversation and keeps the composer at the bottom of the page; **Show conversation** brings it back.
