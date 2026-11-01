//! Settings. Changes save automatically to config.toml.

use std::collections::BTreeMap;

use leptos::prelude::*;
use leptos::task::spawn_local;
use novumchat_core::config::*;
use novumchat_core::permissions::PermissionLevel;
use novumchat_core::types::{McpState, ModelRef, SearchResponse, ToastLevel, ToolDescriptor};

use crate::api;
use crate::icons::{Icon, Mark};
use crate::state::{ConfirmDialog, SettingsTab, use_state};
use crate::util::hue_style;

type Draft = RwSignal<AppConfig>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SaveState {
    Idle,
    Saving,
    Saved,
    Failed,
}

fn lines(v: &[String]) -> String {
    v.join("\n")
}

fn parse_lines(s: &str) -> Vec<String> {
    s.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

fn kv_text(m: &BTreeMap<String, String>, sep: &str) -> String {
    m.iter().map(|(k, v)| format!("{k}{sep}{v}")).collect::<Vec<_>>().join("\n")
}

fn parse_kv(s: &str, sep: char) -> BTreeMap<String, String> {
    s.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(sep)?;
            let k = k.trim();
            (!k.is_empty()).then(|| (k.to_string(), v.trim().to_string()))
        })
        .collect()
}

fn opt_string(s: String) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

#[component]
fn Toggle(
    draft: Draft,
    #[prop(into)] label: String,
    #[prop(optional, into)] hint: String,
    get: fn(&AppConfig) -> bool,
    set: fn(&mut AppConfig, bool),
) -> impl IntoView {
    let on = Memo::new(move |_| draft.with(get));
    view! {
        <div class="nc-row">
            <div class="nc-row-label grow">
                <b>{label.clone()}</b>
                {(!hint.is_empty()).then(|| view! { <span>{hint.clone()}</span> })}
            </div>
            <button class="nc-switch" class:is-on=move || on.get() role="switch" aria-label=label aria-checked=move || on.get().to_string() on:click=move |_| {
                let v = !on.get_untracked();
                draft.update(|c| set(c, v));
            }></button>
        </div>
    }
}

#[component]
fn Text(
    draft: Draft,
    #[prop(into)] label: String,
    #[prop(optional, into)] hint: String,
    #[prop(optional, into)] placeholder: String,
    #[prop(default = "text")] kind: &'static str,
    get: fn(&AppConfig) -> String,
    set: fn(&mut AppConfig, String),
) -> impl IntoView {
    view! {
        <div class="nc-field">
            <label>{label}</label>
            <input
                class="nc-input"
                type=kind
                placeholder=placeholder
                autocomplete="off"
                spellcheck="false"
                prop:value=move || draft.with(get)
                on:change=move |ev| {
                    let v = event_target_value(&ev);
                    draft.update(|c| set(c, v));
                }
            />
            {(!hint.is_empty()).then(|| view! { <div class="hint">{hint}</div> })}
        </div>
    }
}

#[component]
fn ModelSelect(
    #[prop(into)] label: String,
    #[prop(optional, into)] hint: String,
    #[prop(into)] none_label: String,
    value: Signal<Option<ModelRef>>,
    on_change: Callback<Option<ModelRef>>,
) -> impl IntoView {
    let state = use_state();
    view! {
        <div class="nc-field">
            <label>{label}</label>
            <select class="nc-select" on:change=move |ev| {
                let v = event_target_value(&ev);
                on_change.run(ModelRef::parse_key(&v));
            }>
                <option value="" selected=move || value.get().is_none()>{none_label.clone()}</option>
                {move || {
                    let current = value.get();
                    let mut options: Vec<(String, String)> = state.models.with(|m| m.models.iter().map(|i| (i.model_ref().key(), format!("{} ({})", i.id, i.provider_name))).collect());
                    if let Some(c) = &current
                        && !options.iter().any(|(k, _)| *k == c.key()) {
                            options.insert(0, (c.key(), format!("{} (unavailable)", c.key())));
                        }
                    options.into_iter().map(|(key, text)| {
                        let selected = current.as_ref().is_some_and(|c| c.key() == key);
                        view! { <option value=key selected=selected>{text}</option> }
                    }).collect_view()
                }}
            </select>
            {(!hint.is_empty()).then(|| view! { <div class="hint">{hint}</div> })}
        </div>
    }
}

// ── tabs ───────────────────────────────────────────────────────────────────

#[component]
fn GeneralTab(draft: Draft) -> impl IntoView {
    view! {
        <p class="lead">"How new chats start and how replies are generated."</p>
        <section class="nc-section">
            <ModelSelect
                label="Default model"
                hint="Used for new chats until you pick something else in the composer."
                none_label="First available model"
                value=Signal::derive(move || draft.with(|c| c.general.default_model.clone()))
                on_change=Callback::new(move |m| draft.update(|c| c.general.default_model = m))
            />
            <div class="nc-field">
                <label>"Default system prompt"</label>
                <textarea class="nc-textfield" prop:value=move || draft.with(|c| c.general.system_prompt.clone()) on:change=move |ev| {
                    let v = event_target_value(&ev);
                    draft.update(|c| c.general.system_prompt = v);
                }></textarea>
                <div class="hint">"{date} becomes today's date. Each chat can override this from its menu."</div>
            </div>
        </section>
        <section class="nc-section">
            <Toggle draft=draft label="Send with Enter" hint="Shift + Enter adds a new line. On touch screens Enter always adds a line." get={|c| c.general.send_on_enter} set={|c, v| c.general.send_on_enter = v}/>
            <Toggle draft=draft label="Name chats automatically" hint="After the first reply, the model suggests a short title." get={|c| c.general.auto_title} set={|c, v| c.general.auto_title = v}/>
        </section>
        <section class="nc-section">
            <h4>"Generation"</h4>
            <div class="nc-grid-2">
                <div class="nc-field">
                    <label>"Thinking effort"</label>
                    <select class="nc-select" on:change=move |ev| {
                        let v = event_target_value(&ev);
                        draft.update(|c| c.general.reasoning_effort = opt_string(v));
                    }>
                        {[("", "Model default"), ("low", "Low"), ("medium", "Medium"), ("high", "High")].into_iter().map(|(v, l)| {
                            view! { <option value=v selected=move || draft.with(|c| c.general.reasoning_effort.as_deref().unwrap_or("") == v)>{l}</option> }
                        }).collect_view()}
                    </select>
                    <div class="hint">"For models that can reason before answering."</div>
                </div>
                <Text draft=draft label="Temperature" placeholder="Model default" kind="number" hint="0 is focused, 1+ is creative."
                    get={|c| c.general.temperature.map(|t| t.to_string()).unwrap_or_default()}
                    set={|c, v| c.general.temperature = v.trim().parse::<f32>().ok().map(|t| t.clamp(0.0, 2.0))}/>
                <Text draft=draft label="Max reply tokens" placeholder="Model default" kind="number"
                    get={|c| c.general.max_tokens.map(|t| t.to_string()).unwrap_or_default()}
                    set={|c, v| c.general.max_tokens = v.trim().parse::<u32>().ok().filter(|v| *v > 0)}/>
                <Text draft=draft label="Max tool rounds" kind="number" hint="How many times a model may call tools in one reply."
                    get={|c| c.general.max_tool_rounds.to_string()}
                    set={|c, v| c.general.max_tool_rounds = v.trim().parse().unwrap_or(16)}/>
            </div>
        </section>
    }
}

