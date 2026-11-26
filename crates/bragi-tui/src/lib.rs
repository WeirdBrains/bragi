//! `bragi-tui`: Bragi in a terminal. Like the desktop and phone shells it only draws
//! `bragi-core`'s state and calls `Core`; it adds nothing of its own beyond the terminal:
//! keyboard shortcuts, OSC 52 copy (so copying works over SSH) and a non-interactive `code`
//! command for scripts.

pub mod app;
pub mod cli;
pub mod code;
pub mod editor;
pub mod entry;
pub mod hit;
pub mod linker_cli;
pub mod markdown;
pub mod mcp_cli;
pub mod picker;
pub mod pool_cli;
pub mod serve;
pub mod session;
pub mod settings;
pub mod theme;
pub mod tree;
pub mod ui;
pub mod wake_cli;
