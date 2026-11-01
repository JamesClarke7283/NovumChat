//! Model selection (single model or an ensemble) and per-chat tool permissions.

use leptos::prelude::*;
use leptos::task::spawn_local;
use novumchat_core::permissions::PermissionLevel;
use novumchat_core::types::{ModelInfo, ModelRef};

use crate::icons::Icon;
use crate::state::{SettingsTab, use_state};
use crate::util::hue_style;

#[component]
pub fn ModelPicker(open: RwSignal<bool>) -> impl IntoView {
    let state = use_state();
    let query = RwSignal::new(String::new());
    let selected = Memo::new(move |_| state.settings_now().models);
    let compare = RwSignal::new(selected.get_untracked().len() > 1);
    let synthesize = Memo::new(move |_| state.settings_now().synthesize);

    let filtered = Memo::new(move |_| {
        let q = query.get().to_lowercase();
        state.models.with(|m| {
            let mut groups: Vec<(String, Vec<ModelInfo>)> = Vec::new();
            for info in m.models.iter().filter(|i| {
                q.is_empty() || i.id.to_lowercase().contains(&q) || i.provider_name.to_lowercase().contains(&q)
            }) {
                match groups.iter_mut().find(|(p, _)| *p == info.provider_name) {
                    Some((_, v)) => v.push(info.clone()),
                    None => groups.push((info.provider_name.clone(), vec![info.clone()])),
                }
            }
            groups
        })
    });
    let errors = Memo::new(move |_| {
        state.models.with(|m| {
            m.providers
                .iter()
                .filter(|p| p.enabled && !p.ok)
                .map(|p| (p.name.clone(), p.error.clone().unwrap_or_default()))
                .collect::<Vec<_>>()
        })
    });

    let pick = move |r: ModelRef| {
        if compare.get_untracked() {
            state.update_settings(|s| {
                if let Some(pos) = s.models.iter().position(|m| m == &r) {
                    if s.models.len() > 1 {
                        s.models.remove(pos);
                    }
                } else if s.models.len() < 8 {
                    s.models.push(r);
                }
            });
        } else {
            state.update_settings(|s| s.models = vec![r]);
            open.set(false);
        }
    };

    view! {
        <div class="nc-sheet-scrim nc-only-mobile" on:click=move |_| open.set(false)></div>
        <div style="position:fixed;inset:0;z-index:34" class="nc-only-desktop" on:click=move |_| open.set(false)></div>
        <div class="nc-popover" role="dialog" aria-label="Choose models">
            <div class="nc-popover-head">
                <h4>{move || if compare.get() { "Compare models" } else { "Choose a model" }}</h4>
                <label class="nc-row" style="gap:8px;font-size:13px;color:var(--text-2);cursor:pointer">
                    "Compare"
                    <button class="nc-switch" class:is-on=move || compare.get() role="switch" aria-checked=move || compare.get().to_string() on:click=move |_| {
                        let next = !compare.get_untracked();
                        compare.set(next);
                        if !next {
                            state.update_settings(|s| s.models.truncate(1));
                        }
                    }></button>
                </label>
                <button class="nc-icon-btn sm" title="Refresh models" aria-label="Refresh models" on:click=move |_| {
                    spawn_local(async move { state.refresh_models(true).await });
                }>
                    {move || if state.models_loading.get() { view! { <span class="nc-spinner"></span> }.into_any() } else { view! { <Icon name="refresh" size=16/> }.into_any() }}
                </button>
            </div>
            <div class="nc-popover-search">
                <input type="search" placeholder="Search models" prop:value=move || query.get() on:input=move |ev| query.set(event_target_value(&ev)) autofocus=true/>
            </div>
            <div class="nc-popover-list">
                <For
                    each=move || filtered.get()
                    key=|(p, list)| format!("{p}:{}", list.len())
                    children=move |(provider, list)| {
                        view! {
                            <div class="nc-popover-group">{provider}</div>
                            {list.into_iter().map(|info| {
                                let r = info.model_ref();
                                let r_sel = r.clone();
                                let is_sel = Memo::new(move |_| selected.with(|s| s.contains(&r_sel)));
                                let style = hue_style(Some(&r));
                                let size = info.size_bytes.map(novumchat_core::util::human_bytes);
                                view! {
                                    <button class="nc-model-row" class:is-selected=move || is_sel.get() style=style on:click=move |_| pick(r.clone())>
                                        <span class="nc-hue-dot lg"></span>
                                        <span class="name" title=info.id.clone()>{info.id.clone()}</span>
                                        <span class="caps">
                                            {info.vision.unwrap_or(false).then(|| view! { <span title="Understands images"><Icon name="eye" size=14/></span> })}
                                            {info.tools.unwrap_or(false).then(|| view! { <span title="Supports tools"><Icon name="tool" size=14/></span> })}
                                            {info.reasoning.unwrap_or(false).then(|| view! { <span title="Can reason step by step"><Icon name="brain" size=14/></span> })}
                                        </span>
                                        {size.map(|s| view! { <small style="color:var(--muted);font-size:11.5px">{s}</small> })}
                                        <Show when=move || compare.get()>
                                            <span class="check">{move || is_sel.get().then(|| view! { <Icon name="check" size=13/> })}</span>
                                        </Show>
                                    </button>
                                }
                            }).collect_view()}
                        }
                    }
                />
                <Show when=move || filtered.with(|f| f.is_empty())>
                    <div style="padding:22px 14px;text-align:center;color:var(--muted);font-size:14px;line-height:1.5">
                        {move || if query.with(|q| q.is_empty()) {
                            "No models found. Start Ollama, or add an OpenAI-compatible provider."
                        } else {
                            "No models match your search."
                        }}
                        <div style="margin-top:12px">
                            <button class="nc-btn nc-btn-secondary nc-btn-sm" on:click=move |_| {
                                open.set(false);
                                state.settings.set(Some(SettingsTab::Providers));
                            }>"Manage providers"</button>
                        </div>
                    </div>
                </Show>
                {move || errors.get().into_iter().map(|(name, err)| view! {
                    <div class="nc-popover-group" style="color:var(--danger);align-items:flex-start">
                        <Icon name="alert" size=14/>
                        <span style="font-weight:500;line-height:1.4">{format!("{name}: {err}")}</span>
                    </div>
                }).collect_view()}
            </div>
            <Show when=move || compare.get()>
                <div class="nc-popover-foot">
                    <span style="flex:1">{move || format!("{} selected. Each model answers in its own column.", selected.with(|s| s.len()))}</span>
                    <label class="nc-row" style="gap:8px;cursor:pointer" title="After all models answer, merge their answers into one">
                        "Combine"
                        <button class="nc-switch" class:is-on=move || synthesize.get() role="switch" aria-checked=move || synthesize.get().to_string() on:click=move |_| {
                            state.update_settings(|s| s.synthesize = !s.synthesize);
                        }></button>
                    </label>
                </div>
            </Show>
        </div>
    }
}

