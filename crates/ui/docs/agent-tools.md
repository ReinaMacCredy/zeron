Zeron gives agents extra tools through its own MCP server, so they can work across your chats, use your terminal, ask you questions, and show results inline. You decide which tools are on in [Settings > Tools](zeron://settings/tools).

## What agents can do for you

The tools come in six groups, the same ones you see in Settings > Tools:

| Group | What the agent can do |
| --- | --- |
| **Threads** | List, read, and rename your chats, start a new chat, message another chat, wait for or watch chats, stop a chat's turn, archive or settle chats, and answer another chat's pending question. |
| **Questions, terminal, actions and tasks** | Ask you questions and wait for the answers, use this chat's terminals in the terminal panel, manage the project's Actions menu, write and assign tasks on your Tasks list, plan a Mission Control task's steps, read the Zeron app's own UI, drive a Zeron Dev build's UI with a separate tool, change this chat's auto-compact window, compact its own context, and read these Docs and the release notes to answer how Zeron works. |
| **Delegation** | Run child agents on any installed provider and model, check on or cancel them, start a code review, report what it did about review findings, and manage agent profiles when you ask. |
| **Pages and visuals** | Show live HTML pages and native charts, tables, and mockups inline in the thread. |
| **Device hub** | Open iOS Simulators and Android Emulators in the Device panel, take screenshots of them, and close them. |
| **Computer** | Control the desktop and a browser on this Mac. |

## Every tool

Each tool has its own switch in Settings > Tools, under its group. The names are what agents call them.

| Group | Tool | What it does |
| --- | --- | --- |
| **Threads** | `list_threads`, `read_thread` | List the chats in the sidebar with their live status, and read a chat's transcript. |
| | `create_thread`, `send_message_to_thread` | Start a new chat with a first message, and message another chat on this device. |
| | `wait_threads`, `watch_threads` | Wait until chats stop working, or be woken when they ask a question or need an approval. |
| | `set_thread_title`, `set_thread_archived`, `set_thread_settled` | Rename, archive, or settle chats. |
| | `interrupt_thread`, `respond_to_input` | Stop another chat's turn, and answer the question it is waiting on. |
| **Questions, terminal, actions and tasks** | `ask_user` | Ask you one to four questions and wait for the answers. See [When an agent asks](zeron://docs/agent-questions). |
| | `terminal` | Use this chat's terminals. See [Terminal](zeron://docs/terminal). |
| | `project_actions` | Manage the project's saved Actions. See [Projects and worktrees](zeron://docs/projects). |
| | `ui_driver`, `ui_driver_drive` | Read a running Zeron app's UI, and drive a Zeron Dev build's UI. |
| | `tasks` | Write, update, and assign tasks, and plan a mission's steps. See [Tasks](zeron://docs/tasks). |
| | `context_settings`, `compact` | Read the chat's context settings and set its auto-compact window, and compact its own context. See [Compaction and context](zeron://docs/compaction). |
| | `docs` | Read these Docs and the release notes to answer how Zeron works. |
| **Delegation** | `list_projects`, `list_harnesses`, `list_models`, `orchestrator_capabilities` | Look up your projects, the agents installed on this device, and the models each one offers. |
| | `delegate_task`, `task_status`, `task_cancel` | Start a child agent, read how it is doing, and stop it. See [Delegation and subagents](zeron://docs/delegation). |
| | `review_changes`, `resolve_findings` | Start a code review, and report what it did about each finding. See [Code review](zeron://docs/review). |
| | `agent_profiles` | Create, change, or remove your agent profiles when you ask. See [Agent profiles](zeron://docs/agent-profiles). |
| **Pages and visuals** | `html_render`, `html_preview` | Show a finished HTML page in the chat, and check how a page looks before showing it. See [Browser](zeron://docs/browser). |
| | `show_visual` | Show a native, interactive chart, table, or 3D view in the chat. |
| **Device hub** | `device_list`, `device_open`, `device_screenshot`, `device_close` | List simulators and emulators, open one in the Device panel, capture its screen, and close it. See [Device hub](zeron://docs/device-hub). |
| **Computer** | `computer`, `computer_zeron_browser`, and the `computer_*` tools | See and drive the apps on your Mac (click, type, scroll, screenshots, windows, menus, clipboard) and the Zeron browser tabs (navigate, click, type, uploads, downloads, dialogs). See [Computer use](zeron://docs/computer-use). |

The threads tools are described from your side in [Agents and other threads](zeron://docs/threads).

## Built-in limits

- Project actions only touch the project of the chat that calls them.
- The UI driving tool only drives Zeron Dev builds, and never the app that serves the agent's own chat. Reading the UI works in every build and includes text typed into fields. Turning the reading tool off turns the driving tool off too.
- In a Codex chat, closing a device runs without asking for approval. Driving the UI runs only when the chat is at **Full access**; at Ask or Auto, Codex's approval request for it is declined.
- An agent's actions toward other chats (messages, new chats, interrupts, answers) are capped at 16 per turn.
- An agent can answer another chat's permission request only when its own chat runs at **Full access**.
- A child agent, or a profile an agent applies, never gets a higher permission mode than the chat that started it.
- A question from an agent waits up to 25 minutes for your answer.
- An agent can keep up to 8 chats it started open at once. The chats a task's lead starts for its steps do not count; **Delegated tasks per chat** in Settings > General caps how many steps run at once instead.
- Only a [Mission Control task](zeron://docs/mission-control)'s lead writes its plan and runs its steps, and while the plan waits for your approval, the lead cannot start a step.

## Turn tools off

Open [Settings > Tools](zeron://settings/tools). Each group has a switch, and each tool inside it has its own. Tools are on by default, except the Computer and Device hub groups described below. A group you turn off keeps your per-tool choices for when you turn it back on.

![Settings > Tools](agent-tools-settings.jpg)

The picture at the top of the page lights a wire for each group that is on, and dims it while only some of the group's tools are on. **Turn all off** turns every group off; **Turn all on** turns every group on, Computer included, and turns every single tool back on. It turns on Device hub only when the Device hub itself is on in [Settings > Device hub](zeron://settings/device-hub).

Some tools depend on another. When you turn one off, the tools that need it are off too and say so, for example **Off while delegate_task is off**.

The **Computer** group is the same switch as [Settings > Computer](zeron://settings/computer), and the **Device hub** group is the same switch as agent access in [Settings > Device hub](zeron://settings/device-hub). Both start off. While the Device hub itself is off, the group's switch is greyed out and reads **Turn on Device hub first**.

A chat that is already running keeps its tool list until its next turn, but a call to a tool you turned off is refused right away.

> To turn tools off for only some chats, set the **Tools** field of an [agent profile](zeron://docs/agent-profiles).
