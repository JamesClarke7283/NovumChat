//! Internet search backends: DuckDuckGo (no API key), SearXNG and Brave.

use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use novumchat_core::config::{SearchConfig, SearchEngine};
use novumchat_core::types::{SearchResponse, SearchResult};
use scraper::{Html, Selector};
use serde_json::Value;

/// A common desktop browser UA: DuckDuckGo's HTML endpoint rejects unknown clients.
pub const BROWSER_UA: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

/// HTTP client for search engines and web pages. On desktop it uses the
/// platform TLS stack: DuckDuckGo rejects the TLS fingerprint of rustls.
pub fn client() -> reqwest::Client {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
        .user_agent(BROWSER_UA)
        .gzip(true)
        .brotli(true);
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.tls_backend_native();
    builder.build().unwrap_or_default()
}

type CacheKey = (SearchEngine, String, Option<String>, bool);
type Cache = std::collections::HashMap<CacheKey, (std::time::Instant, Vec<SearchResult>)>;

/// Recent results, so repeated searches don't hit (and anger) the engine.
static CACHE: std::sync::LazyLock<parking_lot::Mutex<Cache>> = std::sync::LazyLock::new(Default::default);
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);

pub async fn search(cfg: &SearchConfig, query: &str, max_results: Option<usize>) -> Result<SearchResponse> {
    let query = query.trim();
    if query.is_empty() {
        bail!("Empty search query");
    }
    let n = max_results.unwrap_or(cfg.max_results).clamp(1, 20);
    let key: CacheKey = (cfg.engine, query.to_lowercase(), cfg.region.clone(), cfg.safe_search);
    let cached = CACHE.lock().get(&key).filter(|(at, _)| at.elapsed() < CACHE_TTL).map(|(_, r)| r.clone());
    let mut results = match cached {
        Some(r) => r,
        None => {
            let mut fresh = match cfg.engine {
                SearchEngine::Duckduckgo => duckduckgo(cfg, query).await?,
                SearchEngine::Searxng => searxng(cfg, query).await?,
                SearchEngine::Brave => brave(cfg, query, n.max(10)).await?,
            };
            dedupe(&mut fresh);
            if !fresh.is_empty() {
                let mut cache = CACHE.lock();
                cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
                cache.insert(key, (std::time::Instant::now(), fresh.clone()));
            }
            fresh
        }
    };
    results.truncate(n);
    Ok(SearchResponse { engine: cfg.engine, query: query.to_string(), results })
}

fn dedupe(results: &mut Vec<SearchResult>) {
    let mut seen = std::collections::HashSet::new();
    results.retain(|r| !r.url.is_empty() && seen.insert(r.url.trim_end_matches('/').to_string()));
}

// ── DuckDuckGo ─────────────────────────────────────────────────────────────
//
// No API key: DuckDuckGo's HTML, JSON (d.js), lite and Instant Answer
// endpoints are tried in turn, because any one of them may answer with a
// bot check ("202 Accepted") at times.

async fn duckduckgo(cfg: &SearchConfig, query: &str) -> Result<Vec<SearchResult>> {
    use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, HOST, REFERER, USER_AGENT};
    let http = client();
    let region = cfg.region.clone().unwrap_or_else(|| "wt-wt".into());
    let safe = if cfg.safe_search { "1" } else { "-2" };
    let mut blocked = false;

    // 1. HTML endpoint (browser-like header order).
    let form = format!(
        "q={}&kl={}&kp={}&b=",
        urlencoding::encode(query).replace("%20", "+"),
        urlencoding::encode(&region),
        safe
    );
    if let Ok(resp) = http
        .post("https://html.duckduckgo.com/html/")
        .header(HOST, "html.duckduckgo.com")
        .header(USER_AGENT, BROWSER_UA)
        .header(ACCEPT, "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header(ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .header(REFERER, "https://html.duckduckgo.com/")
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form)
        .send()
        .await
    {
        blocked |= resp.status() == reqwest::StatusCode::ACCEPTED;
        let body = resp.text().await.unwrap_or_default();
        let results = parse_ddg_html(&body);
        if !results.is_empty() {
            return Ok(results);
        }
    }

    // 2. JSON endpoint used by duckduckgo.com itself (needs a vqd token).
    if let Ok(results) = ddg_json(&http, query, &region, cfg.safe_search).await
        && !results.is_empty()
    {
        return Ok(results);
    }

    // 3. Lite endpoint.
    if let Ok(resp) = http
        .get("https://lite.duckduckgo.com/lite/")
        .query(&[("q", query), ("kl", region.as_str()), ("kp", safe)])
        .header(REFERER, "https://lite.duckduckgo.com/")
        .send()
        .await
    {
        blocked |= resp.status() == reqwest::StatusCode::ACCEPTED;
        let body = resp.text().await.unwrap_or_default();
        let results = parse_ddg_lite(&body);
        if !results.is_empty() {
            return Ok(results);
        }
    }

    // 4. Instant Answer API: no full web results, but rarely blocked.
    if let Ok(resp) = http
        .get("https://api.duckduckgo.com/")
        .query(&[("q", query), ("format", "json"), ("no_html", "1"), ("skip_disambig", "1")])
        .send()
        .await
        && let Ok(v) = resp.json::<Value>().await
    {
        let results = parse_ddg_instant(&v);
        if !results.is_empty() {
            return Ok(results);
        }
    }

    if blocked {
        bail!(
            "DuckDuckGo is rate-limiting automated searches from this network right now. Try again in a few minutes, or switch to SearXNG or Brave in Settings → Web search."
        );
    }
    Ok(Vec::new())
}

