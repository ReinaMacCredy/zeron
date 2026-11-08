//! Computer use's gpui-free core, shared by the gpui app and zgx.

pub mod control_admission;
pub mod cursor_preference;
pub mod desktop_lifecycle;
#[cfg(unix)]
pub mod host;
#[cfg(unix)]
pub mod runtime_ownership;

// Mirrors of engine constants; zeron-ui asserts they match, so this crate stays off zeron-engine.
pub const CUA_HOST_CALL_TIMEOUT_MS: u64 = 35_000;
pub const COMPUTER_USER_INTERACTION_QUIET_MS: u64 = 2_000;