const ACCENTS: [(&str, &str, &str); 6] = [
    ("aurora", "Aurora", "linear-gradient(135deg,#5be7ff,#4bf2a6 40%,#ffd166 70%,#c77dff)"),
    ("ocean", "Ocean", "linear-gradient(135deg,#38d6d0,#0f8bb0)"),
    ("sunset", "Sunset", "linear-gradient(135deg,#ffb48f,#d6542f)"),
    ("forest", "Forest", "linear-gradient(135deg,#93f0b7,#1d8a4f)"),
    ("rose", "Rose", "linear-gradient(135deg,#ffb0cb,#c4386c)"),
    ("mono", "Mono", "linear-gradient(135deg,#eaf1fb,#0f1b2d)"),
];

#[component]
fn AppearanceTab(draft: Draft) -> impl IntoView {
    view! {
        <p class="lead">"Make NovumChat comfortable to read, day or night."</p>
        <section class="nc-section">
            <div class="nc-row">
                <div class="nc-row-label grow"><b>"Theme"</b><span>"Auto follows your system's light or dark setting."</span></div>
                <div class="nc-segmented" role="group" aria-label="Theme">
                    {[(ThemeMode::Auto, "auto", "Auto"), (ThemeMode::Light, "sun", "Light"), (ThemeMode::Dark, "moon", "Dark")].into_iter().map(|(m, icon, label)| view! {
                        <button class:is-active=move || draft.with(|c| c.appearance.theme == m) on:click=move |_| draft.update(|c| c.appearance.theme = m)>
                            <Icon name=icon size=15/>
                            {label}
                        </button>
                    }).collect_view()}
                </div>
            </div>
            <div class="nc-field">
                <label>"Accent"</label>
                <div class="nc-accent-swatches">
                    {ACCENTS.into_iter().map(|(id, label, bg)| view! {
                        <button
                            class="nc-swatch"
                            class:is-active=move || draft.with(|c| c.appearance.accent == id)
                            style=format!("background:{bg}")
                            title=label
                            aria-label=label
                            on:click=move |_| draft.update(|c| c.appearance.accent = id.into())
                        ></button>
                    }).collect_view()}
                </div>
            </div>
            <div class="nc-field">
                <label>{move || format!("Text size: {:.0}%", draft.with(|c| c.appearance.font_scale) * 100.0)}</label>
                <input type="range" min="0.85" max="1.3" step="0.05" style="accent-color:var(--accent)"
                    prop:value=move || draft.with(|c| c.appearance.font_scale.to_string())
                    on:input=move |ev| {
                        let v: f32 = event_target_value(&ev).parse().unwrap_or(1.0);
                        draft.update(|c| c.appearance.font_scale = v);
                    }/>
            </div>
        </section>
        <section class="nc-section">
            <Toggle draft=draft label="Live previews" hint="Show ```html, ```svg and ```mermaid replies as running apps and diagrams." get={|c| c.appearance.live_previews} set={|c, v| c.appearance.live_previews = v}/>
            <Toggle draft=draft label="Expand reasoning while streaming" hint="Otherwise the thinking panel stays collapsed until you open it." get={|c| c.appearance.expand_reasoning} set={|c, v| c.appearance.expand_reasoning = v}/>
            <Toggle draft=draft label="Load remote images automatically" hint="Off keeps replies from contacting other servers. Blocked images show a button to load them." get={|c| c.appearance.load_remote_images} set={|c, v| c.appearance.load_remote_images = v}/>
            <Toggle draft=draft label="Animations" hint="Streaming effects and transitions. Also respects your system's reduced-motion setting." get={|c| c.appearance.animations} set={|c, v| c.appearance.animations = v}/>
        </section>
    }
}

struct Preset {
    name: &'static str,
    id: &'static str,
    kind: ProviderKind,
    url: &'static str,
    env: Option<&'static str>,
}

const PRESETS: [Preset; 11] = [
    Preset { name: "Ollama", id: "ollama", kind: ProviderKind::Ollama, url: "http://localhost:11434", env: None },
    Preset {
        name: "OpenAI",
        id: "openai",
        kind: ProviderKind::Openai,
        url: "https://api.openai.com/v1",
        env: Some("OPENAI_API_KEY"),
    },
    Preset {
        name: "OpenRouter",
        id: "openrouter",
        kind: ProviderKind::Openai,
        url: "https://openrouter.ai/api/v1",
        env: Some("OPENROUTER_API_KEY"),
    },
    Preset {
        name: "LM Studio",
        id: "lmstudio",
        kind: ProviderKind::Openai,
        url: "http://localhost:1234/v1",
        env: None,
    },
    Preset {
        name: "Groq",
        id: "groq",
        kind: ProviderKind::Openai,
        url: "https://api.groq.com/openai/v1",
        env: Some("GROQ_API_KEY"),
    },
    Preset {
        name: "Google Gemini",
        id: "gemini",
        kind: ProviderKind::Openai,
        url: "https://generativelanguage.googleapis.com/v1beta/openai",
        env: Some("GEMINI_API_KEY"),
    },
    Preset {
        name: "DeepSeek",
        id: "deepseek",
        kind: ProviderKind::Openai,
        url: "https://api.deepseek.com/v1",
        env: Some("DEEPSEEK_API_KEY"),
    },
    Preset {
        name: "Mistral",
        id: "mistral",
        kind: ProviderKind::Openai,
        url: "https://api.mistral.ai/v1",
        env: Some("MISTRAL_API_KEY"),
    },
    Preset {
        name: "Together",
        id: "together",
        kind: ProviderKind::Openai,
        url: "https://api.together.xyz/v1",
        env: Some("TOGETHER_API_KEY"),
    },
    Preset {
        name: "Ollama Cloud",
        id: "ollama-cloud",
        kind: ProviderKind::Ollama,
        url: "https://ollama.com",
        env: Some("OLLAMA_API_KEY"),
    },
    Preset {
        name: "Custom endpoint",
        id: "custom",
        kind: ProviderKind::Openai,
        url: "http://localhost:8000/v1",
        env: None,
    },
];

fn update_provider(draft: Draft, index: usize, f: impl FnOnce(&mut ProviderConfig)) {
    draft.update(|c| {
        if let Some(p) = c.providers.get_mut(index) {
            f(p);
        }
    });
}

