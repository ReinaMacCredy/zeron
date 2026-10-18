# UI scenarios

A scenario is one JSON file that seeds the engine, drives the UI through the UI driver,
asserts, and snapshots. The runner writes a pass/fail report.

```sh
scripts/ui-scenario.sh scripts/scenarios/mock-question.json --mock-script scripts/scenarios/mock-question.mock.json
scripts/ui-scenario.sh scripts/scenarios/journal-replay.json --mock-script scripts/scenarios/journal-replay.jsonl
zeron ui scenario <file.json> [--target <pid|label>] [--out <dir>]   # against an app that is already running
```

The step format, the launcher's behavior, mock scripts and how to read a report are in
`docs/ui-driver.md`, recipe (e).
