Zeron gives agents extra tools through its own MCP server, so they can work across your chats, use your terminal, ask you questions, and show results inline. You decide which tools are on in [Settings > Context & tools > Tools](zeron://settings/tools).

## What agents can do for you

The tools come in seven groups, the same ones you see in Settings > Context & tools > Tools:

| Group | What the agent can do |
| --- | --- |
| **Threads** | List, read, and rename your chats, start a new chat, message another chat, wait for or watch chats, stop a chat's turn, archive or settle chats, and answer another chat's pending question. |
| **Questions, terminal, actions and tasks** | Ask you questions and wait for the answers, use this chat's terminals in the terminal panel, manage the project's Actions menu, write and assign tasks on your Tasks list, change this chat's auto-compact window, compact its own context, and read these Docs and the release notes to answer how Zeron works. |
| **Delegation** | Run child agents on any installed provider and model, check on or cancel them, write a plan and run its steps, start a code review, report what it did about review findings, and manage agent profiles when you ask. |
| **Pages and visuals** | Show live HTML pages and native charts, tables, and mockups inline in the thread. |
| **Device hub** | Open iOS Simulators and Android Emulators in the Device panel, take screenshots of them, and close them. |
| **Computer** | Control the desktop and a browser on this Mac. |

## Every tool

Each tool has its own switch in Settings > Context & tools > Tools, under its group. The names are what agents call them. `set_thread` and `device` keep one switch per action, under the names they had before they became one tool, so you can still turn off archiving or closing a device alone. The counts on each group and at the bottom of the page are the tools an agent receives, with actions counted beside their tool: on this Mac, Computer is one tool.

| Group | Tool | What it does |
| --- | --- | --- |
| **Threads** | `list_threads`, `read_thread` | List the chats in the sidebar with their live status, and read a chat's transcript. |
| | `create_thread`, `send_message_to_thread` | Start a new chat with a first message, and message another chat on this device. |
| | `wait_threads`, `watch_threads` | Wait until chats stop working, or be woken when they ask a question or need an approval. |
| | `set_thread` (switches `set_thread_title`, `set_thread_archived`, `set_thread_settled`) | Rename, archive, or settle chats. |
| | `interrupt_thread`, `respond_to_input` | Stop another chat's turn, and answer the question it is waiting on. |
| **Questions, terminal, actions and tasks** | `ask_user` | Ask you one to four questions and wait for the answers. See [When an agent asks](zeron://docs/agent-questions). |
| | `request_secret` | Ask you for a secret in a private card. The agent gets a private file to read in its commands, never the value. See [When an agent asks](zeron://docs/agent-questions). |
| | `terminal` | Use this chat's terminals. See [Terminal](zeron://docs/terminal). |
| | `project_actions` | Manage the project's saved Actions. See [Projects and worktrees](zeron://docs/projects). |
| | `tasks` | Write, update, and assign tasks. See [Tasks](zeron://docs/tasks). |
| | `context_settings`, `compact` | Read the chat's context settings and set its auto-compact window, and compact its own context. See [Compaction and context](zeron://docs/compaction). |
| | `docs` | Read these Docs and the release notes to answer how Zeron works and the full reference for `show_visual`. |
| **Delegation** | `list_projects`, `orchestrator_capabilities` | Look up your projects, the agents installed on this device, and their models: a short list per agent (the models in use plus the first eight), with a count of the rest and a search for any of them. |
| | `delegate_task`, `task_status`, `task_cancel` | Start a child agent, read how it is doing, and stop it. See [Delegation and subagents](zeron://docs/delegation). |
| | `plan` | Write this chat's plan of steps, mark each step done or skipped, and read the plan. See [Mission](zeron://docs/mission). |
| | `review_changes`, `resolve_findings` | Start a code review, and report what it did about each finding. See [Code review](zeron://docs/review). |
| | `agent_profiles` | Create, change, or remove your agent profiles when you ask. See [Agent profiles](zeron://docs/agent-profiles). |
| **Pages and visuals** | `html_render`, `html_preview` | Show a finished HTML page in the chat, and check a page before showing it: a screenshot, its elements as text, a crop, and clicks or typing step by step. See [Browser](zeron://docs/browser). |
| | `show_visual` | Show a native, interactive chart, table, or 3D view in the chat. |
| | `show_image` | Show an image file (a screenshot, a rendered chart, a photo) in the chat. The file is copied in, so it stays after the original changes. |
| **Device hub** | `device` (switches `device_list`, `device_open`, `device_screenshot`, `device_close`) | List simulators and emulators, open one in the Device panel, capture its screen, and close it. See [Device hub](zeron://docs/device-hub). |
| **Computer** | `computer`, `computer_zeron_browser`, and the `computer_*` tools | See and drive the apps on your Mac (click, type, scroll, screenshots, windows, menus, clipboard) and the Zeron browser tabs (navigate, click, type, uploads, downloads, dialogs). See [Computer use](zeron://docs/computer-use). |

The threads tools are described from your side in [Agents and other threads](zeron://docs/threads).

## Built-in limits

- Agents reach Zeron with a limited key, not the app's own. They can use the tools on this page, but cannot allow plugins, sign you in or change what a chat may do.
- `terminal`, `computer` and saving project Actions work only in a chat at **Full access**. In a chat at **Supervised** or **Auto-accept edits** the call is refused and says so.
- If a tool you need is refused, **Agents use the full token** in [Settings > Lab](zeron://settings/experimental) gives new runs everything the app can reach, with no question to you. It is off by default. This key limits what the tools do; a determined shell command in a chat without an OS sandbox can still read the app's own key on disk.
- Project actions only touch the project of the chat that calls them.
- In a Codex chat, the `device` tool runs without asking for approval.
- An agent can answer another chat's permission request only when its own chat runs at **Full access**.
- A child agent, or a profile an agent applies, never gets a higher access mode than the chat that started it.
- A question from an agent waits up to 25 minutes for your answer.
- **Delegated tasks per chat** in Settings > Agents > Routing caps what one chat has running at once: its delegated tasks, the steps of its plan, and the chats it started that are still working.
- A [plan](zeron://docs/mission) belongs to the chat that wrote it: only that chat changes it and runs its steps, and while the plan waits for your approval, it cannot start a step.

## Turn tools off

Open [Settings > Context & tools > Tools](zeron://settings/tools). Each group has a switch, and each tool inside it has its own. Tools are on by default, except the Computer and Device hub groups described below. A group you turn off keeps your per-tool choices for when you turn it back on.

![Settings > Context & tools > Tools](agent-tools-settings.jpg)

The picture at the top of the page lights a wire for each group that is on, and dims it while only some of the group's tools are on. **Turn all off** turns every group off; **Turn all on** turns every group on, Computer included, and turns every single tool back on. It turns on Device hub only when the Device hub itself is on in [Settings > Devices & updates > Device hub](zeron://settings/device-hub).

Some tools depend on another. When you turn one off, the tools that need it are off too and say so, for example **Off while delegate_task is off**.

The **Computer** group is the same switch as [Settings > This Mac > Computer use](zeron://settings/computer), and the **Device hub** group is the same switch as agent access in [Settings > Devices & updates > Device hub](zeron://settings/device-hub). Both start off. While the Device hub itself is off, the group's switch is greyed out and its link reads **Turn on Device hub first**.

A chat that is already running keeps its tool list until its next turn, but a call to a tool you turned off is refused right away.

> To turn tools off for only some chats, set the **Tools** field of an [agent profile](zeron://docs/agent-profiles).
