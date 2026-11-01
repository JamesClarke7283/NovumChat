//! Persistence of chats, messages and attachments (SeaORM + SQLite), with
//! transparent column encryption through the [`Vault`].

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use novumchat_core::types::{
    AttachmentKind, AttachmentMeta, Chat, ChatSettings, ChatSummary, Message, MessageStatus, ModelRef, Part, Role,
    Usage,
};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait, Unchanged,
};
use serde::{Deserialize, Serialize};

use crate::crypto::{Codec, Vault, VaultHeader};
use crate::db::{attachment, chat, kv, message};

/// Message fields stored together in the (encrypted) `meta` column.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct MessageMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<ModelRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ttft_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// An attachment with its bytes.
#[derive(Debug, Clone)]
pub struct StoredAttachment {
    pub meta: AttachmentMeta,
    pub data: Vec<u8>,
    pub text: Option<String>,
}

pub struct Store {
    pub db: DatabaseConnection,
    vault: Arc<Vault>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

impl Store {
    pub fn new(db: DatabaseConnection, vault: Arc<Vault>) -> Self {
        Self { db, vault }
    }

    fn codec(&self) -> Result<Codec> {
        self.vault.codec()
    }

    // ── key/value ──────────────────────────────────────────────────────────

    pub async fn kv_get(&self, key: &str) -> Result<Option<String>> {
        Ok(kv::Entity::find_by_id(key.to_string()).one(&self.db).await?.map(|m| m.value))
    }

    pub async fn kv_set(&self, key: &str, value: &str) -> Result<()> {
        kv_set_on(&self.db, key, value).await
    }

    pub async fn kv_delete(&self, key: &str) -> Result<()> {
        kv::Entity::delete_by_id(key.to_string()).exec(&self.db).await?;
        Ok(())
    }

    pub async fn vault_header(&self) -> Result<Option<VaultHeader>> {
        match self.kv_get(VaultHeader::KEY).await? {
            Some(json) => Ok(Some(serde_json::from_str(&json).context("corrupted vault header")?)),
            None => Ok(None),
        }
    }

    // ── chats ──────────────────────────────────────────────────────────────

    fn chat_from_model(codec: &Codec, m: chat::Model) -> Result<Chat> {
        let settings_json = codec.dec("chat.settings", &m.settings)?;
        let settings: ChatSettings = serde_json::from_str(&settings_json).unwrap_or_default();
        Ok(Chat {
            title: codec.dec("chat.title", &m.title)?,
            system_prompt: codec.dec_opt("chat.system_prompt", m.system_prompt.as_deref())?,
            settings,
            id: m.id,
            pinned: m.pinned,
            archived: m.archived,
            current_leaf: m.current_leaf,
            created_at: m.created_at,
            updated_at: m.updated_at,
        })
    }

    fn chat_active(codec: &Codec, c: &Chat) -> Result<chat::ActiveModel> {
        Ok(chat::ActiveModel {
            id: Set(c.id.clone()),
            title: Set(codec.enc("chat.title", &c.title)),
            system_prompt: Set(codec.enc_opt("chat.system_prompt", c.system_prompt.as_deref())),
            settings: Set(codec.enc("chat.settings", &serde_json::to_string(&c.settings)?)),
            pinned: Set(c.pinned),
            archived: Set(c.archived),
            current_leaf: Set(c.current_leaf.clone()),
            created_at: Set(c.created_at),
            updated_at: Set(c.updated_at),
        })
    }

    pub async fn insert_chat(&self, c: &Chat) -> Result<()> {
        let am = Self::chat_active(&self.codec()?, c)?;
        chat::Entity::insert(am).exec_without_returning(&self.db).await?;
        Ok(())
    }

    pub async fn save_chat(&self, c: &Chat) -> Result<()> {
        let mut am = Self::chat_active(&self.codec()?, c)?;
        am.id = Unchanged(c.id.clone());
        am.update(&self.db).await?;
        Ok(())
    }

