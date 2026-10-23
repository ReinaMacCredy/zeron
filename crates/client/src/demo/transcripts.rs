//! Demo transcripts: hand-written fixtures that exercise every renderer path
//! (headings, nested/ordered/task lists, tables, quotes, links, inline code,
//! code blocks in several languages, long paths/URLs, CJK + emoji, tool
//! groups, questions, errors, attachments), the synthetic big transcripts
//! for benchmarks, and the scripted streaming reply.

use zeron_doc::{
    MessagePart, MessageRole, MessageStatus, SessionMessageEntry, SubagentStatus, ToolDiffStat,
};
use zeron_proto::computer::cards::{
    COMPUTER_CONSENT_ALLOW, COMPUTER_CONSENT_DECLINE, COMPUTER_CONTROL_ON_DISCLOSURE,
    COMPUTER_DESKTOP_REVOKED_NOTE, COMPUTER_ESCAPE_STOP_NOTICE, COMPUTER_FOREGROUND_ALLOW,
    COMPUTER_FOREGROUND_DECLINE, COMPUTER_SETUP_NOTE, COMPUTER_UNAVAILABLE_NOTE,
    COMPUTER_USER_PAUSE_NOTE_PREFIX, ConsentOutcome, RevokeSource, consent_always_label,
    foreground_title,
};
use zeron_proto::computer::{ComputerConsentApp, ComputerConsentScope};
use zeron_proto::{
    TodoItem, TodoStatus, ToolCall, UserInputAnswer, UserInputKind, UserInputQuestion,
};

pub(crate) const PHONE: &str = "ios-demo";

pub(crate) fn text(id: &str, body: &str) -> MessagePart {
    MessagePart::Text {
        id: id.into(),
        text: body.into(),
    }
}

fn reasoning(id: &str, body: &str) -> MessagePart {
    MessagePart::Reasoning {
        id: id.into(),
        text: body.into(),
    }
}

pub(crate) fn tool(id: &str, call: ToolCall, is_error: bool, output: Option<&str>) -> MessagePart {
    MessagePart::Tool {
        id: id.into(),
        call,
        is_error,
        resolved: true,
        output: output.map(str::to_owned),
        diff: None,
        output_ref: None,
        output_bytes: output.map(|o| o.len() as u64),
        diff_ref: None,
        diff_stats: None,
        subagent_ref: None,
        subagent_status: None,
        subagent_tail: None,
        html_render: None,
        delegate_route: None,
    }
}

fn edit(id: &str, path: &str, additions: u64, deletions: u64) -> MessagePart {
    let mut part = tool(
        id,
        ToolCall::EditFile {
            path: path.into(),
            old_string: None,
            new_string: None,
        },
        false,
        None,
    );
    if let MessagePart::Tool { diff_stats, .. } = &mut part {
        *diff_stats = Some(vec![ToolDiffStat {
            path: path.into(),
            additions,
            deletions,
        }]);
    }
    part
}

fn exec(command: &str) -> ToolCall {
    ToolCall::Exec {
        command: command.into(),
    }
}

fn read(path: &str) -> ToolCall {
    ToolCall::ReadFile { path: path.into() }
}

pub(crate) fn entry(
    id: &str,
    role: MessageRole,
    device: &str,
    created_at: i64,
    parts: Vec<MessagePart>,
) -> SessionMessageEntry {
    SessionMessageEntry {
        id: id.into(),
        role,
        parts,
        created_at,
        device_id: device.into(),
        status: Some(MessageStatus::Complete),
        continuation_of: None,
        duration_ms: (role == MessageRole::Assistant).then_some(48_000),
        from_chat_id: None,
        fork_import: false,
        answers_request_id: None,
    }
}

fn user(id: &str, at: i64, body: &str) -> SessionMessageEntry {
    entry(id, MessageRole::User, PHONE, at, vec![text("t0", body)])
}

fn assistant(id: &str, host: &str, at: i64, parts: Vec<MessagePart>) -> SessionMessageEntry {
    entry(id, MessageRole::Assistant, host, at, parts)
}

/// Paths the demo serves generated images for.
pub(crate) const DEMO_IMAGES: &[(&str, u32, u32, u32)] = &[
    (
        "/Users/dev/.zeron/uploads/4f1c9a2e-Image_1.png",
        960,
        540,
        1,
    ),
    ("/Users/dev/.zeron/uploads/scroll-tall.png", 400, 800, 2),
    ("/Users/dev/.zeron/uploads/square.png", 600, 600, 3),
];

const VEIL_PLAN: &str = r#"## Veil port plan

The desktop veil (`veil.rs`) multiplies a fading alpha into each appended chunk's text color — **paint-layer only**, so shaping and wrapping never change. Three invariants to carry over:

1. Chunk spans keep their *exact* byte length when split
2. Fade duration tracks the append cadence: `clamp(ema × 3, 120, 400)` ms
3. Re-attach seeds the baseline — only post-switch appends animate
   - a cold open renders history fully opaque
   - a warm reopen keeps the in-flight fade

| Constant | Value | Where |
| --- | ---: | --- |
| `VEIL_MIN_FADE_MS` | 120 | `crates/ui/src/markdown/veil.rs` |
| `VEIL_MAX_FADE_MS` | 400 | same |
| `VEIL_CURVE_POW` | 1.6 | same |

> The curve is `1 − (1−p)^1.6` — fast attack, soft landing.
> It reads calm even at 140ms token cadence."#;

const VEIL_IMPL: &str = r#"Implementation lands in `Veil.swift`:

```swift
func veilOpacity(_ p: Double) -> Double {
    1 - pow(1 - p, 1.6)  // fast attack, soft landing
}

// Duration follows the streaming cadence EMA.
let duration = min(max(ema * 3, 120), 400)
```

And the Rust side stays the source of truth:

```rust
pub fn veil_alpha(elapsed_ms: f32, duration_ms: f32) -> f32 {
    let p = (elapsed_ms / duration_ms).clamp(0.0, 1.0);
    1.0 - (1.0 - p).powf(VEIL_CURVE_POW)
}
```

