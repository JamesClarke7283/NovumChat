//! Model Context Protocol client manager (official `rmcp` SDK).
//!
//! Servers come from `[mcp_servers.<name>]` in `config.toml` (Codex format):
//! `command`/`args`/`env` start a stdio server, `url` connects over
//! streamable HTTP. Their tools are offered to models as
//! `mcp__<server>__<tool>`.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use novumchat_core::config::McpServerConfig;
use novumchat_core::events::AppEvent;
use novumchat_core::types::{McpServerStatus, McpState, McpToolInfo};
use novumchat_core::util::sanitize_tool_name;
use parking_lot::{Mutex, RwLock};
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio_util::sync::CancellationToken;

use crate::events::EventHub;
use crate::llm::ToolSpec;
use crate::tools::ToolOutput;

type Client = RunningService<RoleClient, ()>;

#[derive(Debug, Clone)]
pub struct McpTool {
    /// Name exposed to models.
    pub qualified: String,
    pub server: String,
    pub tool: String,
    pub description: String,
    pub schema: Value,
    pub read_only: bool,
}

struct Server {
    config: McpServerConfig,
    status: McpServerStatus,
    tools: Vec<McpTool>,
    client: Option<Arc<Client>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    /// Bumped on every (re)connect so stale connection tasks can't overwrite state.
    generation: u64,
}

pub struct McpManager {
    servers: RwLock<BTreeMap<String, Server>>,
    events: Arc<EventHub>,
}

pub fn qualified_name(server: &str, tool: &str) -> String {
    let name = format!("mcp__{}__{}", sanitize_tool_name(server), sanitize_tool_name(tool));
    if name.len() <= 64 {
        return name;
    }
    // Keep names unique when truncating.
    let hash = name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    format!("{}_{hash:08x}", &name[..55])
}

impl McpManager {
    pub fn new(events: Arc<EventHub>) -> Self {
        Self { servers: RwLock::new(BTreeMap::new()), events }
    }

    pub fn statuses(&self) -> Vec<McpServerStatus> {
        self.servers.read().values().map(|s| s.status.clone()).collect()
    }

    fn emit_status(&self) {
        self.events.emit(AppEvent::McpStatus { servers: self.statuses() });
    }

    /// Tools of all connected servers.
    pub fn tools(&self) -> Vec<McpTool> {
        self.servers.read().values().filter(|s| s.client.is_some()).flat_map(|s| s.tools.clone()).collect()
    }

    pub fn find_tool(&self, qualified: &str) -> Option<(McpTool, McpServerConfig)> {
        let servers = self.servers.read();
        servers
            .values()
            .find_map(|s| s.tools.iter().find(|t| t.qualified == qualified).map(|t| (t.clone(), s.config.clone())))
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools()
            .into_iter()
            .map(|t| ToolSpec {
                name: t.qualified.clone(),
                description: if t.description.is_empty() {
                    format!("Tool `{}` from MCP server `{}`", t.tool, t.server)
                } else {
                    format!("{} (MCP server: {})", t.description, t.server)
                },
                parameters: t.schema,
            })
            .collect()
    }

    /// Applies a new configuration: connects new/changed servers, drops removed ones.
    pub fn apply_config(self: &Arc<Self>, configs: &BTreeMap<String, McpServerConfig>) {
        let mut to_start = Vec::new();
        {
            let mut servers = self.servers.write();
            servers.retain(|name, _| configs.contains_key(name));
            for (name, cfg) in configs {
                let changed = servers.get(name).is_none_or(|s| &s.config != cfg);
                if changed {
                    let generation = servers.get(name).map(|s| s.generation + 1).unwrap_or(1);
                    servers.insert(name.clone(), Server::new(name, cfg.clone(), generation));
                    if cfg.enabled {
                        to_start.push((name.clone(), generation));
                    }
                }
            }
        }
        self.emit_status();
        for (name, generation) in to_start {
            let me = self.clone();
            tokio::spawn(async move { me.connect(name, generation).await });
        }
    }