#[component]
fn ProviderCard(draft: Draft, index: usize) -> impl IntoView {
    let state = use_state();
    let open = RwSignal::new(false);
    let p = Memo::new(move |_| draft.with(|c| c.providers.get(index).cloned().unwrap_or_default()));
    let status = Memo::new(move |_| {
        let id = p.with(|p| p.id.clone());
        state.models.with(|m| m.providers.iter().find(|s| s.id == id).cloned())
    });
    view! {
        <div class="nc-list-card">
            <div class="nc-list-head">
                <span class="ico">{move || view! { <Icon name=if p.get().kind == ProviderKind::Ollama { "cpu" } else { "zap" } size=17/> }}</span>
                <div style="flex:1;min-width:0">
                    <b class="nc-truncate">{move || p.get().name}</b>
                    <small class="nc-truncate">{move || { let p = p.get(); format!("{} at {}", p.kind.label(), p.base_url) }}</small>
                </div>
                {move || match status.get() {
                    _ if !p.get().enabled => view! { <span class="nc-badge">"Off"</span> }.into_any(),
                    Some(s) if s.ok => view! { <span class="nc-badge ok">{format!("{} models", s.model_count)}</span> }.into_any(),
                    Some(s) => view! { <span class="nc-badge danger" title=s.error.clone().unwrap_or_default()>"Unreachable"</span> }.into_any(),
                    None => view! { <span class="nc-badge">"Not checked"</span> }.into_any(),
                }}
                <button class="nc-switch" class:is-on=move || p.get().enabled role="switch" aria-label="Enabled" aria-checked=move || p.get().enabled.to_string()
                    on:click=move |_| update_provider(draft, index, |p| p.enabled = !p.enabled)></button>
                <button class="nc-icon-btn sm" aria-label="Edit provider" on:click=move |_| open.update(|o| *o = !*o)>
                    {move || view! { <Icon name=if open.get() { "chevron-down" } else { "chevron-right" } size=16/> }}
                </button>
            </div>
            {move || status.get().and_then(|s| s.error).filter(|_| p.get().enabled).map(|e| view! {
                <div class="nc-mcp-error" style="margin:0 14px 12px">{e}</div>
            })}
            <Show when=move || open.get()>
                <div class="nc-list-body">
                    <div class="nc-grid-2">
                        <div class="nc-field">
                            <label>"Name"</label>
                            <input class="nc-input" prop:value=move || p.get().name on:change=move |ev| { let v = event_target_value(&ev); update_provider(draft, index, move |p| p.name = v) }/>
                        </div>
                        <div class="nc-field">
                            <label>"API type"</label>
                            <select class="nc-select" on:change=move |ev| {
                                let v = event_target_value(&ev);
                                update_provider(draft, index, move |p| p.kind = if v == "ollama" { ProviderKind::Ollama } else { ProviderKind::Openai });
                            }>
                                <option value="openai" selected=move || p.get().kind == ProviderKind::Openai>"OpenAI-compatible"</option>
                                <option value="ollama" selected=move || p.get().kind == ProviderKind::Ollama>"Ollama"</option>
                            </select>
                        </div>
                    </div>
                    <div class="nc-field">
                        <label>"Base URL"</label>
                        <input class="nc-input" spellcheck="false" prop:value=move || p.get().base_url on:change=move |ev| { let v = event_target_value(&ev); update_provider(draft, index, move |p| p.base_url = v) }/>
                    </div>
                    <div class="nc-grid-2">
                        <div class="nc-field">
                            <label>"API key"</label>
                            <input class="nc-input" type="password" autocomplete="off" placeholder="Not set" prop:value=move || p.get().api_key.unwrap_or_default()
                                on:change=move |ev| { let v = event_target_value(&ev); update_provider(draft, index, move |p| p.api_key = opt_string(v)) }/>
                            <div class="hint">"Stored in config.toml (readable only by you)."</div>
                        </div>
                        <div class="nc-field">
                            <label>"…or environment variable"</label>
                            <input class="nc-input" spellcheck="false" placeholder="e.g. OPENAI_API_KEY" prop:value=move || p.get().api_key_env.unwrap_or_default()
                                on:change=move |ev| { let v = event_target_value(&ev); update_provider(draft, index, move |p| p.api_key_env = opt_string(v)) }/>
                        </div>
                    </div>
                    <div class="nc-field">
                        <label>"Extra models"</label>
                        <input class="nc-input" spellcheck="false" placeholder="model-a, model-b" prop:value=move || p.get().models.join(", ") on:change=move |ev| {
                            let v = event_target_value(&ev);
                            update_provider(draft, index, move |p| p.models = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
                        }/>
                        <div class="hint">"Listed in addition to the models the endpoint reports."</div>
                    </div>
                    <div class="nc-row">
                        <div class="nc-row-label grow"><b>"Discover models"</b><span>"Ask the endpoint for its model list."</span></div>
                        <button class="nc-switch" class:is-on=move || p.get().discover_models role="switch" aria-label="Discover models"
                            on:click=move |_| update_provider(draft, index, |p| p.discover_models = !p.discover_models)></button>
                    </div>
                    <div class="nc-row">
                        <span class="nc-spacer"></span>
                        <button class="nc-btn nc-btn-danger-ghost nc-btn-sm" on:click=move |_| draft.update(|c| { if index < c.providers.len() { c.providers.remove(index); } })>
                            <Icon name="trash" size=15/>
                            "Remove provider"
                        </button>
                    </div>
                </div>
            </Show>
        </div>
    }
}

#[component]
fn ProvidersTab(draft: Draft) -> impl IntoView {
    let state = use_state();
    let adding = RwSignal::new(false);
    let add = move |preset: &'static Preset| {
        adding.set(false);
        draft.update(|c| {
            c.providers.push(ProviderConfig {
                id: preset.id.into(),
                name: preset.name.into(),
                kind: preset.kind,
                base_url: preset.url.into(),
                api_key_env: preset.env.map(String::from),
                ..Default::default()
            });
        });
    };
    view! {
        <p class="lead">"Connect as many Ollama servers and OpenAI-compatible endpoints as you like. Every model they offer shows up in the model menu."</p>
        <div style="display:flex;gap:8px;margin:0 0 14px;flex-wrap:wrap;position:relative">
            <button class="nc-btn nc-btn-primary" on:click=move |_| adding.update(|a| *a = !*a)>
                <Icon name="plus" size=16/>
                "Add provider"
            </button>
            <button class="nc-btn nc-btn-secondary" on:click=move |_| { spawn_local(async move { state.refresh_models(true).await }); }>
                {move || if state.models_loading.get() { view! { <span class="nc-spinner"></span> }.into_any() } else { view! { <Icon name="refresh" size=16/> }.into_any() }}
                "Check connections"
            </button>
            <Show when=move || adding.get()>
                <div style="position:fixed;inset:0;z-index:39" on:click=move |_| adding.set(false)></div>
                <div class="nc-menu" style="left:0;top:calc(100% + 6px);width:260px;max-height:380px;overflow-y:auto">
                    {PRESETS.iter().map(|p| view! {
                        <button on:click=move |_| add(p)>
                            <Icon name=if p.kind == ProviderKind::Ollama { "cpu" } else { "zap" } size=16/>
                            <span style="flex:1">{p.name}</span>
                        </button>
                    }).collect_view()}
                </div>
            </Show>
        </div>
        <For
            each={move || (0..draft.with(|c| c.providers.len())).collect::<Vec<_>>()}
            key=|i| *i
            children=move |i| view! { <ProviderCard draft=draft index=i/> }
        />
        <Show when=move || draft.with(|c| c.providers.is_empty())>
            <div class="nc-section"><p style="margin:0">"No providers yet. Add Ollama to use local models, or any OpenAI-compatible service."</p></div>
        </Show>
    }
}

