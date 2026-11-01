//! Chat list, search and global controls.

use leptos::prelude::*;
use leptos::task::spawn_local;
use novumchat_core::config::ThemeMode;
use novumchat_core::types::ChatSummary;

use crate::api;
use crate::icons::{Icon, Mark};
use crate::state::{SettingsTab, use_state};
use crate::util::{date_group, hue_for};

#[component]
pub fn ThemeSwitch() -> impl IntoView {
    let state = use_state();
    let mode = Memo::new(move |_| state.config.with(|c| c.as_ref().map(|c| c.appearance.theme).unwrap_or_default()));
    let item = move |m: ThemeMode, icon: &'static str, label: &'static str| {
        view! {
            <button
                class:is-active=move || mode.get() == m
                title=label
                aria-label=label
                aria-pressed=move || (mode.get() == m).to_string()
                on:click=move |_| state.set_theme(m)
            >
                <Icon name=icon size=16/>
            </button>
        }
    };
    view! {
        <div class="nc-segmented icons" role="group" aria-label="Theme">
            {item(ThemeMode::Auto, "auto", "Match system")}
            {item(ThemeMode::Light, "sun", "Light")}
            {item(ThemeMode::Dark, "moon", "Dark")}
        </div>
    }
}

fn grouped(list: Vec<ChatSummary>, searching: bool) -> Vec<(&'static str, Vec<ChatSummary>)> {
    let mut groups: Vec<(&'static str, Vec<ChatSummary>)> = Vec::new();
    for c in list {
        let label = if searching {
            "Results"
        } else if c.pinned {
            "Pinned"
        } else {
            date_group(c.updated_at)
        };
        match groups.iter_mut().find(|(l, _)| *l == label) {
            Some((_, v)) => v.push(c),
            None => groups.push((label, vec![c])),
        }
    }
    groups
}

#[component]
fn ChatItem(chat: ChatSummary) -> impl IntoView {
    let state = use_state();
    let id = chat.id.clone();
    let active = {
        let id = id.clone();
        Memo::new(move |_| state.current.with(|c| c.as_deref() == Some(id.as_str())))
    };
    let renaming = RwSignal::new(false);
    let menu = RwSignal::new(false);
    let title = RwSignal::new(chat.title.clone());
    let models = chat.models.clone();
    let snippet = chat.snippet.clone();
    let pinned = chat.pinned;

    let open = {
        let id = id.clone();
        move |_| {
            if !renaming.get_untracked() {
                state.open_chat(id.clone());
            }
        }
    };
    let commit = {
        let id = id.clone();
        move || {
            renaming.set(false);
            let t = title.get_untracked();
            if !t.trim().is_empty() {
                state.rename_chat(id.clone(), t);
            }
        }
    };
    let commit_blur = commit.clone();
    let id_pin = id.clone();
    let id_del = id.clone();
    let id_export = id.clone();
    let title_del = chat.title.clone();

    view! {
        <div class="nc-chat-item" class:is-active=move || active.get() class:is-menu=move || menu.get() on:click=open role="button" tabindex="0">
            <span class="nc-hues">
                {models.iter().take(3).map(|m| view! { <span class="nc-hue-dot" style=format!("--hue:{}", hue_for(m))></span> }).collect_view()}
            </span>
            <Show
                when=move || renaming.get()
                fallback=move || {
                    let snippet = snippet.clone();
                    view! {
                        <span class="nc-chat-title nc-truncate">
                            {move || title.get()}
                            {snippet.map(|s| view! { <span class="nc-chat-snippet nc-truncate">{s}</span> })}
                        </span>
                    }
                }
            >
                <input
                    class="nc-input"
                    style="height:30px;font-size:14px"
                    prop:value=move || title.get()
                    on:input=move |ev| title.set(event_target_value(&ev))
                    on:click=|ev| ev.stop_propagation()
                    on:keydown={
                        let commit = commit.clone();
                        move |ev: web_sys::KeyboardEvent| {
                            if ev.key() == "Enter" {
                                commit();
                            } else if ev.key() == "Escape" {
                                renaming.set(false);
                            }
                        }
                    }
                    on:blur={
                        let commit = commit_blur.clone();
                        move |_| commit()
                    }
                    autofocus=true
                />
            </Show>
            {pinned.then(|| view! { <span style="color:var(--muted);display:inline-grid"><Icon name="pin" size=13/></span> })}
            <span class="nc-chat-actions">
                <button title="More" aria-label="Chat actions" on:click=move |ev| {
                    ev.stop_propagation();
                    menu.update(|m| *m = !*m);
                }>
                    <Icon name="more" size=16/>
                </button>
            </span>
            <Show when=move || menu.get()>
                <div style="position:fixed;inset:0;z-index:39" on:click=move |ev| {
                    ev.stop_propagation();
                    menu.set(false);
                }></div>
                <div class="nc-menu" style="right:4px;top:calc(100% - 2px)" on:click=|ev| ev.stop_propagation()>
                    <button on:click=move |_| {
                        menu.set(false);
                        renaming.set(true);
                    }>
                        <Icon name="edit" size=16/>
                        "Rename"
                    </button>
                    <button on:click={
                        let id = id_pin.clone();
                        move |_| {
                            menu.set(false);
                            state.pin_chat(id.clone(), !pinned);
                        }
                    }>
                        <Icon name="pin" size=16/>
                        {if pinned { "Unpin" } else { "Pin to top" }}
                    </button>
                    <button on:click={
                        let id = id_export.clone();
                        move |_| {
                            menu.set(false);
                            let id = id.clone();
                            spawn_local(async move {
                                match api::export_chat(&id, "markdown").await {
                                    Ok(r) => crate::bridge::download(&r.filename, &r.content, "text/markdown"),
                                    Err(e) => state.error(e),
                                }
                            });
                        }
                    }>
                        <Icon name="download" size=16/>
                        "Export as Markdown"
                    </button>
                    <hr/>
                    <button class="danger" on:click={
                        let id = id_del.clone();
                        let title = title_del.clone();
                        move |_| {
                            menu.set(false);
                            state.delete_chat(id.clone(), title.clone());
                        }
                    }>
                        <Icon name="trash" size=16/>
                        "Delete"
                    </button>
                </div>
            </Show>
        </div>
    }
}

#[component]
pub fn Sidebar() -> impl IntoView {
    let state = use_state();
    let timer: StoredValue<Option<leptos::prelude::TimeoutHandle>> = StoredValue::new(None);
    let on_search = move |ev| {
        let q: String = event_target_value(&ev);
        state.search.set(q.clone());
        if let Some(h) = timer.get_value() {
            h.clear();
        }
        if q.trim().is_empty() {
            state.search_results.set(None);
            return;
        }
        let handle = set_timeout_with_handle(
            move || {
                let q = q.clone();
                spawn_local(async move {
                    match api::list_chats(Some(q.clone())).await {
                        Ok(list) => {
                            if state.search.get_untracked() == q {
                                state.search_results.set(Some(list));
                            }
                        }
                        Err(e) => state.error(e),
                    }
                });
            },
            std::time::Duration::from_millis(220),
        )
        .ok();
        timer.set_value(handle);
    };
    let groups = Memo::new(move |_| {
        let searching = state.search_results.with(|r| r.is_some());
        let list = state.search_results.get().unwrap_or_else(|| state.chats.get());
        grouped(list, searching)
    });
    let encrypted = Memo::new(move |_| state.vault.with(|v| v.encrypted));

    view! {
        <aside class="nc-sidebar" aria-label="Chats">
            <div class="nc-sidebar-inner">
                <div class="nc-brand">
                    <Mark size=28/>
                    <span class="nc-brand-name">"NovumChat"</span>
                    <span class="nc-spacer"></span>
                    <button class="nc-icon-btn sm nc-only-desktop" title="Hide sidebar" aria-label="Hide sidebar" on:click=move |_| {
                        state.sidebar_hidden.set(true);
                        crate::bridge::store("novum-sidebar-hidden", "1");
                    }>
                        <Icon name="sidebar" size=18/>
                    </button>
                    <button class="nc-icon-btn sm nc-only-mobile" aria-label="Close" on:click=move |_| state.drawer.set(false)>
                        <Icon name="x" size=18/>
                    </button>
                </div>
                <button class="nc-new-chat" on:click=move |_| state.new_chat()>
                    <Icon name="plus" size=18/>
                    "New chat"
                    <kbd class="nc-only-desktop">"Ctrl ⇧ O"</kbd>
                </button>
                <label class="nc-search">
                    <Icon name="search" size=16/>
                    <span class="nc-sr">"Search chats"</span>
                    <input type="search" placeholder="Search chats" prop:value=move || state.search.get() on:input=on_search/>
                </label>
                <nav class="nc-chat-list">
                    <For
                        each=move || groups.get()
                        key=|(label, chats)| format!("{label}:{}", chats.iter().map(|c| format!("{}{}{}", c.id, c.title, c.pinned)).collect::<String>())
                        children=move |(label, chats)| {
                            view! {
                                <div class="nc-chat-group">{label}</div>
                                {chats.into_iter().map(|c| view! { <ChatItem chat=c/> }).collect_view()}
                            }
                        }
                    />
                    <Show when=move || groups.with(|g| g.is_empty())>
                        <div class="nc-sidebar-empty">
                            {move || if state.search_results.with(|r| r.is_some()) {
                                "No chats match your search.".to_string()
                            } else {
                                "Your conversations will appear here. Start one with any model, or with several at once.".to_string()
                            }}
                        </div>
                    </Show>
                </nav>
                <div class="nc-sidebar-foot">
                    <ThemeSwitch/>
                    <span class="nc-spacer"></span>
                    <Show when=move || encrypted.get()>
                        <button class="nc-icon-btn" title="Lock chats" aria-label="Lock chats" on:click=move |_| {
                            spawn_local(async move {
                                if let Err(e) = api::vault_lock().await {
                                    state.error(e);
                                }
                            });
                        }>
                            <Icon name="lock" size=18/>
                        </button>
                    </Show>
                    <button class="nc-icon-btn" title="Settings (Ctrl ,)" aria-label="Settings" on:click=move |_| {
                        state.drawer.set(false);
                        state.settings.set(Some(SettingsTab::General));
                    }>
                        <Icon name="settings" size=19/>
                    </button>
                </div>
            </div>
        </aside>
    }
}
