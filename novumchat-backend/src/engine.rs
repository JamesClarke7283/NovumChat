//! Chat operations and reply generation: streaming, the tool loop with
//! permission checks and approvals, ensembles, synthesis and auto-titles.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use novumchat_core::config::{AppConfig, McpServerConfig};
use novumchat_core::events::AppEvent;
use novumchat_core::permissions::{CallContext, PermissionLevel, ToolRisk, Verdict, decide, tool_offered};
use novumchat_core::rpc::{
    CreateChatParams, ExportChatParams, ExportResult, ImportResult, RegenerateParams, SendMessageParams, SendResult,
    SetLeafParams, SynthesizeParams, UpdateChatParams,
};
use novumchat_core::types::*;
use novumchat_core::util::tree::Tree;
use novumchat_core::util::{clean_title, slugify, title_from_text, truncate_chars};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::attachments;
use crate::crypto::{Codec, FieldCipher, KdfParams, create_vault, rewrap, unwrap_key};
use crate::llm::{ChatRequest, Finish, ImageData, LlmMessage, LlmToolCall, StreamEvent, ToolSpec};
use crate::state::App;
use crate::store::{new_id, now_ms};
use crate::tools::{Assessment, Builtin, ToolEnv, ToolOutput, assess_builtin, run_builtin};

/// Deltas are batched for this long to keep IPC traffic reasonable.
const DELTA_FLUSH: Duration = Duration::from_millis(40);
pub const DEFAULT_TITLE: &str = "New chat";
/// Tool results kept in the message for the UI.
const MAX_STORED_RESULT: usize = 64 * 1024;

pub enum GenKind {
    Reply,
    Synthesis { members: Vec<Message> },
}

/// Batches streamed text into `MessageDelta` events.
struct DeltaBuffer {
    chat_id: Id,
    message_id: Id,
    kind: Option<DeltaKind>,
    text: String,
    last: Instant,
}

impl DeltaBuffer {
    fn new(chat_id: &str, message_id: &str) -> Self {
        Self {
            chat_id: chat_id.into(),
            message_id: message_id.into(),
            kind: None,
            text: String::new(),
            last: Instant::now(),
        }
    }

    fn push(&mut self, events: &crate::events::EventHub, kind: DeltaKind, t: &str) {
        if self.kind != Some(kind) {
            self.flush(events);
            self.kind = Some(kind);
        }
        self.text.push_str(t);
        if self.last.elapsed() >= DELTA_FLUSH {
            self.flush(events);
        }
    }

    fn flush(&mut self, events: &crate::events::EventHub) {
        if let (Some(kind), false) = (self.kind, self.text.is_empty()) {
            events.emit(AppEvent::MessageDelta {
                chat_id: self.chat_id.clone(),
                message_id: self.message_id.clone(),
                kind,
                text: std::mem::take(&mut self.text),
            });
        }
        self.last = Instant::now();
    }
}

fn close_reasoning(msg: &mut Message, started: Instant) {
    if let Some(Part::Reasoning { duration_ms, .. }) =
        msg.parts.iter_mut().rev().find(|p| matches!(p, Part::Reasoning { .. }))
        && duration_ms.is_none()
    {
        *duration_ms = Some(started.elapsed().as_millis() as u64);
    }
}

/// Lenient argument parsing: empty → `{}`, double-encoded strings are unwrapped.
fn parse_args(raw: &str) -> std::result::Result<Value, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::String(inner)) => serde_json::from_str(&inner).map_err(|e| e.to_string()),
        Ok(v) => Ok(v),
        Err(e) => Err(e.to_string()),
    }
}

/// Expands a stored assistant message into model messages (text, tool calls, results).
fn push_assistant(out: &mut Vec<LlmMessage>, m: &Message) {
    let mut text = String::new();
    let mut calls: Vec<(LlmToolCall, String)> = Vec::new();
    fn flush(out: &mut Vec<LlmMessage>, text: &mut String, calls: &mut Vec<(LlmToolCall, String)>) {
        if calls.is_empty() {
            return;
        }
        out.push(LlmMessage::Assistant {
            text: std::mem::take(text),
            tool_calls: calls.iter().map(|(c, _)| c.clone()).collect(),
        });
        for (c, result) in calls.drain(..) {
            out.push(LlmMessage::Tool { call_id: c.id, name: c.name, content: result });
        }
    }
    for part in &m.parts {
        match part {
            Part::Text { text: t } => {
                flush(out, &mut text, &mut calls);
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(t);
            }
            Part::Reasoning { .. } => {}
            Part::ToolCall(tc) => calls.push((
                LlmToolCall { id: tc.id.clone(), name: tc.name.clone(), arguments: tc.arguments.clone() },
                tc.result.clone().unwrap_or_else(|| "(no result)".into()),
            )),
        }
    }
    if !calls.is_empty() {
        flush(out, &mut text, &mut calls);
    } else if !text.trim().is_empty() {
        out.push(LlmMessage::Assistant { text, tool_calls: Vec::new() });
    }
}

fn apply_synthesis(context: &mut [LlmMessage], members: &[Message], instructions: &str) {
    let mut block = String::from("\n\n---\nCandidate answers written by different models:\n");
    for (i, m) in members.iter().enumerate() {
        let model = m.model.as_ref().map(ModelRef::key).unwrap_or_default();
        block.push_str(&format!(
            "\n<answer index=\"{}\" model=\"{}\">\n{}\n</answer>\n",
            i + 1,
            model,
            m.text().trim()
        ));
    }
    block.push_str("\n---\n");
    block.push_str(instructions);
    if let Some(LlmMessage::User { text, .. }) = context.iter_mut().rev().find(|m| matches!(m, LlmMessage::User { .. }))
    {
        text.push_str(&block);
    }
}

impl App {
    fn emit_update(&self, msg: &Message) {
        self.events.emit(AppEvent::MessageUpdated { message: msg.clone() });
    }

    // ── chats ──────────────────────────────────────────────────────────────

