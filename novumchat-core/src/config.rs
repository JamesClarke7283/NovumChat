//! The `config.toml` schema.
//!
//! The file lives in the platform configuration directory (see the backend's
//! `paths` module). Every section is optional; missing values fall back to the
//! defaults below, so a hand-written config can be as small as one provider.
//!
//! MCP servers use the same layout as the OpenAI Codex CLI `config.toml`:
//!
//! ```toml
//! [mcp_servers.filesystem]
//! command = "npx"
//! args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/me/projects"]
//! env = { "LOG_LEVEL" = "info" }
//!
//! [mcp_servers.docs]
//! url = "https://example.com/mcp"
//! bearer_token_env_var = "DOCS_TOKEN"
//! http_headers = { "X-Region" = "eu" }
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::permissions::PermissionLevel;
use crate::types::ModelRef;

fn default_true() -> bool {
    true
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_true(v: &bool) -> bool {
    *v
}

/// Root of `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub appearance: AppearanceConfig,
    /// `[[providers]]`: as many OpenAI-compatible and Ollama endpoints as you like.
    pub providers: Vec<ProviderConfig>,
    pub ensemble: EnsembleConfig,
    pub tools: ToolsConfig,
    pub search: SearchConfig,
    pub server: ServerConfig,
    /// `[mcp_servers.<name>]`, Codex-compatible.
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            appearance: AppearanceConfig::default(),
            providers: vec![
                ProviderConfig {
                    id: "ollama".into(),
                    name: "Ollama".into(),
                    kind: ProviderKind::Ollama,
                    base_url: "http://localhost:11434".into(),
                    ..ProviderConfig::default()
                },
                ProviderConfig {
                    id: "openai".into(),
                    name: "OpenAI".into(),
                    kind: ProviderKind::Openai,
                    base_url: "https://api.openai.com/v1".into(),
                    api_key_env: Some("OPENAI_API_KEY".into()),
                    ..ProviderConfig::default()
                },
            ],
            ensemble: EnsembleConfig::default(),
            tools: ToolsConfig::default(),
            search: SearchConfig::default(),
            server: ServerConfig::default(),
            mcp_servers: BTreeMap::new(),
        }
    }
}

impl AppConfig {
    /// Looks up a provider by id.
    pub fn provider(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// Normalises user input: trims ids/urls, drops duplicate provider ids and
    /// fills in empty names. Returns human readable problems that were fixed or
    /// that the caller should surface.
    pub fn normalize(&mut self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut providers = Vec::with_capacity(self.providers.len());
        for mut p in std::mem::take(&mut self.providers) {
            p.id = crate::util::slugify(p.id.trim());
            if p.id.is_empty() {
                p.id = crate::util::slugify(&p.name);
            }
            if p.id.is_empty() {
                p.id = format!("provider-{}", providers.len() + 1);
            }
            if !seen.insert(p.id.clone()) {
                let base = p.id.clone();
                let mut n = 2;
                while !seen.insert(format!("{base}-{n}")) {
                    n += 1;
                }
                p.id = format!("{base}-{n}");
                problems.push(format!("Duplicate provider id `{base}` renamed to `{}`", p.id));
            }
            p.base_url = p.base_url.trim().trim_end_matches('/').to_string();
            if p.name.trim().is_empty() {
                p.name = p.id.clone();
            }
            p.models.retain(|m| !m.trim().is_empty());
            providers.push(p);
        }
        self.providers = providers;
        self.general.max_tool_rounds = self.general.max_tool_rounds.clamp(1, 100);
        self.search.max_results = self.search.max_results.clamp(1, 20);
        self.tools.bash_timeout_secs = self.tools.bash_timeout_secs.clamp(1, 3600);
        self.appearance.font_scale = self.appearance.font_scale.clamp(0.8, 1.4);
        problems
    }
}

/// `[general]`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GeneralConfig {
    /// Model used for new chats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<ModelRef>,
    /// Default system prompt for new chats (empty = none). `{date}` is replaced
    /// with today's date.
    pub system_prompt: String,
    /// Enter sends, Shift+Enter inserts a newline (desktop only).
    pub send_on_enter: bool,
    /// Generate a short title after the first exchange.
    pub auto_title: bool,
    /// Model used for titles; defaults to the chat's model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_model: Option<ModelRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Upper bound on model ↔ tool round trips for one reply.
    pub max_tool_rounds: u32,
    /// Ask models that support it to think before answering ("low" | "medium" | "high").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            default_model: None,
            system_prompt: "You are NovumChat, a helpful, precise assistant. Today is {date}. \
Use Markdown. Write math as LaTeX ($...$ inline, $$...$$ display). When asked for an \
interactive demo, app or visualisation, reply with one self-contained ```html code block \
(inline CSS/JS) — it is rendered live in a sandbox."
                .into(),
            send_on_enter: true,
            auto_title: true,
            title_model: None,
            temperature: None,
            max_tokens: None,
            max_tool_rounds: 16,
            reasoning_effort: None,
        }
    }
}

