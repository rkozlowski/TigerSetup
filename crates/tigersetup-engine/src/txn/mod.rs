//! The transaction engine: the prepare → apply → applied loop, the commit,
//! the idempotent reverse walk, recovery on restart, and fault injection at
//! every journal/mutation boundary.

pub mod actions;
pub mod executor;
pub mod fault;
pub mod recovery;
pub mod rollback;

pub use executor::{Executor, ForwardStats, GROUP_BATCHES};
pub use fault::{FaultAction, FaultInjector, FaultPoint, FaultSpec};
pub use rollback::RollbackStats;
