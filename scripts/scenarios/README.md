# UI scenarios

A scenario is one JSON file that seeds the engine, drives the UI through the UI
driver, asserts, and snapshots. The runner writes a pass/fail report.

```sh
scripts/ui-scenario.sh scripts/scenarios/mock-question.json --mock-script scripts/scenarios/mock-question.mock.json
scripts/ui-scenario.sh scripts/scenarios/journal-replay.json --mock-script scripts/scenarios/journal-replay.jsonl
zeron ui scenario <file.json> [--target <pid|label>] [--out <dir>]   # against an app that is already running
```

`ui-scenario.sh` builds and launches a fresh `target/ui-scenario/bundle/Zeron Scenario.app`
(bundle id `sh.zeron.app.dev.scenario`, engine port 49880 or `--port`, `ZERON_HARNESS=mock`,
`ZERON_AUTO_UPDATE=0`, `ZERON_MOCK_SCRIPT` from `--mock-script`). Each run gets an empty data
dir and project dir under `target/ui-scenario/<name>/`; the previous run's folder moves to the
Trash. The bundle quits at the end unless you pass `--keep`. The project dir is exported as
`ZERON_SCENARIO_PROJECT`.

Reports go to `--out`. `ui-scenario.sh` uses `target/ui-scenario/<name>/report/`, and the
default is `~/.zeron/run/scenarios/<name>-<time>/`. A report holds `report.json`, `report.md`
(a step table with snapshot links) and `shots/NN-<label>.png`. The last line of stdout is
`PASS n/n` or `FAIL k/n: step <i> <name>: <error>`, where k counts the passed steps. The
exit code is 0 or 1.

## Format

`{"name": str, "target"?: pid|label, "steps": [step]}`. A step has exactly one of:

| Step | Effect |
|---|---|
| `{"engine": "<RPC method>", "params"?: {...}, "first"?: bool, "save"?: "n"}` | Calls the target's engine at `ws://127.0.0.1:<ipcPort>` over one connection per run. With `first`, it subscribes and keeps the first item, e.g. `WatchDocMessages {chatId}` returns `{reset: [entries]}`. |
| `{"ui": "<driver action>", ...args, "save"?: "n"}` | Sends one UI driver request (`zeron ui <action>`). |
| `{"assert": {"select": "Role \"label\"", "nth"?: n, "exists"?: bool}}` | `true` waits for a match (`wait for:select`); `false` waits for none (`wait for:gone`). |
| `{"assert": {"text": "substring", "exists"?: bool}}` | `true` waits for a label, value or description that contains it; `false` reads the tree once. |
| `{"assert": {"saved": "n.path", "equals": json}}` | Compares a saved value. `"contains": "s"` instead searches its text (a JSON value is searched as compact JSON). |
| `{"snapshot": "label"}` | Writes `shots/NN-label.png`. |
| `{"sleep": ms}` | Waits. |

- `select` and `text` asserts take `timeoutMs` (default 3000).
- Any step may set `"name"` for the report, and `"continueOnFail": true`. Without it, the
  first failure skips the remaining steps.
- Strings in `params`, args and asserts may hold `${n.path.0.field}`: a saved result, `target`
  (the app descriptor), `run.out`, `run.shots`, or `${env.NAME}`. A string that is exactly one
  reference keeps the JSON type of the value. A reference that does not resolve fails the step.
- `select` and `text` see only the a11y tree (`zeron ui tree`). Transcript text, tool chips,
  question options and sidebar rows have no nodes yet, so assert those through a
  `WatchDocMessages` read plus a snapshot. A `text` assert with `exists: false` on such content
  passes for the wrong reason.
- Mock scripts are `ZERON_MOCK_SCRIPT` turn scripts or `{seq,event}` journals; see
  `crates/harness/src/mock.rs`. A mock chat is created with `config.harness: "mock"`.
