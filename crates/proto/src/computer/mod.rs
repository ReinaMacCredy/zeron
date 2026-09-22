//! Computer use: the cua-driver host protocol, the Computer contracts and the
//! engine↔host wire, ported from Synara's `packages/shared` and
//! `packages/contracts` computer modules.

pub mod audit;
mod bounds;
pub mod browser;
pub mod contract;
pub mod diagnostics;
pub mod driver;
pub mod frame;
pub mod grants;
pub mod key_names;
pub mod spaces;
pub mod wire;

pub use audit::*;
pub use browser::*;
pub use contract::*;
pub use diagnostics::*;
pub use driver::*;
pub use frame::*;
pub use grants::*;
pub use key_names::*;
pub use spaces::*;
pub use wire::*;