    pub async fn get_chat(&self, id: &str) -> Result<Option<Chat>> {
        let codec = self.codec()?;
        chat::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .map(|m| Self::chat_from_model(&codec, m))
            .transpose()
    }

    pub async fn require_chat(&self, id: &str) -> Result<Chat> {
        self.get_chat(id).await?.ok_or_else(|| anyhow!("Chat not found"))
    }

    pub async fn set_current_leaf(&self, chat_id: &str, leaf: Option<&str>) -> Result<()> {
        chat::Entity::update_many()
            .col_expr(chat::Column::CurrentLeaf, Expr::value(leaf.map(str::to_string)))
            .col_expr(chat::Column::UpdatedAt, Expr::value(now_ms()))
            .filter(chat::Column::Id.eq(chat_id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn touch_chat(&self, chat_id: &str) -> Result<()> {
        chat::Entity::update_many()
            .col_expr(chat::Column::UpdatedAt, Expr::value(now_ms()))
            .filter(chat::Column::Id.eq(chat_id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn list_chats(&self, archived: bool) -> Result<Vec<ChatSummary>> {
        let codec = self.codec()?;
        let rows = chat::Entity::find()
            .filter(chat::Column::Archived.eq(archived))
            .order_by_desc(chat::Column::UpdatedAt)
            .all(&self.db)
            .await?;
        rows.into_iter().map(|m| Self::chat_from_model(&codec, m).map(|c| ChatSummary::from(&c))).collect()
    }

    /// Case-insensitive search over titles and message text.
    pub async fn search_chats(&self, query: &str, archived: bool) -> Result<Vec<ChatSummary>> {
        let needle = query.trim().to_lowercase();
        let mut chats = self.list_chats(archived).await?;
        if needle.is_empty() {
            return Ok(chats);
        }
        let codec = self.codec()?;
        let rows: Vec<(String, String)> = message::Entity::find()
            .select_only()
            .column(message::Column::ChatId)
            .column(message::Column::Content)
            .order_by_asc(message::Column::CreatedAt)
            .into_tuple()
            .all(&self.db)
            .await?;
        let mut snippets: HashMap<String, String> = HashMap::new();
        for (chat_id, content) in rows {
            if snippets.contains_key(&chat_id) {
                continue;
            }
            let Ok(json) = codec.dec("message.content", &content) else { continue };
            let parts: Vec<Part> = serde_json::from_str(&json).unwrap_or_default();
            for p in parts {
                if let Part::Text { text } = p
                    && let Some(snippet) = snippet_around(&text, &needle)
                {
                    snippets.insert(chat_id.clone(), snippet);
                    break;
                }
            }
        }
        chats.retain_mut(|c| {
            if let Some(s) = snippets.remove(&c.id) {
                c.snippet = Some(s);
                true
            } else {
                c.title.to_lowercase().contains(&needle)
            }
        });
        Ok(chats)
    }

    pub async fn delete_chat(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        attachment::Entity::delete_many().filter(attachment::Column::ChatId.eq(id)).exec(&txn).await?;
        message::Entity::delete_many().filter(message::Column::ChatId.eq(id)).exec(&txn).await?;
        chat::Entity::delete_by_id(id.to_string()).exec(&txn).await?;
        txn.commit().await?;
        Ok(())
    }

    // ── messages ───────────────────────────────────────────────────────────

    fn message_from_model(codec: &Codec, m: message::Model, attachments: Vec<AttachmentMeta>) -> Result<Message> {
        let content = codec.dec("message.content", &m.content)?;
        let parts: Vec<Part> = serde_json::from_str(&content).unwrap_or_else(|_| vec![Part::Text { text: content }]);
        let meta: MessageMeta = serde_json::from_str(&codec.dec("message.meta", &m.meta)?).unwrap_or_default();
        Ok(Message {
            id: m.id,
            chat_id: m.chat_id,
            parent_id: m.parent_id,
            role: Role::parse(&m.role),
            parts,
            attachments,
            model: meta.model,
            group_id: m.group_id,
            synthesis: m.synthesis,
            status: MessageStatus::parse(&m.status),
            error: meta.error,
            usage: meta.usage,
            created_at: m.created_at,
            duration_ms: meta.duration_ms,
            ttft_ms: meta.ttft_ms,
        })
    }

    fn message_active(codec: &Codec, m: &Message) -> Result<message::ActiveModel> {
        let meta = MessageMeta {
            model: m.model.clone(),
            usage: m.usage,
            duration_ms: m.duration_ms,
            ttft_ms: m.ttft_ms,
            error: m.error.clone(),
        };
        Ok(message::ActiveModel {
            id: Set(m.id.clone()),
            chat_id: Set(m.chat_id.clone()),
            parent_id: Set(m.parent_id.clone()),
            role: Set(m.role.as_str().to_string()),
            content: Set(codec.enc("message.content", &serde_json::to_string(&m.parts)?)),
            meta: Set(codec.enc("message.meta", &serde_json::to_string(&meta)?)),
            group_id: Set(m.group_id.clone()),
            synthesis: Set(m.synthesis),
            status: Set(m.status.as_str().to_string()),
            created_at: Set(m.created_at),
            updated_at: Set(now_ms()),
        })
    }

    pub async fn insert_message(&self, m: &Message) -> Result<()> {
        let am = Self::message_active(&self.codec()?, m)?;
        message::Entity::insert(am).exec_without_returning(&self.db).await?;
        Ok(())
    }

    pub async fn save_message(&self, m: &Message) -> Result<()> {
        let mut am = Self::message_active(&self.codec()?, m)?;
        am.id = Unchanged(m.id.clone());
        am.chat_id = Unchanged(m.chat_id.clone());
        am.created_at = Unchanged(m.created_at);
        am.update(&self.db).await?;
        Ok(())
    }

    async fn attachment_metas_for(
        &self,
        codec: &Codec,
        message_ids: &[String],
    ) -> Result<HashMap<String, Vec<AttachmentMeta>>> {
        let mut out: HashMap<String, Vec<AttachmentMeta>> = HashMap::new();
        if message_ids.is_empty() {
            return Ok(out);
        }
        let rows: Vec<(String, Option<String>, String, String, String, i64)> = attachment::Entity::find()
            .select_only()
            .column(attachment::Column::Id)
            .column(attachment::Column::MessageId)
            .column(attachment::Column::Name)
            .column(attachment::Column::Mime)
            .column(attachment::Column::Kind)
            .column(attachment::Column::Size)
            .filter(attachment::Column::MessageId.is_in(message_ids.iter().cloned()))
            .order_by_asc(attachment::Column::CreatedAt)
            .into_tuple()
            .all(&self.db)
            .await?;
        for (id, message_id, name, mime, kind, size) in rows {
            let Some(mid) = message_id else { continue };
            out.entry(mid).or_default().push(AttachmentMeta {
                id,
                name: codec.dec("attachment.name", &name)?,
                mime,
                size: size.max(0) as u64,
                kind: AttachmentKind::parse(&kind),
            });
        }
        Ok(out)
    }

    pub async fn get_message(&self, id: &str) -> Result<Option<Message>> {
        let codec = self.codec()?;
        let Some(m) = message::Entity::find_by_id(id.to_string()).one(&self.db).await? else {
            return Ok(None);
        };
        let mut metas = self.attachment_metas_for(&codec, std::slice::from_ref(&m.id)).await?;
        let atts = metas.remove(&m.id).unwrap_or_default();
        Ok(Some(Self::message_from_model(&codec, m, atts)?))
    }

    pub async fn chat_messages(&self, chat_id: &str) -> Result<Vec<Message>> {
        let codec = self.codec()?;
        let rows = message::Entity::find()
            .filter(message::Column::ChatId.eq(chat_id))
            .order_by_asc(message::Column::CreatedAt)
            .all(&self.db)
            .await?;
        let ids: Vec<String> = rows.iter().map(|m| m.id.clone()).collect();
        let mut metas = self.attachment_metas_for(&codec, &ids).await?;
        rows.into_iter()
            .map(|m| {
                let atts = metas.remove(&m.id).unwrap_or_default();
                Self::message_from_model(&codec, m, atts)
            })
            .collect()
    }

    pub async fn delete_messages(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let txn = self.db.begin().await?;
        attachment::Entity::delete_many()
            .filter(attachment::Column::MessageId.is_in(ids.iter().cloned()))
            .exec(&txn)
            .await?;
        message::Entity::delete_many().filter(message::Column::Id.is_in(ids.iter().cloned())).exec(&txn).await?;
        txn.commit().await?;
        Ok(())
    }

    /// Marks messages left streaming by a crash/quit as cancelled.
    pub async fn recover_interrupted(&self) -> Result<u64> {
        let res = message::Entity::update_many()
            .col_expr(message::Column::Status, Expr::value(MessageStatus::Cancelled.as_str()))
            .filter(message::Column::Status.is_in([MessageStatus::Pending.as_str(), MessageStatus::Streaming.as_str()]))
            .exec(&self.db)
            .await?;
        Ok(res.rows_affected)
    }

    // ── attachments ────────────────────────────────────────────────────────

    pub async fn insert_attachment(
        &self,
        chat_id: &str,
        message_id: &str,
        meta: &AttachmentMeta,
        data: &[u8],
        text: Option<&str>,
    ) -> Result<()> {
        let codec = self.codec()?;
        let am = attachment::ActiveModel {
            id: Set(meta.id.clone()),
            chat_id: Set(chat_id.to_string()),
            message_id: Set(Some(message_id.to_string())),
            name: Set(codec.enc("attachment.name", &meta.name)),
            mime: Set(meta.mime.clone()),
            kind: Set(meta.kind.as_str().to_string()),
            size: Set(meta.size as i64),
            data: Set(codec.enc_bytes("attachment.data", data)),
            text: Set(codec.enc_opt("attachment.text", text)),
            created_at: Set(now_ms()),
        };
        attachment::Entity::insert(am).exec_without_returning(&self.db).await?;
        Ok(())
    }

    fn attachment_from_model(codec: &Codec, m: attachment::Model) -> Result<StoredAttachment> {
        Ok(StoredAttachment {
            meta: AttachmentMeta {
                id: m.id,
                name: codec.dec("attachment.name", &m.name)?,
                mime: m.mime,
                size: m.size.max(0) as u64,
                kind: AttachmentKind::parse(&m.kind),
            },
            data: codec.dec_bytes("attachment.data", &m.data)?,
            text: codec.dec_opt("attachment.text", m.text.as_deref())?,
        })
    }

    pub async fn get_attachment(&self, id: &str) -> Result<Option<StoredAttachment>> {
        let codec = self.codec()?;
        attachment::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .map(|m| Self::attachment_from_model(&codec, m))
            .transpose()
    }

    pub async fn message_attachments(&self, message_id: &str) -> Result<Vec<StoredAttachment>> {
        let codec = self.codec()?;
        attachment::Entity::find()
            .filter(attachment::Column::MessageId.eq(message_id))
            .order_by_asc(attachment::Column::CreatedAt)
            .all(&self.db)
            .await?
            .into_iter()
            .map(|m| Self::attachment_from_model(&codec, m))
            .collect()
    }

    // ── encryption migration ───────────────────────────────────────────────

    /// Re-encodes every sensitive value from `from` to `to` and stores the new
    /// vault header, all in one transaction.
    pub async fn reencode_all(&self, from: &Codec, to: &Codec, header: Option<&VaultHeader>) -> Result<()> {
        let txn = self.db.begin().await?;
        for m in chat::Entity::find().all(&txn).await? {
            let am = chat::ActiveModel {
                id: Unchanged(m.id.clone()),
                title: Set(to.enc("chat.title", &from.dec("chat.title", &m.title)?)),
                system_prompt: Set(to.enc_opt(
                    "chat.system_prompt",
                    from.dec_opt("chat.system_prompt", m.system_prompt.as_deref())?.as_deref(),
                )),
                settings: Set(to.enc("chat.settings", &from.dec("chat.settings", &m.settings)?)),
                ..Default::default()
            };
            am.update(&txn).await?;
        }
        for m in message::Entity::find().all(&txn).await? {
            let am = message::ActiveModel {
                id: Unchanged(m.id.clone()),
                content: Set(to.enc("message.content", &from.dec("message.content", &m.content)?)),
                meta: Set(to.enc("message.meta", &from.dec("message.meta", &m.meta)?)),
                ..Default::default()
            };
            am.update(&txn).await?;
        }
        let ids: Vec<String> =
            attachment::Entity::find().select_only().column(attachment::Column::Id).into_tuple().all(&txn).await?;
        for id in ids {
            let Some(m) = attachment::Entity::find_by_id(id.clone()).one(&txn).await? else { continue };
            let am = attachment::ActiveModel {
                id: Unchanged(id),
                name: Set(to.enc("attachment.name", &from.dec("attachment.name", &m.name)?)),
                data: Set(to.enc_bytes("attachment.data", &from.dec_bytes("attachment.data", &m.data)?)),
                text: Set(to.enc_opt("attachment.text", from.dec_opt("attachment.text", m.text.as_deref())?.as_deref())),
                ..Default::default()
            };
            am.update(&txn).await?;
        }
        match header {
            Some(h) => kv_set_on(&txn, VaultHeader::KEY, &serde_json::to_string(h)?).await?,
            None => {
                kv::Entity::delete_by_id(VaultHeader::KEY.to_string()).exec(&txn).await?;
            }
        }
        txn.commit().await?;
        Ok(())
    }
}

async fn kv_set_on<C: ConnectionTrait>(db: &C, key: &str, value: &str) -> Result<()> {
    use sea_orm::sea_query::OnConflict;
    let am = kv::ActiveModel { key: Set(key.to_string()), value: Set(value.to_string()) };
    kv::Entity::insert(am)
        .on_conflict(OnConflict::column(kv::Column::Key).update_column(kv::Column::Value).to_owned())
        .exec_without_returning(db)
        .await?;
    Ok(())
}

/// ~100 characters around the first case-insensitive match of `needle`.
fn snippet_around(text: &str, needle: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let pos = lower.find(needle)?;
    // Map the byte offset in the lowercase copy to a char offset (approximate
    // for scripts whose lowercase changes length, which is fine for a snippet).
    let char_pos = lower[..pos].chars().count();
    let chars: Vec<char> = text.chars().collect();
    let start = char_pos.saturating_sub(40);
    let end = (char_pos + needle.chars().count() + 60).min(chars.len());
    let mut s: String = chars[start..end].iter().collect::<String>().replace('\n', " ");
    if start > 0 {
        s.insert(0, '…');
    }
    if end < chars.len() {
        s.push('…');
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{FieldCipher, KdfParams, create_vault};

    async fn store() -> (tempfile::TempDir, Store, Arc<Vault>) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let vault = Arc::new(Vault::new(None));
        (dir, Store::new(db, vault.clone()), vault)
    }

    fn chat(id: &str, title: &str) -> Chat {
        Chat {
            id: id.into(),
            title: title.into(),
            system_prompt: Some("be nice".into()),
            settings: ChatSettings { models: vec![ModelRef::new("p", "m")], ..Default::default() },
            pinned: false,
            archived: false,
            current_leaf: None,
            created_at: now_ms(),
            updated_at: now_ms(),
        }
    }

    fn msg(id: &str, chat_id: &str, text: &str) -> Message {
        Message {
            id: id.into(),
            chat_id: chat_id.into(),
            parent_id: None,
            role: Role::User,
            parts: vec![Part::Text { text: text.into() }],
            attachments: vec![],
            model: Some(ModelRef::new("p", "m")),
            group_id: None,
            synthesis: false,
            status: MessageStatus::Complete,
            error: None,
            usage: None,
            created_at: now_ms(),
            duration_ms: Some(5),
            ttft_ms: None,
        }
    }

    #[tokio::test]
    async fn crud_search_and_encryption_migration() {
        let (_dir, store, vault) = store().await;
        store.insert_chat(&chat("c1", "Rust lifetimes")).await.unwrap();
        store.insert_chat(&chat("c2", "Cooking")).await.unwrap();
        store.insert_message(&msg("m1", "c2", "How do I make a soufflé rise?")).await.unwrap();
        let att = AttachmentMeta {
            id: "a1".into(),
            name: "notes.txt".into(),
            mime: "text/plain".into(),
            size: 3,
            kind: AttachmentKind::Text,
        };
        store.insert_attachment("c2", "m1", &att, b"abc", Some("abc")).await.unwrap();

        assert_eq!(store.list_chats(false).await.unwrap().len(), 2);
        let found = store.search_chats("soufflé", false).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "c2");
        assert!(found[0].snippet.as_deref().unwrap().contains("soufflé"));
        assert_eq!(store.search_chats("lifetimes", false).await.unwrap()[0].id, "c1");

        let msgs = store.chat_messages("c2").await.unwrap();
        assert_eq!(msgs[0].attachments.len(), 1);
        assert_eq!(msgs[0].model, Some(ModelRef::new("p", "m")));

        // Enable encryption.
        let (header, key) = create_vault("correct horse battery", KdfParams { m: 1024, t: 1, p: 1 }).unwrap();
        let to = Codec(Some(FieldCipher::new(key.clone())));
        store.reencode_all(&Codec::plain(), &to, Some(&header)).await.unwrap();
        vault.install(Some(header.clone()), Some(key.clone()));

        // Raw rows are now ciphertext.
        let raw = message::Entity::find_by_id("m1".to_string()).one(&store.db).await.unwrap().unwrap();
        assert!(raw.content.starts_with(crate::crypto::TEXT_PREFIX));
        assert!(!raw.content.contains("soufflé"));
        let raw_att = attachment::Entity::find_by_id("a1".to_string()).one(&store.db).await.unwrap().unwrap();
        assert!(raw_att.data.starts_with(crate::crypto::BLOB_MAGIC));
        assert_eq!(store.vault_header().await.unwrap(), Some(header.clone()));

        // Still readable through the store.
        assert_eq!(store.chat_messages("c2").await.unwrap()[0].text(), "How do I make a soufflé rise?");
        assert_eq!(store.get_attachment("a1").await.unwrap().unwrap().data, b"abc");
        assert_eq!(store.search_chats("soufflé", false).await.unwrap().len(), 1);

        // Locked → errors instead of leaking ciphertext.
        vault.lock();
        assert!(store.list_chats(false).await.is_err());
        vault.set_unlocked(key.clone());

        // Disable again.
        store.reencode_all(&to, &Codec::plain(), None).await.unwrap();
        vault.install(None, None);
        let raw = message::Entity::find_by_id("m1".to_string()).one(&store.db).await.unwrap().unwrap();
        assert!(raw.content.contains("soufflé"));
        assert!(store.vault_header().await.unwrap().is_none());

        store.delete_chat("c2").await.unwrap();
        assert!(store.get_attachment("a1").await.unwrap().is_none());
        assert!(store.chat_messages("c2").await.unwrap().is_empty());
    }
}
