//! `bragi-core`: everything Bragi does, for both shells (GPUI on the desktop, Flutter on phones
//! through `bragi-bridge`). The shells draw [`AppState`] and [`ChatEvent`]s and call [`Core`]'s
//! actions; there is no other way in.
//!
//! - Pairing: `weirdnet-device-link` with `weirdnet-bragi://` codes and the 6-digit check.
//! - The catalog from one inference node, followed, through `weirdnet-agent`'s pool.
//! - Streaming chat with cancel and continue, over the pool (infer/2).
//! - History on the shared context store, so conversations follow the user between devices.

pub mod agent;
pub mod attach;
pub mod catalog;
pub mod chat;
pub mod context;
mod core;
pub mod demo;
pub mod error;
pub mod history;
pub mod identity;

pub use crate::agent::{Choice, DEFAULT_COMPANION, ToolState};
pub use crate::attach::Attachment;
pub use crate::catalog::{CatalogView, ModelInfo, ModelStatus};
pub use crate::chat::ChatEvent;
pub use crate::core::{AppState, ChatHandle, Config, Core, LinkCheck, PEERS_FILE, PairState, SCHEME, Transport, node_config, normalize_link};
pub use crate::error::{CoreError, ErrorKind, Result};
pub use crate::history::{ChatMessage, Conversation, ConversationSummary, MessageState, Role};
