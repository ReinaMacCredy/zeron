# UI driver: verify and debug Zeron UI changes as an agent

Every headed Zeron process has an in-process UI driver. It reads the window as text, acts on
elements, waits for the UI to change, measures animation frames and takes snapshots, without
computer use. It runs on the gpui main thread and sees what gpui sees: the AccessKit tree, the
Shell route, the frame clock and the window's own pixels.

Use it for anything inside a Zeron window: layout and copy checks, click paths, transitions,
accessibility, scripted sessions. Keep computer use for other apps, for paths the driver skips
on purpose (real trackpad and IME input, TCC prompts, the window server's event stream), and
for testing computer use itself.

## Two levels

| Level | Actions | Builds | MCP tool |
|---|---|---|---|
| Observe | `state`, `tree`, `snapshot`, `wait`, `frames`, `track`, `audit` | every build, the stable app included | `ui_driver` |
| Drive | `click`, `move`, `drag`, `scroll`, `key`, `type`, `press`, `action`, `set`, plus `during` on `frames`/`track` | debug builds only (Zeron Dev bundles) | `ui_driver_drive` |

Drive code is compiled out of release builds. A release build answers a drive request with:
`click` drives the UI and exists only in debug builds (Zeron Dev bundles); this build is
observe-only. A `frames` or `track` with a drive `during` gets the same refusal for `during`,
after ``invalid `frames` request:``. In every build, a `during` whose action is not a drive
action gets `` `during` must be a drive request (click, ...) ``. To drive, launch a Dev
bundle (next sections). The stable app in `/Applications/Zeron.app` hosts the person's
running sessions, so observe it only. It serves the driver only once it runs a build that
includes it.

What each level exposes:

- Observe reads what the app's windows show, text field values included (a key typed into
  the Add provider form is in `tree` and in a `snapshot`), to any chat that has the `ui_driver`
  tool on. Turn the tool off in Settings > Tools to withhold it.
- Drive in a Dev bundle goes around the chat permission gates of that bundle: an agent can click
  Approve on a pending request there, or type into its terminal panel. Keep Dev bundles on test
  data.