#[component]
fn EnsembleTab(draft: Draft) -> impl IntoView {
    let state = use_state();
    view! {
        <p class="lead">"Ensemble mode sends one message to several models at once. Their answers appear side by side, and can be merged into a single combined answer."</p>
        <section class="nc-section">
            <h4>"Default models to compare"</h4>
            <p>"Used when you switch on Compare in the model menu of a new chat."</p>
            <div style="display:flex;flex-wrap:wrap;gap:8px">
                {move || state.models.with(|m| m.models.iter().map(|info| {
                    let r = info.model_ref();
                    let r2 = r.clone();
                    let on = Memo::new(move |_| draft.with(|c| c.ensemble.default_models.contains(&r2)));
                    let style = hue_style(Some(&r));
                    view! {
                        <button class="nc-chip-btn" class:is-on=move || on.get() style=style on:click=move |_| {
                            let r = r.clone();
                            draft.update(move |c| {
                                if let Some(p) = c.ensemble.default_models.iter().position(|x| *x == r) {
                                    c.ensemble.default_models.remove(p);
                                } else {
                                    c.ensemble.default_models.push(r);
                                }
                            });
                        }>
                            <span class="nc-hue-dot"></span>
                            {info.id.clone()}
                        </button>
                    }
                }).collect_view())}
            </div>
        </section>
        <section class="nc-section">
            <Toggle draft=draft label="Combine answers automatically" hint="When every model has answered, a synthesizer model writes one answer that merges their strengths." get={|c| c.ensemble.synthesize} set={|c, v| c.ensemble.synthesize = v}/>
            <ModelSelect
                label="Synthesizer model"
                none_label="The first model of the ensemble"
                value=Signal::derive(move || draft.with(|c| c.ensemble.synthesizer.clone()))
                on_change=Callback::new(move |m| draft.update(|c| c.ensemble.synthesizer = m))
            />
            <div class="nc-field">
                <label>"Synthesis instructions"</label>
                <textarea class="nc-textfield" prop:value=move || draft.with(|c| c.ensemble.synthesis_prompt.clone()) on:change=move |ev| {
                    let v = event_target_value(&ev);
                    draft.update(|c| c.ensemble.synthesis_prompt = v);
                }></textarea>
            </div>
        </section>
        <section class="nc-section">
            <p style="margin:0">"New chats start with these defaults: "{move || if state.draft_settings.with(|s| s.models.len() > 1) { "compare is on" } else { "a single model" }}"."</p>
            <div>
                <button class="nc-btn nc-btn-secondary nc-btn-sm" on:click=move |_| {
                    let models = draft.with_untracked(|c| c.ensemble.default_models.clone());
                    if models.len() < 2 {
                        state.toast(ToastLevel::Info, "Pick at least two default models first.");
                        return;
                    }
                    let synth = draft.with_untracked(|c| c.ensemble.synthesize);
                    state.draft_settings.update(|s| { s.models = models; s.synthesize = synth; });
                    state.toast(ToastLevel::Success, "The next new chat will compare your default models.");
                }>
                    <Icon name="layers" size=15/>
                    "Use defaults for my next chat"
                </button>
            </div>
        </section>
    }
}

#[component]
fn ToolsTab(draft: Draft) -> impl IntoView {
    let state = use_state();
    let tools = RwSignal::new(Vec::<ToolDescriptor>::new());
    spawn_local(async move {
        if let Ok(t) = api::list_tools().await {
            tools.set(t);
        }
    });
    let pick_level = move |level: PermissionLevel| {
        if level == PermissionLevel::Yolo {
            state.confirm.set(Some(ConfirmDialog {
                title: "Turn on YOLO mode?".into(),
                message: "Models will run commands, edit any file and use MCP tools without asking. Only the denied command patterns are still blocked. Use this only with models and prompts you trust.".into(),
                confirm: "Turn on YOLO".into(),
                danger: true,
                on_confirm: Callback::new(move |_| draft.update(|c| c.tools.permission = PermissionLevel::Yolo)),
            }));
        } else {
            draft.update(|c| c.tools.permission = level);
        }
    };
    let workspace =
        Memo::new(move |_| state.info.with(|i| i.as_ref().map(|i| i.workspace.clone()).unwrap_or_default()));
    view! {
        <p class="lead">"Tools let models read and edit files, run shell commands, search the web and use MCP servers. You decide how much they may do without asking."</p>
        <section class="nc-section">
            <Toggle draft=draft label="Enable tools" hint="Chats can still switch tools off individually." get={|c| c.tools.enabled} set={|c, v| c.tools.enabled = v}/>
        </section>
        <section class="nc-section">
            <h4>"Permission level"</h4>
            <div class="nc-levels">
                {PermissionLevel::ALL.into_iter().map(|level| view! {
                    <button
                        class="nc-level"
                        class:is-active=move || draft.with(|c| c.tools.permission == level)
                        class:is-yolo=level == PermissionLevel::Yolo
                        on:click=move |_| pick_level(level)
                    >
                        <b>
                            {level.label()}
                            {(level == PermissionLevel::Ask).then(|| view! { <span class="tag">"Recommended"</span> })}
                        </b>
                        <span>{level.description()}</span>
                    </button>
                }).collect_view()}
            </div>
        </section>
        <section class="nc-section">
            <h4>"Workspace"</h4>
            <Text draft=draft label="Working directory" placeholder="~/NovumChat" hint="Relative paths in file tools and shell commands start here. Reads inside it don't need approval."
                get={|c| c.tools.workspace.clone().unwrap_or_default()}
                set={|c, v| c.tools.workspace = opt_string(v)}/>
            <div class="nc-path">{move || workspace.get()}</div>
            <div class="nc-grid-2">
                <Text draft=draft label="Shell" placeholder="bash (default)" get={|c| c.tools.shell.clone().unwrap_or_default()} set={|c, v| c.tools.shell = opt_string(v)}/>
                <Text draft=draft label="Command timeout (seconds)" kind="number" get={|c| c.tools.bash_timeout_secs.to_string()} set={|c, v| c.tools.bash_timeout_secs = v.trim().parse().unwrap_or(120)}/>
            </div>
        </section>
        <section class="nc-section">
            <h4>"Commands"</h4>
            <div class="nc-grid-2">
                <div class="nc-field">
                    <label>"Run without asking"</label>
                    <textarea class="nc-textfield" style="font-family:var(--font-mono);font-size:13px;min-height:160px" prop:value=move || draft.with(|c| lines(&c.tools.allowed_commands)) on:change=move |ev| {
                        let v = event_target_value(&ev);
                        draft.update(|c| c.tools.allowed_commands = parse_lines(&v));
                    }></textarea>
                    <div class="hint">"One command prefix per line. Chained commands (;, &&, |) always ask."</div>
                </div>
                <div class="nc-field">
                    <label>"Always refuse"</label>
                    <textarea class="nc-textfield" style="font-family:var(--font-mono);font-size:13px;min-height:160px" prop:value=move || draft.with(|c| lines(&c.tools.denied_commands)) on:change=move |ev| {
                        let v = event_target_value(&ev);
                        draft.update(|c| c.tools.denied_commands = parse_lines(&v));
                    }></textarea>
                    <div class="hint">"Blocked at every permission level, including YOLO."</div>
                </div>
            </div>
        </section>
        <section class="nc-section">
            <h4>"Built-in tools"</h4>
            {move || tools.get().into_iter().filter(|t| t.source == "builtin").map(|t| {
                let name = t.name.clone();
                let name2 = t.name.clone();
                let on = Memo::new(move |_| draft.with(|c| !c.tools.disabled_tools.contains(&name)));
                view! {
                    <div class="nc-row">
                        <div class="nc-row-label grow">
                            <b><code style="font-family:var(--font-mono);font-size:13px">{t.name.clone()}</code>" "<span class="nc-badge" style="margin-left:6px">{t.risk.label()}</span></b>
                            <span>{t.description.clone()}</span>
                        </div>
                        <button class="nc-switch" class:is-on=move || on.get() role="switch" aria-label=t.name.clone() on:click=move |_| {
                            let n = name2.clone();
                            draft.update(move |c| {
                                if let Some(p) = c.tools.disabled_tools.iter().position(|x| *x == n) {
                                    c.tools.disabled_tools.remove(p);
                                } else {
                                    c.tools.disabled_tools.push(n);
                                }
                            });
                        }></button>
                    </div>
                }
            }).collect_view()}
        </section>
    }
}

