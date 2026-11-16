# Do it in Zeron

Use this when the person says "implement this in Zeron", "code this in Zeron", "do this in Zeron" or "clone this repo in Zeron". Zeron is the coding agent app on this Mac; this skill starts a new Zeron chat for the job. The Yab chat needs Mac access to run the command.

## Implement or code something

1. Gather what Zeron needs from the page the person means: its text, its links, and a description in words of any image that matters (Zeron cannot see the page). Put it in `prompt`, after the person's own request, so Zeron can work without opening the page.
2. Pick the Zeron project. If the person named one, pass it as `project` (a project's name, id or path). If they did not, ask which Zeron project before running anything.
3. Run:

   `zeron plugin call yab-bridge start '<json>' --timeout 300`

   where `<json>` is `{"prompt": "...", "project": "..."}` (for a clone, `{"repo": "...", "root": "...", "prompt": "..."}`). `title` may be added, but Zeron ignores it for now.

## Clone a repository

When the person says "clone this repo" (or gives a GitHub address), pass `repo` as `https://github.com/<owner>/<name>`. If they also asked for something to be done with it, pass that as `prompt`; otherwise leave `prompt` out and Zeron reads the repository and summarises it. Do not pass `project`: the clone becomes a new project. Only github.com addresses are accepted.

Also pass `root`, the absolute path of the person's code folder: `$HOME/Code` expanded to a full path by you before you build the JSON (for example `/Users/name/Code`), unless the person names another folder. Never put a literal `~` or `$HOME` in the JSON.

## Building the command

Page text contains quotes, newlines and backslashes. Build the JSON with a real JSON encoder (for example `JSON.stringify`, or `json.dumps`), never by joining strings. Pass the result to `zeron plugin call` as one argument, quoted for the shell with a real shell quoter (for example Python's `shlex.quote`), or written to stdin or a file by a script that does the quoting. Never paste page text into the command line by hand.

## Reading the result

- Exit 0: stdout is JSON, `{"chat": "...", "project": "...", "cwd": "..."}`. Tell the person a Zeron chat was started and give the chat id and the folder.
- Exit 1: show stderr to the person, in its own words. When it lists project names, ask the person to pick one and run the command again with that `project`. When it says the clone folder is not set, tell them to choose one in Zeron under Settings, Yab bridge.
- Do not run the command twice for one request: a second run starts a second chat.
- Every chat this starts is Supervised: Zeron asks the person before it runs a command or changes a file, because the request carries text from a web page. Tell the person they can raise its access in Zeron if they trust the task.
