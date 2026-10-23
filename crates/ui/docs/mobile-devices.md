When an agent uses apps on a Mac, or runs commands in a project, the iPhone shows it in the session and lets you answer, watch and stop it. The agent always runs on the Mac; see [Computer use](zeron://docs/computer-use) for how it works there.

## Computer use in the transcript

![The consent card, and Stop computer use for one chat](ios-computer-use.jpg)

- Each app action is a row in the transcript with the same label as on the Mac, and a turn's actions fold under one summary of the apps it used and how many actions it took. Calls to a phone or simulator fold under "Used device controls" in the same way.
- **Allow Zeron to use** an app? asks before the agent first uses it. Terminals, messaging apps, browsers and file managers carry an **Elevated Risk** line. Answer with **Deny**, **Always allow** or **Allow this conversation**; the card then reads, for example, "Allowed Terminal for this task".
- **Bring** an app **to the front?** asks before the agent raises a window over yours, with **No** or **Allow for this turn**.
- When computer use needs setup or is unavailable, a card says so. When it was turned down, the card has **Enable**, which puts `/computer-use` in the composer so the next message may use the Mac.
- Short notes mark what happened on the Mac, such as "Desktop access revoked by you", "Paused while you used" an app, or how many windows it left open.

## The still preview

While the agent uses apps, a still preview of the Mac's screen floats over the chat with a **Live** pill, and briefly marks when computer use finished. Its hide button offers **Hide for this task** or **Hide for all active tasks**.

## Stop computer use

![The preview's hide menu, and Stop computer use on a whole Mac](ios-devices-stop.jpg)

- In a chat: the session's **…** menu has **Stop Computer Use** while the agent is using apps. It asks first: "Zeron stops using apps on" that Mac "for this chat now. Other chats keep their access."
- On a whole Mac: **Settings** > **Devices** > the Mac > **Stop computer use** stops every chat on that Mac. It asks "Stop computer use on" that Mac first. The row shows only for a computer that can run computer use.

## Terminal

**Terminal** in the session's **…** menu opens the chat's terminals on the Mac, one tab each, in their colors, refreshed every second. "Output only. Type into this terminal on the Mac." With none yet, it says "No terminals in this chat yet." See [Terminal](zeron://docs/terminal).

## Project actions

**Project Actions** in the session's **…** menu lists the project's actions with their icon and command. Tap **Run** to start one on the Mac; it shows **Running**, then how it ended, such as "Failed (exit 1)". Tap a run to open its terminal: "Each run opens a tab named after the action in Terminal." A project with no actions says to add them on the Mac. See [Projects](zeron://docs/projects).
