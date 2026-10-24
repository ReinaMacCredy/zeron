# UI driver: verify and debug Zeron UI changes as an agent

Every headed Zeron process has an in-process UI driver. It reads the window as text, acts on
elements, waits for the UI to change, measures animation frames, takes snapshots, compares a
region against its HTML mockup, records a person's clicks as a scenario file and steps
animations on a paused clock, without computer use. It runs on the gpui main thread and sees
what gpui sees: the AccessKit tree, the pages inside the window's web views, the Shell route,
the frame clock and the window's own pixels.

Use it for anything inside a Zeron window: layout and copy checks, click paths, transitions,
accessibility, scripted sessions. Keep computer use for other apps, for paths the driver skips
on purpose (real trackpad and IME input, TCC prompts, the window server's event stream), and
for testing computer use itself.

## Two levels

| Level | Actions | Builds | MCP tool |
|---|---|---|---|
| Observe | `state`, `tree`, `snapshot`, `wait`, `frames`, `track`, `audit`, `record`, `compare` | every build, the stable app included | `ui_driver` |
| Drive | `click`, `move`, `drag`, `scroll`, `key`, `type`, `press`, `action`, `set`, `clock`, plus `during` on `frames`/`track` | debug builds only (Zeron Dev bundles) | `ui_driver_drive` |
| Process | `launch`, `quit` | run by the client on macOS, never sent to an app | `ui_driver_drive`; `ui_driver` refuses them |

Drive code is compiled out of release builds. A release build answers a drive request with:
`click` drives the UI and exists only in debug builds (Zeron Dev bundles); this build is
observe-only. The same refusal answers `clock` and `record` with `includeDriven`. A `frames`
or `track` with a drive `during` gets it for `during`, after ``invalid `frames` request:``. In
every build, a `during` whose action is not a drive action gets
`` `during` must be a drive request (click, ...) ``, and `during` cannot be `clock`. To drive,
launch a Dev bundle (next sections). The stable app in `/Applications/Zeron.app` hosts the
person's running sessions, so observe it only. It serves the driver only once it runs a build
that includes it.

`launch` and `quit` start and stop Zeron Dev bundles (see Launching a Dev bundle). `launch`
takes no `target`; `quit` needs one.

What each level exposes:

- Observe reads what the app's windows show, text field values included (a key typed into
  the Add provider form is in `tree` and in a `snapshot`), to any chat that has the `ui_driver`
  tool on. Turn the tool off in Settings > Tools to withhold it.
- Observe changes no app state, so `ui_driver` is annotated read-only, but it has local
  effects: `snapshot` and `compare` write a PNG under `~/.zeron/run/shots` and `record` a
  scenario file under `~/.zeron/run/recordings`, which nothing prunes; `frames` and `track`
  turn on the frame trace while they record; a tree read keeps the window's accessibility tree
  on for 60 s, and a running `record` keeps it on until `stop`.
- `record` taps the person's real clicks, keys, typing and scrolls in that app, so
  `ui_driver` refuses to record the app that serves the calling chat: a recording there would
  capture what the person types into this chat. That refusal names the way out: the person
  runs `zeron ui record` in their own shell, or the agent records a Dev bundle. The CLI has no
  such check, because it is the owner's path. Text typed into a field that looks secret is
  written as `<redacted>` in every build (see Recording a session).
- Drive in a Dev bundle goes around the chat permission gates of that bundle: an agent can click
  Approve on a pending request there, or type into its terminal panel. Keep Dev bundles on test
  data.