    pub fn default_chat_settings(&self, cfg: &AppConfig) -> ChatSettings {
        ChatSettings {
            models: cfg.general.default_model.clone().into_iter().collect(),
            synthesize: cfg.ensemble.synthesize,
            synthesizer: cfg.ensemble.synthesizer.clone(),
            tools: cfg.tools.enabled,
            web_search: false,
            temperature: cfg.general.temperature,
            max_tokens: cfg.general.max_tokens,
            reasoning_effort: cfg.general.reasoning_effort.clone(),
            permission: None,
        }
    }

    pub async fn create_chat(&self, p: CreateChatParams) -> Result<Chat> {
        let cfg = self.config.get();
        let now = now_ms();
        let chat = Chat {
            id: new_id(),
            title: DEFAULT_TITLE.into(),
            system_prompt: p.system_prompt.filter(|s| !s.trim().is_empty()),
            settings: p.settings.unwrap_or_else(|| self.default_chat_settings(&cfg)),
            pinned: false,
            archived: false,
            current_leaf: None,
            created_at: now,
            updated_at: now,
        };
        self.store.insert_chat(&chat).await?;
        self.events.emit(AppEvent::ChatUpdated { chat: chat.clone() });
        Ok(chat)
    }

    pub async fn update_chat(&self, p: UpdateChatParams) -> Result<Chat> {
        let mut chat = self.store.require_chat(&p.id).await?;
        if let Some(t) = p.title {
            let t = t.trim();
            chat.title = if t.is_empty() { DEFAULT_TITLE.into() } else { truncate_chars(t, 120) };
        }
        if let Some(v) = p.pinned {
            chat.pinned = v;
        }
        if let Some(v) = p.archived {
            chat.archived = v;
        }
        if let Some(s) = p.system_prompt {
            chat.system_prompt = (!s.trim().is_empty()).then_some(s);
        }
        if let Some(s) = p.settings {
            chat.settings = s;
        }
        self.store.save_chat(&chat).await?;
        self.events.emit(AppEvent::ChatUpdated { chat: chat.clone() });
        Ok(chat)
    }

    pub async fn delete_chat(&self, id: &str) -> Result<()> {
        self.stop(id, None);
        self.store.delete_chat(id).await?;
        self.approvals.forget_chat(id);
        self.events.emit(AppEvent::ChatDeleted { chat_id: id.to_string() });
        Ok(())
    }

    pub async fn chat_detail(&self, id: &str) -> Result<ChatDetail> {
        let chat = self.store.require_chat(id).await?;
        let messages = self.store.chat_messages(id).await?;
        Ok(ChatDetail { chat, messages })
    }

    pub async fn set_leaf(&self, p: SetLeafParams) -> Result<Chat> {
        let mut chat = self.store.require_chat(&p.chat_id).await?;
        let all = self.store.chat_messages(&chat.id).await?;
        let tree = Tree::new(&all);
        let leaf = tree.deepest_leaf(&p.message_id).ok_or_else(|| anyhow!("Message not found"))?;
        chat.current_leaf = Some(leaf.to_string());
        self.store.set_current_leaf(&chat.id, chat.current_leaf.as_deref()).await?;
        self.events.emit(AppEvent::ChatUpdated { chat: chat.clone() });
        Ok(chat)
    }

    pub async fn delete_message(&self, chat_id: &str, message_id: &str) -> Result<()> {
        let mut chat = self.store.require_chat(chat_id).await?;
        let all = self.store.chat_messages(chat_id).await?;
        let tree = Tree::new(&all);
        let target = tree.get(message_id).ok_or_else(|| anyhow!("Message not found"))?;
        let parent = target.parent_id.clone();
        let ids = tree.descendants(message_id);
        for id in &ids {
            self.stop(chat_id, Some(id));
        }
        self.store.delete_messages(&ids).await?;
        if chat.current_leaf.as_ref().is_none_or(|l| ids.contains(l)) {
            let remaining: Vec<Message> = all.into_iter().filter(|m| !ids.contains(&m.id)).collect();
            let tree = Tree::new(&remaining);
            chat.current_leaf = match parent.as_deref().and_then(|p| tree.deepest_leaf(p)) {
                Some(l) => Some(l.to_string()),
                None => tree.default_leaf().map(String::from),
            };
            self.store.set_current_leaf(chat_id, chat.current_leaf.as_deref()).await?;
        }
        self.events.emit(AppEvent::MessagesDeleted { chat_id: chat_id.into(), ids });
        self.events.emit(AppEvent::ChatUpdated { chat });
        Ok(())
    }

    fn chat_busy(&self, chat_id: &str) -> bool {
        self.generations.lock().values().any(|g| g.chat_id == chat_id)
    }

    async fn resolve_models(&self, chat: &Chat, cfg: &AppConfig) -> Result<Vec<ModelRef>> {
        let mut models: Vec<ModelRef> = Vec::new();
        for m in &chat.settings.models {
            if cfg.provider(&m.provider).is_some_and(|p| p.enabled) && !models.contains(m) {
                models.push(m.clone());
            }
        }
        if !models.is_empty() {
            return Ok(models);
        }
        if let Some(m) = &cfg.general.default_model
            && cfg.provider(&m.provider).is_some_and(|p| p.enabled)
        {
            return Ok(vec![m.clone()]);
        }
        let list = self.list_models(false).await;
        list.models
            .first()
            .map(|m| vec![m.model_ref()])
            .ok_or_else(|| anyhow!("No models available. Start Ollama or add a provider in Settings → Providers."))
    }

    // ── sending ────────────────────────────────────────────────────────────

