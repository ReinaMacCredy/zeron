The iPhone opens the same [pull requests](zeron://docs/pull-requests) and [Git](zeron://docs/git) pages as your computer. Everything runs on a Mac signed in to GitHub with `gh`; the phone reads and acts through it.

Tap the **Pull requests** button in the **Sessions** bar to open them. The switch at the top moves between **Pull requests** and **Git**. A pull request link in a transcript, or the pull request on a session, opens its page in the app instead of Safari. On iPad, the board, a pull request and Git open in the detail column beside the sidebar.

## The pull request board

- The pills at the top pick the Mac and the **Repository**. A Mac that is offline is marked **Offline**, and the phone keeps your last pick.
- **Authored**, **Reviewing** or **All** picks whose pull requests you see.
- **Everything**, **Needs you** and **Starred** narrow the list. Sort by **Recently updated**, **Recently opened**, **Newest first**, **Oldest first** or **Most changes**.
- **Search pull requests** filters by text. Each row shows its checks; swipe left on one to **Star** or **Unstar** it. **Load more** fetches the next page.

With no Mac able to serve them, the board says **No Mac serves pull requests** and what to set up on the Mac.

## A pull request's page

![A pull request's Summary, and the merge confirm](ios-git.jpg)

The page has three tabs:

- **Summary**: the status, **Branch**, checks, **Labels** and **Reviewers**, with the merge and hand-off actions.
- **Code**: each changed file's diff, with line threads under their line.
- **Activity**: comments, reviews and line threads.

What you can do from it:

- Comment with **Write a comment…**, **Reply** to a thread, and **Resolve** or **Unresolve** it.
- **Review**: **Approve**, **Request changes** or **Comment as review**. The text in the box becomes the review summary.
- **Change labels** and **Request a review** open searchable pickers.
- **Merge pull request** with the method you pick. It asks first, for example "This merges #90 into main using squash."
- **Hand off to an agent**: **Review**, **Address feedback**, **Fix failing checks** or **Resolve conflicts** opens a new session on the pull request's Mac and project, with the prompt ready to send.
- **Ask about this PR** opens a new session that carries the pull request's context. See [Pull requests](zeron://docs/pull-requests).

## Git

The **Git** side shows one checkout on a Mac: its branch, how far it is ahead and behind, and **Pull**, **Push** and **Commit**. The checkout picker lists the **Main checkout** and the **Agent worktrees**. When an agent is working in the checkout, a banner says so.

**Working tree**:

- **Staged** and **Changes**, each file with **Stage** or **Unstage**, plus **Stage all** and **Unstage all**. Tap a file to see its diff.
- Stashes are listed under **Stashes**; the phone does not change them.
- If the checkout changed since it was shown, the page refreshes and says so instead of acting.

**Commit** opens the commit sheet: a **Commit message**, **Write with AI** to draft one, and a button such as **Commit 2 files**. **More commit options** has **Commit all** (stage everything, then commit) and **Amend** (fold into the previous commit, when it is not pushed).

**Pull** runs at once. **Push** asks first.

**History** lists the commits with their branch lanes. Tap a commit for its author, date, files and diffs, and **Copy SHA**.
