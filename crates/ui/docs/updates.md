Zeron updates itself and the agent CLIs it runs, and tells you what changed in each release.

## Updating the app

Zeron checks for a new version shortly after launch and then every hour. When one is out, a round download button appears in the app rail, just above **Settings**. Hover it to read its status:

- **Update available**: click to download in the background.
- **Downloading**: keep working while it downloads.
- **Update ready**: click to restart into the new version.
- **Update failed**: click to try again.

To check by hand, choose **Zeron > Check for Updates…** in the menu bar. The dialog tells you whether **Zeron is up to date**, or offers **Download** and then **Restart to update**.

## Updating agents

On the new chat canvas, a small card at the bottom lists agent CLIs (Claude, Codex, and others) with an update, across your devices. Each row shows the installed and new version and an **Update** button, or **View steps** when Zeron cannot install that agent for you. While an update runs you can **Cancel** it, and a failed check offers **Check again**. A busy agent shows "Waiting for the current run" and updates once the run ends.

When two or more updates are waiting, the app itself included, the card shows **Update all**. The dialog lists what will update; the app restarts last. Running threads are interrupted, so keep **Continue threads after restarts** checked to have them pick up again afterwards. Click **Update** to start.

[Settings > Providers](zeron://settings/providers) sets each agent's **Update policy**: **Notify**, **Auto when idle**, or **Off**. Click **Check now** there to look for new versions.

## Resuming after a usage limit

When a thread stops on a plan's usage limit, a pill above the composer says when the limit resets, for example "Usage limit · resumes 3:45 PM". Zeron continues the thread on its own shortly after the reset. Click **Cancel** on the pill to keep that thread stopped, or **Resume at reset** to turn it back on.

This follows **Auto-resume after usage limits** in [Settings > Providers](zeron://settings/providers), which is on by default and set per device.

## What's new

After an update, a card at the bottom of the sidebar names the release's headline feature. Click **See what's new** to open the **What's new?** dialog with that release's highlights, or click the **×** to dismiss the card. In the dialog, **View changelog** shows the **Changelog**: every release, newest first, each one expandable. **Got it** closes the dialog.

To open the changelog any time, choose **Zeron > What's New…** in the menu bar.

## The Help menu

The question-mark button in the app rail (**Help**) opens a menu with:

- **What's new**: the three most recent releases, each with its headline and date. Click one to read it in the changelog.
- **Full changelog**: the complete [changelog](zeron://changelog).
- **Keybindings**: opens [Settings > Shortcuts](zeron://settings/shortcuts).
- **Docs**: opens these help pages. See [Getting around](zeron://docs/getting-around).