#[component]
fn SearchTab(draft: Draft) -> impl IntoView {
    let query = RwSignal::new("open source large language models".to_string());
    let result = RwSignal::new(None::<Result<SearchResponse, String>>);
    let busy = RwSignal::new(false);
    let engine = Memo::new(move |_| draft.with(|c| c.search.engine));
    let state = use_state();
    view! {
        <p class="lead">"When web search is on for a chat, models can look things up and read pages, citing their sources."</p>
        <section class="nc-section">
            <div class="nc-field">
                <label>"Search engine"</label>
                <div class="nc-segmented" style="align-self:flex-start">
                    {[SearchEngine::Duckduckgo, SearchEngine::Searxng, SearchEngine::Brave].into_iter().map(|e| view! {
                        <button class:is-active=move || engine.get() == e on:click=move |_| draft.update(|c| c.search.engine = e)>{e.label()}</button>
                    }).collect_view()}
                </div>
                <div class="hint">{move || match engine.get() {
                    SearchEngine::Duckduckgo => "No account or API key needed.",
                    SearchEngine::Searxng => "Your own SearXNG instance. Enable the JSON format in its settings.yml (search.formats).",
                    SearchEngine::Brave => "Needs a Brave Search API key (free tier available).",
                }}</div>
            </div>
            <Show when=move || engine.get() == SearchEngine::Searxng>
                <Text draft=draft label="SearXNG URL" placeholder="http://localhost:8888" get={|c| c.search.searxng_url.clone()} set={|c, v| c.search.searxng_url = v.trim().to_string()}/>
            </Show>
            <Show when=move || engine.get() == SearchEngine::Brave>
                <div class="nc-grid-2">
                    <Text draft=draft label="Brave API key" kind="password" placeholder="Not set" get={|c| c.search.brave_api_key.clone().unwrap_or_default()} set={|c, v| c.search.brave_api_key = opt_string(v)}/>
                    <Text draft=draft label="…or environment variable" placeholder="BRAVE_API_KEY" get={|c| c.search.brave_api_key_env.clone().unwrap_or_default()} set={|c, v| c.search.brave_api_key_env = opt_string(v)}/>
                </div>
            </Show>
            <div class="nc-grid-2">
                <Text draft=draft label="Results per search" kind="number" get={|c| c.search.max_results.to_string()} set={|c, v| c.search.max_results = v.trim().parse().unwrap_or(6)}/>
                <Text draft=draft label="Region or language" placeholder="Automatic" hint="e.g. us-en (DuckDuckGo) or en (SearXNG, Brave)." get={|c| c.search.region.clone().unwrap_or_default()} set={|c, v| c.search.region = opt_string(v)}/>
            </div>
            <Toggle draft=draft label="Safe search" get={|c| c.search.safe_search} set={|c, v| c.search.safe_search = v}/>
        </section>
        <section class="nc-section">
            <h4>"Try it"</h4>
            <form class="nc-row" on:submit=move |ev| {
                ev.prevent_default();
                busy.set(true);
                let q = query.get_untracked();
                spawn_local(async move {
                    result.set(Some(api::web_search(q).await));
                    busy.set(false);
                });
            }>
                <input class="nc-input grow" prop:value=move || query.get() on:input=move |ev| query.set(event_target_value(&ev))/>
                <button class="nc-btn nc-btn-secondary" type="submit" disabled=move || busy.get()>
                    {move || if busy.get() { view! { <span class="nc-spinner"></span> }.into_any() } else { view! { <Icon name="search" size=16/> }.into_any() }}
                    "Search"
                </button>
            </form>
            {move || result.get().map(|r| match r {
                Ok(resp) => view! {
                    <div style="display:grid;gap:10px">
                        <small style="color:var(--muted)">{format!("{} results from {}", resp.results.len(), resp.engine.label())}</small>
                        {resp.results.into_iter().map(|r| {
                            let url = r.url.clone();
                            view! {
                                <button class="nc-source" style="width:100%" on:click=move |_| state.open_link(url.clone())>
                                    <b>{r.title.clone()}</b>
                                    <small class="nc-truncate">{r.url.clone()}</small>
                                    <span style="font-size:13px;color:var(--text-2);line-height:1.45">{r.snippet.clone()}</span>
                                </button>
                            }
                        }).collect_view()}
                    </div>
                }.into_any(),
                Err(e) => view! { <div class="nc-error">{e}</div> }.into_any(),
            })}
        </section>
    }
}

fn update_mcp(draft: Draft, name: StoredValue<String>, f: impl FnOnce(&mut McpServerConfig)) {
    let n = name.get_value();
    draft.update(|c| {
        if let Some(s) = c.mcp_servers.get_mut(&n) {
            f(s);
        }
    });
}