/// `[appearance]`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppearanceConfig {
    pub theme: ThemeMode,
    /// Accent palette: "aurora" | "ocean" | "sunset" | "forest" | "rose" | "mono".
    pub accent: String,
    /// UI text scale (0.8 – 1.4).
    pub font_scale: f32,
    /// Show the reasoning ("thinking") panel expanded while streaming.
    pub expand_reasoning: bool,
    /// Render ```html blocks as live, sandboxed previews.
    pub live_previews: bool,
    /// Load images from remote URLs in replies. Off by default because remote
    /// image URLs can leak conversation data (click a blocked image to load it).
    pub load_remote_images: bool,
    /// Animated background and streaming effects.
    pub animations: bool,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Auto,
            accent: "aurora".into(),
            font_scale: 1.0,
            expand_reasoning: false,
            live_previews: true,
            load_remote_images: false,
            animations: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Auto,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::Auto => "auto",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Any OpenAI-compatible `/v1/chat/completions` endpoint (OpenAI, OpenRouter,
    /// LM Studio, vLLM, llama.cpp, Groq, Together, DeepSeek, …).
    #[default]
    Openai,
    /// Native Ollama API (`/api/chat`).
    Ollama,
}

impl ProviderKind {
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Openai => "OpenAI-compatible",
            ProviderKind::Ollama => "Ollama",
        }
    }
}

/// `[[providers]]`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProviderConfig {
    /// Unique, stable identifier (lowercase, no spaces). Referenced by model selections.
    pub id: String,
    /// Display name.
    pub name: String,
    pub kind: ProviderKind,
    /// e.g. `https://api.openai.com/v1` or `http://localhost:11434`.
    pub base_url: String,
    /// API key stored in the config file (prefer `api_key_env`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Name of an environment variable holding the API key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Extra models to list (in addition to discovered ones).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// Query the endpoint for its model list.
    #[serde(skip_serializing_if = "is_true")]
    pub discover_models: bool,
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Extra HTTP headers sent with every request.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: ProviderKind::Openai,
            base_url: String::new(),
            api_key: None,
            api_key_env: None,
            models: Vec::new(),
            discover_models: true,
            enabled: true,
            headers: BTreeMap::new(),
        }
    }
}

/// `[ensemble]`: ask several models at once.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EnsembleConfig {
    /// Models pre-selected when ensemble mode is switched on.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub default_models: Vec<ModelRef>,
    /// Automatically merge the responses into one answer.
    pub synthesize: bool,
    /// Model that writes the merged answer (defaults to the first ensemble model).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synthesizer: Option<ModelRef>,
    pub synthesis_prompt: String,
}

impl Default for EnsembleConfig {
    fn default() -> Self {
        Self {
            default_models: Vec::new(),
            synthesize: false,
            synthesizer: None,
            synthesis_prompt: "You are given a user request and several candidate answers written by \
different AI models. Write the single best possible answer: combine their strengths, fix mistakes, \
resolve disagreements explicitly and keep the formatting clean. Do not mention the candidates \
unless they disagree on something important."
                .into(),
        }
    }
}

