//! `bragi-core`: everything Bragi does, for both shells (GPUI on the desktop, Flutter on phones
//! through `bragi-bridge`). The shells draw [`AppState`] and [`ChatEvent`]s and call [`Core`]'s
//! actions; there is no other way in.
//!
//! - Pairing: `weirdnet-device-link` with `weirdnet-bragi://` codes and the 6-digit check.
//! - The catalog from one inference node, followed, through `weirdnet-agent`'s pool.
//! - Streaming chat with cancel and continue, over the pool (infer/2).
//! - Local history (B4 moves it to the synced context store).

pub mod catalog;
pub mod chat;
mod core;
pub mod demo;
pub mod error;
pub mod history;
pub mod identity;

pub use crate::catalog::{CatalogView, ModelInfo, ModelStatus};
pub use crate::chat::ChatEvent;
pub use crate::core::{AppState, CONTINUE_PROMPT, ChatHandle, Config, Core, LinkCheck, PairState, SCHEME, Transport, node_config};
pub use crate::error::{CoreError, ErrorKind, Result};
pub use crate::history::{ChatMessage, Conversation, ConversationSummary, MessageState, Role};
