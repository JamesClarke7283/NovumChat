//! Shared application state.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use novumchat_core::config::AppConfig;
use novumchat_core::types::{AppInfo, Id, ModelInfo, ModelsResponse, ProviderStatus};
use parking_lot::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::approvals::Approvals;
use crate::config_store::ConfigStore;
use crate::crypto::Vault;
use crate::events::EventHub;
use crate::llm::Provider;
use crate::mcp::McpManager;
use crate::paths::AppPaths;
use crate::store::Store;

/// Opens URLs in the system browser (installed by the desktop shell).
pub type Opener = Arc<dyn Fn(&str) -> Result<()> + Send + Sync>;

pub struct Generation {
    pub chat_id: Id,
    pub cancel: CancellationToken,
}

pub struct App {
    pub paths: AppPaths,
    pub config: ConfigStore,
    pub store: Store,
    pub vault: Arc<Vault>,
    pub events: Arc<EventHub>,
    pub mcp: Arc<McpManager>,
    pub approvals: Approvals,
    pub web_mode: bool,
    providers: RwLock<HashMap<String, Arc<Provider>>>,
    provider_errors: RwLock<HashMap<String, String>>,
    models_cache: tokio::sync::Mutex<Option<(Instant, ModelsResponse)>>,
    pub(crate) generations: Mutex<HashMap<Id, Generation>>,
    /// Chats whose title is being generated.
    pub(crate) titling: Mutex<std::collections::HashSet<Id>>,
    opener: OnceLock<Opener>,
}

const MODELS_TTL: Duration = Duration::from_secs(90);

impl App {
    pub async fn init(paths: AppPaths, web_mode: bool) -> Result<Arc<Self>> {
        let config = ConfigStore::load(paths.config_file())?;
        let db = crate::db::connect(&paths.database_file()).await?;
        let vault = Arc::new(Vault::new(None));
        let store = Store::new(db, vault.clone());
        let header = store.vault_header().await?;
        vault.install(header, None);
        let recovered = store.recover_interrupted().await?;
        if recovered > 0 {
            tracing::info!("marked {recovered} interrupted replies as cancelled");
        }
        let events = Arc::new(EventHub::new());
        let mcp = Arc::new(McpManager::new(events.clone()));
        let app = Arc::new(Self {
            paths,
            config,
            store,
            vault,
            events,
            mcp,
            approvals: Approvals::default(),
            web_mode,
            providers: RwLock::new(HashMap::new()),
            provider_errors: RwLock::new(HashMap::new()),
            models_cache: tokio::sync::Mutex::new(None),
            generations: Mutex::new(HashMap::new()),
            titling: Mutex::new(std::collections::HashSet::new()),
            opener: OnceLock::new(),
        });
        app.rebuild_providers();
        let servers = app.config.with(|c| c.mcp_servers.clone());
        app.mcp.apply_config(&servers);
        Ok(app)
    }

    pub fn set_opener(&self, opener: Opener) {
        let _ = self.opener.set(opener);
    }

