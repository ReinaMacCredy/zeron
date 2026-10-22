//! zeron-proto — wire types shared by engine, UI, and RPC.
//!
//! Ported from zeron's `packages/control/src/wire.ts` + `packages/harness/src/types.ts`.
//! Context occupancy is replicated per chat; billing `Usage` remains a harness passthrough.

pub mod account_usage;
pub mod agent;
pub mod attachment_mentions;
pub mod change_request_assessment;
pub mod chat_mentions;
pub mod computer;
pub mod device_hub;
pub mod device_paths;
pub mod entities;
pub mod file_mentions;
pub mod home;
pub mod html_render;
pub mod invocation;
pub mod model_catalog;
pub mod motion;
pub mod preview;
pub mod projectless;
pub mod provider_instance;
pub mod routing;
pub mod sidebar_pins;
pub mod source_control;
pub mod supervisor;
pub mod tools;
pub mod ui_driver;
pub mod usage;
pub mod view;
pub mod voice;
pub mod workspace;

pub use agent::*;
pub use entities::*;
pub use home::*;
pub use preview::*;
pub use provider_instance::*;
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
