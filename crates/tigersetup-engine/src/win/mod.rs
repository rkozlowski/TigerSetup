//! The engine's Windows layer: durable file primitives, registry,
//! shell-link and firewall primitives, access control lists, known folders,
//! the environment and association broadcasts, the running Windows build,
//! and starting a program as the signed-in user. Roots come in as
//! parameters or through documented seams, so tests redirect them.

pub mod acl;
pub mod env;
pub mod file_version;
pub mod firewall;
pub mod fs;
pub mod interactive;
pub mod process;
pub mod registry;
pub mod shortcut;
pub mod version;
