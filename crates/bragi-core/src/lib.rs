//! `bragi-core`: everything Bragi does, for both shells (GPUI on the desktop, Flutter on phones
//! through `bragi-bridge`). The shells draw [`AppState`] and [`ChatEvent`]s and call [`Core`]'s
//! actions; there is no other way in.
//!
//! - Pairing: `weirdnet-device-link` with `weirdnet-bragi://` codes and the 6-digit check.
//! - The catalog from one inference node, followed, through `weirdnet-agent`'s pool.
//! - Streaming chat with cancel and continue, over the pool (infer/2).
//! - History on the shared context store, so conversations follow the user between devices.
//! - Sessions in folders, each sending to the Weirdnet pool or one OpenAI-compatible endpoint, and
//!   a Ymir bridge as the Weirdnet session's fallback when no inference node answers.

pub mod agent;
pub mod attach;
pub mod catalog;
pub mod chat;
pub mod context;
mod core;
pub mod demo;
pub mod demo_bridge;
pub mod endpoint;
pub mod error;
pub mod history;
pub mod identity;
pub mod links;
pub mod session;

pub use crate::agent::{Choice, DEFAULT_COMPANION, ToolState};
pub use crate::attach::Attachment;
pub use crate::catalog::{CatalogView, ModelInfo, ModelStatus};
pub use crate::chat::ChatEvent;
pub use crate::core::{
    AppState, ChatHandle, Config, Core, LinkCheck, NODE_ADDED, PEERS_FILE, PairState, Route, RouteKind, SCHEME, Transport, is_node_address,
    node_config, normalize_link,
};
pub use crate::endpoint::{EndpointHealth, EndpointKind, EndpointState, EndpointTest, Rebind};
pub use crate::error::{CoreError, ErrorKind, Result};
pub use crate::history::{ChatMessage, Conversation, ConversationSummary, MessageState, Role};
pub use crate::links::LinkState;
pub use crate::session::{DEFAULT_SESSION, Folder, Row, Session, Sessions, Target};