#[component]
fn McpCard(draft: Draft, name: String) -> impl IntoView {
    let state = use_state();
    let open = RwSignal::new(false);
    let key = StoredValue::new(name.clone());
    let cfg = Memo::new(move |_| draft.with(|c| c.mcp_servers.get(&key.get_value()).cloned().unwrap_or_default()));
    let status = Memo::new(move |_| state.mcp.with(|s| s.iter().find(|x| x.name == key.get_value()).cloned()));
    let is_http = Memo::new(move |_| cfg.with(|c| c.url.is_some()));
    let mono = "font-family:var(--font-mono);font-size:13px";
    view! {
        <div class="nc-list-card">
            <div class="nc-list-head">
                <span class="ico"><Icon name="server" size=17/></span>
                <div style="flex:1;min-width:0">
                    <b class="nc-truncate">{name.clone()}</b>
                    <small class="nc-truncate">{move || {
                        let c = cfg.get();
                        let target = c.url.clone().unwrap_or_else(|| format!("{} {}", c.command.clone().unwrap_or_default(), c.args.join(" ")));
                        let info = status.get().and_then(|s| s.server_info).map(|i| format!("{i} via ")).unwrap_or_default();
                        format!("{info}{target}")
                    }}</small>
                </div>
                {move || match status.get() {
                    Some(s) => match s.state {
                        McpState::Connected => view! { <span class="nc-badge ok">{format!("{} tools", s.tools.len())}</span> }.into_any(),
                        McpState::Connecting => view! { <span class="nc-badge"><span class="nc-spinner" style="width:10px;height:10px"></span>"Connecting"</span> }.into_any(),
                        McpState::Failed => view! { <span class="nc-badge danger">"Failed"</span> }.into_any(),
                        McpState::Disabled => view! { <span class="nc-badge">"Off"</span> }.into_any(),
                    },
                    None => view! { <span class="nc-badge">"Pending"</span> }.into_any(),
                }}
                <button class="nc-switch" class:is-on=move || cfg.get().enabled role="switch" aria-label="Enabled"
                    on:click=move |_| update_mcp(draft, key, |s| s.enabled = !s.enabled)></button>
                <button class="nc-icon-btn sm" aria-label="Edit server" on:click=move |_| open.update(|o| *o = !*o)>
                    {move || view! { <Icon name=if open.get() { "chevron-down" } else { "chevron-right" } size=16/> }}
                </button>
            </div>
            {move || status.get().and_then(|s| s.error).map(|e| view! { <div class="nc-mcp-error" style="margin:0 14px 12px">{e}</div> })}
            {move || status.get().filter(|s| !s.tools.is_empty()).map(|s| view! {
                <div class="nc-mcp-tools" style="padding:0 14px 12px">
                    {s.tools.into_iter().map(|t| view! { <span title=t.description>{t.name}</span> }).collect_view()}
                </div>
            })}
            <Show when=move || open.get()>
                <div class="nc-list-body">
                    <Show when=move || is_http.get() fallback=move || view! {
                        <div class="nc-field">
                            <label>"Command"</label>
                            <input class="nc-input" spellcheck="false" placeholder="npx" prop:value=move || cfg.get().command.unwrap_or_default()
                                on:change=move |ev| { let v = event_target_value(&ev); update_mcp(draft, key, move |s| s.command = opt_string(v)) }/>
                        </div>
                        <div class="nc-field">
                            <label>"Arguments (one per line)"</label>
                            <textarea class="nc-textfield" style=format!("{mono};min-height:84px") prop:value=move || lines(&cfg.get().args)
                                on:change=move |ev| { let v = event_target_value(&ev); update_mcp(draft, key, move |s| s.args = parse_lines(&v)) }></textarea>
                        </div>
                        <div class="nc-field">
                            <label>"Environment (KEY=value per line)"</label>
                            <textarea class="nc-textfield" style=format!("{mono};min-height:70px") prop:value=move || kv_text(&cfg.get().env, "=")
                                on:change=move |ev| { let v = event_target_value(&ev); update_mcp(draft, key, move |s| s.env = parse_kv(&v, '=')) }></textarea>
                        </div>
                        <div class="nc-field">
                            <label>"Working directory"</label>
                            <input class="nc-input" spellcheck="false" placeholder="Default" prop:value=move || cfg.get().cwd.unwrap_or_default()
                                on:change=move |ev| { let v = event_target_value(&ev); update_mcp(draft, key, move |s| s.cwd = opt_string(v)) }/>
                        </div>
                    }>
                        <div class="nc-field">
                            <label>"URL"</label>
                            <input class="nc-input" spellcheck="false" placeholder="https://example.com/mcp" prop:value=move || cfg.get().url.unwrap_or_default()
                                on:change=move |ev| { let v = event_target_value(&ev); update_mcp(draft, key, move |s| s.url = Some(v.trim().to_string())) }/>
                        </div>
                        <div class="nc-field">
                            <label>"Bearer token environment variable"</label>
                            <input class="nc-input" spellcheck="false" placeholder="MY_SERVER_TOKEN" prop:value=move || cfg.get().bearer_token_env_var.unwrap_or_default()
                                on:change=move |ev| { let v = event_target_value(&ev); update_mcp(draft, key, move |s| s.bearer_token_env_var = opt_string(v)) }/>
                        </div>
                        <div class="nc-field">
                            <label>"HTTP headers (Name: value per line)"</label>
                            <textarea class="nc-textfield" style=format!("{mono};min-height:70px") prop:value=move || kv_text(&cfg.get().http_headers, ": ")
                                on:change=move |ev| { let v = event_target_value(&ev); update_mcp(draft, key, move |s| s.http_headers = parse_kv(&v, ':')) }></textarea>
                        </div>
                    </Show>
                    <div class="nc-row">
                        <div class="nc-row-label grow"><b>"Trust this server"</b><span>"Run its tools without asking (except in read-only mode)."</span></div>
                        <button class="nc-switch" class:is-on=move || cfg.get().trusted role="switch" aria-label="Trusted"
                            on:click=move |_| update_mcp(draft, key, |s| s.trusted = !s.trusted)></button>
                    </div>
                    <div class="nc-row">
                        <button class="nc-btn nc-btn-secondary nc-btn-sm" on:click=move |_| {
                            let n = key.get_value();
                            spawn_local(async move {
                                if let Ok(s) = api::mcp_restart(Some(n)).await {
                                    state.mcp.set(s);
                                }
                            });
                        }>
                            <Icon name="refresh" size=15/>
                            "Restart"
                        </button>
                        <span class="nc-spacer"></span>
                        <button class="nc-btn nc-btn-danger-ghost nc-btn-sm" on:click=move |_| {
                            let n = key.get_value();
                            draft.update(move |c| { c.mcp_servers.remove(&n); });
                        }>
                            <Icon name="trash" size=15/>
                            "Remove"
                        </button>
                    </div>
                </div>
            </Show>
        </div>
    }
}