    pub async fn send_message(self: &Arc<Self>, p: SendMessageParams) -> Result<SendResult> {
        let cfg = self.config.get();
        let mut chat = self.store.require_chat(&p.chat_id).await?;
        if self.chat_busy(&chat.id) {
            bail!("A reply is still being generated. Stop it or wait for it to finish.");
        }
        let text = p.text.trim().to_string();
        if text.is_empty() && p.attachments.is_empty() && p.edit_of.is_none() {
            bail!("Message is empty");
        }
        let models = self.resolve_models(&chat, &cfg).await?;
        let all = self.store.chat_messages(&chat.id).await?;
        let tree = Tree::new(&all);
        let parent_id = match &p.edit_of {
            Some(orig) => tree.get(orig).ok_or_else(|| anyhow!("Edited message not found"))?.parent_id.clone(),
            None => tree.active_path(chat.current_leaf.as_deref()).last().map(|m| m.id.clone()),
        };

        let user_id = new_id();
        let mut metas = Vec::new();
        if let Some(orig) = &p.edit_of {
            for att in self.store.message_attachments(orig).await? {
                let meta = AttachmentMeta { id: new_id(), ..att.meta.clone() };
                self.store.insert_attachment(&chat.id, &user_id, &meta, &att.data, att.text.as_deref()).await?;
                metas.push(meta);
            }
        }
        for upload in p.attachments {
            let prepared = attachments::prepare(upload).await?;
            self.store
                .insert_attachment(&chat.id, &user_id, &prepared.meta, &prepared.data, prepared.text.as_deref())
                .await?;
            metas.push(prepared.meta);
        }
        let user = Message {
            id: user_id,
            chat_id: chat.id.clone(),
            parent_id: parent_id.clone(),
            role: Role::User,
            parts: if text.is_empty() { Vec::new() } else { vec![Part::Text { text: text.clone() }] },
            attachments: metas,
            model: None,
            group_id: None,
            synthesis: false,
            status: MessageStatus::Complete,
            error: None,
            usage: None,
            created_at: now_ms(),
            duration_ms: None,
            ttft_ms: None,
        };
        self.store.insert_message(&user).await?;
        self.events.emit(AppEvent::MessageCreated { message: user.clone() });

        if parent_id.is_none() && (chat.title == DEFAULT_TITLE || p.edit_of.is_some()) {
            chat.title = if text.is_empty() {
                user.attachments.first().map(|a| a.name.clone()).unwrap_or_else(|| DEFAULT_TITLE.into())
            } else {
                title_from_text(&text)
            };
        }
        let replies = self.start_replies(&mut chat, &user, models, &cfg).await?;
        Ok(SendResult { user_message: Some(user), assistant_messages: replies })
    }

    pub async fn regenerate(self: &Arc<Self>, p: RegenerateParams) -> Result<SendResult> {
        let cfg = self.config.get();
        let mut chat = self.store.require_chat(&p.chat_id).await?;
        if self.chat_busy(&chat.id) {
            bail!("A reply is still being generated. Stop it or wait for it to finish.");
        }
        let all = self.store.chat_messages(&chat.id).await?;
        let tree = Tree::new(&all);
        let target = tree.get(&p.message_id).ok_or_else(|| anyhow!("Message not found"))?.clone();
        if target.synthesis
            && let Some(gid) = target.group_id.clone()
        {
            let model = p.models.as_ref().and_then(|m| m.first().cloned()).or(target.model.clone());
            let msg = self.start_synthesis(&chat.id, &gid, model).await?;
            return Ok(SendResult { user_message: None, assistant_messages: vec![msg] });
        }
        let prompt = match target.role {
            Role::User => target.clone(),
            _ => tree.prompt_for(&target).cloned().ok_or_else(|| anyhow!("Prompt for this reply not found"))?,
        };
        let models = match p.models.filter(|m| !m.is_empty()) {
            Some(m) => m,
            None => match (target.role, target.model.clone()) {
                (Role::Assistant, Some(m)) => vec![m],
                _ => self.resolve_models(&chat, &cfg).await?,
            },
        };
        let replies = self.start_replies(&mut chat, &prompt, models, &cfg).await?;
        Ok(SendResult { user_message: None, assistant_messages: replies })
    }

    async fn start_replies(
        self: &Arc<Self>,
        chat: &mut Chat,
        prompt: &Message,
        models: Vec<ModelRef>,
        cfg: &AppConfig,
    ) -> Result<Vec<Message>> {
        let _ = cfg;
        let group_id = (models.len() > 1).then(new_id);
        let base = now_ms();
        let mut created = Vec::with_capacity(models.len());
        for (i, model) in models.into_iter().enumerate() {
            let m = Message {
                id: new_id(),
                chat_id: chat.id.clone(),
                parent_id: Some(prompt.id.clone()),
                role: Role::Assistant,
                parts: Vec::new(),
                attachments: Vec::new(),
                model: Some(model),
                group_id: group_id.clone(),
                synthesis: false,
                status: MessageStatus::Pending,
                error: None,
                usage: None,
                created_at: base + i as i64,
                duration_ms: None,
                ttft_ms: None,
            };
            self.store.insert_message(&m).await?;
            self.events.emit(AppEvent::MessageCreated { message: m.clone() });
            created.push(m);
        }
        chat.current_leaf = created.first().map(|m| m.id.clone());
        chat.updated_at = now_ms();
        self.store.save_chat(chat).await?;
        self.events.emit(AppEvent::ChatUpdated { chat: chat.clone() });

        let mut handles = Vec::new();
        for m in &created {
            let cancel = self.register_generation(&m.id, &chat.id);
            let app = self.clone();
            let chat_c = chat.clone();
            let msg = m.clone();
            handles.push(tokio::spawn(async move { app.generate(chat_c, msg, GenKind::Reply, cancel).await }));
        }
        if let (Some(gid), true) = (group_id, chat.settings.synthesize) {
            let key = format!("group:{gid}");
            let cancel = self.register_generation(&key, &chat.id);
            let app = self.clone();
            let chat_id = chat.id.clone();
            let synthesizer = chat.settings.synthesizer.clone();
            tokio::spawn(async move {
                let results = futures::future::join_all(handles).await;
                app.finish_generation(&key);
                let any_ok = results.iter().any(|r| r.as_ref().is_ok_and(|m| m.status == MessageStatus::Complete));
                if any_ok
                    && !cancel.is_cancelled()
                    && let Err(e) = app.start_synthesis(&chat_id, &gid, synthesizer).await
                {
                    app.events.toast(ToastLevel::Error, format!("Synthesis failed: {e}"));
                }
            });
        }
        Ok(created)
    }

    pub async fn synthesize(self: &Arc<Self>, p: SynthesizeParams) -> Result<Message> {
        if self.chat_busy(&p.chat_id) {
            bail!("A reply is still being generated. Stop it or wait for it to finish.");
        }
        self.start_synthesis(&p.chat_id, &p.group_id, p.model).await
    }

