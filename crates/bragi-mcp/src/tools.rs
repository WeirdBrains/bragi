//! The MCP surface over a [`Core`]: the tool table, what each tool does, and the resources.
//!
//! Everything goes through `bragi-core`, so a chat from an MCP client lands in the same history,
//! folders and endpoints as the app's. Tools that delete are not listed (and refuse) unless the
//! server was started with `--allow-deletes`; tools that write project files need
//! `--allow-writes`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use bragi_core::{Attachment, Conversation, Core, CoreError, ErrorKind, FileKind, MessageState, PairState, Role, Target};
use serde_json::{Value, json};
use weirdnet_mcp::{
    CallToolResult, Content, Param, ParamType as T, Resource, ResourceContents, ResourceError, ResourceTemplate, Server, ServerInfo, ToolError,
    ToolSpec, Tools,
};

use crate::collect;

/// What the server allows beyond reading, asking and filing sessions.
#[derive(Debug, Clone)]
pub struct Options {
    /// Offer the tools that write files under [`root`](Self::root): saving a session's file,
    /// applying and undoing a code proposal.
    pub allow_writes: bool,
    /// Offer the tools that delete sessions and folders.
    pub allow_deletes: bool,
    /// The project folder: files a tool reads or writes must be inside it.
    pub root: PathBuf,
    /// How long one answer may take before it is stopped.
    pub timeout: Duration,
    /// The undo journal for `bragi_code_apply` (in Bragi's folder, never in the project).
    pub journal: PathBuf,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            allow_writes: false,
            allow_deletes: false,
            root: PathBuf::from("."),
            timeout: Duration::from_secs(600),
            journal: crate::code::journal_dir(&bragi_serve::cli::default_home()),
        }
    }
}

/// The most sessions `bragi_sessions_list` returns at once, and resources lists.
pub const MAX_SESSIONS: usize = 200;
/// Sessions listed as resources (the newest).
pub const RESOURCE_SESSIONS: usize = 50;

const INSTRUCTIONS: &str = "Bragi is a chat app for the models on the user's own Weirdnet (Ymir nodes) and the OpenAI-compatible \
endpoints in Bragi's Settings. Use bragi_chat to ask a model; the conversation is kept as a Bragi session the user also sees in the app. \
bragi_models_list and bragi_endpoints_list show where a chat can go, bragi_sessions_list and bragi_session_get read past sessions, \
bragi_code_suggest and bragi_code_rewrite propose code changes as unified diffs (writing nothing), and bragi_status explains why nothing answers. Sessions are also resources: bragi://sessions/{id}.";

pub(crate) const fn spec(name: &'static str, description: &'static str, params: &'static [Param], read_only: bool, destructive: bool) -> ToolSpec {
    ToolSpec { name, description, params, auth: false, read_only, destructive, open_world: false }
}