#[component]
fn McpTab(draft: Draft) -> impl IntoView {
    let state = use_state();
    let new_name = RwSignal::new(String::new());
    let new_kind = RwSignal::new("stdio");
    let add = move || {
        let name = novumchat_core::util::slugify(&new_name.get_untracked());
        if name.is_empty() {
            state.toast(ToastLevel::Info, "Give the server a name first.");
            return;
        }
        if draft.with_untracked(|c| c.mcp_servers.contains_key(&name)) {
            state.toast(ToastLevel::Warning, format!("A server named {name} already exists."));
            return;
        }
        let cfg = if new_kind.get_untracked() == "http" {
            McpServerConfig { url: Some("https://".into()), ..Default::default() }
        } else {
            McpServerConfig { command: Some("npx".into()), args: vec!["-y".into()], ..Default::default() }
        };
        draft.update(|c| {
            c.mcp_servers.insert(name, cfg);
        });
        new_name.set(String::new());
    };
    view! {
        <p class="lead">"MCP servers give models extra tools. They use the same [mcp_servers.name] format as the OpenAI Codex config.toml, so you can copy entries across."</p>
        <section class="nc-section">
            <h4>"Add a server"</h4>
            <div class="nc-row" style="flex-wrap:wrap">
                <input class="nc-input grow" style="min-width:160px" placeholder="Name, e.g. filesystem" prop:value=move || new_name.get() on:input=move |ev| new_name.set(event_target_value(&ev)) on:keydown=move |ev: web_sys::KeyboardEvent| if ev.key() == "Enter" { add() }/>
                <div class="nc-segmented">
                    <button class:is-active=move || new_kind.get() == "stdio" on:click=move |_| new_kind.set("stdio")>"Command"</button>
                    <button class:is-active=move || new_kind.get() == "http" on:click=move |_| new_kind.set("http")>"URL"</button>
                </div>
                <button class="nc-btn nc-btn-primary" on:click=move |_| add()><Icon name="plus" size=16/>"Add"</button>
            </div>
            <div class="nc-row" style="flex-wrap:wrap">
                <button class="nc-btn nc-btn-secondary nc-btn-sm" on:click=move |_| {
                    spawn_local(async move {
                        match api::import_codex_mcp().await {
                            Ok(r) if !r.imported.is_empty() => {
                                state.toast(ToastLevel::Success, format!("Imported {}", r.imported.join(", ")));
                                if let Ok(cfg) = api::get_config().await {
                                    draft.set(cfg.clone());
                                    state.config.set(Some(cfg));
                                }
                            }
                            Ok(r) if !r.skipped.is_empty() => state.toast(ToastLevel::Info, format!("Already added: {}", r.skipped.join(", "))),
                            Ok(_) => state.toast(ToastLevel::Info, "No MCP servers found in ~/.codex/config.toml."),
                            Err(e) => state.error(e),
                        }
                    });
                }>
                    <Icon name="download" size=15/>
                    "Import from Codex"
                </button>
                <button class="nc-btn nc-btn-ghost nc-btn-sm" on:click=move |_| {
                    spawn_local(async move {
                        if let Ok(s) = api::mcp_restart(None).await {
                            state.mcp.set(s);
                        }
                    });
                }>
                    <Icon name="refresh" size=15/>
                    "Restart all"
                </button>
            </div>
        </section>
        <For
            each={move || draft.with(|c| c.mcp_servers.keys().cloned().collect::<Vec<_>>())}
            key=|n| n.clone()
            children=move |n| view! { <McpCard draft=draft name=n/> }
        />
        <Show when=move || draft.with(|c| c.mcp_servers.is_empty())>
            <div class="nc-section"><p style="margin:0">"No MCP servers yet."</p></div>
        </Show>
    }
}

