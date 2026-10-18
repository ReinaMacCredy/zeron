A project is a folder on one of your devices that agents work in. Chats in a project run in that folder, and each project has its own worktrees and Actions.

## Add a project

Click the folder button at the top of the sidebar (tooltip **New project**), press Cmd+Shift+N, or pick **New project** in the Cmd+K palette.

Pick a device, a location (**Home** or a drive), then folders. Type to filter, press Enter to open a folder, and Left to go back up. With the folder you want open, click **Add project** or press Cmd+Enter.

## Rename or remove

Click the project filter button at the top of the sidebar (tooltip **Filter by project**) and right-click a project to get **Rename…** and **Remove…**. Removing a project permanently deletes its sessions on that device.

## Project icons

Zeron picks the icon from a favicon or app icon in the repository, such as `public/favicon.svg` or `favicon.ico`. With none, it shows the project's first letter on a colored tile. To hide icons, click the **...** button above the sidebar (tooltip **Sidebar view options**) and turn off **Project icon**.

## Worktrees

In a git project, the checkout chip on the New chat composer offers **Current checkout** and **New worktree**. A new worktree gets its own folder and a new `zeron/` branch, so the agent's edits stay away from your checkout.

Worktrees go in `~/.zeron/worktrees/<repository>/` by default. To change that, open [Settings > General](zeron://settings/general), scroll to **Worktrees**, choose a **Device**, and turn on **Custom worktree location**. Each device keeps its own location, and existing worktrees stay where they are.

## Actions

Actions are saved commands for a project, such as a dev server, a build, or tests. They show in the **Actions** section of the **Live state** column beside a chat, and in the Cmd+K palette.

- Click **Add action**, then fill in **Name**, **Command**, and **Icon**, and click **Save action**.
- Click an action to run it. It opens in a new tab of the chat's terminal, in the chat's folder, and shows **Running**, **Done**, or **Failed**.
- Hover an action and click the gear to edit or delete it.

Check **Run automatically on worktree creation** to make an action the project's setup action. Only one action per project can have it. It runs in each new worktree that Zeron creates for a chat.

> Zeron also reads a `zeron.json` file at the project root and offers its actions with an **Import** tag.