const NO_PARAMS: &[Param] = &[];
const SESSION: Param = Param::required("session", T::String, "A session id from bragi_sessions_list.");
const FOLDER_OR_TOP: T = T::AnyOf(&[T::String, T::Json(r#"{"type":"null"}"#)]);

const CHAT_PARAMS: &[Param] = &[
    Param::required("prompt", T::String, "The message to send."),
    Param::optional("session", T::String, "Continue this session (from bragi_sessions_list). Default: a new session."),
    Param::optional(
        "endpoint",
        T::String,
        "Where to send it: `pool` (the Weirdnet pool) or an endpoint id from bragi_endpoints_list. Default: where the session is bound, or for a new session the pool (or the first endpoint offered alongside it when this device isn't linked).",
    ),
    Param::optional("model", T::String, "A model id from bragi_models_list. Default: the pool's pick, or the endpoint's model."),
    Param::optional("folder", T::String, "File a new session in this folder (an id from bragi_folders_list)."),
    Param::optional(
        "files",
        T::StringArray,
        "Files to attach: paths inside the project root (text, PNG/JPEG/GIF/WebP images up to 5 MB, WAV/MP3 audio up to 10 MB), or data: URLs. At most 5.",
    ),
];

const LIST_PARAMS: &[Param] = &[
    Param::optional("offset", T::Integer, "Skip this many of the newest sessions (default 0)."),
    Param::optional("limit", T::Integer, "Return at most this many (default 50, at most 200)."),
    Param::optional("folder", T::String, "Only the sessions in this folder (an id from bragi_folders_list)."),
];

const GET_PARAMS: &[Param] = &[SESSION, Param::optional("last", T::Integer, "Only the last N messages.")];

const FOLDER_CREATE_PARAMS: &[Param] = &[
    Param::required("name", T::String, "The folder's name."),
    Param::optional("parent", T::String, "Make it inside this folder (an id). Default: at the top."),
];

const FOLDER_MOVE_PARAMS: &[Param] = &[
    Param::required("folder", T::String, "The folder id."),
    Param::required("parent", FOLDER_OR_TOP, "The new parent folder id, or null for the top."),
];

const SESSION_MOVE_PARAMS: &[Param] = &[SESSION, Param::required("folder", FOLDER_OR_TOP, "The folder id, or null to take it out of any folder.")];

const MODELS_PARAMS: &[Param] =
    &[Param::optional("endpoint", T::String, "List the models of this endpoint (an id from bragi_endpoints_list) instead of the pool's.")];

const FILE_PARAMS: &[Param] = &[SESSION, Param::required("file", T::String, "The file id from bragi_files_list (`<message id>#a1`, `#r1`, …).")];

const SAVE_PARAMS: &[Param] = &[
    SESSION,
    Param::required("file", T::String, "The file id from bragi_files_list."),
    Param::optional(
        "path",
        T::String,
        "Where to save it, inside the project root: a folder (the file keeps its name; ` (2)` is added rather than replacing anything) or a file path. Default: the project root.",
    ),
    Param::optional("overwrite", T::Boolean, "Replace the file at `path` when it exists (default false: refuse)."),
];

const FOLDER_DELETE_PARAMS: &[Param] = &[Param::required("folder", T::String, "The folder id. What it holds moves up one level.")];

/// Tools every server offers.
pub const READ_TOOLS: &[ToolSpec] = &[
    spec(
        "bragi_status",
        "Whether Bragi is linked to the user's Weirdnet, whether an inference node answers, the nodes and how many models are warm, and the endpoints; with what to do when nothing answers.",
        NO_PARAMS,
        true,
        false,
    ),
    spec(
        "bragi_chat",
        "Ask a model and get its whole answer. The exchange is kept as a Bragi session (new, or the one given), which the user also sees in the app. Uses the pool's compute or the endpoint's.",
        CHAT_PARAMS,
        false,
        false,
    ),
    spec(
        "bragi_sessions_list",
        "Bragi's sessions, newest first: id, title, message count, folder and where each one sends.",
        LIST_PARAMS,
        true,
        false,
    ),
    spec("bragi_session_get", "One session's messages: who said what, the model that answered, the files attached.", GET_PARAMS, true, false),
    spec("bragi_folders_list", "The folders sessions are filed in (id, name, parent).", NO_PARAMS, true, false),
    spec("bragi_folder_create", "Make a folder for sessions. Changes Bragi's session list.", FOLDER_CREATE_PARAMS, false, false),
    spec("bragi_folder_move", "Move a folder into another folder, or to the top. Changes Bragi's session list.", FOLDER_MOVE_PARAMS, false, false),
    spec(
        "bragi_session_move",
        "File a session in a folder, or take it out of its folder. Changes Bragi's session list.",
        SESSION_MOVE_PARAMS,
        false,
        false,
    ),
    spec(
        "bragi_endpoints_list",
        "Where a chat can go: the Weirdnet pool and the OpenAI-compatible endpoints in Bragi's Settings (name, address, default model, whether it has a key; never the key).",
        NO_PARAMS,
        true,
        false,
    ),
    spec(
        "bragi_models_list",
        "The pool's chat models with their state (warm, loading, busy, loadable), replicas and speed; or with `endpoint`, the models that endpoint serves.",
        MODELS_PARAMS,
        true,
        false,
    ),
    spec(
        "bragi_files_list",
        "The files in a session: what was attached to its messages and what its answers wrote (fenced code blocks that name a file, data: images). Id, name, path, kind (text, image, audio), size.",
        &[SESSION],
        true,
        false,
    ),
    spec("bragi_file_fetch", "One file in a session: text as text, an image or a sound as MCP image or audio content.", FILE_PARAMS, true, false),
];

/// Tools offered only with `--allow-writes`.
pub const WRITE_TOOLS: &[ToolSpec] = &[spec(
    "bragi_file_save",
    "WRITES a file from a session (an attachment, or a file an answer wrote) into the project folder. Never replaces a file unless overwrite=true.",
    SAVE_PARAMS,
    false,
    true,
)];

/// Tools offered only with `--allow-deletes`.
pub const DELETE_TOOLS: &[ToolSpec] = &[
    spec("bragi_session_delete", "DELETES a session and its messages from Bragi's history, on every linked device.", &[SESSION], false, true),
    spec(
        "bragi_folder_delete",
        "DELETES a folder. The sessions and folders in it move up one level; no session is deleted.",
        FOLDER_DELETE_PARAMS,
        false,
        true,
    ),
];

/// Bragi's tools over one [`Core`].
#[derive(Clone)]
pub struct BragiTools {
    core: Core,
    opts: Arc<Options>,
    root: Arc<PathBuf>,
    specs: Vec<ToolSpec>,
    proposals: Arc<crate::code::Proposals>,
}

/// The protocol server for `tools`.
pub fn server(tools: BragiTools) -> Server<BragiTools> {
    let info =
        ServerInfo { name: "bragi".into(), title: "Bragi".into(), version: env!("CARGO_PKG_VERSION").into(), instructions: INSTRUCTIONS.into() };
    Server::new(info, tools)
}

pub(crate) fn bad(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, msg)
}

/// A fresh id such as `p-3f9a…`.
pub(crate) fn random_id(prefix: &str) -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    format!("{prefix}-{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

fn err(e: CoreError) -> ToolError {
    ToolError(e.message)
}

impl BragiTools {
    /// The tools `opts` allows over `core`. `opts.root` must exist.
    pub fn new(core: Core, opts: Options) -> anyhow::Result<Self> {
        let root = opts.root.canonicalize().map_err(|e| anyhow::anyhow!("project root {}: {e}", opts.root.display()))?;
        let mut specs = READ_TOOLS.to_vec();
        specs.extend_from_slice(crate::code::CODE_TOOLS);
        if opts.allow_writes {
            specs.extend_from_slice(WRITE_TOOLS);
            specs.extend_from_slice(crate::code::CODE_WRITE_TOOLS);
        }
        if opts.allow_deletes {
            specs.extend_from_slice(DELETE_TOOLS);
        }
        Ok(Self { core, opts: Arc::new(opts), root: Arc::new(root), specs, proposals: Arc::default() })
    }

    pub fn core(&self) -> &Core {
        &self.core
    }

    pub(crate) fn options(&self) -> &Options {
        &self.opts
    }

    /// The project root, canonical.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn proposals(&self) -> &crate::code::Proposals {
        &self.proposals
    }

    /// The `endpoint` argument as a target: `None` when left out (the session's or the default).
    pub(crate) fn target_arg(&self, args: &Value, model: Option<&str>) -> Result<Option<Target>, CoreError> {
        match weirdnet_mcp::args::opt_string(args, "endpoint") {
            None | Some("") => Ok(None),
            Some("pool") => Ok(Some(Target::Pool)),
            Some(id) => {
                let e = self
                    .core
                    .endpoints()
                    .into_iter()
                    .find(|e| e.id == id)
                    .ok_or_else(|| CoreError::new(ErrorKind::NotFound, format!("no endpoint `{id}`; see bragi_endpoints_list")))?;
                Ok(Some(Target::Endpoint { id: e.id, model: model.map(str::to_string).or(e.model).unwrap_or_default() }))
            }
        }
    }

    /// A path a client gave, inside the project root (relative to it, or absolute).
    pub(crate) fn resolve(&self, p: &str) -> Result<PathBuf, CoreError> {
        let path = Path::new(p);
        let full = if path.is_absolute() { path.to_path_buf() } else { self.root.join(path) };
        let real = full.canonicalize().map_err(|e| CoreError::new(ErrorKind::NotFound, format!("{p}: {e}")))?;
        if !real.starts_with(self.root.as_path()) {
            return Err(bad(format!("{p} is outside the project root; the server only reads files inside it")));
        }
        Ok(real)
    }

    fn attachment(&self, f: &str) -> Result<Attachment, CoreError> {
        if let Some(rest) = f.strip_prefix("data:") {
            let (head, data) = rest.split_once(',').ok_or_else(|| bad("a data: URL needs a comma before its data"))?;
            if !head.ends_with(";base64") {
                return Err(bad("only base64 data: URLs can be attached"));
            }
            let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|_| bad("the data: URL is not valid base64"))?;
            let ext = match head.trim_end_matches(";base64") {
                "image/png" => "png",
                "image/jpeg" => "jpg",
                "image/gif" => "gif",
                "image/webp" => "webp",
                "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
                "audio/mpeg" | "audio/mp3" => "mp3",
                _ => "txt",
            };
            return Attachment::from_bytes(&format!("attachment.{ext}"), &bytes);
        }
        Attachment::from_path(&self.resolve(f)?)
    }

    /// Bragi's state, for `bragi_status` and `bragi://status`.
    pub fn status(&self) -> Value {
        let s = self.core.state();
        let (linked, device) = match &s.pair {
            PairState::Linked { device, .. } => (true, Some(device.clone())),
            _ => (false, None),
        };
        let warm = s.catalog.models.iter().filter(|m| matches!(m.status, bragi_core::ModelStatus::Warm)).count();
        let endpoints = self.core.endpoints();
        let mut v = json!({
            "version": env!("CARGO_PKG_VERSION"),
            "linked": linked,
            "device": device,
            "online": s.online,
            "nodes": s.catalog.nodes.iter().map(|n| json!({"id": n.id, "models": n.models, "warm": n.warm})).collect::<Vec<_>>(),
            "models": s.catalog.models.len(),
            "warm_models": warm,
            "endpoints": endpoints.iter().filter(|e| e.enabled).count(),
            "project_root": self.root.display().to_string(),
            "allow_writes": self.opts.allow_writes,
            "allow_deletes": self.opts.allow_deletes,
        });
        if !linked {
            v["hint"] = json!("Not linked: run `bragi mcp link` and open the code on a linked device (on a Ymir node: `ymirctl pool add CODE`).");
        } else if !s.online && endpoints.is_empty() {
            v["hint"] = json!(self.core.no_node_message());
        }
        v
    }

    fn conversation(&self, args: &Value) -> Result<Conversation, CoreError> {
        let id = weirdnet_mcp::args::string(args, "session").map(str::to_string).map_err(bad)?;
        self.core.conversation(&id)
    }

    async fn chat(&self, args: &Value) -> Result<CallToolResult, CoreError> {
        let a = |k| weirdnet_mcp::args::opt_string(args, k).map(str::to_string);
        let prompt = weirdnet_mcp::args::string(args, "prompt").map(str::to_string).map_err(bad)?;
        if prompt.trim().is_empty() {
            return Err(bad("the prompt is empty"));
        }
        let session = a("session");
        let model = a("model").filter(|m| !m.is_empty());
        let target = self.target_arg(args, model.as_deref())?;
        let files = weirdnet_mcp::args::strings(args, "files").unwrap_or_default();
        if files.len() > bragi_core::attach::MAX_PER_MESSAGE {
            return Err(bad(format!("at most {} files go with one message", bragi_core::attach::MAX_PER_MESSAGE)));
        }
        let attachments = files.iter().map(|f| self.attachment(f)).collect::<Result<Vec<_>, _>>()?;
        let handle = self.core.send_to(session.as_deref(), &prompt, model, attachments, target)?;
        let answer = collect::answer(&self.core, handle, self.opts.timeout).await?;
        if session.is_none()
            && let Some(folder) = a("folder").filter(|f| !f.is_empty())
        {
            self.core.set_session_folder(&answer.conversation, Some(&folder))?;
        }
        let structured = json!({
            "session": answer.conversation,
            "model": answer.model,
            "node": answer.node,
            "finish_reason": answer.finish_reason,
            "text": answer.text,
        });
        Ok(CallToolResult::new(vec![Content::text(answer.text.clone())]).with_structured(structured))
    }

    fn files_list(&self, session: &str) -> Result<Value, CoreError> {
        let files: Vec<Value> = self
            .core
            .files(session)?
            .into_iter()
            .map(|f| {
                let mut v = serde_json::to_value(&f).unwrap_or_default();
                v["uri"] = json!(file_uri(session, &f.id));
                v
            })
            .collect();
        Ok(json!({"session": session, "files": files}))
    }

    /// `bragi_file_save`: the destination must be inside the project root.
    fn save_file(&self, session: &str, file: &str, path: Option<&str>, overwrite: bool) -> Result<PathBuf, CoreError> {
        let dest = match path.filter(|p| !p.is_empty()) {
            None => self.root.as_ref().clone(),
            Some(p) => {
                let given = Path::new(p);
                let full = if given.is_absolute() { given.to_path_buf() } else { self.root.join(given) };
                if full.is_dir() {
                    self.resolve(p)?
                } else {
                    let name = full.file_name().ok_or_else(|| bad(format!("{p} names no file")))?.to_owned();
                    let parent = full.parent().map(Path::to_path_buf).unwrap_or_else(|| self.root.as_ref().clone());
                    let parent = parent.canonicalize().map_err(|e| CoreError::new(ErrorKind::NotFound, format!("{}: {e}", parent.display())))?;
                    if !parent.starts_with(self.root.as_path()) {
                        return Err(bad(format!("{p} is outside the project root; the server only writes inside it")));
                    }
                    let dest = parent.join(name);
                    if dest.exists() && !overwrite {
                        return Err(bad(format!("{p} exists; pass overwrite=true to replace it")));
                    }
                    dest
                }
            }
        };
        self.core.save_file(session, file, &dest)
    }

    async fn dispatch(&self, name: &str, args: &Value) -> Result<CallToolResult, CoreError> {
        let json_out = |v: Value| Ok(CallToolResult::json(&v));
        let opt_int = |k| -> Result<Option<usize>, CoreError> {
            match args.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v.as_u64().map(|n| Some(n as usize)).ok_or_else(|| bad(format!("`{k}` must be a whole number of at least 0"))),
            }
        };
        let folder_or_top = |k: &str| -> Result<Option<String>, CoreError> {
            match args.get(k) {
                Some(Value::Null) => Ok(None),
                Some(Value::String(s)) if s.is_empty() => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.clone())),
                _ => Err(bad(format!("`{k}` must be a folder id or null"))),
            }
        };
        let s = |k| weirdnet_mcp::args::string(args, k).map(str::to_string).map_err(bad);
        match name {
            "bragi_status" => json_out(self.status()),
            "bragi_chat" => self.chat(args).await,
            "bragi_sessions_list" => {
                let offset = opt_int("offset")?.unwrap_or(0);
                let limit = opt_int("limit")?.unwrap_or(50).clamp(1, MAX_SESSIONS);
                let folder = weirdnet_mcp::args::opt_string(args, "folder").map(str::to_string);
                let sessions = match &folder {
                    // Filter over everything, then page, so a folder's sessions aren't cut off.
                    Some(f) => {
                        self.core.sessions(0, usize::MAX)?.into_iter().filter(|x| x.folder.as_deref() == Some(f)).skip(offset).take(limit).collect()
                    }
                    None => self.core.sessions(offset, limit)?,
                };
                json_out(json!({"sessions": sessions}))
            }
            "bragi_session_get" => {
                let c = self.conversation(args)?;
                let meta = self.core.session_target(&c.id);
                json_out(session_json(&c, opt_int("last")?, &meta))
            }
            "bragi_folders_list" => json_out(json!({"folders": self.core.folders()})),
            "bragi_folder_create" => {
                let parent = weirdnet_mcp::args::opt_string(args, "parent").map(str::to_string).filter(|p| !p.is_empty());
                json_out(json!({"folder": self.core.create_folder(&s("name")?, parent.as_deref())?}))
            }
            "bragi_folder_move" => {
                let id = s("folder")?;
                let parent = folder_or_top("parent")?;
                self.core.move_folder(&id, parent.as_deref())?;
                json_out(json!({"folder": id, "parent": parent}))
            }
            "bragi_session_move" => {
                let c = self.conversation(args)?;
                let folder = folder_or_top("folder")?;
                self.core.set_session_folder(&c.id, folder.as_deref())?;
                json_out(json!({"session": c.id, "folder": folder}))
            }
            "bragi_endpoints_list" => json_out(json!({"targets": self.core.targets(), "endpoints": self.core.endpoints()})),
            "bragi_models_list" => {
                match weirdnet_mcp::args::opt_string(args, "endpoint").map(str::to_string).filter(|e| !e.is_empty() && e != "pool") {
                    Some(id) => json_out(json!({"endpoint": id, "models": self.core.endpoint_models(&id).await?})),
                    None => {
                        let s = self.core.state();
                        json_out(json!({"online": s.online, "models": s.catalog.models}))
                    }
                }
            }
            "bragi_files_list" => json_out(self.files_list(&s("session")?)?),
            "bragi_file_fetch" => {
                let (f, bytes) = self.core.file_bytes(&s("session")?, &s("file")?)?;
                let content = match f.kind {
                    FileKind::Text => Content::text(String::from_utf8_lossy(&bytes).into_owned()),
                    FileKind::Image => Content::image_bytes(&bytes, f.mime.clone()),
                    FileKind::Audio => Content::audio_bytes(&bytes, f.mime.clone()),
                };
                let meta = json!({"id": f.id, "name": f.name, "path": f.path, "mime_type": f.mime, "bytes": f.size});
                Ok(CallToolResult::new(vec![content]).with_structured(meta))
            }
            "bragi_file_save" => {
                let path = weirdnet_mcp::args::opt_string(args, "path");
                let saved = self.save_file(&s("session")?, &s("file")?, path, weirdnet_mcp::args::boolean(args, "overwrite", false))?;
                json_out(json!({"saved": saved.display().to_string()}))
            }
            "bragi_code_suggest" => self.code_suggest(args).await,
            "bragi_code_rewrite" => self.code_rewrite(args).await,
            "bragi_code_apply" => self.code_apply(args),
            "bragi_code_undo" => self.code_undo(args),
            "bragi_session_delete" => {
                let c = self.conversation(args)?;
                self.core.delete_conversation(&c.id)?;
                json_out(json!({"deleted": c.id}))
            }
            "bragi_folder_delete" => {
                let id = s("folder")?;
                self.core.delete_folder(&id)?;
                json_out(json!({"deleted": id}))
            }
            other => Err(bad(format!("no tool `{other}`"))),
        }
    }
}