async fn ddg_json(http: &reqwest::Client, query: &str, region: &str, safe: bool) -> Result<Vec<SearchResult>> {
    let page = http
        .get("https://duckduckgo.com/")
        .query(&[("q", query)])
        .header("Referer", "https://duckduckgo.com/")
        .send()
        .await?
        .text()
        .await?;
    let vqd = extract_vqd(&page).ok_or_else(|| anyhow!("no vqd token"))?;
    let body = http
        .get("https://links.duckduckgo.com/d.js")
        .query(&[
            ("q", query),
            ("vqd", vqd.as_str()),
            ("kl", region),
            ("l", region),
            ("p", if safe { "1" } else { "-1" }),
            ("s", "0"),
            ("df", ""),
            ("ex", if safe { "1" } else { "-1" }),
        ])
        .header("Referer", "https://duckduckgo.com/")
        .send()
        .await?
        .text()
        .await?;
    Ok(parse_ddg_json(&body))
}

/// The `vqd` token embedded in duckduckgo.com result pages.
pub fn extract_vqd(page: &str) -> Option<String> {
    for marker in ["vqd=\"", "vqd='", "vqd=", "\"vqd\":\""] {
        if let Some(pos) = page.find(marker) {
            let rest = &page[pos + marker.len()..];
            let token: String =
                rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
            if token.len() > 4 {
                return Some(token);
            }
        }
    }
    None
}

