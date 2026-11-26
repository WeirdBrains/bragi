//! `bragi-core`: everything Bragi does, for both shells (GPUI on the desktop, Flutter on phones
//! through `bragi-bridge`). The shells draw [`AppState`] and [`ChatEvent`]s and call [`Core`]'s
//! actions; there is no other way in.
//!
//! - Pairing: `weirdnet-device-link` with `weirdnet-bragi://` codes and the 6-digit check.
//! - The catalog from one inference node, followed, through `weirdnet-agent`'s pool.
//! - Streaming chat with cancel and continue, over the pool (infer/2).
//! - History on the shared context store, so conversations follow the user between devices.
//! - Code suggestions and rewrites ([`code`]): prompts from an editor's context or from files and folders,
//!   reviewable hunk diffs and an undo journal, shared by the editor extension and the terminal app.
//! - The desktop link ([`link`]): a paired phone runs code mode on a desktop's project, directly or
//!   through Weirdnet.

pub mod agent;
pub mod appearance;
pub mod attach;
pub mod audio;
pub mod catalog;
pub mod chat;
pub mod code;
pub mod context;
mod core;
pub mod demo;
pub mod demo_models;
pub mod endpoints;
pub mod error;
pub mod files;
pub mod highlight;
pub mod history;
pub mod identity;
pub mod link;
pub mod markdown;
pub mod mcp_servers;
pub mod models;
mod models_core;
pub mod notifications;
pub mod openai;
pub mod pools;
pub mod project;
pub mod prompt_history;
pub mod reactions;
pub mod secrets;
pub mod sessions;
pub mod shell;
mod targets;
pub mod think;
pub mod video;
mod video_core;
pub mod video_play;
pub mod wake;
mod wake_core;
pub use video_core::VideoPlan;
pub use wake_core::WakeableDevice;
pub mod demo_wake;

pub use crate::agent::{Choice, DEFAULT_COMPANION, ToolState};
pub use crate::appearance::{Appearance, ModePref, Palette, Preference, ThemeRow};
pub use crate::attach::{Attachment, Media};
pub use crate::catalog::{CatalogView, ModelInfo, ModelStatus};
pub use crate::chat::ChatEvent;
pub use crate::core::{
    AppState, ChatHandle, Config, Core, LinkCheck, NODE_ADDED, NODE_FOUND_SHOWN, NODE_SEARCH_LIMIT, NodeSearch, PEERS_FILE, PairState, SCHEME,
    ServerRole, Transport, is_node_address, merge_turns, node_config, normalize_link,
};
pub use crate::endpoints::{Endpoint, EndpointInput, EndpointMode, EndpointView, KeyChange};
pub use crate::error::{CoreError, ErrorKind, Result};
pub use crate::files::{ChatFile, FileKind, FileOrigin};
pub use crate::history::{ChatMessage, Conversation, ConversationSummary, Deleted, MessageState, Mode, Role, UNDO_MS};
pub use crate::mcp_servers::{McpScope, McpSecrets, McpServer, McpServerInput, McpServerView, McpTestReport, McpTransport};
pub use crate::models::upload::{PendingUpload, RolloutAsk, UploadPreview, UploadSpec, UploadedView};
pub use crate::models::{
    HubHit, ModelNode, ModelsEvent, NodeFit, NodeModels, NodeStatus, PullSpec, PullTarget, PulledView, QuantFit, StoredView, Verdict, human_bytes,
};
pub use crate::models_core::ModelsRun;
pub use crate::openai::TestReport;
pub use crate::pools::{PoolRef, PoolView};
pub use crate::reactions::{Feedback, QUICK_REACTIONS, ReactionEvent};
pub use crate::sessions::{Folder, SessionMeta, Target};
pub use crate::targets::{DeleteReport, LinkView, SessionSummary, TargetOption};
