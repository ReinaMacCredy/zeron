Dictation turns your speech into text in the composer, using a speech model that runs on this device. Voice mode is a live spoken conversation with a Codex agent.

## Turn on dictation

Open [Settings > This Mac > Voice](zeron://settings/voice) and switch on **Dictation**. The first time, Zeron downloads the speech model (Parakeet v3); the row shows its size and then the download progress. Dictation turns on when the download finishes.

The same card has:

- **Microphone**, shown once dictation is on and the model is downloaded: **System default** or a specific input. If the microphone you chose is unplugged, it shows as **Disconnected microphone** and Zeron records from the system default until it comes back.
- **Shortcut**, always shown: hold to talk, release to transcribe. The default is Cmd+D. You can rebind it before the model is downloaded.

Clicking **Dictation** while the model downloads cancels the download.

The **Speech model** card has **Remove** to delete the downloaded model. Switching **Dictation** off keeps the model on disk.

## Dictate into the composer

With dictation on, a microphone button appears in the composer. Hold the button or hold Cmd+D, wait for **Listening**, speak, then release. The composer shows **Transcribing…** while your audio is processed on this device, and the text lands in your draft. One recording can be up to one minute long. Keep holding while you speak; a tap only shows a reminder.

The composer tells you where dictation is:

| Status | Meaning |
| --- | --- |
| **Getting ready…** | The microphone is starting. Wait for **Listening** before you speak. |
| **Listening** | Speak now; release when you are done. |
| **Transcribing…** | Your audio is being turned into text on this device. |
| **No speech detected** | Nothing was heard. Check your microphone, then try again. |
| **Hold to dictate** | You tapped instead of holding. Keep holding the microphone or the shortcut while you speak. |
| **Dictation unavailable** or **Dictation stopped** | Something went wrong; the reason is shown under it. Click **Dismiss** to clear it. |

To throw a recording away, click the **×** that replaces the attach button while you dictate (**Cancel dictation**), or press Escape.

## Voice mode

Voice mode needs Codex installed on this device and turned on in [Settings > Providers](zeron://settings/providers). When it is available, a microphone button sits in the lower part of the left rail (tooltip **Start voice · Codex orchestrator**). Click it to start a call. A full-window voice stage opens with the call timer and these controls:

- **Mute** (or press M)
- **Transcript** opens the voice session's chat. It shows a dot when Codex is asking you something.
- **End** hangs up.
- **Back to chats** (or press Escape) hides the stage. The call keeps running.

While a call is live, the rail button turns into an animated orb. Click it to bring the stage back.

## Voice settings

- **Codex voice device** picks which of your devices runs Codex for the call. The audio goes from this device straight to Codex's voice service, not through the device you pick. You cannot change it during a call.
- **Codex voice style** picks the voice, or **Codex default**. The list offers **Juniper**, **Maple**, **Spruce**, **Ember**, **Vale**, **Breeze**, **Arbor**, **Sol**, and **Cove**; when a call reports its own list of voices, the menu shows that list instead. A change applies to your next voice session.

A call plays a short chime when it starts and when it ends. You can also call Codex from your phone; see [Zeron on iPhone](zeron://docs/mobile).

> If a call cannot start, the rail button shows a small warning dot and a card explaining why, such as signing in to Codex with ChatGPT.
