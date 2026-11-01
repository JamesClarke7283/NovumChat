//! Headless web mode: `novumchat --serve [addr]` serves the UI and the same
//! RPC API over HTTP (+ Server-Sent Events), e.g. to use NovumChat from a
//! phone browser on your LAN.
//!
//! * `POST /api/rpc/<method>`: JSON params in, JSON result out.
//! * `GET /api/events`: event stream.
//! * Everything else: the bundled UI.
//!
//! Non-loopback binds require an access token (generated and stored in
//! `config.toml` on first use). Open `http://host:port/?token=…` once; the
//! token is then kept in a cookie.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::Stream;
use tokio::sync::broadcast::error::RecvError;

use crate::state::App;

/// Frontend files: embedded in the binary or served from a directory.
pub enum Assets {
    Embedded(HashMap<String, Vec<u8>>),
    Dir(PathBuf),
}

impl Assets {
    pub fn from_tauri<R: tauri::Runtime>(ctx: &tauri::Context<R>) -> Self {
        let assets = ctx.assets();
        let keys: Vec<String> = assets.iter().map(|(k, _)| k.to_string()).collect();
        let mut map = HashMap::new();
        for k in keys {
            if let Some(bytes) = assets.get(&tauri::utils::assets::AssetKey::from(k.as_str())) {
                map.insert(k.trim_start_matches('/').to_string(), bytes.into_owned());
            }
        }
        Assets::Embedded(map)
    }

    /// The file for a request path, with the name it resolved to.
    async fn get(&self, path: &str) -> Option<(String, Vec<u8>)> {
        let path = path.trim_start_matches('/');
        let path = if path.is_empty() || path.ends_with('/') { format!("{path}index.html") } else { path.to_string() };
        if path.split('/').any(|seg| seg == "..") {
            return None;
        }
        let bytes = match self {
            Assets::Embedded(map) => map.get(&path).cloned(),
            Assets::Dir(dir) => tokio::fs::read(dir.join(&path)).await.ok(),
        }?;
        Some((path, bytes))
    }
}

struct ServerState {
    app: Arc<App>,
    assets: Assets,
    token: Option<String>,
}

type Shared = Arc<ServerState>;

pub async fn serve(app: Arc<App>, addr: SocketAddr, assets: Assets) -> Result<()> {
    let mut cfg = app.config.get();
    let token = if addr.ip().is_loopback() {
        cfg.server.token.clone().filter(|t| !t.is_empty())
    } else {
        match cfg.server.token.clone().filter(|t| !t.is_empty()) {
            Some(t) => Some(t),
            None => {
                let t = uuid::Uuid::new_v4().simple().to_string();
                cfg.server.token = Some(t.clone());
                app.config.save(cfg).context("saving the generated access token")?;
                Some(t)
            }
        }
    };
    let state: Shared = Arc::new(ServerState { app, assets, token: token.clone() });
    let router = Router::new()
        .route("/api/rpc/{method}", post(rpc))
        .route("/api/events", get(events))
        .fallback(get(static_file))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await.with_context(|| format!("binding {addr}"))?;
    let shown =
        if addr.ip().is_unspecified() { format!("http://localhost:{}", addr.port()) } else { format!("http://{addr}") };
    match &token {
        Some(t) => println!("NovumChat is running at {shown}/?token={t}"),
        None => println!("NovumChat is running at {shown}"),
    }
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|c| c.trim().strip_prefix("novum_token=").map(String::from))
}

async fn auth(State(state): State<Shared>, req: Request, next: Next) -> Response {
    let Some(expected) = state.token.clone() else {
        return next.run(req).await;
    };
    let query_token = req
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")))
        .map(|t| urlencoding::decode(t).map(|c| c.into_owned()).unwrap_or_default());
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(String::from);
    let provided = bearer.or_else(|| cookie_token(req.headers())).or(query_token.clone());
    if provided.as_deref() != Some(expected.as_str()) {
        return (
            StatusCode::UNAUTHORIZED,
            "NovumChat: missing or wrong access token. Open the URL printed on startup.",
        )
            .into_response();
    }
    let mut resp = next.run(req).await;
    if query_token.is_some() {
        let cookie = format!("novum_token={expected}; Path=/; HttpOnly; SameSite=Strict; Max-Age=31536000");
        if let Ok(v) = HeaderValue::from_str(&cookie) {
            resp.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    resp
}

async fn rpc(State(state): State<Shared>, Path(method): Path<String>, body: Bytes) -> Response {
    let params: serde_json::Value = if body.is_empty() {
        serde_json::Value::Null
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return (StatusCode::BAD_REQUEST, format!("invalid JSON: {e}")).into_response(),
        }
    };
    match crate::rpc::dispatch(&state.app, &method, params).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(msg) => (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(serde_json::json!({ "error": msg }))).into_response(),
    }
}

async fn events(State(state): State<Shared>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.app.events.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(json) => yield Ok(Event::default().data(&json)),
                Err(RecvError::Lagged(_)) => yield Ok(Event::default().data(r#"{"type":"resync"}"#)),
                Err(RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn static_file(State(state): State<Shared>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    let (served, bytes) = match state.assets.get(&path).await {
        Some(found) => found,
        // SPA fallback.
        None => match state.assets.get("index.html").await {
            Some(found) => found,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    "UI assets not found. Build the frontend (trunk build) or pass --static <dir>.",
                )
                    .into_response();
            }
        },
    };
    let mime = mime_guess::from_path(&served).first_or_octet_stream();
    let mut resp = Response::new(Body::from(bytes));
    if let Ok(v) = HeaderValue::from_str(mime.as_ref()) {
        resp.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache_policy(&served)));
    resp
}

/// Files with a content hash in their name (Trunk output such as
/// `app-0123456789abcdef.css`) never change and can be cached forever;
/// everything else must be revalidated so upgrades are picked up.
fn cache_policy(path: &str) -> &'static str {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.split('.').next().unwrap_or(file).trim_end_matches("_bg");
    let hashed = stem.rsplit_once('-').is_some_and(|(_, h)| h.len() == 16 && h.chars().all(|c| c.is_ascii_hexdigit()));
    if hashed { "public, max-age=31536000, immutable" } else { "no-cache" }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cache_policy() {
        assert_eq!(super::cache_policy("app-ed841873283e1149.css"), "public, max-age=31536000, immutable");
        assert_eq!(
            super::cache_policy("novumchat-frontend-4332fbf3fe06347c_bg.wasm"),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(super::cache_policy("js/bridge.js"), "no-cache");
        assert_eq!(super::cache_policy("index.html"), "no-cache");
        assert_eq!(super::cache_policy("vendor/katex/katex.min.js"), "no-cache");
    }
}
