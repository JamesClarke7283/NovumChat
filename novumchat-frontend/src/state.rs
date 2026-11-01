//! Application state, backend events and user actions.

use std::collections::HashMap;

use leptos::prelude::*;
use leptos::task::spawn_local;
use novumchat_core::config::{AppConfig, ThemeMode};
use novumchat_core::events::AppEvent;
use novumchat_core::permissions::PermissionLevel;
use novumchat_core::rpc::{SendMessageParams, UpdateChatParams};
use novumchat_core::types::*;
use novumchat_core::util::tree::Tree;
use wasm_bindgen::prelude::*;

use crate::{api, bridge};

#[derive(Clone, Debug, PartialEq)]
pub enum Boot {
    Loading,
    Ready,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub level: ToastLevel,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Appearance,
    Providers,
    Ensemble,
    Tools,
    Search,
    Mcp,
    Security,
    Advanced,
    About,
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 10] = [
        SettingsTab::General,
        SettingsTab::Appearance,
        SettingsTab::Providers,
        SettingsTab::Ensemble,
        SettingsTab::Tools,
        SettingsTab::Search,
        SettingsTab::Mcp,
        SettingsTab::Security,
        SettingsTab::Advanced,
        SettingsTab::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::General => "General",
            SettingsTab::Appearance => "Appearance",
            SettingsTab::Providers => "Providers",
            SettingsTab::Ensemble => "Ensemble",
            SettingsTab::Tools => "Tools & permissions",
            SettingsTab::Search => "Web search",
            SettingsTab::Mcp => "MCP servers",
            SettingsTab::Security => "Privacy & encryption",
            SettingsTab::Advanced => "config.toml",
            SettingsTab::About => "About",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            SettingsTab::General => "sliders",
            SettingsTab::Appearance => "palette",
            SettingsTab::Providers => "cpu",
            SettingsTab::Ensemble => "layers",
            SettingsTab::Tools => "tool",
            SettingsTab::Search => "globe",
            SettingsTab::Mcp => "server",
            SettingsTab::Security => "shield",
            SettingsTab::Advanced => "code",
            SettingsTab::About => "info",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fullscreen {
    pub code: String,
    pub kind: String,
    pub title: String,
    pub network: bool,
}

#[derive(Clone)]
pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
    pub confirm: String,
    pub danger: bool,
    pub on_confirm: Callback<()>,
}

/// A file picked in the composer, not yet sent.
#[derive(Clone)]
pub struct PendingAttachment {
    pub id: u64,
    pub file: web_sys::File,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub preview: Option<String>,
}

/// One displayed row of the conversation.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Msg(Id),
    Group { group: Id, members: Vec<Id>, active: Id },
}

impl Row {
    pub fn key(&self) -> String {
        match self {
            Row::Msg(id) => id.clone(),
            Row::Group { group, .. } => format!("g:{group}"),
        }
    }
}

#[derive(Clone, Copy)]
pub struct AppState {
    pub boot: RwSignal<Boot>,
    pub info: RwSignal<Option<AppInfo>>,
    pub config: RwSignal<Option<AppConfig>>,
    pub vault: RwSignal<VaultStatus>,
    pub chats: RwSignal<Vec<ChatSummary>>,
    pub search: RwSignal<String>,
    pub search_results: RwSignal<Option<Vec<ChatSummary>>>,
    pub current: RwSignal<Option<Id>>,
    pub chat: RwSignal<Option<Chat>>,
    pub messages: RwSignal<HashMap<Id, ArcRwSignal<Message>>>,
    pub structure: RwSignal<u64>,
    pub tick: RwSignal<u64>,
    pub models: RwSignal<ModelsResponse>,
    pub models_loading: RwSignal<bool>,
    pub approvals: RwSignal<Vec<ApprovalRequest>>,
    pub mcp: RwSignal<Vec<McpServerStatus>>,
    pub toasts: RwSignal<Vec<Toast>>,
    pub drawer: RwSignal<bool>,
    pub sidebar_hidden: RwSignal<bool>,
    pub settings: RwSignal<Option<SettingsTab>>,
    pub dark: RwSignal<bool>,
    pub system_dark: RwSignal<bool>,
    pub fullscreen: RwSignal<Option<Fullscreen>>,
    pub lightbox: RwSignal<Option<String>>,
    pub confirm: RwSignal<Option<ConfirmDialog>>,
    pub loading_chat: RwSignal<bool>,
    pub draft_text: RwSignal<String>,
    pub draft_files: RwSignal<Vec<PendingAttachment>, LocalStorage>,
    pub draft_settings: RwSignal<ChatSettings>,
    pub sending: RwSignal<bool>,
    pub editing: RwSignal<Option<Id>>,
    pub focus_composer: RwSignal<u64>,
    next_id: StoredValue<u64>,
}

