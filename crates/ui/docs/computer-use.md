Computer use lets an agent see and drive the apps on your Mac: it takes screenshots, clicks, and types. It needs macOS, and nothing is driven until you authorize a Computer task.

## Ask for a task

Type `/computer-use` followed by your task, for example:

```
/computer-use open Calculator and calculate 123 × 45
```

This turns Computer on for that one request. To give agents Computer tools on every turn, switch on **Computer control** in [Settings > Computer](zeron://settings/computer). It is off by default.

On a Mac, when Computer is on for the message you are writing and a Claude Code chat runs at a default effort other than Medium, a tip above the composer reads **Desktop actions are faster at Medium effort**. **Use Medium** sets this chat's effort to Medium; **Dismiss tip** hides it. Either one stops the tip for good.

### Cards in the chat

| Card | What it means |
| --- | --- |
| **Computer is off for ordinary turns** | The agent tried to use the desktop, but **Computer control** is off and you did not ask with `/computer-use`. **Use for this request** puts `/computer-use` in front of your draft so the next message may use it. |
| **Computer control is ready** | The missing permissions are now granted. Send a message and the agent picks up where it left off. |
| **Computer control needs …** | Names the macOS permissions still missing. **Set up** opens the permission guide. |
| **Left 2 Safari windows open** (for example) | Windows the agent opened during the turn are still open. **Show** brings that app forward. |

## Permissions

Zeron needs three macOS permissions, listed under **Advanced** on the Computer page. **Advanced** starts collapsed: click **Show** to open it (**Set up** opens it too).

- **Accessibility** to move the pointer, click, and type.
- **Screen Recording** to see the windows it drives.
- **Input Monitoring** to notice Escape and pause when you take over.

The status line under **Computer control** says whether the desktop is ready. When something is missing, click **Set up** and follow the steps for each permission.

## Approve the work

Before the agent touches an app, the chat shows a consent card such as **Allow Zeron to use Safari?**. Choose **Allow this conversation**, **Always allow**, or **Deny**. Always-allowed apps are listed under **Always-allowed apps** in Settings, where you can remove them. Zeron may still ask **Approve this tool call?** before consequential steps.

![Approving computer use for an app](computer-use-consent.jpg)

## Watch and take back control

A live preview shows what the agent sees and what it is doing. Its menu has **Hide for this task**, **Hide for all active tasks**, **Move to floating window**, and **Move to sidebar**. Closing the preview only hides it.

To stop:

- Open **Computer Use Controls** in the menu bar and choose **Stop Using** an app. The agent gives that app back for the rest of the turn and sums up. **Open chat** jumps to the chat.
- Press Escape to interrupt the agent's input.
- Click the chat's Stop button to end the turn.

Only one conversation drives the desktop at a time; others show who is in control and can still watch.

The agent shares your desktop and works in the background by default. It can bring a window forward when your task asks to watch, and background input may still affect which window has focus.

## Settings > Computer

- **Cursor colors**: **Stock** (the default, a monochrome pointer) or **Custom**, which adds **Fill** and **Rim** fields for hex colors such as `#ff8800`. Custom colors apply to new computer sessions.
- **Preview**: show the live preview the first time an agent acts in a chat (on by default), **Compact** (the default) or **Large**.
- **Placement**: **Floating card** over the transcript, or **Rail + thumbnail** (the default), a small thumbnail under Computer in the right rail; click it to pop out.
- **Click sounds**: **Foreground clicks** (the default), **Foreground and background clicks**, or **Off**.
- **Developer access**: off by default. Lets all tasks run JavaScript and collect partial page diagnostics in their Zeron browser tabs. Page scripts can access sensitive site data. It does not enable full CDP.
- **Getting started**: a three-step guide (**Ask for a task**, **Approve the task**, **Follow and stop**). It stays open until you click **Got it** or **Hide guide**; after that, **Show guide** brings it back.
- **Recent Computer actions**: collapsed until you click **Show history**. It lists recent actions across chats, without read-only observations, typed text, page contents, or file paths. Each row shows the action, the app, the time, and what happened: **Effect observed**, **Sent; effect unconfirmed**, **Not sent**, **Blocked**, or **Failed**. **Refresh history** reloads it and **Load older actions** pages back, up to the latest 100.
- **Advanced**: the macOS permissions, plus two read-only rows. **Desktop driver** shows the driver revision and whether it matches this build. **Desktop abilities**, shown once the desktop is ready, lists what the driver can do, such as screen capture, input, and window raising.

![Settings > Computer](computer-use-settings.jpg)

> To show an agent a window without giving it control, use an [Appshot](zeron://docs/appshots).
