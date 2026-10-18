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

| Level | Actions | Builds |
|---|---|---|
| Observe | `state`, `tree`, `snapshot`, `wait`, `frames`, `track`, `audit` | every build, the stable app included |
| Drive | `click`, `move`, `drag`, `scroll`, `key`, `type`, `press`, `action`, `set`, plus `during` on `frames`/`track` | debug builds only (Zeron Dev bundles) |

Drive code is compiled out of release builds. A release build answers a drive request (or any
`during`) with: `click` drives the UI and exists only in debug builds (Zeron Dev bundles);
this build is observe-only. To drive, launch a Dev bundle (next sections). The stable app in
`/Applications/Zeron.app` hosts the person's running sessions, so observe it only. It serves the
driver only once it runs a build that includes it.

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
- MCP tool `ui_driver` in `zeron mcp` (Settings > Tools, Workspace group). The arguments are
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
per app. An agent in the stable app that omits `target` therefore reaches the release app,
which only observes. Pass the Dev bundle's label or pid to drive it.

The wire format is newline-delimited JSON over the socket: request
`{"id":1,"action":"state","window"?:<id>,...}`, reply `{"id":1,"ok":<value>}` or
`{"id":1,"error":"<msg>"}`. The client sends one request per connection and waits up to
`timeoutMs` + 10 s, else `ms` + 10 s, else 30 s.

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
`ZERON_MOTION_SCALE`, `ZERON_AUTO_UPDATE`, `ZERON_OPEN_ROUTE`, `CODEX_EXECUTABLE`, and the
`ZERON_CUA_*` probes. Arguments after the script go to the app.

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

Under each example, the second line is the same request as `ui_driver` tool arguments. Every
action also takes `window` (a window id from `state`; the default is the frontmost Shell window).

### Observe

| Action | Args (defaults) | Reply |
|---|---|---|
| `state` | none | `pid, build, input, windows[{id,title,kind,size{w,h},active,a11yForced}], route{kind,section,selectedChat}, frames{count,lastDrawAgoMs}, motionScale, theme{mode,appearance}` |
| `tree` | `query`, `root` (ref), `depth`, `full` (false), `json` (false) | text lines, or `{window,title,size,focus,total,nodes[...]}` |
| `snapshot` | `rect {x,y,w,h}` in points, `path` (absolute; default `~/.zeron/run/shots/<pid>-<n>.png`) | `{path,width,height}` |
| `wait` | `for`, plus the fields below | depends on `for` |
| `frames` | `ms` (1000, max 10000), `during` | `{frames,fps,intervalsMs{p50,p95,max},longFrames[{atMs,gapMs}],firstFrameAtMs,lastFrameAtMs,drawMs?,during?}` |
| `track` | `ref` or `select`+`nth`, `ms` (800, max 5000), `jumpPx` (40), `during` | `{samples[[tMs,x,y,w,h]],framesSampled,missingFrames,jumps[{atMs,from,to,delta}],settledAtMs,reresolved,ref,node,framesDrawn,readMs,during?}` |
| `audit` | none | `{counts{nodes,interactive,tabStops,unlabeledInteractive},issues[{ref,rule,role,label?,bounds?,view?,source?}]}` |

`route.kind` is one of `newChat`, `chat`, `pullRequests`, `usage`, `inbox`, `settings` (with
`section`, such as `"General"`), `terminal`, `tasks`, `docs`.

