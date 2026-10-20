An Appshot is a screenshot of another app's window that goes straight into a Zeron chat. Press the hotkey while any app is in front, and Zeron captures that window, comes forward, and attaches the image to a composer. Appshots work in the desktop app on macOS and Linux.

## Turn it on

Open [Settings > Appshots](zeron://settings/appshots) and switch on **Enable Appshots**. The row's status line tells you what is still missing, and reads "Listening" with your hotkey once everything is ready. At launch Zeron shows a **Zeron Appshots are live!** card until you answer it once: **Set up Appshots** opens this page, **Not now** dismisses it.

## Hotkey

On macOS the **Hotkey** is a chord: press both keys of one modifier at the same time. Choose both Command keys (the default), both Option keys, both Shift keys, or **None** for no hotkey.

On Linux you record a regular key combination instead; the default is Ctrl+Alt+Space. Click the field, press the new keys, or press Escape to cancel. On Wayland, Zeron registers the shortcut through your desktop's global shortcuts portal. If your desktop has no such portal, or the shortcut cannot be registered, the status line reads "This shortcut is unavailable. Choose a different key combination." In that case, bind the command `zeron appshot` to a key in your desktop's own keyboard shortcut settings: it takes an Appshot in the running Zeron app, the same as the hotkey. See [Command line and servers](zeron://docs/command-line).

The hotkey lives only on this page; [Settings > Shortcuts](zeron://settings/shortcuts) has no row for it.

## Where the snap goes

**Destination** decides which composer receives the capture:

- **Automatic**: the session you used in the last minute, and consecutive snaps stay together. Otherwise Zeron opens a fresh session with the capture attached.
- **Last session**: the open session, or the last session you used for an Appshot.
- **New session**: a new-session composer, leaving your existing drafts alone.

The capture stays on this device until you send the message. In the chat it shows as an attachment named after the app, such as "Safari Appshot". Zeron itself cannot be captured; switch to another app first.

## Sound

**Capture sound** plays a short shutter cue when a window is captured. Click **Preview** to hear it.

## macOS permissions

The **macOS permissions** card lists what Appshots need:

- **Input Monitoring**: notices the hotkey chord while another app has the keyboard. Nothing you type is recorded.
- **Screen Recording**: captures the single window you snap, only when you press the hotkey.
- **Accessibility**: optional. Lets the agent read the window's text as well as the image.

Click **Grant** next to a permission to see the steps; the card updates when the grant lands, or use **Recheck permissions**. **Restore defaults** at the top of the page turns Appshots off and resets the hotkey, destination, and sound.

On Linux the card keeps the **macOS permissions** title but lists only **Screen Recording** and **Accessibility**, each with a status and no **Grant** button: there is nothing to grant. On X11, Zeron captures the active window and can include the window's text. On Wayland, capture goes through your desktop's screenshot portal, which may ask you to pick the window each time, and the snap is the image only.

> Appshots only attach an image and never give the agent control of your Mac; for that, see [Computer use](zeron://docs/computer-use).