    pub fn open_external(&self, url: &str) -> Result<()> {
        let parsed = url::Url::parse(url).map_err(|e| anyhow!("invalid URL: {e}"))?;
        if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
            return Err(anyhow!("refusing to open `{}` links", parsed.scheme()));
        }
        match self.opener.get() {
            Some(open) => open(parsed.as_str()),
            None => Err(anyhow!("opening links is handled by the browser in web mode")),
        }
    }

    pub fn info(&self) -> AppInfo {
        let cfg = self.config.get();
        AppInfo {
            version: novumchat_core::VERSION.to_string(),
            platform: std::env::consts::OS.to_string(),
            config_path: self.config.path().display().to_string(),
            data_dir: self.paths.data_dir.display().to_string(),
            database_path: self.paths.database_file().display().to_string(),
            workspace: self.workspace(&cfg).display().to_string(),
            web_mode: self.web_mode,
            shell_available: crate::tools::shell::shell_available(),
        }
    }

    pub fn workspace(&self, cfg: &AppConfig) -> PathBuf {
        let ws = self.paths.workspace(cfg.tools.workspace.as_deref());
        let _ = std::fs::create_dir_all(&ws);
        ws
    }

    /// Recreates provider clients after a config change.
    pub fn rebuild_providers(&self) {
        let cfg = self.config.get();
        let mut providers = HashMap::new();
        let mut errors = HashMap::new();
        for p in cfg.providers.iter().filter(|p| p.enabled) {
            match Provider::new(p) {
                Ok(client) => {
                    providers.insert(p.id.clone(), Arc::new(client));
                }
                Err(e) => {
                    errors.insert(p.id.clone(), e.to_string());
                }
            }
        }
        *self.providers.write() = providers;
        *self.provider_errors.write() = errors;
        if let Ok(mut cache) = self.models_cache.try_lock() {
            *cache = None;
        }
    }

    pub fn provider(&self, id: &str) -> Result<Arc<Provider>> {
        if let Some(p) = self.providers.read().get(id) {
            return Ok(p.clone());
        }
        if let Some(e) = self.provider_errors.read().get(id) {
            return Err(anyhow!("Provider `{id}` is misconfigured: {e}"));
        }
        Err(anyhow!("Provider `{id}` is not configured or disabled"))
    }

    /// Models of every enabled provider (cached briefly).
    pub async fn list_models(&self, refresh: bool) -> ModelsResponse {
        let mut cache = self.models_cache.lock().await;
        if !refresh
            && let Some((at, resp)) = cache.as_ref()
            && at.elapsed() < MODELS_TTL
        {
            return resp.clone();
        }
        let cfg = self.config.get();
        let providers = self.providers.read().clone();
        let errors = self.provider_errors.read().clone();
        let futs = cfg.providers.iter().map(|p| {
            let client = providers.get(&p.id).cloned();
            let err = errors.get(&p.id).cloned();
            async move {
                let result: Result<Vec<ModelInfo>> = match (client, err) {
                    _ if !p.enabled => Ok(Vec::new()),
                    (Some(c), _) => match tokio::time::timeout(Duration::from_secs(25), c.list_models()).await {
                        Ok(r) => r,
                        Err(_) => Err(anyhow!("timed out")),
                    },
                    (None, Some(e)) => Err(anyhow!(e)),
                    (None, None) => Err(anyhow!("not initialised")),
                };
                (p.clone(), result)
            }
        });
        let results = futures::future::join_all(futs).await;
        let mut resp = ModelsResponse::default();
        for (p, result) in results {
            match result {
                Ok(models) => {
                    resp.providers.push(ProviderStatus {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        kind: p.kind,
                        enabled: p.enabled,
                        ok: true,
                        error: None,
                        model_count: models.len(),
                    });
                    resp.models.extend(models);
                }
                Err(e) => resp.providers.push(ProviderStatus {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    kind: p.kind,
                    enabled: p.enabled,
                    ok: false,
                    error: Some(e.to_string()),
                    model_count: 0,
                }),
            }
        }
        *cache = Some((Instant::now(), resp.clone()));
        resp
    }

    pub fn register_generation(&self, message_id: &str, chat_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        self.generations
            .lock()
            .insert(message_id.to_string(), Generation { chat_id: chat_id.to_string(), cancel: token.clone() });
        token
    }

    pub fn finish_generation(&self, message_id: &str) {
        self.generations.lock().remove(message_id);
    }

    pub fn active_generations(&self) -> usize {
        self.generations.lock().len()
    }

    /// Cancels one reply, or every reply of a chat.
    pub fn stop(&self, chat_id: &str, message_id: Option<&str>) -> usize {
        let gens = self.generations.lock();
        let mut n = 0;
        for (id, g) in gens.iter() {
            if g.chat_id == chat_id && message_id.is_none_or(|m| m == id) {
                g.cancel.cancel();
                n += 1;
            }
        }
        n
    }

    pub async fn shutdown(&self) {
        for g in self.generations.lock().values() {
            g.cancel.cancel();
        }
        self.mcp.shutdown().await;
    }
}