    async fn start_synthesis(
        self: &Arc<Self>,
        chat_id: &str,
        group_id: &str,
        model: Option<ModelRef>,
    ) -> Result<Message> {
        let cfg = self.config.get();
        let mut chat = self.store.require_chat(chat_id).await?;
        let all = self.store.chat_messages(chat_id).await?;
        let members: Vec<Message> = all
            .iter()
            .filter(|m| {
                m.group_id.as_deref() == Some(group_id)
                    && !m.synthesis
                    && m.status == MessageStatus::Complete
                    && !m.text().trim().is_empty()
            })
            .cloned()
            .collect();
        let first = members.first().ok_or_else(|| anyhow!("No completed responses to synthesize"))?;
        let model = model
            .or_else(|| chat.settings.synthesizer.clone())
            .or_else(|| cfg.ensemble.synthesizer.clone())
            .or_else(|| first.model.clone())
            .ok_or_else(|| anyhow!("No synthesizer model"))?;
        let msg = Message {
            id: new_id(),
            chat_id: chat_id.into(),
            parent_id: first.parent_id.clone(),
            role: Role::Assistant,
            parts: Vec::new(),
            attachments: Vec::new(),
            model: Some(model),
            group_id: Some(group_id.into()),
            synthesis: true,
            status: MessageStatus::Pending,
            error: None,
            usage: None,
            created_at: now_ms(),
            duration_ms: None,
            ttft_ms: None,
        };
        self.store.insert_message(&msg).await?;
        self.events.emit(AppEvent::MessageCreated { message: msg.clone() });
        chat.current_leaf = Some(msg.id.clone());
        self.store.set_current_leaf(chat_id, chat.current_leaf.as_deref()).await?;
        self.events.emit(AppEvent::ChatUpdated { chat: chat.clone() });
        let cancel = self.register_generation(&msg.id, chat_id);
        let app = self.clone();
        let m = msg.clone();
        tokio::spawn(async move { app.generate(chat, m, GenKind::Synthesis { members }, cancel).await });
        Ok(msg)
    }

    // ── generation ─────────────────────────────────────────────────────────

    async fn generate(
        self: Arc<Self>,
        chat: Chat,
        mut msg: Message,
        kind: GenKind,
        cancel: CancellationToken,
    ) -> Message {
        let is_reply = matches!(kind, GenKind::Reply);
        if let Err(e) = self.generate_inner(&chat, &mut msg, &kind, &cancel).await {
            if cancel.is_cancelled() {
                msg.status = MessageStatus::Cancelled;
            } else {
                tracing::warn!("generation failed: {e:#}");
                msg.status = MessageStatus::Error;
                msg.error = Some(format!("{e:#}"));
            }
        }
        if msg.status.is_active() {
            msg.status = if cancel.is_cancelled() { MessageStatus::Cancelled } else { MessageStatus::Complete };
        }
        for part in msg.parts.iter_mut() {
            if let Part::ToolCall(tc) = part
                && !tc.status.is_finished()
            {
                tc.status = ToolCallStatus::Error;
                tc.result.get_or_insert_with(|| "Interrupted".into());
            }
        }
        if let Err(e) = self.store.save_message(&msg).await {
            tracing::error!("saving reply failed: {e:#}");
        }
        self.emit_update(&msg);
        self.finish_generation(&msg.id);
        if is_reply && msg.status == MessageStatus::Complete {
            self.maybe_auto_title(&chat, &msg).await;
        }
        msg
    }

    async fn generate_inner(
        &self,
        chat: &Chat,
        msg: &mut Message,
        kind: &GenKind,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let cfg = self.config.get();
        let model = msg.model.clone().ok_or_else(|| anyhow!("No model selected"))?;
        let provider = self.provider(&model.provider)?;
        let all = self.store.chat_messages(&chat.id).await?;
        let parent_id = msg.parent_id.clone().ok_or_else(|| anyhow!("Reply has no prompt"))?;
        let specs = if matches!(kind, GenKind::Reply) { self.tool_specs(&cfg, &chat.settings) } else { Vec::new() };
        let has_tools = specs.iter().any(|s| !matches!(s.name.as_str(), "web_search" | "fetch_url"));
        let has_web = specs.iter().any(|s| s.name == "web_search");
        let mut context = self.build_context(chat, &all, &parent_id, &cfg, has_tools, has_web).await?;
        if let GenKind::Synthesis { members } = kind {
            apply_synthesis(&mut context, members, &cfg.ensemble.synthesis_prompt);
        }

        let started = Instant::now();
        msg.status = MessageStatus::Streaming;
        self.emit_update(msg);
        let max_rounds = cfg.general.max_tool_rounds.max(1);
        let mut usage = Usage::default();
        let mut round = 0u32;
        loop {
            round += 1;
            let req = ChatRequest {
                model: model.model.clone(),
                messages: context.clone(),
                tools: specs.clone(),
                temperature: chat.settings.temperature,
                max_tokens: chat.settings.max_tokens,
                reasoning_effort: chat.settings.reasoning_effort.clone(),
            };
            let mut round_text = String::new();
            let mut calls: Vec<LlmToolCall> = Vec::new();
            let mut buf = DeltaBuffer::new(&chat.id, &msg.id);
            let mut reasoning_start: Option<Instant> = None;
            let events = self.events.clone();
            let finish = {
                let m = &mut *msg;
                let mut on_event = |ev: StreamEvent| match ev {
                    StreamEvent::Text(t) => {
                        if m.ttft_ms.is_none() {
                            m.ttft_ms = Some(started.elapsed().as_millis() as u64);
                        }
                        if let Some(rs) = reasoning_start.take() {
                            close_reasoning(m, rs);
                        }
                        m.push_delta(DeltaKind::Text, &t);
                        round_text.push_str(&t);
                        buf.push(&events, DeltaKind::Text, &t);
                    }
                    StreamEvent::Reasoning(r) => {
                        if m.ttft_ms.is_none() {
                            m.ttft_ms = Some(started.elapsed().as_millis() as u64);
                        }
                        reasoning_start.get_or_insert_with(Instant::now);
                        m.push_delta(DeltaKind::Reasoning, &r);
                        buf.push(&events, DeltaKind::Reasoning, &r);
                    }
                    StreamEvent::ToolCalls(c) => calls = c,
                    StreamEvent::Usage(u) => usage.add(u),
                    StreamEvent::Notice(n) => events.toast(ToastLevel::Warning, n),
                };
                provider.stream_chat(&req, cancel, &mut on_event).await
            };
            buf.flush(&self.events);
            if let Some(rs) = reasoning_start.take() {
                close_reasoning(msg, rs);
            }
            if usage != Usage::default() {
                msg.usage = Some(usage);
            }
            let finish = finish?;
            if finish == Finish::Cancelled || cancel.is_cancelled() {
                msg.status = MessageStatus::Cancelled;
                break;
            }
            if calls.is_empty() {
                break;
            }
            if round >= max_rounds {
                let note =
                    format!("\n\n_Stopped after {max_rounds} tool rounds (Settings → General → max tool rounds)._");
                msg.push_delta(DeltaKind::Text, &note);
                break;
            }
            let calls = self.uniquify_calls(msg, calls);
            for c in &calls {
                msg.parts.push(Part::ToolCall(ToolCallPart {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    arguments: c.arguments.clone(),
                    status: ToolCallStatus::Pending,
                    risk: Some(self.tool_risk(&c.name)),
                    ..Default::default()
                }));
            }
            self.emit_update(msg);
            let _ = self.store.save_message(msg).await;
            context.push(LlmMessage::Assistant { text: round_text, tool_calls: calls.clone() });
            for c in &calls {
                if cancel.is_cancelled() {
                    break;
                }
                let out = self.execute_tool(chat, msg, c, &cfg, cancel).await;
                context.push(LlmMessage::Tool { call_id: c.id.clone(), name: c.name.clone(), content: out.text });
            }
            if cancel.is_cancelled() {
                msg.status = MessageStatus::Cancelled;
                break;
            }
            let _ = self.store.save_message(msg).await;
        }
        msg.duration_ms = Some(started.elapsed().as_millis() as u64);
        if msg.status.is_active() {
            msg.status = MessageStatus::Complete;
        }
        Ok(())
    }