- `ui_driver_drive` refuses the app that serves the calling chat (any app whose `ipcPort` is the
  chat's engine port), so a chat cannot approve its own requests through the tool. `zeron ui`
  and the raw socket have no such check: an agent with a shell can still reach its own app.
  Turn `ui_driver_drive` off in Settings > Tools to keep reading without driving; turning
  `ui_driver` off turns both off.
- `ui_driver_drive` carries a destructive hint, so a Codex chat at Ask or Auto asks the person
  before every call ("Approve UI drive"). A Codex chat at Full gets the tool pre-approved in its
  thread config (`mcp_servers.zeron.tools.ui_driver_drive.approval_mode = "approve"`), so it
  drives without asking. `device_close` is pre-approved in every mode.

## Reaching a target

Each process writes a descriptor `~/.zeron/run/ui-<pid>.json` and listens for the client:

- macOS and Linux: a unix socket `~/.zeron/run/ui-<pid>.sock` (mode 0600, directory 0700).
  The descriptor names it as `socket`.
- Windows: a loopback TCP port on `127.0.0.1`, picked by the system. The descriptor carries the
  port as `driverPort` and a random `token`. The client sends the token as the first line of
  each connection. A connection whose first line is not that token gets
  `the UI driver refused this connection: its first line must be the token from the app's ui-<pid>.json`
  and is closed. Anything that can read the descriptor can reach the app, so the run folder's
  permissions are the guard. The run folder is `%USERPROFILE%\.zeron\run` when `HOME` is
  unset.

A clean quit removes the descriptor and the socket. A crash, SIGTERM or SIGKILL leaves them, and
the client prunes any descriptor whose pid is gone or whose endpoint refuses.

Two clients share one implementation (`crates/mcp/src/ui_driver.rs`):

- CLI: `zeron ui list`, `zeron ui <action> ['<json args>'] [--target <t>]`, and
  `zeron ui scenario <file.json> [--target <t>] [--out <dir>]`. `zeron` is not on PATH in a
  checkout. Run `cargo build -p zeron` once, then call `$CARGO_TARGET_DIR/debug/zeron` (or
  `target/debug/zeron`). Any build of the CLI can talk to any app. Text replies (`tree`, `compare`) print
  raw and everything else prints as pretty JSON. Errors go to stderr with exit code 1.
- MCP tools in `zeron mcp` (Settings > Tools, Workspace group): `ui_driver` takes `list` and the
  observe actions, `ui_driver_drive` the drive actions, `launch` and `quit`, plus `frames` and
  `track` with `during`. Each refuses the other's actions with an error that names the right
  tool. The arguments are the CLI JSON plus `"action"` and an optional `"target"`. `snapshot`
  also returns the PNG as an image block unless `"inline": false`; `compare` returns its report
  as text.

`zeron ui list` prints the live descriptors: `pid`, `socket` (or `driverPort` and `token` on
Windows), `label`, `bundlePath`, `dataDir`, `ipcPort`, `build` (`debug`|`release`), `input`
(true when drive actions exist), `version`, `startedAt`. The label is the `.app` folder name
(`Zeron Dev 7`, `Zeron Scenario`), or `Zeron` outside a bundle. Errors name a Windows endpoint
as `127.0.0.1:<port>` and never print its token.

`--target` (or `target`) rules, in order:

| Target | Picks |
|---|---|
| omitted, MCP tool | the app serving this chat (same engine `ipcPort`), release first, then the earliest started |
| `self` | the same as omitted; the CLI refuses it, because only the tool knows its port |
| omitted, CLI | the only live app; with two or more it errors |
| all digits | that pid |
| contains `/` | that exact unix socket path |
| any other text | an exact case-insensitive label, else a case-insensitive substring that matches one app |

Every miss or ambiguity error ends with `live targets (pid label build ipcPort):` and one line
per app. An agent that omits `target` reaches the app serving its own chat: `ui_driver`
observes it and `ui_driver_drive` refuses it. To drive, pass the label or pid of a Dev bundle
that runs its own engine port.

The wire format is newline-delimited JSON over the connection, after the token line on Windows:
request
`{"id":1,"action":"state","window"?:<id>,...}`, reply `{"id":1,"ok":<value>}` or
`{"id":1,"error":"<msg>"}`. The client sends one request per connection and waits for the
longest the request can take plus 10 s, and at least 30 s. That estimate adds the largest of
`timeoutMs`, `ms` and `quietMs` (at most 60000), `holdMs` (at most 60000), 100 ms per click,
drag step, keystroke or typed character, and the same for a nested `during`. A drive takes at
most 600 clicks (`count`), drag `steps`, keystrokes or typed characters, and `holdMs` up to
60000; the app refuses more. Closing the connection, including shutting down its write side,
stops a drive at its next step and releases a held drag button. A drive waits up to 5 s for
another drive in the same app to finish, then fails with `another drive is still running`.

## Launching a Dev bundle to drive

### `launch` and `quit` (first choice)

`zeron ui launch` (or `ui_driver_drive` with `"action":"launch"`) picks a free Dev number, builds
and opens that bundle with `scripts/run-macos-dev.sh`, and returns once the new bundle's driver
is live:

```sh
target/debug/zeron ui launch --mock
{"action":"launch","mock":true}
```

```json
{"label":"Zeron Dev 63","pid":69436,"port":49863,"root":"<target>/macos-dev63","log":"<target>/macos-dev63/launch.log","bundleId":"sh.zeron.app.dev63"}
```

| Arg | Default | Use |
|---|---|---|
| `n` (`--n N`) | the lowest free number | Dev number, 60-99; 80 is never picked on its own, since the scenario launcher uses port 49880 |
| `mock` (`--mock`) | false | sets `ZERON_HARNESS=mock` for the bundle |
| `name` (`--name <app>`) | `Zeron Dev <N>` | the `.app` name, which becomes the driver label |
| `appArgs` (after `--`) | none | an array of strings passed to the app |

- Number N gives port `49800+N`, bundle id `sh.zeron.app.dev<N>` and root
  `<target>/macos-dev<N>`, where `<target>` is `CARGO_TARGET_DIR` or the checkout's `target`.
  The checkout is the nearest folder above the current one (the CLI's, or `zeron mcp`'s for the
  tool) that holds `scripts/run-macos-dev.sh`; outside one, `launch` errors.
- A number is taken when its port is the calling chat's engine, a live driver app has that
  label, port or a bundle under that root, a process runs from that root, or something listens
  on the port. An explicit taken `n` errors with `Zeron Dev <N> is not free: <why>`; with none
  free, `every Dev number from 60 to 99 is in use`.
- The script's output goes to `<root>/launch.log`. While it builds, the client prints
  `launch: building and opening <name> (port <port>); log <path>` and then a progress line
  every 15 s to stderr. It waits up to 30 min. A script that exits first fails the call with the
  last 40 log lines.
- The script runs with the caller's environment. `launch` sets `ZERON_DEV_ROOT`,
  `ZERON_DEV_APP_NAME`, `ZERON_DEV_BUNDLE_ID` and `ZERON_DEV_IPC_PORT` from N and drops
  `ZERON_DEV_DATA_DIR`, so the data lives in `<root>/data`. The other variables in the table
  below, `ZERON_DEV_PROFILE` among them, and the passthrough list reach the script as set, so
  `ZERON_DEV_PROFILE=release zeron ui launch` launches a release Dev bundle.

`zeron ui quit --target "Zeron Dev 63"` (or `"action":"quit"` with a `target`) asks the bundle to
quit through its bundle id and waits up to 20 s for its descriptor to go:
`{"quit":"Zeron Dev 63","pid":48233,"bundleId":"sh.zeron.app.dev63"}`. It refuses the app on
port 27654 or under `/Applications` (`` `quit` never quits ... it hosts the running sessions ``),
an app outside a bundle, and any bundle whose id is the release `sh.zeron.app`.

### The script by hand

`scripts/run-macos-dev.sh` builds `zeron`, assembles and signs an isolated bundle, and opens it
through LaunchServices with `open -W`, which blocks until the app quits. Run it in the
background and poll `zeron ui list` until the label shows up (the first build can take minutes).

| Variable | Default | Use |
|---|---|---|
| `ZERON_DEV_ROOT` | `target/macos-dev` | bundle folder; relative paths resolve against the checkout root |
| `ZERON_DEV_APP_NAME` | `Zeron Dev` | `.app` name, which becomes the driver label |
| `ZERON_DEV_BUNDLE_ID` | `sh.zeron.app.dev` | LaunchServices and TCC identity |
| `ZERON_DEV_IPC_PORT` | `49777` | the bundle's own engine port |
| `ZERON_DEV_DATA_DIR` | `$ZERON_DEV_ROOT/data` | the bundle's own data (settings, chats, logs) |
| `ZERON_DEV_BUILD_ONLY=1` | unset | build and sign, do not launch |
| `ZERON_DEV_PROFILE` | `debug` | `release` builds and bundles the release profile, which is observe-only: use it to check a release build's tree, audit or frame timing |

Passed through to the app when set: every `ZERON_MOCK_*`, `ZERON_HARNESS`, `ZERON_FAKE_QUOTA`,
`ZERON_MOTION_SCALE`, `ZERON_OPEN_ROUTE`, `ZERON_UPDATE_PREVIEW`, `CODEX_EXECUTABLE`, and the
`ZERON_CUA_*` probes.
`ZERON_AUTO_UPDATE` is not passed, so an upstream release never replaces a Dev bundle.
Arguments after the script go to the app.

```sh
# In a worktree, build into its own target dir (AGENTS.md).
export CARGO_TARGET_DIR="$PWD/target"
lsof -nP -iTCP:49787 -sTCP:LISTEN          # must print nothing
mkdir -p target/macos-dev7
ZERON_DEV_ROOT=target/macos-dev7 ZERON_DEV_APP_NAME="Zeron Dev 7" \
ZERON_DEV_BUNDLE_ID=sh.zeron.app.dev7 ZERON_DEV_IPC_PORT=49787 ZERON_HARNESS=mock \
  scripts/run-macos-dev.sh > target/macos-dev7/launch.log 2>&1 &
target/debug/zeron ui list                 # repeat until "Zeron Dev 7" is listed
target/debug/zeron ui state --target "Zeron Dev 7"
osascript -e 'tell application id "sh.zeron.app.dev7" to quit'   # prints -128, still quits
```

Rules:

- Give every bundle its own root, name, bundle id and port. Other Dev bundles (another thread's
  `macos-dev82`, say) may be running, and the script refuses to rebuild a bundle that is
  already running.
- Never drive `/Applications/Zeron.app` and never use port 27654, which is the stable app's
  engine.
- `ZERON_HARNESS=mock` makes the mock harness the default for chats without a harness config
  and shows it in the pickers, so a test spends no model quota.
- When you finish a worktree, move `target/macos-dev<N>` to `~/.Trash` (AGENTS.md merge gate).

## Action reference

The examples below use a shell helper bound to the bundle from the previous section:

```sh
zui() { target/debug/zeron ui "$@" --target "Zeron Dev 7"; }
```

Under each example, the second line is the same request as tool arguments, for `ui_driver`
(observe) or `ui_driver_drive` (drive). Every
action also takes `window` (a window id from `state`; the default is the frontmost Shell window).

### Observe

| Action | Args (defaults) | Reply |
|---|---|---|
| `state` | none | `pid, build, input, windows[{id,title,kind,size{w,h},active,a11yForced}], route{kind,section,selectedChat}, frames{count,lastDrawAgoMs}, motionScale, theme{mode,appearance}, clock{paused,virtualMs}` |
| `tree` | `query`, `root` (ref), `depth`, `full` (false), `json` (false) | text lines, or `{window,title,size,focus,total,nodes[...]}` |
| `snapshot` | `rect {x,y,w,h}` in points, `path` (`zeron ui` only, so the read-only `ui_driver` tool never writes where the caller picks; absolute, ending in `.png`, not an existing link or folder; default `~/.zeron/run/shots/<pid>-<n>.png`) | `{path,width,height,stale?}` |
| `wait` | `for`, plus the fields below | depends on `for` |
| `frames` | `ms` (1000, max 10000), `during` | `{frames,fps,intervalsMs{p50,p95,max},longFrames[{atMs,gapMs}],firstFrameAtMs,lastFrameAtMs,drawMs?,during?}` |
| `track` | `ref` or `select`+`nth`, `ms` (800, max 5000), `jumpPx` (40), `during` | `{samples[[tMs,x,y,w,h]],framesSampled,missingFrames,jumps[{atMs,from,to,delta}],settledAtMs,reresolved,ref,node,framesDrawn,readMs,during?}` |
| `audit` | none | `{counts{nodes,interactive,tabStops,unlabeledInteractive},issues[{ref,rule,role,label?,bounds?,view?,source?}]}` |
| `record` | `do` (`start` or `stop`); with `start`, `includeDriven` (false, debug builds only); with `stop`, `path` (`zeron ui` only; absolute, ending in `.json`; default `~/.zeron/run/recordings/<pid>-<n>.json`) | start: `{recording,window,includeDriven}`; stop: `{path,steps,fallbacks,redacted}` |
| `compare` | `html` (the mockup page) or `path` (an `.html` or `.htm` file: in the current folder for `zeron ui`, in the chat's folder for `ui_driver`), one region (`rect`, `ref`, or `select`+`nth`), `width` (the region's), `threshold` (4 pt), `json` (false) | text lines, or `{matched,moved,missing,extra,uncheckedText?,region,pixel{ratio,path},stale?}` |

`route.kind` is one of `newChat`, `chat`, `pullRequests`, `usage`, `inbox`, `settings` (with
`section`, such as `"General"`), `terminal`, `tasks`, `taskCenter`, `supervisor`, `docs`.

```sh
zui state
{"action":"state","target":"Zeron Dev 7"}
zui tree '{"query":"settings"}'
{"action":"tree","query":"settings","target":"Zeron Dev 7"}
zui snapshot '{"rect":{"x":520,"y":200,"w":760,"h":170},"path":"/abs/out/crop.png"}'
{"action":"snapshot","rect":{"x":520,"y":200,"w":760,"h":170},"inline":false,"target":"Zeron Dev 7"}
zui wait '{"for":"select","select":"Button \"New chat\"","timeoutMs":3000}'
{"action":"wait","for":"select","select":"Button \"New chat\"","timeoutMs":3000,"target":"Zeron Dev 7"}
zui frames '{"ms":1500}'
{"action":"frames","ms":1500,"target":"Zeron Dev 7"}
zui track '{"select":"Button \"widgets\"","ms":1500}'
{"action":"track","select":"Button \"widgets\"","ms":1500,"target":"Zeron Dev 7"}
zui audit
{"action":"audit","target":"Zeron Dev 7"}
zui record start
{"action":"record","do":"start","target":"Zeron Dev 7"}
zui record stop --path /abs/out/bug.json
{"action":"record","do":"stop","target":"Zeron Dev 7"}
zui compare '{"select":"Group \"Settings\"","path":"mockups/general.html"}'
{"action":"compare","select":"Group \"Settings\"","path":"mockups/general.html","target":"Zeron Dev 7"}
```

### Drive (Dev bundles only)

| Action | Args (defaults) | Reply |
|---|---|---|
| `click` | target, `button` (`left`, `right`, `middle`), `count` (1), `modifiers` (`cmd`, `shift`, `alt`, `ctrl`) | `{frame}` |
| `move` | target | `{frame}` |
| `drag` | `from`, `to` (each `[x,y]`, `{x,y}`, `{ref}` or `{select,nth}`), `steps` (12), `holdMs` (0), `modifiers` | `{frame}` |
| `scroll` | target, `dx`, `dy` (pixels, 0), `trackpad` (false) | `{frame}` |
| `key` | `keys`: an array of strokes, or one string of space-separated strokes | `{frame,handled[bool per stroke]}` |
| `type` | `text`, optional target | `{frame,unhandled?}`: `unhandled` counts characters no handler took, such as with nothing focused |
| `press` | `ref` or `select`+`nth`, `do` (`click`), `value` (for `setValue`) | `{frame,ref}` |
| `action` | `name` (gpui action), `args` | `{frame}` |
| `set` | `theme` (`light`, `dark`, `system`), `size` (`{w,h}` or `[w,h]`), `motionScale` | `{frame,size,motionScale,theme}` |
| `clock` | `do` (`pause`, `resume`, `advance`), `ms` (for `advance`, at most 60000) | `{paused,virtualMs,frame,stale?}` |

"target" means exactly one of `ref`, `select` (+`nth`), or `x` and `y`. Mixing them, `x`
without `y`, or `nth` without `select` errors with `give one of ref, select, or x and y`. Any
element-addressed drive reply may also carry `"unsettled": true`, and any drive reply
`"stale": true` (see Actionability). A drive that reached a web page carries `"web"` instead of
`stale` (see Web pages).

The tool schemas give every property one JSON type: `keys` is an array, `from`/`to`
are objects, `size` is `{w,h}`. The CLI lines below use the short forms (`"cmd-k"`, `[x,y]`),
which the app also accepts.

```sh
zui click '{"select":"Button \"Settings\""}'
{"action":"click","select":"Button \"Settings\"","target":"Zeron Dev 7"}
zui move '{"x":23,"y":189}'
{"action":"move","x":23,"y":189,"target":"Zeron Dev 7"}
zui drag '{"from":[301,500],"to":[400,500]}'
{"action":"drag","from":{"x":301,"y":500},"to":{"x":400,"y":500},"target":"Zeron Dev 7"}
zui scroll '{"select":"Group \"Settings\"","dy":-600}'
{"action":"scroll","select":"Group \"Settings\"","dy":-600,"target":"Zeron Dev 7"}
zui key '{"keys":"cmd-k"}'
{"action":"key","keys":["cmd-k"],"target":"Zeron Dev 7"}
zui type '{"select":"MultilineTextInput \"mind\"","text":"hello"}'
{"action":"type","select":"MultilineTextInput \"mind\"","text":"hello","target":"Zeron Dev 7"}
zui press '{"select":"Switch \"Compact mode\"","do":"click"}'
{"action":"press","select":"Switch \"Compact mode\"","do":"click","target":"Zeron Dev 7"}
zui action '{"name":"shell::ToggleCommandPalette"}'
{"action":"action","name":"shell::ToggleCommandPalette","target":"Zeron Dev 7"}
zui set '{"theme":"dark","size":[1200,800]}'
{"action":"set","theme":"dark","size":{"w":1200,"h":800},"target":"Zeron Dev 7"}
zui clock '{"do":"advance","ms":50}'
{"action":"clock","do":"advance","ms":50,"target":"Zeron Dev 7"}
```

Input notes:

- `click` moves the pointer, waits a frame, then sends `count` down/up pairs.
- `scroll` sends one wheel event, or with `trackpad: true` a Started, six Moved and an Ended
  phase about 16 ms apart (that is enough to trigger swipe navigation with `dx`).
- `type` with no target key types into the focused element; a malformed target errors. With
  one, it focuses the element
  through accessibility when the node lists `focus`, and otherwise clicks its center first.
  `\n` becomes `enter`, which sends the composer; use `key "shift-enter"` for a newline.
- `key` strokes use gpui syntax: `cmd-k`, `shift-enter`, `escape`, `down down enter`.
  `handled` tells you whether anything consumed each stroke.
- `press` performs an AccessKit action on the node without moving the pointer. Names are
  case-insensitive (`click`, `focus`, `setValue`, `expand`, `collapse`, `increment`,
  `scrollIntoView`, ...). A node that does not list the action is refused, except `click`, for
  which gpui falls back to a synthetic click at the node's center.
- `action` dispatches a registered gpui action by name. An unknown name errors with
  `cannot build action ...`.
- `click`, `press` and `type` addressed by `ref` or `select` refuse a disabled element with
  `` `<selector as given>` is disabled: <its tree line> `` (on a web element,
  `` `#w1-4` is disabled: ... ``). Raw `x`,`y` is not checked, so a click there lands on a
  disabled control and does nothing.

### Addressing

Use `tree` first. Each line is
`<ref> <Role> ["label"] [value="..." | placeholder="..."] [desc="..."] [@x,y wxh] [focused] [selected] [expanded=bool] [toggled=...] [disabled] [[actions]] [web]`,
indented two spaces per level, with bounds in window points. `desc` prints only on a node with
no label. `disabled` marks a control gpui reports as disabled, and `web` a node read from a
page inside a web view (see Web pages):

```
window 4294967297 "Zeron Dev" 1502x903 nodes=23/23 focus=none
e1 Window "Zeron Dev"
  e2 TabList "Chat or Work" @865,52 170x46
    e3 Tab "Chat" @868,55 82x40 selected [click,focus]
  e12 Button "Home" @6,52 34x34 [click,focus]
  e22 Button "Settings" @6,858 34x34 toggled=false [click,focus]
```

- `query` keeps matching lines (role, label, value, description, placeholder) plus their
  ancestors. `root: "e12"` starts at a node, `depth: 0` prints only the start node. A query
  with no match prints only the header (`nodes=0/43`) and still exits 0, so read the count.
- Unlabeled generic containers and zero-size unlabeled nodes are hoisted out of the way;
  `full: true` prints them. `json: true` gives each node `disabled: true` and `web: true` when
  they hold (the keys are left out otherwise), and adds `view`, `elementId` and `source`
  (file:line). A debug build records them always; a release build only while the driver's
  lease forces its tree. Every driver read forces the tree and waits for a frame drawn after,
  so driver reads see them in both builds, while VoiceOver alone records nothing.
- `ref` (`"e12"` or `"#e12"`, or a web ref such as `"w1-3"`) names one node. A native ref is
  stable while the element keeps its AccessKit id.
- `select` grammar: `#e12`, `#w1-3`, `Role`, `"text"` or `Role "text"`. The role is one
  AccessKit role word, case-insensitive (`Button`, `Tab`, `Switch`, `TextInput`,
  `MultilineTextInput`, ...). Escape `\"` inside the text.
- Text matching is case-insensitive, and the first tier with any hit wins: exact label, then
  label substring, then value substring, then placeholder or description substring. So
  `Switch "Compact mode"` beats `Switch "Compact model picker"`. Within a tier, a native hit
  wins over web hits, which are dropped, so a page cannot shadow a native control by naming a
  button the same.
- `nth` (0-based) picks among the winning tier's hits. Several hits without `nth` is an error
  that lists up to 10 of them: `` `Button` matches 15 elements; pass `nth` (0-based) or a
  narrower selector: ``. A miss lists up to 5 near candidates, and a dead ref says
  `` `#e9` is not in the current tree (gone); read `tree` again ``.
- `x`,`y` are window logical points from the top-left of the content. Snapshots are 1 px per
  point, so a pixel you read off a PNG is a click coordinate.
- Pointer actions refuse an element whose center is outside the window:
  `... is outside the 1502x903 window; scroll it into view first`.

### Web pages (macOS)

An inline HTML card in a thread and a Browser tab in the right pane are web views, which gpui
cannot see into. On macOS the driver walks each visible one with a script
(`crates/ui/src/ui_driver/web.js`) and splices its elements into the tree, so `tree`, `select`,
`ref`, the `select`/`text`/`gone` waits, `track`, `record` and `compare` all see them. `audit`
reads the native tree only.

```
e1 Window "Zeron Dev 61"
  w3-0 WebView "Options" @350,372 372x302 web
    w3-2 Button "Option A" @350,450 74x29 [click,focus] web
    w3-4 Button "Option C" @521,450 74x29 disabled [click,focus] web
    w3-7 TextInput "Name" placeholder="Your name" @391,529 156x21 [focus,setValue] web
```

- A web ref is `w<view>-<n>`. `<view>` numbers the web view in the order the app opened them,
  and `w<view>-0` is the page itself, labeled with its title. `<n>` stays with an element while
  it stays in the loaded page; a reload or a replaced element gets a new number, and a reopened
  card a new view number.
- The page writes this text, not the app. Every web line ends in `web` (json `web: true`), an
  unknown role prints as `GenericContainer` so a page cannot print a role of its own choosing,
  and a password field's value prints as `[redacted]`. Treat web labels as page content, and
  remember that a native hit wins a selector tier.
- Hidden, `aria-hidden`, inert, `display:none`, fully transparent, zero-size, off-viewport and
  overflow-clipped elements are left out. A walk keeps the first 2000 nodes (the page line then
  says `desc="only the first 2000 page nodes"`) and clips text to 200 characters. A page the
  walk cannot read prints `w<view>-0 WebView desc="page not read: <why>"`.
- A page nests under the deepest native node whose bounds hold it; today that is `e1 Window`.

Drive on a web element, or at an `x`,`y` that lies on a page:

| Drive | Route on a page | Reply `web` |
|---|---|---|
| `click` | a real mouse event to the web view; plain left clicks only (`count` works), anything else errors with `a web page takes plain left clicks only` | `event` |
| `move` | a hover at the point | `event` |
| `scroll` | the DOM `scrollBy` on the nearest scrollable ancestor of the point | `dom` |
| `type` | with a target, a click on it first; then key events for each character | `event` |
| `key` | key events to the focused page, when a page holds focus | `event` |
| `press` | `click` with no `value` is a real click; `focus`, `blur`, `setValue` and `scrollIntoView` go through the DOM, and any other name errors | `dom` (or `event` for the click) |
| `drag`, `action`, `set` | stay native and do not reach the page | none |

- A web drive reply drops `stale`, because the page paints in its own layer, adds `ref` for an
  element target, and carries `web`.
- A point the app's chrome covers (part of the page lies under a pane or the composer) errors
  with `(x, y) is not on the live part of page w<view>: app chrome covers it there; scroll it into view first`.
- Clicks, hovers and keys reach the page as real AppKit events, so its handlers see
  `isTrusted: true`. `scroll` and the DOM `press` names change the page through script instead.

### Common surfaces

Labels as the tree reports them. Quote a prefix: `"Do anything"` matches `Do anything…`.

| Surface | Nodes |
|---|---|
| Sidebar | a chat row is `Button "<title>"`; its open tab is `Tab "<title>"` in `TabList "Sessions"`, so give the role. |
| Composer | `MultilineTextInput "What's on your mind?"` on New chat, `MultilineTextInput "Do anything…"` in a chat; `Button "Send message"`, which becomes `Button "Stop"` while a turn runs. |
| Command palette | `Dialog "Command palette"` with `SearchInput "Search threads, commands and files…"`, `TabList "Result type"`, and rows `ListBoxOption "<title>"` in `ListBox "Results"`. |
| Question card | `Group "<header>"` holding `RadioGroup "<question>"` with one `RadioButton "<option>"` each, `MultilineTextInput "Type your own answer…"` and `Button "Submit"`. On a card with one single-select question, clicking an option submits it. While a card is open the composer is not in the tree. |
| Transcript | `Group "You: <text>"`, the assistant's text as `Group "<text>"`, `Group "Question: Awaiting your answer…"` that becomes `Group "Question: <header>"` with `value="<answer labels>"` once answered, `Button "Thought process"`, `Button "Ran 1 command"` with a row per call such as `Button "Run ls palette"` (expanded, its body is `Group "Call"` and `Group "Output"`, `"Thought"` or `"Changes"` with the text as `value`), a message footer `Group "<time>"` with `Button "Copy message"` and `Button "Fork chat from here"` or `"Rewind to here"`, and on a finished turn `Button "Worked for 24s"` folding the turn. |
| Home | `TabList "New thread mode"`, a `Group "<card title>"` per widget (with `Meter` rows on This Mac), `Button "Hide widgets"` and `Button "Customize"`. |
| Sidecar | `Group "Live state"` with a section `Group` per heading (`"Side chats"` holds `Button "New sidechat"` and `Button "Fork chat"`), and `Button "Collapse live state"` or `"Expand live state"`. |

A turn has finished when the composer's `Button "Send message"` is back and `Button "Stop"` is
gone. Wait for the first, then the second: while a question card is open neither is in the
tree, so `gone` on Stop alone passes too early.

```sh
zui wait '{"for":"select","select":"Button \"Send message\"","timeoutMs":30000}'
zui wait '{"for":"gone","select":"Button \"Stop\""}'
```

### Actionability, frames, and the a11y lease

- An action addressed by `ref` or `select` first waits for the element to hold still. It
  re-resolves on every drawn frame and acts once two consecutive frames give the same bounds,
  or once no frame draws for 50 ms. After 1 s it acts anyway and adds `"unsettled": true`. Raw
  `x`,`y` skip this wait. An element that moves under about 0.5 pt per frame counts as still.
- Every drive reply returns after the next drawn frame following its last event (capped at
  500 ms); when no frame drew in that time it carries `"stale": true`. One frame is not a
  finished animation, so run `wait {"for":"idle"}` before you judge a transition by a snapshot
  or a tree read.
- Drive actions run one at a time across connections: a second drive call (or a `during`
  drive) waits until the first one replies. Observe actions run alongside each other and
  alongside drive.
- `wait` kinds: `frame` forces one redraw and returns `{frame}`; `idle` returns
  `{frame,quietMs}` once no frame has drawn for `quietMs` (300); `ms` sleeps `ms`; `select`
  returns `{ref,node,waitedMs}` once the element exists; `gone` takes an element (`ref` or `select`) or `text` and
  returns `{gone,waitedMs}` once nothing matches it; `text` returns `{ref,node,waitedMs}` once a label, value or description
  contains `text`. An empty `text` errors at once, since every node contains it. `timeoutMs`
  defaults to 5000. `timeoutMs`, `quietMs` and `ms` are at most 60000; a larger value errors
  at once. A timeout reports what it last saw, for example
  `not idle after 5000 ms: 120 frames drawn (24.0 fps), last 3 ms ago`.
- The tree exists only while a window is forced to build it. The first `tree`, `audit`,
  element target or element wait forces it, which costs one or two frames (15-17 ms live). The
  lease ends 60 s after the last use; `state.windows[].a11yForced` shows it. A read never
  answers from a tree older than the force, even when another call started the lease. When no
  frame draws for about 1.5 s after the force (a minimized or fully hidden window), it errors
  with `the window drew no frame after the accessibility tree was enabled`.
- `snapshot` forces one frame (30-130 ms) so the capture is current; when that frame never
  draws, the reply carries `"stale": true` and the PNG may show an older frame. A full window is
  about 150 KB of PNG against 1-2 KB for its `tree`. Prefer `tree` with `query`, and take a
  snapshot only for a visual check, cropped with `rect` to the node's bounds.

### Measuring motion: `frames`, `track`, `during`, `set`

- `frames` records draw timestamps from the Shell render tail for `ms` without forcing
  redraws. `longFrames` lists gaps over twice the median, and needs at least 3 intervals.
  `drawMs` is the time spent in `Window::draw`; a long gap with a small `drawMs` means the time
  went elsewhere on the main thread. The idle gap after an animation also shows up as a long
  frame, so read `firstFrameAtMs`/`lastFrameAtMs` for the active span.
- `track` reads one element's bounds on every drawn frame. Repeated samples collapse (the first
  and last stay). `missingFrames` counts frames where the element was absent.
  `settledAtMs` is when the final state began, and `reresolved` counts ref changes.
  `framesDrawn` above `framesSampled` means the sampler missed frames.
