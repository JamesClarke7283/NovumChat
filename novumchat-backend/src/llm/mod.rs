//! Provider-agnostic streaming chat.
//!
//! * [`openai`]: any OpenAI-compatible endpoint through `async-openai`.
//! * [`ollama`]: the native Ollama API through `ollama-rs`.

pub mod ollama;
pub mod openai;
pub mod think;

use std::time::Duration;

use anyhow::Result;
use novumchat_core::config::{ProviderConfig, ProviderKind};
use novumchat_core::types::{ModelInfo, Usage};
use tokio_util::sync::CancellationToken;

/// How long a stream may stay silent before it is considered dead.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    pub mime: String,
    pub base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    /// JSON object as a string.
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmMessage {
    System(String),
    User { text: String, images: Vec<ImageData> },
    Assistant { text: String, tool_calls: Vec<LlmToolCall> },
    Tool { call_id: String, name: String, content: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments object.
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<ToolSpec>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Text(String),
    Reasoning(String),
    /// Complete tool calls requested in this round.
    ToolCalls(Vec<LlmToolCall>),
    Usage(Usage),
    /// The provider rejected a feature and the request was retried without it.
    Notice(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    Stop,
    ToolCalls,
    Length,
    Cancelled,
}

/// A configured endpoint.
pub enum Provider {
    OpenAi(openai::OpenAiProvider),
    Ollama(ollama::OllamaProvider),
}

impl Provider {
    pub fn new(cfg: &ProviderConfig) -> Result<Self> {
        Ok(match cfg.kind {
            ProviderKind::Openai => Provider::OpenAi(openai::OpenAiProvider::new(cfg)?),
            ProviderKind::Ollama => Provider::Ollama(ollama::OllamaProvider::new(cfg)?),
        })
    }

    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        match self {
            Provider::OpenAi(p) => p.list_models().await,
            Provider::Ollama(p) => p.list_models().await,
        }
    }

    /// Streams one model round. Events are delivered through `on_event`.
    pub async fn stream_chat(
        &self,
        req: &ChatRequest,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<Finish> {
        match self {
            Provider::OpenAi(p) => p.stream_chat(req, cancel, on_event).await,
            Provider::Ollama(p) => p.stream_chat(req, cancel, on_event).await,
        }
    }
}

/// Resolves an API key from the config or its environment variable.
pub fn resolve_api_key(cfg: &ProviderConfig) -> Option<String> {
    cfg.api_key.as_deref().map(str::trim).filter(|k| !k.is_empty()).map(String::from).or_else(|| {
        cfg.api_key_env
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .and_then(|v| std::env::var(v.trim()).ok())
            .filter(|k| !k.trim().is_empty())
    })
}

/// Pulls a readable message out of an error body (`{"error": {"message": …}}` and friends).
pub fn error_message_from_body(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        let candidates =
            [v.pointer("/error/message"), v.pointer("/error"), v.pointer("/message"), v.pointer("/detail")];
        for c in candidates.into_iter().flatten() {
            if let Some(s) = c.as_str() {
                return s.to_string();
            }
        }
    }
    let t = body.trim();
    if t.is_empty() { "empty error response".into() } else { novumchat_core::util::truncate_chars(t, 400) }
}

/// Is this error the provider saying the model can't use tools?
pub fn is_tools_unsupported(msg: &str) -> bool {
    let m = msg.to_lowercase();
    (m.contains("tool") || m.contains("function"))
        && (m.contains("not support")
            || m.contains("unsupported")
            || m.contains("does not support")
            || m.contains("not available"))
}

/// Is this error the provider rejecting thinking/reasoning parameters?
pub fn is_thinking_unsupported(msg: &str) -> bool {
    let m = msg.to_lowercase();
    (m.contains("think") || m.contains("reasoning"))
        && (m.contains("not support")
            || m.contains("unsupported")
            || m.contains("invalid")
            || m.contains("unrecognized"))
}

/// Is this error about an optional request field the server doesn't know?
pub fn is_param_unsupported(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("stream_options")
        || m.contains("unsupported parameter")
        || m.contains("unrecognized request argument")
        || m.contains("extra inputs are not permitted")
        || m.contains("unknown field")
        || m.contains("max_tokens")
        || m.contains("temperature")
}

/// Heuristic vision support for model names without metadata.
pub fn guess_vision(model: &str) -> bool {
    let m = model.to_lowercase();
    const HINTS: &[&str] = &[
        "vision",
        "-vl",
        "vl-",
        "llava",
        "gpt-4o",
        "gpt-4.1",
        "gpt-5",
        "o3",
        "o4",
        "claude",
        "gemini",
        "pixtral",
        "gemma3",
        "gemma-3",
        "llama4",
        "llama-4",
        "qwen2.5vl",
        "qwen3-vl",
        "minicpm-v",
        "moondream",
        "granite3.2-vision",
        "mistral-small3",
        "bakllava",
        "omni",
        "multimodal",
    ];
    HINTS.iter().any(|h| m.contains(h))
}

/// Heuristic reasoning support for model names without metadata.
pub fn guess_reasoning(model: &str) -> bool {
    let m = model.to_lowercase();
    const HINTS: &[&str] = &[
        "r1",
        "reason",
        "think",
        "qwq",
        "o1",
        "o3",
        "o4",
        "gpt-5",
        "gpt-oss",
        "qwen3",
        "magistral",
        "deepseek-v3.1",
        "phi4-reasoning",
    ];
    HINTS.iter().any(|h| m.contains(h))
}

/// Shared HTTP client settings.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .user_agent(concat!("NovumChat/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_bodies() {
        assert_eq!(error_message_from_body(r#"{"error":{"message":"bad key"}}"#), "bad key");
        assert_eq!(error_message_from_body(r#"{"error":"model not found"}"#), "model not found");
        assert_eq!(error_message_from_body("oops"), "oops");
    }

    #[test]
    fn feature_errors() {
        assert!(is_tools_unsupported("registry.ollama.ai/library/gemma:2b does not support tools"));
        assert!(is_thinking_unsupported("\"llama3\" does not support thinking"));
        assert!(is_param_unsupported("Unsupported parameter: 'max_tokens' is not supported with this model."));
        assert!(!is_tools_unsupported("rate limit exceeded"));
    }

    #[test]
    fn keys() {
        let cfg = ProviderConfig { api_key: Some("  sk-1 ".into()), ..Default::default() };
        assert_eq!(resolve_api_key(&cfg).as_deref(), Some("sk-1"));
        let cfg = ProviderConfig { api_key_env: Some("NOVUMCHAT_TEST_MISSING_KEY".into()), ..Default::default() };
        assert_eq!(resolve_api_key(&cfg), None);
    }
}
