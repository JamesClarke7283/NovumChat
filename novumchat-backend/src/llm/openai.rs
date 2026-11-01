//! OpenAI-compatible chat completions via `async-openai`.
//!
//! Uses the crate's *bring-your-own-types* streaming API so that fields beyond
//! the official schema (`reasoning_content` from DeepSeek/vLLM, `reasoning`
//! from OpenRouter, in-stream `error` objects) are understood, and so that the
//! request can be adapted for servers that reject optional parameters.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use async_openai::Client;
use async_openai::config::OpenAIConfig;
use futures::StreamExt;
use novumchat_core::config::{ProviderConfig, ProviderKind};
use novumchat_core::types::{ModelInfo, Usage};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::think::{Piece, ThinkSplitter};
use super::{
    ChatRequest, Finish, IDLE_TIMEOUT, LlmMessage, LlmToolCall, StreamEvent, error_message_from_body, guess_reasoning,
    guess_vision, http_client, is_param_unsupported, is_thinking_unsupported, is_tools_unsupported, resolve_api_key,
};

pub struct OpenAiProvider {
    client: Client<OpenAIConfig>,
    cfg: ProviderConfig,
    official: bool,
}

/// `http://host:port` → `http://host:port/v1`; anything with a path is kept.
pub fn normalize_base(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    match url::Url::parse(url) {
        Ok(u) if u.path().is_empty() || u.path() == "/" => format!("{url}/v1"),
        _ => url.to_string(),
    }
}

impl OpenAiProvider {
    pub fn new(cfg: &ProviderConfig) -> Result<Self> {
        let base = normalize_base(&cfg.base_url);
        if base.is_empty() {
            return Err(anyhow!("Provider `{}` has no base URL", cfg.id));
        }
        // Set every field explicitly: the defaults read OPENAI_* environment
        // variables, which must never leak to third-party endpoints.
        let mut oc = OpenAIConfig::new()
            .with_api_base(base.clone())
            .with_api_key(resolve_api_key(cfg).unwrap_or_default())
            .with_org_id("")
            .with_project_id("");
        for (k, v) in &cfg.headers {
            let name = reqwest::header::HeaderName::from_bytes(k.trim().as_bytes())
                .map_err(|e| anyhow!("invalid header name `{k}`: {e}"))?;
            oc = oc.with_header(name, v.as_str()).map_err(|e| anyhow!("invalid header `{k}`: {e}"))?;
        }
        let client = Client::with_config(oc).with_http_client(http_client());
        let official = base.contains("api.openai.com");
        Ok(Self { client, cfg: cfg.clone(), official })
    }

    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let mut out: BTreeMap<String, ModelInfo> = BTreeMap::new();
        if self.cfg.discover_models {
            let v: Value = self.client.models().list_byot().await.map_err(|e| anyhow!("{}", describe_error(&e)))?;
            let items = v.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
            for item in items {
                let Some(id) = item.get("id").and_then(Value::as_str) else { continue };
                if self.official && !is_chat_model(id) {
                    continue;
                }
                let modalities = item.pointer("/architecture/input_modalities").and_then(Value::as_array);
                let vision = modalities
                    .map(|m| m.iter().any(|x| x.as_str() == Some("image")))
                    .unwrap_or_else(|| guess_vision(id));
                let params = item.get("supported_parameters").and_then(Value::as_array);
                let tools = params.map(|p| p.iter().any(|x| x.as_str() == Some("tools")));
                let reasoning = params
                    .map(|p| p.iter().any(|x| matches!(x.as_str(), Some("reasoning" | "include_reasoning"))))
                    .unwrap_or_else(|| guess_reasoning(id));
                out.insert(id.to_string(), self.model_info(id, Some(vision), tools, Some(reasoning)));
            }
        }
        for m in &self.cfg.models {
            out.entry(m.clone())
                .or_insert_with(|| self.model_info(m, Some(guess_vision(m)), None, Some(guess_reasoning(m))));
        }
        Ok(out.into_values().collect())
    }

    fn model_info(&self, id: &str, vision: Option<bool>, tools: Option<bool>, reasoning: Option<bool>) -> ModelInfo {
        ModelInfo {
            provider: self.cfg.id.clone(),
            provider_name: self.cfg.name.clone(),
            provider_kind: ProviderKind::Openai,
            id: id.to_string(),
            vision,
            tools,
            reasoning,
            size_bytes: None,
        }
    }

    pub async fn stream_chat(
        &self,
        req: &ChatRequest,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<Finish> {
        let mut opts = BodyOptions { tools: !req.tools.is_empty(), extras: true, reasoning: true };
        let mut attempts = 0;
        let stream = loop {
            attempts += 1;
            let body = build_body(req, &opts, self.official);
            let chat = self.client.chat();
            let result = tokio::select! {
                _ = cancel.cancelled() => return Ok(Finish::Cancelled),
                r = chat.create_stream_byot::<Value, Chunk>(body) => r,
            };
            match result {
                Ok(s) => break s,
                Err(e) => {
                    let msg = describe_error(&e);
                    if attempts < 4 {
                        if opts.tools && is_tools_unsupported(&msg) {
                            opts.tools = false;
                            on_event(StreamEvent::Notice(format!(
                                "{} does not support tools; answering without them.",
                                req.model
                            )));
                            continue;
                        }
                        if opts.reasoning && req.reasoning_effort.is_some() && is_thinking_unsupported(&msg) {
                            opts.reasoning = false;
                            on_event(StreamEvent::Notice(format!("{} does not support reasoning effort.", req.model)));
                            continue;
                        }
                        if opts.extras && is_param_unsupported(&msg) {
                            opts.extras = false;
                            continue;
                        }
                    }
                    return Err(anyhow!("{msg}"));
                }
            }
        };
        consume_stream(stream, cancel, on_event).await
    }
}

