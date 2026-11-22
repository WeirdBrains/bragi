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
pub mod demo_models;
pub mod endpoints;
pub mod error;
pub mod highlight;
pub mod history;
pub mod identity;
pub mod markdown;
pub mod models;
mod models_core;
pub mod openai;
pub mod reactions;
pub mod secrets;
pub mod sessions;
mod targets;
pub mod think;

pub use crate::agent::{Choice, DEFAULT_COMPANION, ToolState};
pub use crate::attach::{Attachment, Media};
pub use crate::catalog::{CatalogView, ModelInfo, ModelStatus};
pub use crate::chat::ChatEvent;
pub use crate::core::{
    AppState, ChatHandle, Config, Core, LinkCheck, NODE_ADDED, NODE_FOUND_SHOWN, NODE_SEARCH_LIMIT, NodeSearch, PEERS_FILE, PairState, SCHEME,
    Transport, is_node_address, node_config, normalize_link,
};
pub use crate::endpoints::{Endpoint, EndpointInput, EndpointMode, EndpointView, KeyChange};
pub use crate::error::{CoreError, ErrorKind, Result};
pub use crate::history::{ChatMessage, Conversation, ConversationSummary, Deleted, MessageState, Role, UNDO_MS};
pub use crate::models::{
    HubHit, ModelNode, ModelsEvent, NodeFit, NodeModels, NodeStatus, PullSpec, PullTarget, PulledView, QuantFit, StoredView, Verdict, human_bytes,
};
pub use crate::models_core::ModelsRun;
pub use crate::openai::TestReport;
pub use crate::reactions::{Feedback, QUICK_REACTIONS, ReactionEvent};
pub use crate::sessions::{Folder, SessionMeta, Target};
pub use crate::targets::{DeleteReport, LinkView, SessionSummary, TargetOption};