pub fn use_state() -> AppState {
    expect_context::<AppState>()
}

fn sort_chats(list: &mut [ChatSummary]) {
    list.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated_at.cmp(&a.updated_at)));
}

impl AppState {
    pub fn new() -> Self {
        Self {
            boot: RwSignal::new(Boot::Loading),
            info: RwSignal::new(None),
            config: RwSignal::new(None),
            vault: RwSignal::new(VaultStatus { encrypted: false, unlocked: true }),
            chats: RwSignal::new(Vec::new()),
            search: RwSignal::new(String::new()),
            search_results: RwSignal::new(None),
            current: RwSignal::new(None),
            chat: RwSignal::new(None),
            messages: RwSignal::new(HashMap::new()),
            structure: RwSignal::new(0),
            tick: RwSignal::new(0),
            models: RwSignal::new(ModelsResponse::default()),
            models_loading: RwSignal::new(false),
            approvals: RwSignal::new(Vec::new()),
            mcp: RwSignal::new(Vec::new()),
            toasts: RwSignal::new(Vec::new()),
            drawer: RwSignal::new(false),
            sidebar_hidden: RwSignal::new(bridge::load("novum-sidebar-hidden").as_deref() == Some("1")),
            settings: RwSignal::new(None),
            dark: RwSignal::new(bridge::prefers_dark()),
            system_dark: RwSignal::new(bridge::prefers_dark()),
            fullscreen: RwSignal::new(None),
            lightbox: RwSignal::new(None),
            confirm: RwSignal::new(None),
            loading_chat: RwSignal::new(false),
            draft_text: RwSignal::new(String::new()),
            draft_files: RwSignal::new_local(Vec::new()),
            draft_settings: RwSignal::new(ChatSettings { tools: true, ..Default::default() }),
            sending: RwSignal::new(false),
            editing: RwSignal::new(None),
            focus_composer: RwSignal::new(0),
            next_id: StoredValue::new(1),
        }
    }

    pub fn next_id(&self) -> u64 {
        let id = self.next_id.get_value();
        self.next_id.set_value(id + 1);
        id
    }

    // ── boot ───────────────────────────────────────────────────────────────

    pub fn start(self) {
        // Subscribe first so no event is missed while loading.
        let cb = Closure::<dyn FnMut(String)>::new(move |json: String| match serde_json::from_str::<AppEvent>(&json) {
            Ok(ev) => self.handle_event(ev),
            Err(e) => log::warn!("bad event: {e}: {json}"),
        });
        bridge::listen(&cb);
        cb.forget();

        let scheme = Closure::<dyn FnMut(bool)>::new(move |dark: bool| self.system_dark.set(dark));
        bridge::on_scheme_change(&scheme);
        scheme.forget();

        spawn_local(async move {
            let info = api::app_info().await;
            let config = api::get_config().await;
            let vault = api::vault_status().await;
            match (info, config, vault) {
                (Ok(info), Ok(config), Ok(vault)) => {
                    self.info.set(Some(info));
                    self.apply_config_defaults(&config);
                    self.config.set(Some(config));
                    self.vault.set(vault);
                    self.boot.set(Boot::Ready);
                    if vault.unlocked {
                        self.load_all().await;
                    }
                }
                (a, b, c) => {
                    let err = a.err().or(b.err()).or(c.err()).unwrap_or_default();
                    self.boot.set(Boot::Failed(err));
                }
            }
        });
    }

