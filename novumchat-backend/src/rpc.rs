//! Routes `method` + JSON params to the application (shared by the Tauri
//! command and the web-mode HTTP API).

use std::sync::Arc;

use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use novumchat_core::rpc::{method as m, *};
use novumchat_core::types::AttachmentData;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::state::App;

fn params<T: DeserializeOwned>(method: &str, v: Value) -> Result<T> {
    let v = if v.is_null() { Value::Object(Default::default()) } else { v };
    serde_json::from_value(v).map_err(|e| anyhow!("invalid parameters for `{method}`: {e}"))
}

fn ok<T: Serialize>(v: T) -> Result<Value> {
    Ok(serde_json::to_value(v)?)
}

pub async fn dispatch(app: &Arc<App>, method: &str, p: Value) -> std::result::Result<Value, String> {
    let result = route(app, method, p).await;
    result.map_err(|e| {
        let msg = format!("{e:#}");
        tracing::debug!("rpc `{method}` failed: {msg}");
        msg
    })
}

async fn route(app: &Arc<App>, method: &str, p: Value) -> Result<Value> {
    match method {
        m::APP_INFO => ok(app.info()),
        m::GET_CONFIG => {
            if let Some(err) = app.config.take_load_error() {
                app.events.toast(novumchat_core::types::ToastLevel::Warning, err);
            }
            ok(app.config.get())
        }
        m::SAVE_CONFIG => {
            let p: SaveConfigParams = params(method, p)?;
            ok(app.save_config(p.config)?)
        }
        m::CONFIG_TOML => ok(app.config.raw()?),
        m::SAVE_CONFIG_TOML => {
            let p: SaveConfigTomlParams = params(method, p)?;
            ok(app.save_config_toml(&p.toml)?)
        }
        m::LIST_MODELS => {
            let p: ListModelsParams = params(method, p)?;
            ok(app.list_models(p.refresh).await)
        }
        m::LIST_CHATS => {
            let p: ListChatsParams = params(method, p)?;
            match p.query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
                Some(q) => ok(app.store.search_chats(q, p.archived).await?),
                None => ok(app.store.list_chats(p.archived).await?),
            }
        }
        m::CREATE_CHAT => ok(app.create_chat(params(method, p)?).await?),
        m::GET_CHAT => {
            let p: IdParams = params(method, p)?;
            ok(app.chat_detail(&p.id).await?)
        }
        m::UPDATE_CHAT => ok(app.update_chat(params(method, p)?).await?),
        m::DELETE_CHAT => {
            let p: IdParams = params(method, p)?;
            app.delete_chat(&p.id).await?;
            ok(())
        }
        m::SEND_MESSAGE => ok(app.send_message(params(method, p)?).await?),
        m::REGENERATE => ok(app.regenerate(params(method, p)?).await?),
        m::SYNTHESIZE => ok(app.synthesize(params(method, p)?).await?),
        m::SET_LEAF => ok(app.set_leaf(params(method, p)?).await?),
        m::STOP => {
            let p: StopParams = params(method, p)?;
            ok(app.stop(&p.chat_id, p.message_id.as_deref()))
        }
        m::DELETE_MESSAGE => {
            let p: DeleteMessageParams = params(method, p)?;
            app.delete_message(&p.chat_id, &p.message_id).await?;
            ok(())
        }
        m::GET_ATTACHMENT => {
            let p: IdParams = params(method, p)?;
            let att = app.store.get_attachment(&p.id).await?.ok_or_else(|| anyhow!("Attachment not found"))?;
            ok(AttachmentData { meta: att.meta, data: B64.encode(&att.data) })
        }
        m::RESOLVE_APPROVAL => {
            let p: ResolveApprovalParams = params(method, p)?;
            ok(app.approvals.resolve(&p.id, p.decision))
        }
        m::PENDING_APPROVALS => ok(app.approvals.pending()),
        m::VAULT_STATUS => ok(app.vault.status()),
        m::VAULT_UNLOCK => {
            let p: PassphraseParams = params(method, p)?;
            ok(app.vault_unlock(p.passphrase).await?)
        }
        m::VAULT_LOCK => ok(app.vault_lock()?),
        m::VAULT_ENABLE => {
            let p: PassphraseParams = params(method, p)?;
            ok(app.vault_enable(p.passphrase).await?)
        }
        m::VAULT_DISABLE => {
            let p: PassphraseParams = params(method, p)?;
            ok(app.vault_disable(p.passphrase).await?)
        }
        m::VAULT_CHANGE => {
            let p: ChangePassphraseParams = params(method, p)?;
            ok(app.vault_change(p.old, p.new).await?)
        }
        m::MCP_STATUS => ok(app.mcp.statuses()),
        m::MCP_RESTART => {
            let p: McpRestartParams = params(method, p)?;
            app.mcp.restart(p.name.as_deref());
            ok(app.mcp.statuses())
        }
        m::IMPORT_CODEX_MCP => ok(app.import_codex_mcp()?),
        m::LIST_TOOLS => ok(app.list_tools()),
        m::WEB_SEARCH => {
            let p: WebSearchParams = params(method, p)?;
            let cfg = app.config.get();
            ok(crate::search::search(&cfg.search, &p.query, None).await?)
        }
        m::OPEN_EXTERNAL => {
            let p: OpenExternalParams = params(method, p)?;
            app.open_external(&p.url)?;
            ok(())
        }
        m::EXPORT_CHAT => ok(app.export_chat(params(method, p)?).await?),
        other => Err(anyhow!("unknown method `{other}`")),
    }
}