impl Tools for BragiTools {
    fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    async fn call_async(&self, tool: &ToolSpec, args: &Value, _token: Option<&str>) -> Result<CallToolResult, ToolError> {
        self.dispatch(tool.name, args).await.map_err(err)
    }

    fn has_resources(&self) -> bool {
        true
    }

    async fn list_resources(&self, _token: Option<&str>) -> Result<Vec<Resource>, ResourceError> {
        let mut v = vec![
            Resource::new("bragi://status", "status")
                .with_title("Bragi status")
                .with_description("Linked or not, the nodes, the models and the endpoints.")
                .with_mime_type("application/json"),
            Resource::new("bragi://sessions", "sessions")
                .with_title("Bragi sessions")
                .with_description("The newest sessions with their folder and target.")
                .with_mime_type("application/json"),
        ];
        let sessions = self.core.sessions(0, RESOURCE_SESSIONS).map_err(|e| ResourceError::Failed(e.message))?;
        v.extend(sessions.into_iter().map(|s| {
            Resource::new(format!("bragi://sessions/{}", s.id), format!("session {}", s.id))
                .with_title(s.title)
                .with_description(format!("{} messages", s.messages))
                .with_mime_type("text/markdown")
        }));
        Ok(v)
    }

    fn resource_templates(&self) -> Vec<ResourceTemplate> {
        vec![
            ResourceTemplate::new("bragi://sessions/{id}", "session")
                .with_title("A Bragi session")
                .with_description("The session's messages as Markdown.")
                .with_mime_type("text/markdown"),
            ResourceTemplate::new("bragi://sessions/{id}/files/{file}", "session file")
                .with_title("A file attached in a Bragi session")
                .with_description("A file in the session (attached, or written by an answer): text, or an image or sound as a blob. {file} is its id from bragi_files_list, percent-encoded."),
        ]
    }

