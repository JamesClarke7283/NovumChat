//! The RPC contract between UI and backend.
//!
//! Every call is `method` + JSON `params`. The desktop app routes it through a
//! single Tauri command (`rpc`), web mode through `POST /api/rpc/<method>`.
//! Keeping the method list here guarantees both sides agree on names.

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::types::{ApprovalDecision, ChatSettings, Id, ModelRef, NewAttachment};

pub mod method {
    pub const APP_INFO: &str = "app_info";
    pub const GET_CONFIG: &str = "get_config";
    pub const SAVE_CONFIG: &str = "save_config";
    pub const CONFIG_TOML: &str = "config_toml";
    pub const SAVE_CONFIG_TOML: &str = "save_config_toml";
    pub const LIST_MODELS: &str = "list_models";
    pub const LIST_CHATS: &str = "list_chats";
    pub const CREATE_CHAT: &str = "create_chat";
    pub const GET_CHAT: &str = "get_chat";
    pub const UPDATE_CHAT: &str = "update_chat";
    pub const DELETE_CHAT: &str = "delete_chat";
    pub const SEND_MESSAGE: &str = "send_message";
    pub const REGENERATE: &str = "regenerate";
    pub const SYNTHESIZE: &str = "synthesize";
    pub const SET_LEAF: &str = "set_leaf";
    pub const STOP: &str = "stop";
    pub const DELETE_MESSAGE: &str = "delete_message";
    pub const GET_ATTACHMENT: &str = "get_attachment";
    pub const RESOLVE_APPROVAL: &str = "resolve_approval";
    pub const PENDING_APPROVALS: &str = "pending_approvals";
    pub const VAULT_STATUS: &str = "vault_status";
    pub const VAULT_UNLOCK: &str = "vault_unlock";
    pub const VAULT_LOCK: &str = "vault_lock";
    pub const VAULT_ENABLE: &str = "vault_enable";
    pub const VAULT_DISABLE: &str = "vault_disable";
    pub const VAULT_CHANGE: &str = "vault_change";
    pub const MCP_STATUS: &str = "mcp_status";
    pub const MCP_RESTART: &str = "mcp_restart";
    pub const IMPORT_CODEX_MCP: &str = "import_codex_mcp";
    pub const LIST_TOOLS: &str = "list_tools";
    pub const WEB_SEARCH: &str = "web_search";
    pub const OPEN_EXTERNAL: &str = "open_external";
    pub const EXPORT_CHAT: &str = "export_chat";
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListModelsParams {
    /// Ignore the cache and query every provider again.
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListChatsParams {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateChatParams {
    #[serde(default)]
    pub settings: Option<ChatSettings>,
    #[serde(default)]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdParams {
    pub id: Id,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateChatParams {
    pub id: Id,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub pinned: Option<bool>,
    #[serde(default)]
    pub archived: Option<bool>,
    /// `Some("")` clears the system prompt.
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub settings: Option<ChatSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SendMessageParams {
    pub chat_id: Id,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<NewAttachment>,
    /// Editing: the user message being replaced. The new message becomes its
    /// sibling (a new branch) and keeps its attachments. `None` continues the
    /// current branch.
    #[serde(default)]
    pub edit_of: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegenerateParams {
    pub chat_id: Id,
    /// The assistant message to regenerate (a sibling is created), or a user
    /// message to answer again.
    pub message_id: Id,
    /// Use different model(s). Defaults to the original model.
    #[serde(default)]
    pub models: Option<Vec<ModelRef>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesizeParams {
    pub chat_id: Id,
    pub group_id: Id,
    #[serde(default)]
    pub model: Option<ModelRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetLeafParams {
    pub chat_id: Id,
    pub message_id: Id,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopParams {
    pub chat_id: Id,
    #[serde(default)]
    pub message_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteMessageParams {
    pub chat_id: Id,
    pub message_id: Id,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveApprovalParams {
    pub id: Id,
    pub decision: ApprovalDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassphraseParams {
    pub passphrase: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePassphraseParams {
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveConfigParams {
    pub config: AppConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveConfigTomlParams {
    pub toml: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpRestartParams {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchParams {
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenExternalParams {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportChatParams {
    pub id: Id,
    /// "markdown" or "json".
    #[serde(default = "default_export_format")]
    pub format: String,
}

fn default_export_format() -> String {
    "markdown".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImportResult {
    pub imported: Vec<String>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub filename: String,
    pub content: String,
}

/// Result of `send_message` / `regenerate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendResult {
    #[serde(default)]
    pub user_message: Option<crate::types::Message>,
    pub assistant_messages: Vec<crate::types::Message>,
}