```sh
zui state
{"action":"state","target":"Zeron Dev 7"}
zui tree '{"query":"settings"}'
{"action":"tree","query":"settings","target":"Zeron Dev 7"}
zui snapshot '{"rect":{"x":520,"y":200,"w":760,"h":170},"path":"/abs/out/crop.png"}'
{"action":"snapshot","rect":{"x":520,"y":200,"w":760,"h":170},"path":"/abs/out/crop.png","inline":false,"target":"Zeron Dev 7"}
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
| `key` | `keys`: one string of space-separated strokes, or an array of them | `{frame,handled[bool per stroke]}` |
| `type` | `text`, optional target | `{frame}` |
| `press` | `ref` or `select`+`nth`, `do` (`click`), `value` (for `setValue`) | `{frame,ref}` |
| `action` | `name` (gpui action), `args` | `{frame}` |
| `set` | `theme` (`light`, `dark`, `system`), `size` (`{w,h}` or `[w,h]`), `motionScale` | `{frame,size,motionScale,theme}` |

"target" means one of `ref`, `select` (+`nth`), or `x`,`y`. Any element-addressed drive reply
may also carry `"unsettled": true` (see Actionability).

```sh
zui click '{"select":"Button \"Settings\""}'
{"action":"click","select":"Button \"Settings\"","target":"Zeron Dev 7"}
zui move '{"x":23,"y":189}'
{"action":"move","x":23,"y":189,"target":"Zeron Dev 7"}
zui drag '{"from":[301,500],"to":[400,500]}'
{"action":"drag","from":[301,500],"to":[400,500],"target":"Zeron Dev 7"}
zui scroll '{"select":"Group \"Settings\"","dy":-600}'
{"action":"scroll","select":"Group \"Settings\"","dy":-600,"target":"Zeron Dev 7"}
zui key '{"keys":"cmd-k"}'
{"action":"key","keys":"cmd-k","target":"Zeron Dev 7"}
zui type '{"select":"MultilineTextInput \"mind\"","text":"hello"}'
{"action":"type","select":"MultilineTextInput \"mind\"","text":"hello","target":"Zeron Dev 7"}
zui press '{"select":"Switch \"Compact mode\"","do":"click"}'
{"action":"press","select":"Switch \"Compact mode\"","do":"click","target":"Zeron Dev 7"}
zui action '{"name":"shell::ToggleCommandPalette"}'
{"action":"action","name":"shell::ToggleCommandPalette","target":"Zeron Dev 7"}
zui set '{"theme":"dark","size":[1200,800]}'
{"action":"set","theme":"dark","size":[1200,800],"target":"Zeron Dev 7"}
```

Input notes:

- `click` moves the pointer, waits a frame, then sends `count` down/up pairs.
- `scroll` sends one wheel event, or with `trackpad: true` a Started, six Moved and an Ended
  phase about 16 ms apart (that is enough to trigger swipe navigation with `dx`).
- `type` without a target types into the focused element. With one, it focuses the element
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
  ancestors. `root: "e12"` starts at a node, `depth: 0` prints only the start node.
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

### Actionability, frames, and the a11y lease

- An action addressed by `ref` or `select` first waits for the element to hold still. It
  re-resolves on every drawn frame and acts once two consecutive frames give the same bounds,
  or once no frame draws for 50 ms. After 1 s it acts anyway and adds `"unsettled": true`. Raw
  `x`,`y` skip this wait. An element that moves under about 0.5 pt per frame counts as still.
- Every drive reply returns after the next drawn frame following its last event (capped at
  500 ms). One frame is not a finished animation, so run `wait {"for":"idle"}` before you
  judge a transition by a snapshot or a tree read.
- `wait` kinds: `frame` forces one redraw and returns `{frame}`; `idle` returns
  `{frame,quietMs}` once no frame has drawn for `quietMs` (300); `ms` sleeps `ms`; `select`
  returns `{ref,node,waitedMs}` once the element exists; `gone` returns `{gone,waitedMs}` once
  nothing matches; `text` returns `{ref,node,waitedMs}` once a label, value or description
  contains `text`. `timeoutMs` defaults to 5000. A timeout reports what it last saw, for example
  `not idle after 5000 ms: 120 frames drawn (24.0 fps), last 3 ms ago`.
- The tree exists only while a window is forced to build it. The first `tree`, `audit`,
  element target or element wait forces it, which costs one or two frames (15-17 ms live). The
  lease ends 60 s after the last use; `state.windows[].a11yForced` shows it.
- `snapshot` forces one frame (30-130 ms) so the capture is current, and a full window is
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
  With `during`, `track` may start before its element exists (a palette row).
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
zui wait '{"for":"select","select":"MultilineTextInput \"Search threads\""}'
zui click '{"select":"Button \"New chat\""}'
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
output (`crates/harness/src/mock.rs`). Titles, commit messages and other one-shot calls ignore
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

A scenario file seeds the engine, drives the UI, asserts and snapshots (format in
`scripts/scenarios/README.md`):

```json
{ "name": "pick-a-color", "steps": [
  { "engine": "LocalDevice", "save": "device" },
  { "ui": "key", "keys": "cmd-k" },
  { "ui": "wait", "for": "idle" },
  { "ui": "click", "select": "Button \"New chat\"" },
  { "ui": "state", "save": "home" },
  { "assert": { "saved": "home.route.kind", "equals": "newChat" } },
  { "assert": { "select": "MultilineTextInput \"Type your own answer\"", "exists": false } },
  { "snapshot": "home" }
] }
```

Step kinds are `engine` (RPC to the target's engine; `first: true` keeps a stream's first
item, as with `WatchDocMessages`), `ui` (any driver action), `assert` (`select`/`text` with
`exists` and `timeoutMs` 3000, or `saved` with `equals`/`contains`), `snapshot` and `sleep`. Any
step takes `name` and `continueOnFail`. `${saved.path}`, `${env.NAME}`, `${target.ipcPort}` and
`${run.shots}` substitute into strings.

```sh
scripts/ui-scenario.sh scripts/scenarios/mock-question.json --mock-script scripts/scenarios/mock-question.mock.json
scripts/ui-scenario.sh scripts/scenarios/journal-replay.json --mock-script scripts/scenarios/journal-replay.jsonl
```

The launcher builds a fresh `target/ui-scenario/bundle/Zeron Scenario.app`
(`sh.zeron.app.dev.scenario`, port 49880 or `--port`, `ZERON_HARNESS=mock`, empty data dir, a
`git init`ed project exported as `ZERON_SCENARIO_PROJECT`), runs the scenario and quits the
bundle unless you pass `--keep`. Against an app that is already running, use
`zeron ui scenario <file> --target <t> --out <dir>`.

Reading the report: the last stdout line is `PASS n/n` or `FAIL k/n: step <i> <name>: <error>`
(k counts passed steps), and the exit code is 0 or 1. `report.md` has a row per step with its
result or error and links each snapshot; `report.json` holds the same steps (`index, kind,
name, status, ms, result, error, shot`). After a failure the rest are `skipped` unless the
failing step set `continueOnFail`. Open the snapshots: a step can pass while the screen is wrong.

## Agent as user: handing QA to a child agent

A lead agent can hand a QA pass to a child agent that acts only as a user would. Give the child:

1. this guide (`docs/ui-driver.md`) and nothing about the implementation;
2. the target label (`Zeron Dev 7`) and the rule that it acts only through `zeron ui` or the
   `ui_driver` tool with that `target`, never by editing code, never through computer use, and
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
  waits cannot see them. In this branch that includes transcript text and tool chips, sidebar
  session rows, question options and Submit, palette thread rows, the tab strip and some Home
  pickers. Fall back to `x`,`y` read off a snapshot, and file the gap. The fix is an id, a role
  and a label on the element (recipe d).
- Refs re-key when an element is rebuilt with a new AccessKit id (the palette search field gets
  a new ref each time it opens). Re-resolve by selector after any view rebuild.
- There is no virtual clock. Animations run on wall time; slow them with `motionScale`, and
  wait with `wait`, never with fixed sleeps.
- `snapshot` exists on macOS only; elsewhere it replies with an error. The socket server is
  unix-only, so Windows has no driver.
- Each connection is served by one thread and runs its requests in order. The CLI and the tool
  open one connection per call, so separate calls run concurrently (a `wait` in one, a `key` in
  another). Within one request, use `during`.
- Frame stamps and `track` assume one Shell window; the frame counter is global across windows.
- Release dumps carry no `view`, `elementId` or `source`, so `audit` has no source locations
  there.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `zeron ui list` prints `[]` | No live app serves the driver. The bundle is still building (check its launch log), it crashed (stale descriptors are pruned by pid), or the app's build predates the driver. |
| `no running Zeron app serves the UI driver: ... has no live ui-<pid>.json` | Same as above, reported at target selection. |
| `N Zeron apps match ...` | Several apps are live; pass `--target` with the pid or exact label from the listed targets. |
| `` `click` drives the UI and exists only in debug builds `` | You reached a release build, usually by omitting `target` from a chat in the stable app. Target the Dev bundle. |
| `` `<sel>` matches N elements `` | Narrow with a role, use the exact label, or pass `nth`. |
| `no element matches ...; near:` | Read the near list or `tree` with `query`; the element may lack a node. |
| `... is outside the WxH window; scroll it into view first` | Scroll its container (`scroll` with `select` on the scroll area), then retry. |
| `timed out after N ms; last seen: ...` / `not idle after ...` | The UI never reached the state, or an animation or spinner keeps drawing. Read the last-seen part, raise `timeoutMs`, or wait on an element instead of `idle`. |
| `<label> (pid n) did not answer within N s` | The main thread is blocked, or a long request outlived the client timeout. Check `state` from a second call. |
| Little Snitch prompts for an unsigned dev binary | Expected for `target/debug` and ad-hoc signed bundles; allow the rule for any process with no code requirement, or the prompt returns after each rebuild. |