    fn apply_config_defaults(&self, cfg: &AppConfig) {
        let remembered: Option<Vec<ModelRef>> = bridge::load("novum-last-models")
            .and_then(|s| serde_json::from_str(&s).ok())
            .filter(|v: &Vec<ModelRef>| !v.is_empty());
        self.draft_settings.update(|s| {
            s.tools = cfg.tools.enabled;
            if s.models.is_empty() {
                s.models = remembered
                    .clone()
                    .or_else(|| cfg.general.default_model.clone().map(|m| vec![m]))
                    .unwrap_or_default();
            }
            s.synthesize = cfg.ensemble.synthesize;
            s.synthesizer = cfg.ensemble.synthesizer.clone();
            s.reasoning_effort = cfg.general.reasoning_effort.clone();
            s.temperature = cfg.general.temperature;
            s.max_tokens = cfg.general.max_tokens;
            if let Some(web) = bridge::load("novum-web") {
                s.web_search = web == "1";
            }
        });
    }

    /// Loads chats, models, MCP status and pending approvals.
    pub async fn load_all(self) {
        self.refresh_chats().await;
        if let Ok(a) = api::pending_approvals().await {
            self.approvals.set(a);
        }
        if let Ok(m) = api::mcp_status().await {
            self.mcp.set(m);
        }
        if let Some(last) = bridge::load("novum-last-chat")
            && self.chats.with_untracked(|c| c.iter().any(|x| x.id == last))
            && !crate::util::is_narrow()
        {
            self.open_chat(last);
        }
        self.refresh_models(false).await;
    }

    pub async fn refresh_chats(self) {
        match api::list_chats(None).await {
            Ok(mut list) => {
                sort_chats(&mut list);
                self.chats.set(list);
            }
            Err(e) => self.toast(ToastLevel::Error, format!("Could not load chats: {e}")),
        }
    }

    pub async fn refresh_models(self, refresh: bool) {
        self.models_loading.set(true);
        match api::list_models(refresh).await {
            Ok(m) => {
                let refs: Vec<ModelRef> = m.models.iter().map(|i| i.model_ref()).collect();
                crate::util::register_models(&refs);
                // If nothing is selected yet, preselect the first model.
                if self.draft_settings.with_untracked(|s| s.models.is_empty())
                    && let Some(first) = m.models.first()
                {
                    let r = first.model_ref();
                    self.draft_settings.update(|s| s.models = vec![r]);
                }
                self.models.set(m);
            }
            Err(e) => log::warn!("listing models failed: {e}"),
        }
        self.models_loading.set(false);
    }

    // ── events ─────────────────────────────────────────────────────────────

    fn is_current(&self, chat_id: &str) -> bool {
        self.current.with_untracked(|c| c.as_deref() == Some(chat_id))
    }