- A step is a jump when its per-frame change (the largest of |dx|, |dy|, |dw|, |dh|, divided by
  the frames between samples) exceeds `jumpPx`, or when it is over 4x the median moving step
  and over 4x both neighbouring steps. A smooth ease-out passes. A snap shows as one step far
  larger than its neighbours, such as `{"atMs":176,"from":[...,395,...],"to":[...,544,...],"delta":148.5}`.
- `jumpPx` is per frame. An inactive window runs at about 24-30 fps, so a smooth slide can
  trip 40. Check `state.windows[].active` before you trust a jump, or compare against
  `motionScale` 5.
- `during` takes one drive request (`{"action":"click",...}`) and starts it once recording
  has begun, so you measure the transition it causes. Its reply comes back under `during`.
  With `during`, `track` may start before its element exists (a palette row); a malformed or
  ambiguous selector, or a ref that is not in the tree, still errors before the action fires.
  `during: null` counts as no `during`.
- `set theme` goes through the same path as Settings > Appearance and is saved to the target's
  `ui-settings.json`; set it back to `system` when you finish. `size` resizes the window
  content and is not saved.
- `set motionScale` multiplies animation durations (1 is normal, 5 is five times slower;
  clamped to 0.01-100; `ZERON_MOTION_SCALE` seeds it at launch). It applies to animations
  started afterwards that read `motion::speed_scale()`: catalog `MotionSpec` animations, Shell
  width tweens, Home's grid reveal, the composer dock, pickers. It does not slow raw
  `Animation::new` sites, `MotionSpec::repeating()`, spinners and pulse loaders,
  `paint_animated_quad`, `DisclosureMotion`, or component tweens that compute their own
  duration. Changing it mid-flight can itself cause a jump in tweens that recompute per frame.
