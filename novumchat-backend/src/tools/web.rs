//! `fetch_url`: downloads a page and converts it to readable text.

use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use scraper::{Html, Selector};
use serde_json::{Value, json};

use super::ToolOutput;
use crate::search::BROWSER_UA;

const MAX_BYTES: usize = 6 * 1024 * 1024;

pub async fn fetch_url(args: &Value, max_output: usize) -> Result<ToolOutput> {
    let raw = args.get("url").and_then(Value::as_str).ok_or_else(|| anyhow!("missing required argument `url`"))?;
    let url = url::Url::parse(raw.trim()).map_err(|e| anyhow!("invalid URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("only http(s) URLs can be fetched");
    }
    let max_chars = args
        .get("max_chars")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(max_output)
        .clamp(1000, max_output.max(1000));
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(40))
        .user_agent(BROWSER_UA)
        .gzip(true)
        .brotli(true)
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()?;
    let mut resp = client
        .get(url.clone())
        .header("Accept", "text/html,application/xhtml+xml,application/json;q=0.9,text/plain;q=0.8,*/*;q=0.5")
        .send()
        .await
        .map_err(|e| anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let final_url = resp.url().to_string();
    let content_type =
        resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| anyhow!("download failed: {e}"))? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() >= MAX_BYTES {
            break;
        }
    }
    if !status.is_success() {
        bail!("HTTP {} for {}", status, final_url);
    }
    let (title, body) = if content_type.contains("html") || looks_like_html(&bytes) {
        html_to_text(&bytes)
    } else if content_type.contains("json") {
        let text = String::from_utf8_lossy(&bytes);
        let pretty = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or_else(|| text.into_owned());
        (None, pretty)
    } else if content_type.starts_with("text/") || content_type.contains("xml") || content_type.is_empty() {
        (None, String::from_utf8_lossy(&bytes).into_owned())
    } else {
        (None, format!("[{} bytes of {content_type} content not shown]", bytes.len()))
    };
    let mut text = format!("URL: {final_url}\n");
    if let Some(t) = &title {
        text.push_str(&format!("Title: {t}\n"));
    }
    text.push('\n');
    let body_chars = body.chars().count();
    if body_chars > max_chars {
        text.push_str(&body.chars().take(max_chars).collect::<String>());
        text.push_str(&format!("\n\n[truncated: showing {max_chars} of {body_chars} characters]"));
    } else {
        text.push_str(&body);
    }
    Ok(ToolOutput::text(text).with_data(json!({ "url": final_url, "title": title, "status": status.as_u16() })))
}

fn looks_like_html(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_lowercase();
    head.contains("<!doctype html") || head.contains("<html")
}

/// Picks the main content (article/main) when present and renders it as text.
pub fn html_to_text(bytes: &[u8]) -> (Option<String>, String) {
    let html = String::from_utf8_lossy(bytes);
    let doc = Html::parse_document(&html);
    let title = Selector::parse("title")
        .ok()
        .and_then(|s| doc.select(&s).next())
        .map(|t| t.text().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty());
    let mut fragment: Option<String> = None;
    for sel in ["article", "main", "[role=main]", "#content", ".content"] {
        if let Ok(s) = Selector::parse(sel)
            && let Some(el) = doc.select(&s).next()
        {
            let len: usize = el.text().map(str::len).sum();
            if len > 400 {
                fragment = Some(el.html());
                break;
            }
        }
    }
    let source = fragment.unwrap_or_else(|| html.into_owned());
    let text = html2text::from_read(source.as_bytes(), 100).unwrap_or_default();
    // Collapse runs of blank lines.
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    (title, out.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_main_content() {
        let page = format!(
            "<html><head><title> My  Page </title><script>var x=1;</script></head><body><nav>Menu Home About</nav>\
<article><h1>Heading</h1><p>{}</p></article><footer>foot</footer></body></html>",
            "Lorem ipsum dolor sit amet. ".repeat(30)
        );
        let (title, text) = html_to_text(page.as_bytes());
        assert_eq!(title.as_deref(), Some("My Page"));
        assert!(text.contains("Heading"));
        assert!(text.contains("Lorem ipsum"));
        assert!(!text.contains("Menu Home"));
        assert!(!text.contains("var x"));
    }
}