    pub fn handle_event(&self, ev: AppEvent) {
        match ev {
            AppEvent::MessageCreated { message } => {
                if self.is_current(&message.chat_id) {
                    let exists = self.messages.with_untracked(|m| m.contains_key(&message.id));
                    if exists {
                        if let Some(sig) = self.message(&message.id) {
                            sig.set(message);
                        }
                    } else {
                        let id = message.id.clone();
                        self.messages.update(|m| {
                            m.insert(id, ArcRwSignal::new(message));
                        });
                        self.structure.update(|v| *v += 1);
                    }
                }
            }
            AppEvent::MessageDelta { chat_id, message_id, kind, text } => {
                if self.is_current(&chat_id)
                    && let Some(sig) = self.message(&message_id)
                {
                    sig.update(|m| {
                        if m.status == MessageStatus::Pending {
                            m.status = MessageStatus::Streaming;
                        }
                        m.push_delta(kind, &text);
                    });
                    self.tick.update(|v| *v += 1);
                }
            }
            AppEvent::MessageUpdated { message } => {
                if self.is_current(&message.chat_id) {
                    match self.message(&message.id) {
                        Some(sig) => sig.set(message),
                        None => {
                            let id = message.id.clone();
                            self.messages.update(|m| {
                                m.insert(id, ArcRwSignal::new(message));
                            });
                            self.structure.update(|v| *v += 1);
                        }
                    }
                    self.tick.update(|v| *v += 1);
                }
            }
            AppEvent::MessagesDeleted { chat_id, ids } => {
                if self.is_current(&chat_id) {
                    self.messages.update(|m| {
                        for id in &ids {
                            m.remove(id);
                        }
                    });
                    self.structure.update(|v| *v += 1);
                }
            }
            AppEvent::ChatUpdated { chat } => {
                let summary = ChatSummary::from(&chat);
                self.chats.update(|list| {
                    match list.iter_mut().find(|c| c.id == chat.id) {
                        Some(c) => *c = summary,
                        None => list.push(summary),
                    }
                    sort_chats(list);
                });
                if self.is_current(&chat.id) {
                    self.chat.set(Some(chat));
                }
            }
            AppEvent::ChatDeleted { chat_id } => {
                self.chats.update(|l| l.retain(|c| c.id != chat_id));
                if self.is_current(&chat_id) {
                    self.new_chat();
                }
            }
            AppEvent::ApprovalRequested { request } => {
                self.approvals.update(|a| {
                    if !a.iter().any(|x| x.id == request.id) {
                        a.push(request);
                    }
                });
                bridge::vibrate(30);
            }
            AppEvent::ApprovalResolved { id, .. } => self.approvals.update(|a| a.retain(|x| x.id != id)),
            AppEvent::McpStatus { servers } => self.mcp.set(servers),
            AppEvent::VaultChanged { status } => {
                let was_unlocked = self.vault.get_untracked().unlocked;
                self.vault.set(status);
                if !status.unlocked {
                    self.clear_chat_state();
                    self.chats.set(Vec::new());
                } else if !was_unlocked {
                    let me = *self;
                    spawn_local(async move { me.load_all().await });
                }
            }
            AppEvent::ConfigChanged => {
                let me = *self;
                spawn_local(async move {
                    if let Ok(cfg) = api::get_config().await {
                        me.config.set(Some(cfg));
                    }
                    if let Ok(info) = api::app_info().await {
                        me.info.set(Some(info));
                    }
                    me.refresh_models(true).await;
                });
            }
            AppEvent::Toast { level, message } => self.toast(level, message),
            AppEvent::Resync => {
                let me = *self;
                spawn_local(async move {
                    me.refresh_chats().await;
                    if let Some(id) = me.current.get_untracked() {
                        me.reload_chat(&id).await;
                    }
                });
            }
        }
    }

    pub fn message(&self, id: &str) -> Option<ArcRwSignal<Message>> {
        self.messages.with_untracked(|m| m.get(id).cloned())
    }

    /// Snapshot of all messages (untracked contents).
    pub fn snapshot(&self) -> Vec<Message> {
        self.messages.with_untracked(|m| m.values().map(|s| s.get_untracked()).collect())
    }

    /// Rows of the active branch.
    pub fn rows(&self) -> Vec<Row> {
        let leaf = self.chat.with(|c| c.as_ref().and_then(|c| c.current_leaf.clone()));
        self.structure.track();
        let msgs = self.snapshot();
        let tree = Tree::new(&msgs);
        let mut rows = Vec::new();
        for m in tree.active_path(leaf.as_deref()) {
            if m.role == Role::Assistant && m.group_id.is_some() {
                let members: Vec<Id> = tree.group_members(m).iter().map(|x| x.id.clone()).collect();
                rows.push(Row::Group { group: m.group_id.clone().unwrap_or_default(), members, active: m.id.clone() });
            } else {
                rows.push(Row::Msg(m.id.clone()));
            }
        }
        rows
    }

