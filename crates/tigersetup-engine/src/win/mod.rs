//! The engine's Windows layer: durable file primitives, registry and
//! shell-link primitives, access control lists, known folders, the
//! environment broadcast and the running Windows build. Roots come in as
//! parameters or through documented seams, so tests redirect them.

pub mod acl;
pub mod env;
pub mod file_version;
pub mod fs;
pub mod process;
pub mod registry;
pub mod shortcut;
pub mod version;
