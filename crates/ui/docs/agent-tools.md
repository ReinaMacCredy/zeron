Zeron gives agents extra tools through its own MCP server, so they can work across your chats, use your terminal, ask you questions, and show results inline. You decide which tools are on in [Settings > Tools](zeron://settings/tools).

## What agents can do for you

The tools come in six groups, the same ones you see in Settings > Tools:

| Group | What the agent can do |
| --- | --- |
| **Threads** | List, read, and rename your chats, start a new chat, message another chat, wait for or watch chats, stop a chat's turn, archive or settle chats, and answer another chat's pending question. |
| **Questions, terminal, actions and tasks** | Ask you questions and wait for the answers, use this chat's terminals in the terminal panel, manage the project's Actions menu, write and assign tasks on your Tasks list, plan a Mission Control task's steps, read the Zeron app's own UI, drive a Zeron Dev build's UI with a separate tool, change this chat's auto-compact window, and compact its own context. |
| **Delegation** | Run child agents on any installed provider and model, check on or cancel them, start a code review, report what it did about review findings, and manage agent profiles when you ask. |
| **Pages and visuals** | Show live HTML pages and native charts, tables, and mockups inline in the thread. |
| **Device hub** | Open iOS Simulators and Android Emulators in the Device panel, take screenshots of them, and close them. |
| **Computer** | Control the desktop and a browser on this Mac. |

## Built-in limits

- Project actions only touch the project of the chat that calls them.
- The UI driving tool only drives Zeron Dev builds, and never the app that serves the agent's own chat. Reading the UI works in every build and includes text typed into fields. Turning the reading tool off turns the driving tool off too.
- In a Codex chat, driving the UI and closing a device run without asking for approval, whatever the chat's permission mode.
- An agent's actions toward other chats (messages, new chats, interrupts, answers) are capped at 16 per turn.
- An agent can answer another chat's permission request only when its own chat runs at **Full access**.
- A child agent, or a profile an agent applies, never gets a higher permission mode than the chat that started it.
- A question from an agent waits up to 25 minutes for your answer.
- An agent can keep up to 8 chats it started open at once. The chats a task's lead starts for its steps do not count; **Delegated tasks per chat** in Settings > General caps how many steps run at once instead.
- Only a [Mission Control task](zeron://docs/tasks)'s lead writes its plan and runs its steps, and while the plan waits for your approval, the lead cannot start a step.

## Turn tools off

Open [Settings > Tools](zeron://settings/tools). Each group has a switch, and each tool inside it has its own. Tools are on by default, except the Computer and Device hub groups described below. A group you turn off keeps your per-tool choices for when you turn it back on.

Some tools depend on another. When you turn one off, the tools that need it are off too and say so, for example **Off while delegate_task is off**.

The **Computer** group is the same switch as [Settings > Computer](zeron://settings/computer), and the **Device hub** group is the same switch as agent access in [Settings > Device hub](zeron://settings/device-hub). Both start off.

A chat that is already running keeps its tool list until its next turn, but a call to a tool you turned off is refused right away.

> To turn tools off for only some chats, set the **Tools** field of an [agent profile](zeron://docs/agent-profiles).