    // ── toasts ─────────────────────────────────────────────────────────────

    pub fn toast(&self, level: ToastLevel, message: impl Into<String>) {
        let id = self.next_id();
        let message = message.into();
        self.toasts.update(|t| {
            if t.len() >= 4 {
                t.remove(0);
            }
            t.push(Toast { id, level, message })
        });
        let toasts = self.toasts;
        let ms = if level == ToastLevel::Error { 8000 } else { 4200 };
        set_timeout(move || toasts.update(|t| t.retain(|x| x.id != id)), std::time::Duration::from_millis(ms));
    }

    pub fn error(&self, e: impl Into<String>) {
        self.toast(ToastLevel::Error, e);
    }

    // ── chats ──────────────────────────────────────────────────────────────

    fn clear_chat_state(&self) {
        self.current.set(None);
        self.chat.set(None);
        self.messages.set(HashMap::new());
        self.structure.update(|v| *v += 1);
        self.editing.set(None);
    }

    pub fn new_chat(&self) {
        self.clear_chat_state();
        bridge::store("novum-last-chat", "");
        self.drawer.set(false);
        self.focus_composer.update(|v| *v += 1);
    }

    pub fn open_chat(&self, id: String) {
        if self.current.get_untracked().as_deref() == Some(id.as_str()) {
            self.drawer.set(false);
            return;
        }
        self.clear_chat_state();
        self.current.set(Some(id.clone()));
        bridge::store("novum-last-chat", &id);
        self.drawer.set(false);
        let me = *self;
        spawn_local(async move { me.reload_chat(&id).await });
    }

    pub async fn reload_chat(self, id: &str) {
        self.loading_chat.set(true);
        match api::get_chat(id).await {
            Ok(detail) => {
                if self.is_current(&detail.chat.id) {
                    let map: HashMap<Id, ArcRwSignal<Message>> =
                        detail.messages.into_iter().map(|m| (m.id.clone(), ArcRwSignal::new(m))).collect();
                    self.messages.set(map);
                    self.chat.set(Some(detail.chat));
                    self.structure.update(|v| *v += 1);
                }
            }
            Err(e) => {
                self.error(format!("Could not open the chat: {e}"));
                self.new_chat();
            }
        }
        self.loading_chat.set(false);
    }

    /// Settings for the composer: the open chat's, or the new-chat draft.
    pub fn settings_now(&self) -> ChatSettings {
        match self.chat.get() {
            Some(c) => c.settings,
            None => self.draft_settings.get(),
        }
    }

    pub fn update_settings(&self, f: impl FnOnce(&mut ChatSettings)) {
        let mut s = self.settings_now();
        f(&mut s);
        if !s.models.is_empty() {
            bridge::store("novum-last-models", &serde_json::to_string(&s.models).unwrap_or_default());
        }
        bridge::store("novum-web", if s.web_search { "1" } else { "0" });
        match self.chat.get_untracked() {
            Some(mut chat) => {
                chat.settings = s.clone();
                self.chat.set(Some(chat.clone()));
                let me = *self;
                spawn_local(async move {
                    if let Err(e) = api::update_chat(UpdateChatParams {
                        id: chat.id.clone(),
                        settings: Some(s),
                        ..Default::default()
                    })
                    .await
                    {
                        me.error(e);
                    }
                });
            }
            None => self.draft_settings.set(s),
        }
    }

    pub fn permission(&self) -> PermissionLevel {
        let global = self.config.with(|c| c.as_ref().map(|c| c.tools.permission).unwrap_or_default());
        self.settings_now().permission.unwrap_or(global)
    }