/// `[tools]`: built-in agent tools and their permissions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ToolsConfig {
    /// Offer tools to models at all.
    pub enabled: bool,
    /// "read_only" | "ask" (default) | "auto_edit" | "yolo".
    pub permission: PermissionLevel,
    /// Working directory for file and shell tools (default: `~/NovumChat`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Shell used by the `bash` tool (default: bash, falling back to sh / PowerShell).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    pub bash_timeout_secs: u64,
    /// Truncate tool output sent back to the model beyond this many bytes.
    pub max_output_bytes: usize,
    /// Command prefixes that never need approval (e.g. "git status").
    pub allowed_commands: Vec<String>,
    /// Substrings that are always refused, even in YOLO mode.
    pub denied_commands: Vec<String>,
    /// Built-in tools to hide from models (e.g. ["bash"]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub disabled_tools: Vec<String>,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            permission: PermissionLevel::Ask,
            workspace: None,
            shell: None,
            bash_timeout_secs: 120,
            max_output_bytes: 32_000,
            allowed_commands: [
                "ls",
                "pwd",
                "cat",
                "head",
                "tail",
                "wc",
                "echo",
                "date",
                "whoami",
                "uname",
                "which",
                "file",
                "stat",
                "du -sh",
                "tree",
                "grep",
                "rg",
                "find",
                "git status",
                "git log",
                "git diff",
                "git show",
                "git branch",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            denied_commands: [
                "rm -rf /",
                "rm -rf ~",
                "rm -rf ~/",
                "rm -rf /*",
                "mkfs",
                ":(){ :|:& };:",
                "dd if=/dev/zero of=/dev",
                "> /dev/sda",
                "chmod -R 777 /",
                "shutdown",
                "reboot",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            disabled_tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Hash)]
#[serde(rename_all = "lowercase")]
pub enum SearchEngine {
    /// No API key required.
    #[default]
    Duckduckgo,
    /// Self-hosted SearXNG (JSON output must be enabled on the instance).
    Searxng,
    /// Brave Search API (needs an API key).
    Brave,
}

impl SearchEngine {
    pub fn label(self) -> &'static str {
        match self {
            SearchEngine::Duckduckgo => "DuckDuckGo",
            SearchEngine::Searxng => "SearXNG",
            SearchEngine::Brave => "Brave Search",
        }
    }
}

/// `[search]`: internet search used by the `web_search` tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SearchConfig {
    pub engine: SearchEngine,
    /// Base URL of your SearXNG instance, e.g. `http://localhost:8888`.
    pub searxng_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brave_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brave_api_key_env: Option<String>,
    pub max_results: usize,
    /// Region / language hint such as "us-en" (DuckDuckGo) or "en" (SearXNG/Brave).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub safe_search: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            engine: SearchEngine::Duckduckgo,
            searxng_url: "http://localhost:8888".into(),
            brave_api_key: None,
            brave_api_key_env: Some("BRAVE_API_KEY".into()),
            max_results: 6,
            region: None,
            safe_search: true,
        }
    }
}

/// `[server]`: headless web mode (`novumchat --serve`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    /// Access token required by the web UI/API. Generated on first use when
    /// binding to a non-loopback address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { bind: "127.0.0.1:8484".into(), token: None }
    }
}

/// `[mcp_servers.<name>]`, using the same keys as the Codex CLI.
///
/// Either `command` (stdio transport) or `url` (streamable HTTP) must be set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct McpServerConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Environment variables forwarded from NovumChat's own environment.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env_vars: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bearer_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bearer_token_env_var: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub http_headers: BTreeMap<String, String>,
    /// Header name → environment variable holding its value.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env_http_headers: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_timeout_sec: Option<f64>,
    /// Legacy Codex key, still accepted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_timeout_sec: Option<f64>,
    /// Only expose these tools (allow-list).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_tools: Option<Vec<String>>,
    /// Hide these tools.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub disabled_tools: Vec<String>,
    /// NovumChat extension: run this server's tools without asking (unless the
    /// permission level is read-only).
    #[serde(skip_serializing_if = "is_false")]
    pub trusted: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            env_vars: Vec::new(),
            cwd: None,
            url: None,
            bearer_token: None,
            bearer_token_env_var: None,
            http_headers: BTreeMap::new(),
            env_http_headers: BTreeMap::new(),
            enabled: default_true(),
            startup_timeout_sec: None,
            startup_timeout_ms: None,
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: Vec::new(),
            trusted: false,
        }
    }
}

