The Supervisor is one agent for this Mac's workspace that sits above the leads of your Mission Control missions. You talk with it about architecture and direction, and it watches the missions you leave on. It does not run all the time: Zeron wakes it when something happens in a mission, and it goes back to sleep when it has nothing to do.

## Open the Supervisor

Click **Supervisor**, the shield in the rail under **Tasks**. The first time you open the page, Zeron starts the Supervisor's chat. It uses your **SLP Supervisor** agent profile when you have one, and otherwise the agent and model you last used on New chat. Its permission starts at **Full**.

The Supervisor's chat never shows in the sidebar or under **Archived**, and it never plays the run-finished sound. You reach it only from this page.

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

**Interventions** lists what the Supervisor did through its tools, newest first: a message to a lead or a step, an answer to a thread's question, a re-brief, or a mission it started. Each row shows the time, the kind of action, the thread it went to, the Supervisor's own words, and what led to it: **after the 14:05 wake** or **after your message**.

Under it, chips show only what has happened since:

- **Sent**: the Supervisor's message went to the thread.
- **Lead ran a turn**: the lead answered after it.
- **Plan edited on steps 2, 3**: those steps' titles or specs changed since.
- **Step 2 thread completed a turn**: that step's thread finished a turn since.

**No effect yet** appears when **Sent** is still the only chip ten minutes later. The chips never judge whether the action helped; the Supervisor's own words are its judgment.

## Read a brief

Click **Brief** on the lead or a step to see what that agent was told, in a drawer on the right. Its header names the thread, the mission and the model, and the branch and commit the thread started on. Then:

- **Task**: the mission's title and notes.
- **Spec**: the plan's overview, and for a step, what the lead wrote for that step.
- **Messages it got**: every message the thread received, oldest first, each tagged by who sent it: **From you**, **From Lead**, **From Supervisor**, or the step that sent it.

Click the × to close the drawer.

## Authority

**Authority** sets what the Supervisor may do on its own:

- **Message a Lead or Peer directly**
- **Re-brief a step, through its Lead**
- **Start a new mission**

**Change a mission's goal or spend** has no switch: the Supervisor always asks you. **Check in on its own** wakes it every 30 or 60 minutes while a watched mission runs; it is **Off** by default. Your choices are saved on this Mac and stay after a restart.

## Wake log

**Wake log** at the end of the page, closed by default, lists the recent wakes, newest first. Each shows its time, how many events it carried, and how it reached the Supervisor: **Started a turn**, **Steered its turn**, **Held for turn end**, **Held while stopped**, or **Failed** with the reason. Under it are the events themselves. Events that went stale before they were sent are listed as **Dropped**.

## Talk to the Supervisor

The Supervisor's conversation is on the right, with a composer under it that has the same model picker as New chat. Type in **Talk to the Supervisor about architecture and direction…** and send. In the conversation, a wake reads **Wake** and its number of events, so you can tell it from what you wrote. **Hide conversation** in the header closes the conversation and keeps the composer at the bottom of the page; **Show conversation** brings it back.
