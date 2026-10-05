//! Roblox Studio-specific control mechanics.
//!
//! Modules here automate or communicate with Roblox Studio specifically:
//! Studio process launch, dock-layout patching, Studio's log files and cookie
//! store, and the StudioMCP JSON-RPC client. Cross-cutting modules (fflags, paths, place,
//! profile_scanner) live at the crate root since they apply to both Studio and
//! Player.

pub mod cookies;
pub mod launch;
pub mod layout;
pub mod log;
pub mod mcp_client;
