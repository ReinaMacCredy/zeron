Three characters open menus in the composer: `/` for commands, `$` for skills, and `@` to mention files and other threads. Use Up and Down to move through a menu, Enter or Tab to pick a row, and Escape to close it.

## `/` commands

Type `/` to list the agent's built-in commands. They come from the agent itself, so the list differs between Claude, Codex, and the others. Zeron adds a few of its own:

| Command | What it does |
| --- | --- |
| `/btw` | Side chat: ask about this chat without touching it. It opens in the right pane. `/sidechat` does the same when typed, but is not listed. |
| `/compact` | Free up context by summarizing the conversation. |
| `/computer-use` | Use Zeron Computer for this request only. Offered for Claude Code, Codex, Devin, Grok, and Hermes. |
| `/model` | Choose agent, model, and reasoning. |
| `/new` | Start a new conversation. |
| `/resume` | Search and open conversations (the Cmd+K palette). |
| `/settings` | Open settings, on the page you used last. |

![The / command menu](commands-slash-menu.jpg)

Inside a chat you also get `/diff` (open changes), `/files` (open project files), `/terminal`, `/rename` (rename this conversation), and `/stop` (stop the active run). `/diff` opens the **Changes** tab and `/files` the **Files** tab in the [right pane](zeron://docs/right-pane). `/terminal` opens the terminal dock under the chat, or focuses it when it is already open; see [Terminal](zeron://docs/terminal).

`/model`, `/new`, `/resume`, `/settings`, and the five chat commands run in Zeron and are never sent to the agent; their menu descriptions start with **Zeron:**. If the agent has a command with one of these names, Zeron's shows as `/zeron:<name>`. `/compact`, `/btw`, and `/sidechat` work differently: when the agent lists its own command with that name, the menu shows only the agent's row, but Zeron still handles what you type. Zeron's `/computer-use` replaces any `/computer-use` the agent has.

Type a request after `/computer-use`. With nothing after it, the message is not sent and the composer shows **Add a request after /computer-use.**

## Claude Code's commands

In a Claude Code chat, some of Claude Code's own commands answer with a card instead of starting a turn. They run on the chat's computer, use no tokens, and work while the agent is still replying, without steering, queueing or stopping the run.

| Command | The card |
| --- | --- |
| `/context` | What fills the context: a bar and a legend by kind (system prompt, MCP tools, memory files, skills, messages and the rest), the free space, and the largest tools, memory files, agents and skills in lists you can open. **Compact now** compacts the chat. |
| `/usage` or `/cost` | No card: opens the [Usage](zeron://docs/usage) page. On the New chat page it opens the page without starting a chat. |
| `/model` | The models to pick from. Picking one changes this chat's model chip. |
| `/mcp` | The folder's MCP servers, connected or not. |

![The /context card](claude-context-card.jpg)

While a command runs, a row reads **Running in Claude Code…** with the seconds so far, and the card replaces it. If it fails, the card shows the error with **Retry**. **Copy** on a card copies what it shows.

Commands that only run in Claude Code's terminal, such as `/status` or `/theme`, answer with a note saying so, and some offer a button to the same thing in Zeron: `/hooks` opens `settings.json` and `/export` saves the chat as Markdown. In the `/` menu these commands sit at the bottom, dimmed, with a **Terminal** tag.

### Claude Code settings

Type `/config` in a Claude Code chat to open Claude Code's settings over the chat, with **Status**, **Config** and **Stats** tabs. Plan usage lives on the rail ring and the Usage page. On the New chat page the panel opens above the composer and pushes it down.

![Claude Code's settings over a chat](claude-config-panel.jpg)

- **Config** lists each setting with its key and value. Search with **Search settings…** (press `/`), or narrow the list with **All**, **Affects Zeron chats** and **Terminal only**. Click a row, or press Enter or Space, to change it: a switch flips, a choice cycles or opens a list, and text opens a field with **Save**. **Model**, **Effort** and **Permission mode** are tagged **= chip** and follow this chat's chips. Settings Claude Code only changes from its terminal are dimmed.
- **Status** has Claude Code's version, the session, folder, model, permission mode, output style, MCP servers, memory and plugins.
- **Usage** has your plan's limits as Claude Code reports them, with **Open Usage page** to open [Usage](zeron://docs/usage).
- **Stats** has your Claude Code activity over the last 36 weeks, your favorite model, sessions, active days, current streak and tokens.

Changes save one at a time through Claude Code itself; the footer reads **Saving…**, then **1 change, saved**. Close the panel with Escape or ×. When you changed something, a row stays in the chat, such as "Claude Code settings · Auto-compact → false", with **Reopen**.

## `$` skills

Type `$` to pick one of the skills the agent finds for this project, such as the skill folders in `.claude/skills` or `.agents/skills`. Each row shows the skill's name and description, with the name in Title Case (`my-skill` reads **My Skill**). Zeron passes your pick to the agent in the form that agent expects.

Two switches per agent control this, under **Completion** when you expand an agent in [Settings > Providers](zeron://settings/providers):

- **Use $ for skills**: type `$` in the composer to pick a skill.
- **Separate / commands**: keep skills out of the `/` menu.

Both are on by default. Turn off **Use $ for skills** and `$` is plain text again.

When Claude Code loads a skill, its row in the transcript names the skill in violet. Click the name to open the skill's `SKILL.md` in the right pane.

## `@` mentions

Type `@` to open one menu with three sections:

- **Threads** lists up to six other threads on the same device, with their status (**Working**, **Input**, **Failed**, **Done**) and project. Threads in this chat's project come first. Type to match a title. Picking one inserts a link the agent can open with Zeron's `read_thread` tool (see [Agents and other threads](zeron://docs/threads)).
- **Tasks** lists up to five open tasks from the [Tasks](zeron://docs/tasks) page, this chat's project first, with their status. Type to match a title, finished tasks included. Picking one inserts the task's title in the accent color with its status circle; click it to open the task's card. The agent gets a link with the task's id and reads the whole task with Zeron's `tasks` tool.
- **Files** lists files and folders in the project, plus anything you have attached. Picking one inserts a link to its path.

Type `@task ` (with the space) to search only tasks. A task already in your message reads **added**, so you can pick several in a row.

![The @ menu listing threads and files](commands-mention-menu.jpg)

You can also drag a file from the file tree, or a session from the sidebar (**Drop to mention**), onto the conversation to insert the same mention.

> `/btw` runs a side question without disturbing the thread. Cmd+; (**Side chat**) and the **Side chat** tab on the composer reopen this chat's latest side chat, or start a new one. See [Side questions with /btw](zeron://docs/side-chat).