- `clock` stops and steps animation time for the whole app (gpui's motion clock, which
  `with_animation`, scene quad animations, GIF frames and Zeron's motion helpers read).
  `pause` freezes it, `advance` with `ms` moves it forward that much while paused, and `resume`
  lets it run again from where it stood. `advance` on a running clock errors with
  `` the clock is running; send `clock` with `do: pause` first ``. Each call redraws every window
  and replies after the next frame: `{"paused":true,"virtualMs":50,"frame":407}`, where
  `virtualMs` is the time advanced since the pause (0 while running). `state.clock` reports the
  same in every build. Pause, start the transition, then alternate `clock advance` and
  `snapshot` to see each stage (recipe g). Debounces, caret blink, toasts' auto-dismiss and
  network deadlines stay on wall time.

### `audit` rules

| Rule | Fires when |
|---|---|
| `unlabeled` | an interactive role has no label, value or description |
| `clickable-without-role` | a `GenericContainer`, `Unknown` or `Label` node lists `click` |
| `offscreen-interactive` | an interactive node lies fully outside the window (also fires for content scrolled out of a scroll view) |
| `duplicate-label` | siblings share a role and a label |
| `no-focus-node` | gpui focus is held but no node reports focus (carries `detail`, no `ref`) |

A disabled interactive node still counts in `interactive` but is exempt from `unlabeled`,
`offscreen-interactive` and `duplicate-label`. Issues carry `source` (file:line of the element), which is where you
fix them, in debug and release builds alike (see Addressing). `audit` reads the native tree
only, and it cannot see an element that has no node at all.

### Recording a session: `record`

`record start` taps the window's real clicks, drags, scrolls, keys and typing; `record stop`
writes them as a scenario file that `zeron ui scenario` replays (recipe h).

```sh
zeron ui record start --target "Zeron Dev 7"     # {"recording":true,"window":...,"includeDriven":false}
# the person reproduces the bug in that window
zeron ui record stop --path /abs/out/bug.json --target "Zeron Dev 7"
# {"path":"/abs/out/bug.json","steps":6,"fallbacks":0,"redacted":0}
zeron ui scenario /abs/out/bug.json --target "Zeron Dev 7"
```

- Steps name elements the way `select` does (`{"ui":"click","select":"Button \"Settings\""}`),
  with `nth` when the label is shared. A click also keeps `button`, `count` and `modifiers`; a
  pointer that moves past 4 pt while down becomes a `drag`; wheel events merge into one
  `scroll`, and typed characters into one `type`. When the tree changed since the last step,
  a `wait` for the next step's element goes in first, so a replay does not outrun the UI.
- An event with no labeled node under it records `x`,`y` instead, and the step's `name` says
  why (`x,y fallback: no node at the point`). `fallbacks` counts them; read those steps before
  you trust a replay on another window size.
- Redaction: text typed into a field whose role holds `password`, or whose label, placeholder
  or description contains `token`, `secret` or `password`, or the word `key` or `api`, is
  written as `"text":"<redacted>"`, and so is text typed while no tree could be read. A field's
  `value` is never used in a selector, so typed text cannot leak through one. `redacted` counts
  the redacted steps.
- Input the driver itself sends is left out, so an agent's drive does not end up in the
  person's recording. `includeDriven: true` (debug builds) keeps it.
- One recording runs per app at a time: a second `start` errors with
  `` a recording is already running; `record {"do":"stop"}` first ``, and `stop` with none
  running errors with `` no recording is running; `record {"do":"start"}` first ``. A `stop`
  whose write fails keeps recording, so stop again with another `path`.
- `ui_driver` refuses to record the app serving the calling chat (see Two levels); the CLI is
  the owner's path for that.

### Checking a build against its mockup: `compare`

`compare` lays the approved HTML mockup out in the target app's own web engine, at the
region's size (or `width`) and current theme, then matches its elements to the native nodes in
the region by role and label and diffs the pixels:

```sh
zui compare '{"select":"Group \"Settings\"","path":"mockups/general.html"}'
```

```
moved   Button "150K" dx=-40.5 dy=0 dw=0 dh=-0.5 native @517.5,347 72x33.5 mock @558,347 72x34
missing Button "Classify" mock @585,211 77x34 web
extra   Button "Classifier" native @584.5,210.5 77x33.5
compare 679x575 at @470,137: 14 matched, 1 moved, 1 missing, 1 extra (threshold 4 pt); 22 mock text nodes not compared: the native region exposes no text, the pixels cover it; 9.02% of pixels differ, highlighted in ~/.zeron/run/shots/44333-compare-2.png
```

- The region is one of `rect`, `ref`, or `select` (+`nth`). The node that is the region itself
  is not compared.
- Roles match by class: `Label`, `Paragraph`, `StaticText` and `Heading` count as one text
  class. Labels match case-insensitively with whitespace collapsed and a trailing `…` or `...`
  dropped. `moved` lists a matched pair whose x, y, w or h differs by more than `threshold`
  points, with `dx` = native minus mock; `missing` is in the mockup only, `extra` in the build
  only. A pair whose roles differ carries `mockRole` in json.
- Pixels: each channel may differ by 24 before a pixel counts. The highlighted PNG goes under
  `~/.zeron/run/shots`. Fonts and antialiasing differ between gpui and the web engine, so a
  faithful build still differs by a few percent; compare the ratio before and after a change,
  not against zero.
- `json: true` returns the lists as objects with both rects and the deltas.
- macOS only; elsewhere it errors with `` `compare` is only implemented on macOS ``.
- The mockup comes as `html` or as `path`, not both. `zeron ui compare` reads `path` from the
  current folder, never outside it. The `ui_driver` tool reads it from the chat's folder by
  `html_preview`'s rule (a chat in the home folder is refused) and only inside a live Zeron run,
  so the page never passes through the agent's context; otherwise pass `html`.

## html_preview speaks the same vocabulary

`html_preview` (a `zeron mcp` tool, next to `html_render`) lays a page out in Zeron's own web
engine without a window and returns a PNG, `contentHeight` and the console output. It shares
the driver's tree lines, selectors and step syntax, so a mockup can be checked the way the
build will be:

| Arg | Use |
|---|---|
| `html` or `path` | the page, one of them. `path` is an `.html` or `.htm` file in the chat's folder, relative to it or absolute; a file outside it, a non-HTML file and a chat that runs in the home folder are refused. Pages are at most 512000 bytes. |
| `width` | viewport width in CSS pixels, 240-1600 (736, the reply column; about 390 for a phone) |
| `appearance` | `dark` (default) or `light` |
| `tree` | true adds the page's elements as driver tree lines: a `page` header in place of `window`, refs `w1-<n>`, every line marked `web` |
| `rect` or `select` (+`nth`) | crops the screenshot: `rect {x,y,w,h}` in CSS pixels from the page's top-left, or the element a selector names |
| `steps` | up to 20, in order: `{"action":"click","select"\|"ref","nth"?}`, `{"action":"type","text","select"\|"ref"?}`, `{"action":"key","keys":"enter"}`, `{"action":"wait","for":"ms","ms"}` or `for` `select`, `text` or `gone` as in `wait` |

```json
{"html":"<!doctype html>...","width":480,"tree":true,"select":"Button \"Option B\"",
 "steps":[{"action":"click","select":"Button \"Option B\""},{"action":"wait","for":"text","text":"Picked: Option B"}]}
```

- The reply's content is the summary as JSON text (`width`, `contentHeight`,
  `capturedHeight`, `consoleMessages`, `crop`, `steps`), the screenshot, the tree when asked,
  then for each step a `step <i>: ...` text, its screenshot and its tree.
  `structuredContent` keeps the summary and the trees without the PNGs.