    /// Providers sometimes reuse call ids across rounds; ids must be unique per message.
    fn uniquify_calls(&self, msg: &Message, calls: Vec<LlmToolCall>) -> Vec<LlmToolCall> {
        let mut seen: std::collections::HashSet<String> = msg
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::ToolCall(tc) => Some(tc.id.clone()),
                _ => None,
            })
            .collect();
        calls
            .into_iter()
            .map(|mut c| {
                if c.id.is_empty() || seen.contains(&c.id) {
                    c.id = format!("call_{}", uuid::Uuid::new_v4().simple());
                }
                seen.insert(c.id.clone());
                c
            })
            .collect()
    }

    fn tool_risk(&self, name: &str) -> ToolRisk {
        Builtin::from_name(name).map(Builtin::risk).unwrap_or(ToolRisk::External)
    }

    fn permission_level(&self, cfg: &AppConfig, settings: &ChatSettings) -> PermissionLevel {
        settings.permission.unwrap_or(cfg.tools.permission)
    }

    fn builtin_enabled(&self, b: Builtin, cfg: &AppConfig, settings: &ChatSettings) -> bool {
        let level = self.permission_level(cfg, settings);
        if cfg.tools.disabled_tools.iter().any(|d| d == b.name()) || !tool_offered(level, b.risk()) {
            return false;
        }
        if b.is_web() {
            settings.web_search
        } else {
            cfg.tools.enabled && settings.tools && (b != Builtin::Bash || crate::tools::shell::shell_available())
        }
    }

    pub fn tool_specs(&self, cfg: &AppConfig, settings: &ChatSettings) -> Vec<ToolSpec> {
        let mut specs: Vec<ToolSpec> =
            Builtin::ALL.into_iter().filter(|b| self.builtin_enabled(*b, cfg, settings)).map(Builtin::spec).collect();
        if cfg.tools.enabled && settings.tools {
            let level = self.permission_level(cfg, settings);
            for t in self.mcp.tools() {
                if level == PermissionLevel::ReadOnly && !t.read_only {
                    continue;
                }
                if let Some(spec) = self.mcp.specs().into_iter().find(|s| s.name == t.qualified) {
                    specs.push(spec);
                }
            }
        }
        specs
    }

    pub fn list_tools(&self) -> Vec<ToolDescriptor> {
        let cfg = self.config.get();
        let level = cfg.tools.permission;
        let mut out: Vec<ToolDescriptor> = Builtin::ALL
            .into_iter()
            .map(|b| ToolDescriptor {
                name: b.name().into(),
                description: b.description().into(),
                risk: b.risk(),
                source: "builtin".into(),
                enabled: !cfg.tools.disabled_tools.iter().any(|d| d == b.name())
                    && tool_offered(level, b.risk())
                    && (b != Builtin::Bash || crate::tools::shell::shell_available()),
            })
            .collect();
        for t in self.mcp.tools() {
            out.push(ToolDescriptor {
                name: t.qualified.clone(),
                description: t.description.clone(),
                risk: ToolRisk::External,
                source: t.server.clone(),
                enabled: level != PermissionLevel::ReadOnly || t.read_only,
            });
        }
        out
    }

    fn set_call_status(msg: &mut Message, id: &str, status: ToolCallStatus) {
        if let Some(tc) = msg.tool_call_mut(id) {
            tc.status = status;
        }
    }

    fn finish_call(&self, msg: &mut Message, id: &str, status: ToolCallStatus, out: &ToolOutput, started: Instant) {
        if let Some(tc) = msg.tool_call_mut(id) {
            tc.status = status;
            tc.result = Some(truncate_chars(&out.text, MAX_STORED_RESULT));
            tc.data = out.data.clone();
            tc.duration_ms = Some(started.elapsed().as_millis() as u64);
        }
        self.emit_update(msg);
    }

    async fn execute_tool(
        &self,
        chat: &Chat,
        msg: &mut Message,
        call: &LlmToolCall,
        cfg: &AppConfig,
        cancel: &CancellationToken,
    ) -> ToolOutput {
        let mut started = Instant::now();
        let level = self.permission_level(cfg, &chat.settings);
        let workspace = self.workspace(cfg);
        let args = match parse_args(&call.arguments) {
            Ok(v) => v,
            Err(e) => {
                let out =
                    ToolOutput::error(format!("Invalid JSON arguments ({e}). Call the tool again with valid JSON."));
                self.finish_call(msg, &call.id, ToolCallStatus::Error, &out, started);
                return out;
            }
        };
        enum Target {
            Builtin(Builtin),
            Mcp,
        }
        let builtin = Builtin::from_name(&call.name).filter(|b| self.builtin_enabled(*b, cfg, &chat.settings));
        let mcp: Option<(crate::mcp::McpTool, McpServerConfig)> =
            if builtin.is_none() { self.mcp.find_tool(&call.name) } else { None };
        let (target, mut assessment) = match (builtin, mcp) {
            (Some(b), _) => (Target::Builtin(b), assess_builtin(b, &args, &workspace, &cfg.tools)),
            (None, Some((tool, server_cfg))) => {
                let mut ctx = CallContext::new(ToolRisk::External);
                ctx.trusted_server = server_cfg.trusted;
                ctx.read_only_hint = tool.read_only;
                let detail = serde_json::to_string_pretty(&args).unwrap_or_else(|_| call.arguments.clone());
                (Target::Mcp, Assessment { ctx, title: format!("Use `{}` from {}", tool.tool, tool.server), detail })
            }
            (None, None) => {
                let out = ToolOutput::error(format!("Unknown or disabled tool `{}`.", call.name));
                self.finish_call(msg, &call.id, ToolCallStatus::Error, &out, started);
                return out;
            }
        };
        assessment.ctx.granted_for_chat = self.approvals.is_granted(&chat.id, &call.name);
        match decide(level, assessment.ctx) {
            Verdict::Deny => {
                let why = if assessment.ctx.denylisted {
                    "it matches a denied command pattern".to_string()
                } else {
                    format!("the permission level is \"{}\"", level.label())
                };
                let out =
                    ToolOutput::error(format!("Permission denied: `{}` is not allowed because {why}.", call.name));
                self.finish_call(msg, &call.id, ToolCallStatus::Denied, &out, started);
                return out;
            }
            Verdict::Ask => {
                Self::set_call_status(msg, &call.id, ToolCallStatus::AwaitingApproval);
                self.emit_update(msg);
                let _ = self.store.save_message(msg).await;
                let request = ApprovalRequest {
                    id: new_id(),
                    chat_id: chat.id.clone(),
                    message_id: msg.id.clone(),
                    tool_call_id: call.id.clone(),
                    tool: call.name.clone(),
                    title: assessment.title.clone(),
                    detail: truncate_chars(&assessment.detail, 20_000),
                    risk: assessment.ctx.risk,
                    created_at: now_ms(),
                };
                if self.approvals.request(&self.events, request, cancel).await == ApprovalDecision::Deny {
                    let out = ToolOutput::error(
                        "The user declined this tool call. Do not retry it; continue without it or ask the user how to proceed.",
                    );
                    self.finish_call(msg, &call.id, ToolCallStatus::Denied, &out, started);
                    return out;
                }
                // Report execution time, not time spent waiting for the user.
                started = Instant::now();
            }
            Verdict::Allow => {}
        }
        Self::set_call_status(msg, &call.id, ToolCallStatus::Running);
        self.emit_update(msg);
        let out = match target {
            Target::Builtin(b) => {
                let env = ToolEnv { workspace: &workspace, tools: &cfg.tools, search: &cfg.search, cancel };
                run_builtin(b, &args, &env).await
            }
            Target::Mcp => self.mcp.call(&call.name, &args, cancel).await,
        };
        let status = if out.is_error { ToolCallStatus::Error } else { ToolCallStatus::Success };
        self.finish_call(msg, &call.id, status, &out, started);
        out
    }

    fn system_prompt(&self, chat: &Chat, cfg: &AppConfig, tools: bool, web: bool) -> String {
        let base = chat
            .system_prompt
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| cfg.general.system_prompt.clone());
        let date = chrono::Local::now().format("%A, %B %-d, %Y").to_string();
        let mut s = base.replace("{date}", &date);
        if tools {
            s.push_str(&format!(
                "\n\nYou can use tools to read and edit files and run shell commands on the user's {} computer. \
The workspace directory is `{}`; prefer paths relative to it. Briefly say what you are about to do before \
changing anything, and check results after running commands.",
                std::env::consts::OS,
                self.workspace(cfg).display()
            ));
        }
        if web {
            s.push_str(
                "\n\nYou can search the internet with `web_search` and read pages with `fetch_url`. Search when the \
answer depends on recent or specific facts, and cite sources inline as [n](url).",
            );
        }
        s.trim().to_string()
    }

    async fn build_context(
        &self,
        chat: &Chat,
        all: &[Message],
        parent_id: &str,
        cfg: &AppConfig,
        tools: bool,
        web: bool,
    ) -> Result<Vec<LlmMessage>> {
        let tree = Tree::new(all);
        let path = tree.path_to(parent_id);
        if path.is_empty() {
            bail!("Conversation history not found");
        }
        let mut out = Vec::with_capacity(path.len() + 1);
        let sys = self.system_prompt(chat, cfg, tools, web);
        if !sys.is_empty() {
            out.push(LlmMessage::System(sys));
        }
        for m in path {
            match m.role {
                Role::User => {
                    let mut text = m.text();
                    let mut images = Vec::new();
                    if !m.attachments.is_empty() {
                        for att in self.store.message_attachments(&m.id).await? {
                            if att.meta.kind == AttachmentKind::Image {
                                images.push(ImageData { mime: att.meta.mime.clone(), base64: B64.encode(&att.data) });
                            } else {
                                if !text.is_empty() {
                                    text.push_str("\n\n");
                                }
                                text.push_str(&attachments::inline_text(&att));
                            }
                        }
                    }
                    if text.is_empty() && !images.is_empty() {
                        text = "(image attached)".into();
                    }
                    out.push(LlmMessage::User { text, images });
                }
                Role::Assistant => push_assistant(&mut out, m),
                Role::System => out.push(LlmMessage::System(m.text())),
            }
        }
        Ok(out)
    }

    // ── titles ─────────────────────────────────────────────────────────────

    async fn maybe_auto_title(self: &Arc<Self>, chat: &Chat, reply: &Message) {
        let cfg = self.config.get();
        if !cfg.general.auto_title {
            return;
        }
        let Ok(all) = self.store.chat_messages(&chat.id).await else { return };
        let tree = Tree::new(&all);
        let path = tree.path_to(&reply.id);
        if path.len() != 2 || path[0].role != Role::User {
            return;
        }
        let user_text = path[0].text();
        let Ok(Some(current)) = self.store.get_chat(&chat.id).await else { return };
        let derived = if user_text.is_empty() { None } else { Some(title_from_text(&user_text)) };
        let untouched = current.title == DEFAULT_TITLE
            || derived.as_deref() == Some(current.title.as_str())
            || path[0].attachments.first().is_some_and(|a| a.name == current.title);
        if !untouched || !self.titling.lock().insert(chat.id.clone()) {
            return;
        }
        let Some(model) = cfg.general.title_model.clone().or_else(|| reply.model.clone()) else { return };
        let app = self.clone();
        let chat_id = chat.id.clone();
        let reply_text = reply.text();
        tokio::spawn(async move {
            match app.generate_title(&model, &user_text, &reply_text).await {
                Ok(title) => {
                    if let Ok(Some(mut c)) = app.store.get_chat(&chat_id).await {
                        c.title = title;
                        if app.store.save_chat(&c).await.is_ok() {
                            app.events.emit(AppEvent::ChatUpdated { chat: c });
                        }
                    }
                }
                Err(e) => tracing::debug!("title generation failed: {e:#}"),
            }
            app.titling.lock().remove(&chat_id);
        });
    }

    async fn generate_title(&self, model: &ModelRef, user: &str, assistant: &str) -> Result<String> {
        let provider = self.provider(&model.provider)?;
        let prompt = format!(
            "Write a short, specific title (2-6 words) for this conversation. Reply with the title only — no quotes, \
no punctuation at the end, no explanation.\n\nUser: {}\n\nAssistant: {}",
            truncate_chars(user, 1500),
            truncate_chars(assistant, 1500)
        );
        let req = ChatRequest {
            model: model.model.clone(),
            messages: vec![LlmMessage::User { text: prompt, images: Vec::new() }],
            temperature: Some(0.2),
            ..Default::default()
        };
        let cancel = CancellationToken::new();
        let mut text = String::new();
        let mut on_event = |ev: StreamEvent| {
            if let StreamEvent::Text(t) = ev {
                text.push_str(&t);
                if text.len() > 160 {
                    cancel.cancel();
                }
            }
        };
        let fut = provider.stream_chat(&req, &cancel, &mut on_event);
        let _ = tokio::time::timeout(Duration::from_secs(90), fut).await;
        let title = clean_title(&text);
        if title.is_empty() {
            bail!("empty title");
        }
        Ok(title)
    }

    // ── export / import ────────────────────────────────────────────────────

    pub async fn export_chat(&self, p: ExportChatParams) -> Result<ExportResult> {
        let detail = self.chat_detail(&p.id).await?;
        let base = {
            let s = slugify(&detail.chat.title);
            if s.is_empty() { "chat".to_string() } else { s }
        };
        if p.format == "json" {
            return Ok(ExportResult {
                filename: format!("{base}.json"),
                content: serde_json::to_string_pretty(&detail)?,
            });
        }
        let tree = Tree::new(&detail.messages);
        let mut md = format!("# {}\n\n", detail.chat.title);
        for m in tree.active_path(detail.chat.current_leaf.as_deref()) {
            match m.role {
                Role::User => md.push_str("## You\n\n"),
                _ => md.push_str(&format!(
                    "## {}{}\n\n",
                    m.model.as_ref().map(ModelRef::key).unwrap_or_else(|| "Assistant".into()),
                    if m.synthesis { " (synthesis)" } else { "" }
                )),
            }
            for a in &m.attachments {
                md.push_str(&format!("> 📎 {} ({})\n\n", a.name, novumchat_core::util::human_bytes(a.size)));
            }
            for part in &m.parts {
                match part {
                    Part::Text { text } => {
                        md.push_str(text.trim());
                        md.push_str("\n\n");
                    }
                    Part::ToolCall(tc) => md.push_str(&format!("> 🔧 `{}` — {:?}\n\n", tc.name, tc.status)),
                    Part::Reasoning { .. } => {}
                }
            }
        }
        Ok(ExportResult { filename: format!("{base}.md"), content: md })
    }

    pub fn import_codex_mcp(self: &Arc<Self>) -> Result<ImportResult> {
        let home = std::env::var_os("CODEX_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join(".codex")))
            .ok_or_else(|| anyhow!("home directory not found"))?;
        let path = home.join("config.toml");
        let text = std::fs::read_to_string(&path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
        let table: toml::Table = toml::from_str(&text).map_err(|e| anyhow!("{}: {e}", path.display()))?;
        let mut result = ImportResult::default();
        let Some(servers) = table.get("mcp_servers").and_then(toml::Value::as_table) else {
            return Ok(result);
        };
        let mut cfg = self.config.get();
        for (name, value) in servers {
            if cfg.mcp_servers.contains_key(name) {
                result.skipped.push(name.clone());
                continue;
            }
            match value.clone().try_into::<McpServerConfig>() {
                Ok(sc) => {
                    cfg.mcp_servers.insert(name.clone(), sc);
                    result.imported.push(name.clone());
                }
                Err(_) => result.skipped.push(name.clone()),
            }
        }
        if !result.imported.is_empty() {
            self.save_config(cfg)?;
        }
        Ok(result)
    }

    pub fn save_config(self: &Arc<Self>, cfg: AppConfig) -> Result<AppConfig> {
        let saved = self.config.save(cfg)?;
        self.after_config_change(&saved);
        Ok(saved)
    }

    pub fn save_config_toml(self: &Arc<Self>, text: &str) -> Result<AppConfig> {
        let saved = self.config.save_raw(text)?;
        self.after_config_change(&saved);
        Ok(saved)
    }

    fn after_config_change(self: &Arc<Self>, cfg: &AppConfig) {
        self.rebuild_providers();
        self.mcp.apply_config(&cfg.mcp_servers);
        self.events.emit(AppEvent::ConfigChanged);
    }

    // ── vault ──────────────────────────────────────────────────────────────

    fn emit_vault(&self) -> VaultStatus {
        let status = self.vault.status();
        self.events.emit(AppEvent::VaultChanged { status });
        status
    }

    pub async fn vault_unlock(&self, passphrase: String) -> Result<VaultStatus> {
        let header = self.vault.header().ok_or_else(|| anyhow!("The database is not encrypted"))?;
        let key = tokio::task::spawn_blocking(move || unwrap_key(&header, &passphrase)).await??;
        self.vault.set_unlocked(key);
        Ok(self.emit_vault())
    }

    pub fn vault_lock(&self) -> Result<VaultStatus> {
        if !self.vault.status().encrypted {
            bail!("Enable encryption first");
        }
        for g in self.generations.lock().values() {
            g.cancel.cancel();
        }
        self.vault.lock();
        Ok(self.emit_vault())
    }

    pub async fn vault_enable(&self, passphrase: String) -> Result<VaultStatus> {
        if self.vault.status().encrypted {
            bail!("The database is already encrypted");
        }
        if self.active_generations() > 0 {
            bail!("Stop running replies before enabling encryption");
        }
        let (header, key) =
            tokio::task::spawn_blocking(move || create_vault(&passphrase, KdfParams::default())).await??;
        let to = Codec(Some(FieldCipher::new(key.clone())));
        self.store.reencode_all(&Codec::plain(), &to, Some(&header)).await?;
        self.vault.install(Some(header), Some(key));
        if let Err(e) = crate::db::compact(&self.store.db).await {
            tracing::warn!("compacting database failed: {e:#}");
        }
        Ok(self.emit_vault())
    }

    pub async fn vault_disable(&self, passphrase: String) -> Result<VaultStatus> {
        let header = self.vault.header().ok_or_else(|| anyhow!("The database is not encrypted"))?;
        if self.active_generations() > 0 {
            bail!("Stop running replies before disabling encryption");
        }
        let key = tokio::task::spawn_blocking(move || unwrap_key(&header, &passphrase)).await??;
        let from = Codec(Some(FieldCipher::new(key)));
        self.store.reencode_all(&from, &Codec::plain(), None).await?;
        self.vault.install(None, None);
        if let Err(e) = crate::db::compact(&self.store.db).await {
            tracing::warn!("compacting database failed: {e:#}");
        }
        Ok(self.emit_vault())
    }

    pub async fn vault_change(&self, old: String, new: String) -> Result<VaultStatus> {
        let header = self.vault.header().ok_or_else(|| anyhow!("The database is not encrypted"))?;
        let h2 = header.clone();
        let (new_header, key) = tokio::task::spawn_blocking(move || -> Result<_> {
            let key = unwrap_key(&h2, &old)?;
            Ok((rewrap(&h2, &old, &new)?, key))
        })
        .await??;
        self.store.kv_set(crate::crypto::VaultHeader::KEY, &serde_json::to_string(&new_header)?).await?;
        self.vault.install(Some(new_header), Some(key));
        Ok(self.emit_vault())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_part(id: &str, name: &str, result: &str) -> Part {
        Part::ToolCall(ToolCallPart {
            id: id.into(),
            name: name.into(),
            arguments: "{}".into(),
            status: ToolCallStatus::Success,
            result: Some(result.into()),
            ..Default::default()
        })
    }

    #[test]
    fn assistant_history_expansion() {
        let m = Message {
            id: "a".into(),
            chat_id: "c".into(),
            parent_id: Some("u".into()),
            role: Role::Assistant,
            parts: vec![
                Part::Reasoning { text: "hmm".into(), duration_ms: None },
                Part::Text { text: "Let me look.".into() },
                tool_part("1", "list_dir", "a.txt"),
                tool_part("2", "read_file", "hello"),
                Part::Text { text: "It says hello.".into() },
            ],
            attachments: vec![],
            model: None,
            group_id: None,
            synthesis: false,
            status: MessageStatus::Complete,
            error: None,
            usage: None,
            created_at: 0,
            duration_ms: None,
            ttft_ms: None,
        };
        let mut out = Vec::new();
        push_assistant(&mut out, &m);
        assert_eq!(out.len(), 4, "{out:?}");
        match &out[0] {
            LlmMessage::Assistant { text, tool_calls } => {
                assert_eq!(text, "Let me look.");
                assert_eq!(tool_calls.len(), 2);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&out[1], LlmMessage::Tool { content, .. } if content == "a.txt"));
        assert!(
            matches!(&out[3], LlmMessage::Assistant { text, tool_calls } if text == "It says hello." && tool_calls.is_empty())
        );
    }

    #[test]
    fn lenient_args() {
        assert_eq!(parse_args("").unwrap(), serde_json::json!({}));
        assert_eq!(parse_args("{\"a\":1}").unwrap()["a"], 1);
        assert_eq!(parse_args("\"{\\\"a\\\":2}\"").unwrap()["a"], 2);
        assert!(parse_args("{nope").is_err());
    }

    #[test]
    fn synthesis_prompt_is_appended() {
        let mut ctx = vec![LlmMessage::System("s".into()), LlmMessage::User { text: "Q?".into(), images: vec![] }];
        let member = Message {
            id: "a".into(),
            chat_id: "c".into(),
            parent_id: None,
            role: Role::Assistant,
            parts: vec![Part::Text { text: "Answer A".into() }],
            attachments: vec![],
            model: Some(ModelRef::new("p", "m")),
            group_id: Some("g".into()),
            synthesis: false,
            status: MessageStatus::Complete,
            error: None,
            usage: None,
            created_at: 0,
            duration_ms: None,
            ttft_ms: None,
        };
        apply_synthesis(&mut ctx, &[member], "Merge them.");
        match &ctx[1] {
            LlmMessage::User { text, .. } => {
                assert!(text.starts_with("Q?"));
                assert!(text.contains("<answer index=\"1\" model=\"p/m\">\nAnswer A\n</answer>"));
                assert!(text.ends_with("Merge them."));
            }
            _ => panic!(),
        }
    }
}
