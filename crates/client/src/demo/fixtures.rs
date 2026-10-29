//! The demo workspace: devices, projects, chats (pinned, sectioned,
//! projectless, child, archived), live statuses, and PRs in every state.

use chrono::Utc;
use zeron_doc::RegistryDoc;
use zeron_proto::{
    ChangeRequestState, ChangeRequestSummary, Chat, ChatConfig, CheckoutChangeRequestStatus,
    ChildRoute, ConversationSourceContext, Delegation, DelegationStatus, DelegationWake, Device,
    HarnessId, LimitOffer, LimitResume, LimitResumes, OfferChoice, ReasoningLevel, RouteTarget,
    SandboxLevel, Session, SessionStatus, SidebarPinChange, SidebarSectionChange, SidebarSettle,
    Space,
};

use crate::client::ms;
use crate::config::DemoFixture;
use crate::rpc::capability;

pub(crate) const MAC: &str = "dev-mac";
pub(crate) const VPS: &str = "dev-vps";
pub(crate) const STUDIO: &str = "dev-studio";
pub(crate) const MINI: &str = "dev-mini";

pub(crate) struct DemoChat {
    pub id: &'static str,
    pub space: Option<&'static str>,
    pub device: &'static str,
    pub title: &'static str,
    pub preview: &'static str,
    pub harness: HarnessId,
    pub model: &'static str,
    pub reasoning: ReasoningLevel,
    pub branch: Option<&'static str>,
    pub pr: Option<(u64, ChangeRequestState, &'static str)>,
    pub status: Option<SessionStatus>,
    pub last_ago_ms: i64,
    pub created_ago_ms: i64,
    pub seen: bool,
    pub archived: bool,
    pub parent: Option<&'static str>,
    pub delegated: Option<(DelegationStatus, Option<ChildRoute>)>,
    pub compacting: bool,
    pub side_of: Option<&'static str>,
    /// (settled, how long ago) of a Settle / Unsettle.
    pub settle: Option<(bool, i64)>,
    pub previous: &'static [HarnessId],
    pub instance: Option<&'static str>,
}

/// Each host's `settle_after_days` (`GetSidebarSettings`); the rest keep the default.
pub(crate) const SETTLE_DAYS: [(&str, u32); 2] = [(MAC, 3), (MINI, 7)];

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;

pub(crate) fn chats() -> Vec<DemoChat> {
    use ChangeRequestState::*;
    use HarnessId::*;
    use ReasoningLevel::*;
    let chat = |id, space, device, title, preview| DemoChat {
        id,
        space,
        device,
        title,
        preview,
        harness: ClaudeCode,
        model: "claude-fable-5",
        reasoning: XHigh,
        branch: None,
        pr: None,
        status: None,
        last_ago_ms: HOUR,
        created_ago_ms: DAY,
        seen: true,
        archived: false,
        parent: None,
        delegated: None,
        compacting: false,
        side_of: None,
        settle: None,
        previous: &[],
        instance: None,
    };
    vec![
        DemoChat {
            previous: &[Codex],
            branch: Some("veil-fade"),
            pr: Some((90, Open, "Stream pull request status on every client")),
            status: Some(SessionStatus::Working),
            last_ago_ms: 40_000,
            created_ago_ms: HOUR,
            ..chat(
                "chat-veil",
                Some("space-zeron"),
                MAC,
                "Streaming veil on transcript rows",
                "Opening the PR now. Running the checks first:",
            )
        },
        DemoChat {
            branch: Some("catalog-sync"),
            pr: Some((84, Merged, "Synchronize model catalogs")),
            status: Some(SessionStatus::AwaitingInput),
            last_ago_ms: 2 * MIN,
            created_ago_ms: 2 * HOUR,
            seen: false,
            model: "claude-opus-5-5",
            ..chat(
                "chat-picker",
                Some("space-zeron"),
                MAC,
                "Model picker catalog sync",
                "Before I wire the RPC, two decisions:",
            )
        },
        DemoChat {
            harness: Codex,
            model: "gpt-5.6-terra",
            reasoning: High,
            status: Some(SessionStatus::AwaitingInput),
            last_ago_ms: 5 * MIN,
            seen: false,
            ..chat(
                "chat-approve-test",
                Some("space-edge"),
                VPS,
                "Run: cargo test",
                "Codex wants to run the test suite.",
            )
        },
        DemoChat {
            status: Some(SessionStatus::AwaitingInput),
            last_ago_ms: 9 * MIN,
            seen: false,
            ..chat(
                "chat-approve-push",
                Some("space-zeron"),
                MAC,
                "Publish the release branch",
                "Claude wants to push to origin.",
            )
        },
        DemoChat {
            harness: Codex,
            model: "gpt-5.6-terra",
            reasoning: High,
            branch: Some("fix/tool-colors"),
            pr: Some((77, Closed, "Refine tool group colors")),
            last_ago_ms: 15 * MIN,
            seen: false,
            ..chat(
                "chat-tabs",
                Some("space-zeron"),
                MAC,
                "Tool group header colors",
                "Done — failed children stay quiet.",
            )
        },
        DemoChat {
            status: Some(SessionStatus::Errored),
            last_ago_ms: 33 * MIN,
            seen: false,
            ..chat(
                "chat-errored",
                Some("space-edge"),
                VPS,
                "Durable Object hibernation flush",
                "Harness exited with status 1: rate limited",
            )
        },
        DemoChat {
            last_ago_ms: DAY,
            created_ago_ms: 2 * DAY,
            ..chat(
                "chat-deploy",
                Some("space-edge"),
                VPS,
                "Wrangler deploy hygiene",
                "Hibernation-safe flush timer",
            )
        },
        DemoChat {
            branch: Some("scroll-pinning"),
            pr: Some((
                102,
                Open,
                "Clamp transcript offset on every geometry change",
            )),
            last_ago_ms: 80 * MIN,
            seen: false,
            harness: Codex,
            model: "gpt-6-astra",
            reasoning: Ultra,
            ..chat(
                "chat-ios-scroll",
                Some("space-mobile"),
                MAC,
                "Transcript scroll pinning",
                "Clamp the invariant on every geometry change",
            )
        },
        DemoChat {
            last_ago_ms: 5 * HOUR,
            ..chat(
                "chat-ios-keyboard",
                Some("space-mobile"),
                MAC,
                "Keyboard avoidance for composer",
                "Tracking keyboardLayoutGuide fixes it",
            )
        },
        DemoChat {
            last_ago_ms: 2 * HOUR,
            model: "claude-sonnet-5",
            reasoning: Medium,
            ..chat(
                "chat-home",
                None,
                MAC,
                "Clean up ~/Downloads",
                "Found 412 items older than 90 days.",
            )
        },
        DemoChat {
            last_ago_ms: 2 * MIN,
            ..chat(
                "chat-visuals",
                None,
                MAC,
                "Agent visuals",
                "A dashboard, a canvas, a scene and a shader.",
            )
        },
        DemoChat {
            last_ago_ms: 10 * MIN,
            ..chat(
                "chat-cjk",
                Some("space-zeron-vps"),
                VPS,
                "多言語テキストのレイアウト 🌏",
                "日本語の長い段落です。",
            )
        },
        DemoChat {
            last_ago_ms: 5 * DAY,
            created_ago_ms: 6 * DAY,
            // Started without picking a model: it runs the default.
            model: "",
            ..chat(
                "chat-blog",
                Some("space-blog"),
                STUDIO,
                "Draft the launch post",
                "## Outline",
            )
        },
        DemoChat {
            last_ago_ms: DAY + MIN,
            created_ago_ms: 2 * DAY,
            ..chat(
                "chat-computer",
                Some("space-zeron"),
                MAC,
                "Calculator check with Computer",
                "Allow Computer for this task?",
            )
        },
        DemoChat {
            parent: Some("chat-veil"),
            last_ago_ms: 30 * MIN,
            ..chat(
                "chat-side",
                Some("space-zeron"),
                MAC,
                "Side chat: veil timing",
                "α = 0.2 over inter-append gaps",
            )
        },
        DemoChat {
            side_of: Some("chat-blog"),
            last_ago_ms: 26 * HOUR,
            created_ago_ms: 30 * HOUR,
            ..chat(
                "chat-btw",
                Some("space-zeron"),
                MAC,
                "btw: launch post tone",
                "Keep it plain; no superlatives.",
            )
        },
        DemoChat {
            parent: Some("chat-veil"),
            harness: Codex,
            model: "gpt-5.6-terra",
            reasoning: High,
            last_ago_ms: 6 * MIN,
            ..chat(
                "chat-side-easing",
                Some("space-zeron"),
                MAC,
                "Spring vs. power curve for the veil",
                "Keep the curve; use the spring only for the follow scroll.",
            )
        },
        DemoChat {
            archived: true,
            last_ago_ms: 3 * DAY,
            created_ago_ms: 4 * DAY,
            ..chat(
                "chat-oklch",
                Some("space-zeron"),
                MAC,
                "OKLCH conversion drift",
                "Gamma encode matches now.",
            )
        },
        DemoChat {
            archived: true,
            harness: Codex,
            model: "gpt-5.5",
            reasoning: High,
            last_ago_ms: 6 * DAY,
            created_ago_ms: 7 * DAY,
            ..chat(
                "chat-presence",
                Some("space-edge"),
                VPS,
                "Presence beat coalescing",
                "Batched to one beat per 25s.",
            )
        },
        DemoChat {
            harness: Opencode,
            model: "default",
            last_ago_ms: 2 * HOUR,
            ..chat(
                "chat-agents",
                Some("space-agents"),
                MAC,
                "Fan out the parity audit",
                "Four agents reported back.",
            )
        },
        DemoChat {
            harness: Cursor,
            model: "default",
            last_ago_ms: 4 * HOUR,
            parent: Some("chat-agents"),
            delegated: Some((
                DelegationStatus::Completed,
                Some(ChildRoute {
                    source: ChildRoute::ROUTED.into(),
                    role: Some("reviewer".into()),
                    table: Some(RouteTarget {
                        harness: Cursor,
                        model: Some("default".into()),
                        reasoning: None,
                    }),
                    ..Default::default()
                }),
            )),
            ..chat(
                "chat-cursor",
                Some("space-agents"),
                MAC,
                "Tab bar hit targets",
                "Raised to 44pt.",
            )
        },
        DemoChat {
            harness: Devin,
            model: "swe-1-7-medium",
            last_ago_ms: 6 * HOUR,
            parent: Some("chat-agents"),
            delegated: Some((
                DelegationStatus::Running,
                Some(ChildRoute {
                    source: ChildRoute::FELL_BACK.into(),
                    role: Some("debugger".into()),
                    table: Some(RouteTarget {
                        harness: Codex,
                        model: Some("gpt-5.6-sol".into()),
                        reasoning: None,
                    }),
                    reason: Some("Codex out of quota".into()),
                    ..Default::default()
                }),
            )),
            ..chat(
                "chat-devin",
                Some("space-agents"),
                MAC,
                "Flaky e2e on CI",
                "Seeded the clock.",
            )
        },
        DemoChat {
            harness: Grok,
            model: "grok-4.5",
            last_ago_ms: 8 * HOUR,
            parent: Some("chat-agents"),
            delegated: Some((DelegationStatus::Failed, None)),
            ..chat(
                "chat-grok",
                Some("space-agents"),
                MAC,
                "Edge cache headers",
                "Vary on Accept-Encoding.",
            )
        },
        DemoChat {
            harness: Hermes,
            model: "hermes-4-70b",
            last_ago_ms: 10 * HOUR,
            ..chat(
                "chat-hermes",
                Some("space-agents"),
                MAC,
                "Changelog draft",
                "Grouped by surface.",
            )
        },
        DemoChat {
            harness: Pi,
            model: "default",
            last_ago_ms: 12 * HOUR,
            ..chat(
                "chat-pi",
                Some("space-agents"),
                MAC,
                "Bench the layout pass",
                "p95 at 3.1ms.",
            )
        },
        DemoChat {
            harness: Antigravity,
            model: "gemini-3.1-pro",
            last_ago_ms: 14 * HOUR,
            ..chat(
                "chat-antigravity",
                Some("space-agents"),
                MAC,
                "Android bindings",
                "UniFFI Kotlin builds.",
            )
        },
        DemoChat {
            model: "claude-opus-5",
            last_ago_ms: 3 * MIN,
            created_ago_ms: 40 * MIN,
            ..chat(
                "chat-lead-ios",
                Some("space-zeron"),
                MAC,
                "Build iOS App Install",
                "The plan is ready: six steps, one screen each.",
            )
        },
        DemoChat {
            last_ago_ms: 2 * MIN,
            created_ago_ms: DAY,
            ..chat(
                "supervisor-dev-mac",
                None,
                MAC,
                "Supervisor",
                "Asked you about TestFlight before the widget.",
            )
        },
        DemoChat {
            last_ago_ms: 6 * MIN,
            created_ago_ms: DAY,
            ..chat(
                "supervisor-dev-mini",
                None,
                MINI,
                "Supervisor",
                "Re-briefed step 3 after the swipe failure.",
            )
        },
        DemoChat {
            model: "claude-opus-5",
            last_ago_ms: 5 * MIN,
            created_ago_ms: 25 * MIN,
            ..chat(
                "chat-lead-widget",
                Some("space-zeron"),
                MAC,
                "Needs you widget",
                "The Supervisor approved the plan; step 1 starts after the hold.",
            )
        },
        DemoChat {
            model: "claude-opus-5",
            status: Some(SessionStatus::Working),
            last_ago_ms: 30_000,
            created_ago_ms: 3 * HOUR,
            ..chat(
                "chat-lead-shelf",
                Some("space-fork"),
                MINI,
                "Inbox settle shelf",
                "Step 3 is on the swipe actions.",
            )
        },
        DemoChat {
            harness: Codex,
            model: "gpt-5.6-terra",
            reasoning: High,
            last_ago_ms: 2 * HOUR,
            parent: Some("chat-lead-shelf"),
            ..chat(
                "chat-shelf-1",
                Some("space-fork"),
                MINI,
                "Settled fold",
                "The Settled fold sits under Inbox.",
            )
        },
        DemoChat {
            harness: Codex,
            model: "gpt-5.6-terra",
            reasoning: High,
            last_ago_ms: 90 * MIN,
            parent: Some("chat-lead-shelf"),
            ..chat(
                "chat-shelf-2",
                Some("space-fork"),
                MINI,
                "Slim rows",
                "Settled rows drop the preview line.",
            )
        },
        DemoChat {
            harness: Codex,
            model: "gpt-5.6-terra",
            reasoning: High,
            status: Some(SessionStatus::Working),
            last_ago_ms: 20_000,
            parent: Some("chat-lead-shelf"),
            ..chat(
                "chat-shelf-3",
                Some("space-fork"),
                MINI,
                "Swipe Settle and Unsettle",
                "Wiring the trailing swipe action.",
            )
        },
        DemoChat {
            status: Some(SessionStatus::AwaitingInput),
            last_ago_ms: 4 * MIN,
            seen: false,
            parent: Some("chat-lead-shelf"),
            ..chat(
                "chat-shelf-4",
                Some("space-fork"),
                MINI,
                "Settle after days",
                "One question before I read the setting:",
            )
        },
        DemoChat {
            last_ago_ms: 3 * HOUR,
            settle: Some((true, 2 * HOUR)),
            ..chat(
                "chat-settled",
                Some("space-zeron"),
                MAC,
                "Changelog wording",
                "Merged; nothing left here.",
            )
        },
        DemoChat {
            last_ago_ms: 4 * DAY,
            created_ago_ms: 5 * DAY,
            ..chat(
                "chat-old-mac",
                Some("space-zeron"),
                MAC,
                "Flaky snapshot test",
                "Pinned the clock; it passes 200 runs in a row.",
            )
        },
        DemoChat {
            harness: Codex,
            model: "gpt-5.6-terra",
            reasoning: High,
            last_ago_ms: 4 * DAY,
            created_ago_ms: 5 * DAY,
            ..chat(
                "chat-old-mini",
                Some("space-fork"),
                MINI,
                "Bench the sync relay",
                "p99 is 41 ms on the Mac mini.",
            )
        },
        DemoChat {
            last_ago_ms: 9 * DAY,
            created_ago_ms: 10 * DAY,
            settle: Some((false, 50 * MIN)),
            ..chat(
                "chat-unsettled",
                Some("space-zeron"),
                MAC,
                "Font fallback audit",
                "CJK falls back to PingFang; Latin stays on Inter.",
            )
        },
        DemoChat {
            harness: Acp,
            model: "default",
            instance: Some(super::composer::ACP_INSTANCE),
            last_ago_ms: 16 * HOUR,
            ..chat(
                "chat-acp",
                Some("space-agents"),
                MAC,
                "Registry agent smoke test",
                "Handshake ok.",
            )
        },
        DemoChat {
            harness: ClaudeCode,
            model: "claude-opus-5",
            last_ago_ms: 18 * HOUR,
            status: Some(SessionStatus::Idle),
            compacting: true,
            ..chat(
                "chat-seams",
                Some("space-agents"),
                MAC,
                "Pick up the layout bench",
                "Resumed after the restart.",
            )
        },
        DemoChat {
            last_ago_ms: 8 * DAY,
            created_ago_ms: 9 * DAY,
            ..chat(
                "chat-old-1",
                Some("space-blog"),
                STUDIO,
                "Old relay spike",
                "Dropped it; the relay keeps its queue.",
            )
        },
        DemoChat {
            last_ago_ms: 9 * DAY,
            created_ago_ms: 10 * DAY,
            ..chat(
                "chat-old-2",
                Some("space-blog"),
                STUDIO,
                "Docs pass",
                "Every page builds without warnings.",
            )
        },
        DemoChat {
            last_ago_ms: 10 * DAY,
            created_ago_ms: 11 * DAY,
            ..chat(
                "chat-old-3",
                Some("space-blog"),
                STUDIO,
                "Tidy the theme tokens",
                "Removed the six unused colors.",
            )
        },
        DemoChat {
            last_ago_ms: 11 * DAY,
            created_ago_ms: 12 * DAY,
            ..chat(
                "chat-old-4",
                Some("space-blog"),
                STUDIO,
                "Crash on empty project",
                "Guarded the empty path and added a test.",
            )
        },
        DemoChat {
            last_ago_ms: 12 * DAY,
            created_ago_ms: 13 * DAY,
            ..chat(
                "chat-old-5",
                Some("space-blog"),
                STUDIO,
                "Bump the toolchain",
                "Builds on the new compiler.",
            )
        },
        DemoChat {
            last_ago_ms: 13 * DAY,
            created_ago_ms: 14 * DAY,
            ..chat(
                "chat-old-6",
                Some("space-blog"),
                STUDIO,
                "Retry budget for uploads",
                "Three tries, then the row shows the error.",
            )
        },
        DemoChat {
            last_ago_ms: 14 * DAY,
            created_ago_ms: 15 * DAY,
            ..chat(
                "chat-old-7",
                Some("space-blog"),
                STUDIO,
                "Trim the changelog script",
                "Down to one pass over the commits.",
            )
        },
        DemoChat {
            last_ago_ms: 15 * DAY,
            created_ago_ms: 16 * DAY,
            ..chat(
                "chat-old-8",
                Some("space-blog"),
                STUDIO,
                "Window restore on launch",
                "Restores size and position per display.",
            )
        },
        DemoChat {
            last_ago_ms: 16 * DAY,
            created_ago_ms: 17 * DAY,
            ..chat(
                "chat-old-9",
                Some("space-blog"),
                STUDIO,
                "Icon cache warmup",
                "Warm cache cuts first paint by 80 ms.",
            )
        },
    ]
}

pub(crate) fn spaces(now: i64) -> Vec<Space> {
    let space =
        |id: &str, device: &str, path: &str, name: Option<&str>, git: bool, ago: i64| Space {
            id: id.into(),
            device_id: device.into(),
            path: path.into(),
            name: name.map(str::to_owned),
            git_detected: git,
            git_checked_at: git.then(|| ms(now)),
            checkout_id: git.then(|| format!("co-{id}")),
            repository_id: None,
            github_repo: None,
            created_at: ms(now - ago),
        };
    let zeron = |mut space: Space| {
        space.repository_id = Some("github.com/zeronsh/zeron".into());
        // Cloud reaches it in the demo (its fixture repositories).
        space.github_repo = Some("acme/web".into());
        space
    };
    vec![
        space(
            "space-blog",
            STUDIO,
            "/Users/dev/Projects/blog",
            None,
            false,
            20 * DAY,
        ),
        zeron(space(
            "space-zeron",
            MAC,
            "/Users/dev/zeron",
            None,
            true,
            9 * DAY,
        )),
        // A second clone of the same repository on the VPS.
        zeron(space(
            "space-zeron-vps",
            VPS,
            "/srv/src/zeron",
            None,
            true,
            6 * DAY,
        )),
        space("space-edge", VPS, "/srv/deploys/edge", None, true, 4 * DAY),
        space(
            "space-mobile",
            MAC,
            "/Users/dev/zeron-ios",
            Some("Zeron iOS"),
            true,
            2 * DAY,
        ),
        space(
            "space-fork",
            MINI,
            "/Users/dev/zeron-fork",
            None,
            true,
            12 * HOUR,
        ),
        space(
            "space-agents",
            MAC,
            "/Users/dev/agents",
            Some("Agents"),
            false,
            DAY,
        ),
    ]
}

pub(crate) fn devices(self_id: &str, self_name: &str, now: i64) -> Vec<Device> {
    let device =
        |id: &str, name: &str, platform: &str, version: &str, caps: bool, seen: i64| Device {
            id: id.into(),
            name: name.into(),
            platform: platform.into(),
            last_seen_at: Some(ms(seen)),
            created_at: Some(ms(now - 30 * DAY)),
            version: Some(version.into()),
            cursor_sdk_version: None,
            capabilities: if caps {
                use zeron_proto::capabilities as host;
                capability::ALL_QUEUE
                    .iter()
                    .chain([&capability::PROVIDER_SWITCH_V1])
                    .chain(&[
                        host::PULL_REQUESTS_PAGE_V1,
                        host::SOURCE_CONTROL_V1,
                        host::SOURCE_CONTROL_SYNC_V1,
                        host::SOURCE_CONTROL_COMMITS_V1,
                    ])
                    .map(|c| (*c).to_owned())
                    .collect()
            } else {
                Vec::new()
            },
        };
    vec![
        device(
            MAC,
            "MacBook Pro",
            "macos",
            env!("CARGO_PKG_VERSION"),
            true,
            now,
        ),
        device(
            VPS,
            "hetzner-01",
            "linux",
            env!("CARGO_PKG_VERSION"),
            true,
            now,
        ),
        device(
            MINI,
            "Mac mini",
            "macos",
            env!("CARGO_PKG_VERSION"),
            true,
            now,
        ),
        device(
            STUDIO,
            "Mac Studio",
            "macos",
            "0.2.80",
            false,
            now - 3 * DAY,
        ),
        device(
            self_id,
            self_name,
            "ios",
            env!("CARGO_PKG_VERSION"),
            false,
            now,
        ),
    ]
}

/// Devices whose presence the demo keeps fresh.
pub(crate) const ONLINE: &[&str] = &[MAC, VPS, MINI];

pub(crate) struct Seeded {
    pub change_requests: Vec<CheckoutChangeRequestStatus>,
    pub limits: Vec<(&'static str, LimitResumes)>,
    pub missions: bool,
}

/// Write the whole dataset into the replica (as local writes, which the
/// local-only demo replica folds straight into its rows).
pub(crate) fn seed(
    doc: &mut RegistryDoc,
    fixture: DemoFixture,
    computer_still: bool,
    self_id: &str,
    self_name: &str,
) -> Result<Seeded, zeron_doc::DocError> {
    let now = crate::now_ms();
    let all_devices = devices(self_id, self_name, now);
    let mut change_requests = Vec::new();
    match fixture {
        DemoFixture::IosOnly => {
            if let Some(phone) = all_devices.iter().find(|d| d.id == self_id) {
                doc.upsert_device(phone)?;
            }
            doc.reconcile_sidebar_pins(true)?;
            return Ok(Seeded {
                change_requests,
                limits: Vec::new(),
                missions: false,
            });
        }
        DemoFixture::NoProjects => {
            for device in &all_devices {
                doc.upsert_device(device)?;
            }
            doc.reconcile_sidebar_pins(true)?;
            return Ok(Seeded {
                change_requests,
                limits: Vec::new(),
                missions: false,
            });
        }
        DemoFixture::Standard => {}
    }
    for device in &all_devices {
        doc.upsert_device(device)?;
    }
    let spaces = spaces(now);
    for space in &spaces {
        doc.upsert_space(space)?;
    }
    for mut demo in chats() {
        if computer_still && demo.id == "chat-computer" {
            demo.status = Some(SessionStatus::Working);
        }
        let space = demo.space.and_then(|id| spaces.iter().find(|s| s.id == id));
        let last = now - demo.last_ago_ms;
        let source_context = match (demo.branch, space) {
            (Some(branch), Some(space)) => Some(ConversationSourceContext {
                checkout_id: format!("co-{}", demo.id),
                repo_root: space.path.clone(),
                cwd: space.path.clone(),
                branch: branch.into(),
                head_sha: None,
                observed_at: ms(last),
            }),
            _ => None,
        };
        let chat = Chat {
            id: demo.id.into(),
            device_id: demo.device.into(),
            title: Some(demo.title.into()),
            archived: demo.archived,
            cwd: Some(space.map_or_else(|| "~".to_owned(), |s| s.path.clone())),
            branch: demo
                .branch
                .map(str::to_owned)
                .or_else(|| space.filter(|s| s.git_detected).map(|_| "main".to_owned())),
            checkout_id: source_context.as_ref().map(|s| s.checkout_id.clone()),
            source_context: source_context.clone(),
            config: Some(super::composer::with_profile(
                demo.id,
                ChatConfig {
                    harness: demo.harness,
                    model: (!demo.model.is_empty()).then(|| demo.model.into()),
                    reasoning: Some(demo.reasoning),
                    model_options: Default::default(),
                    sandbox: SandboxLevel::WorkspaceWrite,
                    permission_mode: Default::default(),
                    instructions: String::new(),
                    profile_id: None,
                    instance_id: demo.instance.map(str::to_owned),
                    disabled_tools: Vec::new(),
                    access: Default::default(),
                    plan: false,
                },
            )),
            last_message_preview: Some(demo.preview.into()),
            last_message_at: Some(ms(last)),
            created_at: ms(now - demo.created_ago_ms.max(demo.last_ago_ms + MIN)),
            harness_session_id: None,
            harness_session_cwd: None,
            harness_session_harness: None,
            space_id: demo.space.map(str::to_owned),
            last_seen_at: Some(ms(if demo.seen { last } else { last - MIN })),
            room_gen: Some(2),
            sidechat_source_chat_id: demo.side_of.map(str::to_owned),
            created_by_chat_id: demo.parent.map(str::to_owned),
            forked_from_chat_id: None,
            previous_harnesses: demo.previous.to_vec(),
            delegation: demo
                .parent
                .zip(demo.delegated.clone())
                .map(|(parent, (status, route))| Delegation {
                    delegated_by_chat_id: parent.into(),
                    depth: 1,
                    anchor: "m1".into(),
                    client_request_id: None,
                    owner_turn_started_at: None,
                    wake: DelegationWake::Always,
                    status,
                    cancel_requested: false,
                    delivery: None,
                    summary: None,
                    started_at: ms(last - 4 * MIN),
                    completed_at: (status != DelegationStatus::Running)
                        .then(|| ms(last - 2 * MIN + 14_000)),
                    finding_outcomes: Default::default(),
                    findings_handed: Default::default(),
                    findings_announced: Default::default(),
                    route,
                }),
            settle: demo.settle.map(|(settled, ago)| SidebarSettle {
                settled,
                at: ms(now - ago),
            }),
            unsettled_at: demo
                .settle
                .filter(|(settled, _)| !settled)
                .map(|(_, ago)| ms(now - ago)),
        };
        doc.upsert_chat(&chat)?;
        if chat.id == super::review::OWNER {
            doc.upsert_chat(&super::review::seeded_child(&chat))?;
        }
        if let Some(status) = demo.status {
            doc.upsert_session(&Session {
                last_completed_turn: None,
                last_turn_outcome: None,
                terminal_activity: None,
                running_subagents: 0,
                compact_plan: None,
                classifying: None,
                chat_id: demo.id.into(),
                device_id: demo.device.into(),
                status,
                started_at: Some(ms(now - 95_000)),
                updated_at: Utc::now(),
                held_by_subagents: false,
                compacting: demo.compacting,
                background_tasks: Vec::new(),
            })?;
        }
        if let (Some((number, state, title)), Some(source)) = (demo.pr, &source_context) {
            change_requests.push(CheckoutChangeRequestStatus {
                checkout_id: source.checkout_id.clone(),
                device_id: demo.device.into(),
                cwd: source.repo_root.clone(),
                branch: source.branch.clone(),
                change_request: Some(ChangeRequestSummary {
                    provider: "github".into(),
                    number,
                    title: title.into(),
                    url: format!("https://github.com/zeron-sh/zeron/pull/{number}"),
                    state,
                    base_ref: "main".into(),
                    head_ref: source.branch.clone(),
                }),
                updated_at: Utc::now(),
            });
        }
    }
    doc.reconcile_sidebar_pins(true)?;
    let mut after: Option<String> = None;
    for id in ["chat-veil", "chat-picker"] {
        doc.change_sidebar_pin(&SidebarPinChange::Pin {
            session_id: id.into(),
            after: after.clone(),
            before: None,
        })?;
        after = Some(id.into());
    }
    for (section, name, members) in [
        ("section-p0", "P0", &["chat-tabs", "chat-errored"][..]),
        (
            "section-mobile",
            "Mobile",
            &["chat-ios-scroll", "chat-ios-keyboard"][..],
        ),
    ] {
        doc.change_sidebar_pin(&SidebarPinChange::Section {
            change: SidebarSectionChange::Create {
                id: section.into(),
                name: name.into(),
            },
        })?;
        for member in members {
            doc.change_sidebar_pin(&SidebarPinChange::Section {
                change: SidebarSectionChange::Assign {
                    session_id: (*member).into(),
                    section_id: Some(section.into()),
                    after: None,
                    before: None,
                },
            })?;
        }
    }
    let limits = vec![(
        MAC,
        LimitResumes {
            auto_resume: true,
            chats: vec![LimitResume {
                chat_id: "chat-devin".into(),
                resets_at: Some(now + 72 * 60_000),
                auto: true,
                offer: Some(LimitOffer {
                    to: Some(RouteTarget {
                        harness: HarnessId::ClaudeCode,
                        model: Some("claude-opus-5".into()),
                        reasoning: None,
                    }),
                    preselect: OfferChoice::Switch,
                    act_at: Some(now + 5 * 60_000),
                }),
            }],
        },
    )];
    super::missions::seed(doc, Utc::now())?;
    Ok(Seeded {
        change_requests,
        limits,
        missions: true,
    })
}

pub(crate) fn today_reply(device_id: &str, method: &str, now_ms: i64) -> Option<serde_json::Value> {
    use serde_json::json;
    use zeron_rpc::methods as m;
    let min = 60_000;
    match method {
        m::GET_TODAY_RUNS => {
            let runs: &[(&str, i64, Option<i64>)] = match device_id {
                MAC => &[
                    ("chat-lead-shelf", 5, None),
                    ("chat-visuals", 25, Some(9)),
                    ("chat-lead-shelf", 40, Some(14)),
                    ("chat-blog", 70, Some(6)),
                    ("chat-visuals", 95, Some(18)),
                    ("chat-deploy", 130, Some(11)),
                    ("chat-blog", 180, Some(7)),
                ],
                MINI => &[
                    ("chat-old-mini", 12, Some(8)),
                    ("chat-old-mini", 55, Some(21)),
                    ("chat-approve-push", 150, Some(5)),
                ],
                _ => &[],
            };
            let since = chrono::DateTime::from_timestamp_millis(now_ms).map_or(0, |now| {
                zeron_proto::today::work_day_start(now.with_timezone(&chrono::Local))
                    .timestamp_millis()
            });
            // Early in the work day the offsets shrink so every run stays today.
            let (elapsed, span) = ((now_ms - since).max(0), 181 * min);
            Some(json!({
                "runs": runs
                    .iter()
                    .map(|(chat, ago, minutes)| json!({
                        "chatId": chat,
                        "startedAtMs": now_ms - ago * min * elapsed.min(span) / span,
                        "durationMs": minutes.map(|m| m * min),
                        "status": if minutes.is_some() { "complete" } else { "running" },
                    }))
                    .collect::<Vec<_>>(),
                "waits": [],
            }))
        }
        _ => None,
    }
}