#[component]
pub fn PermissionMenu(open: RwSignal<bool>) -> impl IntoView {
    let state = use_state();
    let settings = Memo::new(move |_| state.settings_now());
    let global = Memo::new(move |_| state.config.with(|c| c.as_ref().map(|c| c.tools.permission).unwrap_or_default()));
    view! {
        <div class="nc-sheet-scrim nc-only-mobile" on:click=move |_| open.set(false)></div>
        <div style="position:fixed;inset:0;z-index:34" class="nc-only-desktop" on:click=move |_| open.set(false)></div>
        <div class="nc-popover" style="width:min(400px, calc(100vw - 28px))" role="dialog" aria-label="Tools">
            <div class="nc-popover-head">
                <h4>"Tools for this chat"</h4>
                <button class="nc-switch" class:is-on=move || settings.get().tools role="switch" aria-label="Enable tools" aria-checked=move || settings.get().tools.to_string() on:click=move |_| {
                    state.update_settings(|s| s.tools = !s.tools);
                }></button>
            </div>
            <div class="nc-popover-list" style="display:grid;gap:4px;padding:8px">
                {PermissionLevel::ALL.into_iter().map(|level| {
                    let active = Memo::new(move |_| settings.get().permission.unwrap_or(global.get()) == level);
                    view! {
                        <button class="nc-model-row" class:is-selected=move || active.get() style="align-items:flex-start" on:click=move |_| {
                            state.update_settings(|s| s.permission = if level == global.get_untracked() { None } else { Some(level) });
                        }>
                            <span style="margin-top:2px;color:var(--muted)">
                                <Icon name=match level {
                                    PermissionLevel::ReadOnly => "eye",
                                    PermissionLevel::Ask => "shield",
                                    PermissionLevel::AutoEdit => "file-edit",
                                    PermissionLevel::Yolo => "zap",
                                } size=16/>
                            </span>
                            <span style="flex:1;min-width:0">
                                <b style="display:block;font-weight:620">{level.label()}{(level == PermissionLevel::Ask).then_some(" (recommended)")}</b>
                                <span style="display:block;font-size:12.5px;color:var(--muted);line-height:1.45;white-space:normal">{level.description()}</span>
                            </span>
                            <span class="check" style="border-radius:50%">{move || active.get().then(|| view! { <Icon name="check" size=13/> })}</span>
                        </button>
                    }
                }).collect_view()}
            </div>
            <div class="nc-popover-foot">
                <span style="flex:1">{move || format!("Default for new chats: {}", global.get().label())}</span>
                <button class="nc-btn nc-btn-ghost nc-btn-sm" on:click=move |_| {
                    open.set(false);
                    state.settings.set(Some(SettingsTab::Tools));
                }>"Settings"</button>
            </div>
        </div>
    }
}