The row keeps one `RowVeil` while streaming and drops it on the live→complete flip, exactly like the desktop lifecycle. Details in [the veil design note](https://github.com/zeron-sh/zeron/blob/main/docs/design/transcript-veil.md#lifecycle)."#;

const CJK_REPLY: &str = r#"### Grapheme-safe fading

Chunk boundaries now snap to **extended grapheme clusters**, so these all fade as single units:

- 日本語のテキストは文字単位でフェードします。
- 中文段落也一样：每个汉字都是一个独立的字素簇。
- 한국어 음절도 마찬가지로 처리됩니다.
- Emoji sequences: 👩🏽‍💻 🏳️‍🌈 👨‍👩‍👧‍👦 🇯🇵 — each one cluster, never split mid-ZWJ.
- Mixed runs: `veil` + 絵文字 🎉 + العربية stay aligned.

| Script | Sample | Clusters |
| --- | --- | ---: |
| Japanese | こんにちは世界 | 7 |
| Emoji ZWJ | 👨‍👩‍👧‍👦 | 1 |
| Flags | 🇨🇦🇯🇵 | 2 |

The splitter lives in `/Users/dev/zeron/crates/ui/src/markdown/veil/grapheme_boundaries_for_streamed_chunks.rs` and is covered by a table test:

```python
CASES = [
    ("日本語", 3),
    ("👨‍👩‍👧‍👦", 1),
    ("é", 1),  # combining acute accent
]
for text, expected in CASES:
    assert len(grapheme_clusters(text)) == expected, text
```

```ts
export function clusters(text: string): string[] {
  const seg = new Intl.Segmenter(undefined, { granularity: "grapheme" });
  return Array.from(seg.segment(text), (s) => s.segment);
}
```

Reference: https://www.unicode.org/reports/tr29/#Grapheme_Cluster_Boundaries_and_Extended_Grapheme_Clusters_in_Full_Detail"#;

const VEIL_LIVE_PREFIX: &str = "All 14 veil tests pass. Pushing `veil-fade` and";

/// The remainder `chat-veil`'s live entry streams when the session opens.
pub(crate) const VEIL_LIVE_REST: &str = r#" opening the pull request against `main`:

- [x] `cargo test -p zeron-ui veil` — 14 passed
- [x] `xcodebuild -scheme Zeron build`
- [ ] Screenshots for the PR description

```bash
git push -u origin veil-fade
gh pr create --base main --title "Stream pull request status on every client"
```

PR **#90** is open: https://github.com/zeron-sh/zeron/pull/90"#;

fn veil(host: &str, now: i64) -> Vec<SessionMessageEntry> {
    // Sent from the desktop composer: file, folder and skill chips, plus the
    // image and recording it attached, each mentioned by its chip.
    let review = zeron_proto::invocation::Invocation::Skill {
        name: "review".into(),
        path: "/Users/dev/.claude/skills/review/SKILL.md".into(),
        command: None,
    };
    let attach = crate::attachments::with_attachments(
        &format!(
            "Port the streaming fade-in veil from [transcript.rs](zeron-file:crates/ui/src/transcript.rs) into [Transcript](zeron-file:apps/ios/Zeron/Transcript/). It must never affect layout — opacity only, split at chunk boundaries. Here's how it looks today: [Image 1](zeron-image:1), and the fade is in [veil-recording.zip](zeron-attachment:2). Run {} when done.",
            review.link()
        ),
        &[
            DEMO_IMAGES[0].0.to_owned(),
            "/Users/dev/.zeron/uploads/7d03b6e1-veil-recording.zip".to_owned(),
        ],
    );
    let mut live = assistant(
        "m6",
        host,
        now - 60_000,
        vec![
            text("t0", "Opening the PR now. Running the checks first:"),
            tool(
                "k1",
                exec("cargo test -p zeron-ui veil -- --nocapture"),
                false,
                Some("test result: ok. 14 passed; 0 failed; 0 ignored"),
            ),
            text("t1", VEIL_LIVE_PREFIX),
        ],
    );
    live.status = Some(MessageStatus::Streaming);
    live.duration_ms = None;
    vec![
        user("m1", now - 3_500_000, &attach),
        assistant(
            "m2",
            host,
            now - 3_400_000,
            vec![
                reasoning(
                    "r0",
                    "The veil must be paint-only. Check how veil.rs splits chunk spans and where the fade duration comes from before touching the Swift side.",
                ),
                text("t0", VEIL_PLAN),
                tool("k1", read("crates/ui/src/markdown/veil.rs"), false, None),
                tool(
                    "k2",
                    ToolCall::Search {
                        pattern: "VEIL_".into(),
                        path: Some("crates/ui".into()),
                    },
                    false,
                    Some("crates/ui/src/markdown/veil.rs:12: pub const VEIL_MIN_FADE_MS"),
                ),
                edit("k3", "apps/ios/Zeron/Transcript/Veil.swift", 84, 12),
                tool(
                    "k4",
                    exec(
                        "xcodebuild -scheme Zeron -destination 'platform=iOS Simulator,name=iPhone 17 Pro' build",
                    ),
                    false,
                    Some("** BUILD SUCCEEDED **"),
                ),
                text("t1", VEIL_IMPL),
            ],
        ),
        user(
            "m3",
            now - 1_800_000,
            "Also check it handles CJK and emoji — 日本語のテキストと絵文字 🎉 must fade per grapheme, not per byte.",
        ),
        assistant(
            "m4",
            host,
            now - 1_700_000,
            vec![
                tool(
                    "k1",
                    ToolCall::Glob {
                        pattern: "crates/ui/src/markdown/**/*.rs".into(),
                    },
                    false,
                    None,
                ),
                tool(
                    "k2",
                    ToolCall::WebFetch {
                        url: "https://www.unicode.org/reports/tr29/#Grapheme_Cluster_Boundaries"
                            .into(),
                        prompt: None,
                    },
                    false,
                    None,
                ),
                tool(
                    "k3",
                    ToolCall::Todo {
                        items: vec![
                            TodoItem::new(
                                "Snap chunk splits to grapheme clusters",
                                TodoStatus::Completed,
                            ),
                            TodoItem::new("Table test for ZWJ sequences", TodoStatus::Completed),
                            TodoItem::new(
                                "Measure veil cost on 600-turn transcript",
                                TodoStatus::Pending,
                            ),
                        ],
                    },
                    false,
                    None,
                ),
                edit("k4", "crates/ui/src/markdown/veil.rs", 31, 4),
                text("t0", CJK_REPLY),
            ],
        ),
        user("m5", now - 120_000, "Ship it and open the PR."),
        live,
    ]
}

fn picker(host: &str, now: i64) -> Vec<SessionMessageEntry> {
    vec![
        user(
            "m1",
            now - 400_000,
            "The model picker shows stale catalogs after switching devices — where should the catalog come from?",
        ),
        assistant(
            "m2",
            host,
            now - 380_000,
            vec![
                tool(
                    "k1",
                    ToolCall::Search {
                        pattern: "list_models".into(),
                        path: None,
                    },
                    false,
                    None,
                ),
                tool("k2", read("crates/ui/src/shell/pickers.rs"), false, None),
                text(
                    "t0",
                    "Two viable sources — the local device's harness install, or the space's owning device. The desktop recently moved to the latter (`aa128a6`). Before I wire the RPC, two decisions:",
                ),
                MessagePart::Input {
                    id: "req-1".into(),
                    request_id: "req-1".into(),
                    questions: picker_questions(),
                    resolved: false,
                    answers: None,
                    asked_at_ms: None,
                    answered_at_ms: None,
                },
            ],
        ),
    ]
}

pub(crate) fn picker_questions() -> Vec<UserInputQuestion> {
    vec![
        UserInputQuestion {
            approval: false,
            id: "q1".into(),
            header: "Catalog source".into(),
            question: "Which device should serve harness/model catalogs for the picker?".into(),
            options: vec![
                "Space's device (Recommended)".into(),
                "Local device".into(),
                "Union of both".into(),
            ],
            prefill: None,
            multiline: false,
            multi_select: false,
            kind: None,
        },
        UserInputQuestion {
            approval: false,
            id: "q2".into(),
            header: "Harnesses".into(),
            question: "Which harnesses should the picker offer on phones?".into(),
            options: vec![
                "Claude Code".into(),
                "Codex".into(),
                "OpenCode".into(),
                "Grok".into(),
            ],
            prefill: None,
            multiline: false,
            multi_select: true,
            kind: None,
        },
    ]
}

fn shelf(host: &str, now: i64) -> Vec<SessionMessageEntry> {
    vec![
        user(
            "m1",
            now - 300_000,
            "Step 4 of Inbox settle shelf: read settle_after_days from the host.",
        ),
        assistant(
            "m2",
            host,
            now - 240_000,
            vec![
                text(
                    "t0",
                    "The phone has no settings row for it yet. One question before I read the setting:",
                ),
                MessagePart::Input {
                    id: super::missions::SHELF_QUESTION.into(),
                    request_id: super::missions::SHELF_QUESTION.into(),
                    questions: super::missions::shelf_questions(),
                    resolved: false,
                    answers: None,
                    asked_at_ms: None,
                    answered_at_ms: None,
                },
            ],
        ),
    ]
}

fn approval(
    host: &str,
    now: i64,
    prompt: &str,
    reply: &str,
    question: (&str, &str, [&str; 2]),
) -> Vec<SessionMessageEntry> {
    let (header, ask, options) = question;
    vec![
        user("m1", now - 200_000, prompt),
        assistant(
            "m2",
            host,
            now - 180_000,
            vec![
                text("t0", reply),
                MessagePart::Input {
                    id: "req-approve".into(),
                    request_id: "req-approve".into(),
                    questions: vec![UserInputQuestion {
                        approval: true,
                        id: "approve".into(),
                        header: header.into(),
                        question: ask.into(),
                        options: options.iter().map(|o| (*o).to_owned()).collect(),
                        prefill: None,
                        multiline: false,
                        multi_select: false,
                        kind: None,
                    }],
                    resolved: false,
                    answers: None,
                    asked_at_ms: None,
                    answered_at_ms: None,
                },
            ],
        ),
    ]
}

fn tabs(host: &str, now: i64) -> Vec<SessionMessageEntry> {
    vec![
        user(
            "m1",
            now - 1_000_000,
            "Tool group headers turn red when any child fails — they should stay quiet, chips carry the error.",
        ),
        assistant(
            "m2",
            host,
            now - 950_000,
            vec![
                // The chat moved from Claude Code to Codex before this turn.
                MessagePart::Switch {
                    id: "s0".into(),
                    from: "Claude Code".into(),
                    to: "Codex".into(),
                    from_harness: Some("claude-code".into()),
                    to_harness: Some("codex".into()),
                },
                tool(
                    "k1",
                    ToolCall::Search {
                        pattern: "group_header_color".into(),
                        path: None,
                    },
                    false,
                    None,
                ),
                tool(
                    "k2",
                    exec("cargo test -p zeron-ui tool_group"),
                    true,
                    Some("error[E0425]: cannot find value `danger_muted` in this scope"),
                ),
                edit("k3", "crates/ui/src/shell/transcript.rs", 6, 9),
                tool(
                    "k4",
                    exec("cargo test -p zeron-ui tool_group"),
                    false,
                    Some("test result: ok. 9 passed"),
                ),
                text(
                    "t0",
                    "Done — the header keeps `text_muted` even on failure; only the chip label and the summary segment (\"1 failed\") pick up `danger`. Matches the desktop fix in `1749890`.",
                ),
            ],
        ),
    ]
}

fn errored(host: &str, now: i64) -> Vec<SessionMessageEntry> {
    let mut failed = assistant(
        "m2",
        host,
        now - 2_000_000,
        vec![
            text(
                "t0",
                "Reproducing the flush-timer wake on a hibernated Durable Object:",
            ),
            tool(
                "k1",
                exec("npx wrangler dev --test-scheduled"),
                true,
                Some("✘ [ERROR] Could not resolve \"node:async_hooks\""),
            ),
            MessagePart::Error {
                id: "e0".into(),
                message: "Harness exited with status 1: wrangler crashed before the first request".into(),
            },
        ],
    );
    failed.status = Some(MessageStatus::Aborted);
    // The error states, oldest first: signed out, a usage limit, then a plain
    // failure (the first two offer "Log in").
    let mut signed_out = assistant(
        "m2a",
        host,
        now - 2_500_000,
        vec![MessagePart::Error {
            id: "e0".into(),
            message: "Not logged in · Please run /login".into(),
        }],
    );
    signed_out.status = Some(MessageStatus::Aborted);
    let mut limited = assistant(
        "m2b",
        host,
        now - 2_250_000,
        vec![MessagePart::Error {
            id: "e0".into(),
            message: "Claude usage limit reached — try again after the limit resets.".into(),
        }],
    );
    limited.status = Some(MessageStatus::Aborted);
    vec![
        user("m1a", now - 2_600_000, "Look into the flush timer wakes."),
        signed_out,
        user("m1b", now - 2_300_000, "Signed in — try again."),
        limited,
        user(
            "m1",
            now - 2_100_000,
            "Why does the flush timer wake hibernated rooms every 5s?",
        ),
        failed,
    ]
}

fn scroll(host: &str, now: i64) -> Vec<SessionMessageEntry> {
    let attach = crate::attachments::with_attachments(
        "The transcript goes blank until I scroll after reopening a long chat.",
        &[DEMO_IMAGES[1].0.to_owned()],
    );
    vec![
        user("m1", now - 5_000_000, &attach),
        assistant(
            "m2",
            host,
            now - 4_900_000,
            vec![
                text(
                    "t0",
                    r#"## Root cause

"Blank until scroll" means the content offset is stranded **past the end** of the content after a geometry change:

1. The keyboard dismisses → the viewport grows
2. Rows below re-measure shorter
3. The offset still points at the old bottom

### Fix

Clamp the invariant on *every* geometry change instead of adding per-trigger clamps:

```swift
override func layoutSubviews() {
    super.layoutSubviews()
    let maxY = max(0, contentSize.height - bounds.height + adjustedContentInset.bottom)
    if contentOffset.y > maxY { contentOffset.y = maxY }
}
```

- See `TranscriptScrollMatrixTests` for the 36-case matrix
- [UIScrollView docs](https://developer.apple.com/documentation/uikit/uiscrollview)"#,
                ),
                tool(
                    "k1",
                    exec("xcodebuild test -only-testing:ZeronTests/TranscriptScrollMatrixTests"),
                    false,
                    Some("Executed 36 tests, with 0 failures"),
                ),
            ],
        ),
    ]
}

fn home(host: &str, now: i64) -> Vec<SessionMessageEntry> {
    vec![
        user(
            "m1",
            now - 7_200_000,
            "Clean up ~/Downloads — anything older than 90 days goes to an archive folder.",
        ),
        assistant(
            "m2",
            host,
            now - 7_100_000,
            vec![
                tool(
                    "k1",
                    exec("find ~/Downloads -maxdepth 1 -mtime +90 | wc -l"),
                    false,
                    Some("412"),
                ),
                text(
                    "t0",
                    r#"Found **412** items older than 90 days. Plan:

- [x] Create `~/Downloads/_archive-2026`
- [x] Move files, keep folder structure
  - `.dmg` installers → deleted instead (re-downloadable)
  - screenshots → `~/Pictures/Screenshots/old`
- [ ] Empty the Trash (waiting for your OK)

```bash
mkdir -p ~/Downloads/_archive-2026
find ~/Downloads -maxdepth 1 -mtime +90 ! -name '_archive-*' \
  -exec mv {} ~/Downloads/_archive-2026/ \;
```

```json
{ "moved": 377, "deleted": 35, "freedBytes": 18432000000 }
```"#,
                ),
            ],
        ),
    ]
}

fn visuals(host: &str, now: i64) -> Vec<SessionMessageEntry> {
    let local = super::visuals::local();
    if !local.is_empty() {
        let mut parts = Vec::new();
        for (i, visual) in local.iter().enumerate() {
            parts.push(text(&format!("t{i}"), &format!("**{}**", visual.title)));
            parts.push(MessagePart::Visual {
                id: visual.id.clone(),
                path: visual.path.clone(),
                title: visual.title.clone(),
            });
        }
        return vec![
            user("m1", now - 300_000, "Show the desktop's visuals."),
            assistant("m2", host, now - 290_000, parts),
        ];
    }
    vec![
        user("m1", now - 300_000, "Show me what visuals can do."),
        assistant(
            "m2",
            host,
            now - 290_000,
            vec![
                text("th", "Three treatments of the onboarding card:"),
                super::pages::part(),
                text("t0", "A canvas that follows your finger:"),
                super::visuals::part(1),
                text(
                    "t1",
                    "A scene you can orbit, with metal, glass and a light:",
                ),
                super::visuals::part(2),
                text("t2", "A shader; drag to move the light."),
                super::visuals::part(3),
                text(
                    "t3",
                    "And a dashboard: switch the range, scrub the chart, and toggle the table.",
                ),
                super::visuals::part(0),
            ],
        ),
    ]
}

fn cjk(host: &str, now: i64) -> Vec<SessionMessageEntry> {
    vec![
        user(
            "m1",
            now - 600_000,
            "テキストのレイアウトを多言語で確認して 🌏 — 中文、한국어、emoji も。",
        ),
        assistant(
            "m2",
            host,
            now - 590_000,
            vec![text(
                "t0",
                r#"# 多言語レイアウト確認 🌏

日本語の長い段落です。禁則処理により、句読点（、。）や閉じ括弧」は行頭に来ません。長い文章でも自然に折り返されることを確認してください。これはテスト用の文章です。

中文段落：排版引擎需要正确处理标点挤压和避头尾规则，例如“引号”和（括号）不应出现在行首。

한국어 문단: 단어 단위 줄바꿈이 올바르게 동작하는지 확인합니다.

> 引用ブロック内の日本語 — 👩🏽‍💻 と 🏳️‍🌈 は一つの書記素クラスタです。

1. 第一項目
2. 第二項目 `inline コード`
3. 第三項目 [リンク](https://ja.wikipedia.org/wiki/禁則処理)

| 言語 | 例 |
| --- | --- |
| 日本語 | 吾輩は猫である |
| 中文 | 我思故我在 |
| 한국어 | 안녕하세요 |
| emoji | 🎉🚀✨🧪 |"#,
            )],
        ),
    ]
}

fn computer_tool(
    id: &str,
    name: &str,
    input: Option<serde_json::Value>,
    denied: bool,
) -> MessagePart {
    let call = ToolCall::Mcp {
        server: "zeron".into(),
        tool: name.into(),
        input,
    };
    let output = r#"{"error":{"code":"capability_denied","message":"Computer control is off for this turn; no input was sent."}}"#;
    tool(id, call, denied, denied.then_some(output))
}

fn consent(
    id: &str,
    scope: ComputerConsentScope,
    question: &str,
    app: Option<ComputerConsentApp>,
    tool: Option<&str>,
    answer: Option<&str>,
) -> MessagePart {
    let mut options = vec![COMPUTER_CONSENT_ALLOW.to_owned()];
    options.extend(app.as_ref().and_then(consent_always_label));
    options.push(COMPUTER_CONSENT_DECLINE.into());
    input(
        id,
        "Computer",
        question,
        options,
        UserInputKind::Consent {
            scope,
            app,
            tool: tool.map(str::to_owned),
            params: None,
        },
        answer,
    )
}

fn input(
    id: &str,
    header: &str,
    question: &str,
    options: Vec<String>,
    kind: UserInputKind,
    answer: Option<&str>,
) -> MessagePart {
    let question_id = match kind {
        UserInputKind::Foreground { .. } => "foreground",
        _ => "consent",
    };
    MessagePart::Input {
        id: id.into(),
        request_id: id.into(),
        questions: vec![UserInputQuestion {
            approval: false,
            id: question_id.into(),
            header: header.into(),
            question: question.into(),
            options,
            multi_select: false,
            kind: Some(kind),
            prefill: None,
            multiline: false,
        }],
        resolved: answer.is_some(),
        answers: answer.map(|label| {
            vec![UserInputAnswer {
                question_id: question_id.into(),
                labels: vec![label.into()],
            }]
        }),
        asked_at_ms: None,
        answered_at_ms: None,
    }
}

fn app(name: &str, bundle_id: &str) -> Option<ComputerConsentApp> {
    Some(ComputerConsentApp {
        name: name.into(),
        bundle_id: Some(bundle_id.into()),
        pid: None,
    })
}

fn computer(host: &str, now: i64) -> Vec<SessionMessageEntry> {
    let note = |id: &str, at: i64, body: &str, hint: Option<&str>| {
        let mut parts = vec![text("t0", body)];
        parts.extend(hint.map(|hint| text("t1", hint)));
        entry(id, MessageRole::System, host, at, parts)
    };
    let script = serde_json::json!({
        "title": "Work out 2 + 2 in Calculator",
        "app": "Calculator",
        "code": "let app = await cua.getApp(\"Calculator\");\nawait app.click(12);\nawait app.typeText(\"2+2=\");",
        "actions": [
            { "tool": "launch_app" },
            { "tool": "click", "n": 3 },
            { "tool": "type_text", "detail": "2+2=" },
        ],
    });
    vec![
        user(
            "m1",
            now - 600_000,
            "/computer-use Open Calculator and work out 2 + 2",
        ),
        note(
            "n1",
            now - 560_000,
            COMPUTER_SETUP_NOTE,
            Some("Allow Accessibility for Zeron in System Settings, then continue."),
        ),
        note(
            "n2",
            now - 540_000,
            COMPUTER_UNAVAILABLE_NOTE,
            Some(
                "OpenCode does not receive Zeron's Computer tools. Switch this chat to Claude, Codex or another agent that does to use Computer.",
            ),
        ),
        note("n3", now - 520_000, COMPUTER_ESCAPE_STOP_NOTICE, None),
        assistant(
            "m2",
            host,
            now - 400_000,
            vec![
                computer_tool(
                    "c0",
                    "mcp__zeron__computer_click",
                    Some(serde_json::json!({ "x": 412, "y": 300 })),
                    true,
                ),
                consent(
                    "computer-consent-demo-call",
                    ComputerConsentScope::Call,
                    "Allow the agent to run computer_read_clipboard once?",
                    None,
                    Some("computer_read_clipboard"),
                    Some(COMPUTER_CONSENT_ALLOW),
                ),
            ],
        ),
        note("n4", now - 350_000, COMPUTER_CONTROL_ON_DISCLOSURE, None),
        assistant(
            "m3",
            host,
            now - 300_000,
            vec![
                computer_tool("c1", "computer", Some(script), false),
                computer_tool(
                    "c2",
                    "computer_launch_app",
                    Some(serde_json::json!({ "name": "Calculator" })),
                    false,
                ),
                computer_tool(
                    "c3",
                    "computer_type_text",
                    Some(serde_json::json!({ "text": "2+2=" })),
                    false,
                ),
                computer_tool("c4", "computer_read_clipboard", None, false),
                input(
                    "computer-foreground-demo",
                    "Computer",
                    &foreground_title(Some("Calculator")),
                    vec![
                        COMPUTER_FOREGROUND_ALLOW.into(),
                        COMPUTER_FOREGROUND_DECLINE.into(),
                    ],
                    UserInputKind::Foreground {
                        app: app("Calculator", "com.apple.calculator"),
                    },
                    Some(COMPUTER_FOREGROUND_ALLOW),
                ),
            ],
        ),
        note(
            "n5",
            now - 280_000,
            &format!("{COMPUTER_USER_PAUSE_NOTE_PREFIX}Calculator"),
            None,
        ),
        note(
            "n6",
            now - 260_000,
            COMPUTER_DESKTOP_REVOKED_NOTE,
            Some(RevokeSource::Stop.hint()),
        ),
        note("n7", now - 240_000, "Left 2 Calculator windows open", None),
        assistant(
            "m4",
            host,
            now - 200_000,
            vec![
                text(
                    "t0",
                    "Calculator shows 4. To paste it into your shell I need Terminal:",
                ),
                consent(
                    "computer-consent-demo-task",
                    ComputerConsentScope::Task,
                    "Allow Computer for this task? The agent wants to use Terminal and will drive this Mac's apps until the turn ends.",
                    app("Terminal", "com.apple.Terminal"),
                    None,
                    None,
                ),
            ],
        ),
    ]
}

fn short(host: &str, now: i64, prompt: &str, reply: &str) -> Vec<SessionMessageEntry> {
    vec![
        user("m1", now - 120_000, prompt),
        assistant("m2", host, now - 100_000, vec![text("t0", reply)]),
    ]
}

fn spawn(id: &str, task: &str, model: Option<&str>, status: SubagentStatus) -> MessagePart {
    let input = match model {
        Some(model) => serde_json::json!({ "description": task, "model": model }),
        None => serde_json::json!({ "description": task }),
    };
    let mut part = tool(
        id,
        ToolCall::Unknown {
            name: format!("Agent: {task}"),
            input: Some(input),
        },
        false,
        None,
    );
    if let MessagePart::Tool {
        subagent_ref,
        subagent_status,
        ..
    } = &mut part
    {
        *subagent_ref = Some(format!("sub-{id}"));
        *subagent_status = Some(status);
    }
    part
}

fn agents(host: &str, now: i64) -> Vec<SessionMessageEntry> {
    vec![
        user(
            "m1",
            now - 300_000,
            "Fan the parity audit out to one agent per surface.",
        ),
        assistant(
            "m2",
            host,
            now - 280_000,
            vec![
                text("t0", "Spawning one agent per surface:"),
                spawn(
                    "s1",
                    "Audit the layout cache",
                    Some("claude-haiku-5-5"),
                    SubagentStatus::Done,
                ),
                spawn(
                    "s2",
                    "Port the veil to Android",
                    Some("openai/gpt-5.6-sol"),
                    SubagentStatus::Done,
                ),
                spawn(
                    "s3",
                    "Review the edge worker",
                    Some("xai/grok-4.5"),
                    SubagentStatus::Running,
                ),
                spawn("s4", "Summarize the CRDT notes", None, SubagentStatus::Done),
                text("t1", "Four agents reported back."),
            ],
        ),
        from_thread(
            "m3",
            now - 200_000,
            "chat-cursor",
            "Delegated task chat-cursor (\"Raise the tab bar hit targets\") completed.\n<agent-response>\nRaised every tab to **44pt** and kept the icons centered.\n</agent-response>",
        ),
        assistant(
            "m4",
            host,
            now - 190_000,
            vec![text("t0", "Cursor finished the hit targets.")],
        ),
        from_thread(
            "m5",
            now - 150_000,
            "chat-pi",
            "The layout pass p95 is 3.1ms on the A17. Want the trace?",
        ),
        from_thread(
            "m6",
            now - 120_000,
            "chat-hermes",
            "Step \"Draft the changelog\" reported back on its thread chat-hermes.\n<agent-response>\nGrouped the changelog by surface: iOS, desktop, engine.\n</agent-response>",
        ),
        from_thread(
            "m7",
            now - 90_000,
            "chat-agents",
            "Continue with the Android bindings once the explorer reports.",
        ),
        assistant(
            "m8",
            host,
            now - 80_000,
            vec![
                text("t0", "Delegating the remaining surfaces:"),
                delegate(
                    "d1",
                    "Raise the tab bar hit targets",
                    Some("chat-cursor"),
                    None,
                ),
                delegate("d2", "Find the flaky e2e on CI", Some("chat-devin"), None),
                delegate("d3", "Tune the edge cache headers", Some("chat-grok"), None),
                delegate(
                    "d4",
                    "Map the Android bindings",
                    None,
                    Some(zeron_proto::ChildRoute {
                        source: zeron_proto::ChildRoute::CHOSEN_BY_AGENT.into(),
                        role: Some("explorer".into()),
                        ..Default::default()
                    }),
                ),
                started_thread("c1", "Changelog draft", "chat-hermes"),
            ],
        ),
    ]
}

fn delegate(
    id: &str,
    task: &str,
    child: Option<&str>,
    route: Option<zeron_proto::ChildRoute>,
) -> MessagePart {
    let mut part = tool(
        id,
        ToolCall::Mcp {
            server: "zeron".into(),
            tool: "delegate_task".into(),
            input: Some(serde_json::json!({ "task": task })),
        },
        false,
        None,
    );
    if let MessagePart::Tool {
        subagent_ref,
        delegate_route,
        ..
    } = &mut part
    {
        *subagent_ref = child.map(str::to_owned);
        *delegate_route = route;
    }
    part
}

fn started_thread(id: &str, title: &str, chat_id: &str) -> MessagePart {
    let mut part = tool(
        id,
        ToolCall::Mcp {
            server: "zeron".into(),
            tool: "create_thread".into(),
            input: Some(serde_json::json!({ "title": title })),
        },
        false,
        None,
    );
    if let MessagePart::Tool { subagent_ref, .. } = &mut part {
        *subagent_ref = Some(chat_id.into());
    }
    part
}

fn from_thread(id: &str, at: i64, from: &str, body: &str) -> SessionMessageEntry {
    let mut entry = user(id, at, body);
    entry.from_chat_id = Some(from.into());
    entry
}

fn system(id: &str, host: &str, at: i64, parts: Vec<MessagePart>) -> SessionMessageEntry {
    entry(id, MessageRole::System, host, at, parts)
}

fn seams(host: &str, now: i64) -> Vec<SessionMessageEntry> {
    let packet = "User: Bench the layout pass on the A17.\nAssistant: p95 is 3.1ms; the markdown prepare dominates.\nUser: Can the prepare move off the main thread?\nAssistant: It already runs on the layout worker.";
    vec![
        user("m1", now - 600_000, "Bench the layout pass on the A17."),
        assistant(
            "m2",
            host,
            now - 590_000,
            vec![text("t0", "p95 is **3.1ms**; the markdown prepare dominates.")],
        ),
        system(
            "m3",
            host,
            now - 500_000,
            vec![MessagePart::Handoff {
                id: "h0".into(),
                harness: zeron_proto::HarnessId::ClaudeCode,
                text: packet.into(),
                route: zeron_doc::parts::HandoffRoute {
                    from_harness: Some(zeron_proto::HarnessId::Codex),
                    from_model: Some("gpt-5.6-sol".into()),
                    to_model: Some("claude-opus-5".into()),
                    source_chat_id: Some("chat-pi".into()),
                    source_title: Some("Bench the layout pass".into()),
                    kind: Default::default(),
                },
            }],
        ),
        user(
            "m4",
            now - 400_000,
            "Now profile the scroll path:\n\n- frame time at `120Hz`\n- the $O(n)$ row walk on each fling",
        ),
        system(
            "m5",
            host,
            now - 399_000,
            vec![MessagePart::Classifier {
                id: "k0".into(),
                record: zeron_doc::parts::ClassifierRecord {
                    harness: Some(zeron_proto::HarnessId::ClaudeCode),
                    model: Some("claude-sonnet-5".into()),
                    verdict: zeron_doc::parts::ClassifierVerdict::Carry,
                    classifier_verdict: None,
                    decided_by: zeron_doc::parts::DecidedBy::Stage1,
                    duration_ms: 820,
                    context_tokens: 48_200,
                    input: "Now profile the scroll path:\n\n- frame time at `120Hz`\n- the $O(n)$ row walk on each fling".into(),
                    reply: "CARRY: same benchmark thread, new surface.".into(),
                    stage2_reply: None,
                    input_tokens: Some(1_240),
                    output_tokens: Some(12),
                },
            }],
        ),
        assistant(
            "m6",
            host,
            now - 380_000,
            vec![text("t0", "Scroll stays under 2ms per frame.")],
        ),
        system(
            "m7",
            host,
            now - 300_000,
            vec![
                text("t0", "Context compacted"),
                text("t1", "Older turns were summarized to free the context window."),
            ],
        ),
        system("m8", host, now - 200_000, vec![text("t0", "Host restarted")]),
        user("m9", now - 120_000, zeron_proto::RESUME_PROMPT),
        assistant(
            "m10",
            host,
            now - 100_000,
            vec![
                tool(
                    "v0",
                    ToolCall::Mcp {
                        server: "zeron".into(),
                        tool: "device_open".into(),
                        input: None,
                    },
                    false,
                    None,
                ),
                tool(
                    "v1",
                    ToolCall::Mcp {
                        server: "zeron".into(),
                        tool: "device_screenshot".into(),
                        input: None,
                    },
                    false,
                    Some(
                        r#"{"device":{"name":"iPhone 17 Pro"},"screenshot":{"mimeType":"image/png","width":1206,"height":2622}}"#,
                    ),
                ),
                tool(
                    "a0",
                    ToolCall::Unknown {
                        name: zeron_proto::ADVISOR_TOOL_NAME.into(),
                        input: None,
                    },
                    false,
                    None,
                ),
                text("t1", "Back on the scroll profile."),
            ],
        ),
        user(
            "m11",
            now - 60_000,
            &format!(
                "Address these before merging.\n\n{}\n- crates/mobile/src/layout/rows.rs:412 (R): The divider should keep its hint.\n\n{}\n> Pull request #94 in zeron-sh/zeron: Transcript seams on iOS\n> Adds peer rows, delegate chips and dividers to the phone transcript.\n\n> Checks: 2 of 3 passed\n> ios-build: pending",
                zeron_proto::badges::COMMENT_BLOCK_HEADER,
                zeron_proto::badges::PR_CONTEXT_HEADER,
            ),
        ),
        assistant(
            "m12",
            host,
            now - 50_000,
            vec![
                tool(
                    "t-skill",
                    ToolCall::Unknown {
                        name: "Skill".into(),
                        input: Some(serde_json::json!({ "skill": "review" })),
                    },
                    false,
                    Some("Launching skill: review"),
                ),
                text("t0", "Kept the hint on the divider; `ios-build` is green now."),
            ],
        ),
    ]
}

/// Fixture transcript for a demo chat (`None` = starts empty).
pub(crate) fn fixture(chat_id: &str, host: &str, last_activity: i64) -> Vec<SessionMessageEntry> {
    let now = last_activity;
    match chat_id {
        "chat-veil" => veil(host, crate::now_ms()),
        "chat-picker" => picker(host, now),
        "chat-tabs" => tabs(host, now),
        "chat-approve-test" => approval(
            host,
            now,
            "Run the mobile tests before we tag.",
            "Running the mobile crate's tests first.",
            (
                "Approve command",
                "Codex wants to run `cargo test -p zeron-mobile`. Allow it?",
                ["Yes", "No"],
            ),
        ),
        "chat-approve-push" => approval(
            host,
            now,
            "Ship the release branch.",
            "The branch is ready; publishing it needs a push.",
            (
                "Allow Bash?",
                "Claude wants to use Bash: git push origin release",
                ["Allow", "Deny"],
            ),
        ),
        "chat-errored" => errored(host, now),
        "chat-ios-scroll" => scroll(host, now),
        "chat-home" => home(host, now),
        "chat-cjk" => cjk(host, now),
        "chat-computer" => computer(host, now),
        "chat-visuals" => visuals(host, now),
        "chat-deploy" => short(
            host,
            now,
            "Audit the wrangler config for hibernation hygiene.",
            "Flush timer now only arms while dirty; ping/pong uses the auto-response path so the DO never wakes for keepalives.",
        ),
        "chat-ios-keyboard" => short(
            host,
            now,
            "The composer jumps when the keyboard animates in.",
            "Tracking `keyboardLayoutGuide` instead of the notification frame fixes it — the composer now rides the guide with the system curve.",
        ),
        "chat-blog" => short(
            host,
            now,
            "Draft the launch post outline.",
            "## Outline\n\n1. Why a phone viewport\n2. The CRDT under the hood\n3. What's next",
        ),
        "chat-side" => short(
            host,
            now,
            "What EMA window does the veil use?",
            "α = 0.2 over inter-append gaps, seeded from the first two chunks.",
        ),
        "chat-oklch" => short(
            host,
            now,
            "OKLCH conversion drifts from the desktop.",
            "Gamma encode matches now.",
        ),
        "chat-presence" => short(
            host,
            now,
            "Presence beats spam the DO.",
            "Batched to one beat per 25s.",
        ),
        "chat-lead-ios" => short(
            host,
            now,
            "Plan the iOS install flow: install sheet, sign-in, TestFlight, then push, the widget and release notes.",
            "The plan is ready: six steps, one screen each. I'll wait for your review before step 1.",
        ),
        "supervisor-dev-mac" => short(
            host,
            now,
            "Wake: step failed, needs you on Build iOS App Install.",
            "Asked you whether to ship to TestFlight before the widget; the lead holds step 3 until you answer.",
        ),
        "supervisor-dev-mini" => short(
            host,
            now,
            "Wake: step failed on Inbox settle shelf.",
            "Re-briefed step 3 to use the system swipe actions.",
        ),
        "chat-agents" => agents(host, now),
        "chat-cursor" => short(
            host,
            now,
            "Raise the tab bar hit targets",
            "Raised every tab to **44pt** and kept the icons centered.",
        ),
        "chat-seams" => seams(host, now),
        "chat-shelf-4" => shelf(host, now),
        _ => Vec::new(),
    }
}

const PROSE: &str = r#"The dropdown's open handler awaits `loadRefs()` **before** it paints, so
the menu can't render until the full ref index resolves. On a repo with
many refs that's a visible hang, and it is paid again on every open
because the result is never memoized between mounts.

Three things stack up here:

1. `loadRefs()` walks every ref and builds a fresh array each call
2. The handler `await`s it inline instead of rendering an empty menu
3. `useRefIndex` has no cache, so remount re-does the whole walk

| Stage | Cost | Cached |
| --- | --- | --- |
| `loadRefs` | O(refs) | no |
| `useRefIndex` | O(refs) | no |
| paint | O(visible) | n/a |

> The fix is to paint first and fill in — the index can arrive late.

```ts
const refs = useRefIndex()          // memoized, suspense-free
useEffect(() => { void warmRefIndex() }, [])
return <Menu items={refs ?? []} loading={refs == null} />
```"#;

/// `turns` synthetic user/assistant pairs (legacy BenchRunner shape).
pub(crate) fn synthetic(turns: u32, host: &str, start: i64) -> Vec<SessionMessageEntry> {
    let mut out = Vec::with_capacity(turns as usize * 2);
    for i in 0..turns as i64 {
        let at = start + i * 60_000;
        out.push(user(
            &format!("u{i}"),
            at,
            &format!("Turn {i}: the ref dropdown still hangs on open — dig into it."),
        ));
        let mut parts = vec![text(
            "t0",
            &format!("## Pass {i}: where the dropdown stalls\n\n{PROSE}"),
        )];
        for t in 0..4i64 {
            let index = i * 4 + t;
            let call = match index % 4 {
                0 => exec("rg -n 'refDropdown' src/components --glob '!*.test.ts'"),
                1 => read("src/components/refs/RefDropdown.tsx"),
                2 => ToolCall::EditFile {
                    path: "src/components/refs/useRefIndex.ts".into(),
                    old_string: None,
                    new_string: None,
                },
                _ => ToolCall::Search {
                    pattern: "loadRefs\\(".into(),
                    path: None,
                },
            };
            parts.push(tool(&format!("k{i}.{t}"), call, index % 17 == 0, None));
        }
        parts.push(text(
            "t1",
            &format!("Landed the pass-{i} change behind `refIndexCache`. Open latency drops to\na paint, and the index warms in the background on first hover."),
        ));
        out.push(assistant(&format!("a{i}"), host, at + 1_000, parts));
    }
    out
}

/// One step of a scripted assistant turn.
pub(crate) enum Step {
    Reasoning(String),
    Text(String),
    Tool {
        call: ToolCall,
        output: Option<String>,
        is_error: bool,
        run_ms: u64,
    },
    Question(Vec<UserInputQuestion>),
}

const REPLY: &str = r#"Here's how the streamed reply renders on this device:

- Markdown re-parses **only the tail** — the last two top-level blocks
- New text fades in through the paint-only veil
- The transcript stays glued to the bottom until you scroll up

```rust
// The desktop constant carries over verbatim.
const STREAM_COMMIT_MS: u64 = 120;
```"#;

const REPLY_TAIL: &str = r#"| Stage | Per token |
| --- | --- |
| doc decode | one entry |
| row rebuild | one row |

When the turn settles, this entry flips `streaming → complete`, the veil drops, and the row ids stay stable so nothing flickers."#;

/// The scripted reply to a prompt (the legacy demo responder, plus a thinking
/// block and a tool group so every live-row kind streams).
pub(crate) fn reply(prompt: &str, long: bool) -> Vec<Step> {
    let first_line = prompt.lines().next().unwrap_or("").trim();
    let quoted: String = first_line.chars().take(80).collect();
    let body = if long {
        [REPLY; 12].join("\n\n---\n\n")
    } else {
        REPLY.to_owned()
    };
    vec![
        Step::Reasoning(format!(
            "The user asked: \"{quoted}\". Walk through the streaming path end to end and show a small code sample."
        )),
        Step::Text(body),
        Step::Tool {
            call: read("crates/doc/src/transcript_delta.rs"),
            output: None,
            is_error: false,
            run_ms: 350,
        },
        Step::Tool {
            call: exec("cargo test -p zeron-client transcript"),
            output: Some("test result: ok. 12 passed; 0 failed".into()),
            is_error: false,
            run_ms: 900,
        },
        Step::Text(REPLY_TAIL.to_owned()),
    ]
}

/// Reply after answering a question panel.
pub(crate) fn answered(labels: &[String]) -> Vec<Step> {
    let picked = if labels.is_empty() {
        "your defaults".to_owned()
    } else {
        labels.join(", ")
    };
    vec![Step::Text(format!(
        "Going with **{picked}**. Wiring `ListModels` to the session's host device now — the picker will refresh whenever the host changes."
    ))]
}

pub(crate) fn consent_answered(outcome: Option<ConsentOutcome>) -> Vec<Step> {
    let reply = match outcome {
        Some(ConsentOutcome::Allowed | ConsentOutcome::AlwaysAllowed) => {
            "Pasted 4 into Terminal, so the sum checks out."
        }
        _ => "Stopping here: I won't touch the desktop without your go-ahead.",
    };
    vec![Step::Text(reply.into())]
}

/// A reply that ends by asking a question (prompts containing "?ask").
pub(crate) fn asking() -> Vec<Step> {
    vec![
        Step::Text("Before I continue I need one decision:".into()),
        Step::Question(vec![UserInputQuestion {
            approval: false,
            id: "q1".into(),
            header: "Scope".into(),
            question: "Should the fix cover Android too?".into(),
            options: vec!["Yes, both platforms".into(), "iOS only".into()],
            prefill: None,
            multiline: false,
            multi_select: false,
            kind: None,
        }]),
    ]
}
