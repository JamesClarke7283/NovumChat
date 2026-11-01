//! NovumChat backend: the Tauri desktop/mobile shell plus everything behind
//! the UI — providers, storage, encryption, tools, MCP and web search.

pub mod approvals;
pub mod attachments;
pub mod config_store;
pub mod crypto;
pub mod db;
pub mod engine;
pub mod events;
pub mod llm;
pub mod mcp;
pub mod paths;
pub mod rpc;
pub mod search;
pub mod state;
pub mod store;
pub mod tools;

#[cfg(all(desktop, feature = "web-server"))]
pub mod server;

use std::sync::Arc;

use novumchat_core::EVENT_CHANNEL;
use novumchat_core::events::AppEvent;
use tauri::{Emitter, Manager};

use crate::events::EventSink;
use crate::paths::AppPaths;
use crate::state::App;

/// The application, or the reason it failed to start (shown by the UI).
type AppSlot = Result<Arc<App>, String>;

struct TauriSink(tauri::AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: &AppEvent) {
        if let Err(e) = self.0.emit(EVENT_CHANNEL, event) {
            tracing::warn!("emitting event failed: {e}");
        }
    }
}

/// The single IPC entry point used by the UI.
#[tauri::command]
async fn rpc(
    state: tauri::State<'_, AppSlot>,
    method: String,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    tracing::debug!(%method, "ipc");
    match state.inner() {
        Ok(app) => crate::rpc::dispatch(app, &method, params).await,
        Err(e) => Err(format!("NovumChat failed to start: {e}")),
    }
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,novumchat_backend_lib=info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_target(false).try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    let context = tauri::generate_context!();

    #[cfg(all(desktop, feature = "web-server"))]
    {
        match ServeArgs::parse() {
            Ok(Some(args)) => {
                if let Err(e) = run_server(&context, args) {
                    eprintln!("error: {e:#}");
                    std::process::exit(1);
                }
                return;
            }
            Ok(None) => {}
            Err(msg) => {
                eprintln!("{msg}");
                std::process::exit(if msg.starts_with("NovumChat") { 0 } else { 2 });
            }
        }
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let fallback = mobile_paths(&handle);
            let slot: AppSlot = tauri::async_runtime::block_on(async move {
                let paths = AppPaths::resolve(fallback)?;
                tracing::info!("config: {}", paths.config_file().display());
                tracing::info!("database: {}", paths.database_file().display());
                App::init(paths, false).await
            })
            .map_err(|e| format!("{e:#}"));
            match &slot {
                Ok(state) => {
                    state.events.set_sink(Arc::new(TauriSink(handle.clone())));
                    // Debug builds: `NOVUMCHAT_SELFTEST_EVENTS=1` emits an event a few seconds
                    // after start; the UI answers by re-reading the config over IPC, which
                    // proves the event → webview → IPC round trip without GUI automation.
                    #[cfg(debug_assertions)]
                    if std::env::var_os("NOVUMCHAT_SELFTEST_EVENTS").is_some() {
                        let events = state.events.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                            tracing::info!("selftest: emitting ConfigChanged");
                            events.emit(AppEvent::ConfigChanged);
                        });
                    }
                    let opener_handle = handle.clone();
                    state.set_opener(Arc::new(move |url: &str| {
                        use tauri_plugin_opener::OpenerExt;
                        opener_handle
                            .opener()
                            .open_url(url, None::<&str>)
                            .map_err(|e| anyhow::anyhow!("could not open link: {e}"))
                    }));
                }
                Err(e) => tracing::error!("startup failed: {e}"),
            }
            app.manage(slot);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![rpc])
        .build(context)
        .expect("error while building the NovumChat application");

    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event
            && let Some(Ok(state)) = handle.try_state::<AppSlot>().map(|s| s.inner().clone())
        {
            tauri::async_runtime::block_on(state.shutdown());
        }
    });
}

/// Sandbox directories on Android/iOS, where `directories` has no answer.
fn mobile_paths(handle: &tauri::AppHandle) -> Option<AppPaths> {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        let p = handle.path();
        let data = p.app_data_dir().ok()?;
        Some(AppPaths {
            config_dir: p.app_config_dir().unwrap_or_else(|_| data.join("config")),
            cache_dir: p.app_cache_dir().unwrap_or_else(|_| data.join("cache")),
            default_workspace: data.join("workspace"),
            data_dir: data,
        })
    } else {
        None
    }
}

#[cfg(all(desktop, feature = "web-server"))]
struct ServeArgs {
    addr: Option<String>,
    static_dir: Option<std::path::PathBuf>,
}

#[cfg(all(desktop, feature = "web-server"))]
impl ServeArgs {
    const USAGE: &'static str = "NovumChat — multi-model AI chat\n\n\
Usage:\n  novumchat                       start the desktop app\n  \
novumchat --serve [ADDR]        serve the UI over HTTP (default ADDR from config, 127.0.0.1:8484)\n  \
novumchat --serve --static DIR  serve UI files from DIR instead of the bundled copy\n\n\
Environment: NOVUMCHAT_CONFIG_DIR, NOVUMCHAT_DATA_DIR override storage locations; RUST_LOG sets logging.";

    fn parse() -> Result<Option<Self>, String> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.iter().any(|a| a == "--help" || a == "-h") {
            return Err(Self::USAGE.to_string());
        }
        if args.iter().any(|a| a == "--version" || a == "-V") {
            return Err(format!("NovumChat {}", novumchat_core::VERSION));
        }
        let Some(pos) = args.iter().position(|a| a == "--serve") else { return Ok(None) };
        let addr = args.get(pos + 1).filter(|a| !a.starts_with("--")).cloned();
        let static_dir = args
            .iter()
            .position(|a| a == "--static")
            .map(|i| args.get(i + 1).cloned().ok_or_else(|| "--static needs a directory".to_string()))
            .transpose()?
            .map(std::path::PathBuf::from);
        Ok(Some(Self { addr, static_dir }))
    }
}

#[cfg(all(desktop, feature = "web-server"))]
fn run_server(context: &tauri::Context<tauri::Wry>, args: ServeArgs) -> anyhow::Result<()> {
    let assets = match args.static_dir {
        Some(dir) => server::Assets::Dir(dir),
        None => server::Assets::from_tauri(context),
    };
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let paths = AppPaths::resolve(None)?;
        println!("config:   {}", paths.config_file().display());
        println!("database: {}", paths.database_file().display());
        let app = App::init(paths, true).await?;
        let addr = args.addr.unwrap_or_else(|| app.config.get().server.bind);
        let addr: std::net::SocketAddr = addr.parse().map_err(|e| anyhow::anyhow!("invalid address `{addr}`: {e}"))?;
        let result = server::serve(app.clone(), addr, assets).await;
        app.shutdown().await;
        result
    })
}