/// OpenAI's own `/models` lists embeddings, TTS, image models… keep chat ones.
fn is_chat_model(id: &str) -> bool {
    let id = id.to_lowercase();
    let chatty = id.starts_with("gpt-")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || id.starts_with("chatgpt")
        || id.starts_with("ft:gpt");
    let excluded = [
        "embedding",
        "tts",
        "whisper",
        "dall-e",
        "image",
        "audio",
        "realtime",
        "transcribe",
        "moderation",
        "search",
        "instruct",
        "codex",
    ];
    chatty && !excluded.iter().any(|x| id.contains(x))
}

struct BodyOptions {
    tools: bool,
    /// stream_options, temperature, max tokens.
    extras: bool,
    reasoning: bool,
}

fn message_json(m: &LlmMessage) -> Value {
    match m {
        LlmMessage::System(text) => json!({ "role": "system", "content": text }),
        LlmMessage::User { text, images } => {
            if images.is_empty() {
                json!({ "role": "user", "content": text })
            } else {
                let mut parts = vec![json!({ "type": "text", "text": text })];
                for img in images {
                    parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:{};base64,{}", img.mime, img.base64) }
                    }));
                }
                json!({ "role": "user", "content": parts })
            }
        }
        LlmMessage::Assistant { text, tool_calls } => {
            if tool_calls.is_empty() {
                json!({ "role": "assistant", "content": text })
            } else {
                let calls: Vec<Value> = tool_calls
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "type": "function",
                            "function": { "name": c.name, "arguments": c.arguments }
                        })
                    })
                    .collect();
                let content = if text.is_empty() { Value::Null } else { Value::String(text.clone()) };
                json!({ "role": "assistant", "content": content, "tool_calls": calls })
            }
        }
        LlmMessage::Tool { call_id, content, .. } => {
            json!({ "role": "tool", "tool_call_id": call_id, "content": content })
        }
    }
}

fn build_body(req: &ChatRequest, opts: &BodyOptions, official: bool) -> Value {
    let mut messages: Vec<Value> = Vec::with_capacity(req.messages.len());
    for m in &req.messages {
        // Without tools, earlier tool rounds are flattened to plain text so
        // servers that reject tool messages still accept the history.
        match m {
            LlmMessage::Assistant { text, tool_calls } if !opts.tools && !tool_calls.is_empty() => {
                let mut s = text.clone();
                for c in tool_calls {
                    s.push_str(&format!("\n\n[Called tool `{}` with {}]", c.name, c.arguments));
                }
                messages.push(json!({ "role": "assistant", "content": s.trim() }));
            }
            LlmMessage::Tool { name, content, .. } if !opts.tools => {
                messages.push(json!({ "role": "user", "content": format!("[Result of tool `{name}`]\n{content}") }));
            }
            _ => messages.push(message_json(m)),
        }
    }
    let mut body = json!({ "model": req.model, "messages": messages, "stream": true });
    let obj = body.as_object_mut().expect("object");
    if opts.tools && !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
                })
            })
            .collect();
        obj.insert("tools".into(), Value::Array(tools));
    }
    if opts.extras {
        obj.insert("stream_options".into(), json!({ "include_usage": true }));
        if let Some(t) = req.temperature {
            obj.insert("temperature".into(), json!(t));
        }
        if let Some(n) = req.max_tokens {
            let key = if official { "max_completion_tokens" } else { "max_tokens" };
            obj.insert(key.into(), json!(n));
        }
    }
    if opts.reasoning
        && let Some(effort) = req.reasoning_effort.as_deref().filter(|e| !e.is_empty())
    {
        obj.insert("reasoning_effort".into(), json!(effort));
    }
    body
}