    async fn read_resource(&self, uri: &str, _token: Option<&str>) -> Result<Vec<ResourceContents>, ResourceError> {
        let failed = |e: CoreError| match e.kind {
            ErrorKind::NotFound => ResourceError::NotFound,
            _ => ResourceError::Failed(e.message),
        };
        match uri {
            "bragi://status" => return Ok(vec![ResourceContents::json(uri, &self.status())]),
            "bragi://sessions" => {
                let s = self.core.sessions(0, MAX_SESSIONS).map_err(failed)?;
                return Ok(vec![ResourceContents::json(uri, &json!({"sessions": s}))]);
            }
            _ => {}
        }
        let rest = uri.strip_prefix("bragi://sessions/").ok_or(ResourceError::NotFound)?;
        let (id, file) = match rest.split_once("/files/") {
            Some((id, name)) => (id, Some(unpercent(name))),
            None => (rest, None),
        };
        if id.is_empty() || id.contains('/') {
            return Err(ResourceError::NotFound);
        }
        let c = self.core.conversation(id).map_err(failed)?;
        match file {
            None => Ok(vec![ResourceContents::text(uri, Some("text/markdown"), transcript(&c, None))]),
            Some(file) => {
                let (f, bytes) = self.core.file_bytes(id, &file).map_err(failed)?;
                Ok(vec![match f.kind {
                    FileKind::Text => ResourceContents::text(uri, Some(f.mime.as_str()), String::from_utf8_lossy(&bytes).into_owned()),
                    _ => ResourceContents::bytes(uri, Some(f.mime.as_str()), &bytes),
                }])
            }
        }
    }
}

