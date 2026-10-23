Zeron updates itself and the agent CLIs it runs, and tells you what changed in each release.

## Updating the app

Zeron checks for a new version shortly after launch and then every hour. When one is out, a round download button appears in the app rail, just above **Settings**. Hover it to read its status:

- **Update available**: click to download in the background.
- **Downloading**: keep working while it downloads.
- **Update ready — restart to apply**: click to restart into the new version.
- **Update failed**: click to try again.
- **Restarting…**: Zeron is quitting to start the new version.

Zeron downloads only when you click, and a downloaded update installs only when you restart from the button or the update dialog; quitting Zeron does not install it. (Setting `ZERON_AUTO_UPDATE=1` makes it download on its own and install when you quit; see [Command line and servers](zeron://docs/command-line).)

Some copies of Zeron cannot replace themselves, and the button says so:

- **Update available — v… · download from GitHub**: a copy no installer set up, such as a source build. Clicking opens the releases page and hides the button until the next version.
- **Update available — v… · run `zeron update`**: install it from a terminal (see [Command line and servers](zeron://docs/command-line)). Clicking hides the button until the next version.
- A copy without permission to replace itself, or a Mac copy running from a disk image or another temporary location, shows **Update available**, and clicking explains why. On a Mac, move Zeron to your Applications folder and reopen it.

To check by hand, choose **Zeron > Check for Updates…** in the menu bar on macOS, or **Check for updates** in the account menu on Windows and Linux. The dialog tells you whether **Zeron is up to date**, or offers **Download** and then **Restart to update**. **Later** closes it without restarting.

From a terminal, `zeron update` checks and installs in one step, and `zeron update --check` only reports.

## Updating agents

On the new chat canvas, a small card at the bottom lists agent CLIs (Claude, Codex, and others) with an update, across your devices. Each row shows the installed and new version and an **Update** button, or **View steps** when Zeron cannot install that agent for you. While an update runs you can **Cancel** it, and a failed check offers **Check again**. A busy agent shows "Waiting for the current run" and updates once the run ends.

![Agent updates waiting on the new chat canvas](updates-agent-card.jpg)

When two or more updates are waiting, the app itself included, the card shows **Update all**. The dialog lists what will update; the app restarts last. Running threads are interrupted, so keep **Continue threads after restarts** checked to have them pick up again afterwards. Click **Update** to start.

[Settings > Providers](zeron://settings/providers) sets each agent's **Update policy**: **Notify**, **Auto when idle**, or **Off**. Click **Check now** there to look for new versions.

## Resuming after a usage limit

When a thread stops on a plan's usage limit, a pill above the composer says when the limit resets, for example "Usage limit · resumes 3:45 PM". Zeron continues the thread on its own shortly after the reset. Click **Cancel** on the pill to keep that thread stopped, or **Resume at reset** to turn it back on.

This follows **Auto-resume after usage limits** in [Settings > Providers](zeron://settings/providers), which is on by default and set per device.

## What's new

After an update, a card at the bottom of the sidebar names the release's headline feature. Click **See what's new** to open the **What's new?** dialog with that release's highlights, or click the **×** to dismiss the card. In the dialog, **View changelog** shows the **Changelog**: every release, newest first, each one expandable. **Got it** closes the dialog.

![The What's new card](updates-whats-new-card.jpg)

To open the changelog any time, choose **Zeron > What's New…** in the menu bar.

On iPhone, the same card shows above the tabs after an update, and **Settings** > **Help** > **What's New** opens the release and the changelog. See [Zeron on iPhone](zeron://docs/mobile).

## The Help menu

The question-mark button in the app rail (**Help**) opens a menu with:

- **What's new**: the three most recent releases, each with its headline and date. Click one to read it in the changelog.
- **Full changelog**: the complete [changelog](zeron://changelog).
- **Keybindings**: opens [Settings > Shortcuts](zeron://settings/shortcuts).
- **Docs**: opens these help pages. See [Getting around](zeron://docs/getting-around).
- **Mission Control intro**: shows the [Mission Control](zeron://docs/mission-control) introduction again.
- **New since 0.2.114**: shows the introduction to what changed since 0.2.114 again.

## Introductions

When a release brings something you have not used yet, Zeron introduces it once in a sheet over the window, with a picture on each page, page dots, **Not now** and **Next**. The left and right arrow keys turn the pages and Escape closes the sheet.

**New since 0.2.114** has four pages: **Tasks is home for missions**, **A mission's tabs travel together**, **Missions on the sidebar**, and **Ask the Supervisor about any thread**. **Open Tasks** on its last page opens [Tasks](zeron://docs/tasks).

![The New since 0.2.114 sheet](updates-new-since.jpg)

An introduction stays out of your way:

- Once you close it, with **Not now** or its last button, it does not come back on its own. The **Help** menu above shows it again.
- A page about something you have already used is left out. Opening Tasks, folding a mission's tab group, opening a mission's steps on the sidebar, and opening the Supervisor's **Conversation** each count. When every page is covered, the sheet does not appear at all.
- A new install skips the introductions to earlier releases, since everything is new to it.
- At most one introduction appears each time Zeron starts, and only after sign-in and sync are done. Another one waits for the next start.
