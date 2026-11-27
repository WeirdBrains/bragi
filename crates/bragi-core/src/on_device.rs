//! The optional on-device model (B4): with no reachable node, or when the user picks it, Bragi
//! answers with a small model running in this process through `weirdnet-model`.
//!
//! Off by default on every platform. Turning it on downloads nothing: the user picks a model and
//! says yes to its download ([`Core::on_device_consent`] shows what and from where, and
//! [`Core::download_on_device_model`] refuses without that yes). It then shows in the target
//! picker as "On this device", and a pool chat falls back to it when no node answers.
//!
//! The engine is llama.cpp, compiled in only with the `on-device` Cargo feature (it pulls
//! llama.cpp's C++ build, so everyday builds stay fast without it). A build without it still
//! shows the setting and says what it lacks. Tests swap the engine
//! ([`Core::set_on_device_engine`]) and the Hub ([`Core::set_on_device_source`]); they never
//! fetch real weights.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use weirdnet_agent::pool::PoolEvent;
use weirdnet_agent::{CancelToken, Message};
use weirdnet_model::{ChatMessage, ChatRole, Chunk, Consent, ModelStore, Worker};

use crate::chat::{ChatEvent, coalesce};
use crate::core::{Core, lock};
use crate::error::{CoreError, ErrorKind, Result};
use crate::history::MessageState;

pub use weirdnet_model::worker::StopCause;
pub use weirdnet_model::{Engine, GenerateRequest, ModelSpec, Stats};
/// What an [`Engine`] returns.
pub type EngineResult<T> = weirdnet_model::Result<T>;

/// This device's on-device model settings, in the data dir (not synced: each device decides).
pub const SETTINGS_FILE: &str = "on-device.json";
/// Where downloaded models go, in the data dir.
pub const MODELS_DIR: &str = "models";
/// What the picker and the answers call it.
pub const LABEL: &str = "On this device";

/// Loads a model file into an engine.
pub type EngineFactory = Arc<dyn Fn(&Path) -> weirdnet_model::Result<Box<dyn Engine>> + Send + Sync>;

/// What the user chose on this device.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OnDeviceSettings {
    #[serde(default)]
    pub enabled: bool,
    /// The catalog id of the model to run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// A model the user can download and run here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnDeviceModel {
    pub id: String,
    pub label: String,
    pub size: u64,
    /// `2.84 GB`.
    pub size_label: String,
    pub installed: bool,
    /// Bytes of an unfinished download (a new download resumes it).
    pub partial: u64,
    pub chosen: bool,
}

/// A download under way.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OnDeviceDownload {
    pub id: String,
    pub done: u64,
    pub total: u64,
}

/// What the shells show about the on-device model (in [`crate::AppState::on_device`]).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OnDeviceStatus {
    pub enabled: bool,
    /// The chosen model's id and label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_label: Option<String>,
    /// The chosen model is downloaded.
    #[serde(default)]
    pub installed: bool,
    /// The engine this build runs (`llama.cpp (GPU)`); `None` in a build without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<OnDeviceDownload>,
    /// What went wrong last (a failed download), until the next try.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl OnDeviceStatus {
    /// On, downloaded, and an engine to run it: it shows in the picker and takes chats.
    pub fn ready(&self) -> bool {
        self.enabled && self.installed && self.engine.is_some()
    }

    /// One line for Settings.
    pub fn line(&self) -> String {
        if let Some(d) = &self.download {
            let pct = d.done.saturating_mul(100).checked_div(d.total).unwrap_or(0);
            return format!("Downloading {} ({pct}% of {})…", d.id, weirdnet_model::human(d.total));
        }
        if let Some(e) = &self.error {
            return e.clone();
        }
        let label = self.model_label.clone().unwrap_or_else(|| "no model chosen".into());
        match (self.enabled, self.installed, &self.engine) {
            (false, _, _) => "Off. Nothing is downloaded or run on this device.".into(),
            (true, _, None) => "This build of Bragi has no on-device engine.".into(),
            (true, false, _) => format!("{label}: not downloaded yet."),
            (true, true, Some(engine)) => format!("{label}, ready ({engine})."),
        }
    }
}

