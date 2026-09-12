//! Typed resources. Each module knows how to describe its desired state,
//! inspect its target, and perform one kind of Windows mutation. The
//! journaling around those steps lives in the transaction executor, so a
//! resource never decides what is durable — it only performs the mutation
//! it is asked for.

pub mod directory;
pub mod file;
pub mod path;
pub mod registration;
pub mod registry;
pub mod shortcut;