impl McpServerConfig {
    pub fn transport_label(&self) -> &'static str {
        if self.url.is_some() { "http" } else { "stdio" }
    }

    pub fn startup_timeout(&self) -> std::time::Duration {
        if let Some(s) = self.startup_timeout_sec {
            return std::time::Duration::from_secs_f64(s.clamp(1.0, 600.0));
        }
        if let Some(ms) = self.startup_timeout_ms {
            return std::time::Duration::from_millis(ms.clamp(1000, 600_000));
        }
        std::time::Duration::from_secs(30)
    }

    pub fn tool_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.tool_timeout_sec.unwrap_or(120.0).clamp(1.0, 3600.0))
    }

    /// Whether a tool reported by the server should be exposed to models.
    pub fn tool_allowed(&self, tool: &str) -> bool {
        if self.disabled_tools.iter().any(|t| t == tool) {
            return false;
        }
        match &self.enabled_tools {
            Some(list) => list.iter().any(|t| t == tool),
            None => true,
        }
    }
}

/// The commented template written on first launch.
pub const DEFAULT_CONFIG_TOML: &str = r#"# ─────────────────────────────────────────────────────────────────────────────
#  NovumChat configuration
#  Edit here or in Settings inside the app (saving from the app rewrites this file).
# ─────────────────────────────────────────────────────────────────────────────

[general]
# default_model = { provider = "ollama", model = "llama3.2" }
send_on_enter = true
auto_title = true
max_tool_rounds = 16
# temperature = 0.7
# max_tokens = 4096
# reasoning_effort = "medium"   # low | medium | high (models that support thinking)

[appearance]
theme = "auto"            # auto | light | dark
accent = "aurora"         # aurora | ocean | sunset | forest | rose | mono
font_scale = 1.0
expand_reasoning = false
live_previews = true      # render ```html replies as live sandboxed apps
load_remote_images = false
animations = true

# ── Providers ───────────────────────────────────────────────────────────────
# Add as many as you like. kind = "openai" works with any OpenAI-compatible
# endpoint; kind = "ollama" uses the native Ollama API.

[[providers]]
id = "ollama"
name = "Ollama"
kind = "ollama"
base_url = "http://localhost:11434"

[[providers]]
id = "openai"
name = "OpenAI"
kind = "openai"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

# [[providers]]
# id = "openrouter"
# name = "OpenRouter"
# kind = "openai"
# base_url = "https://openrouter.ai/api/v1"
# api_key_env = "OPENROUTER_API_KEY"
#
# [[providers]]
# id = "lmstudio"
# name = "LM Studio"
# kind = "openai"
# base_url = "http://localhost:1234/v1"

# ── Ensemble: ask several models at once ────────────────────────────────────
[ensemble]
synthesize = false
# synthesizer = { provider = "openai", model = "gpt-4.1" }
# default_models = [
#   { provider = "ollama", model = "llama3.2" },
#   { provider = "openai", model = "gpt-4.1-mini" },
# ]

# ── Tools & permissions ─────────────────────────────────────────────────────
# permission = "read_only" : models may only read files and search the web
# permission = "ask"       : (default) anything that changes your system asks first
# permission = "auto_edit" : file edits inside the workspace run without asking
# permission = "yolo"      : trust everything (denied_commands still apply)
[tools]
enabled = true
permission = "ask"
# workspace = "/home/me/projects"
bash_timeout_secs = 120
max_output_bytes = 32000

