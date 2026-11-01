//! Fan-out of [`AppEvent`]s to the desktop webview and web-mode clients.

use std::sync::{Arc, OnceLock};

use novumchat_core::events::AppEvent;
use novumchat_core::types::ToastLevel;
use tokio::sync::broadcast;

/// Something that can deliver events to a UI (the Tauri webview).
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &AppEvent);
}

pub struct EventHub {
    sink: OnceLock<Arc<dyn EventSink>>,
    /// Serialised events for Server-Sent-Event subscribers (web mode).
    tx: broadcast::Sender<Arc<str>>,
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

impl EventHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(8192);
        Self { sink: OnceLock::new(), tx }
    }

    pub fn set_sink(&self, sink: Arc<dyn EventSink>) {
        let _ = self.sink.set(sink);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<str>> {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: AppEvent) {
        if let Some(sink) = self.sink.get() {
            sink.emit(&event);
        }
        if self.tx.receiver_count() > 0 {
            match serde_json::to_string(&event) {
                Ok(json) => {
                    let _ = self.tx.send(Arc::from(json));
                }
                Err(e) => tracing::error!("failed to serialise event: {e}"),
            }
        }
    }

    pub fn toast(&self, level: ToastLevel, message: impl Into<String>) {
        self.emit(AppEvent::Toast { level, message: message.into() });
    }
}
