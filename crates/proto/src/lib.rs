//! zeron-proto — wire types shared by engine, UI, and RPC.
//!
//! Ported from zeron's `packages/control/src/wire.ts` + `packages/harness/src/types.ts`.
//! Context occupancy is replicated per chat; billing `Usage` remains a harness passthrough.

pub mod account_usage;
pub mod agent;
pub mod app_commands;
pub mod agent_messages;
pub mod attachment_mentions;
pub mod badges;
/// Stable capability error codes transported without provider stderr.
pub mod capability_errors {
    pub const PULL_REQUESTS_CLI_UNAVAILABLE: &str = "pull_requests.cli_unavailable";
    pub const PULL_REQUESTS_AUTHENTICATION: &str = "pull_requests.authentication";
    pub const PULL_REQUESTS_RATE_LIMITED: &str = "pull_requests.rate_limited";
    pub const PULL_REQUESTS_TIMEOUT: &str = "pull_requests.timeout";
    pub const PULL_REQUESTS_DECODE: &str = "pull_requests.decode";
    pub const PULL_REQUESTS_COMMAND_FAILED: &str = "pull_requests.command_failed";
}
pub mod background;
pub mod change_request_assessment;
pub mod chat_mentions;
pub mod chat_wallpaper;
pub mod claude_cards;
pub mod claude_config;
pub mod claude_panel;
pub mod command_catalog;
pub mod comments;
pub mod compaction;
pub mod computer;
pub mod device_hub;
pub mod cloud;
pub mod device_paths;
pub mod device_tools;
pub mod entities;
pub mod annotation;
pub mod file_mentions;
pub mod git_graph;
pub mod git_tab;
pub mod handoff;
pub mod home;
pub mod html_render;
pub mod invocation;
pub mod mission;
pub mod model_catalog;
pub mod model_options;
pub mod model_switch;
pub mod motion;
pub mod patch;
pub mod picker;
pub mod plan_usage;
pub mod prediction;
pub mod preview;
pub mod program_status;
pub mod projectless;
pub mod provider_instance;
pub mod providers;
pub mod pull_request;
pub mod quotes;
pub mod review;
pub mod route_preview;
pub mod routing;
pub mod run_errors;
pub mod sidebar_pins;
pub mod sidebar_view;
pub mod sidechat;
pub mod source_control;
pub mod stats_view;
pub mod supervisor;
pub mod today;
pub mod tools;
pub mod transcript;
pub mod ui_driver;
pub mod usage;
pub mod view;
pub mod voice;
pub mod workspace;

pub use agent::*;
pub use cloud::*;
pub use entities::*;
pub use home::*;
pub use preview::*;
pub use provider_instance::*;
pub use program_status::*;
pub use sidebar_pins::*;
pub use source_control::*;
pub use workspace::*;

/// Parse "0.2.12" (tolerating a `-suffix`/`+build` tail on the last part)
/// into a comparable triple — the fleet feature-gate primitive (device rows
/// stamp `Device::version` at boot). `None` for anything that doesn't lead
/// with three dotted integers, and gates treat `None` as "too old".
pub fn version_triple(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().splitn(3, '.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    let patch = parts.next()?;
    let patch: u64 = patch
        .split(['-', '+'])
        .next()
        .unwrap_or(patch)
        .parse()
        .ok()?;
    Some((major, minor, patch))
}
