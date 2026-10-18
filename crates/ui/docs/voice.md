Dictation turns your speech into text in the composer, using a speech model that runs on this device. Voice mode is a live spoken conversation with a Codex agent.

## Turn on dictation

Open [Settings > Voice](zeron://settings/voice) and switch on **Dictation**. The first time, Zeron downloads the speech model (Parakeet v3); the row shows its size and then the download progress. Dictation turns on when the download finishes.

Once it is on you can also set:

- **Microphone**: **System default** or a specific input. If the microphone you chose is unplugged, Zeron records from the system default until it comes back.
- **Shortcut**: hold to talk, release to transcribe. The default is Cmd+D. You can rebind it here even before the model is downloaded.

The **Speech model** card has **Remove** to delete the downloaded model. Switching **Dictation** off keeps the model on disk.

## Dictate into the composer

With dictation on, a microphone button appears in the composer. Hold the button or hold Cmd+D, wait for **Listening**, speak, then release. The composer shows **Transcribing…** while your audio is processed on this device, and the text lands in your draft. One recording can be up to one minute long. Keep holding while you speak; a tap only shows a reminder.

## Voice mode

Voice mode needs Codex installed on this device. When it is available, a microphone button sits in the lower part of the left rail (tooltip **Start voice · Codex orchestrator**). Click it to start a call. A full-window voice stage opens with the call timer and these controls:

- **Mute** (or press M)
- **Transcript** opens the voice session's chat. It shows a dot when Codex is asking you something.
- **End** hangs up.
- **Back to chats** (or press Escape) hides the stage. The call keeps running.

While a call is live, the rail button turns into an animated orb. Click it to bring the stage back.

## Voice settings

- **Codex voice device** picks which of your devices runs Codex for the call. Audio stays on this device. You cannot change it during a call.
- **Codex voice style** picks the voice, or **Codex default**. A change applies to your next voice session.

> If a call cannot start, the rail button shows a small warning dot and a card explaining why, such as signing in to Codex with ChatGPT.