#[component]
fn SecurityTab() -> impl IntoView {
    let state = use_state();
    let p1 = RwSignal::new(String::new());
    let p2 = RwSignal::new(String::new());
    let p3 = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let vault = Memo::new(move |_| state.vault.get());
    let clear = move || {
        p1.set(String::new());
        p2.set(String::new());
        p3.set(String::new());
    };
    let run = move |op: &'static str| {
        error.set(None);
        let (a, b, c) = (p1.get_untracked(), p2.get_untracked(), p3.get_untracked());
        match op {
            "enable" if a != b => return error.set(Some("The passphrases don't match.".into())),
            "change" if b != c => return error.set(Some("The new passphrases don't match.".into())),
            _ => {}
        }
        busy.set(true);
        spawn_local(async move {
            let r = match op {
                "enable" => api::vault_enable(a).await,
                "disable" => api::vault_disable(a).await,
                _ => api::vault_change(a, b).await,
            };
            match r {
                Ok(s) => {
                    state.vault.set(s);
                    clear();
                    let msg = match op {
                        "enable" => "Your chats are now encrypted.",
                        "disable" => "Encryption is off. Chats are stored unencrypted.",
                        _ => "Passphrase changed.",
                    };
                    state.toast(ToastLevel::Success, msg);
                }
                Err(e) => error.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let pw = move |sig: RwSignal<String>, ph: &'static str, auto: &'static str| {
        view! {
            <input class="nc-input" type="password" autocomplete=auto placeholder=ph prop:value=move || sig.get() on:input=move |ev| sig.set(event_target_value(&ev))/>
        }
    };
    view! {
        <p class="lead">"Optionally encrypt your chat history on this device with a passphrase."</p>
        <section class="nc-section">
            <div class="nc-row">
                <span class="nc-tool-icon" style=move || format!("--risk:{};width:40px;height:40px;border-radius:12px", if vault.get().encrypted { "var(--ok)" } else { "var(--muted)" })>
                    {move || view! { <Icon name=if vault.get().encrypted { "lock" } else { "unlock" } size=20/> }}
                </span>
                <div class="nc-row-label grow">
                    <b>{move || if vault.get().encrypted { "Chats are encrypted" } else { "Chats are not encrypted" }}</b>
                    <span>"Messages, titles, prompts and attachments are sealed with AES-256-GCM. The key is derived from your passphrase with Argon2id and is never stored."</span>
                </div>
            </div>
        </section>
        {move || error.get().map(|e| view! { <div class="nc-error" style="margin-bottom:14px">{e}</div> })}
        <Show when=move || !vault.get().encrypted fallback=move || view! {
            <section class="nc-section">
                <h4>"Lock"</h4>
                <p>"Hide your chats until the passphrase is entered again."</p>
                <div><button class="nc-btn nc-btn-secondary" on:click=move |_| { spawn_local(async move { if let Err(e) = api::vault_lock().await { state.error(e) } }); }><Icon name="lock" size=16/>"Lock now"</button></div>
            </section>
            <section class="nc-section">
                <h4>"Change passphrase"</h4>
                {pw(p1, "Current passphrase", "current-password")}
                <div class="nc-grid-2">{pw(p2, "New passphrase", "new-password")}{pw(p3, "Repeat new passphrase", "new-password")}</div>
                <div><button class="nc-btn nc-btn-primary" disabled=move || busy.get() on:click=move |_| run("change")>"Change passphrase"</button></div>
            </section>
            <section class="nc-section">
                <h4>"Turn off encryption"</h4>
                <p>"Decrypts everything and stores it in plain form again."</p>
                {pw(p1, "Passphrase", "current-password")}
                <div><button class="nc-btn nc-btn-danger-ghost" disabled=move || busy.get() on:click=move |_| run("disable")>"Decrypt and turn off"</button></div>
            </section>
        }>
            <section class="nc-section">
                <h4>"Encrypt my chats"</h4>
                <p>"Use at least 8 characters; a few unrelated words work well. If you forget the passphrase, your chats can't be recovered."</p>
                <div class="nc-grid-2">{pw(p1, "Passphrase", "new-password")}{pw(p2, "Repeat passphrase", "new-password")}</div>
                <div>
                    <button class="nc-btn nc-btn-primary" disabled=move || busy.get() on:click=move |_| run("enable")>
                        {move || if busy.get() { view! { <span class="nc-spinner"></span> }.into_any() } else { view! { <Icon name="lock" size=16/> }.into_any() }}
                        "Encrypt chats"
                    </button>
                </div>
            </section>
        </Show>
    }
}

#[component]
fn AdvancedTab(draft: Draft) -> impl IntoView {
    let state = use_state();
    let text = RwSignal::new(String::new());
    let loaded = RwSignal::new(false);
    let load = move || {
        spawn_local(async move {
            match api::config_toml().await {
                Ok(t) => {
                    text.set(t);
                    loaded.set(true);
                }
                Err(e) => state.error(e),
            }
        });
    };
    load();
    let info = Memo::new(move |_| state.info.get());
    view! {
        <p class="lead">"Edit the configuration file directly. Comments are kept when you save from here."</p>
        <section class="nc-section">
            <textarea class="nc-textfield code" spellcheck="false" prop:value=move || text.get() on:input=move |ev| text.set(event_target_value(&ev))></textarea>
            <div class="nc-row">
                <button class="nc-btn nc-btn-primary" disabled=move || !loaded.get() on:click=move |_| {
                    let t = text.get_untracked();
                    spawn_local(async move {
                        match api::save_config_toml(t).await {
                            Ok(cfg) => {
                                draft.set(cfg.clone());
                                state.config.set(Some(cfg));
                                state.toast(ToastLevel::Success, "config.toml saved.");
                            }
                            Err(e) => state.error(e),
                        }
                    });
                }>"Save file"</button>
                <button class="nc-btn nc-btn-ghost" on:click=move |_| load()>"Reload"</button>
            </div>
        </section>
        <section class="nc-section">
            <h4>"Locations"</h4>
            {move || info.get().map(|i| view! {
                <div class="nc-field"><label>"Configuration file"</label><div class="nc-path">{i.config_path}</div></div>
                <div class="nc-field"><label>"Chat database"</label><div class="nc-path">{i.database_path}</div></div>
                <div class="nc-field"><label>"Tool workspace"</label><div class="nc-path">{i.workspace}</div></div>
            })}
        </section>
    }
}

#[component]
fn AboutTab() -> impl IntoView {
    let state = use_state();
    let info = Memo::new(move |_| state.info.get());
    view! {
        <section class="nc-section" style="justify-items:start">
            <Mark size=48/>
            <div>
                <b style="font-family:var(--font-display);font-size:22px;letter-spacing:-0.02em">"NovumChat"</b>
                <p style="margin:4px 0 0;color:var(--muted)">{move || info.get().map(|i| format!("Version {} on {}{}", i.version, i.platform, if i.web_mode { " (web mode)" } else { "" }))}</p>
            </div>
            <p style="margin:0;color:var(--text-2);line-height:1.6;max-width:60ch">
                "A private, multi-model chat client for Ollama and OpenAI-compatible endpoints: streaming replies, side-by-side ensembles, tools with permission levels, MCP servers, web search, live HTML apps, LaTeX and optional encryption."
            </p>
            <p style="margin:0;color:var(--muted);font-size:13px">"Built with Rust, Tauri, Leptos, SeaORM, async-openai, ollama-rs, rmcp, KaTeX, highlight.js, DOMPurify and Mermaid."</p>
        </section>
    }
}

#[component]
pub fn SettingsModal() -> impl IntoView {
    let state = use_state();
    view! {
        <Show when=move || state.settings.with(|s| s.is_some())>
            <SettingsInner/>
        </Show>
    }
}

#[component]
fn SettingsInner() -> impl IntoView {
    let state = use_state();
    let draft: Draft = RwSignal::new(state.config.get_untracked().unwrap_or_default());
    let save_state = RwSignal::new(SaveState::Idle);
    let timer: StoredValue<Option<leptos::prelude::TimeoutHandle>> = StoredValue::new(None);

    // Auto-save (debounced). The live config is updated immediately so
    // appearance changes preview instantly.
    let last_saved: StoredValue<AppConfig> = StoredValue::new(draft.get_untracked());
    Effect::new(move |prev: Option<()>| {
        let cfg = draft.get();
        if prev.is_none() {
            return;
        }
        if last_saved.with_value(|l| *l == cfg) {
            return;
        }
        state.config.set(Some(cfg.clone()));
        if let Some(h) = timer.get_value() {
            h.clear();
        }
        save_state.set(SaveState::Saving);
        let handle = set_timeout_with_handle(
            move || {
                let cfg = draft.get_untracked();
                let providers_changed = last_saved.with_value(|l| l.providers != cfg.providers);
                spawn_local(async move {
                    match api::save_config(cfg.clone()).await {
                        Ok(saved) => {
                            last_saved.set_value(saved.clone());
                            state.config.set(Some(saved.clone()));
                            if saved != cfg && draft.get_untracked() == cfg {
                                draft.set(saved);
                            }
                            save_state.set(SaveState::Saved);
                            if providers_changed {
                                state.refresh_models(true).await;
                            }
                        }
                        Err(e) => {
                            save_state.set(SaveState::Failed);
                            state.error(format!("Settings not saved: {e}"));
                        }
                    }
                });
            },
            std::time::Duration::from_millis(450),
        )
        .ok();
        timer.set_value(handle);
    });

    let tab = Memo::new(move |_| state.settings.get().unwrap_or(SettingsTab::General));
    let close = move || state.settings.set(None);

    view! {
        <div class="nc-backdrop" on:click=move |_| close()>
            <div class="nc-settings" role="dialog" aria-modal="true" aria-label="Settings" on:click=|ev| ev.stop_propagation()>
                <nav class="nc-settings-nav">
                    <h2>"Settings"</h2>
                    {SettingsTab::ALL.into_iter().map(|t| view! {
                        <button class:is-active=move || tab.get() == t on:click=move |_| state.settings.set(Some(t))>
                            <Icon name=t.icon() size=17/>
                            {t.label()}
                        </button>
                    }).collect_view()}
                </nav>
                <div class="nc-settings-main">
                    <div class="nc-settings-top">
                        <h3>{move || tab.get().label()}</h3>
                        <span style="font-size:12.5px;color:var(--muted);display:inline-flex;gap:6px;align-items:center">
                            {move || match save_state.get() {
                                SaveState::Idle => view! { <span></span> }.into_any(),
                                SaveState::Saving => view! { <span class="nc-spinner" style="width:11px;height:11px"></span>"Saving"}.into_any(),
                                SaveState::Saved => view! { <Icon name="check" size=13/>"Saved to config.toml" }.into_any(),
                                SaveState::Failed => view! { <span style="color:var(--danger)">"Not saved"</span> }.into_any(),
                            }}
                        </span>
                        <button class="nc-icon-btn" aria-label="Close settings" on:click=move |_| close()>
                            <Icon name="x" size=19/>
                        </button>
                    </div>
                    <div class="nc-settings-body">
                        {move || match tab.get() {
                            SettingsTab::General => view! { <GeneralTab draft=draft/> }.into_any(),
                            SettingsTab::Appearance => view! { <AppearanceTab draft=draft/> }.into_any(),
                            SettingsTab::Providers => view! { <ProvidersTab draft=draft/> }.into_any(),
                            SettingsTab::Ensemble => view! { <EnsembleTab draft=draft/> }.into_any(),
                            SettingsTab::Tools => view! { <ToolsTab draft=draft/> }.into_any(),
                            SettingsTab::Search => view! { <SearchTab draft=draft/> }.into_any(),
                            SettingsTab::Mcp => view! { <McpTab draft=draft/> }.into_any(),
                            SettingsTab::Security => view! { <SecurityTab/> }.into_any(),
                            SettingsTab::Advanced => view! { <AdvancedTab draft=draft/> }.into_any(),
                            SettingsTab::About => view! { <AboutTab/> }.into_any(),
                        }}
                    </div>
                </div>
            </div>
        </div>
    }
}