- `ui_driver_drive` refuses the app that serves the calling chat (any app whose `ipcPort` is the
  chat's engine port), so a chat cannot approve its own requests through the tool. `zeron ui`
  and the raw socket have no such check: an agent with a shell can still reach its own app.
  Turn `ui_driver_drive` off in Settings > Tools to keep reading without driving; turning
  `ui_driver` off turns both off.
- `ui_driver_drive` carries a destructive hint, so Codex would ask before every call. Zeron's
  Codex thread config pre-approves it, as it does `device_close`
  (`mcp_servers.zeron.tools.<tool>.approval_mode = "approve"`), so a Codex chat drives without
  a prompt in Ask, Auto and Full alike.

## Reaching a target

Each process listens on `~/.zeron/run/ui-<pid>.sock` (mode 0600, directory 0700) and writes a
descriptor `~/.zeron/run/ui-<pid>.json`. A clean quit removes both. A crash or SIGKILL leaves
them, and the client prunes any descriptor whose pid is gone or whose socket refuses.

Two clients share one implementation (`crates/mcp/src/ui_driver.rs`):

- CLI: `zeron ui list`, `zeron ui <action> ['<json args>'] [--target <t>]`, and
  `zeron ui scenario <file.json> [--target <t>] [--out <dir>]`. `zeron` is not on PATH in a
  checkout. Run `cargo build -p zeron` once, then call `$CARGO_TARGET_DIR/debug/zeron` (or
  `target/debug/zeron`). Any build of the CLI can talk to any app. Text replies (`tree`) print
  raw and everything else prints as pretty JSON. Errors go to stderr with exit code 1.
- MCP tools in `zeron mcp` (Settings > Tools, Workspace group): `ui_driver` takes `list` and the
  observe actions, `ui_driver_drive` the drive actions plus `frames` and `track` with `during`.
  Each refuses the other's actions with an error that names the right tool. The arguments are
  the CLI JSON plus `"action"` and an optional `"target"`. `snapshot` also returns the PNG as an
  image block unless `"inline": false`.

`zeron ui list` prints the live descriptors: `pid`, `socket`, `label`, `bundlePath`,
`dataDir`, `ipcPort`, `build` (`debug`|`release`), `input` (true when drive actions exist),
`version`, `startedAt`. The label is the `.app` folder name (`Zeron Dev 7`, `Zeron Scenario`),
or `Zeron` outside a bundle.

`--target` (or `target`) rules, in order:

| Target | Picks |
|---|---|
| omitted, MCP tool | the app serving this chat (same engine `ipcPort`), release first, then the earliest started |
| `self` | the same as omitted; the CLI refuses it, because only the tool knows its port |
| omitted, CLI | the only live app; with two or more it errors |
| all digits | that pid |
| contains `/` | that exact socket path |
| any other text | an exact case-insensitive label, else a case-insensitive substring that matches one app |

Every miss or ambiguity error ends with `live targets (pid label build ipcPort):` and one line
per app. An agent that omits `target` reaches the app serving its own chat: `ui_driver`
observes it and `ui_driver_drive` refuses it. To drive, pass the label or pid of a Dev bundle
that runs its own engine port.

The wire format is newline-delimited JSON over the socket: request
`{"id":1,"action":"state","window"?:<id>,...}`, reply `{"id":1,"ok":<value>}` or
`{"id":1,"error":"<msg>"}`. The client sends one request per connection and waits for the
longest the request can take plus 10 s, and at least 30 s. That estimate adds the largest of
`timeoutMs`, `ms` and `quietMs` (at most 60000), `holdMs`, 100 ms per drag step or typed
character, and the same for a nested `during`.

## Launching a Dev bundle to drive

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

Passed through to the app when set: every `ZERON_MOCK_*`, `ZERON_HARNESS`,
`ZERON_MOTION_SCALE`, `ZERON_OPEN_ROUTE`, `CODEX_EXECUTABLE`, and the `ZERON_CUA_*` probes.
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
| `state` | none | `pid, build, input, windows[{id,title,kind,size{w,h},active,a11yForced}], route{kind,section,selectedChat}, frames{count,lastDrawAgoMs}, motionScale, theme{mode,appearance}` |
| `tree` | `query`, `root` (ref), `depth`, `full` (false), `json` (false) | text lines, or `{window,title,size,focus,total,nodes[...]}` |
| `snapshot` | `rect {x,y,w,h}` in points, `path` (`zeron ui` only, so the read-only `ui_driver` tool never writes where the caller picks; absolute, ending in `.png`, not an existing link or folder; default `~/.zeron/run/shots/<pid>-<n>.png`) | `{path,width,height,stale?}` |
| `wait` | `for`, plus the fields below | depends on `for` |
| `frames` | `ms` (1000, max 10000), `during` | `{frames,fps,intervalsMs{p50,p95,max},longFrames[{atMs,gapMs}],firstFrameAtMs,lastFrameAtMs,drawMs?,during?}` |
| `track` | `ref` or `select`+`nth`, `ms` (800, max 5000), `jumpPx` (40), `during` | `{samples[[tMs,x,y,w,h]],framesSampled,missingFrames,jumps[{atMs,from,to,delta}],settledAtMs,reresolved,ref,node,framesDrawn,readMs,during?}` |
| `audit` | none | `{counts{nodes,interactive,tabStops,unlabeledInteractive},issues[{ref,rule,role,label?,bounds?,view?,source?}]}` |

`route.kind` is one of `newChat`, `chat`, `pullRequests`, `usage`, `inbox`, `settings` (with
`section`, such as `"General"`), `terminal`, `tasks`, `taskCenter`, `docs`.

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
```

### Drive (Dev bundles only)

| Action | Args (defaults) | Reply |
|---|---|---|
| `click` | target, `button` (`left`, `right`, `middle`), `count` (1), `modifiers` (`cmd`, `shift`, `alt`, `ctrl`) | `{frame}` |
| `move` | target | `{frame}` |
| `drag` | `from`, `to` (each `[x,y]`, `{x,y}`, `{ref}` or `{select,nth}`), `steps` (12), `holdMs` (0), `modifiers` | `{frame}` |
| `scroll` | target, `dx`, `dy` (pixels, 0), `trackpad` (false) | `{frame}` |
| `key` | `keys`: an array of strokes, or one string of space-separated strokes | `{frame,handled[bool per stroke]}` |
| `type` | `text`, optional target | `{frame}` |
| `press` | `ref` or `select`+`nth`, `do` (`click`), `value` (for `setValue`) | `{frame,ref}` |
| `action` | `name` (gpui action), `args` | `{frame}` |
| `set` | `theme` (`light`, `dark`, `system`), `size` (`{w,h}` or `[w,h]`), `motionScale` | `{frame,size,motionScale,theme}` |

"target" means exactly one of `ref`, `select` (+`nth`), or `x` and `y`. Mixing them, `x`
without `y`, or `nth` without `select` errors with `give one of ref, select, or x and y`. Any
element-addressed drive reply may also carry `"unsettled": true`, and any drive reply
`"stale": true` (see Actionability).

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

### Addressing

Use `tree` first. Each line is
`<ref> <Role> ["label"] [value="..." | placeholder="..."] [desc="..."] [@x,y wxh] [focused] [selected] [expanded=bool] [toggled=...] [[actions]]`,
indented two spaces per level, with bounds in window points:

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
  `full: true` prints them. `json: true` adds `view`, `elementId` and `source` (file:line) in
  debug builds.
- `ref` (`"e12"` or `"#e12"`) names one node. A ref is stable while the element keeps its
  AccessKit id.
- `select` grammar: `#e12`, `Role`, `"text"` or `Role "text"`. The role is one AccessKit role
  word, case-insensitive (`Button`, `Tab`, `Switch`, `TextInput`, `MultilineTextInput`, ...).
  Escape `\"` inside the text.
- Text matching is case-insensitive, and the first tier with any hit wins: exact label, then
  label substring, then value substring, then placeholder or description substring. So
  `Switch "Compact mode"` beats `Switch "Compact model picker"`.
- `nth` (0-based) picks among the winning tier's hits. Several hits without `nth` is an error
  that lists up to 10 of them: `` `Button` matches 15 elements; pass `nth` (0-based) or a
  narrower selector: ``. A miss lists up to 5 near candidates, and a dead ref says
  `` `#e9` is not in the current tree (gone); read `tree` again ``.
- `x`,`y` are window logical points from the top-left of the content. Snapshots are 1 px per
  point, so a pixel you read off a PNG is a click coordinate.
- Pointer actions refuse an element whose center is outside the window:
  `... is outside the 1502x903 window; scroll it into view first`.

### Common surfaces

Labels as the tree reports them. Quote a prefix: `"Do anything"` matches `Do anything…`.

| Surface | Nodes |
|---|---|
| Sidebar | a chat row is `Button "<title>"`; its open tab is `Tab "<title>"` in `TabList "Sessions"`, so give the role. |
| Composer | `MultilineTextInput "What's on your mind?"` on New chat, `MultilineTextInput "Do anything…"` in a chat; `Button "Send message"`, which becomes `Button "Stop"` while a turn runs. |
| Command palette | `Dialog "Command palette"` with `SearchInput "Search threads, commands and files…"`, `TabList "Result type"`, and rows `ListBoxOption "<title>"` in `ListBox "Results"`. |
| Question card | `Group "<header>"` holding `RadioGroup "<question>"` with one `RadioButton "<option>"` each, `MultilineTextInput "Type your own answer…"` and `Button "Submit"`. On a card with one single-select question, clicking an option submits it. While a card is open the composer is not in the tree. |
| Transcript | `Group "You: <text>"`, the assistant's text as `Group "<text>"`, `Group "Question: Awaiting your answer…"` that becomes `Group "Question: <header>"`, `Button "Thought process"`, `Button "Ran 1 command"` with a row per call such as `Button "Run ls palette"`, and on a finished turn `Button "Worked for 24s"` folding the turn. |

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

### `audit` rules

| Rule | Fires when |
|---|---|
| `unlabeled` | an interactive role has no label, value or description |
| `clickable-without-role` | a `GenericContainer`, `Unknown` or `Label` node lists `click` |
| `offscreen-interactive` | an interactive node lies fully outside the window (also fires for content scrolled out of a scroll view) |
| `duplicate-label` | siblings share a role and a label |
| `no-focus-node` | gpui focus is held but no node reports focus (carries `detail`, no `ref`) |

Issues carry `source` (file:line of the element) in debug builds, which is where you fix them.
`audit` cannot see an element that has no node at all.

## Recipes

### (a) Verify a layout or copy change

1. Launch your Dev bundle (above) and wait for `zeron ui list` to show it.
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
| `{"engine": "<RPC method>", "params"?: {...}, "first"?: bool, "save"?: "n"}` | Calls the target's engine at `ws://127.0.0.1:<ipcPort>` over one connection per run. With `first`, it subscribes and keeps the first item, e.g. `WatchDocMessages {chatId}` returns `{reset: [entries]}`. |
| `{"ui": "<driver action>", ...args, "save"?: "n"}` | Sends one driver request, as `zeron ui <action>` would. |
| `{"assert": {"select": "Role \"label\"", "nth"?: n, "exists"?: bool}}` | `true` waits for a match (`wait for:select`); `false` waits for none (`wait for:gone`, which ignores `nth`). |
| `{"assert": {"text": "substring", "exists"?: bool}}` | `true` waits for a label, value or description that contains it; `false` waits until none does (`wait for:gone` with `text`). |
| `{"assert": {"saved": "n.path", "equals": json}}` | Compares a saved value. `"contains": "s"` searches its text instead (a JSON value is searched as compact JSON). |
| `{"snapshot": "label"}` | Writes `shots/NN-label.png`. |
| `{"sleep": ms}` | Waits. |

- `select` and `text` asserts take `timeoutMs` (default 3000).
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
  waits cannot see them. The sidebar (rows, group headers, search, filter), the tab strip, the
  composer and its picker chips and popovers, the command palette and Add project lists, the
  question card, transcript rows, tool group and tool row buttons, and the welcome dialogs have
  nodes. Still without: the Home widget cards, the sidecar column, the "New task" and "Running
  agents" tabs, an expanded tool row's body (command text and output), the answer on an
  answered question row, and a message's footer (time, copy, branch). Read those from a
  snapshot, fall back to `x`,`y` for clicks, and file the gap. The fix is an id, a role and a
  label on the element (recipe d).
- gpui has no disabled state in the tree, so a disabled button (Back or Forward with no history,
  a question card's Submit before an answer) reads as a plain `Button`.
- Transcript row bounds include the row padding, and an open question row spans the card below
  it (937x396 with the card inside its lower part). Address a card's controls by selector, never
  by a row's center.
- Refs re-key when an element is rebuilt with a new AccessKit id (the palette search field gets
  a new ref each time it opens). Re-resolve by selector after any view rebuild.
- There is no virtual clock. Animations run on wall time; slow them with `motionScale`, and
  wait with `wait`, never with fixed sleeps.
- `snapshot` exists on macOS only; elsewhere it replies with an error. The socket server is
  unix-only, so Windows has no driver.
- Each connection is served by one thread and runs its requests in order. The CLI and the tool
  open one connection per call, so separate observe calls run concurrently (a `wait` in one, a
  `key` in another), while drive calls queue behind each other. To act at a moment inside a
  measurement, use `during`.
- Frame stamps and `track` assume one Shell window; the frame counter is global across windows.
- Release dumps carry no `view`, `elementId` or `source`, so `audit` has no source locations
  there.

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
| `<label> (pid n) did not answer within N s` | The main thread is blocked, or a drive call waited behind another long drive. Check `state` from a second call. |
| `` `timeoutMs` must be at most 60000 ms `` | Waits are capped at 60 s. Wait on an element or for `idle` in steps instead of one long wait. |
| `` `text` must not be empty `` | A `text` or `gone` wait (or a scenario `text` assert) got `""`, which every node contains. Pass the text to wait for. |
| `user rejected MCP tool call` on `ui_driver_drive` in a Codex chat | The chat runs on a Zeron build from before the pre-approval. Rebuild, or switch the chat to Full. |
| `the window drew no frame after the accessibility tree was enabled` | The window is minimized or fully hidden, so gpui does not draw. Bring it back on screen and retry. |
| `give one of ref, select, or x and y` | The request mixed target kinds, or gave `x` without `y` or `nth` without `select`. |
| `` `path` must end in .png `` / `is not a regular file` | Snapshot paths are absolute `.png` files; pick a new file name, not a link or folder. |
| `` ui_driver never writes where the caller picks, so it takes no `path` `` | Drop `path`: the tool saves under `~/.zeron/run/shots` and returns the PNG, or its path with `inline: false`. Only `zeron ui snapshot` takes `path`. |
| Little Snitch prompts for an unsigned dev binary | Expected for `target/debug` and ad-hoc signed bundles; allow the rule for any process with no code requirement, or the prompt returns after each rebuild. |
