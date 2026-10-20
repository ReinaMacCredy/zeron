The **Review** chip runs a fresh agent over your changes and turns what it finds into findings you can fix or dismiss. The reviewer reports problems only; it is told never to edit files or commit.

## Start a review

The **Review** chip sits in the titlebar of a chat, next to the handoff button, with the size of the change. It shows when the chat's project is a git repository and there is something to review. Click it to review the current scope.

Click the caret next to it to choose what to review and who reviews:

- **Scope**: **Branch vs** the base branch, **Uncommitted**, **Latest turn**, or a single **Commit** from the recent history (**Filter commits…** narrows the list).
- **Reviewer**: **Auto**, a specific agent, or one of your agent profiles. Hover an agent to pick its model and **Thinking** level.

A reviewer picked here applies to this chat only.

While the review runs, the chip reads **Reviewing**. Hover it and click **Stop review** to stop the reviewer; your chat keeps running. The reviewer works in a child chat you can follow from the agents pill.

## Read the findings

When the review ends, the chip shows the number of findings, **No issues**, or **Review again** if the code changed since. Click the findings count to open the list. Each finding has a severity (**Bug**, **Risk**, or **Nit**), a title, and its file and line.

- **Fix N remaining** adds every open finding to your next message as comments. Send it, and your chat's agent works through them.
- **Open Changes** shows the right pane's **Working tree** diff tab, or the first Diffs tab you have open, or opens a new **Working tree** tab. Each finding sits as a card on its line with **Fix** and **Dismiss**, but only in a Diffs tab whose scope matches the review: **Working tree** for **Uncommitted**, **Branch changes** for **Branch vs**, **Latest turn** for **Latest turn**, and that commit's own tab (opened from **History**) for a **Commit** review. A finding whose code has changed since the review gets no card. See [Right pane](zeron://docs/right-pane).
- **Fix** adds one finding to your next message as a comment. A finding already handed to the agent is not added again. **Dismiss** removes it from the review.

## Outcomes

After the agent works on findings, each one shows its outcome: **Fixed**, **Skipped**, **No change**, or still **Open**. Skipped and unchanged findings show the agent's reason.

## Settings > Code review

[Settings > Code review](zeron://settings/code-review) sets the default reviewer for every chat:

- **Reviewer**: **Auto** picks a profile named reviewer, otherwise the other agent family from the chat's (Claude reviews Codex work and the reverse).
- **Model** and **Thinking** for the reviewer. They show only when **Reviewer** is a specific agent, not **Auto** or a profile.
- **Review instructions**: text added to every review prompt, for example what to focus on. Click **Save** after editing.

> Name an agent profile "reviewer" to make Auto use it.
