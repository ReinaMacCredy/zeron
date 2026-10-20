The Pull requests page lists your GitHub pull requests and opens each one in a native view, with an AI chat that knows the PR. It reads GitHub through the GitHub CLI, so install `gh` on the device and run `gh auth login` once.

## The board

Click **Pull requests** in the rail. Pick a repository in the repository menu, which lists your projects and recent repositories.

- Choose **Authored**, **Reviewing**, or **All**, or one of the **Needs you** and **Starred** views.
- Type in **Search pull requests** to match a title, repository, number, status, or author.
- Use the sort button for **Recently updated**, **Newest first**, **Largest changes**, and more.

Pull requests are grouped into **Needs attention**, **Awaiting review**, **Approved**, and **Drafts**. Star one to keep it in **Starred**. Click a row to open it.

## A pull request

The PR view has three sections. Switch with the pill at the bottom, or press 1, 2, or 3 when no field has focus. Press Escape to close the PR.

| Section | What it shows |
| --- | --- |
| **Summary** | Status, review, branch, changes, checks, the description, and the **Merge pull request** card |
| **Code** | The changed files and their diffs |
| **Activity** | Comments and review threads, with a comment box and **Review**: **Approve**, **Request changes**, or **Comment as review** |

The title bar has **Refresh pull request**, **Copy link**, and **Open in default browser**. The **Hand off to an agent** card on Summary opens a new session with a prompt ready for **Review**, **Address feedback**, **Fix failing checks**, or **Resolve conflicts**.

## Ask AI about a pull request

Click the chat button next to the section pill (tooltip **Ask AI**), or press `/`, to open the PR chat dock; click it again to fold the chat. The button shows a spinning ring while the agent works and a badge with the number of new replies. A dot on it means you left an unsent message in the folded chat.

Ask about the change or hand off work. Above the message box, chips show the context a new chat gets: the PR (`#123 owner/repo`), its **Diff**, its comments, and its **Checks**. Hover a chip and click its × (**Remove context**) to leave that part out. The context goes with the message that starts the chat.

The dock header has **New chat**, **Open as thread** to move the chat into a normal session, and **Fold chat**. With **Chat history** set to **Keep per PR** and **New chat when idle** set to a time, it also shows how long the chat has left before Zeron starts a fresh one.

Set **Chat history** and **New chat when idle** in [Settings > Source Control](zeron://settings/source-control); [Git](zeron://docs/git) explains the choices.

Cmd+R (**Toggle right sidebar**) opens and closes the right pane on the Pull requests page too.

## Native view or browser

In [Settings > General](zeron://settings/general), under **Links**, **Open pull requests in** chooses **Pull request view** or **Default browser**. It applies to PR badges and the Pull requests board. Pull requests outside github.com always open in the browser, and PR links inside a chat follow your normal link setting.

> The page has a second tab, **Git**, for commits and the working tree. See [Git](zeron://docs/git).