    /// Reconnects one server (or all).
    pub fn restart(self: &Arc<Self>, name: Option<&str>) {
        let mut to_start = Vec::new();
        {
            let mut servers = self.servers.write();
            for (n, s) in servers.iter_mut() {
                if name.is_some_and(|x| x != n) {
                    continue;
                }
                let cfg = s.config.clone();
                let generation = s.generation + 1;
                *s = Server::new(n, cfg.clone(), generation);
                if cfg.enabled {
                    to_start.push((n.clone(), generation));
                }
            }
        }
        self.emit_status();
        for (n, generation) in to_start {
            let me = self.clone();
            tokio::spawn(async move { me.connect(n, generation).await });
        }
    }

    async fn connect(self: Arc<Self>, name: String, generation: u64) {
        let Some((cfg, stderr)) = self
            .servers
            .read()
            .get(&name)
            .filter(|s| s.generation == generation)
            .map(|s| (s.config.clone(), s.stderr.clone()))
        else {
            return;
        };
        let timeout = cfg.startup_timeout();
        let result = tokio::time::timeout(timeout, start_client(&cfg, stderr.clone())).await;
        let outcome: Result<(Client, Vec<rmcp::model::Tool>)> = match result {
            Err(_) => Err(anyhow!("timed out after {}s while starting", timeout.as_secs())),
            Ok(Err(e)) => Err(e),
            Ok(Ok(client)) => match tokio::time::timeout(timeout, client.list_all_tools()).await {
                Ok(Ok(tools)) => Ok((client, tools)),
                Ok(Err(e)) => Err(anyhow!("listing tools failed: {e}")),
                Err(_) => Err(anyhow!("timed out listing tools")),
            },
        };
        {
            let mut servers = self.servers.write();
            let Some(server) = servers.get_mut(&name).filter(|s| s.generation == generation) else {
                return;
            };
            match outcome {
                Ok((client, tools)) => {
                    let info = client.peer_info().and_then(|i| {
                        let v = serde_json::to_value(&*i).ok()?;
                        let n = v.pointer("/serverInfo/name")?.as_str()?.to_string();
                        let ver = v.pointer("/serverInfo/version").and_then(Value::as_str).unwrap_or("");
                        Some(if ver.is_empty() { n } else { format!("{n} {ver}") })
                    });
                    let mut list = Vec::new();
                    for t in tools {
                        if !cfg.tool_allowed(&t.name) {
                            continue;
                        }
                        let mut schema = Value::Object((*t.input_schema).clone());
                        if schema.get("type").is_none() {
                            schema["type"] = Value::String("object".into());
                        }
                        list.push(McpTool {
                            qualified: qualified_name(&name, &t.name),
                            server: name.clone(),
                            tool: t.name.to_string(),
                            description: t.description.as_deref().unwrap_or("").to_string(),
                            schema,
                            read_only: t.annotations.as_ref().and_then(|a| a.read_only_hint).unwrap_or(false),
                        });
                    }
                    server.status.state = McpState::Connected;
                    server.status.error = None;
                    server.status.server_info = info;
                    server.status.tools = list
                        .iter()
                        .map(|t| McpToolInfo {
                            name: t.tool.clone(),
                            description: t.description.clone(),
                            read_only: t.read_only,
                        })
                        .collect();
                    server.tools = list;
                    server.client = Some(Arc::new(client));
                    tracing::info!("MCP server `{name}` connected with {} tools", server.tools.len());
                }
                Err(e) => {
                    let tail: Vec<String> = stderr.lock().iter().rev().take(6).rev().cloned().collect();
                    let mut msg = e.to_string();
                    if !tail.is_empty() {
                        msg.push('\n');
                        msg.push_str(&tail.join("\n"));
                    }
                    tracing::warn!("MCP server `{name}` failed: {msg}");
                    server.status.state = McpState::Failed;
                    server.status.error = Some(msg);
                }
            }
        }
        self.emit_status();
    }