/// Parses `DDG.pageLayout.load('d',[...])` from the d.js endpoint.
pub fn parse_ddg_json(body: &str) -> Vec<SearchResult> {
    let Some(start) = body.find("DDG.pageLayout.load('d',") else { return Vec::new() };
    let rest = &body[start + "DDG.pageLayout.load('d',".len()..];
    // The array ends at the matching bracket.
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    let mut end = None;
    for (i, c) in rest.char_indices() {
        if in_str {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(end) = end else { return Vec::new() };
    let Ok(items) = serde_json::from_str::<Vec<Value>>(&rest[..end]) else { return Vec::new() };
    items
        .iter()
        .filter_map(|r| {
            let url = r.get("u")?.as_str()?.to_string();
            if !url.starts_with("http") || url.contains("duckduckgo.com/y.js") {
                return None;
            }
            Some(SearchResult {
                title: strip_tags(r.get("t")?.as_str()?),
                url,
                snippet: strip_tags(r.get("a").and_then(Value::as_str).unwrap_or("")),
            })
        })
        .collect()
}

/// Instant Answer API → results (abstract + related topics).
pub fn parse_ddg_instant(v: &Value) -> Vec<SearchResult> {
    let mut out = Vec::new();
    if let (Some(url), Some(text)) = (
        v.get("AbstractURL").and_then(Value::as_str).filter(|u| !u.is_empty()),
        v.get("AbstractText").and_then(Value::as_str).filter(|t| !t.is_empty()),
    ) {
        out.push(SearchResult {
            title: v.get("Heading").and_then(Value::as_str).unwrap_or("Summary").to_string(),
            url: url.to_string(),
            snippet: text.to_string(),
        });
    }
    fn topics(list: &[Value], out: &mut Vec<SearchResult>) {
        for t in list {
            if let Some(sub) = t.get("Topics").and_then(Value::as_array) {
                topics(sub, out);
                continue;
            }
            let (Some(url), Some(text)) =
                (t.get("FirstURL").and_then(Value::as_str), t.get("Text").and_then(Value::as_str))
            else {
                continue;
            };
            let title = text.split(" - ").next().unwrap_or(text).to_string();
            out.push(SearchResult { title, url: url.to_string(), snippet: text.to_string() });
        }
    }
    if let Some(list) = v.get("RelatedTopics").and_then(Value::as_array) {
        topics(list, &mut out);
    }
    if let Some(list) = v.get("Results").and_then(Value::as_array) {
        topics(list, &mut out);
    }
    out
}

/// DuckDuckGo wraps result links as `//duckduckgo.com/l/?uddg=<encoded>&rut=…`.
pub fn ddg_decode_href(href: &str) -> String {
    let href = href.trim();
    if let Some(pos) = href.find("uddg=") {
        let rest = &href[pos + 5..];
        let enc = rest.split('&').next().unwrap_or(rest);
        if let Ok(decoded) = urlencoding::decode(enc) {
            return decoded.into_owned();
        }
    }
    if let Some(stripped) = href.strip_prefix("//") {
        return format!("https://{stripped}");
    }
    href.to_string()
}

fn text_of(el: scraper::ElementRef<'_>) -> String {
    el.text().collect::<Vec<_>>().join("").split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn parse_ddg_html(body: &str) -> Vec<SearchResult> {
    let doc = Html::parse_document(body);
    let result_sel = Selector::parse("div.result").expect("selector");
    let title_sel = Selector::parse("a.result__a").expect("selector");
    let snippet_sel = Selector::parse(".result__snippet").expect("selector");
    let mut out = Vec::new();
    for r in doc.select(&result_sel) {
        let classes: Vec<&str> = r.value().classes().collect();
        if classes.iter().any(|c| c.contains("result--ad")) {
            continue;
        }
        let Some(a) = r.select(&title_sel).next() else { continue };
        let url = ddg_decode_href(a.value().attr("href").unwrap_or(""));
        if url.contains("duckduckgo.com/y.js") || !url.starts_with("http") {
            continue;
        }
        let title = text_of(a);
        let snippet = r.select(&snippet_sel).next().map(text_of).unwrap_or_default();
        out.push(SearchResult { title, url, snippet });
    }
    out
}

pub fn parse_ddg_lite(body: &str) -> Vec<SearchResult> {
    let doc = Html::parse_document(body);
    let link_sel = Selector::parse("a.result-link").expect("selector");
    let snippet_sel = Selector::parse("td.result-snippet").expect("selector");
    let snippets: Vec<String> = doc.select(&snippet_sel).map(text_of).collect();
    doc.select(&link_sel)
        .enumerate()
        .filter_map(|(i, a)| {
            let url = ddg_decode_href(a.value().attr("href")?);
            if !url.starts_with("http") || url.contains("duckduckgo.com/y.js") {
                return None;
            }
            Some(SearchResult { title: text_of(a), url, snippet: snippets.get(i).cloned().unwrap_or_default() })
        })
        .collect()
}

// ── SearXNG ────────────────────────────────────────────────────────────────

async fn searxng(cfg: &SearchConfig, query: &str) -> Result<Vec<SearchResult>> {
    let base = cfg.searxng_url.trim().trim_end_matches('/');
    if base.is_empty() {
        bail!("Set the SearXNG URL in Settings → Web search");
    }
    let mut params: Vec<(&str, String)> = vec![
        ("q", query.to_string()),
        ("format", "json".into()),
        ("safesearch", if cfg.safe_search { "1".into() } else { "0".into() }),
    ];
    if let Some(lang) = &cfg.region {
        params.push(("language", lang.clone()));
    }
    let resp = client()
        .get(format!("{base}/search"))
        .query(&params)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| anyhow!("Could not reach SearXNG at {base}: {e}"))?;
    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        bail!("SearXNG refused JSON output. Enable it in settings.yml: search.formats: [html, json]");
    }
    if !resp.status().is_success() {
        bail!("SearXNG returned HTTP {}", resp.status());
    }
    let v: Value = resp.json().await.map_err(|e| anyhow!("Invalid SearXNG response: {e}"))?;
    Ok(parse_searxng(&v))
}

pub fn parse_searxng(v: &Value) -> Vec<SearchResult> {
    v.get("results")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    Some(SearchResult {
                        title: r.get("title")?.as_str()?.trim().to_string(),
                        url: r.get("url")?.as_str()?.to_string(),
                        snippet: r.get("content").and_then(Value::as_str).unwrap_or("").trim().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ── Brave ──────────────────────────────────────────────────────────────────

fn brave_key(cfg: &SearchConfig) -> Option<String> {
    cfg.brave_api_key
        .clone()
        .filter(|k| !k.trim().is_empty())
        .or_else(|| cfg.brave_api_key_env.as_deref().and_then(|v| std::env::var(v).ok()))
        .filter(|k| !k.trim().is_empty())
}

async fn brave(cfg: &SearchConfig, query: &str, n: usize) -> Result<Vec<SearchResult>> {
    let key = brave_key(cfg).ok_or_else(|| anyhow!("Add a Brave Search API key in Settings → Web search"))?;
    let mut params: Vec<(&str, String)> = vec![
        ("q", query.to_string()),
        ("count", n.to_string()),
        ("safesearch", if cfg.safe_search { "moderate".into() } else { "off".into() }),
    ];
    if let Some(lang) = &cfg.region {
        params.push(("search_lang", lang.clone()));
    }
    let resp = client()
        .get("https://api.search.brave.com/res/v1/web/search")
        .query(&params)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", key.trim())
        .send()
        .await
        .map_err(|e| anyhow!("Brave Search request failed: {e}"))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED || resp.status() == reqwest::StatusCode::FORBIDDEN {
        bail!("Brave Search rejected the API key");
    }
    if !resp.status().is_success() {
        bail!("Brave Search returned HTTP {}", resp.status());
    }
    let v: Value = resp.json().await.map_err(|e| anyhow!("Invalid Brave response: {e}"))?;
    Ok(parse_brave(&v))
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

pub fn parse_brave(v: &Value) -> Vec<SearchResult> {
    v.pointer("/web/results")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    Some(SearchResult {
                        title: strip_tags(r.get("title")?.as_str()?),
                        url: r.get("url")?.as_str()?.to_string(),
                        snippet: strip_tags(r.get("description").and_then(Value::as_str).unwrap_or("")),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Formats results for the model.
pub fn format_for_model(resp: &SearchResponse) -> String {
    if resp.results.is_empty() {
        return format!("No results for \"{}\".", resp.query);
    }
    let mut s = format!("Search results for \"{}\" ({}):\n\n", resp.query, resp.engine.label());
    for (i, r) in resp.results.iter().enumerate() {
        s.push_str(&format!("[{}] {}\n{}\n", i + 1, r.title, r.url));
        if !r.snippet.is_empty() {
            s.push_str(&r.snippet);
            s.push('\n');
        }
        s.push('\n');
    }
    s.push_str("Cite sources inline as [n](url). Use fetch_url to read a page in full when snippets are not enough.");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const DDG_HTML: &str = r#"
<div class="serp__results"><div id="links" class="results">
<div class="result results_links results_links_deep result--ad">
  <div class="links_main links_deep result__body">
  <h2 class="result__title"><a rel="nofollow" class="result__a" href="https://duckduckgo.com/y.js?ad_provider=x">Sponsored thing</a></h2>
  </div>
</div>
<div class="result results_links results_links_deep web-result ">
  <div class="links_main links_deep result__body">
    <h2 class="result__title">
      <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust%2Dlang.org%2F&amp;rut=abc">Rust <b>Programming</b> Language</a>
    </h2>
    <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust%2Dlang.org%2F">A language empowering everyone to build reliable and efficient software.</a>
  </div>
</div>
<div class="result results_links results_links_deep web-result ">
  <div class="links_main links_deep result__body">
    <h2 class="result__title"><a rel="nofollow" class="result__a" href="https://doc.rust-lang.org/book/">The Rust Book</a></h2>
    <a class="result__snippet">Learn Rust.</a>
  </div>
</div>
</div></div>"#;

    #[test]
    fn parses_duckduckgo_html() {
        let r = parse_ddg_html(DDG_HTML);
        assert_eq!(r.len(), 2, "{r:?}");
        assert_eq!(r[0].title, "Rust Programming Language");
        assert_eq!(r[0].url, "https://www.rust-lang.org/");
        assert!(r[0].snippet.starts_with("A language empowering"));
        assert_eq!(r[1].url, "https://doc.rust-lang.org/book/");
    }

    #[test]
    fn parses_duckduckgo_lite() {
        let body = r#"<table>
<tr><td>1.&nbsp;</td><td><a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa" class='result-link'>Example A</a></td></tr>
<tr><td>&nbsp;</td><td class='result-snippet'>Snippet <b>A</b></td></tr>
<tr><td>2.&nbsp;</td><td><a rel="nofollow" href="https://example.org/b" class='result-link'>Example B</a></td></tr>
<tr><td>&nbsp;</td><td class='result-snippet'>Snippet B</td></tr>
</table>"#;
        let r = parse_ddg_lite(body);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].url, "https://example.com/a");
        assert_eq!(r[0].snippet, "Snippet A");
        assert_eq!(r[1].title, "Example B");
    }

    #[test]
    fn parses_searxng_and_brave() {
        let v: Value =
            serde_json::from_str(r#"{"results":[{"title":" T ","url":"https://a","content":"c"},{"title":"no url"}]}"#)
                .unwrap();
        let r = parse_searxng(&v);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "T");
        let v: Value = serde_json::from_str(
            r#"{"web":{"results":[{"title":"<strong>Rust</strong> &amp; co","url":"https://r","description":"fast <strong>and</strong> safe"}]}}"#,
        )
        .unwrap();
        let r = parse_brave(&v);
        assert_eq!(r[0].title, "Rust & co");
        assert_eq!(r[0].snippet, "fast and safe");
    }

    #[test]
    fn parses_duckduckgo_json_and_vqd() {
        let page = r#"<script>DDG.deep.initialize('/d.js?q=rust&t=D&l=us-en&s=0&dl=en&ct=GB&ss_mkt=us&vqd=4-12345678901234567890&p_ent=&ex=-1');</script>"#;
        assert_eq!(extract_vqd(page).as_deref(), Some("4-12345678901234567890"));
        assert_eq!(extract_vqd(r#"vqd="4-999_abc""#).as_deref(), Some("4-999_abc"));
        let body = r#"if (DDG.pageLayout) DDG.pageLayout.load('d',[{"a":"A <b>language</b> empowering everyone [x]","c":"https://www.rust-lang.org/","t":"Rust Programming Language","u":"https://www.rust-lang.org/"},{"a":"Ad","t":"Sponsored","u":"https://duckduckgo.com/y.js?ad"},{"n":"/d.js?q=rust&s=30"}]);DDG.duckbar.load('images');"#;
        let r = parse_ddg_json(body);
        assert_eq!(r.len(), 1, "{r:?}");
        assert_eq!(r[0].title, "Rust Programming Language");
        assert_eq!(r[0].snippet, "A language empowering everyone [x]");
    }

    #[test]
    fn parses_duckduckgo_instant_answers() {
        let v: Value = serde_json::from_str(r#"{"Heading":"Rust","AbstractText":"Rust is a language.","AbstractURL":"https://en.wikipedia.org/wiki/Rust",
            "RelatedTopics":[{"FirstURL":"https://duckduckgo.com/Cargo","Text":"Cargo - The Rust package manager"},{"Name":"More","Topics":[{"FirstURL":"https://x.y/z","Text":"Z thing"}]}]}"#).unwrap();
        let r = parse_ddg_instant(&v);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].title, "Rust");
        assert_eq!(r[1].title, "Cargo");
        assert_eq!(r[2].url, "https://x.y/z");
    }

    #[test]
    fn dedupes() {
        let mut v = vec![
            SearchResult { title: "a".into(), url: "https://x/".into(), snippet: String::new() },
            SearchResult { title: "b".into(), url: "https://x".into(), snippet: String::new() },
        ];
        dedupe(&mut v);
        assert_eq!(v.len(), 1);
    }
}

#[cfg(test)]
mod live_tests {
    /// Hits the real engine: `cargo test -p novumchat-backend live_duckduckgo -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_duckduckgo() {
        let cfg = novumchat_core::config::SearchConfig::default();
        match super::search(&cfg, "rust programming language", None).await {
            Ok(r) => {
                for x in &r.results {
                    println!("{} — {}", x.title, x.url);
                }
                assert!(!r.results.is_empty());
            }
            Err(e) => println!("search failed: {e}"),
        }
    }
}