# ── Internet search ─────────────────────────────────────────────────────────
[search]
engine = "duckduckgo"     # duckduckgo (no key) | searxng | brave
searxng_url = "http://localhost:8888"
brave_api_key_env = "BRAVE_API_KEY"
max_results = 6
safe_search = true

# ── Headless web mode: `novumchat --serve` ──────────────────────────────────
[server]
bind = "127.0.0.1:8484"

# ── MCP servers (same format as the OpenAI Codex config.toml) ───────────────
# [mcp_servers.filesystem]
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/me/projects"]
#
# [mcp_servers.context7]
# url = "https://mcp.context7.com/mcp"
# http_headers = { "CONTEXT7_API_KEY" = "..." }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_parses_and_matches_defaults() {
        let parsed: AppConfig = toml::from_str(DEFAULT_CONFIG_TOML).expect("template parses");
        let defaults = AppConfig::default();
        assert_eq!(parsed.providers, defaults.providers);
        assert_eq!(parsed.tools.permission, PermissionLevel::Ask);
        assert_eq!(parsed.search.engine, SearchEngine::Duckduckgo);
        assert_eq!(parsed.appearance, defaults.appearance);
        assert!(parsed.mcp_servers.is_empty());
    }

    #[test]
    fn codex_style_mcp_servers_parse() {
        let src = r#"
[mcp_servers.node_repl]
args = []
command = "node"
env_vars = ["A", "B"]
startup_timeout_sec = 120

[mcp_servers.node_repl.env]
NODE_PATH = "/usr/lib/node"

[mcp_servers.context7]
url = "https://mcp.context7.com/mcp"

[mcp_servers.context7.http_headers]
Authorization = "Bearer abc"

[mcp_servers.blender]
command = "uv"
args = ["--directory", "/tmp/x", "run", "blender-mcp"]
enabled = false
disabled_tools = ["dangerous"]
"#;
        let cfg: AppConfig = toml::from_str(src).unwrap();
        assert_eq!(cfg.mcp_servers.len(), 3);
        let repl = &cfg.mcp_servers["node_repl"];
        assert_eq!(repl.command.as_deref(), Some("node"));
        assert_eq!(repl.env_vars, vec!["A", "B"]);
        assert_eq!(repl.env["NODE_PATH"], "/usr/lib/node");
        assert_eq!(repl.startup_timeout().as_secs(), 120);
        assert!(repl.enabled);
        let c7 = &cfg.mcp_servers["context7"];
        assert_eq!(c7.transport_label(), "http");
        assert_eq!(c7.http_headers["Authorization"], "Bearer abc");
        let blender = &cfg.mcp_servers["blender"];
        assert!(!blender.enabled);
        assert!(!blender.tool_allowed("dangerous"));
        assert!(blender.tool_allowed("other"));
        // Providers fall back to defaults when absent.
        assert_eq!(cfg.providers.len(), 2);
    }

    #[test]
    fn config_round_trips_through_toml() {
        let mut cfg = AppConfig::default();
        cfg.general.default_model = Some(ModelRef::new("ollama", "qwen3:8b"));
        cfg.ensemble.default_models =
            vec![ModelRef::new("ollama", "llama3.2"), ModelRef::new("openai", "gpt-4.1-mini")];
        cfg.mcp_servers.insert(
            "fs".into(),
            McpServerConfig {
                command: Some("npx".into()),
                args: vec!["-y".into(), "server".into()],
                ..Default::default()
            },
        );
        let text = toml::to_string_pretty(&cfg).expect("serialises");
        let back: AppConfig = toml::from_str(&text).expect("parses back");
        assert_eq!(back, cfg);
    }

    #[test]
    fn normalize_dedupes_provider_ids() {
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "Ollama".into(),
            name: "Second".into(),
            kind: ProviderKind::Ollama,
            base_url: "http://box:11434/".into(),
            ..Default::default()
        });
        let problems = cfg.normalize();
        assert_eq!(problems.len(), 1);
        assert_eq!(cfg.providers[2].id, "ollama-2");
        assert_eq!(cfg.providers[2].base_url, "http://box:11434");
    }
}
