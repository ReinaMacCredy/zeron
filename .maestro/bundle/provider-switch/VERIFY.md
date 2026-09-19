# VERIFY — provider-switch

Scenarios point at work acceptance/claims; do not restate them.

| # | Scenario | Work / acceptance | Result |
|---|----------|-------------------|--------|
| 1 | setChatConfig with another harness tombstones the session, writes the divider, first run of the new harness gets the history as text with resume=None, second run resumes its own session, old harness never runs again | w48 | pass: `cargo test -p zeron-engine --test provider_switch` (3/3) |
| 2 | Switch on an empty chat hands nothing over | w48 | pass (same suite) |
| 3 | Fork at a pre-switch reply refused; fork after the switch branches; rewind onto a pre-switch reply tombstones and re-hands the remaining history | w48 | pass (same suite) |
| 4 | Existing fork/rewind and sidechat behavior unchanged | w48 | pass: fork_rewind 8/8, sidechat 8/8 |
| 5 | Picker: browsing a foreign tab changes nothing; picking a foreign model parks the dialog with harness set; cancel leaves the row; confirm stamps harness+model | w49 | pass: pickers:: 29/29 |
| 6 | Full ui lib suite single-threaded and engine e2e still green (modulo the 3 known base failures) | w47 | pass: ui lib 1111/1111 single-threaded; engine --no-fail-fast 35 binaries green, 2 known env failures (generated_image TMPDIR symlink, local_first shutdown prober from a running app) |