    pub async fn call(&self, qualified: &str, args: &Value, cancel: &CancellationToken) -> ToolOutput {
        let found = {
            let servers = self.servers.read();
            servers.values().find_map(|s| {
                let t = s.tools.iter().find(|t| t.qualified == qualified)?;
                Some((t.clone(), s.client.clone(), s.config.tool_timeout()))
            })
        };
        let Some((tool, Some(client), timeout)) = found else {
            return ToolOutput::error(format!("MCP tool `{qualified}` is not available (server disconnected?)"));
        };
        let arguments = match args {
            Value::Object(map) => map.clone(),
            Value::Null => serde_json::Map::new(),
            other => {
                let mut m = serde_json::Map::new();
                m.insert("input".into(), other.clone());
                m
            }
        };
        let params = CallToolRequestParams::new(tool.tool.clone()).with_arguments(arguments);
        let call = client.call_tool(params);
        let result = tokio::select! {
            _ = cancel.cancelled() => return ToolOutput::error("Cancelled"),
            r = tokio::time::timeout(timeout, call) => r,
        };
        match result {
            Err(_) => ToolOutput::error(format!("MCP tool timed out after {}s", timeout.as_secs())),
            Ok(Err(e)) => ToolOutput::error(format!("MCP error: {e}")),
            Ok(Ok(res)) => {
                let v = serde_json::to_value(&res).unwrap_or(Value::Null);
                let mut out = ToolOutput::text(render_content(&v));
                out.is_error = v.get("isError").and_then(Value::as_bool).unwrap_or(false);
                out.data = Some(v);
                out
            }
        }
    }

    pub async fn shutdown(&self) {
        let clients: Vec<Arc<Client>> = self.servers.write().values_mut().filter_map(|s| s.client.take()).collect();
        for c in clients {
            if let Ok(c) = Arc::try_unwrap(c) {
                let _ = c.cancel().await;
            }
        }
    }
}

impl Server {
    fn new(name: &str, config: McpServerConfig, generation: u64) -> Self {
        let state = if config.enabled { McpState::Connecting } else { McpState::Disabled };
        Self {
            status: McpServerStatus {
                name: name.to_string(),
                transport: config.transport_label().into(),
                state,
                error: None,
                server_info: None,
                tools: Vec::new(),
            },
            config,
            tools: Vec::new(),
            client: None,
            stderr: Arc::new(Mutex::new(VecDeque::new())),
            generation,
        }
    }
}

fn resolve_env(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|v| !v.is_empty())
}

async fn start_client(cfg: &McpServerConfig, stderr_buf: Arc<Mutex<VecDeque<String>>>) -> Result<Client> {
    if let Some(url) = cfg.url.as_deref().filter(|u| !u.trim().is_empty()) {
        let mut tc = StreamableHttpClientTransportConfig::with_uri(url.trim().to_string());
        tc.auth_header = cfg
            .bearer_token
            .clone()
            .filter(|t| !t.is_empty())
            .or_else(|| cfg.bearer_token_env_var.as_deref().and_then(resolve_env));
        let mut headers = HashMap::new();
        let all = cfg
            .http_headers
            .iter()
            .map(|(k, v)| (k.clone(), Some(v.clone())))
            .chain(cfg.env_http_headers.iter().map(|(k, var)| (k.clone(), resolve_env(var))));
        for (k, v) in all {
            let Some(v) = v else { continue };
            let name = reqwest::header::HeaderName::from_bytes(k.trim().as_bytes())
                .map_err(|e| anyhow!("invalid header `{k}`: {e}"))?;
            let value =
                reqwest::header::HeaderValue::from_str(v.trim()).map_err(|e| anyhow!("invalid header `{k}`: {e}"))?;
            headers.insert(name, value);
        }
        let tc = tc.custom_headers(headers);
        let transport = StreamableHttpClientTransport::from_config(tc);
        return ().serve(transport).await.map_err(|e| anyhow!("{e}"));
    }
    let Some(command) = cfg.command.as_deref().filter(|c| !c.trim().is_empty()) else {
        return Err(anyhow!("set either `command` or `url`"));
    };
    if !crate::tools::shell::shell_available() {
        return Err(anyhow!("stdio MCP servers are not supported on this platform; use `url`"));
    }
    let mut cmd = tokio::process::Command::new(command.trim());
    cmd.args(&cfg.args).env("PATH", crate::tools::shell::augmented_path());
    for var in &cfg.env_vars {
        if let Some(v) = std::env::var_os(var) {
            cmd.env(var, v);
        }
    }
    cmd.envs(&cfg.env);
    if let Some(cwd) = cfg.cwd.as_deref().filter(|c| !c.is_empty()) {
        cmd.current_dir(crate::paths::expand_tilde(cwd));
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let (transport, stderr) = TokioChildProcess::builder(cmd)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("could not start `{command}`: {e}"))?;
    if let Some(stderr) = stderr {
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut buf = stderr_buf.lock();
                buf.push_back(novumchat_core::util::truncate_chars(&line, 400));
                while buf.len() > 50 {
                    buf.pop_front();
                }
            }
        });
    }
    ().serve(transport).await.map_err(|e| anyhow!("{e}"))
}