fn describe_error(e: &async_openai::error::OpenAIError) -> String {
    use async_openai::error::OpenAIError;
    match e {
        OpenAIError::Reqwest(re) => {
            if re.is_connect() {
                "Could not connect to the server. Is it running and is the base URL correct?".into()
            } else if re.is_timeout() {
                "The server took too long to respond.".into()
            } else {
                format!("HTTP error: {re}")
            }
        }
        OpenAIError::JSONDeserialize(_, content) => error_message_from_body(content),
        other => other.to_string(),
    }
}

#[derive(Debug, Deserialize, Default)]
struct Chunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<ChunkUsage>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct ChunkChoice {
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Deserialize, Default)]
struct ToolCallDelta {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FnDelta>,
}

#[derive(Debug, Deserialize, Default)]
struct FnDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ChunkUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
}

/// Accumulates streamed tool-call fragments.
#[derive(Default)]
struct ToolAccumulator {
    calls: BTreeMap<u32, (String, String, String)>,
    next_index: u32,
}

impl ToolAccumulator {
    fn push(&mut self, d: ToolCallDelta) {
        let index = match d.index {
            Some(i) => i,
            None => {
                // Providers without indices send each call whole.

                self.next_index
            }
        };
        self.next_index = self.next_index.max(index + 1);
        let entry = self.calls.entry(index).or_default();
        if let Some(id) = d.id.filter(|s| !s.is_empty()) {
            entry.0 = id;
        }
        if let Some(f) = d.function {
            if let Some(n) = f.name {
                entry.1.push_str(&n);
            }
            if let Some(a) = f.arguments {
                entry.2.push_str(&a);
            }
        }
    }

    fn finish(self) -> Vec<LlmToolCall> {
        self.calls
            .into_iter()
            .filter(|(_, (_, name, _))| !name.is_empty())
            .map(|(i, (id, name, args))| LlmToolCall {
                id: if id.is_empty() { format!("call_{i}_{}", uuid::Uuid::new_v4().simple()) } else { id },
                name,
                arguments: if args.trim().is_empty() { "{}".into() } else { args },
            })
            .collect()
    }
}

