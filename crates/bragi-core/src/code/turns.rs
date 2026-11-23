//! The conversation sent along with a code request: a plan made in chat mode and earlier code
//! turns go to the model between the code instructions and the files, so chat and code are one
//! conversation. Shared by the terminal app and the phone's code mode.

use crate::code::budget::{PER_MESSAGE, estimate_tokens};
use crate::code::{CodeRequest, PromptMessage, PromptRole};
use crate::error::Result;
use crate::{ChatMessage, MessageState, Role};

/// The conversation as prompt turns, newest kept, within `max` estimated tokens: text only,
/// streaming and empty answers left out, turns from the same side joined. Returns the turns and
/// their estimate.
pub fn conversation_turns(messages: &[ChatMessage], max: usize) -> (Vec<PromptMessage>, usize) {
    let done: Vec<ChatMessage> = messages.iter().filter(|m| m.state != MessageState::Streaming && !m.text.trim().is_empty()).cloned().collect();
    let mut merged = crate::merge_turns(&done);
    // The ask that follows is a user turn: the history has to end with an answer.
    while merged.last().is_some_and(|m| m.role == Role::User) {
        merged.pop();
    }
    let mut out = Vec::new();
    let mut used = 0;
    for m in merged.iter().rev() {
        let n = estimate_tokens(&m.text) + PER_MESSAGE;
        if used + n > max {
            break;
        }
        used += n;
        let role = if m.role == Role::User { PromptRole::User } else { PromptRole::Assistant };
        out.push(PromptMessage { role, content: m.text.clone() });
    }
    out.reverse();
    // And start with a question.
    while out.first().is_some_and(|m| m.role == PromptRole::Assistant) {
        let m = out.remove(0);
        used = used.saturating_sub(estimate_tokens(&m.content) + PER_MESSAGE);
    }
    (out, used)
}

/// The prompt for `req`: the code instructions, then the conversation so far, then the files
/// and the ask.
pub fn messages(req: &CodeRequest, conversation: &[PromptMessage]) -> Result<Vec<PromptMessage>> {
    let mut m = crate::code::prompt::build(req)?;
    let at = m.iter().take_while(|x| x.role == PromptRole::System).count();
    m.splice(at..at, conversation.iter().cloned());
    Ok(m)
}
