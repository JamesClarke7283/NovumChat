//! Events pushed from the backend to every connected UI.
//!
//! In the desktop app they travel over Tauri's event system on
//! [`crate::EVENT_CHANNEL`]; in web mode over Server-Sent Events.

use serde::{Deserialize, Serialize};

use crate::types::{
    ApprovalDecision, ApprovalRequest, Chat, DeltaKind, Id, McpServerStatus, Message, ToastLevel, VaultStatus,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppEvent {
    /// A message was inserted (user message or assistant placeholder).
    MessageCreated {
        message: Message,
    },
    /// Streamed text for a message; apply with [`Message::push_delta`].
    MessageDelta {
        chat_id: Id,
        message_id: Id,
        kind: DeltaKind,
        text: String,
    },
    /// Full replacement of a message (tool call progress, status changes…).
    MessageUpdated {
        message: Message,
    },
    /// Messages were deleted (a message and its descendants).
    MessagesDeleted {
        chat_id: Id,
        ids: Vec<Id>,
    },
    /// Chat metadata changed (title, branch, settings, pin…).
    ChatUpdated {
        chat: Chat,
    },
    ChatDeleted {
        chat_id: Id,
    },
    ApprovalRequested {
        request: ApprovalRequest,
    },
    ApprovalResolved {
        id: Id,
        decision: ApprovalDecision,
    },
    McpStatus {
        servers: Vec<McpServerStatus>,
    },
    VaultChanged {
        status: VaultStatus,
    },
    ConfigChanged,
    Toast {
        level: ToastLevel,
        message: String,
    },
    /// The UI missed events (slow connection) and should reload its state.
    Resync,
}

impl AppEvent {
    /// Chat this event belongs to, if any.
    pub fn chat_id(&self) -> Option<&str> {
        match self {
            AppEvent::MessageCreated { message } | AppEvent::MessageUpdated { message } => Some(&message.chat_id),
            AppEvent::MessageDelta { chat_id, .. }
            | AppEvent::MessagesDeleted { chat_id, .. }
            | AppEvent::ChatDeleted { chat_id } => Some(chat_id),
            AppEvent::ChatUpdated { chat } => Some(&chat.id),
            AppEvent::ApprovalRequested { request } => Some(&request.chat_id),
            _ => None,
        }
    }
}