/// The on-device model's live parts in [`crate::core::Inner`].
pub(crate) struct OnDevice {
    hub: Mutex<String>,
    catalog: Mutex<Vec<ModelSpec>>,
    engine: Mutex<Option<(String, EngineFactory)>>,
    worker: Mutex<Option<(PathBuf, Arc<Worker>)>>,
    download: Mutex<Option<Arc<AtomicBool>>>,
    error: Mutex<Option<String>>,
}

impl OnDevice {
    pub(crate) fn new() -> Self {
        Self {
            hub: Mutex::new(weirdnet_model::store::HUB.into()),
            catalog: Mutex::new(weirdnet_model::catalog::catalog()),
            engine: Mutex::new(built_in_engine()),
            worker: Mutex::new(None),
            download: Mutex::new(None),
            error: Mutex::new(None),
        }
    }
}

/// llama.cpp, in builds with the `on-device` feature.
#[cfg(feature = "on-device")]
fn built_in_engine() -> Option<(String, EngineFactory)> {
    use weirdnet_model::llama::{LlamaEngine, LlamaOptions};
    static DESCRIBED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let name = DESCRIBED.get_or_init(weirdnet_model::llama::describe).clone();
    let load: EngineFactory = Arc::new(|path: &Path| Ok(Box::new(LlamaEngine::load(path, LlamaOptions::default())?) as Box<dyn Engine>));
    Some((name, load))
}

#[cfg(not(feature = "on-device"))]
fn built_in_engine() -> Option<(String, EngineFactory)> {
    None
}

/// The model a device starts with: the smaller Gemma on phones, the larger elsewhere.
fn default_model(catalog: &[ModelSpec]) -> Option<String> {
    let device = if cfg!(any(target_os = "android", target_os = "ios")) { weirdnet_model::Device::Phone } else { weirdnet_model::Device::Desktop };
    let want = weirdnet_model::catalog::default_for(device).id;
    catalog.iter().find(|m| m.id == want).or(catalog.first()).map(|m| m.id.clone())
}

/// The chat as the local model reads it: text only, tool turns left out.
pub(crate) fn prompt(messages: &[Message]) -> Vec<ChatMessage> {
    use weirdnet_agent::Role;
    messages
        .iter()
        .filter_map(|m| {
            let role = match m.role {
                Role::System => ChatRole::System,
                Role::User => ChatRole::User,
                Role::Assistant => ChatRole::Assistant,
                _ => return None,
            };
            let text = m.text();
            (!text.trim().is_empty()).then(|| ChatMessage::new(role, text.into_owned()))
        })
        .collect()
}

/// `2.84 GB` (decimal, like download pages).
pub fn human(bytes: u64) -> String {
    weirdnet_model::human(bytes)
}

fn storage(e: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorKind::Storage, e.to_string())
}

impl Core {
    fn on_device_store(&self) -> ModelStore {
        ModelStore::new(self.inner.cfg.dir.join(MODELS_DIR)).with_hub(lock(&self.inner.on_device.hub).clone())
    }