- Steps act through the DOM, since a windowless page takes no AppKit input, so the events a
  click step fires have `isTrusted: false`. A page that checks `isTrusted` behaves differently
  here than in the app, where `click` on a web node sends a real event (see Web pages). A
  disabled element refuses a step the way it refuses a drive, and a failed step returns its
  error and stops the rest.
- It needs the Zeron desktop app open on the same machine and a live Zeron run.

## Recipes

### (a) Verify a layout or copy change

1. Launch your Dev bundle: `zeron ui launch --mock` returns its label once it is up.
2. Navigate: `zui click '{"select":"Button \"Settings\""}'`, then
   `zui wait '{"for":"idle"}'`.
3. Check the copy and position as text: `zui tree '{"query":"compact"}'`. The line gives
   the label and `@x,y wxh`.
4. Check the pixels for that node only:
   `zui snapshot '{"rect":{"x":1180,"y":470,"w":300,"h":40},"path":"/abs/report/compact.png"}'`,
   then open the PNG. Save it under your topic folder, never in the source tree.

### (b) Reproduce a click-path bug

Write the path as selector steps and re-read state after each one:

```sh
zui key '{"keys":"cmd-k"}'
zui wait '{"for":"select","select":"SearchInput \"Search threads\""}'
zui click '{"select":"ListBoxOption \"New chat\""}'
zui state          # route.kind should be newChat
```