    pub fn busy(&self) -> bool {
        self.structure.track();
        self.tick.track();
        self.messages.with_untracked(|m| m.values().any(|s| s.with_untracked(|m| m.status.is_active())))
    }

    pub fn rename_chat(&self, id: String, title: String) {
        let me = *self;
        spawn_local(async move {
            if let Err(e) = api::update_chat(UpdateChatParams { id, title: Some(title), ..Default::default() }).await {
                me.error(e);
            }
        });
    }

    pub fn pin_chat(&self, id: String, pinned: bool) {
        let me = *self;
        spawn_local(async move {
            if let Err(e) = api::update_chat(UpdateChatParams { id, pinned: Some(pinned), ..Default::default() }).await
            {
                me.error(e);
            }
        });
    }

    pub fn delete_chat(&self, id: String, title: String) {
        let me = *self;
        self.confirm.set(Some(ConfirmDialog {
            title: "Delete chat?".into(),
            message: format!("“{title}” and all of its messages will be permanently deleted."),
            confirm: "Delete".into(),
            danger: true,
            on_confirm: Callback::new(move |_| {
                let id = id.clone();
                spawn_local(async move {
                    if let Err(e) = api::delete_chat(&id).await {
                        me.error(e);
                    }
                });
            }),
        }));
    }

    // ── messages ───────────────────────────────────────────────────────────

    pub fn send(&self) {
        if self.sending.get_untracked() {
            return;
        }
        let text = self.draft_text.get_untracked();
        let files = self.draft_files.get_untracked();
        if text.trim().is_empty() && files.is_empty() {
            return;
        }
        if self.busy() {
            self.toast(ToastLevel::Info, "Wait for the reply to finish, or stop it first.");
            return;
        }
        if self.settings_now().models.is_empty() && self.models.with_untracked(|m| m.models.is_empty()) {
            self.toast(ToastLevel::Warning, "No models available yet. Add a provider in Settings → Providers.");
            self.settings.set(Some(SettingsTab::Providers));
            return;
        }
        self.sending.set(true);
        let me = *self;
        spawn_local(async move {
            let result = me.send_inner(text.clone(), files.clone(), None).await;
            match result {
                Ok(()) => {
                    me.draft_text.set(String::new());
                    for f in &files {
                        if let Some(url) = &f.preview {
                            bridge::revoke_url(url);
                        }
                    }
                    me.draft_files.set(Vec::new());
                }
                Err(e) => me.error(e),
            }
            me.sending.set(false);
        });
    }

    async fn read_files(files: &[PendingAttachment]) -> Result<Vec<NewAttachment>, String> {
        let mut out = Vec::with_capacity(files.len());
        for f in files {
            let data = bridge::await_promise(bridge::read_file_base64(&f.file)).await?;
            out.push(NewAttachment {
                name: f.name.clone(),
                mime: f.mime.clone(),
                data: data.as_string().unwrap_or_default(),
            });
        }
        Ok(out)
    }

    async fn send_inner(self, text: String, files: Vec<PendingAttachment>, edit_of: Option<Id>) -> Result<(), String> {
        let chat_id = match self.current.get_untracked() {
            Some(id) => id,
            None => {
                let mut settings = self.draft_settings.get_untracked();
                if settings.models.is_empty()
                    && let Some(first) = self.models.with_untracked(|m| m.models.first().map(|x| x.model_ref()))
                {
                    settings.models = vec![first];
                }
                let chat = api::create_chat(settings).await?;
                self.current.set(Some(chat.id.clone()));
                bridge::store("novum-last-chat", &chat.id);
                self.chat.set(Some(chat.clone()));
                chat.id
            }
        };
        let attachments = Self::read_files(&files).await?;
        let result =
            api::send_message(SendMessageParams { chat_id: chat_id.clone(), text, attachments, edit_of }).await?;
        // Events normally arrive first; make sure the messages exist either way.
        if self.is_current(&chat_id) {
            for m in result.user_message.into_iter().chain(result.assistant_messages) {
                if self.message(&m.id).is_none() {
                    self.handle_event(AppEvent::MessageCreated { message: m });
                }
            }
        }
        Ok(())
    }

