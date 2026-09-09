//! Staging use cases. Filesystem preparation, SQLite commit and post-commit cleanup have explicit boundaries.
mod access;
mod context;
mod recovery;
mod removal;
mod stage;
mod text;
pub use access::{
    copy_staged_to_clipboard, open_pod_folder, open_staged_item, reveal_staged_items,
};
pub use context::{data_dir, item_path, load_settings, load_settings_from, validate_item_pods};
pub use recovery::recover_pending_moves;
pub use removal::{list_pod_items, remove_items};
pub use stage::{stage_paths, StagePathsResult};
pub use text::stage_text;
#[cfg(test)]
mod tests;
