The Supervisor is one agent for this Mac's workspace that sits above the leads of your Mission Control missions. You talk with it about architecture and direction, and it watches the missions you leave on. It does not run all the time: Zeron wakes it when something happens in a mission, and it goes back to sleep when it has nothing to do.

## Open the Supervisor

Click **Supervisor**, the shield in the rail under **Tasks**. The first time you open the page, Zeron starts the Supervisor's chat. It uses your **SLP Supervisor** agent profile when you have one, and otherwise the agent and model you last used on New chat. Its permission starts at **Full**.

The Supervisor's chat never shows in the sidebar or under **Archived**, and it never plays the run-finished sound. You reach it only from this page.

## The header

Under the title, one line says what the Supervisor is for. The line below it reads **Watching** and the number of missions it watches, then the Supervisor's model, then its state:

- **Asleep**, with the time it last woke and why, for example **woke 14:06 for step failed**.
- **Awake** while it works on a wake.
- **Off** before its chat exists.

The dot pulses only while the Supervisor is working.

## Missions

**Missions** lists every Mission Control task that is not done, with the same ring, status and progress as the Missions widget on Home. Each row has a **Watched** switch. Turn it off and that mission stops waking the Supervisor; turn it on again to bring it back. A mission whose lead runs on another device reads **on** that device's name, **not watched**: the Supervisor can only reach threads on its own Mac.

Click a mission to see who works on it: the **Lead**, then each step as **Step 2 of 5** and its title, in the order Mission Control numbers them. Each one shows its agent's mark and model, and the agent profile's name when its chat runs one. **Open Mission Control** opens the mission's page.

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

**Wake log** at the end of the page, closed by default, lists the recent wakes, newest first. Each shows its time, how many events it carried, and how it reached the Supervisor: **Started a turn**, **Steered its turn**, **Held for turn end**, or **Failed** with the reason. Under it are the events themselves. Events that went stale before they were sent are listed as **Dropped**.

## Talk to the Supervisor

The Supervisor's conversation is on the right, with a composer under it that has the same model picker as New chat. Type in **Talk to the Supervisor about architecture and direction…** and send. In the conversation, a wake reads **Wake** and its number of events, so you can tell it from what you wrote. **Hide conversation** in the header closes the conversation and keeps the composer at the bottom of the page; **Show conversation** brings it back.