/// Flattens MCP content blocks into text for the model.
pub fn render_content(result: &Value) -> String {
    let mut out = Vec::new();
    if let Some(items) = result.get("content").and_then(Value::as_array) {
        for item in items {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => out.push(item.get("text").and_then(Value::as_str).unwrap_or("").to_string()),
                Some("image") => {
                    out.push(format!("[image: {}]", item.get("mimeType").and_then(Value::as_str).unwrap_or("image")))
                }
                Some("audio") => out.push("[audio]".into()),
                Some("resource") => {
                    let r = item.get("resource").cloned().unwrap_or(Value::Null);
                    let uri = r.get("uri").and_then(Value::as_str).unwrap_or("");
                    match r.get("text").and_then(Value::as_str) {
                        Some(t) => out.push(format!("[resource {uri}]\n{t}")),
                        None => out.push(format!("[binary resource {uri}]")),
                    }
                }
                Some("resource_link") => {
                    out.push(format!("[resource link: {}]", item.get("uri").and_then(Value::as_str).unwrap_or("")))
                }
                _ => out.push(item.to_string()),
            }
        }
    }
    if out.is_empty() {
        if let Some(s) = result.get("structuredContent") {
            return s.to_string();
        }
        return "(no content)".into();
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names() {
        assert_eq!(qualified_name("github", "search_repos"), "mcp__github__search_repos");
        assert_eq!(qualified_name("my server", "do.it"), "mcp__my_server__do_it");
        let long = qualified_name(&"s".repeat(40), &"t".repeat(40));
        assert!(long.len() <= 64);
        assert_ne!(long, qualified_name(&"s".repeat(40), &"u".repeat(40)));
    }

    #[test]
    fn content_rendering() {
        let v = json!({"content": [
            {"type": "text", "text": "hello"},
            {"type": "image", "data": "AAA", "mimeType": "image/png"},
            {"type": "resource", "resource": {"uri": "file:///a", "text": "body"}}
        ]});
        assert_eq!(render_content(&v), "hello\n[image: image/png]\n[resource file:///a]\nbody");
        assert_eq!(render_content(&json!({"content": [], "structuredContent": {"a": 1}})), "{\"a\":1}");
    }

    #[tokio::test]
    async fn failed_server_reports_error() {
        let hub = Arc::new(EventHub::new());
        let mgr = Arc::new(McpManager::new(hub));
        let mut cfgs = BTreeMap::new();
        cfgs.insert(
            "broken".to_string(),
            McpServerConfig {
                command: Some("/definitely/not/a/real/binary".into()),
                startup_timeout_sec: Some(5.0),
                ..Default::default()
            },
        );
        cfgs.insert(
            "off".to_string(),
            McpServerConfig { command: Some("x".into()), enabled: false, ..Default::default() },
        );
        mgr.apply_config(&cfgs);
        for _ in 0..50 {
            if mgr.statuses().iter().any(|s| s.state == McpState::Failed) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let st = mgr.statuses();
        let broken = st.iter().find(|s| s.name == "broken").unwrap();
        assert_eq!(broken.state, McpState::Failed);
        assert!(broken.error.as_deref().unwrap().contains("could not start"));
        assert_eq!(st.iter().find(|s| s.name == "off").unwrap().state, McpState::Disabled);
        assert!(mgr.tools().is_empty());
    }
}
