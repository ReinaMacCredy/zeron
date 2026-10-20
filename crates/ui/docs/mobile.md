The Zeron iPhone app lets you follow and steer your sessions from your phone. The phone is one more device in your synced workspace: it shows the same sessions, sends your messages and answers, and starts new sessions, but no agent runs on it. Every session runs on one of your computers or servers, which has to be online to act on what you send.

The app also runs on iPad, where the session list sits in a sidebar next to the open session.

## Sign in

The phone works only with a synced workspace. If your computer still uses a local workspace, turn on sync there first (see [Devices and sync](zeron://docs/devices)).

Open the app and tap **Sign In**, then sign in in the browser sheet with the same account you use on your computer. If you belong to more than one workspace, pick yours from **Choose an organization**. There is no pairing code: the same account and workspace is all it takes. The phone then shows up in [Settings > Devices](zeron://settings/devices) on your other devices.

## Your sessions

The **Sessions** tab lists your sessions. Each row shows its project and a status: **Working**, **Input** (the agent is waiting on you), **Failed**, or **Done** for a finished run you have not opened yet. **Pinned** sessions and your own sections appear as folders at the top.

- Swipe right on a row to **Pin** or **Unpin** it.
- Swipe left to **Archive** it (a toast offers **Undo**) or **Move** it to a section (**Move to Section**, or **No Section** to take it out).
- Touch and hold a row for the same actions plus **Rename…**.
- The **…** button at the top has **New Section…**, **Collapse All**, **Group by Project**, and **Archived**.
- Touch and hold the header of one of your own sections to **Rename Section…** or **Delete Section**. Deleting a section moves its sessions back to the main list.

The search tab looks through **Sessions, projects, branches**.

The bar above the tabs holds **New session** and a live summary such as "2 working · 1 needs you".

## Reading and acting in a session

Tap a session to open its transcript. Touch and hold a message to **Copy**, **Copy Message**, or **Select Text**. When you have scrolled up, the **Jump to latest** button takes you back to the bottom. The session's **…** menu has **Pin** or **Unpin**, **Copy Transcript**, and **Archive**.

A line above the composer tells you how your message is doing: **Sending to host…**, **Offline — sends are saved** when the phone has no connection, "<device> is offline — will send when it's back" when the computer running the session is offline, or **Not delivered · Tap to retry**.

## The composer

The composer reads "Message" followed by the agent's name. **Attach** offers **Photo Library**, **Take Photo**, **Choose File**, and, when your clipboard holds an image, **Paste Image**. Type `@` to mention a file from the project. The composer's chips change the session's **Model** and **Reasoning effort**. With a keyboard attached, Cmd+Return sends.

While the agent is working, a message you send waits for the next turn. Touch and hold the send button for **While the agent works**: **Queue for next turn**, **Steer now** (deliver into the running turn), or **Stop and send**. With nothing typed, the button stops the run (**Stop response**).

Queued messages wait in a list above the composer. Tap the arrow on one to send it now, or open its **…** menu to **Edit**, **Move Up**, **Move Down**, or **Remove** it. If another device is editing that message at the same moment, or it was just sent, the phone says **Can't edit right now**.

## Answering the agent

When the agent asks you something, a question panel opens at the composer. Pick an option, or type your own answer in **Other…**. With several questions, step through them with **Back** and **Next**, then tap **Submit**. See [When an agent asks you something](zeron://docs/agent-questions).

When an agent wants to use an app on a Mac through [Computer use](zeron://docs/computer-use), its consent card shows in the transcript with buttons to allow or deny it, so you can answer from the phone.

## Start a session

Tap **New session**. Describe the task under "What are we building?", then set where it runs with the chips:

- **Project**: one of your projects, **No Project…**, or **New Project…**.
- **Run on**: which of your computers runs it (**Choose host** until you pick one).
- **Checkout**: **New worktree**, or a **Branch** of the project.
- **Model** and **Reasoning effort**.

If Zeron cannot tell where to run it, it says **Couldn't start the session**: choose a project or a host.

**New Project…** browses folders on one of your computers (switch computers with the **Device** button). Open folders to find yours, then tap **Use “<folder>”**. With no computers in your workspace yet, it says "No desktop devices yet — open Zeron on a computer to add one."

## Notifications

The phone can notify you when a run finishes (**Run finished**), when a session waits on you (**Waiting on your input**), and when a run fails (**Run failed**). Tap a notification to open that session. They stay quiet while the app is open.

Turn them on or off in the phone's **Settings** tab under **Notifications**, with a switch for each kind. If iOS has notifications turned off for Zeron, the row says "Turned off in iOS Settings"; turning the switch on then shows **Notifications are off**, where **Open Settings** takes you to iOS Settings.

## Voice

Voice mode on the phone is a call with Codex: the audio stays on your iPhone, while Codex and its tools run on one of your computers. It appears once one of your Macs or servers runs a Zeron version that supports it; a device that needs an update says "Update Zeron and enable remote voice". Codex must be signed in on that computer.

Tap the waveform next to **New session** to call. Touch and hold it to choose the device (**Codex runs on**) and the voice style. During a call the bar shows the call with its timer and mute and hang-up buttons. Tap it to open the call screen, which shows what is happening (**Listening**, **Speaking**, **Working on it…**, **Codex needs your answer**, **Muted**) and can open the voice session's transcript.

You can hold the phone to your ear (the screen turns off and Codex plays through the earpiece) or lock it: the call keeps going on the Lock Screen and in the Dynamic Island, with **Mute** or **Unmute** and **End**. An incoming phone call ends it.

**Settings** > **Voice** sets the device and the voice for your next call. See [Voice and dictation](zeron://docs/voice) for voice mode on your computer.

## Phone settings

The **Settings** tab has:

| Section | What is there |
| --- | --- |
| **Account** | Who you are signed in as. |
| **Devices** | Your computers and servers, each **Online** or **Offline**. |
| **Voice** | Device and voice for calls, when voice is available. |
| **Notifications** | The switches above. |
| **Appearance** | **System**, **Light**, or **Dark**. |
| **Wallpaper** | **Choose Wallpaper…** (then **Change Wallpaper…**) for new chats and the session list, its **Effect**, and **Remove Wallpaper**. |
| **Math** | **Render Math** (off shows LaTeX source), **Math Font**, and **Math Size**. |
| **Sessions** | **Group by Project** and **Archived Sessions**. |

**Sign Out** at the bottom signs the phone out. Drafts you have not sent stay on the phone.

## iPad keyboard shortcuts

| Keys | Action |
| --- | --- |
| Cmd+N | New session |
| Cmd+Shift+F | Search |
| Cmd+B | Show or hide the sidebar |
| Cmd+Return | Send |