/// The resource URI of a file in a session (by its [`bragi_core::ChatFile`] id).
pub fn file_uri(session: &str, file: &str) -> String {
    format!("bragi://sessions/{session}/files/{}", percent(file))
}

fn percent(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn unpercent(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn state_name(s: MessageState) -> &'static str {
    match s {
        MessageState::Complete => "complete",
        MessageState::Streaming => "streaming",
        MessageState::Stopped => "stopped",
        MessageState::Failed => "failed",
    }
}

fn session_json(c: &Conversation, last: Option<usize>, meta: &bragi_core::SessionMeta) -> Value {
    let skip = last.map(|n| c.messages.len().saturating_sub(n)).unwrap_or(0);
    let messages: Vec<Value> = c
        .messages
        .iter()
        .enumerate()
        .skip(skip)
        .map(|(i, m)| {
            json!({
                "index": i,
                "role": m.role,
                "text": m.text,
                "state": state_name(m.state),
                "model": m.model,
                "node": m.node,
                "files": m.attachments.iter().map(|f| f.label()).collect::<Vec<_>>(),
                "at_ms": m.at_ms,
            })
        })
        .collect();
    json!({
        "id": c.id,
        "title": c.title,
        "created_ms": c.created_ms,
        "updated_ms": c.updated_ms,
        "folder": meta.folder,
        "target": meta.target,
        "messages": messages,
    })
}

/// A session as Markdown.
pub fn transcript(c: &Conversation, last: Option<usize>) -> String {
    let skip = last.map(|n| c.messages.len().saturating_sub(n)).unwrap_or(0);
    let mut s = format!("# {}\n", c.title);
    for m in c.messages.iter().skip(skip) {
        let who = match m.role {
            Role::User => "You".to_string(),
            Role::Assistant => m.model.clone().unwrap_or_else(|| "Assistant".into()),
        };
        s.push_str(&format!("\n## {who}\n\n{}\n", m.text.trim_end()));
        for f in &m.attachments {
            s.push_str(&format!("\n(attached: {})\n", f.label()));
        }
        if !matches!(m.state, MessageState::Complete) {
            s.push_str(&format!("\n({})\n", state_name(m.state)));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_tables_are_valid() {
        let mut all = READ_TOOLS.to_vec();
        all.extend_from_slice(crate::code::CODE_TOOLS);
        all.extend_from_slice(crate::code::CODE_WRITE_TOOLS);
        all.extend_from_slice(WRITE_TOOLS);
        all.extend_from_slice(DELETE_TOOLS);
        ToolSpec::check(&all).unwrap();
    }

    #[test]
    fn file_uris_round_trip() {
        let name = "m-1#a2 notes∕100%";
        let uri = file_uri("c-1", name);
        assert!(!uri.contains(' '));
        assert_eq!(unpercent(uri.rsplit('/').next().unwrap()), name);
        assert_eq!(unpercent("bad%zz%4"), "bad%zz%4");
    }
}