    pub fn edit_and_resend(&self, message_id: Id, text: String) {
        if self.busy() {
            self.toast(ToastLevel::Info, "Wait for the reply to finish, or stop it first.");
            return;
        }
        self.editing.set(None);
        let me = *self;
        spawn_local(async move {
            if let Err(e) = me.send_inner(text, Vec::new(), Some(message_id)).await {
                me.error(e);
            }
        });
    }

    pub fn stop(&self) {
        if let Some(id) = self.current.get_untracked() {
            let me = *self;
            spawn_local(async move {
                if let Err(e) = api::stop(&id, None).await {
                    me.error(e);
                }
            });
        }
    }

    pub fn regenerate(&self, message_id: Id, models: Option<Vec<ModelRef>>) {
        let Some(chat_id) = self.current.get_untracked() else { return };
        if self.busy() {
            self.toast(ToastLevel::Info, "Wait for the reply to finish, or stop it first.");
            return;
        }
        let me = *self;
        spawn_local(async move {
            if let Err(e) = api::regenerate(&chat_id, &message_id, models).await {
                me.error(e);
            }
        });
    }

    pub fn synthesize(&self, group_id: Id) {
        let Some(chat_id) = self.current.get_untracked() else { return };
        let me = *self;
        spawn_local(async move {
            if let Err(e) = api::synthesize(&chat_id, &group_id, None).await {
                me.error(e);
            }
        });
    }

    pub fn set_leaf(&self, message_id: Id) {
        let Some(chat_id) = self.current.get_untracked() else { return };
        let me = *self;
        spawn_local(async move {
            match api::set_leaf(&chat_id, &message_id).await {
                Ok(chat) => {
                    if me.is_current(&chat.id) {
                        me.chat.set(Some(chat));
                    }
                }
                Err(e) => me.error(e),
            }
        });
    }

    pub fn delete_message(&self, message_id: Id) {
        let Some(chat_id) = self.current.get_untracked() else { return };
        let me = *self;
        self.confirm.set(Some(ConfirmDialog {
            title: "Delete message?".into(),
            message: "This message and every reply that branches from it will be deleted.".into(),
            confirm: "Delete".into(),
            danger: true,
            on_confirm: Callback::new(move |_| {
                let chat_id = chat_id.clone();
                let message_id = message_id.clone();
                spawn_local(async move {
                    if let Err(e) = api::delete_message(&chat_id, &message_id).await {
                        me.error(e);
                    }
                });
            }),
        }));
    }

    pub fn resolve_approval(&self, id: Id, decision: ApprovalDecision) {
        self.approvals.update(|a| a.retain(|x| x.id != id));
        let me = *self;
        spawn_local(async move {
            if let Err(e) = api::resolve_approval(&id, decision).await {
                me.error(e);
            }
        });
    }

    pub fn open_link(&self, url: String) {
        if bridge::is_tauri() {
            let me = *self;
            spawn_local(async move {
                if let Err(e) = api::open_external(&url).await {
                    me.error(e);
                }
            });
        } else if let Some(w) = web_sys::window() {
            let _ = w.open_with_url_and_target_and_features(&url, "_blank", "noopener,noreferrer");
        }
    }

    // ── config ─────────────────────────────────────────────────────────────

    pub fn save_config(&self, cfg: AppConfig) {
        self.config.set(Some(cfg.clone()));
        let me = *self;
        spawn_local(async move {
            match api::save_config(cfg).await {
                Ok(saved) => me.config.set(Some(saved)),
                Err(e) => me.error(format!("Saving settings failed: {e}")),
            }
        });
    }

    pub fn set_theme(&self, mode: ThemeMode) {
        if let Some(mut cfg) = self.config.get_untracked() {
            cfg.appearance.theme = mode;
            self.save_config(cfg);
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
