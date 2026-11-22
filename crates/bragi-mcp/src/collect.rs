//! Waiting for a whole answer: an MCP tool returns one result, not a stream.

use std::time::Duration;

use bragi_core::{ChatEvent, ChatHandle, Core, CoreError, ErrorKind};

/// A finished answer.
#[derive(Debug, Clone, Default)]
pub(crate) struct Answer {
    pub conversation: String,
    pub text: String,
    pub model: Option<String>,
    pub node: Option<String>,
    pub finish_reason: Option<String>,
}

/// Stops the request when the call is dropped before the answer ends (the client went away, or
/// the transport dropped the call).
struct StopOnDrop<'a> {
    core: &'a Core,
    request: String,
    done: bool,
}

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.core.cancel(&self.request);
        }
    }
}

/// Reads `handle` to its end, applying deltas at their offsets the way the shells do. Stops the
/// answer (and returns an error) when `timeout` passes first.
pub(crate) async fn answer(core: &Core, mut handle: ChatHandle, timeout: Duration) -> Result<Answer, CoreError> {
    let mut guard = StopOnDrop { core, request: handle.request_id.clone(), done: false };
    let mut out = Answer { conversation: handle.conversation_id.clone(), ..Answer::default() };
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        let ev = tokio::select! {
            ev = handle.events.recv() => ev,
            _ = &mut deadline => {
                return Err(CoreError::new(ErrorKind::Unavailable, format!("no full answer within {}s; it was stopped", timeout.as_secs())));
            }
        };
        let Some(ev) = ev else {
            guard.done = true;
            return Err(CoreError::new(ErrorKind::Unavailable, "the answer stream ended early"));
        };
        match ev {
            ChatEvent::Started { node, model, .. } => {
                out.node = Some(node);
                out.model = Some(model);
            }
            ChatEvent::Delta { offset, text } => {
                let keep: String = out.text.chars().take(offset as usize).collect();
                out.text = keep + &text;
            }
            ChatEvent::Done { finish_reason } => {
                guard.done = true;
                out.finish_reason = finish_reason;
                return Ok(out);
            }
            ChatEvent::Error { kind, message, .. } => {
                guard.done = true;
                return Err(CoreError::new(kind, message));
            }
            _ => {}
        }
    }
}