If the click lands on the wrong element, `track` the target during the step that opens it. A
first frame laid out 42 px off was exactly such a bug (the palette's `New chat` row, opened from
a chat route). Addressed actions now wait for still bounds, so a reply with `unsettled: true`
is itself a finding.

### (c) Measure a transition and catch a snap

```sh
zui state                          # windows[0].active should be true
zui track '{"select":"Button \"widgets\"","ms":1500,"during":{"action":"click","select":"Button \"Hide widgets\""}}'
zui set '{"motionScale":5}'
zui track '{"select":"Button \"widgets\"","ms":4000,"during":{"action":"click","select":"Button \"Show widgets\""}}'
zui set '{"motionScale":1}'
```

Read `jumps` first, then `samples` around each jump's `atMs`. A jump that persists at scale 5
is layout that flips in one frame, not a fast ease. Pair it with
`zui frames '{"ms":1500,"during":{...}}'` to see whether hitches come from draw time
(`drawMs`) or from work outside the draw.

### (d) Accessibility check

1. `zui audit` on each screen you changed (open popovers and dialogs too).
2. For each issue, open `source` (for example `crates/ui/src/app_rail.rs:93:5`) and fix it there.
   A node needs an id and a role, plus a label, the pattern `app_rail.rs` uses:
   `div().id("rail-settings").role(gpui::Role::Button).aria_label("Settings")`.
3. Compare `tree` against a snapshot of the same screen. Anything visible and clickable that
   has no line has no node. Add one the same way.
4. Re-run `audit` and `tree` and confirm the issue count dropped and the new nodes resolve by
   selector.

### (e) Script a realistic session: mock scripts and scenario files

`ZERON_MOCK_SCRIPT=<path>` makes the mock harness play scripted turns instead of its knob
output (`crates/harness/src/mock/script.rs`). Titles, commit messages and other one-shot calls ignore
the script.

- A `.json` step list is one turn, played on every run. A turn list
  `[{"match"?: "prompt substring", "steps": [...]}]` plays a matching turn, else the next
  unmatched turn in order per chat, staying on the last one once it runs out.
- Steps: any `AgentEvent` object (`textDelta`, `reasoningDelta`, `toolCall`, `toolResult`,
  `done`, ...), `{"sleep": ms}`, `{"ask": [questions]}` (a real question card),
  `{"echoAnswers": "prefix"}` (text with the labels picked in the last `ask`), and
  `{"awaitInterrupt": true}` (wait for Stop, end the turn interrupted). A turn with no `done`
  gets `Done{completed}` appended.
- A message sent while a turn runs is taken at the next step boundary as a `steered` event, with
  no scripted reply of its own. An `ask` holds it until the question is answered;
  `awaitInterrupt` takes it and keeps waiting for Stop.
- A `.jsonl` engine journal (`{"seq":n,"event":{...}}` lines) replays a recorded session,
  split into turns at each top-level `done`, with engine-written records dropped,
  `inputRequested` turned into an `ask` and `done.sessionId` cleared. Journals carry real
  paths and output; do not replay a private one in a shared demo.
