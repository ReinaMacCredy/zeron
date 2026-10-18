Three characters open menus in the composer: `/` for commands, `$` for skills, and `@` to mention files and other threads. Use Up and Down to move through a menu, Enter or Tab to pick a row, and Escape to close it.

## `/` commands

Type `/` to list the agent's built-in commands. They come from the agent itself, so the list differs between Claude, Codex, and the others. Zeron adds a few of its own:

| Command | What it does |
| --- | --- |
| `/compact` | Free up context by summarizing the conversation. |
| `/btw` or `/sidechat` | Ask about this chat on the side, without touching it. |
| `/computer-use` | Use Zeron Computer for this request only (supported agents). |
| `/model` | Choose agent, model, and reasoning. |
| `/new` | Start a new conversation. |
| `/resume` | Search and open conversations. |
| `/settings` | Open settings. |

Inside a chat you also get `/diff` (open changes), `/files` (open project files), `/terminal` (open a terminal), `/rename` (rename this conversation), and `/stop` (stop the active run). These run in Zeron and are never sent to the agent. If the agent has a command with the same name, Zeron's shows as `/zeron:<name>`.

## `$` skills

Type `$` to pick one of the skills the agent finds for this project, such as the skill folders in `.claude/skills` or `.agents/skills`. Each row shows the skill's name and description. Zeron passes your pick to the agent in the form that agent expects.

Two switches per agent control this, under **Completion** when you expand an agent in [Settings > Providers](zeron://settings/providers):

- **Use $ for skills**: type `$` in the composer to pick a skill.
- **Separate / commands**: keep skills out of the `/` menu.

Both are on by default. Turn off **Use $ for skills** and `$` is plain text again.

## `@` mentions

Type `@` to open one menu with two sections:

- **Threads** lists other threads on the same device, with their status (**Working**, **Input**, **Failed**, **Done**) and project. Picking one inserts a link the agent can open with Zeron's `read_thread` tool.
- **Files** lists files and folders in the project, plus anything you have attached. Picking one inserts a link to its path.

You can also drag a file from the file tree, or a session from the sidebar (**Drop to mention**), onto the conversation to insert the same mention.

> `/btw` runs a side question without disturbing the thread. Cmd+; toggles the sidechat too.