    fn on_device_spec(&self, id: &str) -> Result<ModelSpec> {
        lock(&self.inner.on_device.catalog)
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, format!("no on-device model called {id}")))
    }

    /// This device's choices (off, no model, when never set).
    pub fn on_device_settings(&self) -> OnDeviceSettings {
        std::fs::read(self.inner.cfg.dir.join(SETTINGS_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save_on_device_settings(&self, s: &OnDeviceSettings) -> Result<()> {
        let json = serde_json::to_vec_pretty(s).map_err(storage)?;
        crate::files::write_atomic(&self.inner.cfg.dir.join(SETTINGS_FILE), &json)
    }

    /// The status as it is now (also kept in [`crate::AppState::on_device`]).
    pub fn on_device_status(&self) -> OnDeviceStatus {
        let s = self.on_device_settings();
        let spec = s.model.as_deref().and_then(|id| self.on_device_spec(id).ok());
        let store = self.on_device_store();
        OnDeviceStatus {
            enabled: s.enabled,
            model: s.model.clone(),
            model_label: spec.as_ref().map(|m| m.label.clone()),
            installed: spec.as_ref().is_some_and(|m| store.installed(m)),
            engine: lock(&self.inner.on_device.engine).as_ref().map(|(name, _)| name.clone()),
            download: self.state().on_device.download,
            error: lock(&self.inner.on_device.error).clone(),
        }
    }

    pub(crate) fn publish_on_device(&self) {
        let status = self.on_device_status();
        self.inner.state.send_modify(|s| s.on_device = status);
    }

    /// Turns the on-device model on or off. On picks the default model when none is chosen (it
    /// downloads nothing); off stops a download and unloads the model.
    pub fn set_on_device(&self, enabled: bool) -> Result<OnDeviceStatus> {
        let mut s = self.on_device_settings();
        s.enabled = enabled;
        if enabled && s.model.is_none() {
            s.model = default_model(&lock(&self.inner.on_device.catalog));
        }
        self.save_on_device_settings(&s)?;
        if !enabled {
            self.cancel_on_device_download();
            *lock(&self.inner.on_device.worker) = None;
        }
        self.publish_on_device();
        Ok(self.on_device_status())
    }

    /// The models this device can run, the chosen one marked.
    pub fn on_device_models(&self) -> Vec<OnDeviceModel> {
        let store = self.on_device_store();
        let chosen = self.on_device_settings().model;
        lock(&self.inner.on_device.catalog)
            .iter()
            .map(|m| OnDeviceModel {
                id: m.id.clone(),
                label: m.label.clone(),
                size: m.size,
                size_label: weirdnet_model::human(m.size),
                installed: store.installed(m),
                partial: store.partial_bytes(m),
                chosen: chosen.as_deref() == Some(m.id.as_str()),
            })
            .collect()
    }

    /// Runs `id` from now on (download it first if it isn't).
    pub fn choose_on_device_model(&self, id: &str) -> Result<()> {
        self.on_device_spec(id)?;
        let mut s = self.on_device_settings();
        s.model = Some(id.into());
        self.save_on_device_settings(&s)?;
        *lock(&self.inner.on_device.worker) = None;
        self.publish_on_device();
        Ok(())
    }

    /// What the user agrees to before `id` downloads: what, how big, from where, where it goes
    /// and how much disk is free.
    pub fn on_device_consent(&self, id: &str) -> Result<Vec<String>> {
        let spec = self.on_device_spec(id)?;
        let store = self.on_device_store();
        let mut lines = spec.consent_text(store.hub());
        let free = store.free_bytes();
        lines.push(format!(
            "Saved in {}{}.",
            store.dir().display(),
            if free > 0 { format!(" ({} free)", weirdnet_model::human(free)) } else { String::new() }
        ));
        lines.push("It runs only on this device; your chats with it stay here.".into());
        Ok(lines)
    }

    /// Downloads `id` in the background, resuming a partial one; progress and the outcome show
    /// in [`crate::AppState::on_device`]. `consent` is the user's yes to
    /// [`on_device_consent`](Self::on_device_consent): without it nothing downloads. A finished
    /// download becomes the chosen model when none is.
    pub fn download_on_device_model(&self, id: &str, consent: bool) -> Result<()> {
        let spec = self.on_device_spec(id)?;
        if !consent {
            return Err(CoreError::new(ErrorKind::BadRequest, "nothing is downloaded without your OK"));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut d = lock(&self.inner.on_device.download);
            if d.is_some() {
                return Err(CoreError::new(ErrorKind::Busy, "a model is already downloading"));
            }
            *d = Some(cancel.clone());
        }
        *lock(&self.inner.on_device.error) = None;
        let first = OnDeviceDownload { id: spec.id.clone(), done: self.on_device_store().partial_bytes(&spec), total: spec.size };
        self.inner.state.send_modify(|s| s.on_device.download = Some(first));
        self.publish_on_device();
        let core = self.clone();
        std::thread::Builder::new().name("bragi-model-download".into()).spawn(move || core.run_download(spec, cancel)).map_err(storage)?;
        Ok(())
    }

    fn run_download(&self, spec: ModelSpec, cancel: Arc<AtomicBool>) {
        let store = self.on_device_store();
        let mut last = Instant::now() - Duration::from_secs(1);
        let result = weirdnet_model::download(&store, &spec, &Consent::given(&spec), &mut |p| {
            if last.elapsed() >= Duration::from_millis(200) || p.done == p.total {
                last = Instant::now();
                let d = OnDeviceDownload { id: spec.id.clone(), done: p.done, total: p.total };
                self.inner.state.send_modify(|s| s.on_device.download = Some(d));
            }
            !cancel.load(Ordering::SeqCst)
        });
        *lock(&self.inner.on_device.download) = None;
        self.inner.state.send_modify(|s| s.on_device.download = None);
        match result {
            Ok(_) => {
                let mut s = self.on_device_settings();
                if s.model.is_none() {
                    s.model = Some(spec.id.clone());
                    if let Err(e) = self.save_on_device_settings(&s) {
                        tracing::warn!("saving the on-device model: {e}");
                    }
                }
            }
            Err(weirdnet_model::Error::Cancelled { .. }) => {}
            Err(e) => *lock(&self.inner.on_device.error) = Some(format!("Downloading {} failed: {e}", spec.label)),
        }
        self.publish_on_device();
    }

    /// Stops the download under way (what came is kept, so the next one resumes). False when
    /// none was running.
    pub fn cancel_on_device_download(&self) -> bool {
        match lock(&self.inner.on_device.download).as_ref() {
            Some(c) => {
                c.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Deletes `id`'s file and any partial download. True when a file was removed.
    pub fn remove_on_device_model(&self, id: &str) -> Result<bool> {
        let spec = self.on_device_spec(id)?;
        let store = self.on_device_store();
        if lock(&self.inner.on_device.worker).as_ref().is_some_and(|(p, _)| *p == store.path(&spec)) {
            *lock(&self.inner.on_device.worker) = None;
        }
        let removed = store.remove(&spec);
        self.publish_on_device();
        Ok(removed)
    }

    /// Runs models with `load` instead of this build's engine, named `name` in the status (tests,
    /// and embedders with their own engine).
    pub fn set_on_device_engine(&self, name: &str, load: EngineFactory) {
        *lock(&self.inner.on_device.engine) = Some((name.into(), load));
        *lock(&self.inner.on_device.worker) = None;
        self.publish_on_device();
    }

    /// Downloads from `hub` and offers `catalog` instead of the Hugging Face Hub and
    /// `weirdnet-model`'s catalog (tests use a local mock Hub and a tiny model).
    pub fn set_on_device_source(&self, hub: &str, catalog: Vec<ModelSpec>) {
        *lock(&self.inner.on_device.hub) = hub.into();
        *lock(&self.inner.on_device.catalog) = catalog;
        self.publish_on_device();
    }

    /// The chosen model and its file when the on-device model can answer.
    pub(crate) fn on_device_ready(&self) -> Option<ModelSpec> {
        let st = self.on_device_status();
        if !st.ready() {
            return None;
        }
        st.model.and_then(|id| self.on_device_spec(&id).ok())
    }

    /// Why `model` can't answer here, if it can't.
    pub(crate) fn check_on_device(&self, model: &str) -> Result<ModelSpec> {
        let st = self.on_device_status();
        if !st.enabled {
            return Err(CoreError::new(ErrorKind::Unavailable, "the on-device model is off; turn it on in Settings"));
        }
        if st.engine.is_none() {
            return Err(CoreError::new(ErrorKind::Unavailable, "this build of Bragi has no on-device engine"));
        }
        let id = if model.is_empty() { st.model.unwrap_or_default() } else { model.to_string() };
        let spec = self.on_device_spec(&id)?;
        if !self.on_device_store().installed(&spec) {
            return Err(CoreError::new(ErrorKind::Unavailable, format!("{} isn't downloaded; download it in Settings", spec.label)));
        }
        Ok(spec)
    }

    /// The worker running `spec` (loaded on first use, kept until the model changes).
    fn on_device_worker(&self, spec: &ModelSpec) -> Result<Arc<Worker>> {
        let path = self.on_device_store().path(spec);
        let mut w = lock(&self.inner.on_device.worker);
        if let Some((p, worker)) = w.as_ref()
            && *p == path
        {
            return Ok(worker.clone());
        }
        let load = lock(&self.inner.on_device.engine)
            .as_ref()
            .map(|(_, f)| f.clone())
            .ok_or_else(|| CoreError::new(ErrorKind::Unavailable, "this build of Bragi has no on-device engine"))?;
        let file = path.clone();
        let worker = Arc::new(Worker::spawn(move || load(&file)));
        *w = Some((path, worker.clone()));
        Ok(worker)
    }

    /// Answers `messages` with `spec` on this device; saves the answer in `conversation` (none
    /// when empty), ends the request and sends the last event.
    pub(crate) async fn answer_on_device(
        &self,
        conversation: &str,
        request_id: &str,
        spec: ModelSpec,
        messages: Vec<Message>,
        cancel: &CancelToken,
        out: &mpsc::UnboundedSender<ChatEvent>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel::<PoolEvent>();
        let _ = tx.send(PoolEvent::Accepted { node: LABEL.into(), model: spec.label.clone(), resume: false });
        let co = tokio::spawn(coalesce(rx, out.clone(), 0, 0));
        let result = match self.on_device_worker(&spec) {
            Err(e) => Err(e),
            Ok(worker) => {
                let mut g = worker.generate(GenerateRequest::new(prompt(&messages)));
                let mut stopping = false;
                loop {
                    tokio::select! {
                        chunk = g.recv() => match chunk {
                            Some(Chunk::Text(t)) => {
                                let _ = tx.send(PoolEvent::Text(t));
                            }
                            Some(Chunk::Done(stats)) => break match stats.stop {
                                weirdnet_model::worker::StopCause::Cancelled => Err(CoreError::new(ErrorKind::Cancelled, "stopped")),
                                weirdnet_model::worker::StopCause::MaxTokens => Ok("length"),
                                weirdnet_model::worker::StopCause::End => Ok("stop"),
                            },
                            Some(Chunk::Failed(m)) => break Err(CoreError::new(ErrorKind::Unavailable, format!("the on-device model failed: {m}"))),
                            None => break Err(CoreError::new(ErrorKind::Unavailable, "the on-device model stopped")),
                        },
                        _ = cancel.cancelled(), if !stopping => {
                            stopping = true;
                            g.cancel();
                        }
                    }
                }
            }
        };
        drop(tx);
        let s = co.await.unwrap_or_default();
        let tail = match result {
            Ok(reason) => {
                if !conversation.is_empty() {
                    self.finish_turn(conversation, s.text.clone(), &s, MessageState::Complete, Some(LABEL.into()), Some(spec.label.clone()));
                }
                ChatEvent::Done { finish_reason: Some(reason.into()) }
            }
            Err(e) => {
                let state = if e.kind == ErrorKind::Cancelled { MessageState::Stopped } else { MessageState::Failed };
                if !conversation.is_empty() {
                    self.finish_turn(conversation, s.text.clone(), &s, state, Some(LABEL.into()), Some(spec.label.clone()));
                }
                ChatEvent::Error { kind: e.kind, message: e.message, partial: s.text }
            }
        };
        lock(&self.inner.requests).remove(request_id);
        let _ = out.send(tail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_keeps_text_turns_only() {
        let p = prompt(&[Message::system("be brief"), Message::user("hi"), Message::tool("t1", "42"), Message::assistant("", Vec::new())]);
        assert_eq!(p, vec![ChatMessage::new(ChatRole::System, "be brief"), ChatMessage::new(ChatRole::User, "hi")]);
    }

    #[test]
    fn the_status_line_says_what_is_missing() {
        let mut s = OnDeviceStatus::default();
        assert!(s.line().starts_with("Off."));
        s.enabled = true;
        assert_eq!(s.line(), "This build of Bragi has no on-device engine.");
        s.engine = Some("llama.cpp (CPU)".into());
        s.model_label = Some("Tiny".into());
        assert_eq!(s.line(), "Tiny: not downloaded yet.");
        assert!(!s.ready());
        s.installed = true;
        assert!(s.ready());
        assert_eq!(s.line(), "Tiny, ready (llama.cpp (CPU)).");
        s.download = Some(OnDeviceDownload { id: "tiny".into(), done: 50, total: 200 });
        assert_eq!(s.line(), "Downloading tiny (25% of 200 bytes)…");
    }

    #[test]
    fn phones_start_small_and_desktops_large() {
        let c = weirdnet_model::catalog::catalog();
        let want = if cfg!(any(target_os = "android", target_os = "ios")) { "gemma-4-e2b" } else { "gemma-4-e4b" };
        assert_eq!(default_model(&c).as_deref(), Some(want));
        assert_eq!(default_model(&[]), None);
    }
}