- `ZERON_MOCK_DELAY_MS` and `ZERON_MOCK_CHARS` still pace the script; other knobs are ignored.

A scenario file seeds the engine, drives the UI, asserts and snapshots:

```json
{ "name": "pick-a-color", "steps": [
  { "engine": "LocalDevice", "save": "device" },
  { "ui": "key", "keys": "cmd-k" },
  { "ui": "wait", "for": "idle" },
  { "ui": "click", "select": "ListBoxOption \"New chat\"" },
  { "ui": "state", "save": "home" },
  { "assert": { "saved": "home.route.kind", "equals": "newChat" } },
  { "assert": { "select": "MultilineTextInput \"Type your own answer\"", "exists": false } },
  { "snapshot": "home" }
] }
```

The file is `{"name": str, "target"?: pid|label, "steps": [step]}`, and a step has exactly one
of these keys:

| Step | Effect |
|---|---|
| `{"engine": "<RPC method>", "params"?: {...}, "first"?: bool, "save"?: "n"}` | Calls the target's engine at `ws://127.0.0.1:<ipcPort>` over one connection per run. With `first`, it subscribes and keeps the first item, e.g. `WatchDocMessages {chatId}` returns `{reset: [entries]}`. A scenario with an engine step refuses a release-build target. |
| `{"ui": "<driver action>", ...args, "save"?: "n"}` | Sends one driver request, as `zeron ui <action>` would. |
| `{"assert": {"select": "Role \"label\"", "nth"?: n, "exists"?: bool}}` | `true` waits for a match (`wait for:select`); `false` waits for none (`wait for:gone`, which ignores `nth`). |
| `{"assert": {"text": "substring", "exists"?: bool}}` | `true` waits for a label, value or description that contains it; `false` waits until none does (`wait for:gone` with `text`). |
| `{"assert": {"saved": "n.path", "equals": json}}` | Compares a saved value. `"contains": "s"` searches its text instead (a JSON value is searched as compact JSON). |
| `{"snapshot": "label"}` | Writes `shots/NN-label.png`. |
| `{"sleep": ms}` | Waits, up to 60000 ms. |

- `select` and `text` asserts take `timeoutMs` (default 3000). An assert takes exactly one of
  `select`, `text` or `saved`, and no other keys than the ones above.
- A `save` name has no dots and is not `env`, `target` or `run`.
- Any step may set `"name"` for the report, and `"continueOnFail": true`. Without it, the first
  failure skips the remaining steps.
- Strings in `params`, args and asserts may hold `${n.path.0.field}`: a saved result, `target`
  (the app descriptor), `run.out`, `run.shots`, or `${env.NAME}`. A string that is exactly one
  reference keeps the JSON type of the value. A reference that does not resolve fails the step.
- A mock chat is created with `config.harness: "mock"`.

```sh
scripts/ui-scenario.sh scripts/scenarios/mock-question.json --mock-script scripts/scenarios/mock-question.mock.json
scripts/ui-scenario.sh scripts/scenarios/journal-replay.json --mock-script scripts/scenarios/journal-replay.jsonl
```

The launcher builds a fresh `target/ui-scenario/bundle/Zeron Scenario.app`
(`sh.zeron.app.dev.scenario`, port 49880 or `--port`, `ZERON_HARNESS=mock`, empty data dir, a
`git init`ed project exported as `ZERON_SCENARIO_PROJECT`), runs the scenario and quits the
bundle unless you pass `--keep`. Each run works in `target/ui-scenario/<name>/` and moves the
previous run's folder to the Trash. It refuses to start while a `Zeron Scenario` app is still
running (quit one kept with `--keep` first) or while a live app's data dir lies in that folder,
and it only stops a pid whose executable is inside the scenario bundle. Against an app that is
already running, use `zeron ui scenario <file> --target <t> --out <dir>`; without `--out` the
report goes to `~/.zeron/run/scenarios/<name>-<time>/`.

Reading the report: the last stdout line is `PASS n/n` or `FAIL k/n: step <i> <name>: <error>`
(k counts passed steps), and the exit code is 0 or 1. `report.md` has a row per step with its
result or error and links each snapshot; `report.json` holds the same steps (`index, kind,
name, status, ms, result, error, warning, shot`). After a failure the rest are `skipped` unless
the failing step set `continueOnFail`. A `ui` step whose reply says `unsettled: true` or has a
`false` in `handled` still passes, with a `warning` (shown first in its `report.md` cell); read
those as findings. Open the snapshots: a step can pass while the screen is wrong.

### (f) Check a build against its mockup

1. Keep the approved mockup as an `.html` file in the checkout (or the topic folder you run
   from), and check it alone first with `html_preview` and `tree: true`, so the roles and
   labels you expect are in it.
2. Launch the build, open the screen, and find the region's node:
   `zui tree '{"query":"settings"}'`.
3. `zui compare '{"select":"Group \"Settings\"","path":"mockups/general.html"}'` from the
   folder that holds `mockups/`. From the `ui_driver` tool, `path` is relative to the chat's
   folder.
4. Read `moved` first: each line gives `dx`/`dy`/`dw`/`dh` in points and both rects. Then
   `missing` and `extra`: a label that differs by a word (`Classify` against `Classifier`)
   shows as one of each. Open the highlighted PNG for what the node lists cannot say (text,
   color, icons).
5. Fix, rebuild, and compare again until the lists are empty and the pixel ratio stops
   dropping.

### (g) Step a transition with the clock

```sh
zui clock '{"do":"pause"}'
zui click '{"select":"Button \"Customize\""}'
zui snapshot '{"rect":{"x":900,"y":60,"w":290,"h":500},"path":"/abs/out/t000.png"}'
zui clock '{"do":"advance","ms":50}'
zui snapshot '{"rect":{"x":900,"y":60,"w":290,"h":500},"path":"/abs/out/t050.png"}'
zui clock '{"do":"advance","ms":50}'
zui snapshot '{"rect":{"x":900,"y":60,"w":290,"h":500},"path":"/abs/out/t100.png"}'
zui clock '{"do":"resume"}'
zui wait '{"for":"idle"}'
```

The `rect` is the popover's bounds, read from `tree` on an earlier open. Each snapshot is one
exact instant of the animation, whatever the machine's frame rate. Lay the
PNGs side by side to see a frame that skips a stage or lands out of place, and `track` the
same node across `advance` calls for numbers. Resume when you finish: the clock stays paused
until you do. While paused, a tween that keeps requesting frames can stop `wait idle` from
settling, so snapshot after `advance` instead of waiting for idle.

### (h) Hand a bug over by recording

1. The person runs `zeron ui record start --target <app>` in their own shell (or you start it
   on a Dev bundle), reproduces the bug, and runs `zeron ui record stop --path /abs/bug.json`.
2. Read the file: it is a scenario, one step per click, drag, scroll, key or typed run, with
   a `wait` before a step whose screen changed. Check `fallbacks` and the `x,y fallback`
   names; replace a fallback with a selector if you can, since a raw point depends on the
   window size.
3. Replay it on a Dev bundle: `zeron ui scenario /abs/bug.json --target "Zeron Dev 7"`. Add
   `assert` and `snapshot` steps at the point the bug shows, so the file becomes the bug's
   repro and, after the fix, its check.
4. Secret fields arrive as `<redacted>`; put test values in before you replay.

## Agent as user: handing QA to a child agent

A lead agent can hand a QA pass to a child agent that acts only as a user would. Give the child:

1. this guide (`docs/ui-driver.md`) and nothing about the implementation;
2. the target label (`Zeron Dev 7`) and the rule that it acts only through `zeron ui` or the
   `ui_driver` and `ui_driver_drive` tools with that `target`, never by editing code, never
   through computer use, and
   never against `/Applications/Zeron.app`;
