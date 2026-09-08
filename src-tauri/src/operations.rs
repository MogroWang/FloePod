//! Persistent operation history, conservative undo and retry use cases.
mod identity;
mod model;
mod preview;
mod retention;
mod retry;
mod store;
mod undo;
pub use identity::signature;
pub use model::*;
pub use preview::{preview_export, preview_remove};
pub use retention::{purge_expired, remove_to_undo_store};
pub use retry::retry;
pub use store::{list, record, snapshot};
pub use undo::undo;
#[cfg(test)]
mod tests;
