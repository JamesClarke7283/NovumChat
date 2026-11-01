//! Top bar: navigation, chat title, models and chat actions.

use leptos::prelude::*;
use leptos::task::spawn_local;
use novumchat_core::rpc::UpdateChatParams;

use crate::api;
use crate::icons::Icon;
use crate::state::use_state;
use crate::util::{hue_for, short_model};

#[component]
pub fn Header() -> impl IntoView {
    let state = use_state();
    let editing = RwSignal::new(false);
    let draft = RwSignal::new(String::new());
    let menu = RwSignal::new(false);
    let prompt_open = RwSignal::new(false);
    let prompt_text = RwSignal::new(String::new());

    let title = Memo::new(move |_| {
        state.chat.with(|c| c.as_ref().map(|c| c.title.clone())).unwrap_or_else(|| "New chat".into())
    });
    let has_chat = Memo::new(move |_| state.chat.with(|c| c.is_some()));
    let models = Memo::new(move |_| state.settings_now().models);

    let save_title = move || {
        editing.set(false);
        if let Some(chat) = state.chat.get_untracked() {
            let t = draft.get_untracked();
            if !t.trim().is_empty() && t != chat.title {
                state.rename_chat(chat.id, t);
            }
        }
    };

    let export = move |format: &'static str| {
        menu.set(false);
        if let Some(chat) = state.chat.get_untracked() {
            spawn_local(async move {
                match api::export_chat(&chat.id, format).await {
                    Ok(r) => {
                        let mime = if format == "json" { "application/json" } else { "text/markdown" };
                        crate::bridge::download(&r.filename, &r.content, mime);
                    }
                    Err(e) => state.error(e),
                }
            });
        }
    };

    view! {
        <header class="nc-header">
            <button class="nc-icon-btn nc-only-mobile" aria-label="Open chats" on:click=move |_| state.drawer.set(true)>
                <Icon name="menu" size=21/>
            </button>
            <Show when=move || state.sidebar_hidden.get()>
                <button class="nc-icon-btn nc-only-desktop" title="Show sidebar" aria-label="Show sidebar" on:click=move |_| {
                    state.sidebar_hidden.set(false);
                    crate::bridge::store("novum-sidebar-hidden", "0");
                }>
                    <Icon name="sidebar" size=19/>
                </button>
                <button class="nc-icon-btn nc-only-desktop" title="New chat" aria-label="New chat" on:click=move |_| state.new_chat()>
                    <Icon name="edit" size=18/>
                </button>
            </Show>
            <Show
                when=move || editing.get()
                fallback=move || view! {
                    <button
                        class="nc-header-title nc-truncate"
                        title="Rename"
                        disabled=move || !has_chat.get()
                        on:click=move |_| {
                            draft.set(title.get_untracked());
                            editing.set(true);
                        }
                    >
                        {move || title.get()}
                    </button>
                }
            >
                <input
                    class="nc-header-title-input"
                    prop:value=move || draft.get()
                    on:input=move |ev| draft.set(event_target_value(&ev))
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Enter" {
                            save_title();
                        } else if ev.key() == "Escape" {
                            editing.set(false);
                        }
                    }
                    on:blur=move |_| save_title()
                    autofocus=true
                />
            </Show>
            <div class="nc-header-models nc-only-desktop">
                {move || {
                    let list = models.get();
                    if list.len() > 2 {
                        view! {
                            <span class="nc-model-chip">
                                <span class="nc-hues">
                                    {list.iter().take(5).map(|m| view! { <span class="nc-hue-dot" style=format!("--hue:{}", hue_for(m))></span> }).collect_view()}
                                </span>
                                {format!("{} models", list.len())}
                            </span>
                        }
                        .into_any()
                    } else {
                        list.into_iter()
                            .map(|m| view! {
                                <span class="nc-model-chip" title=m.key()>
                                    <span class="nc-hue-dot" style=format!("--hue:{}", hue_for(&m))></span>
                                    <span class="nc-truncate">{short_model(&m)}</span>
                                </span>
                            })
                            .collect_view()
                            .into_any()
                    }
                }}
            </div>
            <span class="nc-spacer"></span>
            <button class="nc-icon-btn nc-only-mobile" aria-label="New chat" on:click=move |_| state.new_chat()>
                <Icon name="edit" size=19/>
            </button>
            <Show when=move || has_chat.get()>
                <div style="position:relative">
                    <button class="nc-icon-btn" aria-label="Chat options" title="Chat options" on:click=move |_| menu.update(|m| *m = !*m)>
                        <Icon name="more" size=19/>
                    </button>
                    <Show when=move || menu.get()>
                        <div style="position:fixed;inset:0;z-index:39" on:click=move |_| menu.set(false)></div>
                        <div class="nc-menu" style="right:0;top:calc(100% + 6px)">
                            <button on:click=move |_| {
                                menu.set(false);
                                prompt_text.set(state.chat.get_untracked().and_then(|c| c.system_prompt).unwrap_or_default());
                                prompt_open.set(true);
                            }>
                                <Icon name="sliders" size=16/>
                                "System prompt…"
                            </button>
                            <button on:click=move |_| {
                                menu.set(false);
                                if let Some(c) = state.chat.get_untracked() {
                                    state.pin_chat(c.id, !c.pinned);
                                }
                            }>
                                <Icon name="pin" size=16/>
                                {move || if state.chat.with(|c| c.as_ref().is_some_and(|c| c.pinned)) { "Unpin" } else { "Pin to top" }}
                            </button>
                            <button on:click=move |_| export("markdown")>
                                <Icon name="download" size=16/>
                                "Export as Markdown"
                            </button>
                            <button on:click=move |_| export("json")>
                                <Icon name="code" size=16/>
                                "Export as JSON"
                            </button>
                            <hr/>
                            <button class="danger" on:click=move |_| {
                                menu.set(false);
                                if let Some(c) = state.chat.get_untracked() {
                                    state.delete_chat(c.id, c.title);
                                }
                            }>
                                <Icon name="trash" size=16/>
                                "Delete chat"
                            </button>
                        </div>
                    </Show>
                </div>
            </Show>
            <Show when=move || prompt_open.get()>
                <div class="nc-backdrop" on:click=move |_| prompt_open.set(false)>
                    <div class="nc-dialog" style="width:min(560px,100%)" on:click=|ev| ev.stop_propagation()>
                        <h3>"System prompt"</h3>
                        <p>"Instructions the model follows for this chat only. Leave empty to use the default from Settings."</p>
                        <textarea
                            class="nc-textfield"
                            style="min-height:160px"
                            placeholder="You are a concise assistant that answers in British English…"
                            prop:value=move || prompt_text.get()
                            on:input=move |ev| prompt_text.set(event_target_value(&ev))
                        ></textarea>
                        <div class="nc-dialog-actions">
                            <button class="nc-btn nc-btn-ghost" on:click=move |_| prompt_open.set(false)>"Cancel"</button>
                            <button class="nc-btn nc-btn-primary" on:click=move |_| {
                                prompt_open.set(false);
                                if let Some(c) = state.chat.get_untracked() {
                                    let text = prompt_text.get_untracked();
                                    spawn_local(async move {
                                        if let Err(e) = api::update_chat(UpdateChatParams { id: c.id, system_prompt: Some(text), ..Default::default() }).await {
                                            state.error(e);
                                        }
                                    });
                                }
                            }>"Save prompt"</button>
                        </div>
                    </div>
                </div>
            </Show>
        </header>
    }
}
