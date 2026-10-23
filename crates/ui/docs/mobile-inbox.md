This page covers how the iPhone's **Sessions** tab groups and sorts your sessions, and the [Supervisor](zeron://docs/supervisor) page you open from **Tasks**. For the sessions list on your computer, see [Sessions in the sidebar](zeron://docs/sessions-sidebar).

## Organize the sessions list

The **…** button at the top of **Sessions** sets how the list is built.

- **Organize**: **By device** groups sessions under the computer that runs them, **By project** under their project, and **None** keeps one list.
- **Sort**: **Last updated** or **Created**, and **Active first** to keep running sessions on top.
- **New Section…** adds one of your own sections, **Collapse All** folds every group, project and device groups included, and **Archived** lists archived sessions.

## Your sections

**Pinned** sessions and your own sections show as folders at the top; tap one to open it. Touch and hold a section's header to **Move Up**, **Move Down**, **Rename Section…** or **Delete Section**. Deleting a section moves its sessions back to the main list.

To put a session in a section, swipe left on it and tap **Move**, or touch and hold it and choose **Move to Section**.

## Threads under their lead

Threads another thread started sit under it: a mission's step threads under the lead, chats an agent created under that chat, and side chats under the chat they came from. Tap the arrow beside the parent to fold them. A side chat that expired stays in the list, dimmed and marked **Expired**.

## Peek at a session

Touch and hold a row to see a preview of the session above its menu. The menu has **Pin** or **Unpin**, **Move to Section**, **Rename…** and **Archive**.

Each row carries its agent's mark, including OpenCode, Cursor, Devin, Grok, Hermes, Pi and Antigravity; an ACP agent shows a bot mark. A row whose turn is held by background work names it, such as "1 shell", in place of **Working**. An idle row with work still running shows a ring and the count beside its time.

## The Supervisor

Tap the **Supervisor** card at the top of **Tasks** to open the Supervisor page. Each computer runs its own Supervisor, so the page lists each one with its state: what it watches, how many of its wakes this hour it used, and its next check. A computer that is offline says **Offline**.

Further down, **Done today** lists the tasks finished today with their computer and time, and **Wake log** lists the Supervisor's recent wakes and what woke it.

![The Supervisor page, and its authority on one computer](ios-supervisor.jpg)

Tap a computer for its actions:

- **Stop** asks first, then stops its Supervisor; it then reads **Stopped by you**, and "Wakes wait until you resume it." **Resume** starts it again. **Start** starts one that is not running, and **Try again** retries one that failed to start.
- **Open conversation** opens the Supervisor's own chat with you.
- **Watch everything** has it watch every tree and mission on that computer; **Stop watching everything** turns that off. To watch one thread or mission, use **Watched by the Supervisor** in the thread's **…** menu or on the mission's page.
- **Authority…** opens what it may do on that computer.

## Authority

"Each Mac runs its own Supervisor; these switches apply to this one."

- **Full authority**: **Decide for me** lets it answer the lead's decisions and approve plans for you, and **Change a mission's goal or spend** lets it decide those too.
- **Also allowed**: **Message a Lead or Peer directly**, **Re-brief a step, through its Lead**, **Pause a step that blocks a shared resource**, and **Start a new mission**.
- **Check in on its own**: how often it checks in when nothing wakes it, or **Off**.

When the Supervisor approves a plan for you, the mission shows a countdown with **Take back** before the lead hears of it (see [Tasks and missions on iPhone](zeron://docs/mobile-tasks)).
