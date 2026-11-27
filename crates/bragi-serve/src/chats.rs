//! What an editor does with a chat besides sending to it (LF-40): open one, react to a message,
//! delete a message (with an undo), file chats in folders, and show a link code for another
//! device. Thin routes over the [`Core`] methods the apps use; nothing new is decided here.
//!
//! - `GET /v1/sessions/{id}`: the conversation with its messages (attachments included), its
//!   folder and target.
//! - `POST /v1/sessions/{id}/messages/{message}/react {emoji}`: toggles a reaction; returns the
//!   message's reactions.
//! - `DELETE /v1/sessions/{id}/messages/{message}?with_reply=true`: deletes a message (and the
//!   answer that followed it); returns `{undo, messages, ids}`. `POST /v1/undo {undo}` puts it back.
//! - `GET|POST /v1/folders`, `POST /v1/folders/{id}/rename {name}`, `POST /v1/folders/{id}/move
//!   {parent}`, `DELETE /v1/folders/{id}` (what it holds moves up), `POST /v1/sessions/{id}/folder
//!   {folder}` (`null` takes it out).
//! - `POST /v1/link/offer`: a link code for a device already in the user's Weirdnet (`{link,
//!   expires_ms}`); progress is `pair` in `GET /v1/state`. `POST /v1/link/offer/confirm {matches}`
//!   once the codes were compared, `POST /v1/link/offer/cancel` stops showing it.

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bragi_core::Attachment;
use serde::Deserialize;
use serde_json::json;

use super::{App, core_error, error};

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/v1/sessions/{id}/messages/{message}/react", post(react))
        .route("/v1/sessions/{id}/messages/{message}", delete(delete_message))
        .route("/v1/sessions/{id}/folder", post(set_folder))
        .route("/v1/undo", post(undo))
        .route("/v1/folders", get(folders).post(create_folder))
        .route("/v1/folders/{id}", delete(delete_folder))
        .route("/v1/folders/{id}/rename", post(rename_folder))
        .route("/v1/folders/{id}/move", post(move_folder))
        .route("/v1/link/offer", post(offer))
        .route("/v1/link/offer/confirm", post(offer_confirm))
        .route("/v1/link/offer/cancel", post(offer_cancel))
}

/// A file sent with a chat turn: `text` (a file or a selection), or `data` (base64: an image,
/// audio, or a text file read as bytes). Sniffed and limited by [`Attachment::from_bytes`].
#[derive(Deserialize)]
pub(crate) struct AttachmentBody {
    name: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    data: Option<String>,
}

impl AttachmentBody {
    pub(crate) fn read(&self) -> bragi_core::Result<Attachment> {
        let bad = |m: String| bragi_core::CoreError::new(bragi_core::ErrorKind::BadRequest, m);
        match (&self.text, &self.data) {
            (Some(t), None) => Attachment::from_bytes(&self.name, t.as_bytes()),
            (None, Some(d)) => {
                Attachment::from_bytes(&self.name, &STANDARD.decode(d.trim()).map_err(|_| bad(format!("{}: the data is not base64", self.name)))?)
            }
            _ => Err(bad(format!("{}: send `text` or `data`, one of them", self.name))),
        }
    }
}

pub(crate) async fn open(State(app): State<App>, Path(id): Path<String>) -> Response {
    match app.core.conversation(&id) {
        Ok(c) => {
            let meta = app.core.session_target(&id);
            Json(json!({ "conversation": c, "folder": meta.folder, "target": meta.target, "unbound": meta.unbound })).into_response()
        }
        Err(e) => core_error(e),
    }
}

#[derive(Deserialize)]
struct ReactBody {
    emoji: String,
}

async fn react(State(app): State<App>, Path((id, message)): Path<(String, String)>, Json(b): Json<ReactBody>) -> Response {
    match app.core.react(&id, &message, &b.emoji) {
        Ok(reactions) => Json(json!({ "reactions": reactions })).into_response(),
        Err(e) => core_error(e),
    }
}

