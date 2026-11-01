//! Typed RPC client. Works over Tauri IPC (desktop/mobile) or HTTP (web mode);
//! the transport is chosen by `bridge.js`.

use novumchat_core::config::AppConfig;
use novumchat_core::rpc::{method as m, *};
use novumchat_core::types::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::bridge;

pub type ApiResult<T> = Result<T, String>;

pub async fn call<T: DeserializeOwned>(method: &str, params: impl Serialize) -> ApiResult<T> {
    let json = serde_json::to_string(&params).map_err(|e| e.to_string())?;
    let promise = bridge::invoke(method, &json).map_err(bridge::js_error)?;
    let value = bridge::await_promise(promise).await?;
    let text = value.as_string().unwrap_or_else(|| "null".into());
    serde_json::from_str(&text).map_err(|e| format!("unexpected response from `{method}`: {e}"))
}

pub async fn app_info() -> ApiResult<AppInfo> {
    call(m::APP_INFO, ()).await
}

pub async fn get_config() -> ApiResult<AppConfig> {
    call(m::GET_CONFIG, ()).await
}

pub async fn save_config(config: AppConfig) -> ApiResult<AppConfig> {
    call(m::SAVE_CONFIG, SaveConfigParams { config }).await
}

pub async fn config_toml() -> ApiResult<String> {
    call(m::CONFIG_TOML, ()).await
}

pub async fn save_config_toml(toml: String) -> ApiResult<AppConfig> {
    call(m::SAVE_CONFIG_TOML, SaveConfigTomlParams { toml }).await
}

pub async fn list_models(refresh: bool) -> ApiResult<ModelsResponse> {
    call(m::LIST_MODELS, ListModelsParams { refresh }).await
}

pub async fn list_chats(query: Option<String>) -> ApiResult<Vec<ChatSummary>> {
    call(m::LIST_CHATS, ListChatsParams { query, archived: false }).await
}

pub async fn create_chat(settings: ChatSettings) -> ApiResult<Chat> {
    call(m::CREATE_CHAT, CreateChatParams { settings: Some(settings), system_prompt: None }).await
}

pub async fn get_chat(id: &str) -> ApiResult<ChatDetail> {
    call(m::GET_CHAT, IdParams { id: id.into() }).await
}

pub async fn update_chat(p: UpdateChatParams) -> ApiResult<Chat> {
    call(m::UPDATE_CHAT, p).await
}

pub async fn delete_chat(id: &str) -> ApiResult<()> {
    call(m::DELETE_CHAT, IdParams { id: id.into() }).await
}

pub async fn send_message(p: SendMessageParams) -> ApiResult<SendResult> {
    call(m::SEND_MESSAGE, p).await
}

pub async fn regenerate(chat_id: &str, message_id: &str, models: Option<Vec<ModelRef>>) -> ApiResult<SendResult> {
    call(m::REGENERATE, RegenerateParams { chat_id: chat_id.into(), message_id: message_id.into(), models }).await
}

pub async fn synthesize(chat_id: &str, group_id: &str, model: Option<ModelRef>) -> ApiResult<Message> {
    call(m::SYNTHESIZE, SynthesizeParams { chat_id: chat_id.into(), group_id: group_id.into(), model }).await
}

pub async fn set_leaf(chat_id: &str, message_id: &str) -> ApiResult<Chat> {
    call(m::SET_LEAF, SetLeafParams { chat_id: chat_id.into(), message_id: message_id.into() }).await
}

pub async fn stop(chat_id: &str, message_id: Option<String>) -> ApiResult<usize> {
    call(m::STOP, StopParams { chat_id: chat_id.into(), message_id }).await
}

pub async fn delete_message(chat_id: &str, message_id: &str) -> ApiResult<()> {
    call(m::DELETE_MESSAGE, DeleteMessageParams { chat_id: chat_id.into(), message_id: message_id.into() }).await
}

pub async fn get_attachment(id: &str) -> ApiResult<AttachmentData> {
    call(m::GET_ATTACHMENT, IdParams { id: id.into() }).await
}

pub async fn resolve_approval(id: &str, decision: ApprovalDecision) -> ApiResult<bool> {
    call(m::RESOLVE_APPROVAL, ResolveApprovalParams { id: id.into(), decision }).await
}

pub async fn pending_approvals() -> ApiResult<Vec<ApprovalRequest>> {
    call(m::PENDING_APPROVALS, ()).await
}

pub async fn vault_status() -> ApiResult<VaultStatus> {
    call(m::VAULT_STATUS, ()).await
}

pub async fn vault_unlock(passphrase: String) -> ApiResult<VaultStatus> {
    call(m::VAULT_UNLOCK, PassphraseParams { passphrase }).await
}

pub async fn vault_lock() -> ApiResult<VaultStatus> {
    call(m::VAULT_LOCK, ()).await
}

pub async fn vault_enable(passphrase: String) -> ApiResult<VaultStatus> {
    call(m::VAULT_ENABLE, PassphraseParams { passphrase }).await
}

pub async fn vault_disable(passphrase: String) -> ApiResult<VaultStatus> {
    call(m::VAULT_DISABLE, PassphraseParams { passphrase }).await
}

pub async fn vault_change(old: String, new: String) -> ApiResult<VaultStatus> {
    call(m::VAULT_CHANGE, ChangePassphraseParams { old, new }).await
}

pub async fn mcp_status() -> ApiResult<Vec<McpServerStatus>> {
    call(m::MCP_STATUS, ()).await
}

pub async fn mcp_restart(name: Option<String>) -> ApiResult<Vec<McpServerStatus>> {
    call(m::MCP_RESTART, McpRestartParams { name }).await
}

pub async fn import_codex_mcp() -> ApiResult<ImportResult> {
    call(m::IMPORT_CODEX_MCP, ()).await
}

pub async fn list_tools() -> ApiResult<Vec<ToolDescriptor>> {
    call(m::LIST_TOOLS, ()).await
}

pub async fn web_search(query: String) -> ApiResult<SearchResponse> {
    call(m::WEB_SEARCH, WebSearchParams { query }).await
}

pub async fn open_external(url: &str) -> ApiResult<()> {
    call(m::OPEN_EXTERNAL, json!({ "url": url })).await
}

pub async fn export_chat(id: &str, format: &str) -> ApiResult<ExportResult> {
    call(m::EXPORT_CHAT, ExportChatParams { id: id.into(), format: format.into() }).await
}