async fn consume_stream(
    mut stream: async_openai::types::stream::StreamResponse<Chunk>,
    cancel: &CancellationToken,
    on_event: &mut (dyn FnMut(StreamEvent) + Send),
) -> Result<Finish> {
    let mut splitter = ThinkSplitter::new();
    let mut tools = ToolAccumulator::default();
    let mut finish = Finish::Stop;
    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => return Ok(Finish::Cancelled),
            n = tokio::time::timeout(IDLE_TIMEOUT, stream.next()) => n,
        };
        let item = match next {
            Err(_) => return Err(anyhow!("The model stopped responding (no data for 5 minutes).")),
            Ok(None) => break,
            Ok(Some(item)) => item,
        };
        let chunk = match item {
            Ok(c) => c,
            Err(e) => return Err(anyhow!("{}", describe_error(&e))),
        };
        if let Some(err) = chunk.error {
            let msg = err.get("message").and_then(Value::as_str).map(String::from).unwrap_or_else(|| err.to_string());
            return Err(anyhow!("{msg}"));
        }
        if let Some(u) = chunk.usage {
            on_event(StreamEvent::Usage(Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
            }));
        }
        for choice in chunk.choices {
            if let Some(delta) = choice.delta {
                if let Some(r) = delta.reasoning_content.or(delta.reasoning).filter(|r| !r.is_empty()) {
                    on_event(StreamEvent::Reasoning(r));
                }
                if let Some(c) = delta.content.filter(|c| !c.is_empty()) {
                    for piece in splitter.push(&c) {
                        match piece {
                            Piece::Text(t) => on_event(StreamEvent::Text(t)),
                            Piece::Reasoning(r) => on_event(StreamEvent::Reasoning(r)),
                        }
                    }
                }
                if let Some(calls) = delta.tool_calls {
                    for c in calls {
                        tools.push(c);
                    }
                }
            }
            match choice.finish_reason.as_deref() {
                Some("tool_calls") | Some("function_call") => finish = Finish::ToolCalls,
                Some("length") => finish = Finish::Length,
                _ => {}
            }
        }
    }
    for piece in splitter.finish() {
        match piece {
            Piece::Text(t) => on_event(StreamEvent::Text(t)),
            Piece::Reasoning(r) => on_event(StreamEvent::Reasoning(r)),
        }
    }
    let calls = tools.finish();
    if !calls.is_empty() {
        on_event(StreamEvent::ToolCalls(calls));
        finish = Finish::ToolCalls;
    }
    Ok(finish)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ImageData, ToolSpec};

    #[test]
    fn base_urls() {
        assert_eq!(normalize_base("http://localhost:1234"), "http://localhost:1234/v1");
        assert_eq!(normalize_base("http://localhost:1234/"), "http://localhost:1234/v1");
        assert_eq!(normalize_base("https://openrouter.ai/api/v1/"), "https://openrouter.ai/api/v1");
    }

    #[test]
    fn body_shapes() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![
                LlmMessage::System("sys".into()),
                LlmMessage::User {
                    text: "look".into(),
                    images: vec![ImageData { mime: "image/png".into(), base64: "AAA".into() }],
                },
                LlmMessage::Assistant {
                    text: String::new(),
                    tool_calls: vec![LlmToolCall {
                        id: "c1".into(),
                        name: "bash".into(),
                        arguments: "{\"command\":\"ls\"}".into(),
                    }],
                },
                LlmMessage::Tool { call_id: "c1".into(), name: "bash".into(), content: "file.txt".into() },
            ],
            tools: vec![ToolSpec {
                name: "bash".into(),
                description: "d".into(),
                parameters: json!({"type":"object"}),
            }],
            temperature: Some(0.5),
            max_tokens: Some(100),
            reasoning_effort: None,
        };
        let b = build_body(&req, &BodyOptions { tools: true, extras: true, reasoning: true }, true);
        assert_eq!(b["messages"][1]["content"][1]["image_url"]["url"], "data:image/png;base64,AAA");
        assert_eq!(b["messages"][2]["tool_calls"][0]["function"]["name"], "bash");
        assert!(b["messages"][2]["content"].is_null());
        assert_eq!(b["messages"][3]["tool_call_id"], "c1");
        assert_eq!(b["max_completion_tokens"], 100);
        assert_eq!(b["tools"][0]["function"]["name"], "bash");
        // Without tools the history is flattened.
        let b = build_body(&req, &BodyOptions { tools: false, extras: false, reasoning: false }, false);
        assert!(b.get("tools").is_none());
        assert!(b.get("stream_options").is_none());
        assert_eq!(b["messages"][3]["role"], "user");
        assert!(b["messages"][2]["content"].as_str().unwrap().contains("Called tool `bash`"));
    }

    #[test]
    fn tool_call_accumulation() {
        let mut acc = ToolAccumulator::default();
        acc.push(ToolCallDelta {
            index: Some(0),
            id: Some("a".into()),
            function: Some(FnDelta { name: Some("read_file".into()), arguments: Some("{\"pa".into()) }),
        });
        acc.push(ToolCallDelta {
            index: Some(0),
            id: None,
            function: Some(FnDelta { name: None, arguments: Some("th\":\"x\"}".into()) }),
        });
        acc.push(ToolCallDelta {
            index: Some(1),
            id: Some("b".into()),
            function: Some(FnDelta { name: Some("bash".into()), arguments: None }),
        });
        let calls = acc.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments, "{\"path\":\"x\"}");
        assert_eq!(calls[1].arguments, "{}");
    }

    #[test]
    fn chat_model_filter() {
        assert!(is_chat_model("gpt-4.1-mini"));
        assert!(is_chat_model("o4-mini"));
        assert!(!is_chat_model("text-embedding-3-small"));
        assert!(!is_chat_model("gpt-4o-mini-tts"));
        assert!(!is_chat_model("dall-e-3"));
    }
}