async fn delete_message(State(app): State<App>, Path((id, message)): Path<(String, String)>, RawQuery(q): RawQuery) -> Response {
    let with_reply = q.unwrap_or_default().split('&').any(|p| p == "with_reply=true" || p == "with_reply=1");
    // Which messages go: the core removes the answer that followed with `with_reply`.
    let mut ids = vec![message.clone()];
    if with_reply
        && let Ok(c) = app.core.conversation(&id)
        && let Some(r) = c.reply_to(&message)
    {
        ids.push(r.id.clone());
    }
    match app.core.delete_message(&id, &message, with_reply) {
        Ok(d) => Json(json!({ "undo": d.undo, "messages": d.messages, "ids": ids })).into_response(),
        Err(e) => core_error(e),
    }
}

#[derive(Deserialize)]
struct UndoBody {
    undo: String,
}

async fn undo(State(app): State<App>, Json(b): Json<UndoBody>) -> Response {
    match app.core.undo_delete(&b.undo) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => core_error(e),
    }
}

async fn folders(State(app): State<App>) -> Json<serde_json::Value> {
    Json(json!({ "folders": app.core.folders() }))
}

#[derive(Deserialize)]
struct FolderBody {
    name: String,
    #[serde(default)]
    parent: Option<String>,
}

async fn create_folder(State(app): State<App>, Json(b): Json<FolderBody>) -> Response {
    match app.core.create_folder(&b.name, b.parent.as_deref()) {
        Ok(f) => (StatusCode::CREATED, Json(json!(f))).into_response(),
        Err(e) => core_error(e),
    }
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

async fn rename_folder(State(app): State<App>, Path(id): Path<String>, Json(b): Json<NameBody>) -> Response {
    ok(app.core.rename_folder(&id, &b.name))
}

#[derive(Deserialize)]
struct ParentBody {
    #[serde(default)]
    parent: Option<String>,
}

async fn move_folder(State(app): State<App>, Path(id): Path<String>, Json(b): Json<ParentBody>) -> Response {
    ok(app.core.move_folder(&id, b.parent.as_deref()))
}

async fn delete_folder(State(app): State<App>, Path(id): Path<String>) -> Response {
    ok(app.core.delete_folder(&id))
}

#[derive(Deserialize)]
struct SessionFolderBody {
    #[serde(default)]
    folder: Option<String>,
}

async fn set_folder(State(app): State<App>, Path(id): Path<String>, Json(b): Json<SessionFolderBody>) -> Response {
    ok(app.core.set_session_folder(&id, b.folder.as_deref()))
}

fn ok(r: bragi_core::Result<()>) -> Response {
    match r {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => core_error(e),
    }
}

async fn offer(State(app): State<App>) -> Response {
    match app.core.pair_offer().await {
        Ok(link) => {
            let expires_ms = match app.core.state().pair {
                bragi_core::PairState::Offering { expires_ms, .. } => Some(expires_ms),
                _ => None,
            };
            Json(json!({ "link": link, "expires_ms": expires_ms })).into_response()
        }
        Err(e) => core_error(e),
    }
}

#[derive(Deserialize)]
struct MatchesBody {
    matches: bool,
}

async fn offer_confirm(State(app): State<App>, Json(b): Json<MatchesBody>) -> Response {
    match app.core.pair_confirm(b.matches) {
        Ok(()) => Json(json!({ "ok": true, "pair": app.core.state().pair })).into_response(),
        Err(e) if e.kind == bragi_core::ErrorKind::Pairing => error(StatusCode::CONFLICT, "pairing", e.message),
        Err(e) => core_error(e),
    }
}

async fn offer_cancel(State(app): State<App>) -> Json<serde_json::Value> {
    app.core.pair_cancel();
    Json(json!({ "ok": true, "pair": app.core.state().pair }))
}
