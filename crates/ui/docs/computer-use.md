Computer use lets an agent see and drive the apps on your Mac: it takes screenshots, clicks, and types. It needs macOS, and nothing is driven until you authorize a Computer task.

## Ask for a task

Type `/computer-use` followed by your task, for example:

```
/computer-use open Calculator and calculate 123 × 45
```

This turns Computer on for that one request. To give agents Computer tools on every turn, switch on **Computer control** in [Settings > Computer](zeron://settings/computer).

## Permissions

Zeron needs three macOS permissions, listed under **Advanced** on the Computer page:

- **Accessibility** to move the pointer, click, and type.
- **Screen Recording** to see the windows it drives.
- **Input Monitoring** to notice Escape and pause when you take over.

The status line under **Computer control** says whether the desktop is ready. When something is missing, click **Set up** and follow the steps for each permission.

## Approve the work

Before the agent touches an app, the chat shows a consent card such as **Allow Zeron to use Safari?**. Choose **Allow this conversation**, **Always allow**, or **Deny**. Always-allowed apps are listed under **Always-allowed apps** in Settings, where you can remove them. Zeron may still ask **Approve this tool call?** before consequential steps.

## Watch and take back control

A live preview shows what the agent sees and what it is doing. Its menu has **Hide for this task**, **Hide for all active tasks**, **Move to floating window**, and **Move to sidebar**. Closing the preview only hides it.

To stop:

- Open **Computer Use Controls** in the menu bar and choose **Stop Using** an app. The agent gives that app back for the rest of the turn and sums up. **Open chat** jumps to the chat.
- Press Escape to interrupt the agent's input.
- Click the chat's Stop button to end the turn.

Only one conversation drives the desktop at a time; others show who is in control and can still watch.

## Settings > Computer

- **Cursor colors**: **Stock** or **Custom** colors for the agent's pointer.
- **Preview**: show it automatically, **Compact** or **Large**; **Placement**: **Floating card** or **Rail + thumbnail**.
- **Click sounds**: **Foreground clicks**, **Foreground and background clicks**, or **Off**.
- **Developer access**: lets tasks run JavaScript in their Zeron browser tabs.
- **Recent Computer actions**: what agents did, without typed text or page contents.

> To show an agent a window without giving it control, use an [Appshot](zeron://docs/appshots).
