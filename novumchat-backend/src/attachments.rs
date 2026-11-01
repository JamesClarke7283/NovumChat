//! Uploaded files: classification, text extraction and model formatting.

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use novumchat_core::types::{AttachmentKind, AttachmentMeta, NewAttachment};
use novumchat_core::util::{is_text_like, language_for_file};

use crate::store::{StoredAttachment, new_id};

pub const MAX_ATTACHMENT_BYTES: usize = 32 * 1024 * 1024;
/// Characters of a text/document attachment included in the prompt.
pub const MAX_INLINE_CHARS: usize = 200_000;

pub struct Prepared {
    pub meta: AttachmentMeta,
    pub data: Vec<u8>,
    pub text: Option<String>,
}

/// Decodes and classifies an upload.
pub async fn prepare(upload: NewAttachment) -> Result<Prepared> {
    let b64 = upload.data.split_once("base64,").map(|(_, d)| d).unwrap_or(&upload.data);
    let data = B64.decode(b64.trim()).map_err(|e| anyhow!("{}: invalid file data ({e})", upload.name))?;
    if data.len() > MAX_ATTACHMENT_BYTES {
        bail!("{} is larger than {} MB", upload.name, MAX_ATTACHMENT_BYTES / 1024 / 1024);
    }
    let name = sanitize_name(&upload.name);
    let mut mime = upload.mime.trim().to_lowercase();
    if mime.is_empty() || mime == "application/octet-stream" {
        mime = mime_guess::from_path(&name).first_or_octet_stream().essence_str().to_string();
    }
    let (kind, text) = if mime.starts_with("image/") && mime != "image/svg+xml" {
        (AttachmentKind::Image, None)
    } else if mime == "application/pdf" || name.to_lowercase().ends_with(".pdf") {
        let bytes = data.clone();
        let text = tokio::task::spawn_blocking(move || extract_pdf(&bytes)).await.unwrap_or(None);
        (AttachmentKind::Document, text)
    } else if is_text_like(&name, &mime) || looks_like_text(&data) {
        (AttachmentKind::Text, Some(String::from_utf8_lossy(&data).into_owned()))
    } else {
        (AttachmentKind::Binary, None)
    };
    Ok(Prepared { meta: AttachmentMeta { id: new_id(), name, mime, size: data.len() as u64, kind }, data, text })
}

fn sanitize_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    if base.is_empty() { "file".into() } else { novumchat_core::util::truncate_chars(base, 120) }
}

fn looks_like_text(data: &[u8]) -> bool {
    let sample = &data[..data.len().min(64 * 1024)];
    if sample.is_empty() || sample.contains(&0) {
        return false;
    }
    match std::str::from_utf8(sample) {
        Ok(_) => true,
        // `error_len() == None`: a multi-byte character was cut by the sample boundary.
        Err(e) => e.error_len().is_none(),
    }
}

fn extract_pdf(bytes: &[u8]) -> Option<String> {
    // pdf-extract may panic on unusual files; never let that take the app down.
    let result = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(bytes));
    match result {
        Ok(Ok(text)) => {
            let t = text.trim().to_string();
            (!t.is_empty()).then_some(t)
        }
        Ok(Err(e)) => {
            tracing::warn!("PDF text extraction failed: {e}");
            None
        }
        Err(_) => {
            tracing::warn!("PDF text extraction panicked");
            None
        }
    }
}

/// Text block describing a non-image attachment for the model.
pub fn inline_text(att: &StoredAttachment) -> String {
    let name = &att.meta.name;
    match (&att.meta.kind, &att.text) {
        (AttachmentKind::Text | AttachmentKind::Document, Some(text)) => {
            let total = text.chars().count();
            let body: String =
                if total > MAX_INLINE_CHARS { text.chars().take(MAX_INLINE_CHARS).collect() } else { text.clone() };
            let note = if total > MAX_INLINE_CHARS {
                format!("\n[truncated: {MAX_INLINE_CHARS} of {total} characters shown]")
            } else {
                String::new()
            };
            let lang = language_for_file(name);
            if att.meta.kind == AttachmentKind::Document {
                format!("<file name=\"{name}\" type=\"{}\">\n{body}{note}\n</file>", att.meta.mime)
            } else {
                format!("<file name=\"{name}\">\n```{lang}\n{body}\n```{note}\n</file>")
            }
        }
        (AttachmentKind::Document, None) => format!(
            "<file name=\"{name}\" type=\"{}\">[No text could be extracted from this document]</file>",
            att.meta.mime
        ),
        _ => format!(
            "<file name=\"{name}\" type=\"{}\" size=\"{}\">[Binary file — content not shown]</file>",
            att.meta.mime,
            novumchat_core::util::human_bytes(att.meta.size)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn classifies_uploads() {
        let p =
            prepare(NewAttachment { name: "a/b/main.rs".into(), mime: "".into(), data: B64.encode("fn main() {}") })
                .await
                .unwrap();
        assert_eq!(p.meta.kind, AttachmentKind::Text);
        assert_eq!(p.meta.name, "main.rs");
        assert_eq!(p.text.as_deref(), Some("fn main() {}"));
        let p = prepare(NewAttachment {
            name: "pic.png".into(),
            mime: "image/png".into(),
            data: format!("data:image/png;base64,{}", B64.encode([137u8, 80, 78, 71])),
        })
        .await
        .unwrap();
        assert_eq!(p.meta.kind, AttachmentKind::Image);
        assert_eq!(p.data.len(), 4);
        let p = prepare(NewAttachment {
            name: "blob.bin".into(),
            mime: "application/octet-stream".into(),
            data: B64.encode([0u8, 1, 2, 255]),
        })
        .await
        .unwrap();
        assert_eq!(p.meta.kind, AttachmentKind::Binary);
        let p = prepare(NewAttachment {
            name: "broken.pdf".into(),
            mime: "application/pdf".into(),
            data: B64.encode(b"%PDF-1.4 nonsense"),
        })
        .await
        .unwrap();
        assert_eq!(p.meta.kind, AttachmentKind::Document);
        assert!(p.text.is_none());
    }
}
