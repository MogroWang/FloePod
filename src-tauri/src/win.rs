//! Native boundaries: chrome message guards, shape, material, visibility and input.
mod chrome;
mod input;
mod material;
mod shape;
mod visibility;
pub use chrome::{install_bar_chrome_guard, prepare_shaped_window, suppress_panel_frame};
pub use input::{cursor_pos, foreground_exe, modifier_state, ModifierState};
pub use material::{apply_panel_acrylic, disable_accent, redraw_window};
pub use shape::{disable_rounding, prefer_rounded_corners, set_bar_region, set_rounded_region};
pub use visibility::{hide_window, show_bar_no_activate, show_no_activate, show_window};
