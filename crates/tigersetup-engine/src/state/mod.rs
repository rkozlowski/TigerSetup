//! The per-installation SQLite database: committed ownership (installation
//! state) and the transaction journal, kept in separate tables.

pub mod db;
pub mod dependency;
pub mod installation;
pub mod journal;

pub use db::Db;
