Zeron updates itself and the agent CLIs it runs, and tells you what changed in each release.

## Updating the app

Zeron checks for a new version shortly after launch and then every hour. When one is out, a round download button appears in the app rail, just above **Settings**. Hover it to read its status:

- **Update available**: click to download in the background.
- **Downloading**: keep working while it downloads. An arrow drops into a tray under a turning arc.
- **Update ready — restart to apply**: click to restart into the new version. The button pops, shows a check, and turns into a restart arrow with a pulsing halo.
- **Update failed**: click to try again.
- **Restarting…**: Zeron is quitting to start the new version.

Zeron downloads only when you click, and a downloaded update installs only when you restart from the button or the update dialog; quitting Zeron does not install it. Turn on **Automatic updates** in [Settings > Updates](zeron://settings/updates) to have it download on its own and install when you quit. Setting `ZERON_AUTO_UPDATE` wins over the switch while it is set; see [Command line and servers](zeron://docs/command-line).

Some copies of Zeron cannot replace themselves, and the button says so:

- **Update available — v… · download from GitHub**: a copy no installer set up, such as a source build. Clicking opens the releases page and hides the button until the next version.
- **Update available — v… · run `zeron update`**: install it from a terminal (see [Command line and servers](zeron://docs/command-line)). Clicking hides the button until the next version.
- A copy without permission to replace itself, or a Mac copy running from a disk image or another temporary location, shows **Update available**, and clicking explains why. On a Mac, move Zeron to your Applications folder and reopen it.

## Settings > Updates

[Settings > Updates](zeron://settings/updates) gathers every update in one page:

- **This Mac**: **Version** shows the version you run and the same steps as the rail button: **Check for Updates**, **Checking…**, **Download**, **Downloading…**, **Restart to update**, or **Retry** when a check or download failed. **Automatic updates** downloads in the background and installs when Zeron quits.
- **Agent CLIs**: each agent CLI installed on this Mac with its version and the new one, its **Update policy** (**Notify**, **Auto when idle**, or **Off**) and **Update**. **Update all** updates every agent that has one.
- **Devices**: each paired device with the Zeron version it runs. Update another device on that device.

**Update everything** in the page header opens a dialog that lists this Mac with what will update: Zeron itself, which restarts last, and its agent CLIs, with how many running threads will be interrupted. Other devices are listed under **skipped** with "update on that device". Click **Update** to start, or **Cancel**.

![Settings > Updates](settings-updates.jpg)

To check by hand, choose **Zeron > Check for Updates…** in the menu bar on macOS, or **Check for Updates** in [Settings > Updates](zeron://settings/updates) on Windows and Linux. The dialog tells you whether **Zeron is up to date**, or offers **Download** and then **Restart to update**. **Later** closes it without restarting.

From a terminal, `zeron update` checks and installs in one step, and `zeron update --check` only reports.

## Updating agents

When agent CLIs (Claude, Codex, and others) have an update on any of your devices, the **Help** menu in the app rail shows **Agent updates** with how many; click it to open [Settings > Updates](zeron://settings/updates), where each agent has **Update** and **Update all** updates every agent that has one. On a new install, setup's **Your agents** step shows the same count with **Update all**.

[Settings > Updates](zeron://settings/updates) sets each agent's **Update policy**: **Notify**, **Auto when idle**, or **Off**. The refresh button in the header of [Settings > Providers](zeron://settings/providers) looks for new versions.

## Resuming after a usage limit

When a thread stops on a plan's usage limit, a pill above the composer says when the limit resets, for example "Usage limit · resumes 3:45 PM". Zeron continues the thread on its own shortly after the reset. Click **Cancel** on the pill to keep that thread stopped, or **Resume at reset** to turn it back on.

This follows **Auto-resume after usage limits** in [Settings > Providers](zeron://settings/providers), which is on by default and set per device.

## What's new

After an update, a card at the bottom of the sidebar names the release's headline feature, and counts the release's smaller changes, such as "+ 9 improvements and fixes". Click **See what's new** to open the **What's new?** dialog with that release's highlights; when more than one release came out since you last looked, it opens the **Changelog** with each of those releases unfolded instead. You can also click the **×** to dismiss the card. Each new feature has its own section, most with a moving picture; the smaller changes follow as one list under **Improvements and fixes**. In the dialog, **View changelog** shows the **Changelog**: every release, newest first, each one expandable. **Got it** closes the dialog.

![The What's new card](updates-whats-new-card.jpg)

To open the changelog any time, choose **Zeron > What's New…** in the menu bar.

On iPhone, the same card shows above the tabs after an update, and **Settings** > **Help** > **What's New** opens the release and the changelog. See [Zeron on iPhone](zeron://docs/mobile).

## The Help menu

The question-mark button in the app rail (**Help**) opens a menu with:

- **Docs**: opens these help pages. See [Getting around](zeron://docs/getting-around).
- **What's New**: the latest release's highlights, with **View changelog** for every release.
- **Keyboard shortcuts**: opens [Settings > Shortcuts](zeron://settings/shortcuts).
- **Star Zeron on GitHub**: opens the repository in your browser.
- **Agent updates**, when an agent CLI has an update: how many, and a click opens [Settings > Updates](zeron://settings/updates).

## One sheet per launch

Zeron shows at most one sheet each time it starts. A new install opens on setup in three steps: a welcome that offers sync (**Use without an account** keeps everything on this device), **Your agents**, and a last step with a link to these docs. After an update it is the What's new card above, and only one card is shown however many releases you skipped. How Tasks and plans work is in [Tasks](zeron://docs/tasks) and [Mission Control](zeron://docs/mission-control).