3. the goal as a user story ("open Settings, turn on Compact mode, confirm the sidebar
   switches to compact rows, turn it back off");
4. where to write: a report path under `maestro topic path <slug>`.

Ask it to return, in that report:

- each step it ran, as the exact command or tool arguments, with the reply or error;
- `tree` excerpts (with `query`) that prove each state it claims, plus the final `state`;
- snapshot paths for the visual claims, cropped to the relevant node;
- for motion, the `track`/`frames` replies;
- anything it could not reach by selector (no node), with the `x`,`y` fallback it used;
- the scenario report path when it ran one.

The lead then checks the claims against the excerpts and snapshots, not against the summary.

## Limits

- Elements without a11y nodes cannot be addressed by ref or selector, and `select`/`text`
  waits cannot see them. Most of the app has nodes (see Common surfaces). Still without: the
  composer's "New sidechat" tab, and the right pane's Browser, Terminal, Device and New
  sidechat rows. Read those from a snapshot, fall back to `x`,`y` for clicks, and file the
  gap. The fix is an id, a role and a label on the element (recipe d).
- Transcript row bounds include the row padding, and an open question row spans the card below
  it (937x396 with the card inside its lower part). Address a card's controls by selector, never
  by a row's center.
- Refs re-key when an element is rebuilt with a new AccessKit id (the palette search field gets
  a new ref each time it opens), and a web ref when its page reloads or replaces the element.
  Re-resolve by selector after any view rebuild.
- Web pages: nodes, web drives and `compare` exist on macOS only. Every page nests under
  `e1 Window` today, so `root` cannot narrow a read to one page's native container; use
  `root: "w3-0"`. A page takes only plain left clicks, and `drag`, `action` and `set` do not
  reach it.
- `html_preview` steps run through the DOM, so a page that checks `isTrusted` sees `false`
  there but `true` from a driver click in the app.
- `compare`: gpui exposes no text nodes inside most controls, so mock text without a native
  counterpart is checked by pixels only (`uncheckedText` counts it). A region that is a single
  control compares no nodes; give the region that holds the controls. A faithful build still
  differs by a few percent of pixels.
- `clock` moves animation time only. Gesture timing in swipe navigation reads the same clock,
  so a swipe during a pause freezes, and input timing (debounces, caret blink) stays on wall
  time. A tween held mid-flight may keep requesting frames, so `wait idle` may not settle
  while paused.
- `snapshot` reads the window's own pixels on macOS, through `PrintWindow` on Windows, and
  through X11 on Linux. Under Wayland it errors (`` `snapshot` cannot read window pixels under
  Wayland ... ``); run Zeron under X11 or Xwayland (unset `WAYLAND_DISPLAY`).
- `launch` and `quit` run `scripts/run-macos-dev.sh`, `open`, `plutil` and `osascript`, so they
  work on macOS only.
- A release build records `view`, `elementId` and `source` only while the driver's lease forces
  its tree; every driver read forces it, but a tree that VoiceOver alone keeps on has none.
- Each connection is served by one thread and runs its requests in order. The CLI and the tool
  open one connection per call, so separate observe calls run concurrently (a `wait` in one, a
  `key` in another), while drive calls queue behind each other. To act at a moment inside a
  measurement, use `during`.
- Frame stamps and `track` assume one Shell window; the frame counter is global across windows.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `zeron ui list` prints `[]` | No live app serves the driver. The bundle is still building (check its launch log), it crashed (stale descriptors are pruned by pid), or the app's build predates the driver. |
| `no running Zeron app serves the UI driver: ... has no live ui-<pid>.json` | Same as above, reported at target selection. |
| `N Zeron apps match ...` | Several apps are live; pass `--target` with the pid or exact label from the listed targets. |
| `` `click` drives the UI and exists only in debug builds `` | You reached a release build. Target a Dev bundle. |
| `` `click` drives the UI, which ui_driver never does `` | Drive actions and `during` go through `ui_driver_drive`, with a Dev bundle as `target`. |
| `` ui_driver_drive takes a drive action ... `` | The action only observes; call `ui_driver`. |
| `` ui_driver_drive does not drive <label> (pid n): it serves this chat's engine `` | You omitted `target` or named the app that serves your chat. Target a Dev bundle that runs its own engine. |
| `` `<sel>` matches N elements `` | Narrow with a role, use the exact label, or pass `nth`. |
| `no element matches ...; near:` | Read the near list or `tree` with `query`; the element may lack a node. |
| `... is outside the WxH window; scroll it into view first` | Scroll its container (`scroll` with `select` on the scroll area), then retry. |
| `timed out after N ms; last seen: ...` / `not idle after ...` | The UI never reached the state, or an animation or spinner keeps drawing. Read the last-seen part, raise `timeoutMs`, or wait on an element instead of `idle`. |
| `<label> (pid n) did not answer within N s` | The main thread is blocked, or a drive call waited behind another long drive. Check `state` from a second call. The timed-out drive stops when the app sees the connection close, so a retry does not repeat it. |
| `` `timeoutMs` must be at most 60000 ms `` | Waits are capped at 60 s. Wait on an element or for `idle` in steps instead of one long wait. |
| `` `text` must not be empty `` | A `text` or `gone` wait (or a scenario `text` assert) got `""`, which every node contains. Pass the text to wait for. |
| `user rejected MCP tool call` on `ui_driver_drive` in a Codex chat | The person answered No to "Approve UI drive" (the chat is at Ask or Auto). Ask them, or drive from a chat at Full. |
| `the window drew no frame after the accessibility tree was enabled` | The window is minimized or fully hidden, so gpui does not draw. Bring it back on screen and retry. |
| `give one of ref, select, or x and y` | The request mixed target kinds, or gave `x` without `y` or `nth` without `select`. |
| `` `path` must end in .png `` / `is not a regular file` | Snapshot paths are absolute `.png` files; pick a new file name, not a link or folder. |
| `` ui_driver never writes where the caller picks, so ... takes no `path` `` | Drop `path`: the tool saves a snapshot under `~/.zeron/run/shots` and returns the PNG, or its path with `inline: false`, and a recording under `~/.zeron/run/recordings`. Only `zeron ui snapshot` and `zeron ui record` take `path`. |
| `` `<sel>` is disabled: <line> `` | `click`, `press` or `type` named a control that reads `disabled`. Bring the UI to the state that enables it (pick an answer before Submit), or check that it should be disabled. |
| `` `launch` starts or quits a Zeron Dev bundle, which ui_driver never does `` | `launch` and `quit` go through `ui_driver_drive` or `zeron ui`. |
| `Zeron Dev <N> is not free: <why>` / `every Dev number from 60 to 99 is in use` | Omit `n` to take the lowest free number, or quit a bundle you no longer need. |
| `<script> exited (...) before <name> came up` | The build or the bundle failed; the error ends with the last 40 lines of `<root>/launch.log`. |
| `` `quit` never quits ... it hosts the running sessions `` | `quit` stops Dev bundles only, never the stable app or the release bundle id. |
| `ui_driver does not record <label> (pid n): it serves this chat's engine` | Ask the person to run `zeron ui record` themselves, or record a Dev bundle. |
| `` a recording is already running `` / `` no recording is running `` | One recording per app: stop the running one first, or start one before `stop`. |
| `` the clock is running; send `clock` with `do: pause` first `` | `advance` works on a paused clock only. |
| `a web page takes plain left clicks only` | Right, middle and modifier clicks do not reach a page; use `press` with a DOM action, or a plain click. |
| `(x, y) is not on the live part of page w<n>: ...` | App chrome covers that part of the page; scroll the page (`scroll` on the page) until the element is in view. |
| `the UI driver refused this connection: its first line must be the token ...` | Windows only: the client did not send the descriptor's `token` first. Use `zeron ui` or the tools, or send the token line before the request. |
| `` `snapshot` cannot read window pixels under Wayland `` | Run Zeron under X11 or Xwayland (unset `WAYLAND_DISPLAY`). |
| `` `compare` is only implemented on macOS `` | Compare on a Mac build. |
| `` ui_driver `compare` reads `path` only in a live Zeron run; pass `html` `` | The tool runs outside a Zeron chat, so it has no chat folder to read from. Pass the page as `html`, or run `zeron ui compare` from the mockup's folder. |
| `` give `html` or `path`, one of them `` | `compare` got both; drop one. |
| Little Snitch prompts for an unsigned dev binary | Expected for `target/debug` and ad-hoc signed bundles; allow the rule for any process with no code requirement, or the prompt returns after each rebuild. |
