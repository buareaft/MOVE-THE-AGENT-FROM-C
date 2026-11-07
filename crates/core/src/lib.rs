//! Independent Windows core. Host operations are confined to generated fixtures.
mod deepseek;
mod lab;
mod live_scan;
mod local;
mod migration;
mod models;
mod overview;
mod relocation;
mod scanner;
mod settings;
pub use lab::Lab;
pub use models::*;
pub use relocation::{MoveCandidate, MoveJob, MovePlan, MoveProgress};
pub use settings::{StorageSettings, VirtualDiskStatus};
