The Zeron iPhone app lets you follow and steer your sessions, tasks and missions from your phone. The phone is one more device in your synced workspace: it shows the same sessions, sends your messages and answers, starts new sessions and missions, and changes your Macs' settings, but no agent runs on it. Every session runs on one of your computers or servers, which has to be online to act on what you send.

The app has three tabs, **Sessions**, **Tasks** and **Settings**, plus search. It also runs on iPad, where the session list sits in a sidebar next to the open session. The Android app has the same tabs and pages; on an Android tablet, Tasks, missions, pull requests and Git open in the main column beside the session list.

## Sign in

The phone works only with a synced workspace. If your computer still uses a local workspace, turn on sync there first (see [Devices and sync](zeron://docs/devices)).

Open the app and tap **Sign In**, then sign in in the browser sheet with the same account you use on your computer. If you belong to more than one workspace, pick yours from **Choose an organization**. There is no pairing code: the same account and workspace is all it takes. The phone then shows up in [Settings > Devices](zeron://settings/devices) on your other devices.

**Settings** > **Sign Out** signs the phone out and removes the account's data from it, including messages still waiting to send and the **New session** draft. Text left in a session's composer stays on the phone.

## Your sessions

The **Sessions** tab lists your sessions. Each row shows its project and a status: **Working**, **Input** (the agent is waiting on you), **Failed**, or **Done** for a finished run you have not opened yet. **Pinned** sessions and your own sections appear as folders at the top. To organize, sort and nest the list, see [The sessions list and the Supervisor on iPhone](zeron://docs/mobile-inbox).

The search tab looks through **Sessions, projects, branches**.

The bar above the tabs holds **New session** and a live summary such as "2 working · 1 needs you".

Tap a session to open its transcript (see [The transcript on iPhone](zeron://docs/mobile-transcript)), and write in the composer at the bottom (see [The composer on iPhone](zeron://docs/mobile-composer)).

A line above the composer tells you how your message is doing: **Sending to host…**, **Offline — sends are saved** when the phone has no connection, "<device> is offline — will send when it's back" when the computer running the session is offline, or **Not delivered · Tap to retry**.

## Start a session

Tap **New session**. Describe the task under "What are we building?", then set where it runs with the chips:

- **Project**: one of your projects, **No Project…**, or **New Project…**.
- **Run on**: which of your computers runs it (**Choose host** until you pick one).
- **Checkout**: **New worktree**, or a **Branch** of the project.
- **Model** and **Reasoning effort**, and **Start mission** to start a mission instead.

If Zeron cannot tell where to run it, it says **Couldn't start the session**: choose a project or a host.

**New Project…** browses folders on one of your computers (switch computers with the button at the top right, which shows the current computer's name). Open folders to find yours, then tap **Use “<folder>”**. With no computers in your workspace yet, it says "No desktop devices yet — open Zeron on a computer to add one."

## Notifications

The phone can notify you when a run finishes (**Run finished**), when a session waits on you (**Waiting on your input**), and when a run fails (**Run failed**). Tap a notification to open that session. They stay quiet while the app is open.

Turn them on or off in the phone's **Settings** tab under **Notifications**, with a switch for each kind. If iOS has notifications turned off for Zeron, the row says "Turned off in iOS Settings"; turning the switch on then shows **Notifications are off**, where **Open Settings** takes you to iOS Settings.

## Voice

Voice mode on the phone is a call with Codex: your iPhone streams the audio straight to Codex's voice service, not through your computer, while Codex and its tools run on one of your computers. It appears once one of your Macs or servers runs a Zeron version that supports it; a device that needs an update says "Update Zeron and enable remote voice". Codex must be signed in on that computer.

Tap the waveform next to **New session** to call. Touch and hold it to choose the device (**Codex runs on**) and the voice style. During a call the bar shows the call with its timer and mute and hang-up buttons. Tap it to open the call screen, which shows what is happening (**Listening**, **Speaking**, **Working on it…**, **Codex needs your answer**, **Muted**) and can open the voice session's transcript.

You can hold the phone to your ear (the screen turns off and Codex plays through the earpiece) or lock it: the call keeps going on the Lock Screen and in the Dynamic Island, with **Mute** or **Unmute** and **End**. An incoming phone call ends it.

**Settings** > **Voice** sets the device and the voice for your next call. See [Voice and dictation](zeron://docs/voice) for voice mode on your computer.

## More on iPhone

- [The composer on iPhone](zeron://docs/mobile-composer): chips, permissions, context and stats, side chats, hand-offs and missions.
- [The transcript on iPhone](zeron://docs/mobile-transcript): Worked for, other agents, fork and rewind, usage limits and session info.
- [Tasks and missions on iPhone](zeron://docs/mobile-tasks): the Tasks tab, a task's sheet and a mission's page.
- [The sessions list and the Supervisor on iPhone](zeron://docs/mobile-inbox): organizing sessions, and each Mac's Supervisor.
- [Pull requests and Git on iPhone](zeron://docs/mobile-git): the board, a pull request's page, and a checkout's working tree and history.
- [Settings on iPhone](zeron://docs/mobile-settings): your Macs' accounts, usage, providers, profiles, routing and tools, the phone's own settings, What's New and Docs.
- [Devices and computer use on iPhone](zeron://docs/mobile-devices): consent, the still preview, stopping computer use, terminals and project actions.

## iPad keyboard shortcuts

| Keys | Action |
| --- | --- |
| Cmd+N | New session |
| Cmd+Shift+F | Search |
| Cmd+B | Show or hide the sidebar |
| Cmd+Return | Send |
